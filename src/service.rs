use std::{collections::HashMap, sync::Arc, time::Duration};

use futures::future::join_all;
use moka::future::Cache;
use reqwest::Client;

use crate::{
    adapters,
    error::AppError,
    model::{
        Candle, CandleRequest, ComparisonCandle, ComparisonResponse, ComparisonStats, Market,
        TickerListing, Venue,
    },
};

pub const TICKER_CACHE_TTL_SECONDS: u64 = 300;

#[derive(Clone)]
pub struct MarketDataService {
    client: Client,
    candle_cache: Cache<CandleRequest, Arc<Vec<Candle>>>,
    market_cache: Cache<Venue, Arc<Vec<Market>>>,
    ticker_cache: Cache<(), Arc<Vec<TickerListing>>>,
}

impl MarketDataService {
    pub fn new() -> Self {
        let client = Client::builder()
            .user_agent(concat!("basis-lab/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(4))
            .timeout(Duration::from_secs(12))
            .pool_idle_timeout(Duration::from_secs(45))
            .pool_max_idle_per_host(4)
            .gzip(true)
            .brotli(true)
            .build()
            .expect("HTTP client configuration is valid");
        Self {
            client,
            candle_cache: Cache::builder()
                .max_capacity(256)
                .time_to_live(Duration::from_secs(15))
                .build(),
            market_cache: Cache::builder()
                .max_capacity(Venue::ALL.len() as u64)
                .time_to_live(Duration::from_secs(TICKER_CACHE_TTL_SECONDS))
                .build(),
            ticker_cache: Cache::builder()
                .max_capacity(1)
                .time_to_live(Duration::from_secs(TICKER_CACHE_TTL_SECONDS))
                .build(),
        }
    }

    pub async fn candles(&self, request: CandleRequest) -> Result<Arc<Vec<Candle>>, AppError> {
        let client = self.client.clone();
        let fetch_request = request.clone();
        self.candle_cache
            .try_get_with(request, async move {
                adapters::fetch_candles(&client, &fetch_request)
                    .await
                    .map(Arc::new)
            })
            .await
            .map_err(|error| error.as_ref().clone())
    }

    pub async fn markets(&self, venue: Venue) -> Result<Arc<Vec<Market>>, AppError> {
        let client = self.client.clone();
        self.market_cache
            .try_get_with(venue, async move {
                adapters::fetch_markets(&client, venue).await.map(Arc::new)
            })
            .await
            .map_err(|error| error.as_ref().clone())
    }

    pub async fn tickers(&self) -> Result<Arc<Vec<TickerListing>>, AppError> {
        let service = self.clone();
        self.ticker_cache
            .try_get_with((), async move {
                let results = join_all(Venue::ALL.into_iter().map(|venue| {
                    let service = service.clone();
                    async move { (venue, service.markets(venue).await) }
                }))
                .await;

                let mut tickers = Vec::new();
                let mut first_error = None;
                for (venue, result) in results {
                    match result {
                        Ok(markets) => tickers.extend(
                            markets
                                .iter()
                                .filter(|market| market.active)
                                .map(|market| TickerListing::from_market(venue, market)),
                        ),
                        Err(error) if first_error.is_none() => first_error = Some(error),
                        Err(_) => {}
                    }
                }
                if tickers.is_empty() {
                    return Err(first_error.unwrap_or(AppError::Upstream {
                        venue: "ticker_catalog".into(),
                        message: "no venue catalogs were available".into(),
                    }));
                }
                tickers.sort_unstable_by(|left, right| {
                    left.normalized_symbol
                        .cmp(&right.normalized_symbol)
                        .then_with(|| left.venue.cmp(right.venue))
                        .then_with(|| left.symbol.cmp(&right.symbol))
                });
                tickers.dedup_by(|left, right| {
                    left.venue == right.venue && left.symbol == right.symbol
                });
                Ok(Arc::new(tickers))
            })
            .await
            .map_err(|error| error.as_ref().clone())
    }

    pub async fn compare(
        &self,
        left: CandleRequest,
        right: CandleRequest,
        scale: f64,
    ) -> Result<ComparisonResponse, AppError> {
        let (left_candles, right_candles) =
            tokio::try_join!(self.candles(left.clone()), self.candles(right.clone()))?;
        compare_candles(&left, &right, &left_candles, &right_candles, scale)
    }
}

impl Default for MarketDataService {
    fn default() -> Self {
        Self::new()
    }
}

pub fn compare_candles(
    left_request: &CandleRequest,
    right_request: &CandleRequest,
    left: &[Candle],
    right: &[Candle],
    scale: f64,
) -> Result<ComparisonResponse, AppError> {
    if !scale.is_finite() || scale <= 0.0 || scale > 1_000_000_000.0 {
        return Err(AppError::BadRequest(
            "scale must be greater than 0 and at most 1,000,000,000".into(),
        ));
    }

    let right_by_time: HashMap<i64, &Candle> =
        right.iter().map(|candle| (candle.time, candle)).collect();
    let mut candles = Vec::with_capacity(left.len().min(right.len()));
    for a in left {
        let Some(b) = right_by_time.get(&a.time) else {
            continue;
        };
        if [b.open, b.high, b.low, b.close]
            .into_iter()
            .any(|value| value <= 0.0)
        {
            continue;
        }
        let open = (a.open / b.open - 1.0) * scale;
        let close = (a.close / b.close - 1.0) * scale;
        let mut high = (a.high / b.low - 1.0) * scale;
        let mut low = (a.low / b.high - 1.0) * scale;
        high = high.max(open).max(close);
        low = low.min(open).min(close);
        candles.push(ComparisonCandle {
            time: a.time,
            open,
            high,
            low,
            close,
            left_close: a.close,
            right_close: b.close,
        });
    }

    if candles.is_empty() {
        return Err(AppError::NoOverlap);
    }
    let stats = statistics(&candles);
    let matched_candles = candles.len();
    Ok(ComparisonResponse {
        formula: format!(
            "({}:{} / {}:{} - 1) × {}",
            left_request.venue.id(),
            left_request.market,
            right_request.venue.id(),
            right_request.market,
            scale
        ),
        unit: if (scale - 10_000.0).abs() < f64::EPSILON {
            "bps"
        } else {
            "scaled ratio delta"
        },
        scale,
        interval: left_request.interval.name.into(),
        approximation: "OHLC envelope: high = (left.high / right.low - 1) × scale; low = (left.low / right.high - 1) × scale. Venue extremes may not be simultaneous.",
        matched_candles,
        dropped_left: left.len().saturating_sub(matched_candles),
        dropped_right: right.len().saturating_sub(matched_candles),
        candles,
        stats,
    })
}

fn statistics(candles: &[ComparisonCandle]) -> ComparisonStats {
    let count = candles.len() as f64;
    let mean = candles.iter().map(|candle| candle.close).sum::<f64>() / count;
    let variance = candles
        .iter()
        .map(|candle| {
            let delta = candle.close - mean;
            delta * delta
        })
        .sum::<f64>()
        / count;
    let standard_deviation = variance.sqrt();
    let latest = candles
        .last()
        .map(|candle| candle.close)
        .unwrap_or_default();
    ComparisonStats {
        latest,
        mean,
        standard_deviation,
        minimum: candles
            .iter()
            .map(|candle| candle.low)
            .fold(f64::INFINITY, f64::min),
        maximum: candles
            .iter()
            .map(|candle| candle.high)
            .fold(f64::NEG_INFINITY, f64::max),
        z_score: if standard_deviation == 0.0 {
            0.0
        } else {
            (latest - mean) / standard_deviation
        },
    }
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;

    use super::*;
    use crate::model::{Interval, Venue};

    fn request(venue: Venue) -> CandleRequest {
        CandleRequest {
            venue,
            market: "BTCUSDT".into(),
            interval: Interval {
                name: "1m",
                millis: 60_000,
            },
            from: 0,
            to: 120_000,
            limit: 100,
        }
    }

    fn candle(time: i64, open: f64, high: f64, low: f64, close: f64) -> Candle {
        Candle {
            time,
            open,
            high,
            low,
            close,
            volume: None,
        }
    }

    #[test]
    fn comparison_uses_conservative_ohlc_envelope_and_exact_timestamp_join() {
        let left = vec![
            candle(0, 102.0, 106.0, 100.0, 104.0),
            candle(60_000, 110.0, 112.0, 108.0, 111.0),
        ];
        let right = vec![
            candle(0, 100.0, 104.0, 98.0, 102.0),
            candle(120_000, 100.0, 101.0, 99.0, 100.0),
        ];
        let result = compare_candles(
            &request(Venue::BybitPerp),
            &request(Venue::MexcPerp),
            &left,
            &right,
            10_000.0,
        )
        .unwrap();
        assert_eq!(result.candles.len(), 1);
        assert_relative_eq!(result.candles[0].open, 200.0, epsilon = 1e-9);
        assert_relative_eq!(result.candles[0].close, (104.0 / 102.0 - 1.0) * 10_000.0);
        assert_relative_eq!(result.candles[0].high, (106.0 / 98.0 - 1.0) * 10_000.0);
        assert_relative_eq!(result.candles[0].low, (100.0 / 104.0 - 1.0) * 10_000.0);
        assert_eq!(result.dropped_left, 1);
        assert_eq!(result.dropped_right, 1);
    }

    #[test]
    fn comparison_rejects_non_overlapping_series() {
        let result = compare_candles(
            &request(Venue::BybitPerp),
            &request(Venue::MexcPerp),
            &[candle(0, 1.0, 1.0, 1.0, 1.0)],
            &[candle(60_000, 1.0, 1.0, 1.0, 1.0)],
            10_000.0,
        );
        assert!(matches!(result, Err(AppError::NoOverlap)));
    }
}
