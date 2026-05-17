//! superstition-verify <pattern-id>
//!
//! Reproduces a published pattern's p-value from the stored .wasm and a
//! local corpus, then confirms it matches the published value bit-exactly.
//!
//! Usage:
//!   superstition-verify <id> [--feed <feed.json>] [--corpus <dir>]

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use superstition_corpus::Corpus;
use superstition_feed::FeedStore;
use superstition_host::Executor;
use superstition_scorer::fdr;

fn main() -> Result<()> {
    let cfg = parse_args()?;

    let feed = FeedStore::open(&cfg.feed_path);
    let pattern = feed
        .get(&cfg.id)?
        .with_context(|| format!("pattern '{}' not found in {}", cfg.id, cfg.feed_path.display()))?;

    println!("▸ pattern       : {}", pattern.description);
    println!("  hypothesis    : {}", pattern.hypothesis);
    println!("  family        : {}", pattern.family);
    println!("  wasm_hash     : {}", pattern.wasm_hash);
    println!("  corpus_id     : {}", pattern.corpus_id);
    println!("  published     : {}", pattern.published_at);
    println!();

    // ── load corpus ──
    let corpus = Corpus::open(&cfg.corpus_dir)
        .with_context(|| format!("opening corpus {}", cfg.corpus_dir.display()))?;

    if corpus.corpus_id != pattern.corpus_id {
        eprintln!(
            "⚠  corpus_id mismatch!\n   expected : {}\n   got      : {}",
            pattern.corpus_id, corpus.corpus_id
        );
        eprintln!("   Verification may not be meaningful against a different corpus.");
    } else {
        println!("▸ corpus_id     : {} ✓", corpus.corpus_id);
    }

    // ── decode wasm to temp file ──
    let wasm_bytes = pattern.wasm_bytes().context("decoding stored wasm")?;
    let tmp_dir = std::env::temp_dir().join(format!("superstition-verify-{}", pattern.id));
    std::fs::create_dir_all(&tmp_dir)?;
    let wasm_path = tmp_dir.join("detector.wasm");
    std::fs::write(&wasm_path, &wasm_bytes)?;
    println!("▸ wasm decoded  : {} bytes", wasm_bytes.len());

    // ── run detector ──
    let executor = Executor::new()?;
    let t0 = Instant::now();
    let result = executor.run(&wasm_path, Some(&corpus))?;
    let elapsed = t0.elapsed();
    println!("▸ detector ran  : {:.2?}", elapsed);

    // ── compare ──
    let p_reported = pattern.p_value;
    let p_recomputed = result.p_value;

    // BH with a single hypothesis: q == p
    let q_recomputed = fdr::bh_q_values(&[p_recomputed])[0];

    println!();
    println!("  counts        : {:?}", result.counts);
    println!("  detail        : {}", result.detail);
    println!();

    let p_match = (p_reported - p_recomputed).abs() < 1e-12
        || (p_reported == 0.0 && p_recomputed == 0.0)
        || (p_reported.is_nan() && p_recomputed.is_nan());

    println!("  p-value reported  : {p_reported:.6e}");
    println!("  p-value recomputed: {p_recomputed:.6e}");
    println!(
        "  {}",
        if p_match { "✓  MATCH" } else { "✗  MISMATCH — corpus or wasm may differ" }
    );

    println!();
    println!("  q-value (stored)      : {:.6e}", pattern.q_value);
    println!("  q-value (single-hyp)  : {q_recomputed:.6e}");
    println!("  note: stored q may differ if it was BH-adjusted in a multi-hypothesis batch");

    std::fs::remove_dir_all(&tmp_dir).ok();

    if !p_match {
        std::process::exit(1);
    }
    Ok(())
}

// ── arg parsing ───────────────────────────────────────────────────────────────

struct Config {
    id: String,
    feed_path: PathBuf,
    corpus_dir: PathBuf,
}

fn parse_args() -> Result<Config> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut id: Option<String> = None;
    let mut feed_path = PathBuf::from("feed.json");
    let mut corpus_dir = PathBuf::from("corpus/test_fixture");

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--feed" => {
                i += 1;
                feed_path = PathBuf::from(args.get(i).context("--feed requires a path")?);
            }
            "--corpus" => {
                i += 1;
                corpus_dir = PathBuf::from(args.get(i).context("--corpus requires a directory")?);
            }
            s => {
                id = Some(s.to_string());
            }
        }
        i += 1;
    }

    let id = id.context("usage: verify <pattern-id> [--feed <feed.json>] [--corpus <dir>]")?;
    Ok(Config { id, feed_path, corpus_dir })
}
