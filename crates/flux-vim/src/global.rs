//! `:g[lobal]` and `:v[global]` (Vim's `ex_global` and `global_exe`).

use flux_view::Editor;

use crate::engine::Engine;
use crate::ex::Args;
use crate::util::{self, pos};

/// `:[range]g[lobal][!]/{pattern}/[cmd]`, `:[range]v[global]/{pattern}/[cmd]`: run `cmd`
/// (default `:p`) on each line that matches (or doesn't). The lines are marked first; marks
/// move with the changes the commands make, and a deleted line isn't visited.
pub(crate) fn global(engine: &mut Engine, editor: &mut Editor, a: &Args) {
    let invert = a.bang || a.name.starts_with('v');
    if engine.global_lines.is_some() {
        editor.error("E147: Cannot do :global recursive with a range");
        return;
    }
    let args = a.args;
    let (pat, cmd) = match args.chars().next() {
        None => {
            editor.error("E148: Regular expression missing from global");
            return;
        }
        Some('\\') => {
            let kind = args[1..].chars().next();
            let saved = match kind {
                Some('/' | '?') => editor.search.search.clone(),
                Some('&') => editor.search.substitute.clone(),
                _ => {
                    editor.error("E10: \\ should be followed by /, ? or &");
                    return;
                }
            };
            match saved {
                Some(s) => (s.pat, &args[2..]),
                None => {
                    editor.error("E35: No previous regular expression");
                    return;
                }
            }
        }
        Some(delim) if delim.is_ascii_alphanumeric() => {
            editor.error("E146: Regular expressions can't be delimited by letters");
            return;
        }
        Some(delim) => {
            let (pat, rest) = flux_view::search::split_pattern(&args[delim.len_utf8()..], delim);
            let pat = if pat.is_empty() {
                match editor.search.last_pattern() {
                    Some(s) => s.pat.clone(),
                    None => {
                        editor.error("E35: No previous regular expression");
                        return;
                    }
                }
            } else {
                editor.search.add_history(true, &pat);
                pat
            };
            (pat, rest.unwrap_or(""))
        }
    };
    // The pattern becomes both the last search and the last substitute pattern.
    editor.search.set_search(&pat, false);
    editor.search.set_substitute(&pat, false);
    let pattern = match flux_view::search::compile(editor, &pat, false) {
        Ok(p) => p,
        Err(e) => {
            editor.error(e);
            return;
        }
    };
    let hay = flux_view::search::Haystack::lines(
        editor.text(),
        a.line1 - 1,
        a.line2 - 1,
        pattern.is_multiline(),
    );
    let mut marked: Vec<usize> = Vec::new();
    for line in a.line1 - 1..a.line2 {
        let start = hay.line_start_byte(line);
        let matched = pattern
            .find_at(&hay.s, start)
            .is_some_and(|m| m.whole_start <= hay.line_end_byte(line));
        if matched != invert {
            marked.push(line);
        }
    }
    drop(hay);
    if marked.is_empty() {
        editor.info(if invert {
            format!("Pattern found in every line: {pat}")
        } else {
            format!("Pattern not found: {pat}")
        });
        return;
    }
    let cmd = if cmd.trim().is_empty() { "p" } else { cmd };
    crate::normal::set_pcmark(editor);
    let top_before = editor.window.top;
    let lines_before = editor.text().line_count() as isize;
    engine.global_lines = Some(MarkedLines::new(marked));
    editor.global_busy = true;
    engine.global_subs = (0, 0);
    engine.hold_undo += 1;
    let mut printed: Vec<String> = Vec::new();
    let mut positions: Vec<Option<flux_view::Cursor>> = Vec::new();
    let mut error = None;
    let mut need_beginline = false;
    loop {
        let next = engine.global_lines.as_mut().and_then(MarkedLines::pop);
        let Some(line) = next else {
            break;
        };
        if line >= editor.text().line_count() {
            continue;
        }
        editor.window.cursor = pos(line, 0);
        editor.message = None;
        let subs_before = engine.global_subs.0;
        crate::ex::run(engine, editor, cmd);
        if engine.global_subs.0 > subs_before {
            need_beginline = true;
        }
        // Line counts are reported once at the end; printed lines and errors are kept.
        match editor.message.take() {
            Some(m) if m.is_error() => error = Some(m),
            Some(m) if m.kind == flux_view::MessageKind::Full => {
                let cursor = editor.cursor();
                for l in m.text.lines() {
                    printed.push(l.to_string());
                    positions.push(Some(cursor));
                }
            }
            _ => {}
        }
        editor.hit_enter = false;
        editor.more_top = None;
    }
    engine.global_lines = None;
    editor.global_busy = false;
    engine.hold_undo -= 1;
    let text = editor.text();
    let line = editor.cursor().line.min(text.last_line());
    if need_beginline {
        let col = util::first_non_blank(&util::line(editor, line));
        editor.window.cursor = pos(line, col);
    } else {
        editor.window.cursor.line = line;
        crate::normal::normalize_cursor(editor);
    }
    editor.window.set_curswant = true;
    let (nsubs, nlines) = std::mem::take(&mut engine.global_subs);
    if nsubs > 0 {
        // Like `:s` on its own, each substituted line counts for undo's message.
        let added = editor.text().line_count() as isize - lines_before;
        engine.saved_lines = Some((nlines, (nlines as isize + added).max(0) as usize));
    }
    if !printed.is_empty() {
        // The output starts below the command line, which stays in view as its first line.
        if let Some(line) = &engine.typed_cmdline {
            printed.insert(0, format!(":{line}"));
            positions.insert(0, None);
        }
        // Neovim redraws for the first line printed before showing the output.
        let first = positions.iter().find_map(|p| *p);
        editor.full_message(printed.join("\n"));
        editor.message_positions = positions;
        editor.more_restore_top = Some(top_before);
        if let Some(first) = first {
            let (top, last) = (editor.window.top, editor.cursor());
            editor.window.top = top_before;
            editor.window.cursor = first;
            editor.with_window(|w, m| w.scroll_to_cursor(m));
            editor.fresh_message_base = Some((editor.window.top, first));
            editor.window.top = top;
            editor.window.cursor = last;
        }
    } else if let Some(e) = error {
        editor.message = Some(e);
    } else if nsubs > editor.options.report {
        editor.info(crate::substitute::count_message(nsubs, nlines, false));
    } else if let Some(msg) = util::more_lines_message(
        editor.text().line_count() as isize - lines_before,
        editor.options.report,
    ) {
        editor.info(msg);
    }
}

/// The lines `:g` still has to visit, kept in line order as edits move them (Vim marks the
/// lines themselves). Shifting every line after an edit is a range update on a Fenwick tree,
/// so a `:g` over many lines doesn't take quadratic time.
#[derive(Debug)]
pub(crate) struct MarkedLines {
    base: Vec<isize>,
    alive: Vec<bool>,
    tree: Vec<isize>,
    next: usize,
}

impl MarkedLines {
    pub(crate) fn new(lines: Vec<usize>) -> Self {
        let n = lines.len();
        Self {
            base: lines.into_iter().map(|l| l as isize).collect(),
            alive: vec![true; n],
            tree: vec![0; n + 1],
            next: 0,
        }
    }

    /// Add `d` to every line from index `i` on.
    fn add_from(&mut self, i: usize, d: isize) {
        let mut k = i + 1;
        while k < self.tree.len() {
            self.tree[k] += d;
            k += k.isolate_lowest_one();
        }
    }

    fn shift_at(&self, i: usize) -> isize {
        let mut k = i + 1;
        let mut sum = 0;
        while k > 0 {
            sum += self.tree[k];
            k -= k.isolate_lowest_one();
        }
        sum
    }

    fn value(&self, i: usize) -> isize {
        self.base[i] + self.shift_at(i)
    }

    /// The first index (among the lines not yet visited, which stay in order) whose line is
    /// above `line`.
    fn first_above(&self, line: isize) -> usize {
        let (mut lo, mut hi) = (self.next, self.base.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.value(mid) > line {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        lo
    }

    /// Move the lines for an edit.
    pub(crate) fn adjust(&mut self, shift: &flux_view::LineShift) {
        let (first, last, delta) = shift.extent();
        let start = self.first_above(first as isize - 1);
        let after = self.first_above(last as isize);
        for i in start..after {
            if !self.alive[i] {
                continue;
            }
            let v = self.value(i);
            match shift.adjust(pos(v.max(0) as usize, 0)) {
                Some(p) => self.base[i] += p.line as isize - v,
                None => self.alive[i] = false,
            }
        }
        if delta != 0 && after < self.base.len() {
            self.add_from(after, delta);
        }
    }

    /// The next line to visit.
    pub(crate) fn pop(&mut self) -> Option<usize> {
        while self.next < self.base.len() {
            let i = self.next;
            self.next += 1;
            if self.alive[i] {
                self.alive[i] = false;
                return Some(self.value(i).max(0) as usize);
            }
        }
        None
    }
}
