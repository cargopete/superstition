#[allow(warnings)]
mod bindings;

use bindings::exports::superstition::detector::detector::{
    DetectorError, Guest, Metadata, TestResult, TestType,
};
use bindings::superstition::detector::corpus::CorpusHandle;

struct DetBadCounts;

impl Guest for DetBadCounts {
    fn describe() -> Metadata {
        Metadata {
            description: "Adversarial: mismatched counts vs test type".to_string(),
            hypothesis: "Host should reject counts that don't match the declared test type."
                .to_string(),
            family: "adversarial".to_string(),
            version: "0.1.0".to_string(),
        }
    }

    fn test(_handle: &CorpusHandle) -> Result<TestResult, DetectorError> {
        // ChiSquared(df=6) requires df+1 = 7 counts; we return only 3.
        Ok(TestResult {
            counts: vec![10, 20, 30],
            sample_size: 60,
            test_type: TestType::ChiSquared(6),
            detail: "deliberately wrong count length".to_string(),
        })
    }
}

bindings::export!(DetBadCounts with_types_in bindings);
