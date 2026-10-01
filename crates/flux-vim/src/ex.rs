//! Ex commands. M1 has what's needed to edit and save one file; ranges and the rest of the
//! command table arrive in M4.

use std::path::PathBuf;

use flux_view::Editor;

use crate::engine::Engine;
use crate::ex_lines;
use crate::global;
use crate::set;
use crate::substitute;

/// How a command runs: with just the editor, or needing the engine (to edit text as one undo
/// step, or to run keys).
#[derive(Clone, Copy)]
enum Run {
    Editor(fn(&mut Editor, &Args)),
    Engine(fn(&mut Engine, &mut Editor, &Args)),
}

/// What a range before a command means.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RangeKind {
    /// No range allowed (E481).
    None,
    /// Buffer lines, checked against the buffer (E16).
    Lines,
    /// A number for the command (`:5split`, `:3bnext`): passed as `count`.
    Other,
}

/// An Ex command name, how short Vim lets you abbreviate it (`:w`, `:wq`, `:quita`), and what
/// it accepts.
#[derive(Clone, Copy)]
struct Command {
    name: &'static str,
    min_len: usize,
    run: Run,
    range: RangeKind,
    /// A count after the name (`:d 3`) that extends the range.
    count: bool,
    /// A register name after the name (`:d a`, `:pu x`).
    register: bool,
    /// Without a range, the whole buffer (`:w`, `:=`).
    all_by_default: bool,
    /// Line 0 is a valid address (`:0put`, `:m 0`).
    zero: bool,
    /// Keep trailing white space in the argument (`:normal`, `:s`).
    raw: bool,
}

const fn cmd(name: &'static str, min_len: usize, run: fn(&mut Editor, &Args)) -> Command {
    Command {
        name,
        min_len,
        run: Run::Editor(run),
        range: RangeKind::None,
        count: false,
        register: false,
        all_by_default: false,
        zero: false,
        raw: false,
    }
}

const fn ecmd(
    name: &'static str,
    min_len: usize,
    run: fn(&mut Engine, &mut Editor, &Args),
) -> Command {
    Command {
        name,
        min_len,
        run: Run::Engine(run),
        range: RangeKind::None,
        count: false,
        register: false,
        all_by_default: false,
        zero: false,
        raw: false,
    }
}

impl Command {
    const fn lines(mut self) -> Self {
        self.range = RangeKind::Lines;
        self
    }
    const fn other(mut self) -> Self {
        self.range = RangeKind::Other;
        self
    }
    const fn count(mut self) -> Self {
        self.count = true;
        self
    }
    const fn register(mut self) -> Self {
        self.register = true;
        self
    }
    const fn all(mut self) -> Self {
        self.all_by_default = true;
        self
    }
    const fn zero(mut self) -> Self {
        self.zero = true;
        self
    }
    const fn raw(mut self) -> Self {
        self.raw = true;
        self
    }
}

const COMMANDS: &[Command] = &[
    cmd("write", 1, write).lines().all(),
    cmd("wq", 2, write_quit).lines().all(),
    cmd("wall", 2, write_all),
    cmd("wqall", 3, write_quit_all),
    cmd("xit", 1, exit).lines().all(),
    cmd("xall", 2, write_quit_all),
    cmd("exit", 3, exit).lines().all(),
    cmd("update", 2, update).lines().all(),
    cmd("edit", 1, edit),
    cmd("quit", 1, quit),
    cmd("qall", 2, quit_all),
    cmd("quitall", 5, quit_all),
    cmd("checktime", 6, checktime),
    cmd("registers", 3, registers),
    cmd("display", 2, registers),
    cmd("marks", 5, marks),
    cmd("delmarks", 4, delmarks),
    cmd("jumps", 2, jumps),
    cmd("split", 2, split).other(),
    cmd("vsplit", 2, vsplit).other(),
    cmd("new", 3, new_window).other(),
    cmd("vnew", 3, vnew).other(),
    cmd("close", 3, close),
    cmd("only", 2, only),
    cmd("resize", 3, resize).other(),
    cmd("enew", 3, enew),
    cmd("buffer", 1, buffer).other(),
    cmd("bnext", 2, bnext).other(),
    cmd("bNext", 2, bprevious).other(),
    cmd("bprevious", 2, bprevious).other(),
    cmd("bfirst", 2, bfirst),
    cmd("brewind", 2, bfirst),
    cmd("blast", 2, blast),
    cmd("bdelete", 2, bdelete).other(),
    cmd("bwipeout", 2, bwipeout).other(),
    cmd("ls", 2, list_buffers),
    cmd("buffers", 7, list_buffers),
    cmd("files", 5, list_buffers),
    ecmd("delete", 1, ex_lines::delete)
        .lines()
        .count()
        .register(),
    ecmd("yank", 1, ex_lines::yank_lines)
        .lines()
        .count()
        .register(),
    ecmd(">", 1, ex_lines::shift).lines().count(),
    ecmd("<", 1, ex_lines::shift).lines().count(),
    ecmd("move", 1, ex_lines::move_lines).lines(),
    ecmd("copy", 2, ex_lines::copy_lines).lines(),
    ecmd("t", 1, ex_lines::copy_lines).lines(),
    ecmd("join", 1, ex_lines::join).lines().count(),
    ecmd("put", 2, ex_lines::put).lines().register().zero(),
    cmd("print", 1, ex_lines::print).lines().count(),
    cmd("number", 2, ex_lines::number).lines().count(),
    cmd("#", 1, ex_lines::number).lines().count(),
    cmd("=", 1, ex_lines::equal).lines().all(),
    cmd("k", 1, ex_lines::mark).lines(),
    cmd("mark", 2, ex_lines::mark).lines(),
    ecmd("normal", 4, ex_lines::normal).lines().raw(),
    ecmd("undo", 1, ex_lines::undo),
    ecmd("redo", 3, ex_lines::redo),
    cmd("nohlsearch", 3, ex_lines::nohlsearch),
    ecmd("substitute", 1, substitute::substitute).lines().raw(),
    ecmd("&&", 2, substitute::substitute).lines().raw(),
    ecmd("&", 1, substitute::substitute).lines().raw(),
    ecmd("~", 1, substitute::substitute).lines().raw(),
    ecmd("global", 1, global::global).lines().all().raw(),
    ecmd("vglobal", 1, global::global).lines().all().raw(),
    cmd("set", 2, set::set),
    cmd("setlocal", 4, set::setlocal),
    cmd("setglobal", 4, set::setglobal),
];

/// What a command was given besides its name.
pub(crate) struct Args<'a> {
    pub bang: bool,
    pub args: &'a str,
    /// A number before the name (`:5split`, `:2bnext`).
    pub count: Option<usize>,
    /// The `:vertical` modifier.
    pub vertical: bool,
    /// The range, 1-based (line 0 only for commands that allow it), and how many addresses were
    /// given (0 when the command got its default range).
    pub line1: usize,
    pub line2: usize,
    pub addr_count: usize,
    /// `:d a`, `:pu x`.
    pub register: Option<char>,
    /// The command name as typed, for commands that look at it (`:>>`).
    pub name: &'a str,
}

/// Run an Ex command line outside of an engine (tests, and commands that don't edit text).
pub fn execute(editor: &mut Editor, line: &str) {
    let mut engine = Engine::new();
    run(&mut engine, editor, line);
    engine.commit(editor);
}

/// Run an Ex command line.
pub fn run(engine: &mut Engine, editor: &mut Editor, line: &str) {
    editor.quitmore = editor.quitmore.saturating_sub(1);
    let full = line;
    let mut line = line.trim_start_matches([' ', '\t', ':']);
    if line.trim().is_empty() {
        return;
    }
    let mut vertical = false;
    if let Some(rest) = strip_modifier(line, "vertical", 4) {
        vertical = true;
        line = rest;
    }
    let range = match parse_range(editor, line) {
        Ok(r) => r,
        Err(e) => {
            if !e.is_empty() {
                editor.error(e);
            }
            return;
        }
    };
    let rest = range.rest.trim_start_matches([' ', '\t', ':']);
    if rest.is_empty() {
        // `:N`: go to line N (the last line if past the end).
        if range.addr_count > 0 {
            if range.line2 < 0 {
                editor.error("E16: Invalid range");
                return;
            }
            let count = editor.text().line_count() as isize;
            let target = range.line2.clamp(1, count) as usize - 1;
            editor.with_window(|win, m| win.set_cursor_line(target, m));
        }
        return;
    }

    let (name, after) = split_command_name(rest);
    let (bang, args) = match after.strip_prefix('!') {
        Some(args) if !name.starts_with(['<', '>', '&', '~', '=']) => (true, args),
        _ => (false, after),
    };
    let Some(command) = find_command(name) else {
        editor.error(format!("E492: Not an editor command: {}", full.trim()));
        return;
    };
    let args = if command.raw {
        args.trim_start_matches([' ', '\t'])
    } else {
        args.trim()
    };

    let last = editor.text().line_count() as isize;
    let current = editor.cursor().line as isize + 1;
    let (mut line1, mut line2) = if range.addr_count == 0 {
        if command.all_by_default {
            (1, last)
        } else {
            (current, current)
        }
    } else {
        (range.line1, range.line2)
    };
    let mut addr_count = range.addr_count;
    let mut args = args;
    let mut count = None;
    let mut register = None;
    match command.range {
        RangeKind::None if addr_count > 0 => {
            editor.error("E481: No range allowed");
            return;
        }
        RangeKind::None => {}
        RangeKind::Other => {
            if addr_count > 0 {
                count = Some(line2.max(0) as usize);
            }
        }
        RangeKind::Lines => {
            if line1 > line2 {
                if !engine.swap_range {
                    // Vim asks first; the answer runs the command again.
                    engine.confirm_swap = Some(full.to_string());
                    editor.message = Some(flux_view::Message {
                        text: "Backwards range given, OK to swap (y/n)?".into(),
                        kind: flux_view::MessageKind::Question,
                    });
                    return;
                }
                std::mem::swap(&mut line1, &mut line2);
            }
            if line1 < 0 || line2 > last {
                editor.error("E16: Invalid range");
                return;
            }
            if !command.zero {
                line1 = line1.max(1);
                line2 = line2.max(1);
            }
            if command.register
                && let Some(c) = args.chars().next()
                && !c.is_ascii_digit()
                && flux_view::registers::is_valid_name(c)
            {
                register = Some(c);
                args = args[c.len_utf8()..].trim_start();
            }
            if command.count && args.starts_with(|c: char| c.is_ascii_digit()) {
                let digits: String = args.chars().take_while(char::is_ascii_digit).collect();
                args = args[digits.len()..].trim_start();
                let n: isize = digits.parse().unwrap_or(0);
                if n <= 0 {
                    editor.error("E939: Positive count required");
                    return;
                }
                line1 = line2;
                line2 = (line2 + n - 1).min(last);
                addr_count += 1;
            }
        }
    }
    let a = Args {
        bang,
        args,
        count,
        vertical,
        line1: line1.max(0) as usize,
        line2: line2.max(0) as usize,
        addr_count,
        register,
        name,
    };
    // `:e! file`, `:b! 3`, …: may leave a buffer with unsaved changes without 'hidden'.
    editor.force_abandon = bang;
    match command.run {
        Run::Editor(f) => f(editor, &a),
        Run::Engine(f) => f(engine, editor, &a),
    }
    editor.force_abandon = false;
}

/// Split the command name off: letters (`s`, `delete`, `k` alone before a mark name), or one of
/// the symbol commands (`&`, `&&`, `~`, `<<<`, `>`, `=`).
fn split_command_name(s: &str) -> (&str, &str) {
    let first = s.chars().next().unwrap_or(' ');
    let len = match first {
        '<' | '>' => s.chars().take_while(|&c| c == first).count(),
        '&' => {
            if s[1..].starts_with('&') {
                2
            } else {
                1
            }
        }
        '~' | '=' | '#' => 1,
        // `:ka` is `:k a`, `:s#a#b#` is `:s` with `#` as the delimiter.
        'k' if s[1..].starts_with(|c: char| c.is_ascii_alphabetic()) && !s.starts_with("keep") => 1,
        _ => s
            .find(|c: char| !c.is_ascii_alphabetic())
            .unwrap_or(s.len()),
    };
    let len = if len == 0 { 1.min(s.len()) } else { len };
    s.split_at(len)
}

fn find_command(name: &str) -> Option<Command> {
    if name.is_empty() {
        return None;
    }
    let key = match name.chars().next() {
        Some('<') => "<",
        Some('>') => ">",
        _ => name,
    };
    COMMANDS
        .iter()
        .find(|c| key.len() >= c.min_len && c.name.starts_with(key))
        .copied()
}

/// A parsed range: its two lines (1-based; may be out of range, checked by the caller), how
/// many addresses were given, and the rest of the command line.
struct ParsedRange<'a> {
    line1: isize,
    line2: isize,
    addr_count: usize,
    rest: &'a str,
}

/// Vim's `parse_cmd_address`: addresses separated by `,` or `;` (which moves the cursor), `%`
/// and `*`.
fn parse_range<'a>(editor: &mut Editor, s: &'a str) -> Result<ParsedRange<'a>, String> {
    let mut line1;
    let mut line2 = editor.cursor().line as isize + 1;
    let mut addr_count = 0;
    let mut rest = s;
    let mut last_none;
    loop {
        line1 = line2;
        line2 = editor.cursor().line as isize + 1;
        rest = rest.trim_start_matches([' ', '\t']);
        let (lnum, after) = get_address(editor, rest, addr_count == 0)?;
        rest = after;
        last_none = lnum.is_none();
        match lnum {
            None => {
                if let Some(after) = rest.strip_prefix('%') {
                    rest = after;
                    line1 = 1;
                    line2 = editor.text().line_count() as isize;
                    addr_count += 1;
                } else if let Some(after) = rest.strip_prefix('*') {
                    rest = after;
                    let start = crate::motion::mark_position(editor, '<');
                    let end = crate::motion::mark_position(editor, '>');
                    match (start, end) {
                        (Some(a), Some(b)) => {
                            line1 = a.line as isize + 1;
                            line2 = b.line as isize + 1;
                            addr_count += 1;
                        }
                        _ => return Err("E20: Mark not set".into()),
                    }
                }
            }
            Some(l) => line2 = l,
        }
        addr_count += 1;
        if let Some(after) = rest.strip_prefix(';') {
            let last = editor.text().last_line();
            editor.window.cursor.line = (line2.max(1) as usize - 1).min(last);
            rest = after;
        } else if let Some(after) = rest.strip_prefix(',') {
            rest = after;
        } else {
            break;
        }
    }
    if addr_count == 1 {
        line1 = line2;
        if last_none {
            addr_count = 0;
        }
    }
    Ok(ParsedRange {
        line1,
        line2,
        addr_count,
        rest,
    })
}

/// Vim's `get_address`: one address with its `+N`/`-N` offsets, or `None` if there is none.
/// Errors are messages to show (empty when already shown).
fn get_address<'a>(
    editor: &mut Editor,
    s: &'a str,
    _first: bool,
) -> Result<(Option<isize>, &'a str), String> {
    let digits = |s: &str| s.chars().take_while(char::is_ascii_digit).count();
    let mut rest = s.trim_start_matches([' ', '\t']);
    let mut lnum: Option<isize> = None;
    loop {
        let c = rest.chars().next();
        match c {
            Some('.') => {
                lnum = Some(editor.cursor().line as isize + 1);
                rest = &rest[1..];
            }
            Some('$') => {
                lnum = Some(editor.text().line_count() as isize);
                rest = &rest[1..];
            }
            Some('\'') => {
                let Some(name) = rest[1..].chars().next() else {
                    return Err(String::new());
                };
                rest = &rest[1 + name.len_utf8()..];
                match crate::motion::mark_position(editor, name) {
                    Some(p) => lnum = Some(p.line as isize + 1),
                    None => return Err("E20: Mark not set".into()),
                }
            }
            Some(d @ ('/' | '?')) => {
                let from = lnum.unwrap_or(editor.cursor().line as isize + 1);
                let (line, used) = flux_view::search::address_search(editor, d, &rest[1..], from)
                    .map_err(|()| String::new())?;
                lnum = Some(line as isize + 1);
                rest = &rest[1 + used..];
            }
            Some('\\') => {
                let kind = rest[1..].chars().next();
                let from = lnum.unwrap_or(editor.cursor().line as isize + 1);
                let line = match kind {
                    Some(k @ ('/' | '?' | '&')) => {
                        flux_view::search::repeat_address_search(editor, k, from)
                            .map_err(|()| String::new())?
                    }
                    _ => return Err("E10: \\ should be followed by /, ? or &".into()),
                };
                lnum = Some(line as isize + 1);
                rest = &rest[2..];
            }
            Some(d) if d.is_ascii_digit() => {
                let n = digits(rest);
                lnum = rest[..n].parse().ok();
                rest = &rest[n..];
            }
            _ => {}
        }
        // Offsets: `+N`, `-N`, `+`, `-`, and a bare number meaning `+N`.
        loop {
            rest = rest.trim_start_matches([' ', '\t']);
            let Some(c) = rest.chars().next() else {
                break;
            };
            if !matches!(c, '+' | '-') && !c.is_ascii_digit() {
                break;
            }
            let base = lnum.unwrap_or(editor.cursor().line as isize + 1);
            let sign = if c.is_ascii_digit() {
                '+'
            } else {
                rest = &rest[1..];
                c
            };
            let n = digits(rest);
            let amount: isize = if n == 0 {
                1
            } else {
                rest[..n].parse().unwrap_or(0)
            };
            rest = &rest[n..];
            lnum = Some(if sign == '+' {
                base + amount
            } else {
                base - amount
            });
        }
        // Another search can follow: `/foo//bar/`.
        if !rest.starts_with(['/', '?']) {
            break;
        }
    }
    Ok((lnum, rest))
}

/// One address making up the whole of `s` (the destination of `:m` and `:t`).
pub(crate) fn parse_single_address(editor: &mut Editor, s: &str) -> Result<Option<isize>, String> {
    let (lnum, rest) = get_address(editor, s.trim(), true)?;
    if !rest.trim().is_empty() {
        return Err(format!("E488: Trailing characters: {}", rest.trim()));
    }
    Ok(lnum)
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

pub(crate) fn no_args(editor: &mut Editor, args: &str) -> bool {
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
    let whole = a.line1 <= 1 && a.line2 >= editor.text().line_count();
    if a.addr_count > 0 && !whole {
        write_part(editor, a);
        return;
    }
    do_write(editor, a.bang, a.args);
}

/// `:[range]w[!] [file]` for part of the buffer.
fn write_part(editor: &mut Editor, a: &Args) {
    let cwd = editor.cwd.clone();
    let target = if a.args.is_empty() {
        match editor.current_buffer().path.clone() {
            Some(p) => cwd.join(p),
            None => {
                editor.error("E32: No file name");
                return;
            }
        }
    } else {
        cwd.join(a.args)
    };
    let buffer = editor.current_buffer();
    let own = buffer.path.as_ref().map(|p| cwd.join(p)).as_deref() == Some(target.as_path());
    match buffer.write_lines(&target, a.line1 - 1, a.line2 - 1, a.bang, own) {
        Ok(msg) => editor.file_message(msg.replace(&format!("{}/", cwd.display()), "")),
        Err(e) => editor.error(e),
    }
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

/// `s` made printable (`^J`, `^I`, …) and cut to `width` cells without splitting a `^X`.
fn fit_printable(s: &str, width: usize) -> String {
    let mut used = 0;
    let mut out = String::new();
    for c in s.chars() {
        let shown = printable(&c.to_string());
        let w = unicode_width::UnicodeWidthStr::width(shown.as_str()).max(1);
        if used + w > width {
            break;
        }
        used += w;
        out.push_str(&shown);
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
        let prefix = format!("  {kind}  \"{name}   ");
        let room = (width - 1).saturating_sub(prefix.chars().count());
        lines.push(format!("{prefix}{}", fit_printable(&text, room)));
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
