# Verified market aliases

Checked against live catalogs and venue specifications on 2026-09-08.
Native symbols are preserved for candle requests and WebSocket subscriptions.

| Hyperliquid native market | Equivalent base | Verified price units | Example counterpart |
| --- | --- | --- | --- |
| `io:ANTH` | `ANTHROPIC` | Billions of USD market cap / USD per reference share with 1 billion reference shares: numerically 1:1 | Aster `ANTHROPICUSDT` |
| `io:OAI` | `OPENAI` | Same 1 billion reference-share convention | Aster `OPENAIUSDT` |
| `xyz:GOLD` | `XAU` | USD per troy ounce of gold | Lighter `XAU` |
| `xyz:SILVER` | `XAG` | USD per troy ounce of silver | Lighter `XAG` |
| `xyz:PLATINUM` | `XPT` | USD per troy ounce of platinum | Lighter `XPT` |
| `xyz:PALLADIUM` | `XPD` | USD per troy ounce of palladium | Lighter `XPD` |
| `xyz:COPPER` | `XCU` | USD per pound of high-grade copper | Lighter `XCU` |

Sources:

- [Entropy asset directory](https://docs.entropy.io/asset-directory/pre-ipo-assets) and [pre-IPO denomination](https://docs.entropy.io/market-types/pre-ipo-perpetuals).
- [Aster pre-IPO specifications and reference share counts](https://docs.asterdex.com/trading/perpetuals/pre-ipo-perpetuals).
- [XYZ contract specification index](https://docs.trade.xyz/consolidated-resources/specification-index).
- [Lighter RWA market specifications](https://docs.lighter.xyz/trading/real-world-assets-rwas/market-specifications).

The six additional aliases after ANTH are scoped to the exact Hyperliquid native
markets above in discovery and browser validation. For example, `other:GOLD`
does not inherit the `xyz:GOLD` alias. The API's normalized base then drives
search, suggestions, and comparison validation.
Ondo's `COPPER-USD.P` is normalized to `XCU` as well, preserving the existing
XYZ/Ondo copper comparison. Its [public contract metadata](https://api.ondoperps.xyz/v1/perps/contracts?sparkline=false)
identifies it as the USD-denominated copper commodity contract.

Matching an underlying and price unit does not imply identical oracle, funding,
settlement, or collateral terms. In particular, copper futures rolls can differ,
and the pre-IPO contracts have different no-listing resolution provisions.
Recheck the mapping if a venue changes its share count or denomination.

## Candidates left unmerged

- `kPEPE` / `1000PEPE` / `PEPE` and similar contracts need explicit unit handling;
  a thousand-token contract cannot be divided directly by a single-token price.
- `WTI` / `CL` and `BRENT` / `BRENTOIL` / `BZ` need venue-specific contract and
  roll checks before extending the mappings.
- Korean equity names (`SMSN`, `SAMSUNGUSD`, `SKHY`, `SKHYNIXUSD`, and related
  symbols) need checks for share class, currency conversion, and unit scaling.
- `XYZ100`, `US100`, and `USTECH` need index methodology checks. `QQQ` and `SPY`
  are ETF shares and must not be treated as numerically interchangeable with
  their indices. `SPX` remains the distinct SPX6900 token.
- FX symbols such as `JPY` / `USDJPY` require quote-direction checks.

These are research candidates, not enabled equivalences.
