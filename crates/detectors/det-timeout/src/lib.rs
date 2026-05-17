#[allow(warnings)]
mod bindings;

use bindings::exports::superstition::detector::detector::{
    DetectorError, Guest, Metadata, TestResult,
};
use bindings::superstition::detector::corpus::CorpusHandle;

struct DetTimeout;

impl Guest for DetTimeout {
    fn describe() -> Metadata {
        Metadata {
            description: "Adversarial: infinite loop".to_string(),
            hypothesis: "Host epoch interruption should terminate this.".to_string(),
            family: "adversarial".to_string(),
            version: "0.1.0".to_string(),
        }
    }

    fn test(_handle: &CorpusHandle) -> Result<TestResult, DetectorError> {
        // Spin forever — epoch interruption must kill this.
        #[allow(clippy::empty_loop)]
        loop {}
    }
}

bindings::export!(DetTimeout with_types_in bindings);
