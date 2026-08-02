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
use serde_json::{Value, json};
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
    model::{Candle, CandleRequest, Interval, Market, Venue, VenueInfo},
    service::MarketDataService,
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
) -> Result<Json<Value>, AppError> {
    let venue = Venue::from_str(&query.venue)?;
    if query.limit == 0 || query.limit > 1000 {
        return Err(AppError::BadRequest(
            "market limit must be between 1 and 1000".into(),
        ));
    }
    let _permit = acquire(&state).await?;
    let needle = query.query.to_ascii_uppercase();
    let cached = state.service.markets(venue).await?;
    let markets: Vec<Market> = cached
        .iter()
        .filter(|market| {
            market.active
                && (needle.is_empty() || market.symbol.to_ascii_uppercase().contains(&needle))
        })
        .take(query.limit)
        .cloned()
        .collect();
    Ok(Json(json!({ "venue": venue.id(), "data": markets })))
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
    fn openapi_contract_covers_every_venue_and_comparison_field() {
        let spec: Value = serde_json::from_str(include_str!("../web/openapi.json")).unwrap();
        let documented_venues = spec
            .pointer("/components/schemas/Venue/enum")
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(documented_venues.len(), Venue::ALL.len());
        for venue in Venue::ALL {
            assert!(documented_venues.iter().any(|value| value == venue.id()));
        }

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
