use std::path::PathBuf;

use anyhow::{Context, Result};
use superstition_corpus::Corpus;
use superstition_host::Executor;
use superstition_scorer::fdr;

fn main() -> Result<()> {
    let (corpus_dir, wasm_paths) = parse_args()?;

    if wasm_paths.is_empty() {
        anyhow::bail!("usage: host [--corpus <dir>] <detector.wasm> [detector2.wasm …]");
    }

    let corpus = corpus_dir
        .map(|d| Corpus::open(&d).with_context(|| format!("opening corpus {}", d.display())))
        .transpose()?;

    if let Some(c) = &corpus {
        println!("corpus_id   : {}", c.corpus_id);
        println!();
    }

    let executor = Executor::new()?;
    let mut results = Vec::new();
    for wasm_path in &wasm_paths {
        match executor.run(wasm_path, corpus.as_ref()) {
            Ok(r) => results.push(r),
            Err(e) => eprintln!("⚠  {}: {e}", wasm_path.display()),
        }
    }

    if results.is_empty() {
        return Ok(());
    }

    let p_values: Vec<f64> = results.iter().map(|r| r.p_value).collect();
    let family_refs: Vec<&str> = results.iter().map(|r| r.family.as_str()).collect();
    let q_values = fdr::bh_q_values(&p_values);
    let rejected = fdr::group_bh_reject(&p_values, &family_refs, fdr::ALPHA);

    println!("══════ Results ({} detector(s)) ══════\n", results.len());
    for (i, r) in results.iter().enumerate() {
        let sig = rejected[i] && r.passes_effect_floor;
        println!("description : {}", r.description);
        println!("hypothesis  : {}", r.hypothesis);
        println!("counts      : {:?}", r.counts);
        println!("sample_size : {}", r.sample_size);
        println!("detail      : {}", r.detail);
        println!("p-value     : {:.4e}", r.p_value);
        println!("q-value     : {:.4e}", q_values[i]);
        println!("effect_size : {:.4}  (floor passed: {})", r.effect_size, r.passes_effect_floor);
        println!("verdict     : {}", if sig { "SIGNIFICANT ✓" } else { "not significant" });
        println!();
    }

    Ok(())
}

fn parse_args() -> Result<(Option<PathBuf>, Vec<PathBuf>)> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut corpus_dir: Option<PathBuf> = None;
    let mut wasm_paths: Vec<PathBuf> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--corpus" {
            i += 1;
            corpus_dir =
                Some(PathBuf::from(args.get(i).context("--corpus requires a directory")?));
        } else {
            wasm_paths.push(PathBuf::from(&args[i]));
        }
        i += 1;
    }
    Ok((corpus_dir, wasm_paths))
}
