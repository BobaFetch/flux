//! `<Tab>` completion on the `:` command line (Vim's wildmode `full`: the first
//! match first, then cycling through the rest and back to the typed text).

use std::path::PathBuf;

use flux_view::{Editor, Wildmenu};

use crate::engine::Engine;
use crate::ex::{CompleteKind, CompletionTarget};

/// Candidates for a completion target, in cycle order.
fn candidates(editor: &Editor, target: &CompletionTarget) -> Vec<String> {
    match target.kind {
        CompleteKind::Command => {
            let mut names = crate::ex::command_names();
            names.sort();
            names
                .into_iter()
                .filter(|n| n.starts_with(&target.prefix))
                .map(str::to_string)
                .collect()
        }
        CompleteKind::File => file_candidates(editor, &target.prefix),
        CompleteKind::Buffer => editor
            .buffers
            .iter()
            .filter(|b| b.listed && !b.directory)
            .map(|b| b.name())
            .filter(|n| n.starts_with(&target.prefix))
            .collect(),
        CompleteKind::Option => option_candidates(&target.prefix),
        CompleteKind::Register => "\"0123456789abcdefghijklmnopqrstuvwxyz-*+.:%"
            .chars()
            .map(|c| c.to_string())
            .filter(|n| n.starts_with(&target.prefix))
            .collect(),
        CompleteKind::Lsp => ["enable", "disable", "restart", "stop"]
            .iter()
            .filter(|n| n.starts_with(&target.prefix))
            .map(ToString::to_string)
            .collect(),
    }
}

/// File candidates for `prefix` (relative to the working directory, `~/`, or
/// absolute): directories gain a trailing `/` so the next `<Tab>` descends.
fn file_candidates(editor: &Editor, prefix: &str) -> Vec<String> {
    if prefix == "~" {
        return vec!["~/".to_string()];
    }
    let (dir_part, file_prefix) = match prefix.rfind('/') {
        Some(i) => (&prefix[..=i], &prefix[i + 1..]),
        None => ("", prefix),
    };
    let dir: PathBuf = if dir_part.is_empty() {
        editor.cwd.clone()
    } else if let Some(rest) = dir_part.strip_prefix("~/") {
        match std::env::var_os("HOME") {
            Some(home) => PathBuf::from(home).join(rest),
            None => return Vec::new(),
        }
    } else if dir_part.starts_with('/') {
        PathBuf::from(dir_part)
    } else {
        editor.cwd.join(dir_part)
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(file_prefix) {
            continue;
        }
        // A symlink to a directory completes with `/`, like a directory.
        let is_dir = entry
            .file_type()
            .is_ok_and(|t| t.is_dir() || (t.is_symlink() && dir.join(&name).is_dir()));
        if is_dir {
            out.push(format!("{dir_part}{name}/"));
        } else {
            out.push(format!("{dir_part}{name}"));
        }
    }
    out.sort();
    out
}

/// Option names (and short names, `no`/`inv` bool forms) for `:set`.
fn option_candidates(prefix: &str) -> Vec<String> {
    // A value position (`ts=`, `sw+`, `nu?`) completes nothing.
    if prefix
        .chars()
        .any(|c| matches!(c, '=' | '+' | '-' | '^' | '?' | '!' | '&'))
    {
        return Vec::new();
    }
    let mut out = Vec::new();
    for o in flux_view::options::OPTIONS {
        for form in [o.name, o.short] {
            if !form.is_empty() && form.starts_with(prefix) {
                out.push(form.to_string());
            }
        }
        if o.kind == flux_view::options::Kind::Bool {
            for form in [format!("no{}", o.name), format!("inv{}", o.name)] {
                if form.starts_with(prefix) {
                    out.push(form);
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn charslice(s: &str, from: usize, to: usize) -> String {
    s.chars().take(to).skip(from).collect()
}

/// Replace chars `[start, end)` of the command line, leaving the cursor after
/// the replacement.
fn replace_word(editor: &mut Editor, start: usize, end: usize, text: &str) {
    let mut line = editor.cmdline.clone();
    let at = |s: &str, n: usize| s.char_indices().nth(n).map_or(s.len(), |(i, _)| i);
    let from = at(&line, start);
    let to = at(&line, end.max(start));
    line.replace_range(from..to, text);
    editor.cmdline = line;
    editor.cmdline_pos = start + text.chars().count();
}

/// `<Tab>` (`reverse` for `<S-Tab>`): complete the word before the cursor,
/// cycling `full`-style through matches and back to the typed text.
pub(crate) fn cycle(engine: &mut Engine, editor: &mut Editor, reverse: bool) {
    if let Some(wild) = editor.wildmenu.clone() {
        step_wildmenu(editor, wild, reverse);
        return;
    }
    let line = editor.cmdline.clone();
    let pos = editor.cmdline_pos;
    let Some(target) = crate::ex::completion_target(&line, pos) else {
        engine.failed = true;
        return;
    };
    let items = candidates(editor, &target);
    if items.is_empty() {
        engine.failed = true;
        return;
    }
    if items.len() == 1 {
        replace_word(editor, target.start, target.end, &items[0]);
        return;
    }
    let selected = if reverse { items.len() - 1 } else { 0 };
    let original = charslice(&line, target.start, target.end);
    replace_word(editor, target.start, target.end, &items[selected]);
    editor.wildmenu = Some(Wildmenu {
        items,
        selected: Some(selected),
        original,
        start: target.start,
    });
}

/// Step an open wildmenu (past the last match comes the original text, as in
/// Vim's `full` mode).
fn step_wildmenu(editor: &mut Editor, wild: Wildmenu, reverse: bool) {
    let n = wild.items.len();
    let next: Option<usize> = match (wild.selected, reverse) {
        (Some(i), false) if i + 1 < n => Some(i + 1),
        (Some(_), false) => None,
        (None, false) => Some(0),
        (Some(0), true) => None,
        (Some(i), true) => Some(i - 1),
        (None, true) => Some(n - 1),
    };
    // File and buffer names can contain spaces, so the replaced span is the shown
    // match's length rather than the whitespace word after `start`.
    let shown_len = match wild.selected {
        Some(i) => wild.items[i].chars().count(),
        None => wild.original.chars().count(),
    };
    let end = wild.start + shown_len;
    let text = match next {
        Some(i) => wild.items[i].clone(),
        None => wild.original.clone(),
    };
    replace_word(editor, wild.start, end, &text);
    editor.wildmenu = Some(Wildmenu {
        selected: next,
        ..wild
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_forms_and_value_positions() {
        assert!(option_candidates("num").contains(&"number".to_string()));
        assert!(option_candidates("nu").contains(&"nu".to_string()));
        assert!(option_candidates("noh").contains(&"nohlsearch".to_string()));
        assert!(option_candidates("invh").contains(&"invhidden".to_string()));
        assert!(option_candidates("ts=").is_empty());
        assert!(option_candidates("zzz").is_empty());
    }

    #[test]
    fn register_and_lsp_lists() {
        let editor = Editor::new(80, 24);
        let target = CompletionTarget {
            kind: CompleteKind::Register,
            prefix: String::new(),
            start: 0,
            end: 0,
        };
        let regs = candidates(&editor, &target);
        assert!(regs.contains(&"a".to_string()));
        assert!(regs.contains(&"+".to_string()));
        let target = CompletionTarget {
            kind: CompleteKind::Lsp,
            prefix: "re".to_string(),
            start: 0,
            end: 0,
        };
        assert_eq!(candidates(&editor, &target), vec!["restart".to_string()]);
    }
}
