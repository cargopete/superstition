# Superstition

> *Verifiable astrology for on-chain data.*

Superstition is an on-chain analytics platform that publishes statistically-significant
patterns discovered by an autonomous agent fleet. Every published pattern carries its
**bytecode hash**, its **dataset hash**, and its **corrected p-value**. Any reader can
re-run the detector and reproduce the score bit-exactly.

This converts the standard analytics-platform credibility problem —
*"how do I know your numbers are real?"* — into a mechanical check anyone can perform.

---

## The idea in one paragraph

An LLM writes a WebAssembly detector component. The detector runs against a
content-addressed, frozen parquet corpus of EVM chain data. It returns raw counts (not
p-values — it can't lie about statistics it never computes). The host runs the
appropriate statistical test and applies group-aware Benjamini-Hochberg FDR correction.
If the pattern passes the significance gate and has a visible effect size, it gets
published to a feed alongside a `verify` command that anyone can run to reproduce
the exact p-value from the bytecode and corpus ID alone.

The brand is "verifiable astrology". The method is transparent. The result is
exploratory. Both are stated up front.

---

## Architecture

```
┌──────────────────────────────────────────────┐
│  Hypothesis generator  (Haiku 4.5)           │◄── best patterns feed back
│  Code-gen agent        (Sonnet 4.6)          │
│  Build pipeline        (cargo component)     │
└───────────────────┬──────────────────────────┘
                    │ .wasm
                    ▼
┌──────────────────────────────────────────────┐
│  Wasmtime host                               │
│  • WASIp2 component model                    │
│  • Epoch interruption (60s soft / 5m hard)   │
│  • Memory cap (512 MB)                       │
│  • Corpus handle: read-only parquet slab     │
│  • All network / clock / random DENIED       │
└───────────────────┬──────────────────────────┘
                    │ counts
                    ▼
┌──────────────────────────────────────────────┐
│  Scorer (host-side)                          │
│  • Fisher exact / chi-squared / KS / boot.  │
│  • Group-aware sequential BH (FDR α=0.05)   │
│  • Effect-size floor                         │
└───────────────────┬──────────────────────────┘
                    │ q-value
                    ▼
┌──────────────────────────────────────────────┐
│  Public feed  (web + RSS)                    │
│  • bytecode hash · corpus_id · q-value       │
│  • "verify" button → one shell invocation    │
└──────────────────────────────────────────────┘
```

**Key property:** the detector never computes its own p-value. It returns raw counts
and a `test-type` enum; the host runs the statistical test. LLM hallucinated p-values
are architecturally impossible.

---

## WIT contract

All detectors implement a single WIT world (`wit/superstition.wit`):

```wit
world detector-world {
    import corpus;   // host provides frozen corpus access
    export detector; // component implements the test
}
```

The corpus is accessed only through an opaque `corpus-handle` resource. Detectors
cannot enumerate paths, open files, make network requests, read wall-clock time, or
access randomness. Capability isolation is enforced by the Wasmtime sandbox, not by
convention.

---

## Repo layout

```
superstition/
├── wit/superstition.wit          canonical WIT (corpus + detector interfaces)
├── crates/
│   ├── host/                     Wasmtime host binary
│   │   └── src/main.rs
│   └── detectors/
│       └── dow-erc20/            reference detector: ERC-20 day-of-week chi-squared
│           ├── src/lib.rs
│           └── wit/world.wit
└── rust-toolchain.toml           pinned stable + wasm32-wasip2
```

---

## Build

**Prerequisites:** Rust stable, `cargo-component`, `wasm-tools`.

```bash
# install tooling (once)
cargo install cargo-component wasm-tools

# build the reference detector
cargo component build --release -p dow-erc20

# build the host
cargo build --release -p superstition-host
```

**Run the reference detector through the host (stub corpus):**

```bash
cargo run -p superstition-host -- target/wasm32-wasip1/release/dow_erc20.wasm
```

Expected output (7-row stub corpus, one row per day of the week):

```
description : ERC-20 transfers by day of week
hypothesis  : ERC-20 transfer counts are not uniformly distributed across days of the week.
family      : temporal-cyclic
version     : 0.1.0

counts      : [1, 1, 1, 1, 1, 1, 1]
sample_size : 7
test_type   : TestType::ChiSquared(6)
detail      : Sun=1 Mon=1 Tue=1 Wed=1 Thu=1 Fri=1 Sat=1
```

With the real corpus (M3), counts will be in the billions and the chi-squared test
will return a real p-value.

---

## Milestones

| # | Status | Description |
|---|--------|-------------|
| M0 | ✅ | WIT contract, reference detector (`dow-erc20`), workspace |
| M1 | ✅ | Wasmtime host: loads detector, vends corpus handle, enforces caps |
| M2 | ⬜ | Scorer: 4 statistical tests + group-aware sequential BH |
| M3 | ⬜ | Full corpus (cryo parquet) + multi-detector orchestration |
| M4 | ⬜ | Agent loop: Haiku hypothesis gen → Sonnet code-gen → score |
| M5 | ⬜ | Public feed (web + RSS + `verify` CLI) |
| M6 | ⬜ | Hardening + adversarial test suite + first public broadcast |
| M7 | ⬜ | Continuous operation (cron + evolutionary feedback loop) |

---

## Prior art

- **FunSearch / AlphaEvolve** — LLM-in-the-loop evolutionary program search.
  Superstition adapts the same structure (propose → evaluate → feed back) but
  the evaluator is statistical significance under FDR control, not algorithmic
  optimality.
- **Dune / Glassnode / Nansen** — existing on-chain analytics. None publish
  detectors as content-addressed executable artifacts; none offer bit-exact
  reproducibility.
- **Benjamini-Hochberg (1995)** — the FDR correction standard.
  Superstition uses the sequential variant (Wang & Ramdas 2021) to handle
  streaming hypothesis arrival without invalidating earlier rejections.

---

## Reproducibility guarantee

Anyone with the published detector `.wasm` and the `corpus_id` can reproduce the
score:

```bash
superstition verify pattern:bafyreic7...
# ▸ fetching detector.wasm  ✓
# ▸ fetching corpus index   ✓
# ▸ running detector        ✓ (3.2s)
# ▸ q-value reported: 0.0023  recomputed: 0.0023  ✓ MATCH
```

The determinism boundary is everything downstream of `cargo component build`
with a pinned toolchain. The LLM generates different code on different calls —
that's accepted. Once built, the `.wasm` + `corpus_id` pair is bit-deterministic
on any machine.

---

*"The stars don't lie. They just don't always tell the truth."*
