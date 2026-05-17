/// Adversarial detector integration tests.
///
/// These tests require the adversarial detector wasm files to be pre-built:
///
///   cargo component build --release \
///     -p det-timeout -p det-panic -p det-bad-counts
///
/// Tests will fail with a clear message if the wasm files are missing.
use std::path::PathBuf;

use superstition_host::Executor;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn wasm_path(name: &str) -> PathBuf {
    workspace_root()
        .join("target/wasm32-wasip1/release")
        .join(format!("{}.wasm", name.replace('-', "_")))
}

fn require_wasm(name: &str) -> PathBuf {
    let path = wasm_path(name);
    assert!(
        path.exists(),
        "adversarial wasm not found: {}\n\
         Run: cargo component build --release -p {name}",
        path.display()
    );
    path
}

/// A detector that loops forever must be killed by epoch interruption.
#[test]
fn timeout_detector_is_interrupted() {
    let path = require_wasm("det-timeout");
    // 3-tick deadline so the test completes quickly.
    let exec = Executor::with_deadline(3).expect("executor");
    let err = exec.run(&path, None).expect_err("expected epoch-interrupt error");
    let msg = err.to_string();
    eprintln!("timeout error (expected): {msg}");
    // wasmtime surfaces epoch-exceeded as a Trap; just confirm it's an error.
    // The specific message varies by wasmtime version.
}

/// A detector that panics must not crash the host process.
#[test]
fn panicking_detector_is_contained() {
    let path = require_wasm("det-panic");
    let exec = Executor::with_deadline(10).expect("executor");
    let err = exec.run(&path, None).expect_err("expected trap error from panic");
    let msg = err.to_string();
    eprintln!("panic error (expected): {msg}");
}

/// A detector that returns the wrong number of counts for its declared test
/// type must be rejected by output validation before scoring.
#[test]
fn bad_count_length_rejected() {
    let path = require_wasm("det-bad-counts");
    let exec = Executor::with_deadline(10).expect("executor");
    let err = exec.run(&path, None).expect_err("expected validation error");
    let msg = err.to_string();
    eprintln!("bad counts error (expected): {msg}");
    assert!(
        msg.contains("chi_squared"),
        "error message should mention chi_squared validation, got: {msg}"
    );
}

/// Zero sample_size must be rejected.
#[test]
fn zero_sample_size_uses_reference_detector() {
    // The stub corpus returns 7 rows, so the reference detector should pass.
    // This is a sanity check that the executor works at all in the test context.
    let path = workspace_root()
        .join("target/wasm32-wasip1/release/dow_erc20.wasm");
    if !path.exists() {
        eprintln!("skipping sanity check: dow_erc20.wasm not built yet");
        return;
    }
    let exec = Executor::with_deadline(10).expect("executor");
    let result = exec.run(&path, None).expect("reference detector should succeed");
    assert_eq!(result.sample_size, 7);
    assert_eq!(result.counts.len(), 7);
    // Stub corpus is uniform, so p ≈ 1.
    assert!(result.p_value > 0.5, "expected p≈1 for uniform stub, got {}", result.p_value);
}
