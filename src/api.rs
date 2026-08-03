use std::{str::FromStr, sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderName, HeaderValue, Method, header},
    response::{IntoResponse, Response},
    routing::get,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::Semaphore;
use tower_http::{
    compression::CompressionLayer,
    cors::CorsLayer,
    limit::RequestBodyLimitLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeaderLayer,
    trace::TraceLayer,
};

use crate::{
    error::AppError,
    model::{
        Candle, CandleRequest, Interval, Market, TickerListing, Venue, VenueInfo, compact_ticker,
    },
    service::{MarketDataService, TICKER_CACHE_TTL_SECONDS},
};

const MAX_LIMIT: usize = 1500;
const MAX_RANGE_MS: i64 = 366 * 86_400_000;

#[derive(Clone)]
pub struct AppState {
    pub service: MarketDataService,
    permits: Arc<Semaphore>,
}

impl AppState {
    pub fn new(max_concurrent_upstream_requests: usize) -> Self {
        Self {
            service: MarketDataService::new(),
            permits: Arc::new(Semaphore::new(max_concurrent_upstream_requests.max(1))),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new(64)
    }
}

pub fn router(state: AppState) -> Router {
    let request_id = HeaderName::from_static("x-request-id");
    let api = Router::new()
        .route("/health", get(health))
        .route("/venues", get(venues))
        .route("/markets", get(markets))
        .route("/tickers", get(tickers))
        .route("/candles", get(candles))
        .route("/compare", get(compare));

    Router::new()
        .nest("/api/v1", api)
        .route("/openapi.json", get(openapi))
        .route("/docs", get(api_docs))
        .fallback_service(
            ServeDir::new("web")
                .append_index_html_on_directories(true)
                .fallback(ServeFile::new("web/index.html")),
        )
        .layer(RequestBodyLimitLayer::new(64 * 1024))
        .layer(CompressionLayer::new())
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::REFERRER_POLICY,
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(
                "default-src 'self'; style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; object-src 'none'; frame-ancestors 'none'; base-uri 'none'",
            ),
        ))
        .layer(
            CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods([Method::GET])
                .allow_headers([header::ACCEPT, header::CONTENT_TYPE]),
        )
        .layer(PropagateRequestIdLayer::new(request_id.clone()))
        .layer(SetRequestIdLayer::new(request_id, MakeRequestUuid))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health() -> impl IntoResponse {
    Json(json!({
        "status": "ok",
        "service": "basis-lab",
        "version": env!("CARGO_PKG_VERSION"),
        "time": Utc::now().to_rfc3339(),
    }))
}

async fn venues() -> impl IntoResponse {
    let venues: Vec<VenueInfo> = Venue::ALL
        .into_iter()
        .map(|venue| VenueInfo {
            id: venue.id(),
            label: venue.label(),
            market_type: venue.market_type(),
            intervals: venue.intervals(),
            symbol_example: match venue {
                Venue::HyperliquidPerp | Venue::LighterPerp => "BTC",
                Venue::OndoPerp => "BTC-USD.P",
                Venue::MexcPerp => "BTC_USDT",
                Venue::OkxSpot => "BTC-USDT",
                Venue::OkxPerp => "BTC-USDT-SWAP",
                _ => "BTCUSDT",
            },
        })
        .collect();
    Json(json!({ "data": venues }))
}

#[derive(Deserialize)]
struct MarketsQuery {
    venue: String,
    #[serde(default)]
    query: String,
    #[serde(default = "default_market_limit")]
    limit: usize,
}

fn default_market_limit() -> usize {
    200
}

async fn markets(
    State(state): State<AppState>,
    Query(query): Query<MarketsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let venue = Venue::from_str(&query.venue)?;
    validate_search(&query.query, query.limit)?;
    let _permit = acquire(&state).await?;
    let needle = compact_ticker(&query.query);
    let cached = state.service.markets(venue).await?;
    let mut markets: Vec<MarketSearchResult> = cached
        .iter()
        .filter(|market| market.active && market_matches(market, &needle))
        .map(MarketSearchResult::from)
        .collect();
    markets.sort_unstable_by(|left, right| {
        search_rank(&left.symbol, &left.normalized_symbol, &left.base, &needle)
            .cmp(&search_rank(
                &right.symbol,
                &right.normalized_symbol,
                &right.base,
                &needle,
            ))
            .then_with(|| left.normalized_symbol.cmp(&right.normalized_symbol))
            .then_with(|| left.symbol.cmp(&right.symbol))
    });
    let total = markets.len();
    markets.truncate(query.limit);
    Ok((
        [(header::CACHE_CONTROL, ticker_cache_control())],
        Json(json!({
            "venue": venue.id(),
            "query": query.query,
            "total": total,
            "cache_ttl_seconds": TICKER_CACHE_TTL_SECONDS,
            "data": markets,
        })),
    ))
}

#[derive(Serialize)]
struct MarketSearchResult {
    symbol: String,
    normalized_symbol: String,
    base: String,
    quote: String,
    active: bool,
}

impl From<&Market> for MarketSearchResult {
    fn from(market: &Market) -> Self {
        Self {
            symbol: market.symbol.clone(),
            normalized_symbol: market.normalized_symbol(),
            base: market.base.trim().to_ascii_uppercase(),
            quote: market.quote.trim().to_ascii_uppercase(),
            active: market.active,
        }
    }
}

#[derive(Deserialize)]
struct TickersQuery {
    venue: Option<String>,
    #[serde(default)]
    query: String,
    #[serde(default = "default_market_limit")]
    limit: usize,
}

async fn tickers(
    State(state): State<AppState>,
    Query(query): Query<TickersQuery>,
) -> Result<impl IntoResponse, AppError> {
    validate_search(&query.query, query.limit)?;
    let venue = query.venue.as_deref().map(Venue::from_str).transpose()?;
    let _permit = acquire(&state).await?;
    let needle = compact_ticker(&query.query);
    let cached = state.service.tickers().await?;
    let mut matches: Vec<TickerListing> = cached
        .iter()
        .filter(|ticker| venue.is_none_or(|value| ticker.venue == value.id()))
        .filter(|ticker| needle.is_empty() || ticker.search_key().contains(&needle))
        .cloned()
        .collect();
    matches.sort_unstable_by(|left, right| {
        search_rank(&left.symbol, &left.normalized_symbol, &left.base, &needle)
            .cmp(&search_rank(
                &right.symbol,
                &right.normalized_symbol,
                &right.base,
                &needle,
            ))
            .then_with(|| left.normalized_symbol.cmp(&right.normalized_symbol))
            .then_with(|| left.venue.cmp(right.venue))
            .then_with(|| left.symbol.cmp(&right.symbol))
    });
    let total = matches.len();
    matches.truncate(query.limit);
    Ok((
        [(header::CACHE_CONTROL, ticker_cache_control())],
        Json(json!({
            "query": query.query,
            "venue": venue.map(Venue::id),
            "total": total,
            "cache_ttl_seconds": TICKER_CACHE_TTL_SECONDS,
            "data": matches,
        })),
    ))
}

fn validate_search(query: &str, limit: usize) -> Result<(), AppError> {
    if limit == 0 || limit > 1000 {
        return Err(AppError::BadRequest(
            "ticker limit must be between 1 and 1000".into(),
        ));
    }
    if query.len() > 64
        || !query.is_ascii()
        || query.bytes().any(|byte| byte.is_ascii_control())
        || (!query.trim().is_empty() && compact_ticker(query).is_empty())
    {
        return Err(AppError::BadRequest(
            "ticker query must be at most 64 printable ASCII characters and include a letter or number"
                .into(),
        ));
    }
    Ok(())
}

fn market_matches(market: &Market, needle: &str) -> bool {
    let normalized = market.normalized_symbol();
    needle.is_empty()
        || [
            market.symbol.as_str(),
            market.base.as_str(),
            market.quote.as_str(),
            normalized.as_str(),
        ]
        .into_iter()
        .any(|value| compact_ticker(value).contains(needle))
}

fn search_rank(symbol: &str, normalized: &str, base: &str, needle: &str) -> u8 {
    if needle.is_empty() {
        return 4;
    }
    let symbol = compact_ticker(symbol);
    let normalized = compact_ticker(normalized);
    let base = compact_ticker(base);
    if symbol == needle || normalized == needle {
        0
    } else if base == needle {
        1
    } else if symbol.starts_with(needle) || normalized.starts_with(needle) {
        2
    } else {
        3
    }
}

fn ticker_cache_control() -> HeaderValue {
    HeaderValue::from_static("public, max-age=30, stale-while-revalidate=300")
}

#[derive(Debug, Deserialize)]
struct CandleQuery {
    venue: String,
    market: String,
    interval: String,
    from: i64,
    to: i64,
    #[serde(default = "default_candle_limit")]
    limit: usize,
}

fn default_candle_limit() -> usize {
    1000
}

impl CandleQuery {
    fn parse(self) -> Result<CandleRequest, AppError> {
        validate_market(&self.market)?;
        let venue = Venue::from_str(&self.venue)?;
        let interval = Interval::from_str(&self.interval)?;
        validate_window(self.from, self.to, self.limit, interval)?;
        Ok(CandleRequest {
            venue,
            market: self.market,
            interval,
            from: self.from,
            to: self.to,
            limit: self.limit,
        })
    }
}

#[derive(Serialize)]
struct CandleResponse {
    venue: String,
    market: String,
    interval: String,
    count: usize,
    candles: Vec<Candle>,
}

async fn candles(
    State(state): State<AppState>,
    Query(query): Query<CandleQuery>,
) -> Result<Json<CandleResponse>, AppError> {
    let request = query.parse()?;
    let _permit = acquire(&state).await?;
    let data = state.service.candles(request.clone()).await?;
    let response = CandleResponse {
        venue: request.venue.id().into(),
        market: request.market,
        interval: request.interval.name.into(),
        count: data.len(),
        candles: data.as_ref().clone(),
    };
    Ok(Json(response))
}

#[derive(Debug, Deserialize)]
struct CompareQuery {
    left_venue: String,
    left_market: String,
    right_venue: String,
    right_market: String,
    interval: String,
    from: i64,
    to: i64,
    #[serde(default = "default_candle_limit")]
    limit: usize,
    #[serde(default = "default_scale")]
    scale: f64,
}

fn default_scale() -> f64 {
    10_000.0
}

async fn compare(
    State(state): State<AppState>,
    Query(query): Query<CompareQuery>,
) -> Result<Json<crate::model::ComparisonResponse>, AppError> {
    validate_market(&query.left_market)?;
    validate_market(&query.right_market)?;
    let interval = Interval::from_str(&query.interval)?;
    validate_window(query.from, query.to, query.limit, interval)?;
    let left = CandleRequest {
        venue: Venue::from_str(&query.left_venue)?,
        market: query.left_market,
        interval,
        from: query.from,
        to: query.to,
        limit: query.limit,
    };
    let right = CandleRequest {
        venue: Venue::from_str(&query.right_venue)?,
        market: query.right_market,
        interval,
        from: query.from,
        to: query.to,
        limit: query.limit,
    };
    let _permit = acquire(&state).await?;
    Ok(Json(state.service.compare(left, right, query.scale).await?))
}

async fn acquire(state: &AppState) -> Result<tokio::sync::SemaphorePermit<'_>, AppError> {
    tokio::time::timeout(Duration::from_secs(2), state.permits.acquire())
        .await
        .map_err(|_| AppError::Busy)?
        .map_err(|_| AppError::Internal)
}

fn validate_market(market: &str) -> Result<(), AppError> {
    if market.is_empty()
        || market.len() > 64
        || !market.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/' | b'@')
        })
    {
        return Err(AppError::BadRequest(
            "market must be 1–64 ASCII letters, numbers, or - _ . : / @".into(),
        ));
    }
    Ok(())
}

fn validate_window(from: i64, to: i64, limit: usize, interval: Interval) -> Result<(), AppError> {
    if from < 0 || to <= from {
        return Err(AppError::BadRequest(
            "`to` must be later than non-negative `from` (Unix milliseconds)".into(),
        ));
    }
    if to - from > MAX_RANGE_MS {
        return Err(AppError::BadRequest(
            "requested range cannot exceed 366 days".into(),
        ));
    }
    if limit == 0 || limit > MAX_LIMIT {
        return Err(AppError::BadRequest(format!(
            "limit must be between 1 and {MAX_LIMIT}"
        )));
    }
    if (to - from) / interval.millis > 20_000 {
        return Err(AppError::BadRequest(
            "range contains more than 20,000 source intervals; select a wider interval".into(),
        ));
    }
    Ok(())
}

async fn openapi() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        include_str!("../web/openapi.json"),
    )
}

async fn api_docs() -> Response {
    axum::response::Html(include_str!("../web/docs.html")).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn market_validation_rejects_url_injection() {
        assert!(validate_market("BTCUSDT").is_ok());
        assert!(validate_market("xyz:TSLA").is_ok());
        assert!(validate_market("../secret?x=1").is_err());
    }

    #[test]
    fn window_validation_is_bounded() {
        let interval: Interval = "1m".parse().unwrap();
        assert!(validate_window(0, 60_000, 100, interval).is_ok());
        assert!(validate_window(10, 0, 100, interval).is_err());
        assert!(validate_window(0, MAX_RANGE_MS + 1, 100, interval).is_err());
        assert!(validate_window(0, 60_000, MAX_LIMIT + 1, interval).is_err());
    }

    #[test]
    fn ticker_search_matches_canonical_and_native_notation() {
        let market = Market {
            symbol: "BTC-USDT-SWAP".into(),
            base: "btc".into(),
            quote: "usdt".into(),
            active: true,
        };
        assert!(market_matches(&market, &compact_ticker("BTC/USDT")));
        assert!(market_matches(&market, &compact_ticker("btc-usdt-swap")));
        assert!(!market_matches(&market, &compact_ticker("ETH/USDT")));
        assert_eq!(
            search_rank(
                &market.symbol,
                &market.normalized_symbol(),
                &market.base,
                &compact_ticker("BTC/USDT"),
            ),
            0
        );
    }

    #[test]
    fn ticker_search_parameters_are_bounded() {
        assert!(validate_search("WLFI/USDT", 100).is_ok());
        assert!(validate_search("BTC\nUSDT", 100).is_err());
        assert!(validate_search("币", 100).is_err());
        assert!(validate_search("///", 100).is_err());
        assert!(validate_search(&"x".repeat(65), 100).is_err());
        assert!(validate_search("BTC", 0).is_err());
        assert!(validate_search("BTC", 1001).is_err());
    }

    #[test]
    fn openapi_contract_covers_every_venue_and_comparison_field() {
        let spec: Value = serde_json::from_str(include_str!("../web/openapi.json")).unwrap();
        assert!(spec.pointer("/paths/~1api~1v1~1tickers/get").is_some());
        let documented_venues = spec
            .pointer("/components/schemas/Venue/enum")
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(documented_venues.len(), Venue::ALL.len());
        for venue in Venue::ALL {
            assert!(documented_venues.iter().any(|value| value == venue.id()));
        }

        let market_required = spec
            .pointer("/components/schemas/Market/required")
            .and_then(Value::as_array)
            .unwrap();
        assert!(
            market_required
                .iter()
                .any(|value| value == "normalized_symbol")
        );

        let required = spec
            .pointer("/components/schemas/ComparisonResponse/required")
            .and_then(Value::as_array)
            .unwrap();
        for field in [
            "formula",
            "unit",
            "scale",
            "interval",
            "approximation",
            "candles",
            "stats",
            "matched_candles",
            "dropped_left",
            "dropped_right",
        ] {
            assert!(required.iter().any(|value| value == field));
        }
    }
}
