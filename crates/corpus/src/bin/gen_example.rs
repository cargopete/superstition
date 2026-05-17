//! Generate a realistic example corpus for end-to-end testing.
//!
//! Usage:  cargo run -p superstition-corpus --bin gen-example -- [output_dir]
//!
//! Default output: `examples/corpus/`
//!
//! Creates two Parquet tables:
//!
//!   erc20_transfers  – 100 000 rows, full calendar year 2024
//!     columns: block_timestamp (u64 unix), value_wei (u64), gas_price_gwei (u64)
//!
//!   dex_swaps        –  40 000 rows, full calendar year 2024
//!     columns: block_timestamp (u64 unix), amount_usd_cents (u64), fee_bps (u64)
//!
//! Statistical signals baked in (things the agent should discover):
//!
//!   erc20_transfers
//!     - Day-of-week: Mon 1.60x, Sun 0.55x (strong weekday effect)
//!     - Hour-of-day: bimodal, peaks at 08-10 UTC (EU) and 14-16 UTC (US open)
//!     - value_wei:   log-normal, heavy-tailed
//!
//!   dex_swaps
//!     - Day-of-week: Tue/Thu 1.55x, Sun 0.40x (different shape from ERC-20)
//!     - Hour-of-day: US-open concentrated, single peak at 13-15 UTC
//!     - amount_usd_cents: Pareto power-law (alpha=1.5, x_min=$10)
//!     - fee_bps: categorical — 5bps 40%, 30bps 50%, 100bps 10%

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use arrow::array::UInt64Array;
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;

// ── PRNG (splitmix64) ─────────────────────────────────────────────────────────

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }

    /// Uniform float in [0, 1).
    fn f64(&mut self) -> f64 {
        (self.next() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Approximate N(0,1) via the 12-uniform CLT trick.
    fn normal(&mut self) -> f64 {
        let s: f64 = (0..12).map(|_| self.f64()).sum();
        s - 6.0
    }

    /// Log-normal: e^(mu + sigma * N(0,1)).
    fn lognormal(&mut self, mu: f64, sigma: f64) -> f64 {
        (mu + sigma * self.normal()).exp()
    }

    /// Pareto variate via inverse CDF: x_min * U^(-1/alpha).
    fn pareto(&mut self, alpha: f64, x_min: f64) -> f64 {
        let u = self.f64().max(1e-9);
        x_min * u.powf(-1.0 / alpha)
    }
}

// ── constants ─────────────────────────────────────────────────────────────────

/// 2024-01-01 00:00:00 UTC (a Monday).
const YEAR_START: u64 = 1_704_067_200;
const DAYS_IN_YEAR: u64 = 366; // 2024 is a leap year

// dow_weight[d] where d = (epoch_day + 4) % 7; 0=Sun … 6=Sat
const DOW_ERC20: [f64; 7] = [0.55, 1.60, 1.25, 1.25, 1.25, 1.20, 0.70];
const DOW_DEX: [f64; 7] = [0.40, 1.40, 1.55, 1.10, 1.55, 1.20, 0.80];

const HOUR_ERC20: [f64; 24] = [
    0.20, 0.15, 0.15, 0.15, 0.20, 0.30, // 00-05 UTC (quiet)
    0.50, 0.80, 1.00, 0.90, 0.80, 0.90, // 06-11 UTC (EU morning)
    1.00, 1.10, 1.30, 1.40, 1.30, 1.20, // 12-17 UTC (US open, EU afternoon)
    1.00, 0.80, 0.60, 0.50, 0.30, 0.20, // 18-23 UTC (winding down)
];

const HOUR_DEX: [f64; 24] = [
    0.10, 0.10, 0.10, 0.10, 0.10, 0.10, // 00-05 UTC
    0.20, 0.40, 0.60, 0.70, 0.80, 0.90, // 06-11 UTC
    1.20, 1.50, 1.50, 1.30, 1.00, 0.80, // 12-17 UTC (NY open peak)
    0.60, 0.40, 0.30, 0.20, 0.10, 0.10, // 18-23 UTC
];

// ── main ──────────────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    let out_dir =
        PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "examples/corpus".to_string()));
    fs::create_dir_all(&out_dir)?;

    let props = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .build();

    // ── erc20_transfers ───────────────────────────────────────────────────────

    let ts_erc20 = timestamps(100_000, &DOW_ERC20, &HOUR_ERC20, 0x1234_5678_dead_beef);
    let n = ts_erc20.len();

    let mut rng = Rng(0xfeed_face_cafe_babe);
    let value_wei: Vec<u64> = (0..n)
        .map(|_| {
            // log-normal centred around ~10^16 wei (≈ 0.01 ETH)
            rng.lognormal(36.8, 2.2).min(u64::MAX as f64) as u64
        })
        .collect();
    let gas_gwei: Vec<u64> = (0..n)
        .map(|_| rng.lognormal(3.4, 0.7).clamp(5.0, 200.0) as u64)
        .collect();

    let schema = Arc::new(Schema::new(vec![
        Field::new("block_timestamp", DataType::UInt64, false),
        Field::new("value_wei", DataType::UInt64, false),
        Field::new("gas_price_gwei", DataType::UInt64, false),
    ]));
    let path = out_dir.join("erc20_transfers.parquet");
    let file = fs::File::create(&path)?;
    let mut w = ArrowWriter::try_new(file, schema.clone(), Some(props.clone()))?;
    for chunk in 0..(n + 3_999) / 4_000 {
        let lo = chunk * 4_000;
        let hi = (lo + 4_000).min(n);
        w.write(&RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(UInt64Array::from(ts_erc20[lo..hi].to_vec())),
                Arc::new(UInt64Array::from(value_wei[lo..hi].to_vec())),
                Arc::new(UInt64Array::from(gas_gwei[lo..hi].to_vec())),
            ],
        )?)?;
    }
    w.close()?;
    println!("wrote {:>7} rows → {}", n, path.display());

    // ── dex_swaps ─────────────────────────────────────────────────────────────

    let ts_dex = timestamps(40_000, &DOW_DEX, &HOUR_DEX, 0xabcd_ef01_2345_6789);
    let m = ts_dex.len();

    let mut rng2 = Rng(0x9999_aaaa_bbbb_cccc);
    let amount_cents: Vec<u64> = (0..m)
        .map(|_| rng2.pareto(1.5, 1_000.0).min(1_000_000_000.0) as u64)
        .collect();
    let fee_bps: Vec<u64> = (0..m)
        .map(|_| {
            let u = rng2.f64();
            if u < 0.40 { 5 } else if u < 0.90 { 30 } else { 100 }
        })
        .collect();

    let schema2 = Arc::new(Schema::new(vec![
        Field::new("block_timestamp", DataType::UInt64, false),
        Field::new("amount_usd_cents", DataType::UInt64, false),
        Field::new("fee_bps", DataType::UInt64, false),
    ]));
    let path2 = out_dir.join("dex_swaps.parquet");
    let file2 = fs::File::create(&path2)?;
    let mut w2 = ArrowWriter::try_new(file2, schema2.clone(), Some(props))?;
    for chunk in 0..(m + 3_999) / 4_000 {
        let lo = chunk * 4_000;
        let hi = (lo + 4_000).min(m);
        w2.write(&RecordBatch::try_new(
            schema2.clone(),
            vec![
                Arc::new(UInt64Array::from(ts_dex[lo..hi].to_vec())),
                Arc::new(UInt64Array::from(amount_cents[lo..hi].to_vec())),
                Arc::new(UInt64Array::from(fee_bps[lo..hi].to_vec())),
            ],
        )?)?;
    }
    w2.close()?;
    println!("wrote {:>7} rows → {}", m, path2.display());

    println!();
    println!("run with:");
    println!("  cargo run --release -p superstition-agent -- \\");
    println!("    --corpus {} \\", out_dir.display());
    println!("    --feed examples/feed.json \\");
    println!("    --state examples/state.json \\");
    println!("    --workspace . \\");
    println!("    --iterations 3 --hypotheses 5");

    Ok(())
}

// ── timestamp generator ───────────────────────────────────────────────────────

/// Fill `target` Unix timestamps across 2024, weighted by DOW and hour-of-day.
///
/// Uses Bresenham-style integer distribution to avoid accumulating rounding
/// error across slots.  The actual row count equals `target` ± 1.
fn timestamps(target: usize, dow_w: &[f64; 7], hour_w: &[f64; 24], seed: u64) -> Vec<u64> {
    // Build joint (day × hour) weight grid.
    let n_slots = (DAYS_IN_YEAR * 24) as usize;
    let mut weights = vec![0.0f64; n_slots];
    for day in 0..DAYS_IN_YEAR {
        let epoch_day = YEAR_START / 86_400 + day;
        let dow = ((epoch_day + 4) % 7) as usize; // 0=Sun … 6=Sat
        for hour in 0..24usize {
            weights[(day * 24) as usize + hour] = dow_w[dow] * hour_w[hour];
        }
    }
    let total_w: f64 = weights.iter().sum();
    let scale = target as f64 / total_w;

    // Bresenham integer counts: fractional parts carry forward.
    let mut counts = vec![0usize; n_slots];
    let mut carry = 0.0f64;
    for (i, &w) in weights.iter().enumerate() {
        let exact = w * scale + carry;
        let c = exact.floor() as usize;
        carry = exact - c as f64;
        counts[i] = c;
    }
    // Remaining fractional row (carry ≈ 0..1) goes into the last non-zero slot.
    if carry >= 0.5 {
        if let Some(last) = counts.iter_mut().rev().find(|c| **c > 0) {
            *last += 1;
        }
    }

    let mut rng = Rng(seed);
    let total: usize = counts.iter().sum();
    let mut out = Vec::with_capacity(total);

    for (slot, &count) in counts.iter().enumerate() {
        if count == 0 {
            continue;
        }
        let day = (slot / 24) as u64;
        let hour = (slot % 24) as u64;
        let slot_start = YEAR_START + day * 86_400 + hour * 3_600;
        for _ in 0..count {
            out.push(slot_start + rng.next() % 3_600);
        }
    }

    out.sort_unstable();
    out
}
