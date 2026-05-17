#[allow(warnings)]
mod bindings;

use bindings::exports::superstition::detector::detector::{
    DetectorError, Guest, Metadata, TestResult,
};
use bindings::superstition::detector::corpus::CorpusHandle;

struct DetPanic;

impl Guest for DetPanic {
    fn describe() -> Metadata {
        Metadata {
            description: "Adversarial: deliberate panic".to_string(),
            hypothesis: "Host should handle detector traps without crashing.".to_string(),
            family: "adversarial".to_string(),
            version: "0.1.0".to_string(),
        }
    }

    fn test(_handle: &CorpusHandle) -> Result<TestResult, DetectorError> {
        panic!("intentional panic for adversarial testing")
    }
}

bindings::export!(DetPanic with_types_in bindings);
