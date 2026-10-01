//! Indents the corpus in `indent/` the way `xtask/indent.lua` has Neovim do it, and compares
//! with Neovim's results in `indent/expected` (regenerate with `cargo xtask indent gen`).
//!
//! `FLUX_INDENT=name` runs only the corpus files whose name contains `name`.

use std::path::{Path, PathBuf};

use flux_view::Editor;
use flux_vim::{Engine, parse_keys};

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/indent")
}

fn stripped(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| l.trim_start().to_string())
        .collect()
}

/// An editor on a fresh directory holding `name` (with `lines`, or missing).
fn editor_on(name: &str, lines: Option<&[String]>, tag: &str) -> Editor {
    let dir = std::env::temp_dir()
        .join(format!("flux-indent-{}", std::process::id()))
        .join(format!("{tag}-{name}"));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    if let Some(lines) = lines {
        let mut text = lines.join("\n");
        text.push('\n');
        std::fs::write(dir.join(name), text).unwrap();
    }
    let mut editor = Editor::new(80, 24);
    editor.cwd = dir;
    editor.open(Path::new(name));
    editor
}

fn feed(editor: &mut Editor, keys: &str) {
    let mut engine = Engine::new();
    for key in parse_keys(keys) {
        engine.handle_key(editor, key);
    }
}

fn lines_of(editor: &Editor) -> Vec<String> {
    let text = &editor.current_buffer().text;
    (0..text.line_count())
        .map(|l| text.line_str(l).into_owned())
        .collect()
}

/// The first few lines that differ, numbered from 1.
fn diff(got: &[String], want: &[String]) -> Option<String> {
    if got == want {
        return None;
    }
    let mut out = Vec::new();
    for i in 0..got.len().max(want.len()) {
        let (g, w) = (got.get(i), want.get(i));
        if g != w {
            out.push(format!(
                "    {:>4}: flux {:?}\n          nvim {:?}",
                i + 1,
                g.map_or("(none)", String::as_str),
                w.map_or("(none)", String::as_str)
            ));
            if out.len() == 8 {
                break;
            }
        }
    }
    Some(out.join("\n"))
}

#[test]
fn indenting_matches_neovim() {
    let only = std::env::var("FLUX_INDENT").ok();
    let mut names: Vec<String> = std::fs::read_dir(corpus())
        .unwrap()
        .filter_map(|e| {
            let e = e.ok()?;
            e.file_type()
                .ok()?
                .is_file()
                .then(|| e.file_name().to_string_lossy().into_owned())
        })
        .filter(|n| !n.starts_with('.'))
        .filter(|n| only.as_deref().is_none_or(|o| n.contains(o)))
        .collect();
    names.sort();
    let mut failures = Vec::new();
    for name in &names {
        let lines = stripped(&corpus().join(name));
        let expected = |kind: &str| -> Vec<String> {
            let path = corpus().join("expected").join(format!("{name}.{kind}"));
            std::fs::read_to_string(&path)
                .unwrap_or_else(|_| {
                    panic!(
                        "{} is missing; run `cargo xtask indent gen`",
                        path.display()
                    )
                })
                .lines()
                .map(str::to_owned)
                .collect()
        };

        let mut editor = editor_on(name, Some(&lines), "reindent");
        feed(&mut editor, "gg=G");
        if let Some(d) = diff(&lines_of(&editor), &expected("reindent")) {
            failures.push(format!("{name} (gg=G):\n{d}"));
        }

        let mut editor = editor_on(name, None, "typed");
        let typed: Vec<String> = lines.iter().map(|l| l.replace('<', "<lt>")).collect();
        feed(
            &mut editor,
            &format!(":setlocal fo-=r fo-=o tw=0<CR>i{}<Esc>", typed.join("<CR>")),
        );
        if let Some(d) = diff(&lines_of(&editor), &expected("typed")) {
            failures.push(format!("{name} (typed):\n{d}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} indent results differ from Neovim:\n{}",
        failures.len(),
        names.len() * 2,
        failures.join("\n")
    );
}
