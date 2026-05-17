# Superstition — Example Corpus

A realistic synthetic corpus for end-to-end testing of the full pipeline:
corpus generation → agent loop → feed publication → verification.

## Tables

### `erc20_transfers` (100 000 rows, 2024 calendar year)

| Column | Type | Notes |
|---|---|---|
| `block_timestamp` | u64 (Unix seconds) | 2024-01-01 – 2024-12-31 |
| `value_wei` | u64 | log-normal, ~0.01 ETH median |
| `gas_price_gwei` | u64 | log-normal, ~30 gwei median |

**Baked-in signals:**
- Day-of-week: Monday 1.60×, Sunday 0.55× (strong weekday bias)
- Hour-of-day: bimodal — peak at 08-10 UTC (EU morning) and 14-16 UTC (US open)
- `value_wei`: log-normal, heavy-tailed

### `dex_swaps` (40 000 rows, 2024 calendar year)

| Column | Type | Notes |
|---|---|---|
| `block_timestamp` | u64 (Unix seconds) | 2024-01-01 – 2024-12-31 |
| `amount_usd_cents` | u64 | Pareto power-law (α=1.5, min=$10) |
| `fee_bps` | u64 | Categorical: 5 (40%), 30 (50%), 100 (10%) |

**Baked-in signals:**
- Day-of-week: Tuesday/Thursday 1.55×, Sunday 0.40× (different shape from ERC-20)
- Hour-of-day: concentrated at 13-15 UTC (NY open) — single peak, not bimodal
- `amount_usd_cents`: Pareto power-law, median ~$16, heavy right tail
- `fee_bps`: non-uniform categorical (30bps dominates)

## Quick start

```bash
# 1. generate the corpus (one-time)
cargo run -p superstition-corpus --bin gen-example

# 2. run the agent for a few iterations
cargo run --release -p superstition-agent -- \
    --corpus examples/corpus \
    --feed   examples/feed.json \
    --state  examples/state.json \
    --workspace . \
    --iterations 3 \
    --hypotheses 5

# 3. start the feed server (optional, separate terminal)
cargo run --release -p superstition-feed --bin server -- examples/feed.json

# 4. verify a published pattern
cargo run --release -p superstition-feed --bin verify -- \
    --feed examples/feed.json <pattern-id>
```

Or just run the convenience script:

```bash
bash examples/run.sh
```

## Expected findings

The agent should reliably detect (among others):

| Detector family | Signal | Expected result |
|---|---|---|
| `temporal-cyclic` | ERC-20 day-of-week | χ²(6) >> critical, SIGNIFICANT |
| `temporal-cyclic` | DEX day-of-week | χ²(6) significant, different DOW shape |
| `temporal-cyclic` | ERC-20 hour-of-day | χ²(23) significant, bimodal |
| `temporal-cyclic` | DEX hour-of-day | χ²(23) significant, single peak |
| `value-distribution` | ERC-20 value log-normality | KS test, significant |
| `value-distribution` | DEX amount power-law | KS / bootstrap, significant |
| `categorical` | DEX fee-tier distribution | χ²(2) significant (30bps dominant) |

## Corpus ID

The `corpus_id` is the first 16 hex characters of the BLAKE3 hash of all
Parquet bytes (sorted by filename). It is printed when the agent starts.
Any published pattern ID embeds this hash, so findings are reproducible
from bytecode + corpus alone.
