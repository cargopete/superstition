mod api;
mod builder;
mod codegen;
mod state;

use std::path::PathBuf;

use anyhow::{Context, Result};
use superstition_corpus::Corpus;
use superstition_feed::{FeedStore, Pattern};
use superstition_host::Executor;
use superstition_scorer::fdr;

use codegen::Hypothesis;
use state::{AgentState, SignificantRecord};

fn main() -> Result<()> {
    let cfg = parse_args()?;

    let corpus = Corpus::open(&cfg.corpus_dir)
        .with_context(|| format!("opening corpus {}", cfg.corpus_dir.display()))?;

    // Build corpus schema string for prompts.
    let schema_map = corpus.column_names()
        .context("reading corpus schema")?;
    let mut table_names: Vec<&str> = schema_map.keys().map(|s| s.as_str()).collect();
    table_names.sort();
    let corpus_schema = table_names
        .iter()
        .map(|t| {
            let cols = schema_map[*t]
                .iter()
                .map(|c| format!("    {c}  uint64"))
                .collect::<Vec<_>>()
                .join("\n");
            format!("  Table: {t}\n{cols}")
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    println!("corpus_id  : {}", corpus.corpus_id);
    println!("tables     : {}", table_names.join(", "));
    println!("iterations : {}", cfg.iterations);
    println!("hypotheses : {} per iteration", cfg.hypotheses_per_iter);
    if let Some(secs) = cfg.loop_interval {
        println!("loop mode  : every {secs}s");
    }
    println!();

    let client = api::Client::from_env()?;
    let executor = Executor::new()?;
    let feed = FeedStore::open(&cfg.feed_path);

    let mut run = 0u64;

    loop {
        run += 1;

        // ── load state at the start of every run (picks up external edits) ──
        let mut st = AgentState::load(&cfg.state_path)
            .with_context(|| format!("loading state {}", cfg.state_path.display()))?;

        println!("━━━ run {run} ━━━  (seen={}, significant={})", st.seen.len(), st.significant.len());

        for iter in 1..=cfg.iterations {
            println!("  ── iteration {iter}/{} ──", cfg.iterations);

            // ── step 1: generate hypotheses (with Stage H feedback) ──
            let hypotheses =
                codegen::generate_hypotheses(&client, &st.seen, &st.significant, cfg.hypotheses_per_iter, &corpus_schema)
                    .context("hypothesis generation")?;
            println!("  generated {} hypotheses", hypotheses.len());

            let mut iter_results: Vec<(Hypothesis, superstition_host::RunResult)> = Vec::new();

            for hyp in &hypotheses {
                println!("    → {}: {}", hyp.name, hyp.description);
                st.add_seen(hyp.clone());

                // ── step 2: generate detector code ──
                let mut code = match codegen::generate_detector_code(&client, hyp, &corpus_schema) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("      codegen failed: {e}");
                        continue;
                    }
                };

                // ── step 3: build (with one retry on compile error) ──
                let wasm_path = loop {
                    match builder::build_detector(&cfg.workspace_root, &hyp.name, &code) {
                        Ok(path) => break Some(path),
                        Err(e) => {
                            eprintln!("      build error — retrying with fix...");
                            let err_str = e.to_string();
                            match codegen::fix_detector_code(&client, hyp, &code, &err_str, &corpus_schema) {
                                Ok(fixed) => {
                                    code = fixed;
                                    match builder::build_detector(&cfg.workspace_root, &hyp.name, &code) {
                                        Ok(path) => break Some(path),
                                        Err(e2) => {
                                            eprintln!("      retry also failed: {e2}");
                                            break None;
                                        }
                                    }
                                }
                                Err(e) => {
                                    eprintln!("      fix codegen failed: {e}");
                                    break None;
                                }
                            }
                        }
                    }
                };

                let Some(wasm_path) = wasm_path else { continue };

                // ── step 4: run + score ──
                match executor.run(&wasm_path, Some(&corpus)) {
                    Ok(result) => {
                        let sig = result.p_value < fdr::ALPHA && result.passes_effect_floor;
                        println!(
                            "      p={:.2e}  V={:.3}  {}",
                            result.p_value,
                            result.effect_size,
                            if sig { "SIGNIFICANT ✓" } else { "not significant" },
                        );
                        iter_results.push((hyp.clone(), result));
                    }
                    Err(e) => eprintln!("      run failed: {e}"),
                }
            }

            // ── step 5: batch BH across this iteration's results ──
            if !iter_results.is_empty() {
                let p_values: Vec<f64> = iter_results.iter().map(|(_, r)| r.p_value).collect();
                let family_refs: Vec<&str> =
                    iter_results.iter().map(|(h, _)| h.family.as_str()).collect();
                let rejected = fdr::group_bh_reject(&p_values, &family_refs, fdr::ALPHA);

                for (i, (hyp, result)) in iter_results.iter().enumerate() {
                    if rejected[i] && result.passes_effect_floor {
                        // ── publish to feed ──
                        let wasm_path = cfg
                            .workspace_root
                            .join("generated")
                            .join(&hyp.name)
                            .join("target/wasm32-wasip1/release")
                            .join(format!("{}.wasm", hyp.name.replace('-', "_")));

                        if let Ok(wasm_bytes) = std::fs::read(&wasm_path) {
                            let pattern = Pattern::new(
                                &wasm_bytes,
                                &corpus.corpus_id,
                                result.description.clone(),
                                result.hypothesis.clone(),
                                hyp.family.clone(),
                                result.counts.clone(),
                                result.sample_size,
                                result.detail.clone(),
                                result.p_value,
                                fdr::bh_q_values(&[result.p_value])[0],
                                result.effect_size,
                            );
                            let pid = pattern.id.clone();
                            if let Err(e) = feed.publish(pattern) {
                                eprintln!("    ⚠  feed publish failed: {e}");
                            } else {
                                println!("    ✦ published → {pid}  (verify: superstition-verify {pid})");
                            }

                            // ── update persistent state with significant find ──
                            st.add_significant(SignificantRecord {
                                name: hyp.name.clone(),
                                description: result.description.clone(),
                                family: hyp.family.clone(),
                                p_value: result.p_value,
                                effect_size: result.effect_size,
                                pattern_id: pid,
                            });
                        }
                    }
                }
            }

            // ── persist state after every iteration ──
            if let Err(e) = st.save(&cfg.state_path) {
                eprintln!("  ⚠  state save failed: {e}");
            }

            println!("  cumulative seen={} significant={}", st.seen.len(), st.significant.len());
        }

        // ── continuous mode: sleep then repeat ──
        match cfg.loop_interval {
            Some(secs) => {
                println!("\nsleeping {secs}s until next run…\n");
                std::thread::sleep(std::time::Duration::from_secs(secs));
            }
            None => break,
        }
    }

    Ok(())
}

// ── config ────────────────────────────────────────────────────────────────────

struct Config {
    corpus_dir: PathBuf,
    workspace_root: PathBuf,
    feed_path: PathBuf,
    state_path: PathBuf,
    iterations: usize,
    hypotheses_per_iter: usize,
    /// If Some(secs), run forever sleeping this many seconds between runs.
    loop_interval: Option<u64>,
}

fn parse_args() -> Result<Config> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut corpus_dir: Option<PathBuf> = None;
    let mut workspace_root = PathBuf::from(".");
    let mut feed_path = PathBuf::from("feed.json");
    let mut state_path = PathBuf::from("state.json");
    let mut iterations = 1usize;
    let mut hypotheses_per_iter = 3usize;
    let mut loop_interval: Option<u64> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--corpus" => {
                i += 1;
                corpus_dir =
                    Some(PathBuf::from(args.get(i).context("--corpus requires a directory")?));
            }
            "--workspace" => {
                i += 1;
                workspace_root = PathBuf::from(
                    args.get(i).context("--workspace requires a directory")?,
                );
            }
            "--feed" => {
                i += 1;
                feed_path = PathBuf::from(args.get(i).context("--feed requires a path")?);
            }
            "--state" => {
                i += 1;
                state_path = PathBuf::from(args.get(i).context("--state requires a path")?);
            }
            "--iterations" | "-n" => {
                i += 1;
                iterations = args.get(i).context("--iterations requires a number")?.parse()?;
            }
            "--hypotheses" | "-h" => {
                i += 1;
                hypotheses_per_iter =
                    args.get(i).context("--hypotheses requires a number")?.parse()?;
            }
            "--loop-interval" => {
                i += 1;
                loop_interval =
                    Some(args.get(i).context("--loop-interval requires seconds")?.parse()?);
            }
            _ => {}
        }
        i += 1;
    }

    let corpus_dir = corpus_dir.context(
        "usage: agent --corpus <dir> [--feed <feed.json>] [--state <state.json>] \
         [--workspace <dir>] [--iterations N] [--hypotheses N] [--loop-interval SECS]",
    )?;

    Ok(Config {
        corpus_dir,
        workspace_root,
        feed_path,
        state_path,
        iterations,
        hypotheses_per_iter,
        loop_interval,
    })
}
