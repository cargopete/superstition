//! LLM client — delegates to the `claude` CLI already installed by Claude Code.
//! No API key required; uses whatever auth Claude Code has.

use std::process::Command;

use anyhow::{Context, Result};

pub struct Client;

impl Client {
    pub fn from_env() -> Result<Self> {
        // Verify the CLI is reachable before committing to the run.
        let status = Command::new("claude")
            .arg("--version")
            .output()
            .context("claude CLI not found — is Claude Code installed?")?;
        if !status.status.success() {
            anyhow::bail!("claude --version failed");
        }
        Ok(Self)
    }

    /// Run `claude -p` with the given system + user prompt concatenated.
    ///
    /// `--model` is passed through so Haiku vs Sonnet selection is respected.
    pub fn complete(
        &self,
        model: &str,
        system: &str,
        user: &str,
        _max_tokens: u32,
    ) -> Result<String> {
        // Combine system + user into a single prompt.
        // claude -p reads the prompt from stdin or a --print argument.
        let full_prompt = format!("{}\n\n---\n\n{}", system.trim(), user.trim());

        let out = Command::new("claude")
            .args([
                "-p",
                &full_prompt,
                "--model", model,
                "--output-format", "text",
            ])
            .output()
            .context("running claude CLI")?;

        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            anyhow::bail!("claude CLI exited with {}: {stderr}", out.status);
        }

        let text = String::from_utf8(out.stdout).context("claude CLI output was not UTF-8")?;
        Ok(text.trim().to_string())
    }
}
