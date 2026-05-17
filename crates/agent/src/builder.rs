//! Build a generated detector crate into a .wasm component.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

const WIT_WORLD: &str = include_str!("../../../wit/superstition.wit");

/// Write the detector source files and run `cargo component build --release`.
///
/// Returns the path to the built .wasm file.
pub fn build_detector(
    workspace_root: &Path,
    name: &str,
    lib_rs: &str,
) -> Result<PathBuf> {
    let crate_dir = workspace_root.join("generated").join(name);
    fs::create_dir_all(crate_dir.join("src"))?;
    fs::create_dir_all(crate_dir.join("wit"))?;

    // rust-toolchain.toml — pin same stable toolchain as workspace
    fs::write(
        crate_dir.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"stable\"\ntargets = [\"wasm32-wasip2\"]\n",
    )?;

    // Cargo.toml — standalone; [workspace] prevents cargo inheriting the
    // parent workspace and avoids the "believes it's in a workspace" error.
    let wit_name = name.replace('_', "-"); // WIT labels require dashes, not underscores
    fs::write(
        crate_dir.join("Cargo.toml"),
        format!(
            r#"[workspace]

[package]
name = "{name}"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen-rt = {{ version = "0.44.0", features = ["bitflags"] }}

[package.metadata.component]
package = "superstition:{wit_name}"
"#
        ),
    )?;

    // wit/world.wit — same canonical interface
    fs::write(crate_dir.join("wit").join("world.wit"), WIT_WORLD)?;

    // src/lib.rs — generated code
    fs::write(crate_dir.join("src").join("lib.rs"), lib_rs)?;

    // cargo component build --release
    let output = Command::new("cargo")
        .args(["component", "build", "--release"])
        .current_dir(&crate_dir)
        .output()
        .context("running cargo component build (is cargo-component installed?)")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(anyhow::anyhow!("build failed:\n{stderr}"));
    }

    // Artifact is at target/wasm32-wasip1/release/<name>.wasm
    // (cargo-component uses wasip1 internally then wraps into a component)
    let wasm_name = name.replace('-', "_");
    let wasm_path = crate_dir
        .join("target")
        .join("wasm32-wasip1")
        .join("release")
        .join(format!("{wasm_name}.wasm"));

    if !wasm_path.exists() {
        // Some versions output to wasip2
        let alt = crate_dir
            .join("target")
            .join("wasm32-wasip2")
            .join("release")
            .join(format!("{wasm_name}.wasm"));
        if alt.exists() {
            return Ok(alt);
        }
        anyhow::bail!("build succeeded but wasm not found at {}", wasm_path.display());
    }

    Ok(wasm_path)
}
