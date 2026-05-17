pub mod fdr;
pub mod stats;

use thiserror::Error;

// ── public types ──────────────────────────────────────────────────────────────

/// Mirror of the WIT `test-type` variant; scorer is independent of wasmtime.
#[derive(Debug, Clone)]
pub enum TestType {
    FisherExact,
    ChiSquared(u32),
    KolmogorovSmirnov,
    Bootstrap { statistic_name: String, permutations: u32 },
}

/// Everything a detector returns, minus the WIT resource machinery.
#[derive(Debug, Clone)]
pub struct DetectorOutput {
    pub counts: Vec<u64>,
    pub sample_size: u64,
    pub test_type: TestType,
    pub detail: String,
}

/// A fully-scored result, ready for the feed.
#[derive(Debug, Clone)]
pub struct ScoredResult {
    pub p_value: f64,
    pub effect_size: f64,
    pub passes_effect_floor: bool,
}

#[derive(Debug, Error)]
pub enum ScorerError {
    #[error("statistical test failed: {0}")]
    TestFailed(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
}

// ── score ─────────────────────────────────────────────────────────────────────

/// Score a single detector output.
///
/// Returns `(p_value, effect_size, passes_floor)`.
pub fn score(output: &DetectorOutput) -> Result<ScoredResult, ScorerError> {
    match &output.test_type {
        TestType::ChiSquared(df) => {
            stats::chi_squared_score(&output.counts, output.sample_size, *df)
        }
        TestType::FisherExact => {
            stats::fisher_exact_score(&output.counts)
        }
        TestType::KolmogorovSmirnov => {
            stats::ks_score(&output.counts, output.sample_size)
        }
        TestType::Bootstrap { permutations, .. } => {
            stats::bootstrap_score(&output.counts, output.sample_size, *permutations)
        }
    }
}
