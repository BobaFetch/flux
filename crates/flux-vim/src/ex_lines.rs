//! Ex commands that work on lines: `:d`, `:y`, `:>`, `:<`, `:m`, `:t`/`:co`, `:j`, `:pu`, `:p`,
//! `:nu`, `:=`, `:k`, `:norm`, `:u`, `:red`, `:noh`. Ported from Vim's `ex_docmd.c` and
//! `ex_cmds.c`.

use flux_core::Edit;
use flux_view::{Editor, Mode, Register, RegisterKind};

use crate::engine::Engine;
use crate::ex::Args;
use crate::key::{Key, KeyCode};
use crate::motion::Want;
use crate::normal::{Range, lines_text, normalize_cursor, set_pcmark, set_want, yank};
use crate::util::{self, pos};

/// Put the cursor on 0-based `line` in the column vertical moves aim for (Vim's
/// `beginline(BL_SOL | BL_FIX)` with 'nostartofline').
fn to_line_keep_column(editor: &mut Editor, line: usize) {
    let line = line.min(editor.text().last_line());
    editor.with_window(|win, m| {
        win.update_curswant(m, false);
        win.cursor.line = line;
        win.cursor.col = m.col_for_vcol(line, win.curswant);
    });
}

/// Put the cursor on the first non-blank of 0-based `line` (Vim's `beginline(BL_WHITE |
/// BL_FIX)`).
fn to_first_non_blank(editor: &mut Editor, line: usize) {
    let line = line.min(editor.text().last_line());
    let col = util::first_non_blank(&util::line(editor, line));
    editor.window.cursor = pos(line, col);
    set_want(editor, Want::Column);
}

fn line_range(a: &Args) -> Range {
    Range {
        start: pos(a.line1 - 1, 0),
        end: pos(a.line2 - 1, 0),
        linewise: true,
        inclusive: false,
        numbered_register: false,
    }
}

/// `:[range]d[elete] [x] [count]`
pub(crate) fn delete(engine: &mut Engine, editor: &mut Editor, a: &Args) {
    if !crate::ex::no_args(editor, a.args) {
        return;
    }
    set_pcmark(editor);
    to_line_keep_column(editor, a.line1 - 1);
    engine.delete(editor, line_range(a), a.register);
    // Unlike `dd`, `:d` leaves the cursor on the first non-blank.
    let line = editor.cursor().line;
    to_first_non_blank(editor, line);
}

/// `:[range]y[ank] [x] [count]`
pub(crate) fn yank_lines(_engine: &mut Engine, editor: &mut Editor, a: &Args) {
    if !crate::ex::no_args(editor, a.args) {
        return;
    }
    let cursor = editor.cursor();
    yank(editor, line_range(a), a.register);
    editor.window.cursor = cursor;
}

/// `:[range]> [count]`, `:>>`, `:<`: shift the lines by as many 'shiftwidth's as there are
/// `>`s; the cursor ends on the last line.
pub(crate) fn shift(engine: &mut Engine, editor: &mut Editor, a: &Args) {
    if !crate::ex::no_args(editor, a.args) {
        return;
    }
    let right = a.name.starts_with('>');
    let amount = a.name.chars().count();
    set_pcmark(editor);
    to_line_keep_column(editor, a.line1 - 1);
    let sw = editor.buf_opts().sw();
    let ts = editor.buf_opts().tabstop;
    for line in a.line1 - 1..a.line2 {
        let s = util::line(editor, line);
        if s.is_empty() {
            continue;
        }
        let indent = util::indent_of(&s);
        let width = util::indent_width(&s, ts);
        let new = if right {
            width + sw * amount
        } else {
            width.saturating_sub(sw * amount)
        };
        let new_indent = util::make_indent(new, editor.buf_opts());
        if new_indent != indent {
            let start = editor.text().line_start(line);
            let end = start + indent.chars().count();
            engine.edit(editor, Edit::replace(start..end, new_indent));
        }
    }
    to_first_non_blank(editor, a.line2 - 1);
    let n = a.line2 - a.line1 + 1;
    if n > editor.options.report {
        let op = if right { '>' } else { '<' };
        let times = if amount == 1 { "time" } else { "times" };
        editor.info(format!("{} {op}ed {amount} {times}", util::lines(n)));
    }
}

/// The destination address of `:m` and `:t`.
fn destination(editor: &mut Editor, arg: &str) -> Option<usize> {
    match crate::ex::parse_single_address(editor, arg) {
        Ok(Some(n)) if n >= 0 && n as usize <= editor.text().line_count() => Some(n as usize),
        Ok(_) => {
            editor.error("E16: Invalid range");
            None
        }
        Err(e) => {
            if !e.is_empty() {
                editor.error(e);
            }
            None
        }
    }
}

/// `:[range]m[ove] {address}`: move the lines below `{address}`. Marks on them move along.
pub(crate) fn move_lines(engine: &mut Engine, editor: &mut Editor, a: &Args) {
    let Some(dest) = destination(editor, a.args) else {
        return;
    };
    let (line1, line2) = (a.line1, a.line2);
    if dest >= line1 && dest < line2 {
        editor.error("E134: Cannot move a range of lines into itself");
        return;
    }
    let count = line2 - line1 + 1;
    let cursor_line = if dest >= line1 { dest } else { dest + count };
    if dest == line1 - 1 || dest == line2 {
        to_line_keep_column(editor, cursor_line - 1);
        return;
    }
    // Where each moved line ends up (0-based), for its marks.
    let new_line = |l: usize| {
        if dest > line2 {
            l + dest - line2
        } else {
            l - (line1 - 1 - dest)
        }
    };
    let buffer_id = editor.window.buffer;
    let moved_marks: Vec<(char, flux_view::Cursor)> = editor
        .current_buffer()
        .marks
        .sorted()
        .into_iter()
        .filter(|(_, p)| p.line >= line1 - 1 && p.line < line2)
        .collect();
    let moved_global: Vec<(char, flux_view::Cursor)> = editor
        .global_marks
        .iter()
        .filter(|(_, (b, p))| *b == buffer_id && p.line >= line1 - 1 && p.line < line2)
        .map(|(&c, &(_, p))| (c, p))
        .collect();

    let text = lines_text(editor, line1 - 1, line2 - 1);
    let t = editor.text();
    if dest > line2 {
        // Copy after line `dest`, then delete the originals above it.
        let at = t.line_start(dest - 1) + t.line_len(dest - 1);
        engine.edit(
            editor,
            Edit::insert(at, format!("\n{}", text.trim_end_matches('\n'))),
        );
        let t = editor.text();
        let from = t.line_start(line1 - 1);
        let to = t.line_start(line2);
        engine.edit(editor, Edit::delete(from..to));
    } else {
        // Copy above line `dest + 1`, then delete the originals, now `count` lines further.
        let at = t.line_start(dest);
        engine.edit(
            editor,
            Edit::insert(at, format!("{}\n", text.trim_end_matches('\n'))),
        );
        let t = editor.text();
        let (first, last) = (line1 - 1 + count, line2 - 1 + count);
        let edit = if last < t.last_line() {
            Edit::delete(t.line_start(first)..t.line_start(last + 1))
        } else {
            Edit::delete(t.line_start(first) - 1..t.len_chars())
        };
        engine.edit(editor, edit);
    }
    let buffer = editor.current_buffer_mut();
    for (c, p) in moved_marks {
        buffer.marks.set(
            c,
            flux_view::Cursor {
                line: new_line(p.line),
                col: p.col,
            },
        );
    }
    for (c, p) in moved_global {
        editor.global_marks.insert(
            c,
            (
                buffer_id,
                flux_view::Cursor {
                    line: new_line(p.line),
                    col: p.col,
                },
            ),
        );
    }
    let (start, end) = if dest > line2 {
        (dest - count, dest - 1)
    } else {
        (dest, dest + count - 1)
    };
    let buffer = editor.current_buffer_mut();
    buffer.marks.set('[', pos(start, 0));
    buffer.marks.set(']', pos(end, 0));
    if count > editor.options.report {
        editor.info(format!("{} moved", util::lines(count)));
    }
    to_line_keep_column(editor, cursor_line - 1);
}

/// `:[range]t {address}` / `:co[py]`: copy the lines below `{address}`.
pub(crate) fn copy_lines(engine: &mut Engine, editor: &mut Editor, a: &Args) {
    let Some(dest) = destination(editor, a.args) else {
        return;
    };
    let count = a.line2 - a.line1 + 1;
    let text = lines_text(editor, a.line1 - 1, a.line2 - 1);
    let t = editor.text();
    if dest == 0 {
        engine.edit(
            editor,
            Edit::insert(0, format!("{}\n", text.trim_end_matches('\n'))),
        );
    } else {
        let at = t.line_start(dest - 1) + t.line_len(dest - 1);
        engine.edit(
            editor,
            Edit::insert(at, format!("\n{}", text.trim_end_matches('\n'))),
        );
    }
    let buffer = editor.current_buffer_mut();
    buffer.marks.set('[', pos(dest, 0));
    buffer.marks.set(']', pos(dest + count - 1, 0));
    if let Some(msg) = util::more_lines_message(count as isize, editor.options.report) {
        editor.more_info(msg);
    }
    to_line_keep_column(editor, dest + count - 1);
}

/// `:[range]j[oin][!] [count]`
pub(crate) fn join(engine: &mut Engine, editor: &mut Editor, a: &Args) {
    if !crate::ex::no_args(editor, a.args) {
        return;
    }
    let (line1, mut line2) = (a.line1, a.line2);
    editor.window.cursor = pos(line1 - 1, 0);
    if line1 == line2 {
        if a.addr_count >= 2 {
            return;
        }
        if line2 == editor.text().line_count() {
            engine.failed = true;
            return;
        }
        line2 += 1;
    }
    engine.join(editor, Some(line2 - line1 + 1), !a.bang);
    to_first_non_blank(editor, line1 - 1);
}

/// `:[line]pu[t][!] [x]`: put a register as lines below (`!`: above) the line; line 0 puts
/// above the first line. The cursor ends on the last new line.
pub(crate) fn put(engine: &mut Engine, editor: &mut Editor, a: &Args) {
    let (mut line, mut above) = (a.line2, a.bang);
    if line == 0 {
        line = 1;
        above = true;
    }
    let Some(reg) = editor.register(a.register).filter(|r| !r.text.is_empty()) else {
        let name = a.register.unwrap_or('"');
        editor.error(format!("E353: Nothing in register {name}"));
        return;
    };
    let text = reg.text.trim_end_matches('\n').to_string();
    let lines = text.split('\n').count();
    let reg = Register::new(text, RegisterKind::Line);
    // Like Vim's `ex_put`, only the line changes (so undo comes back to the column).
    editor.window.cursor.line = line - 1;
    normalize_cursor(editor);
    engine.put_register(editor, &reg, 1, above);
    let last = if above {
        line - 1 + lines - 1
    } else {
        line + lines - 1
    };
    to_first_non_blank(editor, last);
}

/// Show lines as `:p` does, optionally numbered (`:nu`, `:#`, `:p #`).
fn print_lines(editor: &mut Editor, a: &Args, number: bool) {
    let flags = a.args.trim();
    let number = number || flags.contains('#');
    let ts = editor.buf_opts().tabstop;
    let width = editor.text().line_count().to_string().len().max(3);
    let mut out = Vec::new();
    for line in a.line1 - 1..a.line2 {
        let s = util::line(editor, line);
        let mut text = String::new();
        let mut col = 0;
        for c in s.chars() {
            if c == '\t' {
                let n = ts - col % ts;
                text.push_str(&" ".repeat(n));
                col += n;
            } else {
                text.push(c);
                col += 1;
            }
        }
        out.push(if number {
            format!("{:>width$} {text}", line + 1)
        } else {
            text
        });
    }
    to_line_keep_column(editor, a.line2 - 1);
    editor.full_message(out.join("\n"));
}

/// `:[range]p[rint]`
pub(crate) fn print(editor: &mut Editor, a: &Args) {
    print_lines(editor, a, false);
}

/// `:[range]nu[mber]`, `:[range]#`
pub(crate) fn number(editor: &mut Editor, a: &Args) {
    print_lines(editor, a, true);
}

/// `:[range]=`: the last line number of the range (the buffer's last line without one).
pub(crate) fn equal(editor: &mut Editor, a: &Args) {
    editor.info(a.line2.to_string());
}

/// `:[range]k{a-z}`, `:[range]ma[rk] {a-z}`: set a mark on the first non-blank of the last line.
pub(crate) fn mark(editor: &mut Editor, a: &Args) {
    let arg = a.args.trim();
    let mut chars = arg.chars();
    let Some(name) = chars.next() else {
        editor.error("E471: Argument required");
        return;
    };
    if chars.next().is_some() {
        editor.error(format!("E488: Trailing characters: {arg}"));
        return;
    }
    let line = a.line2 - 1;
    let p = pos(line, util::first_non_blank(&util::line(editor, line)));
    match name {
        'a'..='z' | '[' | ']' | '<' | '>' | '"' | '^' | '.' => {
            editor.current_buffer_mut().marks.set(name, p)
        }
        'A'..='Z' => {
            let buffer = editor.window.buffer;
            editor.global_marks.insert(name, (buffer, p));
        }
        '\'' | '`' => {
            let cur = editor.cursor();
            editor.window.cursor = p;
            set_pcmark(editor);
            editor.window.cursor = cur;
        }
        _ => editor.error("E191: Argument must be a letter or forward/backward quote"),
    }
}

/// `:[range]norm[al][!] {commands}`: run Normal-mode keys, once or on every line of the range
/// (starting at its first column). An unfinished command is ended as with `<Esc>`.
pub(crate) fn normal(engine: &mut Engine, editor: &mut Editor, a: &Args) {
    if a.args.is_empty() {
        editor.error("E471: Argument required");
        return;
    }
    let keys: Vec<Key> = a.args.chars().map(Key::char).collect();
    let run = |engine: &mut Engine, editor: &mut Editor| {
        for &key in &keys {
            engine.process_key(editor, key);
        }
        // Finish whatever is left open: a pending command, Insert or Visual mode, a command line.
        if engine.has_pending() || editor.mode != Mode::Normal {
            engine.process_key(editor, Key::plain(KeyCode::Esc));
        }
        if editor.mode == Mode::CmdLine {
            engine.process_key(editor, Key::plain(KeyCode::Esc));
        }
    };
    // Everything `:normal` changes is undone at once.
    engine.hold_undo += 1;
    if a.addr_count == 0 {
        run(engine, editor);
        engine.hold_undo -= 1;
        return;
    }
    // Like Vim, the line numbers of the range are used as they were given.
    for line in a.line1 - 1..a.line2 {
        if line >= editor.text().line_count() {
            break;
        }
        editor.window.cursor = pos(line, 0);
        set_want(editor, Want::Column);
        run(engine, editor);
    }
    engine.hold_undo -= 1;
    normalize_cursor(editor);
}

/// `:u[ndo]`
pub(crate) fn undo(engine: &mut Engine, editor: &mut Editor, _a: &Args) {
    engine.undo(editor, 1, false);
}

/// `:red[o]`
pub(crate) fn redo(engine: &mut Engine, editor: &mut Editor, _a: &Args) {
    engine.undo(editor, 1, true);
}

/// `:noh[lsearch]`: stop highlighting matches until the next search.
pub(crate) fn nohlsearch(editor: &mut Editor, _a: &Args) {
    editor.search.no_hlsearch = true;
}
