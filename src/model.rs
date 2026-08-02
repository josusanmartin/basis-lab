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
