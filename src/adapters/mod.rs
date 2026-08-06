use std::collections::HashMap;

use futures::future::join_all;
use reqwest::{Client, Response};
use serde_json::{Value, json};
use url::Url;

use crate::{
    error::AppError,
    model::{Candle, CandleRequest, Interval, Market, Venue},
};

const MAX_UPSTREAM_BYTES: u64 = 12 * 1024 * 1024;

pub async fn fetch_candles(
    client: &Client,
    request: &CandleRequest,
) -> Result<Vec<Candle>, AppError> {
    ensure_interval(request.venue, request.interval)?;
    let raw = match request.venue {
        Venue::BinanceSpot => binance_candles(client, request, false).await?,
        Venue::BinancePerp => binance_candles(client, request, true).await?,
        Venue::BybitSpot => bybit_candles(client, request, true).await?,
        Venue::BybitPerp => bybit_candles(client, request, false).await?,
        Venue::HyperliquidPerp => hyperliquid_candles(client, request).await?,
        Venue::LighterPerp => lighter_candles(client, request).await?,
        Venue::AsterPerp => aster_candles(client, request).await?,
        Venue::OndoPerp => ondo_candles(client, request).await?,
        Venue::MexcSpot => mexc_spot_candles(client, request).await?,
        Venue::MexcPerp => mexc_perp_candles(client, request).await?,
        Venue::OkxSpot => okx_candles(client, request, false).await?,
        Venue::OkxPerp => okx_candles(client, request, true).await?,
    };
    Ok(normalize(raw, request))
}

pub async fn fetch_markets(client: &Client, venue: Venue) -> Result<Vec<Market>, AppError> {
    let mut markets = match venue {
        Venue::BinanceSpot => binance_markets(client, venue, false).await?,
        Venue::BinancePerp => binance_markets(client, venue, true).await?,
        Venue::BybitSpot => bybit_markets(client, venue, false).await?,
        Venue::BybitPerp => bybit_markets(client, venue, true).await?,
        Venue::HyperliquidPerp => hyperliquid_markets(client, venue).await?,
        Venue::LighterPerp => lighter_markets(client, venue).await?,
        Venue::AsterPerp => aster_markets(client, venue).await?,
        Venue::OndoPerp => ondo_markets(client, venue).await?,
        Venue::MexcSpot => mexc_spot_markets(client, venue).await?,
        Venue::MexcPerp => mexc_perp_markets(client, venue).await?,
        Venue::OkxSpot => okx_markets(client, venue, false).await?,
        Venue::OkxPerp => okx_markets(client, venue, true).await?,
    };
    markets.sort_unstable_by(|a, b| a.symbol.cmp(&b.symbol));
    markets.dedup_by(|a, b| a.symbol == b.symbol);
    Ok(markets)
}

fn ensure_interval(venue: Venue, interval: Interval) -> Result<(), AppError> {
    if venue.intervals().contains(&interval.name) {
        Ok(())
    } else {
        Err(AppError::BadRequest(format!(
            "{} does not support interval {}; supported: {}",
            venue.id(),
            interval.name,
            venue.intervals().join(", ")
        )))
    }
}

fn normalize(mut candles: Vec<Candle>, request: &CandleRequest) -> Vec<Candle> {
    for candle in &mut candles {
        candle.time = candle.time.div_euclid(request.interval.millis) * request.interval.millis;
    }
    candles.retain(|candle| {
        candle.time >= request.from && candle.time <= request.to && candle.validate()
    });
    candles.sort_unstable_by_key(|candle| candle.time);
    candles.dedup_by_key(|candle| candle.time);
    if candles.len() > request.limit {
        candles.drain(..candles.len() - request.limit);
    }
    candles
}

async fn read_json(response: Response, venue: Venue) -> Result<Value, AppError> {
    let status = response.status();
    if response
        .content_length()
        .is_some_and(|size| size > MAX_UPSTREAM_BYTES)
    {
        return Err(upstream(venue, "response exceeded the 12 MiB safety limit"));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| upstream(venue, error))?;
    if bytes.len() as u64 > MAX_UPSTREAM_BYTES {
        return Err(upstream(venue, "response exceeded the 12 MiB safety limit"));
    }
    if !status.is_success() {
        let excerpt = String::from_utf8_lossy(&bytes);
        return Err(upstream(
            venue,
            format!(
                "HTTP {status}: {}",
                excerpt.chars().take(240).collect::<String>()
            ),
        ));
    }
    serde_json::from_slice(&bytes).map_err(|error| upstream(venue, error))
}

fn upstream(venue: Venue, error: impl std::fmt::Display) -> AppError {
    AppError::Upstream {
        venue: venue.id().into(),
        message: error.to_string(),
    }
}

fn value_f64(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::String(text) => text.parse().ok(),
        Value::Number(number) => number.as_f64(),
        _ => None,
    }
}

fn value_i64(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::String(text) => text.parse().ok(),
        Value::Number(number) => number.as_i64(),
        _ => None,
    }
}

fn array_candle(row: &Value) -> Option<Candle> {
    let row = row.as_array()?;
    Some(Candle {
        time: value_i64(row.first())?,
        open: value_f64(row.get(1))?,
        high: value_f64(row.get(2))?,
        low: value_f64(row.get(3))?,
        close: value_f64(row.get(4))?,
        volume: value_f64(row.get(5)),
    })
}

async fn get(client: &Client, venue: Venue, url: Url) -> Result<Value, AppError> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| upstream(venue, error))?;
    read_json(response, venue).await
}

async fn binance_candles(
    client: &Client,
    request: &CandleRequest,
    futures: bool,
) -> Result<Vec<Candle>, AppError> {
    let venue = request.venue;
    let base = if futures {
        "https://fapi.binance.com/fapi/v1/klines"
    } else {
        "https://data-api.binance.vision/api/v3/klines"
    };
    let mut url = Url::parse(base).expect("static URL");
    url.query_pairs_mut()
        .append_pair("symbol", &request.market)
        .append_pair("interval", request.interval.name)
        .append_pair("startTime", &request.from.to_string())
        .append_pair("endTime", &request.to.to_string())
        .append_pair(
            "limit",
            &request
                .limit
                .min(if futures { 1500 } else { 1000 })
                .to_string(),
        );
    let value = get(client, venue, url).await?;
    value
        .as_array()
        .ok_or_else(|| upstream(venue, "unexpected candle response"))?
        .iter()
        .map(|row| array_candle(row).ok_or_else(|| upstream(venue, "malformed candle")))
        .collect()
}

async fn binance_markets(
    client: &Client,
    venue: Venue,
    futures: bool,
) -> Result<Vec<Market>, AppError> {
    if !futures {
        // Spot exchangeInfo is currently larger than our upstream response safety cap. The
        // compact public ticker list is enough for discovery; quote assets are inferred from
        // Binance's conventional symbol suffixes.
        let value = get(
            client,
            venue,
            Url::parse("https://data-api.binance.vision/api/v3/ticker/price").unwrap(),
        )
        .await?;
        let rows = value
            .as_array()
            .ok_or_else(|| upstream(venue, "missing tickers"))?;
        return Ok(rows
            .iter()
            .filter_map(|row| {
                let symbol = row["symbol"].as_str()?;
                let (base, quote) = infer_pair(symbol);
                Some(Market {
                    symbol: symbol.into(),
                    base,
                    quote,
                    active: true,
                })
            })
            .collect());
    }
    let endpoint = if futures {
        "https://fapi.binance.com/fapi/v1/exchangeInfo"
    } else {
        "https://data-api.binance.vision/api/v3/exchangeInfo"
    };
    let value = get(client, venue, Url::parse(endpoint).unwrap()).await?;
    let symbols = value["symbols"]
        .as_array()
        .ok_or_else(|| upstream(venue, "missing symbols"))?;
    Ok(symbols
        .iter()
        .filter_map(|row| {
            let symbol = row["symbol"].as_str()?;
            let status = row["status"].as_str().unwrap_or_default();
            let perpetual = binance_perpetual_contract(row["contractType"].as_str());
            (perpetual || !futures).then(|| Market {
                symbol: symbol.into(),
                base: row["baseAsset"].as_str().unwrap_or_default().into(),
                quote: row["quoteAsset"].as_str().unwrap_or_default().into(),
                active: matches!(status, "TRADING"),
            })
        })
        .collect())
}

fn binance_perpetual_contract(contract_type: Option<&str>) -> bool {
    matches!(
        contract_type.unwrap_or("PERPETUAL"),
        "PERPETUAL" | "TRADIFI_PERPETUAL"
    )
}

fn infer_pair(symbol: &str) -> (String, String) {
    const QUOTES: [&str; 12] = [
        "FDUSD", "USDT", "USDC", "TUSD", "BUSD", "USDP", "DAI", "BTC", "ETH", "BNB", "EUR", "TRY",
    ];
    for quote in QUOTES {
        if let Some(base) = symbol.strip_suffix(quote)
            && !base.is_empty()
        {
            return (base.into(), quote.into());
        }
    }
    (symbol.into(), String::new())
}

fn bybit_interval(interval: Interval) -> &'static str {
    match interval.name {
        "1m" => "1",
        "3m" => "3",
        "5m" => "5",
        "15m" => "15",
        "30m" => "30",
        "1h" => "60",
        "2h" => "120",
        "4h" => "240",
        "1d" => "D",
        _ => unreachable!(),
    }
}

async fn bybit_candles(
    client: &Client,
    request: &CandleRequest,
    spot: bool,
) -> Result<Vec<Candle>, AppError> {
    let venue = request.venue;
    let mut url = Url::parse("https://api.bybit.com/v5/market/kline").unwrap();
    url.query_pairs_mut()
        .append_pair("category", if spot { "spot" } else { "linear" })
        .append_pair("symbol", &request.market)
        .append_pair("interval", bybit_interval(request.interval))
        .append_pair("start", &request.from.to_string())
        .append_pair("end", &request.to.to_string())
        .append_pair("limit", &request.limit.min(1000).to_string());
    let value = get(client, venue, url).await?;
    if value["retCode"].as_i64().unwrap_or(0) != 0 {
        return Err(upstream(
            venue,
            value["retMsg"].as_str().unwrap_or("unknown error"),
        ));
    }
    value["result"]["list"]
        .as_array()
        .ok_or_else(|| upstream(venue, "missing candle list"))?
        .iter()
        .map(|row| array_candle(row).ok_or_else(|| upstream(venue, "malformed candle")))
        .collect()
}

async fn bybit_markets(client: &Client, venue: Venue, spot: bool) -> Result<Vec<Market>, AppError> {
    let mut output = Vec::new();
    let mut cursor = String::new();
    loop {
        let mut url = Url::parse("https://api.bybit.com/v5/market/instruments-info").unwrap();
        url.query_pairs_mut()
            .append_pair("category", if spot { "spot" } else { "linear" })
            .append_pair("limit", "1000");
        if !cursor.is_empty() {
            url.query_pairs_mut().append_pair("cursor", &cursor);
        }
        let value = get(client, venue, url).await?;
        let rows = value["result"]["list"]
            .as_array()
            .ok_or_else(|| upstream(venue, "missing instruments"))?;
        output.extend(rows.iter().filter_map(|row| {
            if !spot && row["contractType"].as_str()? != "LinearPerpetual" {
                return None;
            }
            Some(Market {
                symbol: row["symbol"].as_str()?.into(),
                base: row["baseCoin"].as_str().unwrap_or_default().into(),
                quote: row["quoteCoin"].as_str().unwrap_or_default().into(),
                active: row["status"].as_str() == Some("Trading"),
            })
        }));
        cursor = value["result"]["nextPageCursor"]
            .as_str()
            .unwrap_or_default()
            .into();
        if cursor.is_empty() || spot {
            break;
        }
    }
    Ok(output)
}

async fn hyperliquid_candles(
    client: &Client,
    request: &CandleRequest,
) -> Result<Vec<Candle>, AppError> {
    let venue = request.venue;
    let response = client
        .post("https://api.hyperliquid.xyz/info")
        .json(&json!({"type":"candleSnapshot","req":{"coin":request.market,"interval":request.interval.name,"startTime":request.from,"endTime":request.to}}))
        .send()
        .await
        .map_err(|error| upstream(venue, error))?;
    let value = read_json(response, venue).await?;
    value
        .as_array()
        .ok_or_else(|| upstream(venue, "unexpected candle response"))?
        .iter()
        .map(|row| {
            Ok(Candle {
                time: value_i64(row.get("t")).ok_or_else(|| upstream(venue, "missing time"))?,
                open: value_f64(row.get("o")).ok_or_else(|| upstream(venue, "missing open"))?,
                high: value_f64(row.get("h")).ok_or_else(|| upstream(venue, "missing high"))?,
                low: value_f64(row.get("l")).ok_or_else(|| upstream(venue, "missing low"))?,
                close: value_f64(row.get("c")).ok_or_else(|| upstream(venue, "missing close"))?,
                volume: value_f64(row.get("v")),
            })
        })
        .collect()
}

async fn hyperliquid_markets(client: &Client, venue: Venue) -> Result<Vec<Market>, AppError> {
    let mut markets = hyperliquid_market_universe(client, venue, None).await?;
    let response = client
        .post("https://api.hyperliquid.xyz/info")
        .json(&json!({"type":"perpDexs"}))
        .send()
        .await
        .map_err(|error| upstream(venue, error))?;
    let value = read_json(response, venue).await?;
    let dexes: Vec<String> = value
        .as_array()
        .ok_or_else(|| upstream(venue, "unexpected perpetual DEX response"))?
        .iter()
        .filter_map(|row| row.get("name")?.as_str().map(str::to_owned))
        .collect();
    let hip3 = join_all(
        dexes
            .iter()
            .map(|dex| hyperliquid_market_universe(client, venue, Some(dex))),
    )
    .await;
    for mut universe in hip3.into_iter().flatten() {
        markets.append(&mut universe);
    }
    Ok(markets)
}

async fn hyperliquid_market_universe(
    client: &Client,
    venue: Venue,
    dex: Option<&str>,
) -> Result<Vec<Market>, AppError> {
    let body = match dex {
        Some(dex) => json!({"type":"meta","dex":dex}),
        None => json!({"type":"meta"}),
    };
    let response = client
        .post("https://api.hyperliquid.xyz/info")
        .json(&body)
        .send()
        .await
        .map_err(|error| upstream(venue, error))?;
    parse_hyperliquid_markets(&read_json(response, venue).await?, venue)
}

fn parse_hyperliquid_markets(value: &Value, venue: Venue) -> Result<Vec<Market>, AppError> {
    let rows = value["universe"]
        .as_array()
        .ok_or_else(|| upstream(venue, "missing universe"))?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            let symbol = row["name"].as_str()?;
            let base = symbol.rsplit_once(':').map_or(symbol, |(_, base)| base);
            Some(Market {
                symbol: symbol.into(),
                base: base.into(),
                quote: "USD".into(),
                active: !row["isDelisted"].as_bool().unwrap_or(false),
            })
        })
        .collect())
}

async fn lighter_market_map(client: &Client, venue: Venue) -> Result<Vec<(Market, i64)>, AppError> {
    let value = get(
        client,
        venue,
        Url::parse("https://mainnet.zklighter.elliot.ai/api/v1/orderBooks").unwrap(),
    )
    .await?;
    let rows = value["order_books"]
        .as_array()
        .ok_or_else(|| upstream(venue, "missing order books"))?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            let symbol = row["symbol"].as_str()?;
            let id = row["market_id"].as_i64()?;
            Some((
                Market {
                    symbol: symbol.into(),
                    base: symbol.split('/').next().unwrap_or(symbol).into(),
                    quote: symbol.split('/').nth(1).unwrap_or("USDC").into(),
                    active: row["status"].as_str() == Some("active"),
                },
                id,
            ))
        })
        .collect())
}

async fn lighter_candles(
    client: &Client,
    request: &CandleRequest,
) -> Result<Vec<Candle>, AppError> {
    let venue = request.venue;
    let map = lighter_market_map(client, venue).await?;
    let market_id = map
        .iter()
        .find(|(market, _)| market.symbol.eq_ignore_ascii_case(&request.market))
        .map(|(_, id)| *id)
        .ok_or_else(|| {
            AppError::BadRequest(format!("unknown Lighter market `{}`", request.market))
        })?;
    let mut url = Url::parse("https://mainnet.zklighter.elliot.ai/api/v1/candles").unwrap();
    url.query_pairs_mut()
        .append_pair("market_id", &market_id.to_string())
        .append_pair("resolution", request.interval.name)
        .append_pair("start_timestamp", &request.from.to_string())
        .append_pair("end_timestamp", &request.to.to_string())
        .append_pair("count_back", &request.limit.min(500).to_string());
    let value = get(client, venue, url).await?;
    let rows = value["c"]
        .as_array()
        .ok_or_else(|| upstream(venue, "missing candles"))?;
    rows.iter()
        .map(|row| {
            Ok(Candle {
                time: value_i64(row.get("t")).ok_or_else(|| upstream(venue, "missing time"))?,
                open: value_f64(row.get("o")).ok_or_else(|| upstream(venue, "missing open"))?,
                high: value_f64(row.get("h")).ok_or_else(|| upstream(venue, "missing high"))?,
                low: value_f64(row.get("l")).ok_or_else(|| upstream(venue, "missing low"))?,
                close: value_f64(row.get("c")).ok_or_else(|| upstream(venue, "missing close"))?,
                volume: value_f64(row.get("v")),
            })
        })
        .collect()
}

async fn lighter_markets(client: &Client, venue: Venue) -> Result<Vec<Market>, AppError> {
    Ok(lighter_market_map(client, venue)
        .await?
        .into_iter()
        .map(|item| item.0)
        .collect())
}

async fn aster_candles(client: &Client, request: &CandleRequest) -> Result<Vec<Candle>, AppError> {
    let venue = request.venue;
    let mut url = Url::parse("https://fapi.asterdex.com/fapi/v1/klines").unwrap();
    url.query_pairs_mut()
        .append_pair("symbol", &request.market)
        .append_pair("interval", request.interval.name)
        .append_pair("startTime", &request.from.to_string())
        .append_pair("endTime", &request.to.to_string())
        .append_pair("limit", &request.limit.min(1500).to_string());
    let value = get(client, venue, url).await?;
    value
        .as_array()
        .ok_or_else(|| upstream(venue, "unexpected candle response"))?
        .iter()
        .map(|row| array_candle(row).ok_or_else(|| upstream(venue, "malformed candle")))
        .collect()
}

async fn aster_markets(client: &Client, venue: Venue) -> Result<Vec<Market>, AppError> {
    let value = get(
        client,
        venue,
        Url::parse("https://fapi.asterdex.com/fapi/v1/exchangeInfo").unwrap(),
    )
    .await?;
    let rows = value["symbols"]
        .as_array()
        .ok_or_else(|| upstream(venue, "missing symbols"))?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            Some(Market {
                symbol: row["symbol"].as_str()?.into(),
                base: row["baseAsset"].as_str().unwrap_or_default().into(),
                quote: row["quoteAsset"].as_str().unwrap_or_default().into(),
                active: row["status"].as_str() == Some("TRADING"),
            })
        })
        .collect())
}

fn ondo_resolution(interval: Interval) -> &'static str {
    match interval.name {
        "1m" => "1",
        "3m" => "3",
        "5m" => "5",
        "15m" => "15",
        "30m" => "30",
        "1h" => "60",
        "2h" => "120",
        "4h" => "240",
        "1d" => "1D",
        _ => unreachable!(),
    }
}

const ONDO_MAX_BARS: i64 = 4_900;

fn ondo_request_from(request: &CandleRequest) -> i64 {
    let maximum_window = request
        .interval
        .millis
        .saturating_mul(ONDO_MAX_BARS.saturating_sub(1));
    request.from.max(request.to.saturating_sub(maximum_window))
}

async fn ondo_candles(client: &Client, request: &CandleRequest) -> Result<Vec<Candle>, AppError> {
    let venue = request.venue;
    // The object-shaped /candles route requires auth. The documented UDF history route is
    // public and provides the same venue OHLCV data in parallel arrays.
    let mut url = Url::parse("https://api.ondoperps.xyz/v1/perps/history").unwrap();
    let udf_symbol = request.market.replace('-', "");
    url.query_pairs_mut()
        .append_pair("symbol", &udf_symbol)
        .append_pair("resolution", ondo_resolution(request.interval))
        .append_pair("from", &(ondo_request_from(request) / 1000).to_string())
        .append_pair("to", &(request.to / 1000).to_string());
    let value = get(client, venue, url).await?;
    if value["s"].as_str() == Some("error") {
        return Err(upstream(
            venue,
            value["errmsg"].as_str().unwrap_or("unknown error"),
        ));
    }
    let times = value["t"]
        .as_array()
        .ok_or_else(|| upstream(venue, "missing candle timestamps"))?;
    let mut output = Vec::with_capacity(times.len());
    for index in 0..times.len() {
        output.push(Candle {
            time: value_i64(times.get(index)).ok_or_else(|| upstream(venue, "missing time"))?
                * 1000,
            open: value_f64(value["o"].get(index))
                .ok_or_else(|| upstream(venue, "missing open"))?,
            high: value_f64(value["h"].get(index))
                .ok_or_else(|| upstream(venue, "missing high"))?,
            low: value_f64(value["l"].get(index)).ok_or_else(|| upstream(venue, "missing low"))?,
            close: value_f64(value["c"].get(index))
                .ok_or_else(|| upstream(venue, "missing close"))?,
            volume: value_f64(value["v"].get(index)),
        });
    }
    Ok(output)
}

async fn ondo_markets(client: &Client, venue: Venue) -> Result<Vec<Market>, AppError> {
    let value = get(
        client,
        venue,
        Url::parse("https://api.ondoperps.xyz/v1/perps/contracts?sparkline=false").unwrap(),
    )
    .await?;
    let rows = value["result"]
        .as_array()
        .ok_or_else(|| upstream(venue, "missing contracts"))?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            Some(Market {
                symbol: row["market"].as_str()?.into(),
                base: row["baseCurrency"].as_str().unwrap_or_default().into(),
                quote: row["quoteCurrency"].as_str().unwrap_or_default().into(),
                active: !row["disabled"].as_bool().unwrap_or(false),
            })
        })
        .collect())
}

async fn mexc_spot_candles(
    client: &Client,
    request: &CandleRequest,
) -> Result<Vec<Candle>, AppError> {
    let venue = request.venue;
    let mut url = Url::parse("https://api.mexc.com/api/v3/klines").unwrap();
    let interval = match request.interval.name {
        "1h" => "60m",
        other => other,
    };
    url.query_pairs_mut()
        .append_pair("symbol", &request.market)
        .append_pair("interval", interval)
        .append_pair("startTime", &request.from.to_string())
        .append_pair("endTime", &request.to.to_string())
        .append_pair("limit", &request.limit.min(1000).to_string());
    let value = get(client, venue, url).await?;
    value
        .as_array()
        .ok_or_else(|| upstream(venue, "unexpected candle response"))?
        .iter()
        .map(|row| array_candle(row).ok_or_else(|| upstream(venue, "malformed candle")))
        .collect()
}

async fn mexc_spot_markets(client: &Client, venue: Venue) -> Result<Vec<Market>, AppError> {
    let value = get(
        client,
        venue,
        Url::parse("https://api.mexc.com/api/v3/exchangeInfo").unwrap(),
    )
    .await?;
    let rows = value["symbols"]
        .as_array()
        .ok_or_else(|| upstream(venue, "missing symbols"))?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            Some(Market {
                symbol: row["symbol"].as_str()?.into(),
                base: row["baseAsset"].as_str().unwrap_or_default().into(),
                quote: row["quoteAsset"].as_str().unwrap_or_default().into(),
                active: row["status"].as_str() == Some("1")
                    || row["status"].as_str() == Some("ENABLED"),
            })
        })
        .collect())
}

fn mexc_interval(interval: Interval) -> &'static str {
    match interval.name {
        "1m" => "Min1",
        "5m" => "Min5",
        "15m" => "Min15",
        "30m" => "Min30",
        "1h" => "Min60",
        "4h" => "Hour4",
        "1d" => "Day1",
        _ => unreachable!(),
    }
}

async fn mexc_perp_candles(
    client: &Client,
    request: &CandleRequest,
) -> Result<Vec<Candle>, AppError> {
    let venue = request.venue;
    let symbol = request.market.replace('-', "_");
    let mut url = Url::parse(&format!(
        "https://contract.mexc.com/api/v1/contract/kline/{symbol}"
    ))
    .unwrap();
    url.query_pairs_mut()
        .append_pair("interval", mexc_interval(request.interval))
        .append_pair("start", &(request.from / 1000).to_string())
        .append_pair("end", &(request.to / 1000).to_string());
    let value = get(client, venue, url).await?;
    if value["success"].as_bool() == Some(false) {
        return Err(upstream(
            venue,
            value["message"].as_str().unwrap_or("unknown error"),
        ));
    }
    let data = &value["data"];
    let times = data["time"]
        .as_array()
        .ok_or_else(|| upstream(venue, "missing times"))?;
    let mut output = Vec::with_capacity(times.len());
    for index in 0..times.len() {
        output.push(Candle {
            time: value_i64(times.get(index)).ok_or_else(|| upstream(venue, "missing time"))?
                * 1000,
            open: value_f64(data["open"].get(index))
                .ok_or_else(|| upstream(venue, "missing open"))?,
            high: value_f64(data["high"].get(index))
                .ok_or_else(|| upstream(venue, "missing high"))?,
            low: value_f64(data["low"].get(index)).ok_or_else(|| upstream(venue, "missing low"))?,
            close: value_f64(data["close"].get(index))
                .ok_or_else(|| upstream(venue, "missing close"))?,
            volume: value_f64(data["vol"].get(index)),
        });
    }
    Ok(output)
}

async fn mexc_perp_markets(client: &Client, venue: Venue) -> Result<Vec<Market>, AppError> {
    let value = get(
        client,
        venue,
        Url::parse("https://contract.mexc.com/api/v1/contract/detail").unwrap(),
    )
    .await?;
    let rows = value["data"]
        .as_array()
        .ok_or_else(|| upstream(venue, "missing contracts"))?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            Some(Market {
                symbol: row["symbol"].as_str()?.into(),
                base: row["baseCoin"].as_str().unwrap_or_default().into(),
                quote: row["quoteCoin"].as_str().unwrap_or_default().into(),
                active: row["state"].as_i64() == Some(0),
            })
        })
        .collect())
}

fn okx_bar(interval: Interval) -> &'static str {
    match interval.name {
        "1m" => "1m",
        "3m" => "3m",
        "5m" => "5m",
        "15m" => "15m",
        "30m" => "30m",
        "1h" => "1H",
        "2h" => "2H",
        "4h" => "4H",
        "1d" => "1Dutc",
        _ => unreachable!(),
    }
}

async fn okx_candles(
    client: &Client,
    request: &CandleRequest,
    _spot: bool,
) -> Result<Vec<Candle>, AppError> {
    let venue = request.venue;
    let mut all: HashMap<i64, Candle> = HashMap::new();
    let mut before = request.to + 1;
    while all.len() < request.limit && before > request.from {
        let mut url = Url::parse("https://www.okx.com/api/v5/market/history-candles").unwrap();
        url.query_pairs_mut()
            .append_pair("instId", &request.market)
            .append_pair("bar", okx_bar(request.interval))
            .append_pair("after", &before.to_string())
            .append_pair(
                "limit",
                &request.limit.saturating_sub(all.len()).min(300).to_string(),
            );
        let value = get(client, venue, url).await?;
        if value["code"].as_str().unwrap_or("0") != "0" {
            return Err(upstream(
                venue,
                value["msg"].as_str().unwrap_or("unknown error"),
            ));
        }
        let rows = value["data"]
            .as_array()
            .ok_or_else(|| upstream(venue, "missing candles"))?;
        if rows.is_empty() {
            break;
        }
        let mut oldest = before;
        for row in rows {
            let candle = array_candle(row).ok_or_else(|| upstream(venue, "malformed candle"))?;
            oldest = oldest.min(candle.time);
            all.insert(candle.time, candle);
        }
        if oldest >= before || oldest <= request.from {
            break;
        }
        before = oldest;
    }
    Ok(all.into_values().collect())
}

async fn okx_markets(client: &Client, venue: Venue, spot: bool) -> Result<Vec<Market>, AppError> {
    let mut url = Url::parse("https://www.okx.com/api/v5/public/instruments").unwrap();
    url.query_pairs_mut()
        .append_pair("instType", if spot { "SPOT" } else { "SWAP" });
    let value = get(client, venue, url).await?;
    let rows = value["data"]
        .as_array()
        .ok_or_else(|| upstream(venue, "missing instruments"))?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            Some(Market {
                symbol: row["instId"].as_str()?.into(),
                base: row["baseCcy"]
                    .as_str()
                    .or_else(|| row["ctValCcy"].as_str())
                    .unwrap_or_default()
                    .into(),
                quote: row["quoteCcy"]
                    .as_str()
                    .or_else(|| row["settleCcy"].as_str())
                    .unwrap_or_default()
                    .into(),
                active: row["state"].as_str() == Some("live"),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> CandleRequest {
        CandleRequest {
            venue: Venue::BinanceSpot,
            market: "BTCUSDT".into(),
            interval: "1m".parse().unwrap(),
            from: 60_000,
            to: 240_000,
            limit: 2,
        }
    }

    #[test]
    fn normalization_aligns_deduplicates_filters_and_limits() {
        let candles = vec![
            Candle {
                time: 121_000,
                open: 2.0,
                high: 3.0,
                low: 1.0,
                close: 2.0,
                volume: None,
            },
            Candle {
                time: 122_000,
                open: 4.0,
                high: 5.0,
                low: 3.0,
                close: 4.0,
                volume: None,
            },
            Candle {
                time: 181_000,
                open: 3.0,
                high: 4.0,
                low: 2.0,
                close: 3.0,
                volume: None,
            },
            Candle {
                time: 301_000,
                open: 3.0,
                high: 4.0,
                low: 2.0,
                close: 3.0,
                volume: None,
            },
        ];
        let normalized = normalize(candles, &request());
        assert_eq!(normalized.len(), 2);
        assert_eq!(normalized[0].time, 120_000);
        assert_eq!(normalized[1].time, 180_000);
    }

    #[test]
    fn hyperliquid_hip3_markets_preserve_dex_prefix_and_normalize_base() {
        let value = json!({
            "universe": [
                {"name":"xyz:SPCX","isDelisted":false},
                {"name":"xyz:OLD","isDelisted":true}
            ]
        });
        let markets = parse_hyperliquid_markets(&value, Venue::HyperliquidPerp).unwrap();
        assert_eq!(markets[0].symbol, "xyz:SPCX");
        assert_eq!(markets[0].base, "SPCX");
        assert_eq!(markets[0].normalized_symbol(), "SPCX/USD");
        assert!(markets[0].active);
        assert!(!markets[1].active);
    }

    #[test]
    fn binance_discovery_accepts_crypto_and_tradfi_perpetual_contracts() {
        assert!(binance_perpetual_contract(Some("PERPETUAL")));
        assert!(binance_perpetual_contract(Some("TRADIFI_PERPETUAL")));
        assert!(!binance_perpetual_contract(Some("CURRENT_QUARTER")));
    }

    #[test]
    fn ondo_requests_the_latest_supported_window_instead_of_failing() {
        let mut request = request();
        request.venue = Venue::OndoPerp;
        request.from = 0;
        request.to = 7 * 86_400_000;
        assert_eq!(
            ondo_request_from(&request),
            request.to - request.interval.millis * (ONDO_MAX_BARS - 1)
        );
        request.from = request.to - 86_400_000;
        assert_eq!(ondo_request_from(&request), request.from);
    }
}
