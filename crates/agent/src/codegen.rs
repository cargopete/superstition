//! Hypothesis generation (Haiku) and detector code-gen (Sonnet).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::api::Client;
use crate::state::SignificantRecord;

// Embed at compile time so the binary is self-contained.
const WIT_CONTENT: &str = include_str!("../../../wit/superstition.wit");
const REFERENCE_IMPL: &str = include_str!("../../detectors/dow-erc20/src/lib.rs");

// ── hypothesis type ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hypothesis {
    pub name: String,
    pub description: String,
    pub hypothesis: String,
    pub family: String,
    pub test_type: String,
    pub bins: u32,
    pub notes: String,
}

// ── hypothesis generation (Haiku) ─────────────────────────────────────────────

const HYPO_SYSTEM: &str = r#"
You generate statistical hypotheses about Ethereum EVM blockchain data for the Superstition analytics platform.

Corpus schema:
  Table: erc20_transfers
    block_timestamp  uint64  (Unix seconds since epoch, ~1438000000..present)

Test types available (and when to use each):
  chi_squared(df)  — categorical: k uniform bins, df = k-1
                     e.g. day-of-week (k=7, df=6), hour-of-day (k=24, df=23),
                          weekday/weekend split (k=2, df=1)
  fisher_exact     — 2×2 contingency (EXACTLY 4 counts [a,b,c,d])
                     e.g. AM vs PM × high-timestamp vs low-timestamp
  ks               — continuous uniform comparison (frequency array)
                     e.g. sub-second fractional distribution
  bootstrap        — general permutation (statistic_name + permutations)

Detector constraints:
  - Can only use columns listed above
  - Returns raw COUNTS only (host computes statistics)
  - No network, no clock, no randomness

Return ONLY a JSON array — no prose, no markdown, no fences.
"#;

pub fn generate_hypotheses(
    client: &Client,
    known_patterns: &[Hypothesis],
    significant: &[SignificantRecord],
    n: usize,
) -> Result<Vec<Hypothesis>> {
    let known = if known_patterns.is_empty() {
        "None yet.".to_string()
    } else {
        known_patterns
            .iter()
            .map(|h| format!("- {} ({})", h.description, h.family))
            .collect::<Vec<_>>()
            .join("\n")
    };

    // Stage H: positive-example feedback — what the evolutionary loop found.
    let stage_h = if significant.is_empty() {
        String::new()
    } else {
        let examples = significant
            .iter()
            .map(|s| {
                format!(
                    "  - {} [family={}, p={:.2e}, V={:.3}]",
                    s.description, s.family, s.p_value, s.effect_size
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "\nPreviously SIGNIFICANT patterns (high value — generate variants, extensions, \
             or related phenomena that may share the same underlying mechanism):\n{examples}\n"
        )
    };

    let user = format!(
        r#"Previously attempted patterns (do NOT repeat these):
{known}
{stage_h}
Generate exactly {n} novel hypotheses NOT in the attempted list above.
Use diverse families and test types.

Return a JSON array of objects with these keys:
  name        — snake_case identifier ≤30 chars
  description — human-readable one-liner
  hypothesis  — scientific null-hypothesis statement to reject
  family      — one of: temporal-cyclic, temporal-trend, value-distribution
  test_type   — one of: chi_squared, fisher_exact, ks, bootstrap
  bins        — number of bins (k for chi_squared, 4 for fisher_exact, any for ks/bootstrap)
  notes       — brief implementation hint (formula, etc.)"#,
        known = known,
        stage_h = stage_h,
        n = n,
    );

    let raw = client.complete(
        "claude-haiku-4-5-20251001",
        HYPO_SYSTEM,
        &user,
        1024,
    )?;

    // Extract JSON array from response (tolerate markdown fences if present)
    let json_str = extract_json_array(&raw)
        .with_context(|| format!("could not find JSON array in hypothesis response:\n{raw}"))?;

    serde_json::from_str::<Vec<Hypothesis>>(&json_str)
        .with_context(|| format!("parsing hypothesis JSON:\n{json_str}"))
}

// ── code generation (Sonnet) ──────────────────────────────────────────────────

const CODEGEN_SYSTEM: &str = r#"
You write Rust WebAssembly detector components for the Superstition analytics platform.

=== WIT INTERFACE ===
{WIT}

=== REFERENCE IMPLEMENTATION ===
{REF}

=== RULES ===
1. Return ONLY Rust source code. NO markdown. NO ```rust fences. NO prose.
2. Use the EXACT import pattern from the reference implementation.
3. test() returns raw COUNTS — never statistics or p-values.
4. Format the `detail` string BEFORE constructing TestResult (avoid borrow-after-move).
5. counts.len() must match the test type:
   chi_squared(df): len == df+1
   fisher_exact:    len == 4 exactly, as [a, b, c, d]
   ks / bootstrap:  len == number of bins
6. Access corpus via corpus::iterator(handle, "erc20_transfers").
7. Get block_timestamp: row.fields.iter().find(|(k,_)| k == "block_timestamp")
   then match on Value::U64Val(t).
8. The exported struct name should be PascalCase of the detector name.
9. Start with #[allow(warnings)] mod bindings; as in the reference.
"#;

pub fn generate_detector_code(client: &Client, hyp: &Hypothesis) -> Result<String> {
    let system = CODEGEN_SYSTEM
        .replace("{WIT}", WIT_CONTENT)
        .replace("{REF}", REFERENCE_IMPL);

    let bins_hint = match hyp.test_type.as_str() {
        "chi_squared" => format!("TestType::ChiSquared({}) with {} counts", hyp.bins - 1, hyp.bins),
        "fisher_exact" => "TestType::FisherExact with exactly 4 counts [a, b, c, d]".to_string(),
        "ks" => format!("TestType::KolmogorovSmirnov with {} bins", hyp.bins),
        _ => format!("TestType::Bootstrap with {} bins", hyp.bins),
    };

    let user = format!(
        r#"Implement this detector:

{hyp_json}

Implementation notes:
  {notes}
  Test type: {bins_hint}

Write the complete src/lib.rs."#,
        hyp_json = serde_json::to_string_pretty(hyp)?,
        notes = hyp.notes,
        bins_hint = bins_hint,
    );

    let raw = client.complete(
        "claude-sonnet-4-6",
        &system,
        &user,
        4096,
    )?;

    Ok(strip_code_fences(&raw))
}

/// Retry code generation with compile error feedback.
pub fn fix_detector_code(
    client: &Client,
    hyp: &Hypothesis,
    broken_code: &str,
    compile_error: &str,
) -> Result<String> {
    let system = CODEGEN_SYSTEM
        .replace("{WIT}", WIT_CONTENT)
        .replace("{REF}", REFERENCE_IMPL);

    let user = format!(
        r#"This detector code failed to compile:

```rust
{broken_code}
```

Compiler error:
```
{compile_error}
```

Hypothesis:
{hyp_json}

Fix ALL errors and return the corrected complete src/lib.rs.
Return ONLY Rust source code. NO markdown. NO fences. NO prose."#,
        broken_code = broken_code,
        compile_error = &compile_error[..compile_error.len().min(3000)],
        hyp_json = serde_json::to_string_pretty(hyp)?,
    );

    let raw = client.complete("claude-sonnet-4-6", &system, &user, 4096)?;
    Ok(strip_code_fences(&raw))
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn extract_json_array(s: &str) -> Option<String> {
    let start = s.find('[')?;
    let end = s.rfind(']')?;
    if end > start {
        Some(s[start..=end].to_string())
    } else {
        None
    }
}

fn strip_code_fences(s: &str) -> String {
    let s = s.trim();
    // Strip ```rust ... ``` or ``` ... ```
    if let Some(inner) = s.strip_prefix("```rust") {
        if let Some(inner) = inner.strip_suffix("```") {
            return inner.trim_start_matches('\n').trim_end().to_string();
        }
    }
    if let Some(inner) = s.strip_prefix("```") {
        if let Some(inner) = inner.strip_suffix("```") {
            return inner.trim_start_matches('\n').trim_end().to_string();
        }
    }
    s.to_string()
}
