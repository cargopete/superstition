#[allow(warnings)]
mod bindings;

use bindings::exports::superstition::detector::detector::{
    DetectorError, Guest, Metadata, TestResult, TestType,
};
use bindings::superstition::detector::corpus::{self, CorpusHandle, Value};

struct DowErc20;

impl Guest for DowErc20 {
    fn describe() -> Metadata {
        Metadata {
            description: "ERC-20 transfers by day of week".to_string(),
            hypothesis: "ERC-20 transfer counts are not uniformly distributed \
                         across days of the week."
                .to_string(),
            family: "temporal-cyclic".to_string(),
            version: "0.1.0".to_string(),
        }
    }

    fn test(handle: &CorpusHandle) -> Result<TestResult, DetectorError> {
        let mut counts = vec![0u64; 7];
        let mut total = 0u64;

        let stream = corpus::iterator(handle, "erc20_transfers")
            .map_err(|e| DetectorError::CorpusError(format!("{e:?}")))?;

        while let Some(row) = stream.next() {
            let ts = row
                .fields
                .iter()
                .find(|(k, _)| k == "block_timestamp")
                .and_then(|(_, v)| match v {
                    Value::U64Val(t) => Some(*t),
                    _ => None,
                });

            if let Some(t) = ts {
                // 1970-01-01 was a Thursday (day 4); bias so Sunday = 0.
                let dow = ((t / 86400 + 4) % 7) as usize;
                counts[dow] += 1;
                total += 1;
            }
        }

        // Format detail before moving counts into TestResult.
        let detail = format!(
            "Sun={} Mon={} Tue={} Wed={} Thu={} Fri={} Sat={}",
            counts[0], counts[1], counts[2], counts[3], counts[4], counts[5], counts[6],
        );

        Ok(TestResult {
            counts,
            sample_size: total,
            test_type: TestType::ChiSquared(6), // df = k - 1
            detail,
        })
    }
}

bindings::export!(DowErc20 with_types_in bindings);
