use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};

// ── Pattern ───────────────────────────────────────────────────────────────────

/// A published, significant pattern — the atom of the feed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    /// First 16 hex of BLAKE3(wasm_bytes ‖ corpus_id) — stable content address.
    pub id: String,
    /// BLAKE3 hex of the .wasm bytes (first 16 chars).
    pub wasm_hash: String,
    /// Content-addressed corpus identifier.
    pub corpus_id: String,
    pub description: String,
    pub hypothesis: String,
    pub family: String,
    pub counts: Vec<u64>,
    pub sample_size: u64,
    pub detail: String,
    /// Raw p-value from the statistical test — deterministically reproducible.
    pub p_value: f64,
    /// BH-adjusted q-value from the batch in which this pattern was discovered.
    pub q_value: f64,
    pub effect_size: f64,
    /// RFC 2822 timestamp (RSS-compatible).
    pub published_at: String,
    /// Base64-encoded .wasm bytes — self-contained for offline verification.
    pub wasm_b64: String,
}

impl Pattern {
    pub fn new(
        wasm_bytes: &[u8],
        corpus_id: &str,
        description: String,
        hypothesis: String,
        family: String,
        counts: Vec<u64>,
        sample_size: u64,
        detail: String,
        p_value: f64,
        q_value: f64,
        effect_size: f64,
    ) -> Self {
        let wasm_hash = blake3::hash(wasm_bytes).to_hex()[..16].to_string();

        let mut hasher = blake3::Hasher::new();
        hasher.update(wasm_bytes);
        hasher.update(corpus_id.as_bytes());
        let id = hasher.finalize().to_hex()[..16].to_string();

        let published_at = chrono::Utc::now().to_rfc2822();
        let wasm_b64 = STANDARD.encode(wasm_bytes);

        Self {
            id,
            wasm_hash,
            corpus_id: corpus_id.to_string(),
            description,
            hypothesis,
            family,
            counts,
            sample_size,
            detail,
            p_value,
            q_value,
            effect_size,
            published_at,
            wasm_b64,
        }
    }

    pub fn wasm_bytes(&self) -> Result<Vec<u8>> {
        STANDARD.decode(&self.wasm_b64).map_err(Into::into)
    }

    /// The one-liner `verify` command a reader can paste into their terminal.
    pub fn verify_cmd(&self) -> String {
        format!("superstition-verify {}", self.id)
    }
}

// ── FeedStore ─────────────────────────────────────────────────────────────────

/// JSON-backed append-only pattern store.
///
/// Thread-safety is the caller's responsibility (use `Arc<Mutex<FeedStore>>`
/// or just serialise writes).  For M5 single-process use, plain reads are safe.
pub struct FeedStore {
    path: PathBuf,
}

impl FeedStore {
    pub fn open(path: impl AsRef<Path>) -> Self {
        Self { path: path.as_ref().to_owned() }
    }

    pub fn patterns(&self) -> Result<Vec<Pattern>> {
        if !self.path.exists() {
            return Ok(vec![]);
        }
        let data = fs::read_to_string(&self.path)?;
        Ok(serde_json::from_str::<Vec<Pattern>>(&data).unwrap_or_default())
    }

    pub fn publish(&self, pattern: Pattern) -> Result<()> {
        let mut patterns = self.patterns()?;
        patterns.retain(|p| p.id != pattern.id); // deduplicate
        patterns.push(pattern);
        // newest first
        patterns.sort_by(|a, b| b.published_at.cmp(&a.published_at));
        fs::write(&self.path, serde_json::to_string_pretty(&patterns)?)?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<Option<Pattern>> {
        Ok(self.patterns()?.into_iter().find(|p| p.id == id))
    }
}
