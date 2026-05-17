//! Persistent agent state — survives across invocations.
//!
//! Saved as a JSON file alongside the feed.  On each run the agent loads the
//! state, uses it to avoid re-generating hypotheses it already tried, and
//! feeds back significant discoveries to the hypothesis generator (Stage H).

use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::codegen::Hypothesis;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AgentState {
    /// Every hypothesis ever attempted (deduped by name).
    pub seen: Vec<Hypothesis>,
    /// Patterns that passed FDR + effect-size gate — the Stage H feedback set.
    pub significant: Vec<SignificantRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignificantRecord {
    pub name: String,
    pub description: String,
    pub family: String,
    pub p_value: f64,
    pub effect_size: f64,
    pub pattern_id: String,
}

impl AgentState {
    pub fn load(path: &Path) -> Result<Self> {
        if path.exists() {
            let bytes = std::fs::read(path)?;
            Ok(serde_json::from_slice(&bytes)?)
        } else {
            Ok(Self::default())
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        std::fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    pub fn add_seen(&mut self, hyp: Hypothesis) {
        if !self.seen.iter().any(|h| h.name == hyp.name) {
            self.seen.push(hyp);
        }
    }

    pub fn add_significant(&mut self, rec: SignificantRecord) {
        if !self.significant.iter().any(|s| s.name == rec.name) {
            self.significant.push(rec);
        }
    }
}
