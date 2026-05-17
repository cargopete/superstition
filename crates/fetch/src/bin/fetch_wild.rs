//! fetch-wild: pull real market data from public APIs + compute moon phases.
//!
//! Usage:  cargo run -p superstition-fetch --bin fetch-wild -- [output_dir]
//!
//! Default output: `examples/corpus-wild/`
//!
//! Tables written:
//!   btc_daily      – BTC/USDT daily OHLCV + trade count  (Binance)
//!   eth_daily      – ETH/USDT daily OHLCV + trade count  (Binance)
//!   eth_tvl_daily  – Ethereum chain daily TVL             (DeFiLlama)
//!   sol_tvl_daily  – Solana chain daily TVL               (DeFiLlama)
//!   sp500_daily    – S&P 500 daily close + volume         (Yahoo Finance)
//!   moon_phases    – lunar phase for every calendar day   (computed)
//!
//! All timestamps are midnight UTC (seconds). All prices in USD cents.
//! TVL in USD millions. Volume in USD thousands.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use arrow::array::UInt64Array;
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;

// ── date range: 2023-01-01 → 2024-12-31 UTC ──────────────────────────────────
const START_TS: u64 = 1_672_531_200; // 2023-01-01 00:00:00 UTC
const END_TS: u64 = 1_735_689_599;   // 2024-12-31 23:59:59 UTC

// ── moon math ─────────────────────────────────────────────────────────────────
// Reference new moon: 2000-01-06 18:14:00 UTC
const NEW_MOON_REF: u64 = 947_182_440;
// Synodic period: 29.53059 days
const SYNODIC_S: u64 = 2_551_443;

fn moon_phase_pct(ts: u64) -> u64 {
    ((ts - NEW_MOON_REF) % SYNODIC_S) * 100 / SYNODIC_S
}

fn moon_age_days(ts: u64) -> u64 {
    ((ts - NEW_MOON_REF) % SYNODIC_S) * 30 / SYNODIC_S
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn day_ts(ts: u64) -> u64 {
    ts - ts % 86_400
}

fn pause() {
    thread::sleep(Duration::from_millis(400));
}

fn fetch_json(url: &str, user_agent: Option<&str>) -> Result<serde_json::Value> {
    let req = ureq::get(url);
    let req = match user_agent {
        Some(ua) => req.set("User-Agent", ua),
        None => req,
    };
    let body = req
        .call()
        .with_context(|| format!("GET {url}"))?
        .into_json::<serde_json::Value>()
        .context("json parse")?;
    Ok(body)
}

fn write_parquet(
    path: &Path,
    schema: Arc<Schema>,
    columns: Vec<Arc<dyn arrow::array::Array>>,
    props: &WriterProperties,
) -> Result<usize> {
    let n = columns[0].len();
    let batch = RecordBatch::try_new(schema.clone(), columns)?;
    let file = fs::File::create(path)?;
    let mut w = ArrowWriter::try_new(file, schema, Some(props.clone()))?;
    w.write(&batch)?;
    w.close()?;
    Ok(n)
}

// ── Binance ───────────────────────────────────────────────────────────────────

struct OhlcvRow {
    ts: u64,
    open_cents: u64,
    high_cents: u64,
    close_cents: u64,
    volume_usd_k: u64,
    trade_count: u64,
}

fn fetch_binance(symbol: &str) -> Result<Vec<OhlcvRow>> {
    let url = format!(
        "https://api.binance.com/api/v3/klines\
         ?symbol={symbol}&interval=1d\
         &startTime={}&endTime={}&limit=1000",
        START_TS * 1000,
        END_TS * 1000,
    );
    let body = fetch_json(&url, None)?;
    let arr = body.as_array().context("expected array from binance")?;

    let mut rows = Vec::new();
    for item in arr {
        let row = item.as_array().context("expected row array")?;
        let open_ms = row[0].as_u64().context("open_time")?;
        let open_s = open_ms / 1000;

        let parse_str = |v: &serde_json::Value, field: &str| -> Result<f64> {
            v.as_str()
                .with_context(|| format!("expected string for {field}"))?
                .parse::<f64>()
                .with_context(|| format!("parse f64 for {field}"))
        };

        rows.push(OhlcvRow {
            ts: day_ts(open_s),
            open_cents: (parse_str(&row[1], "open")? * 100.0) as u64,
            high_cents: (parse_str(&row[2], "high")? * 100.0) as u64,
            close_cents: (parse_str(&row[4], "close")? * 100.0) as u64,
            // index 7 = quote asset volume = USD volume
            volume_usd_k: (parse_str(&row[7], "quote_vol")? / 1_000.0) as u64,
            trade_count: row[8].as_u64().unwrap_or(0),
        });
    }
    Ok(rows)
}

fn write_ohlcv(out_dir: &Path, name: &str, rows: &[OhlcvRow], props: &WriterProperties) -> Result<()> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("timestamp", DataType::UInt64, false),
        Field::new("open_usd_cents", DataType::UInt64, false),
        Field::new("high_usd_cents", DataType::UInt64, false),
        Field::new("close_usd_cents", DataType::UInt64, false),
        Field::new("volume_usd_thousands", DataType::UInt64, false),
        Field::new("trade_count", DataType::UInt64, false),
    ]));
    let path = out_dir.join(format!("{name}.parquet"));
    let n = write_parquet(
        &path,
        schema,
        vec![
            Arc::new(UInt64Array::from_iter_values(rows.iter().map(|r| r.ts))),
            Arc::new(UInt64Array::from_iter_values(rows.iter().map(|r| r.open_cents))),
            Arc::new(UInt64Array::from_iter_values(rows.iter().map(|r| r.high_cents))),
            Arc::new(UInt64Array::from_iter_values(rows.iter().map(|r| r.close_cents))),
            Arc::new(UInt64Array::from_iter_values(rows.iter().map(|r| r.volume_usd_k))),
            Arc::new(UInt64Array::from_iter_values(rows.iter().map(|r| r.trade_count))),
        ],
        props,
    )?;
    println!("wrote {:>5} rows → {}", n, path.display());
    Ok(())
}

// ── DeFiLlama ─────────────────────────────────────────────────────────────────

fn fetch_defillama(chain: &str) -> Result<Vec<(u64, u64)>> {
    let url = format!("https://api.llama.fi/v2/historicalChainTvl/{chain}");
    let body = fetch_json(&url, None)?;
    let arr = body.as_array().context("expected array from defillama")?;

    let mut rows = Vec::new();
    for item in arr {
        let ts = item["date"].as_u64().context("date field")?;
        let tvl = item["tvl"].as_f64().context("tvl field")?;
        if ts >= START_TS && ts <= END_TS {
            rows.push((day_ts(ts), (tvl / 1_000_000.0) as u64));
        }
    }
    rows.sort_by_key(|r| r.0);
    Ok(rows)
}

fn write_tvl(out_dir: &Path, name: &str, rows: &[(u64, u64)], props: &WriterProperties) -> Result<()> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("timestamp", DataType::UInt64, false),
        Field::new("tvl_usd_millions", DataType::UInt64, false),
    ]));
    let path = out_dir.join(format!("{name}.parquet"));
    let n = write_parquet(
        &path,
        schema,
        vec![
            Arc::new(UInt64Array::from_iter_values(rows.iter().map(|r| r.0))),
            Arc::new(UInt64Array::from_iter_values(rows.iter().map(|r| r.1))),
        ],
        props,
    )?;
    println!("wrote {:>5} rows → {}", n, path.display());
    Ok(())
}

// ── Yahoo Finance (S&P 500) ───────────────────────────────────────────────────

fn fetch_sp500() -> Result<Vec<(u64, u64, u64)>> {
    // timestamp, close_cents, volume_thousands
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/%5EGSPC\
         ?interval=1d&period1={START_TS}&period2={END_TS}&includePrePost=false"
    );
    let url = url.as_str();
    let body = fetch_json(url, Some("Mozilla/5.0 (compatible; superstition/0.1)"))?;

    let result = &body["chart"]["result"][0];
    let timestamps = result["timestamp"]
        .as_array()
        .context("sp500: missing timestamps")?;
    let quote = &result["indicators"]["quote"][0];
    let closes = quote["close"].as_array().context("sp500: missing closes")?;
    let volumes = quote["volume"].as_array().context("sp500: missing volumes")?;

    let mut rows = Vec::new();
    for i in 0..timestamps.len() {
        let ts = match timestamps[i].as_u64() {
            Some(t) => day_ts(t),
            None => continue,
        };
        if ts < START_TS || ts > END_TS {
            continue;
        }
        let close_cents = match closes.get(i).and_then(|v| v.as_f64()) {
            Some(c) => (c * 100.0) as u64,
            None => continue, // skip nulls (weekends / holidays)
        };
        let vol_k = volumes
            .get(i)
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            / 1_000;
        rows.push((ts, close_cents, vol_k));
    }
    rows.sort_by_key(|r| r.0);
    Ok(rows)
}

fn write_sp500(out_dir: &Path, rows: &[(u64, u64, u64)], props: &WriterProperties) -> Result<()> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("timestamp", DataType::UInt64, false),
        Field::new("close_usd_cents", DataType::UInt64, false),
        Field::new("volume_thousands", DataType::UInt64, false),
    ]));
    let path = out_dir.join("sp500_daily.parquet");
    let n = write_parquet(
        &path,
        schema,
        vec![
            Arc::new(UInt64Array::from_iter_values(rows.iter().map(|r| r.0))),
            Arc::new(UInt64Array::from_iter_values(rows.iter().map(|r| r.1))),
            Arc::new(UInt64Array::from_iter_values(rows.iter().map(|r| r.2))),
        ],
        props,
    )?;
    println!("wrote {:>5} rows → {}", n, path.display());
    Ok(())
}

// ── Moon phases ───────────────────────────────────────────────────────────────

fn write_moon_phases(out_dir: &Path, props: &WriterProperties) -> Result<()> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("timestamp", DataType::UInt64, false),
        // 0 = new moon, 50 = full moon, approaching 100 = next new moon
        Field::new("phase_pct", DataType::UInt64, false),
        // 0-29: days since last new moon
        Field::new("moon_age_days", DataType::UInt64, false),
        // 1 during the ~3 days around full moon (phase 45-55), else 0
        Field::new("is_full_moon", DataType::UInt64, false),
        // 1 during the ~3 days around new moon (phase 0-5 or 95-100), else 0
        Field::new("is_new_moon", DataType::UInt64, false),
    ]));

    let mut timestamps = Vec::new();
    let mut phases = Vec::new();
    let mut ages = Vec::new();
    let mut is_full = Vec::new();
    let mut is_new = Vec::new();

    let mut ts = day_ts(START_TS);
    while ts <= END_TS {
        let phase = moon_phase_pct(ts);
        let age = moon_age_days(ts);
        timestamps.push(ts);
        phases.push(phase);
        ages.push(age);
        is_full.push(u64::from(phase >= 45 && phase <= 55));
        is_new.push(u64::from(phase <= 5 || phase >= 95));
        ts += 86_400;
    }

    let path = out_dir.join("moon_phases.parquet");
    let n = write_parquet(
        &path,
        schema,
        vec![
            Arc::new(UInt64Array::from(timestamps)),
            Arc::new(UInt64Array::from(phases)),
            Arc::new(UInt64Array::from(ages)),
            Arc::new(UInt64Array::from(is_full)),
            Arc::new(UInt64Array::from(is_new)),
        ],
        props,
    )?;
    println!("wrote {:>5} rows → {}", n, path.display());
    Ok(())
}

// ── main ──────────────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    let out_dir =
        PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "examples/corpus-wild".to_string()));
    fs::create_dir_all(&out_dir)?;

    let props = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .build();

    println!("fetching 2023-2024 market data…\n");

    // BTC
    print!("binance BTCUSDT… ");
    match fetch_binance("BTCUSDT") {
        Ok(rows) => write_ohlcv(&out_dir, "btc_daily", &rows, &props)?,
        Err(e) => println!("SKIP ({e})"),
    }
    pause();

    // ETH
    print!("binance ETHUSDT… ");
    match fetch_binance("ETHUSDT") {
        Ok(rows) => write_ohlcv(&out_dir, "eth_daily", &rows, &props)?,
        Err(e) => println!("SKIP ({e})"),
    }
    pause();

    // ETH TVL
    print!("defillama Ethereum… ");
    match fetch_defillama("Ethereum") {
        Ok(rows) => write_tvl(&out_dir, "eth_tvl_daily", &rows, &props)?,
        Err(e) => println!("SKIP ({e})"),
    }
    pause();

    // SOL TVL
    print!("defillama Solana… ");
    match fetch_defillama("Solana") {
        Ok(rows) => write_tvl(&out_dir, "sol_tvl_daily", &rows, &props)?,
        Err(e) => println!("SKIP ({e})"),
    }
    pause();

    // S&P 500
    print!("yahoo finance ^GSPC… ");
    match fetch_sp500() {
        Ok(rows) => write_sp500(&out_dir, &rows, &props)?,
        Err(e) => println!("SKIP ({e})"),
    }

    // Moon phases (no network)
    print!("moon phases (computed)… ");
    write_moon_phases(&out_dir, &props)?;

    println!();
    println!("corpus ready. run with:");
    println!("  cargo run --release -p superstition-agent -- \\");
    println!("    --corpus {} \\", out_dir.display());
    println!("    --feed   examples/feed-wild.json \\");
    println!("    --state  examples/state-wild.json \\");
    println!("    --workspace . \\");
    println!("    --iterations 3 --hypotheses 5");

    Ok(())
}
