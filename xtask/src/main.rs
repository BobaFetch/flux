//! Project automation. `cargo xtask oracle gen|check` runs the Vim test cases through real Neovim.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::{env, fs};

use anyhow::{Context, Result, bail};
use serde_json::Value;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["oracle", "gen"] => oracle_gen(),
        ["oracle", "check"] => oracle_check(),
        _ => bail!(
            "usage: cargo xtask oracle gen    # rewrite expected.json from Neovim\n       cargo xtask oracle check  # verify expected.json still matches Neovim"
        ),
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn oracle_dir() -> PathBuf {
    root().join("crates/flux-vim/tests/oracle")
}

fn nvim_bin() -> String {
    env::var("NVIM_BIN").unwrap_or_else(|_| "nvim".into())
}

/// Run every case through Neovim and return its results, one JSON object per case.
fn run_nvim() -> Result<Vec<Value>> {
    let nvim = nvim_bin();
    let out = env::temp_dir().join(format!("flux-oracle-{}.json", std::process::id()));
    // Neovim echoes messages while running the cases; keep them only for a failure report.
    let output = Command::new(&nvim)
        .args(["--headless", "--clean", "-l"])
        .arg(root().join("xtask/oracle.lua"))
        .arg(oracle_dir().join("cases.json"))
        .arg(&out)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("running {nvim}; install Neovim or set NVIM_BIN"))?;
    if !output.status.success() {
        bail!(
            "{nvim} exited with {}:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let json = fs::read_to_string(&out)?;
    fs::remove_file(&out).ok();
    match serde_json::from_str(&json)? {
        Value::Array(results) => Ok(results),
        _ => bail!("unexpected oracle output"),
    }
}

/// One case per line, so diffs show exactly which cases changed.
fn format_results(results: &[Value]) -> String {
    let lines: Vec<String> = results.iter().map(|r| format!("  {r}")).collect();
    format!("[\n{}\n]\n", lines.join(",\n"))
}

fn nvim_version() -> String {
    Command::new(nvim_bin())
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.lines().next().map(str::to_owned))
        .unwrap_or_default()
}

#[allow(clippy::print_stdout)]
fn oracle_gen() -> Result<()> {
    let results = run_nvim()?;
    fs::write(oracle_dir().join("expected.json"), format_results(&results))?;
    fs::write(oracle_dir().join("NVIM_VERSION"), nvim_version() + "\n")?;
    println!("wrote {} expectations ({})", results.len(), nvim_version());
    Ok(())
}

#[allow(clippy::print_stdout)]
fn oracle_check() -> Result<()> {
    let fresh = run_nvim()?;
    let committed: Vec<Value> =
        serde_json::from_str(&fs::read_to_string(oracle_dir().join("expected.json"))?)?;
    let differing: Vec<&str> = fresh
        .iter()
        .filter(|r| !committed.contains(r))
        .filter_map(|r| r["id"].as_str())
        .collect();
    if fresh.len() != committed.len() || !differing.is_empty() {
        bail!(
            "expected.json is out of date with {} ({} of {} cases differ: {}). Run `cargo xtask oracle gen`.",
            nvim_version(),
            differing.len(),
            fresh.len(),
            differing.join(", ")
        );
    }
    println!("all {} expectations match {}", fresh.len(), nvim_version());
    Ok(())
}
