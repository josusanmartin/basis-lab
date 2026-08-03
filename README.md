# Basis Lab

[![Deploy to Render](https://render.com/images/deploy-to-render-button.svg)](https://render.com/deploy?repo=https%3A%2F%2Fgithub.com%2Fjosusanmartin%2Fbasis-lab)

Basis Lab is a cross-venue OHLC premium/discount explorer for arbitrage research. It reproduces expressions such as:

```text
(BYBIT:WLFIUSDT.P / MEXC:WLFIUSDT - 1) × 10000
```

without downloading trades. A Rust service fetches venue candles concurrently, normalizes them, joins exact opening timestamps, and returns a synthetic candlestick series in basis points. The same service hosts a responsive, dependency-free canvas UI and a zero-auth JSON API.

> Research tooling only. A visible spread is not executable profit. Fees, funding, borrow, slippage, latency, transfer constraints, and fill risk are not modeled.

## Hosted preview

The responsive public UI is published at <https://josusanmartin.com/basis-lab/> and uses the live Rust API at <https://basis-lab-sg.onrender.com>. Use `?static_demo=1` for the labeled illustrative fallback, or `?api_base=https://your-api.example` to point the same UI at another deployment.

The Render service also hosts the complete UI and agent-callable API on one origin. Self-host with the included `render.yaml`, `fly.toml`, Dockerfile, or Compose service.

## Venue coverage

| Adapter | Market |
|---|---|
| Binance | Spot, USDT perpetual |
| Bybit | Spot, linear perpetual |
| Hyperliquid | Perpetual |
| Lighter | Perpetual |
| Aster | Perpetual |
| Ondo Perps | Perpetual |
| MEXC | Spot, perpetual |
| OKX | Spot, perpetual swap |

All adapters use public, unauthenticated market-data APIs. Symbols retain each venue's native format—for example `BTCUSDT`, `BTC_USDT`, `BTC-USDT-SWAP`, `BTC`, `xyz:SPCX`, or `BTC-USD.P`. Hyperliquid discovery dynamically enumerates the default perpetual universe and every active HIP-3 perp DEX. Use the markets endpoint to discover them.

## Run locally

Requires Rust 1.85 or newer.

```bash
cargo run --release
```

Open <http://localhost:8080>. Configuration:

- `PORT` — listen port, default `8080`
- `MAX_UPSTREAM_CONCURRENCY` — maximum active API request groups, default `64`
- `RUST_LOG` — tracing filter, default `basis_lab=info,tower_http=info`

Or use Docker:

```bash
docker compose up --build
```

Or run the signed release image published from `main`:

```bash
docker run --rm -p 8080:8080 ghcr.io/josusanmartin/basis-lab:latest
```

## API

Interactive examples are at `/docs`; the machine-readable contract is `/openapi.json`.

```bash
NOW=$(date +%s%3N)
FROM=$((NOW - 7 * 86400000))
curl "http://localhost:8080/api/v1/compare?left_venue=bybit_perp&left_market=WLFIUSDT&right_venue=mexc_perp&right_market=WLFI_USDT&interval=1h&from=$FROM&to=$NOW&limit=170&scale=10000"
```

Endpoints:

- `GET /api/v1/health`
- `GET /api/v1/venues`
- `GET /api/v1/markets?venue=bybit_perp&query=WLFI%2FUSDT&limit=100`
- `GET /api/v1/tickers?query=WLFI%2FUSDT&limit=100`
- `GET /api/v1/tickers/suggest?source_venue=ondo_perp&source_symbol=SPCX-USD.P&target_venue=hyperliquid_perp`
- `GET /api/v1/candles?venue=...&market=...&interval=...&from=...&to=...&limit=...`
- `GET /api/v1/compare?left_venue=...&left_market=...&right_venue=...&right_market=...&interval=...&from=...&to=...&limit=...&scale=10000`

Limits are enforced before upstream calls: 1,500 output candles, 366 calendar days, 20,000 source intervals, market identifiers up to 64 safe ASCII characters, a 12 MiB response cap, and a bounded concurrency queue. Successful source candles are cached for 15 seconds; native and normalized ticker catalogs for five minutes. Searches match either venue notation (`BTC-USDT-SWAP`) or canonical notation (`BTC/USDT`) and are relevance-ranked. Every cache has a fixed entry capacity.

## Candle math

For paired source candles `A` and `B`:

```text
open  = (A.open  / B.open  - 1) × scale
close = (A.close / B.close - 1) × scale
high  = (A.high  / B.low   - 1) × scale
low   = (A.low   / B.high  - 1) × scale
```

With `scale=10000`, values are basis points. Comparisons require matching canonical base assets; the suggestion endpoint ranks equivalent native symbols (including both the exact Hyperliquid `mkts:US500` match and the `xyz:SP500` index alias for Ondo `US500`) and flags contract-size aliases that need multiplier review. `SPX` remains distinct because it is not an S&P 500 index market on Hyperliquid. The high/low calculation is a conservative OHLC envelope, not a claim that venue extremes happened simultaneously. Deriving exact synthetic extremes would require synchronized trades or finer-grained bars. Only candles with identical normalized opening timestamps are joined; dropped counts are included in each response.

The browser defaults to one-minute candles over 24 hours. Its 48- and 72-hour views load in daily chunks; longer one-minute selections show the latest 72 hours available. If a venue imposes a smaller upstream bar window, the adapter returns its newest supported subset instead of failing the comparison. When venues supply volume, the comparison preserves each side separately as `left_volume` and `right_volume`. The chart uses separate stacked A and B volume panes, each normalized independently within the visible window, while tooltips retain the raw values.

## Verification

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
npm ci
npx playwright install chromium
npm run test:e2e
```

The deterministic E2E suite boots the Rust binary, loads a Bybit/MEXC WLFI fixture through the UI, validates chart and analytics output, swaps legs, changes intervals, checks the API documentation and static-preview fallback, and repeats core checks at a mobile viewport. Because venue adapters call external APIs, the 12-adapter live smoke matrix is intentionally separate from deterministic unit and browser tests.

## Production notes

- Rustls avoids an OpenSSL runtime dependency.
- Graceful SIGINT/SIGTERM shutdown is enabled.
- Upstream requests have connect and total deadlines with a small idle pool.
- In-flight requests are canceled by the UI when superseded; refresh pauses in background tabs.
- The chart caps device pixel ratio at 2 to avoid oversized backing buffers on dense displays.
- `Dockerfile` uses an unprivileged runtime user and a stripped, LTO release binary.
- Every `main` build publishes multi-attested OCI tags (`latest`, full commit SHA) to GitHub Container Registry with an SBOM and build provenance.
- `render.yaml` and `fly.toml` are included as deployment options.

## License

MIT
