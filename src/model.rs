use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::error::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Venue {
    BinanceSpot,
    BinancePerp,
    BybitSpot,
    BybitPerp,
    HyperliquidPerp,
    LighterPerp,
    AsterPerp,
    OndoPerp,
    MexcSpot,
    MexcPerp,
    OkxSpot,
    OkxPerp,
}

impl Venue {
    pub const ALL: [Self; 12] = [
        Self::BinanceSpot,
        Self::BinancePerp,
        Self::BybitSpot,
        Self::BybitPerp,
        Self::HyperliquidPerp,
        Self::LighterPerp,
        Self::AsterPerp,
        Self::OndoPerp,
        Self::MexcSpot,
        Self::MexcPerp,
        Self::OkxSpot,
        Self::OkxPerp,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::BinanceSpot => "binance_spot",
            Self::BinancePerp => "binance_perp",
            Self::BybitSpot => "bybit_spot",
            Self::BybitPerp => "bybit_perp",
            Self::HyperliquidPerp => "hyperliquid_perp",
            Self::LighterPerp => "lighter_perp",
            Self::AsterPerp => "aster_perp",
            Self::OndoPerp => "ondo_perp",
            Self::MexcSpot => "mexc_spot",
            Self::MexcPerp => "mexc_perp",
            Self::OkxSpot => "okx_spot",
            Self::OkxPerp => "okx_perp",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::BinanceSpot => "Binance Spot",
            Self::BinancePerp => "Binance Perpetual",
            Self::BybitSpot => "Bybit Spot",
            Self::BybitPerp => "Bybit Perpetual",
            Self::HyperliquidPerp => "Hyperliquid",
            Self::LighterPerp => "Lighter",
            Self::AsterPerp => "Aster",
            Self::OndoPerp => "Ondo Perps",
            Self::MexcSpot => "MEXC Spot",
            Self::MexcPerp => "MEXC Perpetual",
            Self::OkxSpot => "OKX Spot",
            Self::OkxPerp => "OKX Perpetual",
        }
    }

    pub fn market_type(self) -> &'static str {
        match self {
            Self::BinanceSpot | Self::BybitSpot | Self::MexcSpot | Self::OkxSpot => "spot",
            _ => "perpetual",
        }
    }

    pub fn intervals(self) -> &'static [&'static str] {
        match self {
            Self::LighterPerp => &["1m", "5m", "15m", "30m", "1h", "4h", "1d"],
            Self::MexcPerp => &["1m", "5m", "15m", "30m", "1h", "4h", "1d"],
            Self::MexcSpot => &["1m", "5m", "15m", "30m", "1h", "4h", "1d"],
            _ => &["1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "1d"],
        }
    }
}

impl fmt::Display for Venue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

impl FromStr for Venue {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|venue| venue.id() == value)
            .ok_or_else(|| AppError::BadRequest(format!("unknown venue `{value}`")))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Interval {
    pub name: &'static str,
    pub millis: i64,
}

impl Interval {
    pub const SUPPORTED: [Self; 9] = [
        Self {
            name: "1m",
            millis: 60_000,
        },
        Self {
            name: "3m",
            millis: 180_000,
        },
        Self {
            name: "5m",
            millis: 300_000,
        },
        Self {
            name: "15m",
            millis: 900_000,
        },
        Self {
            name: "30m",
            millis: 1_800_000,
        },
        Self {
            name: "1h",
            millis: 3_600_000,
        },
        Self {
            name: "2h",
            millis: 7_200_000,
        },
        Self {
            name: "4h",
            millis: 14_400_000,
        },
        Self {
            name: "1d",
            millis: 86_400_000,
        },
    ];
}

impl FromStr for Interval {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::SUPPORTED
            .into_iter()
            .find(|interval| interval.name == value)
            .ok_or_else(|| {
                AppError::BadRequest(format!(
                    "unsupported interval `{value}`; use 1m, 3m, 5m, 15m, 30m, 1h, 2h, 4h, or 1d"
                ))
            })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candle {
    /// Candle opening timestamp in Unix milliseconds.
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,
}

impl Candle {
    pub fn validate(&self) -> bool {
        self.time >= 0
            && [self.open, self.high, self.low, self.close]
                .into_iter()
                .all(|number| number.is_finite() && number > 0.0)
            && self.high >= self.open.max(self.close).max(self.low)
            && self.low <= self.open.min(self.close).min(self.high)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Market {
    pub symbol: String,
    pub base: String,
    pub quote: String,
    pub active: bool,
}

impl Market {
    pub fn normalized_symbol(&self) -> String {
        let base = self.base.trim().to_ascii_uppercase();
        let quote = self.quote.trim().to_ascii_uppercase();
        match (base.is_empty(), quote.is_empty()) {
            (false, false) => format!("{base}/{quote}"),
            (false, true) => base,
            (true, false) => quote,
            (true, true) => self.symbol.trim().to_ascii_uppercase(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TickerListing {
    pub normalized_symbol: String,
    pub symbol: String,
    pub base: String,
    pub quote: String,
    pub venue: &'static str,
    pub venue_label: &'static str,
    pub market_type: &'static str,
}

impl TickerListing {
    pub fn from_market(venue: Venue, market: &Market) -> Self {
        Self {
            normalized_symbol: market.normalized_symbol(),
            symbol: market.symbol.clone(),
            base: market.base.trim().to_ascii_uppercase(),
            quote: market.quote.trim().to_ascii_uppercase(),
            venue: venue.id(),
            venue_label: venue.label(),
            market_type: venue.market_type(),
        }
    }

    pub fn search_key(&self) -> String {
        format!(
            "{}{}{}{}{}",
            compact_ticker(&self.normalized_symbol),
            compact_ticker(&self.symbol),
            compact_ticker(&self.base),
            compact_ticker(&self.quote),
            compact_ticker(self.venue_label),
        )
    }
}

pub fn compact_ticker(value: &str) -> String {
    value
        .bytes()
        .filter(|byte| byte.is_ascii_alphanumeric())
        .map(|byte| byte.to_ascii_uppercase() as char)
        .collect()
}

pub fn canonical_asset(value: &str) -> String {
    let compact = compact_ticker(value);
    match compact.as_str() {
        "XBT" | "WBTC" => "BTC".into(),
        "WETH" => "ETH".into(),
        "XDG" => "DOGE".into(),
        _ => compact,
    }
}

pub fn contract_unit_asset(value: &str) -> Option<String> {
    let compact = compact_ticker(value);
    ["1000000", "100000", "10000", "1000"]
        .into_iter()
        .find_map(|prefix| {
            compact
                .strip_prefix(prefix)
                .filter(|asset| asset.len() >= 2)
                .map(canonical_asset)
        })
}

#[derive(Debug, Clone, Serialize)]
pub struct VenueInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub market_type: &'static str,
    pub intervals: &'static [&'static str],
    pub symbol_example: &'static str,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ComparisonCandle {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub left_close: f64,
    pub right_close: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ComparisonStats {
    pub latest: f64,
    pub mean: f64,
    pub standard_deviation: f64,
    pub minimum: f64,
    pub maximum: f64,
    pub z_score: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComparisonResponse {
    pub formula: String,
    pub unit: &'static str,
    pub scale: f64,
    pub interval: String,
    pub approximation: &'static str,
    pub candles: Vec<ComparisonCandle>,
    pub stats: ComparisonStats,
    pub matched_candles: usize,
    pub dropped_left: usize,
    pub dropped_right: usize,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct CandleRequest {
    pub venue: Venue,
    pub market: String,
    pub interval: Interval,
    pub from: i64,
    pub to: i64,
    pub limit: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticker_listing_preserves_native_symbol_and_adds_canonical_pair() {
        let market = Market {
            symbol: "BTC_USDT".into(),
            base: "btc".into(),
            quote: "usdt".into(),
            active: true,
        };
        let ticker = TickerListing::from_market(Venue::MexcPerp, &market);
        assert_eq!(ticker.symbol, "BTC_USDT");
        assert_eq!(ticker.normalized_symbol, "BTC/USDT");
        assert_eq!(ticker.venue, "mexc_perp");
        assert!(ticker.search_key().contains("BTCUSDT"));
        assert_eq!(compact_ticker(" btc-usdt.swap "), "BTCUSDTSWAP");
        assert_eq!(canonical_asset("xbt"), "BTC");
        assert_eq!(canonical_asset("WETH"), "ETH");
        assert_eq!(contract_unit_asset("1000PEPE").as_deref(), Some("PEPE"));
        assert_eq!(contract_unit_asset("1INCH"), None);
    }
}
