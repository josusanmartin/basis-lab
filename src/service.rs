use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use futures::future::join_all;
use moka::future::Cache;
use reqwest::Client;
use tokio::sync::Mutex;

use crate::{
    adapters,
    error::AppError,
    model::{
        Candle, CandleRequest, ComparisonCandle, ComparisonResponse, ComparisonStats, Market,
        TickerListing, Venue,
    },
};

pub const BASIS_POINT_SCALE: f64 = 10_000.0;
pub const TICKER_CACHE_TTL_SECONDS: u64 = 300;
pub const DEFAULT_BINANCE_REQUEST_WEIGHT_PER_MINUTE: u32 = 300;

#[derive(Clone)]
struct BinanceRateLimiter {
    capacity: f64,
    refill_per_second: f64,
    state: Arc<Mutex<BinanceRateState>>,
}

struct BinanceRateState {
    tokens: f64,
    updated_at: Instant,
}

impl BinanceRateLimiter {
    fn new(weight_per_minute: u32) -> Self {
        let capacity = weight_per_minute.max(1) as f64;
        Self {
            capacity,
            refill_per_second: capacity / 60.0,
            state: Arc::new(Mutex::new(BinanceRateState {
                tokens: capacity,
                updated_at: Instant::now(),
            })),
        }
    }

    async fn reserve(&self, venue: Venue, weight: u32) -> Result<(), AppError> {
        let mut state = self.state.lock().await;
        let now = Instant::now();
        state.tokens = (state.tokens
            + now.duration_since(state.updated_at).as_secs_f64() * self.refill_per_second)
            .min(self.capacity);
        state.updated_at = now;
        if state.tokens >= weight as f64 {
            state.tokens -= weight as f64;
            return Ok(());
        }
        let retry_after_seconds =
            ((weight as f64 - state.tokens) / self.refill_per_second).ceil() as u64;
        Err(AppError::RateLimited {
            venue: venue.id().into(),
            message: "local Binance request-weight budget exhausted; upstream request was not sent"
                .into(),
            retry_after_seconds: Some(retry_after_seconds.max(1)),
        })
    }
}

#[derive(Clone)]
pub struct MarketDataService {
    client: Client,
    candle_cache: Cache<CandleRequest, Arc<Vec<Candle>>>,
    market_cache: Cache<Venue, Arc<Vec<Market>>>,
    ticker_cache: Cache<(), Arc<Vec<TickerListing>>>,
    binance_rate_limiter: BinanceRateLimiter,
}

impl MarketDataService {
    pub fn new() -> Self {
        Self::with_binance_weight_limit(DEFAULT_BINANCE_REQUEST_WEIGHT_PER_MINUTE)
    }

    pub fn with_binance_weight_limit(weight_per_minute: u32) -> Self {
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
            binance_rate_limiter: BinanceRateLimiter::new(weight_per_minute),
        }
    }

    pub async fn candles(&self, request: CandleRequest) -> Result<Arc<Vec<Candle>>, AppError> {
        let request = canonical_candle_request(request);
        if request.from > request.to {
            return Ok(Arc::new(Vec::new()));
        }
        let client = self.client.clone();
        let fetch_request = request.clone();
        let rate_limiter = self.binance_rate_limiter.clone();
        self.candle_cache
            .try_get_with(request, async move {
                if let Some(weight) = binance_candle_weight(&fetch_request) {
                    rate_limiter.reserve(fetch_request.venue, weight).await?;
                }
                adapters::fetch_candles(&client, &fetch_request)
                    .await
                    .map(Arc::new)
            })
            .await
            .map_err(|error| error.as_ref().clone())
    }

    pub async fn markets(&self, venue: Venue) -> Result<Arc<Vec<Market>>, AppError> {
        let client = self.client.clone();
        let rate_limiter = self.binance_rate_limiter.clone();
        self.market_cache
            .try_get_with(venue, async move {
                if let Some(weight) = binance_market_weight(venue) {
                    rate_limiter.reserve(venue, weight).await?;
                }
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

fn canonical_candle_request(mut request: CandleRequest) -> CandleRequest {
    let step = request.interval.millis;
    let from_floor = request.from.div_euclid(step) * step;
    request.from = if from_floor == request.from {
        from_floor
    } else {
        from_floor.saturating_add(step)
    };
    request.to = request.to.div_euclid(step) * step;
    request
}

fn binance_candle_weight(request: &CandleRequest) -> Option<u32> {
    match request.venue {
        Venue::BinanceSpot => Some(2),
        Venue::BinancePerp => Some(match request.limit.min(1500) {
            0..100 => 1,
            100..500 => 2,
            500..=1000 => 5,
            _ => 10,
        }),
        _ => None,
    }
}

fn binance_market_weight(venue: Venue) -> Option<u32> {
    match venue {
        Venue::BinanceSpot => Some(4),
        Venue::BinancePerp => Some(1),
        _ => None,
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
            left_volume: a
                .volume
                .filter(|volume| volume.is_finite() && *volume >= 0.0),
            right_volume: b
                .volume
                .filter(|volume| volume.is_finite() && *volume >= 0.0),
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
        unit: if (scale - BASIS_POINT_SCALE).abs() < f64::EPSILON {
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
    fn candle_cache_keys_share_equivalent_opening_timestamp_windows() {
        let mut first = request(Venue::BinancePerp);
        first.from = 60_001;
        first.to = 120_030;
        let mut second = first.clone();
        second.from = 119_999;
        second.to = 120_059;

        assert_eq!(
            canonical_candle_request(first),
            canonical_candle_request(second)
        );
        assert_eq!(
            canonical_candle_request(request(Venue::BinancePerp)).from,
            0
        );
    }

    #[test]
    fn binance_candle_weights_follow_the_requested_limit_tiers() {
        let mut request = request(Venue::BinancePerp);
        for (limit, weight) in [(99, 1), (100, 2), (499, 2), (500, 5), (1000, 5), (1001, 10)] {
            request.limit = limit;
            assert_eq!(binance_candle_weight(&request), Some(weight));
        }
        request.venue = Venue::BybitPerp;
        assert_eq!(binance_candle_weight(&request), None);
    }

    #[tokio::test]
    async fn binance_token_bucket_rejects_before_exceeding_its_budget() {
        let limiter = BinanceRateLimiter::new(10);
        limiter.reserve(Venue::BinancePerp, 5).await.unwrap();
        limiter.reserve(Venue::BinancePerp, 5).await.unwrap();
        let error = limiter.reserve(Venue::BinancePerp, 1).await.unwrap_err();

        assert!(matches!(
            error,
            AppError::RateLimited {
                retry_after_seconds: Some(6),
                ..
            }
        ));
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
            BASIS_POINT_SCALE,
        )
        .unwrap();
        assert_eq!(result.candles.len(), 1);
        assert_relative_eq!(result.candles[0].open, 200.0, epsilon = 1e-9);
        assert_relative_eq!(
            result.candles[0].close,
            (104.0 / 102.0 - 1.0) * BASIS_POINT_SCALE
        );
        assert_relative_eq!(
            result.candles[0].high,
            (106.0 / 98.0 - 1.0) * BASIS_POINT_SCALE
        );
        assert_relative_eq!(
            result.candles[0].low,
            (100.0 / 104.0 - 1.0) * BASIS_POINT_SCALE
        );
        assert_eq!(result.scale, BASIS_POINT_SCALE);
        assert_eq!(result.unit, "bps");
        assert_eq!(result.candles[0].left_volume, None);
        assert_eq!(result.candles[0].right_volume, None);
        assert_eq!(result.dropped_left, 1);
        assert_eq!(result.dropped_right, 1);
    }

    #[test]
    fn comparison_preserves_each_venues_volume_when_available() {
        let mut left = candle(0, 102.0, 106.0, 100.0, 104.0);
        left.volume = Some(125.5);
        let mut right = candle(0, 100.0, 104.0, 98.0, 102.0);
        right.volume = Some(98.25);
        let result = compare_candles(
            &request(Venue::BybitPerp),
            &request(Venue::MexcPerp),
            &[left],
            &[right],
            BASIS_POINT_SCALE,
        )
        .unwrap();
        assert_eq!(result.candles[0].left_volume, Some(125.5));
        assert_eq!(result.candles[0].right_volume, Some(98.25));
    }

    #[test]
    fn comparison_rejects_non_overlapping_series() {
        let result = compare_candles(
            &request(Venue::BybitPerp),
            &request(Venue::MexcPerp),
            &[candle(0, 1.0, 1.0, 1.0, 1.0)],
            &[candle(60_000, 1.0, 1.0, 1.0, 1.0)],
            BASIS_POINT_SCALE,
        );
        assert!(matches!(result, Err(AppError::NoOverlap)));
    }
}
