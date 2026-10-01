//! `:s`, `:&`, `:&&`, `:~`, and Normal-mode `&` and `g&`, following Vim's `do_sub`.
//!
//! Like Vim, every match is found in the original text: after a match the search goes on
//! from its end in the same (original) line, and a match that ends in a later line continues
//! in that line. The substitutions are then applied in order as one undo step. With the `c`
//! flag each one is confirmed first.

use std::collections::BTreeSet;

use flux_core::Edit;
use flux_core::pattern::{Pattern, PatternOptions};
use flux_view::search::{self, SubFlags};
use flux_view::{Cursor, Editor, Message, MessageKind};

use crate::engine::Engine;
use crate::ex::Args;
use crate::normal::set_pcmark;
use crate::util::{self, pos};

/// Which remembered pattern an empty pattern means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Which {
    /// The last substitute pattern (`:s`, `:&`).
    Substitute,
    /// The last search pattern (`\/`).
    Search,
    /// Whichever was used last (`:s//`, `:~`, the `r` flag).
    Last,
}

/// One substitution, as char offsets into the text before any of them were made.
#[derive(Debug, Clone)]
pub(crate) struct SubMatch {
    start: usize,
    end: usize,
    replacement: String,
    /// The (original) line counted as changed.
    line: usize,
}

/// A `:s///c` waiting for answers.
#[derive(Debug)]
pub(crate) struct ConfirmSession {
    matches: Vec<SubMatch>,
    next: usize,
    /// How far applied substitutions have moved later text, in chars.
    delta: isize,
    replacement_shown: String,
    done: Vec<usize>,
    lines: BTreeSet<usize>,
    range: (usize, usize),
    flags: SubFlags,
    /// Where the last applied substitution ended (a char offset in the current text).
    last_end: Option<usize>,
    /// The last answer made a substitution (rather than skipping or quitting).
    last_was_sub: bool,
    lines_before: usize,
}

/// `:s`, `:substitute`, `:&`, `:&&`, `:~`.
pub(crate) fn substitute(engine: &mut Engine, editor: &mut Editor, a: &Args) {
    let name = a.name;
    let mut which = if name == "~" {
        Which::Last
    } else {
        Which::Substitute
    };
    let mut rest = a.args;
    let is_s = name.starts_with('s');
    let mut new_pattern: Option<String> = None;
    let replacement: String;
    let first = rest.chars().next();
    if is_s
        && let Some(delim) =
            first.filter(|c| !c.is_whitespace() && !"0123456789cegriIp|\"".contains(*c))
    {
        if delim.is_ascii_alphanumeric() {
            editor.error("E146: Regular expressions can't be delimited by letters");
            return;
        }
        let delim_used;
        if delim == '\\' {
            let kind = rest[1..].chars().next();
            match kind {
                Some(k @ ('/' | '?' | '&')) => {
                    which = if k == '&' {
                        Which::Substitute
                    } else {
                        Which::Search
                    };
                    new_pattern = Some(String::new());
                    delim_used = k;
                    rest = &rest[2..];
                }
                _ => {
                    editor.error("E10: \\ should be followed by /, ? or &");
                    return;
                }
            }
        } else {
            which = Which::Last;
            rest = &rest[delim.len_utf8()..];
            let (pat, after) = search::split_pattern(rest, delim);
            new_pattern = Some(pat);
            rest = after.unwrap_or("");
            delim_used = delim;
        }
        let (sub, after) = skip_substitute(rest, delim_used);
        replacement = sub;
        rest = after;
        editor.search.last_sub_command = Some(replacement.clone());
    } else {
        match editor.search.last_sub_command.clone() {
            Some(sub) => replacement = sub,
            None => {
                editor.error("E35: No previous regular expression");
                return;
            }
        }
    }
    // `:&&` keeps the flags; so does a leading `&`.
    let keep = name == "&&" || rest.starts_with('&');
    if rest.starts_with('&') {
        rest = &rest[1..];
    }
    let mut flags = if keep {
        editor.search.sub_flags
    } else {
        SubFlags {
            all: editor.options.gdefault,
            ..SubFlags::default()
        }
    };
    let mut chars = rest.char_indices();
    let mut used = rest.len();
    for (i, c) in chars.by_ref() {
        match c {
            'g' => flags.all = !flags.all,
            'c' => flags.ask = !flags.ask,
            'n' => flags.count = true,
            'e' => flags.error = !flags.error,
            'r' => which = Which::Last,
            'p' => flags.print = true,
            '#' => {
                flags.print = true;
                flags.number = true;
            }
            'l' => {
                flags.print = true;
                flags.list = true;
            }
            'i' => flags.ignore_case = Some(true),
            'I' => flags.ignore_case = Some(false),
            _ => {
                used = i;
                break;
            }
        }
    }
    if flags.count {
        flags.ask = false;
    }
    let saved_flags = flags;
    rest = rest[used..].trim_start();
    // A count: that many lines from the last line of the range.
    let (mut line1, mut line2) = (a.line1, a.line2);
    if rest.starts_with(|c: char| c.is_ascii_digit()) {
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        rest = rest[digits.len()..].trim_start();
        let n: usize = digits.parse().unwrap_or(0);
        if n == 0 {
            if flags.error {
                editor.error("E939: Positive count required");
            }
            return;
        }
        line1 = line2;
        line2 = (line2 + n - 1).min(editor.text().line_count());
    }
    if !rest.is_empty() && !rest.starts_with('"') {
        editor.error(format!("E488: Trailing characters: {rest}"));
        return;
    }
    editor.search.sub_flags = saved_flags;
    // The pattern: a new one, or a remembered one. Either way it becomes the last substitute
    // pattern (and the one `n` uses).
    let (pat, no_scs) = match new_pattern.filter(|p| !p.is_empty()) {
        Some(p) => {
            editor.search.add_history(true, &p);
            (p, false)
        }
        None => {
            let saved = match which {
                Which::Substitute => editor.search.substitute.clone(),
                Which::Search => editor.search.search.clone(),
                Which::Last => editor.search.last_pattern().cloned(),
            };
            match saved {
                Some(s) => (s.pat, s.no_smartcase),
                None => {
                    editor.error("E35: No previous regular expression");
                    return;
                }
            }
        }
    };
    editor.search.set_substitute(&pat, no_scs);
    if replacement.starts_with("\\=") {
        editor.error("E1: flux doesn't support \\= expressions in :s yet");
        return;
    }
    // `:s/\n//` joins lines (Vim's `sub_joining_lines`).
    if pat == "\\n" && replacement.is_empty() && !flags.ask && !flags.count {
        editor.search.last_replacement = Some(String::new());
        let mut count = line2 - line1 + 1;
        if line2 < editor.text().line_count() {
            count += 1;
        }
        if count > 1 {
            editor.window.cursor = pos(line1 - 1, 0);
            engine.join(editor, Some(count), false);
            if count - 1 > editor.options.report {
                editor.info(count_message(count - 1, 1, false));
            }
        }
        return;
    }
    let options = PatternOptions {
        ignorecase: flags.ignore_case.unwrap_or(editor.options.ignorecase),
        smartcase: flags.ignore_case.is_none() && editor.options.smartcase && !no_scs,
        last_substitute: editor.search.last_replacement.as_deref(),
    };
    let pattern = match Pattern::new(&pat, options) {
        Ok(p) => p,
        Err(e) => {
            if flags.error {
                editor.error(e.0);
            }
            return;
        }
    };
    // `~` in the replacement is the previous replacement (after `~` in the pattern used it).
    let replacement = regtilde(&replacement, editor.search.last_replacement.as_deref());
    editor.search.last_replacement = Some(replacement.clone());
    let matches = find_matches(
        editor,
        &pattern,
        &replacement,
        line1 - 1,
        line2 - 1,
        flags,
        None,
    );
    let global_busy = engine.global_lines.is_some();
    if matches.is_empty() {
        if flags.error && !global_busy {
            editor.error(format!("E486: Pattern not found: {pat}"));
        }
        return;
    }
    if flags.count {
        let lines: BTreeSet<usize> = matches.iter().map(|m| m.line).collect();
        editor.info(count_message(matches.len(), lines.len(), true));
        return;
    }
    set_pcmark(editor);
    // Undo comes back to the first changed line, as in Vim (which saves the cursor there).
    let first = editor.text().char_to_pos(matches[0].start).0;
    editor.window.cursor = pos(first, 0);
    let mut session = ConfirmSession {
        matches,
        next: 0,
        delta: 0,
        replacement_shown: editor.search.last_sub_command.clone().unwrap_or_default(),
        done: Vec::new(),
        lines: BTreeSet::new(),
        range: (line1, line2),
        flags,
        last_end: None,
        last_was_sub: false,
        lines_before: editor.text().line_count(),
    };
    if flags.ask {
        engine.hold_undo += 1;
        editor.hide_modified = !editor.current_buffer().modified();
        show_confirm(editor, &mut session);
        engine.confirm_sub = Some(session);
        return;
    }
    while session.next < session.matches.len() {
        apply_next(engine, editor, &mut session);
    }
    if global_busy {
        // `:g` reports the total, and puts the cursor on the first non-blank at the end.
        engine.global_subs.0 += session.done.len();
        engine.global_subs.1 += session.lines.len();
        let end = session.last_end.unwrap_or(0).min(editor.text().len_chars());
        editor.window.cursor = pos(editor.text().char_to_pos(end).0, 0);
        return;
    }
    engine.saved_lines = Some(session.saved_lines(editor));
    finish(editor, &session);
}

/// 'inccommand' ("nosplit"): what `line`, a `:s` command being typed, would do, without
/// changing anything. `None` if it isn't a `:s` with a pattern yet.
/// `botline` is the first line below the window before the command line scrolled it: past
/// it, like Neovim, only the first 'cmdwinheight' (7) + 1 changed lines are previewed.
pub(crate) fn preview(editor: &Editor, line: &str, botline: usize) -> Option<flux_view::Preview> {
    let line = line.trim_start_matches([' ', ':']);
    let (line1, line2, rest) = preview_range(editor, line)?;
    let name_len = rest
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(rest.len());
    let (name, args) = rest.split_at(name_len);
    if name.is_empty() || !"substitute".starts_with(name) {
        return None;
    }
    let delim = args.chars().next()?;
    if delim.is_ascii_alphanumeric() || delim.is_whitespace() || "\\\"|".contains(delim) {
        return None;
    }
    let (pat, after) = search::split_pattern(&args[delim.len_utf8()..], delim);
    let pat = if pat.is_empty() {
        editor.search.last_pattern()?.pat.clone()
    } else {
        pat
    };
    let options = PatternOptions {
        ignorecase: editor.options.ignorecase,
        smartcase: editor.options.smartcase,
        last_substitute: editor.search.last_replacement.as_deref(),
    };
    let pattern = Pattern::new(&pat, options).ok()?;
    let mut flags = SubFlags {
        all: editor.options.gdefault,
        ..SubFlags::default()
    };
    let text = editor.text();
    let Some(after) = after else {
        // Only the pattern so far: show where it matches in the range.
        flags.all = true;
        let matches = find_matches(editor, &pattern, "", line1, line2, flags, Some(botline));
        let highlights: Vec<_> = matches
            .iter()
            .map(|m| (to_cursor(text, m.start), to_cursor(text, m.end)))
            .collect();
        return Some(flux_view::Preview {
            text: text.clone(),
            first_match: highlights.first().map(|h| h.0),
            highlights,
            changed: false,
        });
    };
    let (sub, flag_str) = skip_substitute(after, delim);
    let mut used = flag_str.len();
    for (i, c) in flag_str.char_indices() {
        match c {
            'g' => flags.all = !flags.all,
            'i' | 'I' | 'c' | 'e' | 'n' | 'r' | 'p' | '#' | 'l' | '&' => {}
            _ => {
                used = i;
                break;
            }
        }
    }
    // A count may follow; anything else is an error, and there's no preview.
    let tail = flag_str[used..].trim();
    if !tail.is_empty() && !tail.chars().all(|c| c.is_ascii_digit()) && !tail.starts_with('"') {
        return None;
    }
    let replacement = regtilde(&sub, editor.search.last_replacement.as_deref());
    let matches = find_matches(
        editor,
        &pattern,
        &replacement,
        line1,
        line2,
        flags,
        Some(botline),
    );
    // With a replacement, the cursor goes to the first match (Neovim's incsearch position,
    // which its statusline shows during the preview).
    let first_match = matches.first().map(|m| to_cursor(text, m.start));
    let mut new_text = text.clone();
    let mut highlights = Vec::new();
    let mut delta = 0isize;
    for m in &matches {
        let start = (m.start as isize + delta) as usize;
        let end = (m.end as isize + delta) as usize;
        let len = m.replacement.chars().count();
        new_text.apply(&Edit::replace(start..end, m.replacement.clone()));
        delta += len as isize - (m.end - m.start) as isize;
        highlights.push((start, start + len));
    }
    let highlights = highlights
        .into_iter()
        .map(|(a, b)| (to_cursor(&new_text, a), to_cursor(&new_text, b)))
        .collect();
    Some(flux_view::Preview {
        text: new_text,
        changed: !matches.is_empty(),
        highlights,
        first_match,
    })
}

/// Where 'incsearch' puts the cursor for a `:s` being typed: the first match of its pattern
/// in the range, however the rest of the command looks.
pub(crate) fn pattern_match(editor: &Editor, line: &str) -> Option<Cursor> {
    let line = line.trim_start_matches([' ', ':']);
    let (_, _, rest) = preview_range(editor, line)?;
    let name_len = rest
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(rest.len());
    let (name, args) = rest.split_at(name_len);
    if name.is_empty() || !"substitute".starts_with(name) {
        return None;
    }
    let delim = args.chars().next()?;
    let (pat, _) = search::split_pattern(&args[delim.len_utf8()..], delim);
    let cut = format!(
        "{}{delim}{pat}",
        &line[..line.len() - rest.len() + name_len]
    );
    preview(editor, &cut, usize::MAX)?.first_match
}

fn to_cursor(text: &flux_core::Text, c: usize) -> Cursor {
    let (line, col) = text.char_to_pos(c.min(text.len_chars()));
    Cursor { line, col }
}

/// The range of a `:s` being typed, for the preview: `%`, `.`, `$`, numbers, marks and
/// offsets (0-based lines). Anything else (a search) gives no preview.
fn preview_range<'a>(editor: &Editor, s: &'a str) -> Option<(usize, usize, &'a str)> {
    let last = editor.text().last_line() as isize;
    let cur = editor.cursor().line as isize;
    if let Some(rest) = s.strip_prefix('%') {
        return Some((0, last as usize, rest));
    }
    let mut rest = s;
    let mut addrs = Vec::new();
    loop {
        let mut line: Option<isize> = None;
        match rest.chars().next() {
            Some('.') => {
                line = Some(cur);
                rest = &rest[1..];
            }
            Some('$') => {
                line = Some(last);
                rest = &rest[1..];
            }
            Some('\'') => {
                let name = rest[1..].chars().next()?;
                line = Some(crate::motion::mark_position(editor, name)?.line as isize);
                rest = &rest[1 + name.len_utf8()..];
            }
            Some(c) if c.is_ascii_digit() => {
                let n = rest.chars().take_while(char::is_ascii_digit).count();
                line = Some(rest[..n].parse::<isize>().ok()? - 1);
                rest = &rest[n..];
            }
            Some('/' | '?') => return None,
            _ => {}
        }
        while let Some(sign @ ('+' | '-')) = rest.chars().next() {
            let n = rest[1..].chars().take_while(char::is_ascii_digit).count();
            let amount: isize = if n == 0 {
                1
            } else {
                rest[1..1 + n].parse().ok()?
            };
            let base = line.unwrap_or(cur);
            line = Some(if sign == '+' {
                base + amount
            } else {
                base - amount
            });
            rest = &rest[1 + n..];
        }
        addrs.push(line);
        match rest.chars().next() {
            Some(',' | ';') => rest = &rest[1..],
            _ => break,
        }
    }
    let (a, b) = match addrs.as_slice() {
        [None] => (cur, cur),
        [Some(l)] => (*l, *l),
        [first, .., second] => (first.unwrap_or(cur), second.unwrap_or(cur)),
        [] => (cur, cur),
    };
    let (a, b) = (a.min(b).clamp(0, last), a.max(b).clamp(0, last));
    Some((a as usize, b as usize, rest))
}

/// Find every substitution in lines `first..=last` (0-based), following Vim's `do_sub` loop.
/// For a preview, `preview_botline` stops the search the way Neovim's preview loop stops:
/// once more than 'cmdwinheight' (7) lines matched and the next line is below the window.
fn find_matches(
    editor: &Editor,
    pattern: &Pattern,
    replacement: &str,
    first: usize,
    last: usize,
    flags: SubFlags,
    preview_botline: Option<usize>,
) -> Vec<SubMatch> {
    let text = editor.text();
    let hay = search::Haystack::lines(text, first, last, pattern.is_multiline());
    let s = &hay.s;
    let line_of = |b: usize| hay.line_of(b.min(s.len()));
    let line_end = |l: usize| hay.line_end_byte(l);
    let mut out = Vec::new();
    let mut lnum = first;
    // Vim moves on to the next line, but a multi-line match can stop the whole command.
    let mut do_all = flags.all;
    let mut lines_needed = 0;
    while lnum <= last && lnum < hay.line_count() {
        if let Some(bot) = preview_botline
            && lines_needed > 7
            && lnum > bot
        {
            break;
        }
        let Some(mut m) = pattern.captures_at(s, hay.line_start_byte(lnum)) else {
            break;
        };
        if m.0.whole_start > line_end(lnum) {
            // No match starts in this line; skip to where the next one does.
            lnum = line_of(m.0.whole_start);
            continue;
        }
        let mut cur_line = lnum;
        let mut prev_end: Option<usize> = None;
        loop {
            let start_line = line_of(m.0.start);
            if start_line > cur_line {
                cur_line = start_line;
                lnum = start_line;
            }
            let mut skip_match = false;
            let mut do_again = false;
            let mut nlines = 1;
            let matchcol;
            if prev_end == Some(m.0.start) && m.0.end == m.0.start {
                // An empty match right after the previous one: step over a character.
                if m.0.start >= line_end(cur_line) {
                    skip_match = true;
                    matchcol = m.0.start;
                } else {
                    matchcol = flux_core::pattern::next_char(s, m.0.start);
                }
            } else {
                matchcol = m.0.end;
                prev_end = Some(m.0.end);
                nlines = line_of(m.0.end) - start_line + 1;
                let repl = expand(replacement, &m.1, s);
                out.push(SubMatch {
                    start: hay.char_of(m.0.start),
                    end: hay.char_of(m.0.end),
                    replacement: repl,
                    line: lnum,
                });
                if nlines > 1 {
                    cur_line += nlines - 1;
                    if cur_line <= last {
                        do_again = true;
                    } else {
                        do_all = false;
                    }
                }
                lnum += nlines - 1;
            }
            let lastone = skip_match
                || lnum > last
                || !(do_all || do_again)
                || (matchcol >= line_end(cur_line) && nlines <= 1 && !pattern.is_multiline());
            if lastone {
                break;
            }
            match pattern.captures_at(s, matchcol) {
                Some(next)
                    if next.0.whole_start <= line_end(cur_line)
                        && line_of(next.0.start) == cur_line =>
                {
                    m = next;
                }
                _ => break,
            }
        }
        lines_needed += 1;
        lnum += 1;
    }
    out
}

/// Apply the next substitution of `session`.
fn apply_next(engine: &mut Engine, editor: &mut Editor, session: &mut ConfirmSession) {
    let m = &session.matches[session.next];
    let start = (m.start as isize + session.delta) as usize;
    if session.done.is_empty() {
        // Undo comes back to the start of the first changed line, as in Vim.
        let line = editor.text().char_to_pos(start).0;
        editor.window.cursor = pos(line, 0);
    }
    let end = (m.end as isize + session.delta) as usize;
    let len = m.replacement.chars().count();
    engine.edit(editor, Edit::replace(start..end, m.replacement.clone()));
    session.delta += len as isize - (m.end - m.start) as isize;
    session.lines.insert(m.line);
    session.done.push(session.next);
    session.last_end = Some(start + len);
    session.last_was_sub = true;
    session.next += 1;
}

/// The cursor, marks and message after the substitutions (or all the answers).
fn finish(editor: &mut Editor, session: &ConfirmSession) {
    let nsubs = session.done.len();
    if nsubs == 0 {
        editor.message = None;
        return;
    }
    // The cursor goes to the last changed line, on its first non-blank.
    let text = editor.text();
    let end = session.last_end.unwrap_or(0).min(text.len_chars());
    let line = text.char_to_pos(end).0;
    if !session.flags.ask {
        let col = util::first_non_blank(&util::line(editor, line));
        editor.window.cursor = pos(line, col);
    } else if session.last_was_sub {
        // Vim leaves column 0 of the line after a substitution; after skipping or quitting,
        // the cursor stays on the match it asked about.
        editor.window.cursor = pos(line, 0);
    }
    editor.window.set_curswant = true;
    // `'[` and `']`: the range, grown or shrunk by the lines added or removed.
    let added = editor.text().line_count() as isize - session.lines_before as isize;
    let (l1, l2) = session.range;
    let buffer = editor.current_buffer_mut();
    buffer.marks.set('[', pos(l1 - 1, 0));
    buffer
        .marks
        .set(']', pos((l2 as isize - 1 + added).max(0) as usize, 0));
    if nsubs > editor.options.report {
        editor.info(count_message(nsubs, session.lines.len(), false));
    }
    if session.flags.print {
        let s = util::line(editor, line);
        let text = if session.flags.number {
            format!("{:>3} {s}", line + 1)
        } else {
            s
        };
        editor.info(text);
    }
}

impl ConfirmSession {
    /// Vim saves each substituted line for undo (plus lines a `\r` adds): the line counts
    /// its undo message reports.
    fn saved_lines(&self, editor: &Editor) -> (usize, usize) {
        let before = self.lines.len();
        let added = editor.text().line_count() as isize - self.lines_before as isize;
        (before, (before as isize + added).max(0) as usize)
    }
}

pub(crate) fn count_message(n: usize, lines: usize, count_only: bool) -> String {
    let what = match (count_only, n == 1) {
        (true, true) => "match",
        (true, false) => "matches",
        (false, true) => "substitution",
        (false, false) => "substitutions",
    };
    let line = if lines == 1 { "line" } else { "lines" };
    format!("{n} {what} on {lines} {line}")
}

/// Show the next match and the `replace with …` question.
fn show_confirm(editor: &mut Editor, session: &mut ConfirmSession) {
    let m = &session.matches[session.next];
    let text = editor.text();
    let start = (m.start as isize + session.delta) as usize;
    let end = (m.end as isize + session.delta) as usize;
    let (sl, sc) = text.char_to_pos(start);
    let (el, ec) = text.char_to_pos(end);
    editor.incsearch = Some((Cursor { line: sl, col: sc }, Cursor { line: el, col: ec }));
    editor.window.cursor = pos(sl, sc);
    editor.with_window(|w, m| w.scroll_to_cursor(m));
    editor.message = Some(Message {
        text: format!(
            "replace with {}? (y)es/(n)o/(a)ll/(q)uit/(l)ast/scroll up(^E)/down(^Y)",
            session.replacement_shown
        ),
        kind: MessageKind::Question,
    });
}

impl Engine {
    /// A key answering `:s///c`.
    pub(crate) fn confirm_key(&mut self, editor: &mut Editor, key: crate::key::Key) {
        use crate::key::{KeyCode, Modifiers};
        let Some(mut session) = self.confirm_sub.take() else {
            return;
        };
        let answer = match (key.code, key.mods) {
            (KeyCode::Char(c), Modifiers::NONE) => Some(c),
            (KeyCode::Esc, _) => Some('q'),
            (KeyCode::Char('c'), Modifiers::CTRL) => Some('q'),
            (KeyCode::Char('e'), Modifiers::CTRL) => {
                editor.with_window(|w, m| w.scroll_lines_down(1, m));
                None
            }
            (KeyCode::Char('y'), Modifiers::CTRL) => {
                editor.with_window(|w, m| w.scroll_lines_up(1, m));
                None
            }
            _ => None,
        };
        let mut quit = false;
        let answered_line = session.matches.get(session.next).map(|m| m.line);
        match answer {
            Some('y') => apply_next(self, editor, &mut session),
            Some('l') => {
                apply_next(self, editor, &mut session);
                quit = true;
            }
            Some('n') => {
                session.next += 1;
                session.last_was_sub = false;
            }
            Some('a') => {
                session.flags.ask = false;
                while session.next < session.matches.len() {
                    apply_next(self, editor, &mut session);
                }
            }
            Some('q') => {
                quit = true;
                session.last_was_sub = false;
            }
            _ => {
                self.confirm_sub = Some(session);
                return;
            }
        }
        // Vim puts a line back in the buffer (and shows `[+]`) once all its matches are
        // answered.
        let next_line = session.matches.get(session.next).map(|m| m.line);
        if answered_line.is_some_and(|l| next_line != Some(l) && session.lines.contains(&l)) {
            editor.hide_modified = false;
        }
        if quit || session.next >= session.matches.len() {
            editor.hide_modified = false;
            editor.incsearch = None;
            editor.message = None;
            if !session.done.is_empty() {
                self.saved_lines = Some(session.saved_lines(editor));
            }
            finish(editor, &session);
            self.hold_undo -= 1;
            self.commit(editor);
        } else {
            show_confirm(editor, &mut session);
            self.confirm_sub = Some(session);
        }
    }
}

/// The replacement part of `:s/pat/{string}/flags`, and what follows it (Vim's
/// `skip_substitute`).
fn skip_substitute(s: &str, delim: char) -> (String, &str) {
    let mut out = String::new();
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        if c == delim {
            return (out, &s[i + c.len_utf8()..]);
        }
        out.push(c);
        if c == '\\'
            && let Some((_, next)) = chars.next()
        {
            out.push(next);
        }
    }
    (out, "")
}

/// Vim's `regtilde`: replace `~` with the previous replacement string (`\~` stays a literal
/// `~`).
fn regtilde(sub: &str, previous: Option<&str>) -> String {
    let mut out = String::new();
    let mut chars = sub.chars();
    while let Some(c) = chars.next() {
        match c {
            '~' => out.push_str(previous.unwrap_or("")),
            '\\' => {
                out.push('\\');
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            c => out.push(c),
        }
    }
    out
}

/// Vim's `vim_regsub` (with 'magic'): the text a match is replaced with.
fn expand(sub: &str, groups: &[Option<(usize, usize)>], hay: &str) -> String {
    #[derive(Clone, Copy, PartialEq)]
    enum Case {
        None,
        Upper,
        Lower,
    }
    let mut out = String::new();
    let mut one = Case::None;
    let mut all = Case::None;
    let push = |out: &mut String, c: char, one: &mut Case, all: Case| {
        let case = if *one != Case::None {
            std::mem::replace(one, Case::None)
        } else {
            all
        };
        match case {
            Case::Upper => out.extend(c.to_uppercase()),
            Case::Lower => out.extend(c.to_lowercase()),
            Case::None => out.push(c),
        }
    };
    let mut chars = sub.chars().peekable();
    while let Some(c) = chars.next() {
        let group = match c {
            '&' => Some(0),
            '\\' => match chars.next() {
                Some(d @ '0'..='9') => Some(d as usize - '0' as usize),
                Some('n') => {
                    push(&mut out, '\0', &mut one, all);
                    None
                }
                Some('r') => {
                    out.push('\n');
                    None
                }
                Some('\r') => {
                    push(&mut out, '\r', &mut one, all);
                    None
                }
                Some('t') => {
                    push(&mut out, '\t', &mut one, all);
                    None
                }
                Some('u') => {
                    one = Case::Upper;
                    None
                }
                Some('l') => {
                    one = Case::Lower;
                    None
                }
                Some('U') => {
                    all = Case::Upper;
                    None
                }
                Some('L') => {
                    all = Case::Lower;
                    None
                }
                Some('e' | 'E') => {
                    one = Case::None;
                    all = Case::None;
                    None
                }
                Some(other) => {
                    push(&mut out, other, &mut one, all);
                    None
                }
                None => {
                    push(&mut out, '\\', &mut one, all);
                    None
                }
            },
            // A typed carriage return splits the line too.
            '\r' => {
                out.push('\n');
                None
            }
            c => {
                push(&mut out, c, &mut one, all);
                None
            }
        };
        if let Some(g) = group
            && let Some(Some((a, b))) = groups.get(g)
        {
            for ch in hay[*a..*b].chars() {
                push(&mut out, ch, &mut one, all);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_specials() {
        let hay = "foo bar";
        let g = [Some((0, 7)), Some((0, 3)), Some((4, 7))];
        assert_eq!(expand("[&]", &g, hay), "[foo bar]");
        assert_eq!(expand("\\2 \\1", &g, hay), "bar foo");
        assert_eq!(expand("\\u\\1", &g, hay), "Foo");
        assert_eq!(expand("\\U\\1\\E!", &g, hay), "FOO!");
        assert_eq!(expand("\\L\\uaBC", &g, hay), "Abc");
        assert_eq!(expand("a\\rb", &g, hay), "a\nb");
        assert_eq!(expand("a\\&b\\\\", &g, hay), "a&b\\");
        assert_eq!(regtilde("x~y\\~", Some("P")), "xPy\\~");
    }

    #[test]
    fn skipping_the_replacement() {
        assert_eq!(skip_substitute("a\\/b/g", '/'), ("a\\/b".into(), "g"));
        assert_eq!(skip_substitute("ab", '/'), ("ab".into(), ""));
    }
}
