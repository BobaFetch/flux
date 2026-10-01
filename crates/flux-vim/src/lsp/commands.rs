//! The keys Neovim maps for diagnostics and language servers (`]d`, `K`, `grr`, …).

use flux_view::Editor;

use crate::engine::Engine;
use crate::normal::set_pcmark;
use crate::parse::LspCmd;
use crate::util::pos;

impl Engine {
    pub(crate) fn lsp_command(&mut self, editor: &mut Editor, cmd: LspCmd, count: Option<usize>) {
        let n = count.unwrap_or(1).max(1) as isize;
        match cmd {
            LspCmd::DiagnosticNext => diagnostic_jump(editor, n, true),
            LspCmd::DiagnosticPrev => diagnostic_jump(editor, -n, true),
            LspCmd::DiagnosticLast => diagnostic_jump(editor, isize::MAX, false),
            LspCmd::DiagnosticFirst => diagnostic_jump(editor, -isize::MAX, false),
            LspCmd::DiagnosticFloat => diagnostic_float(editor),
            LspCmd::Hover => super::hover::request(editor),
            LspCmd::References
            | LspCmd::Implementation
            | LspCmd::TypeDefinition
            | LspCmd::DocumentSymbol
            | LspCmd::Definition => super::locations::request(editor, cmd),
            LspCmd::Rename => super::rename::start(self, editor),
            LspCmd::CodeAction => super::code_action::request(editor),
        }
    }
}

/// Neovim's `vim.diagnostic.jump({ count, wrap })`: `count` diagnostics on (back if negative),
/// from the cursor. A line's diagnostics are in column order; columns past the end of the line
/// count as its last character.
fn diagnostic_jump(editor: &mut Editor, mut count: isize, wrap: bool) {
    let buffer = editor.window.buffer;
    let text = &editor.current_buffer().text;
    let byte = |line: usize, col: usize| -> usize {
        let s = text.line_str(line);
        s.char_indices().nth(col).map_or(s.len(), |(i, _)| i)
    };
    let diags: Vec<(usize, usize, usize)> = editor
        .buffer_diagnostics(buffer)
        .into_iter()
        .map(|(s, _, _)| (s.line, byte(s.line, s.col), s.col))
        .collect();
    let line_count = text.line_count();
    let cur = editor.cursor();
    let mut at = (cur.line, byte(cur.line, cur.col));
    let mut found: Option<(usize, usize, usize)> = None;
    while count != 0 {
        let forward = count > 0;
        let mut next = None;
        for i in 0..=line_count {
            let lnum = at.0 as isize + if forward { i as isize } else { -(i as isize) };
            let lnum = if lnum < 0 || lnum >= line_count as isize {
                if !wrap {
                    break;
                }
                lnum.rem_euclid(line_count as isize)
            } else {
                lnum
            } as usize;
            let mut on_line: Vec<_> = diags.iter().filter(|d| d.0 == lnum).copied().collect();
            if on_line.is_empty() {
                continue;
            }
            on_line.sort_by_key(|d| d.1);
            if !forward {
                on_line.reverse();
            }
            if i == 0 {
                let len = text.line_str(lnum).len();
                let clamp = |c: usize| c.min(len.saturating_sub(1));
                next = on_line.into_iter().find(|d| {
                    if forward {
                        clamp(d.1) > at.1
                    } else {
                        clamp(d.1) < at.1
                    }
                });
                if next.is_some() {
                    break;
                }
            } else {
                next = on_line.first().copied();
                break;
            }
        }
        let Some(d) = next else {
            break;
        };
        at = (d.0, d.1);
        found = Some(d);
        count -= count.signum();
    }
    match found {
        Some((line, _, col)) => {
            set_pcmark(editor);
            editor.window.cursor = pos(line, col);
            editor.window.set_curswant = true;
        }
        None => editor.warning("No more valid diagnostics to move to"),
    }
}

/// `CTRL-W d`: Neovim's `vim.diagnostic.open_float()` for the cursor line: a `Diagnostics:`
/// header, then each diagnostic numbered, in its severity's color, with its code.
fn diagnostic_float(editor: &mut Editor) {
    use flux_view::float::FloatHighlight;
    if editor.floats.iter().any(|f| f.focus_id == "line") {
        return;
    }
    let line = editor.cursor().line;
    let buffer = editor.window.buffer;
    let on_line: Vec<(u8, String, Option<String>)> = editor
        .buffer_diagnostics(buffer)
        .into_iter()
        .filter(|(s, e, _)| {
            line >= s.line && line <= e.line && (s.line == e.line || line != e.line || e.col != 0)
        })
        .map(|(_, _, d)| (d.severity, d.message.clone(), d.code.clone()))
        .collect();
    if on_line.is_empty() {
        return;
    }
    let mut lines = vec!["Diagnostics:".to_string()];
    let mut highlights = Vec::new();
    for (i, (severity, message, code)) in on_line.iter().enumerate() {
        let prefix = format!("{}. ", i + 1);
        let suffix = code.as_ref().map(|c| format!(" [{c}]")).unwrap_or_default();
        let group = ["Error", "Warn", "Info", "Hint"][usize::from(severity - 1)];
        let message_lines: Vec<&str> = message.split('\n').collect();
        for (j, m) in message_lines.iter().enumerate() {
            let pre = if j == 0 {
                prefix.clone()
            } else {
                " ".repeat(prefix.len())
            };
            let suf = if j + 1 == message_lines.len() {
                suffix.as_str()
            } else {
                ""
            };
            let n = lines.len();
            // The prefix has its own highlight on the first line only.
            let a = if j == 0 { pre.chars().count() } else { 0 };
            let b = pre.chars().count() + m.chars().count();
            highlights.push(FloatHighlight {
                line: n,
                start: a,
                end: b,
                group: format!("DiagnosticFloating{group}"),
            });
            lines.push(format!("{pre}{m}{suf}"));
        }
    }
    editor.open_float(lines, None, highlights, "line");
}
