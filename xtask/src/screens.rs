//! `cargo xtask screens`: open each file in `xtask/screens/` in Neovim and in flux, each in its
//! own tmux session, and compare the screens cell by cell, colors and attributes included.
//!
//! Neovim gets the parsers flux compiles in (built from the same grammar crates) and flux's
//! queries through its 'runtimepath', and tree-sitter is started for every sample, so both
//! editors highlight with the same grammar and queries.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::{nvim_bin, root};

const SIZE: (u16, u16) = (80, 24);

/// A cell's look, as tmux reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Sgr {
    fg: Option<String>,
    bg: Option<String>,
    bold: bool,
    italic: bool,
    /// Underline style: 1 straight, 3 curly (`4:3`), …; 0 none.
    underline: u8,
    /// Underline color (`58`).
    sp: Option<String>,
    strikethrough: bool,
    reverse: bool,
    /// An OSC 8 hyperlink's URL.
    link: Option<String>,
}

impl fmt::Display for Sgr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "fg={} bg={}",
            self.fg.as_deref().unwrap_or("-"),
            self.bg.as_deref().unwrap_or("-")
        )?;
        for (on, name) in [
            (self.bold, "bold"),
            (self.italic, "italic"),
            (self.strikethrough, "strike"),
            (self.reverse, "reverse"),
        ] {
            if on {
                write!(f, " {name}")?;
            }
        }
        match self.underline {
            0 => {}
            1 => write!(f, " underline")?,
            n => write!(f, " underline:{n}")?,
        }
        if let Some(sp) = &self.sp {
            write!(f, " sp={sp}")?;
        }
        if let Some(url) = &self.link {
            write!(f, " link={url}")?;
        }
        Ok(())
    }
}

impl Sgr {
    /// Apply the parameters of one `ESC [ … m`.
    fn apply(&mut self, params: &str) {
        let nums: Vec<&str> = params.split(';').collect();
        let mut i = 0;
        while i < nums.len() {
            let p = nums[i];
            let n: u32 = p.split(':').next().unwrap_or("0").parse().unwrap_or(0);
            match n {
                0 => {
                    *self = Sgr {
                        link: self.link.take(),
                        ..Sgr::default()
                    }
                }
                1 => self.bold = true,
                3 => self.italic = true,
                4 => self.underline = p.split_once(':').map_or(1, |(_, s)| s.parse().unwrap_or(1)),
                7 => self.reverse = true,
                9 => self.strikethrough = true,
                22 => self.bold = false,
                23 => self.italic = false,
                24 => self.underline = 0,
                59 => self.sp = None,
                27 => self.reverse = false,
                29 => self.strikethrough = false,
                30..=37 => self.fg = Some(format!("ansi{}", n - 30)),
                90..=97 => self.fg = Some(format!("ansi{}", n - 90 + 8)),
                40..=47 => self.bg = Some(format!("ansi{}", n - 40)),
                100..=107 => self.bg = Some(format!("ansi{}", n - 100 + 8)),
                39 => self.fg = None,
                49 => self.bg = None,
                38 | 48 | 58 => {
                    let color = match nums.get(i + 1) {
                        Some(&"2") if i + 4 < nums.len() => {
                            let c = format!(
                                "#{:02x}{:02x}{:02x}",
                                num(nums[i + 2]),
                                num(nums[i + 3]),
                                num(nums[i + 4])
                            );
                            i += 4;
                            c
                        }
                        Some(&"5") if i + 2 < nums.len() => {
                            let c = format!("ansi{}", nums[i + 2]);
                            i += 2;
                            c
                        }
                        _ => String::new(),
                    };
                    match n {
                        38 => self.fg = Some(color),
                        48 => self.bg = Some(color),
                        _ => self.sp = Some(color),
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
}

fn num(s: &str) -> u8 {
    s.parse().unwrap_or(0)
}

/// A screen as `(char, look)` per row, and where the cursor is.
struct Screen {
    rows: Vec<Vec<(char, Sgr)>>,
    cursor: String,
}

fn parse_capture(s: &str) -> Vec<Vec<(char, Sgr)>> {
    let mut rows = Vec::new();
    let mut sgr = Sgr::default();
    for line in s.lines() {
        let mut row = Vec::new();
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            // OSC sequences: an OSC 8 hyperlink applies to the cells that follow.
            if c == '\x1b' && chars.peek() == Some(&']') {
                chars.next();
                let mut body = String::new();
                for p in chars.by_ref() {
                    if p == '\x07' {
                        break;
                    }
                    if p == '\\' && body.ends_with('\x1b') {
                        body.pop();
                        break;
                    }
                    body.push(p);
                }
                if let Some(rest) = body.strip_prefix("8;") {
                    let url = rest.split_once(';').map_or("", |(_, u)| u);
                    sgr.link = (!url.is_empty()).then(|| url.to_string());
                }
                continue;
            }
            if c == '\x1b' && chars.peek() == Some(&'[') {
                chars.next();
                let mut params = String::new();
                for p in chars.by_ref() {
                    if p.is_ascii_alphabetic() {
                        if p == 'm' {
                            sgr.apply(&params);
                        }
                        break;
                    }
                    params.push(p);
                }
                continue;
            }
            row.push((c, sgr.clone()));
        }
        rows.push(row);
    }
    rows
}

fn tmux(args: &[&str]) -> Result<String> {
    let out = Command::new("tmux")
        .args(args)
        .output()
        .context("running tmux (install it to compare screens)")?;
    if !out.status.success() {
        bail!(
            "tmux {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Run `cmd` in a fresh tmux session of the comparison's size and capture its screen.
/// Run `cmd` in a fresh tmux session of the comparison's size, type `keys` (see
/// [`screens`]), and capture its screen.
fn capture(cmd: &str, dir: &Path, truecolor: bool, keys: &str) -> Result<Screen> {
    let session = format!("flux-screens-{}", std::process::id());
    let (w, h) = SIZE;
    // tmux passes its own environment on, so 16 colors needs COLORTERM taken out.
    let (colorterm, cmd) = if truecolor {
        ("COLORTERM=truecolor", cmd.to_string())
    } else {
        ("COLORTERM=", format!("env -u COLORTERM {cmd}"))
    };
    tmux(&[
        "new-session",
        "-d",
        "-s",
        &session,
        "-x",
        &w.to_string(),
        "-y",
        &h.to_string(),
        "-c",
        &dir.display().to_string(),
        "-e",
        colorterm,
        &cmd,
    ])?;
    sleep(Duration::from_millis(1200));
    for line in keys.lines() {
        match line.strip_prefix("keys: ") {
            Some(names) => {
                let mut args = vec!["send-keys", "-t", &session];
                args.extend(names.split_whitespace());
                tmux(&args)?;
            }
            None => {
                // tmux takes a `;` ending an argument as the end of its command.
                let literal = match line.strip_suffix(';') {
                    Some(rest) => format!("{rest}\\;"),
                    None => line.to_string(),
                };
                tmux(&["send-keys", "-t", &session, "-l", &literal])?;
            }
        }
        sleep(Duration::from_millis(300));
    }
    sleep(Duration::from_millis(300));
    let screen = tmux(&["capture-pane", "-p", "-e", "-t", &session]);
    let cursor = tmux(&["display", "-p", "-t", &session, "#{cursor_x},#{cursor_y}"]);
    tmux(&["kill-session", "-t", &session]).ok();
    Ok(Screen {
        rows: parse_capture(&screen?),
        cursor: cursor?.trim().to_string(),
    })
}

/// Where each parser's C sources are, relative to its crate.
const PARSERS: &[(&str, &str, &str)] = &[
    ("bash", "tree-sitter-bash", "src"),
    ("c", "tree-sitter-c", "src"),
    ("javascript", "tree-sitter-javascript", "src"),
    ("json", "tree-sitter-json", "src"),
    ("lua", "tree-sitter-lua", "src"),
    ("markdown", "tree-sitter-md", "tree-sitter-markdown/src"),
    (
        "markdown_inline",
        "tree-sitter-md",
        "tree-sitter-markdown-inline/src",
    ),
    ("python", "tree-sitter-python", "src"),
    ("rust", "tree-sitter-rust", "src"),
    ("toml", "tree-sitter-toml-ng", "src"),
    ("tsx", "tree-sitter-typescript", "tsx/src"),
    ("typescript", "tree-sitter-typescript", "typescript/src"),
];

/// A runtime directory for Neovim with flux's parsers (`parser/*.so`, compiled from the
/// grammar crates flux uses) and queries.
fn neovim_runtime() -> Result<PathBuf> {
    let dir = root().join("target/xtask-runtime");
    let out = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1"])
        .current_dir(root())
        .output()?;
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let crate_dir = |name: &str| -> Option<PathBuf> {
        meta["packages"].as_array()?.iter().find_map(|p| {
            (p["name"] == name).then(|| {
                PathBuf::from(p["manifest_path"].as_str().unwrap_or_default())
                    .parent()
                    .map(Path::to_path_buf)
            })?
        })
    };
    std::fs::create_dir_all(dir.join("parser"))?;
    for (lang, krate, src) in PARSERS {
        let so = dir.join(format!("parser/{lang}.so"));
        if so.exists() {
            continue;
        }
        let src = crate_dir(krate)
            .with_context(|| format!("{krate} not in cargo metadata"))?
            .join(src);
        let mut cc = Command::new("cc");
        cc.args(["-O2", "-shared", "-fPIC", "-w", "-I"])
            .arg(&src)
            .arg(src.join("parser.c"));
        if src.join("scanner.c").exists() {
            cc.arg(src.join("scanner.c"));
        }
        if !cc.arg("-o").arg(&so).status()?.success() {
            bail!("compiling the {lang} parser failed");
        }
    }
    let queries = dir.join("queries");
    std::fs::remove_dir_all(&queries).ok();
    let ours = root().join("crates/flux-syntax/queries");
    for entry in std::fs::read_dir(&ours)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let to = queries.join(entry.file_name());
        std::fs::create_dir_all(&to)?;
        for f in std::fs::read_dir(entry.path())? {
            let f = f?;
            std::fs::copy(f.path(), to.join(f.file_name()))?;
        }
    }
    Ok(dir)
}

/// The parser for a sample, from its extension (as flux detects it).
fn sample_lang(name: &str) -> Option<&'static str> {
    let ext = Path::new(name).extension()?.to_str()?;
    Some(match ext {
        "sh" => "bash",
        "c" => "c",
        "js" => "javascript",
        "json" => "json",
        "lua" => "lua",
        "md" => "markdown",
        "py" => "python",
        "rs" => "rust",
        "toml" => "toml",
        "tsx" => "tsx",
        "ts" => "typescript",
        _ => return None,
    })
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Compare two screens; one line per differing cell.
fn diff(nvim: &Screen, ours: &Screen) -> Vec<String> {
    let mut diffs = Vec::new();
    if nvim.cursor != ours.cursor {
        diffs.push(format!(
            "  cursor: nvim at {}, flux at {}",
            nvim.cursor, ours.cursor
        ));
    }
    let (nvim, ours) = (&nvim.rows, &ours.rows);
    for (y, (a, b)) in nvim.iter().zip(ours).enumerate() {
        for x in 0..a.len().max(b.len()) {
            let (ca, cb) = (a.get(x), b.get(x));
            if ca != cb {
                let show = |c: Option<&(char, Sgr)>| match c {
                    Some((ch, s)) => format!("{ch:?} {s}"),
                    None => "(nothing)".into(),
                };
                diffs.push(format!(
                    "  {y:>2}:{x:<3} nvim {}\n         flux {}",
                    show(ca),
                    show(cb)
                ));
            }
        }
    }
    if nvim.len() != ours.len() {
        diffs.push(format!(
            "  {} rows in Neovim, {} in flux",
            nvim.len(),
            ours.len()
        ));
    }
    diffs
}

/// Both editors' language server config for a sample with a fake server script
/// (`NAME.lsp.json`, see `flux-lsp-fake`): Neovim's as a Lua file to source, flux's as a JSON
/// file for `$FLUX_LSP_CONFIG`.
fn lsp_configs(name: &str, script: &Path) -> Result<(PathBuf, PathBuf)> {
    let dir = root().join("target/xtask-lsp");
    std::fs::create_dir_all(&dir)?;
    let fake = root().join("target/debug/flux-lsp-fake");
    let filetype = match sample_lang(name) {
        Some("bash") => "sh",
        Some(l) => l,
        None => "text",
    };
    let lua = dir.join(format!("{name}.lua"));
    std::fs::write(
        &lua,
        format!(
            "vim.lsp.config('fake', {{ cmd = {{ '{}', '{}' }}, filetypes = {{ '{filetype}' }} }})\n\
             vim.lsp.enable('fake')\n",
            fake.display(),
            script.display()
        ),
    )?;
    let json = dir.join(format!("{name}.json"));
    std::fs::write(
        &json,
        serde_json::json!([{
            "name": "fake",
            "cmd": [fake.display().to_string(), script.display().to_string()],
            "filetypes": [filetype],
        }])
        .to_string(),
    )?;
    Ok((lua, json))
}

/// Every file in `xtask/screens/` is opened in both editors, with 24-bit and with 16 colors.
/// A file `NAME.keys` next to it is typed after opening: each line literally, except lines
/// starting `keys: `, which are tmux key names (`keys: Escape C-w v`). With `NAME.lsp.json`,
/// both editors run `flux-lsp-fake` with that script as the file's language server.
/// `filter` keeps only the samples whose name contains it.
#[allow(clippy::print_stdout)]
pub fn screens(filter: Option<&str>) -> Result<()> {
    let status = Command::new(env!("CARGO"))
        .args([
            "build",
            "--quiet",
            "--package",
            "flux",
            "--package",
            "flux-lsp",
        ])
        .current_dir(root())
        .status()?;
    if !status.success() {
        bail!("building flux failed");
    }
    let flux = root().join("target/debug/flux");
    let runtime = neovim_runtime()?;
    let dir = root().join("xtask/screens");
    let mut samples: Vec<_> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
        .filter(|n| !n.ends_with(".keys") && !n.ends_with(".lsp.json"))
        .filter(|n| filter.is_none_or(|f| n.contains(f)))
        .collect();
    samples.sort();
    let mut failed = 0;
    let mut total = 0;
    for name in &samples {
        let keys = std::fs::read_to_string(dir.join(format!("{name}.keys"))).unwrap_or_default();
        let start = match sample_lang(name) {
            Some(lang) => format!(
                " --cmd 'set rtp^={}' -c 'lua vim.treesitter.start(0, \"{lang}\")'",
                runtime.display()
            ),
            None => String::new(),
        };
        let script = dir.join(format!("{name}.lsp.json"));
        let (start, env) = if script.exists() {
            let (lua, json) = lsp_configs(name, &script)?;
            (
                format!(" --cmd 'luafile {}'{start}", lua.display()),
                format!(
                    "env FLUX_LSP_CONFIG={} ",
                    shell_quote(&json.display().to_string())
                ),
            )
        } else {
            // No servers, as in `nvim --clean` (flux would start the installed ones).
            let none = root().join("target/xtask-lsp/none.json");
            std::fs::create_dir_all(none.parent().expect("has a parent"))?;
            std::fs::write(&none, "[]")?;
            (
                start,
                format!(
                    "env FLUX_LSP_CONFIG={} ",
                    shell_quote(&none.display().to_string())
                ),
            )
        };
        for truecolor in [true, false] {
            total += 1;
            let nvim = capture(
                &format!("{} --clean -n{start} {}", nvim_bin(), shell_quote(name)),
                &dir,
                truecolor,
                &keys,
            )?;
            let ours = capture(
                &format!(
                    "{env}{} {}",
                    shell_quote(&flux.display().to_string()),
                    shell_quote(name)
                ),
                &dir,
                truecolor,
                &keys,
            )?;
            let diffs = diff(&nvim, &ours);
            let mode = if truecolor { "24-bit" } else { "16 colors" };
            if diffs.is_empty() {
                println!("{name} ({mode}): identical");
            } else {
                failed += 1;
                println!("{name} ({mode}): {} cells differ", diffs.len());
                for d in diffs.iter().take(40) {
                    println!("{d}");
                }
            }
        }
    }
    if failed > 0 {
        bail!("{failed} of {total} screens differ from Neovim");
    }
    Ok(())
}
