//! Ex commands. M1 has what's needed to edit and save one file; ranges and the rest of the
//! command table arrive in M4.

use std::path::PathBuf;

use flux_view::Editor;

/// An Ex command name, and how short Vim lets you abbreviate it (`:w`, `:wq`, `:quita`).
struct Command {
    name: &'static str,
    min_len: usize,
    run: fn(&mut Editor, &Args),
}

const COMMANDS: &[Command] = &[
    Command {
        name: "write",
        min_len: 1,
        run: write,
    },
    Command {
        name: "wq",
        min_len: 2,
        run: write_quit,
    },
    Command {
        name: "wall",
        min_len: 2,
        run: write_all,
    },
    Command {
        name: "wqall",
        min_len: 3,
        run: write_quit_all,
    },
    Command {
        name: "xit",
        min_len: 1,
        run: exit,
    },
    Command {
        name: "xall",
        min_len: 2,
        run: write_quit_all,
    },
    Command {
        name: "exit",
        min_len: 3,
        run: exit,
    },
    Command {
        name: "update",
        min_len: 2,
        run: update,
    },
    Command {
        name: "edit",
        min_len: 1,
        run: edit,
    },
    Command {
        name: "quit",
        min_len: 1,
        run: quit,
    },
    Command {
        name: "qall",
        min_len: 2,
        run: quit_all,
    },
    Command {
        name: "quitall",
        min_len: 5,
        run: quit_all,
    },
    Command {
        name: "checktime",
        min_len: 6,
        run: checktime,
    },
    Command {
        name: "registers",
        min_len: 3,
        run: registers,
    },
    Command {
        name: "display",
        min_len: 2,
        run: registers,
    },
    Command {
        name: "marks",
        min_len: 5,
        run: marks,
    },
    Command {
        name: "delmarks",
        min_len: 4,
        run: delmarks,
    },
    Command {
        name: "jumps",
        min_len: 2,
        run: jumps,
    },
    Command {
        name: "split",
        min_len: 2,
        run: split,
    },
    Command {
        name: "vsplit",
        min_len: 2,
        run: vsplit,
    },
    Command {
        name: "new",
        min_len: 3,
        run: new_window,
    },
    Command {
        name: "vnew",
        min_len: 3,
        run: vnew,
    },
    Command {
        name: "close",
        min_len: 3,
        run: close,
    },
    Command {
        name: "only",
        min_len: 2,
        run: only,
    },
    Command {
        name: "resize",
        min_len: 3,
        run: resize,
    },
    Command {
        name: "enew",
        min_len: 3,
        run: enew,
    },
    Command {
        name: "buffer",
        min_len: 1,
        run: buffer,
    },
    Command {
        name: "bnext",
        min_len: 2,
        run: bnext,
    },
    Command {
        name: "bNext",
        min_len: 2,
        run: bprevious,
    },
    Command {
        name: "bprevious",
        min_len: 2,
        run: bprevious,
    },
    Command {
        name: "bfirst",
        min_len: 2,
        run: bfirst,
    },
    Command {
        name: "brewind",
        min_len: 2,
        run: bfirst,
    },
    Command {
        name: "blast",
        min_len: 2,
        run: blast,
    },
    Command {
        name: "bdelete",
        min_len: 2,
        run: bdelete,
    },
    Command {
        name: "bwipeout",
        min_len: 2,
        run: bwipeout,
    },
    Command {
        name: "ls",
        min_len: 2,
        run: list_buffers,
    },
    Command {
        name: "buffers",
        min_len: 7,
        run: list_buffers,
    },
    Command {
        name: "files",
        min_len: 5,
        run: list_buffers,
    },
];

/// What a command was given besides its name.
pub(crate) struct Args<'a> {
    pub bang: bool,
    pub args: &'a str,
    /// A number before the name (`:5split`, `:2bnext`).
    pub count: Option<usize>,
    /// The `:vertical` modifier.
    pub vertical: bool,
}

pub fn execute(editor: &mut Editor, line: &str) {
    editor.quitmore = editor.quitmore.saturating_sub(1);
    let mut line = line.trim_start_matches([' ', ':']).trim_end();
    if line.is_empty() {
        return;
    }
    let mut vertical = false;
    if let Some(rest) = strip_modifier(line, "vertical", 4) {
        vertical = true;
        line = rest;
    }
    let (count, rest) = parse_range(editor, line);
    let rest = rest.trim_start();
    if rest.is_empty() {
        if let Some(n) = count {
            let target = n.saturating_sub(1);
            editor.with_window(|win, m| win.set_cursor_line(target, m));
        }
        return;
    }

    let name_len = rest
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(rest.len());
    let (name, rest) = rest.split_at(name_len);
    let (bang, args) = match rest.strip_prefix('!') {
        Some(args) => (true, args.trim()),
        None => (false, rest.trim()),
    };
    let command = COMMANDS
        .iter()
        .find(|c| name.len() >= c.min_len && c.name.starts_with(name));
    match command {
        Some(command) => (command.run)(
            editor,
            &Args {
                bang,
                args,
                count,
                vertical,
            },
        ),
        None => editor.error(format!("E492: Not an editor command: {line}")),
    }
}

/// A leading range (`.`, `$`, `%`, numbers, `+N`/`-N` offsets, two addresses separated by
/// `,`), returning its last line (1-based) and the rest of the command. The full range syntax
/// arrives with M4.
fn parse_range<'a>(editor: &Editor, line: &'a str) -> (Option<usize>, &'a str) {
    if let Some(rest) = line.strip_prefix('%') {
        return (Some(editor.text().line_count()), rest);
    }
    let mut rest = line;
    let mut last = None;
    loop {
        let (addr, after) = parse_address(editor, rest);
        if addr.is_some() {
            last = addr;
        }
        rest = after;
        match rest.strip_prefix([',', ';']) {
            Some(after) => rest = after,
            None => break,
        }
    }
    (last, rest)
}

/// One address: a line (`.`, `$`, a number, a mark) and any `+N`/`-N` offsets.
fn parse_address<'a>(editor: &Editor, s: &'a str) -> (Option<usize>, &'a str) {
    let current = editor.cursor().line as isize + 1;
    let digits = |s: &str| s.chars().take_while(char::is_ascii_digit).count();
    let (mut line, mut rest) = if let Some(rest) = s.strip_prefix('.') {
        (Some(current), rest)
    } else if let Some(rest) = s.strip_prefix('$') {
        (Some(editor.text().line_count() as isize), rest)
    } else if let Some(name) = s.strip_prefix('\'').and_then(|r| r.chars().next()) {
        let line = crate::motion::mark_position(editor, name).map(|p| p.line as isize + 1);
        (line, &s[1 + name.len_utf8()..])
    } else {
        let n = digits(s);
        (s[..n].parse::<isize>().ok(), &s[n..])
    };
    while let Some(sign) = rest.chars().next().filter(|c| matches!(c, '+' | '-')) {
        let after = &rest[1..];
        let n = digits(after);
        let offset = after[..n].parse::<isize>().unwrap_or(1);
        let base = line.unwrap_or(current);
        line = Some(if sign == '+' {
            base + offset
        } else {
            base - offset
        });
        rest = &after[n..];
    }
    (line.map(|l| l.max(0) as usize), rest)
}

/// `line` without a leading command modifier like `vert[ical]`.
fn strip_modifier<'a>(line: &'a str, name: &str, min_len: usize) -> Option<&'a str> {
    let word_len = line
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(line.len());
    let word = &line[..word_len];
    (word.len() >= min_len && name.starts_with(word) && word_len < line.len())
        .then(|| line[word_len..].trim_start())
}

fn no_args(editor: &mut Editor, args: &str) -> bool {
    if args.is_empty() {
        return true;
    }
    editor.error(format!("E488: Trailing characters: {args}"));
    false
}

/// Write the buffer, reporting the result. True on success.
fn do_write(editor: &mut Editor, bang: bool, args: &str) -> bool {
    let path = (!args.is_empty()).then(|| PathBuf::from(args));
    let id = editor.window.buffer;
    match write_buffer(editor, id, path.as_deref(), bang) {
        Ok(msg) => {
            editor.file_message(msg);
            true
        }
        Err(msg) => {
            editor.error(msg);
            false
        }
    }
}

fn write(editor: &mut Editor, a: &Args) {
    let bang = a.bang;
    let args = a.args;
    do_write(editor, bang, args);
}

fn write_all(editor: &mut Editor, a: &Args) {
    if no_args(editor, a.args) {
        write_modified_buffers(editor);
    }
}

/// Write every modified buffer that has a file name. False if one failed.
fn write_modified_buffers(editor: &mut Editor) -> bool {
    let ids: Vec<_> = editor
        .buffers
        .iter()
        .filter(|b| b.modified() && b.path.is_some())
        .map(|b| b.id)
        .collect();
    for id in ids {
        match write_buffer(editor, id, None, false) {
            Ok(msg) => editor.file_message(msg),
            Err(msg) => {
                editor.error(msg);
                return false;
            }
        }
    }
    true
}

/// Write buffer `id` (to `target`, or its own file), taking relative names from the editor's
/// directory while keeping the name as typed.
fn write_buffer(
    editor: &mut Editor,
    id: flux_view::BufferId,
    target: Option<&std::path::Path>,
    force: bool,
) -> Result<String, String> {
    let cwd = editor.cwd.clone();
    let target = target.map(|t| cwd.join(t));
    let buffer = editor.buffer_mut(id).ok_or("E86: Buffer does not exist")?;
    let own = buffer.path.clone();
    if let Some(p) = &own {
        buffer.path = Some(cwd.join(p));
    }
    let result = buffer.write(target.as_deref(), force);
    // Keep the name as the user gave it (or name an unnamed buffer as `:w file` typed it).
    buffer.path = match (own, &buffer.path) {
        (Some(p), _) => Some(p),
        (None, Some(written)) => Some(
            written
                .strip_prefix(&cwd)
                .map(|r| r.to_path_buf())
                .unwrap_or_else(|_| written.clone()),
        ),
        (None, None) => None,
    };
    result.map(|msg| msg.replace(&format!("{}/", cwd.display()), ""))
}

fn update(editor: &mut Editor, a: &Args) {
    if editor.current_buffer().modified() || !a.args.is_empty() {
        do_write(editor, a.bang, a.args);
    }
}

fn write_quit(editor: &mut Editor, a: &Args) {
    if do_write(editor, a.bang, a.args) {
        close_or_quit(editor, a.bang);
    }
}

fn write_quit_all(editor: &mut Editor, a: &Args) {
    if no_args(editor, a.args) && write_modified_buffers(editor) && check_modified(editor, false) {
        editor.quit = true;
    }
}

/// `:x`: write only if there are changes, then quit.
fn exit(editor: &mut Editor, a: &Args) {
    let bang = a.bang;
    let args = a.args;
    if (editor.current_buffer().modified() || !args.is_empty()) && !do_write(editor, bang, args) {
        return;
    }
    close_or_quit(editor, a.bang);
}

fn quit(editor: &mut Editor, a: &Args) {
    let bang = a.bang;
    let args = a.args;
    if !no_args(editor, args) {
        return;
    }
    // With other windows open, `:q` closes this one; 'hidden' keeps its buffer.
    if editor.window_ids().len() > 1 {
        let id = editor.window.id;
        editor.close_window(id);
        return;
    }
    if check_more(editor, bang) && check_modified(editor, bang) {
        editor.quit = true;
    }
}

/// `:qa[ll][!]`: quit, whatever windows are open. Without `!`, any modified buffer stops it.
fn quit_all(editor: &mut Editor, a: &Args) {
    if no_args(editor, a.args) && (a.bang || check_modified(editor, false)) {
        editor.quit = true;
    }
}

/// Vim's `check_more`: quitting the last window before every file in the argument list has
/// been edited is `E173`, unless forced or tried twice in a row. True when quitting is fine.
fn check_more(editor: &mut Editor, bang: bool) -> bool {
    let more = editor.args.len().saturating_sub(1);
    if bang || editor.args.len() < 2 || editor.arg_had_last || editor.quitmore > 0 {
        return true;
    }
    let files = if more == 1 { "file" } else { "files" };
    editor.error(format!("E173: {more} more {files} to edit"));
    editor.quitmore = 2;
    false
}

/// The unsaved-changes check before quitting Vim (`check_changed_any`): E37/E162 for the
/// first modified buffer, the current one first. With `bang` only hidden buffers count, since
/// `!` discards the changes of the one being quit. True when nothing is modified.
fn check_modified(editor: &mut Editor, bang: bool) -> bool {
    let current = editor.window.buffer;
    let modified = std::iter::once(current)
        .chain(editor.buffers.iter().map(|b| b.id))
        .filter(|&id| !bang || !editor.is_shown(id))
        .find(|&id| editor.buffer(id).is_some_and(|b| b.modified()));
    let Some(id) = modified else {
        return true;
    };
    let name = editor.buffer(id).map(|b| b.name()).unwrap_or_default();
    editor.error(format!(
        "E37: No write since last change\nE162: No write since last change for buffer \"{name}\""
    ));
    false
}

/// Close the window after a write (`:wq`, `:x`), or quit when it's the last one.
fn close_or_quit(editor: &mut Editor, bang: bool) {
    if editor.window_ids().len() > 1 {
        let id = editor.window.id;
        editor.close_window(id);
    } else if check_more(editor, bang) && check_modified(editor, bang) {
        editor.quit = true;
    }
}

/// `:e[!] [file]`. Without a file, re-read the current one; `!` discards changes.
fn edit(editor: &mut Editor, a: &Args) {
    let (bang, args) = (a.bang, a.args);
    let modified = editor.current_buffer().modified();
    if args.is_empty() {
        if editor.current_buffer().path.is_none() {
            if bang {
                editor.set_text("");
            } else {
                editor.error("E32: No file name");
            }
            return;
        }
        if modified && !bang {
            editor.error("E37: No write since last change (add ! to override)");
            return;
        }
        let cursor = editor.cursor();
        let cwd = editor.cwd.clone();
        let buffer = editor.current_buffer_mut();
        let result = match buffer.path.clone() {
            Some(p) if p.is_relative() => {
                // Reload through the full path, keeping the name as typed.
                buffer.path = Some(cwd.join(&p));
                let r = buffer.reload((cursor.line, cursor.col));
                buffer.path = Some(p);
                r
            }
            _ => buffer.reload((cursor.line, cursor.col)),
        };
        if let Err(e) = result {
            editor.error(format!("\"{}\" {e}", editor.current_buffer().name()));
        }
        editor.with_window(|win, m| {
            let line = win.cursor.line.min(m.text.last_line());
            win.set_cursor_line(line, m);
        });
        return;
    }
    // `:e #`: the alternate buffer.
    if let Some(rest) = args.strip_prefix('#') {
        let target = match rest.parse::<usize>() {
            Ok(n) => Some(flux_view::BufferId(n)),
            Err(_) => editor.window.alt_buffer,
        };
        match target.filter(|&b| editor.buffer(b).is_some()) {
            Some(b) => editor.show_buffer(b),
            None => editor.error("E23: No alternate file"),
        }
        return;
    }
    // 'hidden' is on: the current buffer can be left with changes, unless `!` discards them.
    if bang && modified {
        let cursor = editor.cursor();
        let _ = editor.current_buffer_mut().history.undo();
        editor.current_buffer_mut().uncommitted = false;
        let cwd = editor.cwd.clone();
        let buffer = editor.current_buffer_mut();
        if let Some(p) = buffer.path.clone() {
            buffer.path = Some(cwd.join(&p));
            let _ = buffer.reload((cursor.line, cursor.col));
            buffer.path = Some(p);
        }
    }
    if let Err(e) = editor.edit_file(&PathBuf::from(args)) {
        editor.error(e);
    }
}

fn split(editor: &mut Editor, a: &Args) {
    open_split(editor, a, a.vertical);
}

fn vsplit(editor: &mut Editor, a: &Args) {
    open_split(editor, a, true);
}

fn open_split(editor: &mut Editor, a: &Args, vertical: bool) {
    let old = editor.window.id;
    if editor.split(vertical, a.count) && !a.args.is_empty() {
        editor.in_new_window = true;
        if let Err(e) = editor.edit_file(&PathBuf::from(a.args)) {
            editor.error(e);
        }
        editor.in_new_window = false;
        // The window split from gets the new file as its alternate.
        let current = editor.window.buffer;
        let from = editor.window_mut(old);
        if from.buffer != current {
            from.alt_buffer = Some(current);
        }
    }
}

fn new_window(editor: &mut Editor, a: &Args) {
    new_in_split(editor, a, a.vertical);
}

fn vnew(editor: &mut Editor, a: &Args) {
    new_in_split(editor, a, true);
}

fn new_in_split(editor: &mut Editor, a: &Args, vertical: bool) {
    if editor.split(vertical, a.count) {
        let id = editor.new_buffer();
        editor.in_new_window = true;
        editor.show_buffer(id);
    }
}

fn close(editor: &mut Editor, _a: &Args) {
    let id = editor.window.id;
    if !editor.close_window(id) {
        editor.error("E444: Cannot close last window");
    }
}

fn only(editor: &mut Editor, _a: &Args) {
    editor.only_window();
}

/// `:resize N`, `:resize +N`, `:vertical resize N`.
fn resize(editor: &mut Editor, a: &Args) {
    let current = if a.vertical {
        editor.window.width
    } else {
        editor.window.height
    };
    let arg = a.args;
    let target = if let Some(n) = arg.strip_prefix('+') {
        current + n.parse::<usize>().unwrap_or(1)
    } else if let Some(n) = arg.strip_prefix('-') {
        current
            .saturating_sub(n.parse::<usize>().unwrap_or(1))
            .max(1)
    } else if let Ok(n) = arg.parse::<usize>() {
        n
    } else if let Some(n) = a.count {
        n
    } else {
        usize::MAX / 2
    };
    let id = editor.window.id;
    if a.vertical {
        editor.layout.set_width(id, target);
    } else {
        editor.layout.set_height(id, target);
    }
    editor.sync_window_sizes();
}

fn enew(editor: &mut Editor, _a: &Args) {
    let id = editor.new_buffer();
    editor.show_buffer(id);
}

/// The buffer a `:buffer`/`:bdelete` argument names: a number, or a unique part of a name.
fn find_buffer_arg(editor: &mut Editor, arg: &str) -> Option<flux_view::BufferId> {
    if let Ok(n) = arg.parse::<usize>() {
        let id = flux_view::BufferId(n);
        if editor.buffer(id).is_none() {
            editor.error(format!("E86: Buffer {n} does not exist"));
            return None;
        }
        return Some(id);
    }
    let matches: Vec<_> = editor
        .buffers
        .iter()
        .filter(|b| b.listed && b.name().contains(arg))
        .map(|b| b.id)
        .collect();
    let exact = editor
        .buffers
        .iter()
        .find(|b| b.listed && b.name() == arg)
        .map(|b| b.id);
    match (exact, matches.len()) {
        (Some(id), _) => Some(id),
        (None, 1) => Some(matches[0]),
        (None, 0) => {
            editor.error(format!("E94: No matching buffer for {arg}"));
            None
        }
        _ => {
            editor.error(format!("E93: More than one match for {arg}"));
            None
        }
    }
}

fn buffer(editor: &mut Editor, a: &Args) {
    let target = if a.args.is_empty() {
        a.count.map(flux_view::BufferId)
    } else {
        find_buffer_arg(editor, a.args)
    };
    match target {
        Some(id) if editor.buffer(id).is_some() => editor.show_buffer(id),
        Some(id) => editor.error(format!("E86: Buffer {} does not exist", id.0)),
        None => {}
    }
}

/// `:bnext` and `:bprevious`, `count` buffers along the listed ones, wrapping around.
fn cycle_buffers(editor: &mut Editor, count: usize, forward: bool) {
    let listed = editor.listed_buffers();
    if listed.is_empty() {
        return;
    }
    let current = editor.window.buffer;
    let pos = listed.iter().position(|&b| b == current).unwrap_or(0);
    let n = listed.len();
    let steps = count % n;
    let i = if forward {
        (pos + steps) % n
    } else {
        (pos + n - steps) % n
    };
    editor.show_buffer(listed[i]);
}

fn bnext(editor: &mut Editor, a: &Args) {
    cycle_buffers(editor, a.count.or(a.args.parse().ok()).unwrap_or(1), true);
}

fn bprevious(editor: &mut Editor, a: &Args) {
    cycle_buffers(editor, a.count.or(a.args.parse().ok()).unwrap_or(1), false);
}

fn bfirst(editor: &mut Editor, _a: &Args) {
    if let Some(&id) = editor.listed_buffers().first() {
        editor.show_buffer(id);
    }
}

fn blast(editor: &mut Editor, _a: &Args) {
    if let Some(&id) = editor.listed_buffers().last() {
        editor.show_buffer(id);
    }
}

fn delete_buffers(editor: &mut Editor, a: &Args, wipe: bool) {
    let target = if a.args.is_empty() {
        Some(a.count.map_or(editor.window.buffer, flux_view::BufferId))
    } else {
        find_buffer_arg(editor, a.args)
    };
    if let Some(id) = target
        && let Err(e) = editor.delete_buffer(id, wipe, a.bang)
    {
        editor.error(e);
    }
}

fn bdelete(editor: &mut Editor, a: &Args) {
    delete_buffers(editor, a, false);
}

fn bwipeout(editor: &mut Editor, a: &Args) {
    delete_buffers(editor, a, true);
}

/// `:ls`: `  2 %a + "name"   line 3`, with the line number from column 40.
fn list_buffers(editor: &mut Editor, a: &Args) {
    let current = editor.window.buffer;
    let alt = editor.window.alt_buffer;
    let win = editor.window.id;
    let cursor_line = editor.cursor().line;
    let mut lines = Vec::new();
    for b in &editor.buffers {
        if !b.listed && !a.bang {
            continue;
        }
        let shown = editor.is_shown(b.id);
        let flags = format!(
            "{}{}{}{}{}",
            if b.listed { ' ' } else { 'u' },
            if b.id == current {
                '%'
            } else if Some(b.id) == alt {
                '#'
            } else {
                ' '
            },
            if !b.loaded {
                ' '
            } else if shown {
                'a'
            } else {
                'h'
            },
            ' ',
            if b.modified() { '+' } else { ' ' },
        );
        let line = if b.id == current {
            cursor_line + 1
        } else if b.loaded {
            b.listed_line(win)
        } else {
            0
        };
        let entry = format!("{:>3}{flags} \"{}\"", b.id.0, b.name());
        let pad = 40usize
            .saturating_sub(unicode_width::UnicodeWidthStr::width(entry.as_str()))
            .max(1);
        lines.push(format!("{entry}{}line {line}", " ".repeat(pad)));
    }
    let command = if a.bang { "ls!" } else { "ls" };
    show_list(editor, command, lines);
}

fn checktime(editor: &mut Editor, a: &Args) {
    let args = a.args;
    if no_args(editor, args) {
        editor.check_time();
    }
}

/// Vim's display of text in lists: control characters as `^X`, line breaks as `^J`.
fn printable(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\n' => out.push_str("^J"),
            '\x7f' => out.push_str("^?"),
            c if (c as u32) < 0x20 => {
                out.push('^');
                out.push(char::from(c as u8 + 64));
            }
            c => out.push(c),
        }
    }
    out
}

/// Cut `s` to `width` cells.
fn fit(s: &str, width: usize) -> String {
    let mut used = 0;
    let mut out = String::new();
    for c in s.chars() {
        let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(1);
        if used + w > width {
            break;
        }
        used += w;
        out.push(c);
    }
    out
}

/// Show a list the way Vim does: the command that asked for it, then the lines, then the
/// hit-enter prompt.
fn show_list(editor: &mut Editor, command: &str, lines: Vec<String>) {
    editor.full_message(format!(":{command}\n{}", lines.join("\n")));
    editor.hit_enter = true;
}

/// `:registers [names]`, `:display`.
fn registers(editor: &mut Editor, a: &Args) {
    let args = a.args;
    let width = editor.screen_size().0.max(20);
    let mut lines = vec!["Type Name Content".to_string()];
    for name in "\"0123456789abcdefghijklmnopqrstuvwxyz-.:%".chars() {
        if !args.is_empty() && !args.contains(name) {
            continue;
        }
        let Some(reg) = editor.register(Some(name)) else {
            continue;
        };
        if reg.text.is_empty() {
            continue;
        }
        let (kind, text) = match reg.kind {
            flux_view::RegisterKind::Char => ('c', reg.text.clone()),
            flux_view::RegisterKind::Line => ('l', format!("{}\n", reg.text)),
            flux_view::RegisterKind::Block => ('b', reg.text.clone()),
        };
        let line = format!("  {kind}  \"{name}   {}", printable(&text));
        lines.push(fit(&line, width - 1));
    }
    let command = if args.is_empty() {
        "reg".to_string()
    } else {
        format!("reg {args}")
    };
    show_list(editor, &command, lines);
}

/// The text shown after a mark or jump: the line without its indent, cut to fit after a
/// `lead`-wide prefix.
fn mark_text(editor: &Editor, line: usize, lead: usize) -> String {
    if line > editor.text().last_line() {
        return String::new();
    }
    let text = editor.text().line_str(line);
    // Vim's `mark_line` keeps the text narrower than `Columns - lead`.
    let width = editor.screen_size().0.max(lead + 2);
    fit(
        &printable(text.trim_start_matches([' ', '\t'])),
        width - lead - 1,
    )
}

/// A mark's column as Vim shows it: in bytes, with MAXCOL for "end of line".
fn byte_col(editor: &Editor, p: flux_view::Cursor) -> usize {
    if p.col == usize::MAX {
        return 2147483647;
    }
    if p.line > editor.text().last_line() {
        return p.col;
    }
    editor
        .text()
        .line_str(p.line)
        .chars()
        .take(p.col)
        .map(char::len_utf8)
        .sum()
}

/// `:marks [names]`.
fn marks(editor: &mut Editor, a: &Args) {
    let args = a.args;
    let mut list: Vec<(char, flux_view::Cursor)> = Vec::new();
    if let Some(p) = editor.window.pcmark {
        list.push(('\'', p));
    }
    let marks = editor.current_buffer().marks.clone();
    for name in 'a'..='z' {
        if let Some(p) = marks.get(name) {
            list.push((name, p));
        }
    }
    for name in 'A'..='Z' {
        if let Some(&(_, p)) = editor.global_marks.get(&name) {
            list.push((name, p));
        }
    }
    for name in "\"[]^.<>".chars() {
        if let Some(p) = marks.get(name) {
            list.push((name, p));
        }
    }
    let mut lines = vec!["mark line  col file/text".to_string()];
    for (name, p) in list {
        if !args.is_empty() && !args.contains(name) {
            continue;
        }
        let text = mark_text(editor, p.line, 15);
        let col = byte_col(editor, p);
        lines.push(format!(" {name} {:>6} {col:>4} {text}", p.line + 1));
    }
    if lines.len() == 1 {
        editor.error(format!("E283: No marks matching \"{args}\""));
        return;
    }
    let command = if args.is_empty() {
        "marks".to_string()
    } else {
        format!("marks {args}")
    };
    show_list(editor, &command, lines);
}

/// `:delmarks {names}` (ranges like `a-d` work), and `:delmarks!` for all lowercase marks.
fn delmarks(editor: &mut Editor, a: &Args) {
    let bang = a.bang;
    let args = a.args;
    if !bang && args.is_empty() {
        editor.error("E471: Argument required");
        return;
    }
    let marks = &mut editor.current_buffer_mut().marks;
    if bang {
        for c in 'a'..='z' {
            marks.remove(c);
        }
        return;
    }
    let chars: Vec<char> = args.chars().filter(|c| !c.is_whitespace()).collect();
    let mut i = 0;
    while i < chars.len() {
        if i + 2 < chars.len() && chars[i + 1] == '-' {
            for c in chars[i]..=chars[i + 2] {
                marks.remove(c);
            }
            i += 3;
        } else {
            marks.remove(chars[i]);
            i += 1;
        }
    }
}

/// `:jumps`.
fn jumps(editor: &mut Editor, _a: &Args) {
    let (entries, idx) = editor.window.jumps.entries();
    let entries = entries.to_vec();
    let mut lines = vec![" jump line  col file/text".to_string()];
    for (i, j) in entries.iter().enumerate() {
        let distance = i.abs_diff(idx);
        let marker = if i == idx { '>' } else { ' ' };
        let p = &j.pos;
        // Entries in other files show the file name, as in Vim.
        let text = if j.buffer == editor.window.buffer {
            mark_text(editor, p.line, 16)
        } else {
            editor
                .buffer(j.buffer)
                .map(|b| b.name())
                .unwrap_or_default()
        };
        let col = if j.buffer == editor.window.buffer {
            byte_col(editor, *p)
        } else {
            p.col
        };
        lines.push(format!(
            "{marker}{distance:>3} {:>5} {col:>4} {text}",
            p.line + 1
        ));
    }
    if idx >= entries.len() {
        lines.push(">".to_string());
    }
    show_list(editor, "jumps", lines);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn run(cmd: &str) -> Editor {
        let mut editor = Editor::new(80, 24);
        execute(&mut editor, cmd);
        editor
    }

    fn temp_file(contents: &str) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "flux-ex-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("file.txt");
        fs::write(&path, contents).unwrap();
        path
    }

    fn modify(editor: &mut Editor) {
        let mut engine = crate::Engine::new();
        for key in crate::parse_keys("x") {
            engine.handle_key(editor, key);
        }
    }

    fn message(editor: &Editor) -> &str {
        &editor.message.as_ref().unwrap().text
    }

    #[test]
    fn quit_and_its_abbreviations() {
        for cmd in [
            "q", "qu", "quit", "q!", "qa", "qall", "qa!", "quita", "quitall!", " :q ",
        ] {
            assert!(run(cmd).quit, ":{cmd} should quit");
        }
    }

    #[test]
    fn errors() {
        let e = run("qz");
        assert!(!e.quit);
        assert_eq!(message(&e), "E492: Not an editor command: qz");
        let e = run("q foo");
        assert!(!e.quit);
        assert_eq!(message(&e), "E488: Trailing characters: foo");
        assert!(!run("quitx").quit);
        assert_eq!(message(&run("w")), "E32: No file name");
    }

    #[test]
    fn quit_refuses_unsaved_changes() {
        let path = temp_file("abc\n");
        let mut editor = Editor::new(80, 24);
        editor.open(&path);
        modify(&mut editor);
        execute(&mut editor, "q");
        assert!(!editor.quit);
        assert!(editor.hit_enter);
        assert!(message(&editor).starts_with("E37: No write since last change\nE162:"));
        execute(&mut editor, "e");
        assert!(message(&editor).starts_with("E37"));
        execute(&mut editor, "q!");
        assert!(editor.quit);
    }

    #[test]
    fn write_and_wq() {
        let path = temp_file("abc\n");
        let mut editor = Editor::new(80, 24);
        editor.open(&path);
        modify(&mut editor);
        execute(&mut editor, "w");
        assert!(
            message(&editor).ends_with("1L, 3B written"),
            "{}",
            message(&editor)
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "bc\n");
        execute(&mut editor, "q");
        assert!(editor.quit);

        let mut editor = Editor::new(80, 24);
        editor.open(&path);
        modify(&mut editor);
        execute(&mut editor, "x");
        assert!(editor.quit);
        assert_eq!(fs::read_to_string(&path).unwrap(), "c\n");
    }

    #[test]
    fn edit_bang_reloads_and_is_undoable() {
        let path = temp_file("abc\n");
        let mut editor = Editor::new(80, 24);
        editor.open(&path);
        modify(&mut editor);
        execute(&mut editor, "e!");
        assert_eq!(editor.text().line_str(0), "abc");
        assert!(!editor.current_buffer().modified());
    }

    #[test]
    fn autoread_on_checktime() {
        let path = temp_file("one\n");
        let mut editor = Editor::new(80, 24);
        editor.open(&path);
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&path, "two\nlines\n").unwrap();
        execute(&mut editor, "checktime");
        assert_eq!(editor.text().line_str(0), "two");

        fs::write(&path, "three\n").unwrap();
        modify(&mut editor);
        execute(&mut editor, "checktime");
        assert!(message(&editor).starts_with("W12"));
        assert_eq!(editor.text().line_str(0), "wo");
    }
}
