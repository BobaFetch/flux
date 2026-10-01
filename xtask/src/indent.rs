//! `cargo xtask indent gen|check`: Neovim's indenting of the corpus in
//! `crates/flux-vim/tests/indent` (see `xtask/indent.lua`), which flux's `tests/indent.rs`
//! compares against.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::{nvim_bin, nvim_version, root};

fn corpus() -> PathBuf {
    root().join("crates/flux-vim/tests/indent")
}

/// Run the corpus through Neovim into a fresh directory.
fn run() -> Result<PathBuf> {
    let out = std::env::temp_dir().join(format!("flux-indent-{}", std::process::id()));
    fs::remove_dir_all(&out).ok();
    fs::create_dir_all(&out)?;
    let nvim = nvim_bin();
    let output = Command::new(&nvim)
        .args(["--headless", "--clean", "-l"])
        .arg(root().join("xtask/indent.lua"))
        .arg(corpus())
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
    Ok(out)
}

#[allow(clippy::print_stdout)]
pub fn gen_expected() -> Result<()> {
    let out = run()?;
    let expected = corpus().join("expected");
    fs::remove_dir_all(&expected).ok();
    fs::create_dir_all(&expected)?;
    let mut n = 0;
    for entry in fs::read_dir(&out)? {
        let entry = entry?;
        fs::copy(entry.path(), expected.join(entry.file_name()))?;
        n += 1;
    }
    fs::remove_dir_all(&out).ok();
    println!("wrote {n} expectations ({})", nvim_version());
    Ok(())
}

#[allow(clippy::print_stdout)]
pub fn check() -> Result<()> {
    let out = run()?;
    let expected = corpus().join("expected");
    let mut stale = Vec::new();
    for entry in fs::read_dir(&out)? {
        let entry = entry?;
        let committed = fs::read_to_string(expected.join(entry.file_name())).unwrap_or_default();
        if fs::read_to_string(entry.path())? != committed {
            stale.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    fs::remove_dir_all(&out).ok();
    if !stale.is_empty() {
        bail!(
            "indent expectations are out of date with {}: {}. Run `cargo xtask indent gen`.",
            nvim_version(),
            stale.join(", ")
        );
    }
    println!("indent expectations match {}", nvim_version());
    Ok(())
}
