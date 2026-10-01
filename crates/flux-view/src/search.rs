//! Searching: the last patterns and Vim's `searchit` / `do_search` (`search.c`).

use flux_core::Text;
use flux_core::pattern::{Pattern, PatternOptions};

use crate::{Cursor, Editor, MessageKind};

/// A column past the end of every line (Vim's `MAXCOL`).
pub const MAXCOL: usize = usize::MAX / 4;

/// `{offset}` of a search command (`:h search-offset`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SearchOffset {
    /// A line offset (`/pat/+2`): the match's line plus `off`, and the motion is linewise.
    pub line: bool,
    /// Relative to the end of the match (`/pat/e`).
    pub end: bool,
    pub off: i64,
}

/// A remembered pattern, with whether 'smartcase' was off for it (`*` and `#`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedPattern {
    pub pat: String,
    pub no_smartcase: bool,
}

/// `:s` flags (`:h s_flags`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubFlags {
    /// `g`: every match in the line.
    pub all: bool,
    /// `c`: confirm each one.
    pub ask: bool,
    /// `n`: only count.
    pub count: bool,
    /// Report "pattern not found" (`e` turns it off).
    pub error: bool,
    /// `p`, `#`, `l`: print the last line.
    pub print: bool,
    pub number: bool,
    pub list: bool,
    /// `i` (Some(true)) or `I` (Some(false)); otherwise 'ignorecase'.
    pub ignore_case: Option<bool>,
}

impl Default for SubFlags {
    fn default() -> Self {
        Self {
            all: false,
            ask: false,
            count: false,
            error: true,
            print: false,
            number: false,
            list: false,
            ignore_case: None,
        }
    }
}

/// Vim's `spats`: the last search and substitute patterns and the search direction and offset.
#[derive(Debug, Clone)]
pub struct SearchState {
    /// The last search pattern (`/`, `*`, …).
    pub search: Option<SavedPattern>,
    /// The last substitute pattern (`:s`).
    pub substitute: Option<SavedPattern>,
    /// Which of the two was used last; `n` and hlsearch use that one.
    pub last_was_substitute: bool,
    pub offset: SearchOffset,
    pub forward: bool,
    /// `:nohlsearch`: highlighting is off until the next search.
    pub no_hlsearch: bool,
    /// The last substitute string, for `~` in patterns and replacements (Vim's
    /// `reg_prev_sub`, with its own `~`s expanded).
    pub last_replacement: Option<String>,
    /// The last `:s` replacement as typed, for `:&` and `&` (Vim's `old_sub`).
    pub last_sub_command: Option<String>,
    /// The flags of the last `:s`, kept by `:&&` (Vim's static `subflags`).
    pub sub_flags: SubFlags,
    /// Command-line history for `:` and for searches, oldest first.
    pub cmd_history: Vec<String>,
    pub search_history: Vec<String>,
}

impl Default for SearchState {
    fn default() -> Self {
        Self {
            search: None,
            substitute: None,
            last_was_substitute: false,
            offset: SearchOffset::default(),
            forward: true,
            no_hlsearch: false,
            last_replacement: None,
            last_sub_command: None,
            sub_flags: SubFlags::default(),
            cmd_history: Vec::new(),
            search_history: Vec::new(),
        }
    }
}

impl SearchState {
    /// The pattern `n` and highlighting use (Vim's `RE_LAST`).
    pub fn last_pattern(&self) -> Option<&SavedPattern> {
        if self.last_was_substitute {
            self.substitute.as_ref().or(self.search.as_ref())
        } else {
            self.search.as_ref().or(self.substitute.as_ref())
        }
    }

    /// Remember a search pattern (Vim's `save_re_pat(RE_SEARCH, …)`).
    pub fn set_search(&mut self, pat: &str, no_smartcase: bool) {
        self.search = Some(SavedPattern {
            pat: pat.to_string(),
            no_smartcase,
        });
        self.last_was_substitute = false;
        self.no_hlsearch = false;
    }

    /// Remember a substitute pattern.
    pub fn set_substitute(&mut self, pat: &str, no_smartcase: bool) {
        self.substitute = Some(SavedPattern {
            pat: pat.to_string(),
            no_smartcase,
        });
        self.last_was_substitute = true;
        self.no_hlsearch = false;
    }

    /// Add to a history, moving an existing equal entry to the end (Vim's `add_to_history`).
    pub fn add_history(&mut self, search: bool, entry: &str) {
        if entry.is_empty() {
            return;
        }
        let h = if search {
            &mut self.search_history
        } else {
            &mut self.cmd_history
        };
        h.retain(|e| e != entry);
        h.push(entry.to_string());
        if h.len() > 10_000 {
            h.remove(0);
        }
    }
}

/// Compile `pat` with the editor's 'ignorecase', 'smartcase' and last substitute string.
pub fn compile(editor: &Editor, pat: &str, no_smartcase: bool) -> Result<Pattern, String> {
    Pattern::new(
        pat,
        PatternOptions {
            ignorecase: editor.options.ignorecase,
            smartcase: editor.options.smartcase && !no_smartcase,
            last_substitute: editor.search.last_replacement.as_deref(),
        },
    )
    .map_err(|e| e.0)
}

/// The buffer (or some of its lines) as one string, for matching, with conversions between
/// byte offsets (into `s`) and positions.
pub struct Haystack<'a> {
    text: &'a Text,
    pub s: String,
    /// Where `s` starts in the buffer, in bytes.
    pub base: usize,
}

/// A match found by [`Haystack::regexec`], in chars.
#[derive(Debug, Clone, Copy)]
struct LineMatch {
    start: Cursor,
    end: Cursor,
}

impl<'a> Haystack<'a> {
    pub fn new(text: &'a Text) -> Self {
        Self {
            text,
            s: text.rope().to_string(),
            base: 0,
        }
    }

    /// Lines `first..=last` only (to the end of the buffer if a match can span lines), for
    /// commands that work on a range. Only those lines may be searched.
    pub fn lines(text: &'a Text, first: usize, last: usize, multiline: bool) -> Self {
        let rope = text.rope();
        let base = rope.line_to_byte(first.min(text.line_count() - 1));
        let end = if multiline || last + 1 >= text.line_count() {
            rope.len_bytes()
        } else {
            rope.line_to_byte(last + 1) - 1
        };
        Self {
            text,
            s: rope.byte_slice(base..end.max(base)).to_string(),
            base,
        }
    }

    /// The buffer line of byte offset `byte`.
    pub fn line_of(&self, byte: usize) -> usize {
        let b = (byte + self.base).min(self.text.rope().len_bytes());
        self.text.rope().byte_to_line(b)
    }

    /// The buffer char offset of byte offset `byte`.
    pub fn char_of(&self, byte: usize) -> usize {
        let b = (byte + self.base).min(self.text.rope().len_bytes());
        self.text.rope().byte_to_char(b)
    }

    pub fn line_count(&self) -> usize {
        self.text.line_count()
    }

    pub fn line_len(&self, line: usize) -> usize {
        self.text.line_len(line)
    }

    pub fn line_start_byte(&self, line: usize) -> usize {
        self.text
            .rope()
            .line_to_byte(line)
            .saturating_sub(self.base)
    }

    pub fn line_end_byte(&self, line: usize) -> usize {
        if line + 1 < self.line_count() {
            (self.text.rope().line_to_byte(line + 1) - 1)
                .saturating_sub(self.base)
                .min(self.s.len())
        } else {
            self.s.len()
        }
    }

    /// The position of byte offset `byte`.
    pub fn pos(&self, byte: usize) -> Cursor {
        let c = self.char_of(byte.min(self.s.len()));
        let (line, col) = self.text.char_to_pos(c);
        Cursor { line, col }
    }

    pub fn byte(&self, p: Cursor) -> usize {
        let line = p.line.min(self.line_count() - 1);
        let c = self.text.pos_to_char(line, p.col);
        self.text.rope().char_to_byte(c).saturating_sub(self.base)
    }

    /// The first line at or after `line` where a match starts.
    fn next_match_line(&self, pat: &Pattern, line: usize) -> Option<usize> {
        let m = pat.find_at(&self.s, self.line_start_byte(line))?;
        Some(self.line_of(m.whole_start.min(self.s.len())))
    }

    /// The last line at or before `line` where a match starts. Looks back in windows that
    /// double in size, so a distant match costs about as much as scanning to it once.
    fn prev_match_line(&self, pat: &Pattern, line: usize) -> Option<usize> {
        let mut hi = line;
        let mut size = 64;
        loop {
            let lo = hi.saturating_sub(size - 1);
            // Matching one line at a time can't cross the window's end, so cut the text there.
            let end = if pat.is_multiline() {
                self.s.len()
            } else {
                self.line_end_byte(hi)
            };
            let hay = &self.s[..end];
            let mut last = None;
            let mut from = lo;
            while from <= hi {
                let Some(m) = pat.find_at(hay, self.line_start_byte(from)) else {
                    break;
                };
                let ml = self.line_of(m.whole_start.min(self.s.len()));
                if ml > hi {
                    break;
                }
                last = Some(ml);
                from = ml + 1;
            }
            if last.is_some() {
                return last;
            }
            if lo == 0 {
                return None;
            }
            hi = lo - 1;
            size *= 2;
        }
    }

    /// Vim's `vim_regexec_multi`: the first match starting in `line` at or after `col`.
    fn regexec(&self, pat: &Pattern, line: usize, col: usize) -> Option<LineMatch> {
        let from = self.byte(Cursor { line, col });
        let m = pat.find_at(&self.s, from)?;
        if m.whole_start > self.line_end_byte(line) {
            return None;
        }
        Some(LineMatch {
            start: self.pos(m.start),
            end: self.pos(m.end),
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SearchFlags {
    /// Accept a match at the start position (`SEARCH_START`).
    pub start: bool,
    /// Compare match ends with the start and return the end (`SEARCH_END`, for `/pat/e`).
    pub end: bool,
    pub wrapscan: bool,
}

/// Where a search found its match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Found {
    pub pos: Cursor,
    /// The other end of the match: its end, or its start with `SEARCH_END`.
    pub end: Cursor,
    pub wrapped: bool,
}

/// Why a search failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotFound {
    /// Nowhere (with 'wrapscan').
    Nowhere,
    HitTop,
    HitBottom,
}

/// Vim's `searchit`. `start` is `(line, col)` where line may be -1 (before the first line, with
/// `col` [`MAXCOL`]) or the line count (after the last). Finds the `count`'th match.
pub fn searchit(
    hay: &Haystack<'_>,
    pat: &Pattern,
    start: (isize, usize),
    forward: bool,
    count: usize,
    flags: SearchFlags,
) -> Result<Found, NotFound> {
    let lines = hay.line_count() as isize;
    let dir: isize = if forward { 1 } else { -1 };
    let mut pos = start;
    let mut count = count.max(1);
    let mut wrapped = false;
    let mut first_match = true;
    loop {
        let start_char_len: i64 = if pos.1 == MAXCOL { 0 } else { 1 };
        let extra_col = if forward != flags.start {
            start_char_len
        } else {
            0
        };
        let start_pos = pos;
        let sp_col = if start_pos.1 == MAXCOL {
            MAXCOL as i64
        } else {
            start_pos.1 as i64
        };
        let mut found = None;
        let mut at_first_line = true;
        if pos.0 < 0 {
            pos = (0, 0);
            at_first_line = false;
        }
        let mut lnum = if !forward && start_pos.1 == 0 && !flags.start {
            at_first_line = false;
            pos.0 - 1
        } else {
            pos.0
        };
        'loops: for pass in 0..2 {
            while lnum >= 0 && lnum < lines {
                // Go straight to the next line with a match: the lines in between would each
                // be tried and passed over.
                let next = if forward {
                    hay.next_match_line(pat, lnum as usize)
                } else {
                    hay.prev_match_line(pat, lnum as usize)
                };
                match next {
                    None => {
                        lnum = if forward { lines } else { -1 };
                        break;
                    }
                    Some(t) if t as isize != lnum => {
                        let past_start = if forward {
                            t as isize > start_pos.0
                        } else {
                            (t as isize) < start_pos.0
                        };
                        if pass == 1 && past_start {
                            break;
                        }
                        lnum = t as isize;
                        at_first_line = false;
                    }
                    Some(_) => {}
                }
                let l = lnum as usize;
                if let Some(mut m) = hay.regexec(pat, l, 0) {
                    let line_len = hay.line_len(l);
                    let mut ok = true;
                    if forward && at_first_line {
                        loop {
                            if m.start.line != l {
                                break;
                            }
                            let before = if flags.end && first_match {
                                m.end.line == l && (m.end.col as i64) - 1 < sp_col + extra_col
                            } else {
                                (m.start.col as i64) - i64::from(m.start.col >= line_len)
                                    < sp_col + extra_col
                            };
                            if !before {
                                break;
                            }
                            // 'cpoptions' has `c`: go on from the end of the match.
                            if m.end.line > l {
                                ok = false;
                                break;
                            }
                            let mut matchcol = m.end.col;
                            if matchcol == m.start.col && matchcol < line_len {
                                matchcol += 1;
                            }
                            if matchcol == 0 && flags.start {
                                break;
                            }
                            if matchcol >= line_len {
                                ok = false;
                                break;
                            }
                            match hay.regexec(pat, l, matchcol) {
                                Some(next) => m = next,
                                None => {
                                    ok = false;
                                    break;
                                }
                            }
                        }
                    }
                    if !forward {
                        ok = false;
                        let mut best = m;
                        let sp_line = start_pos.0;
                        loop {
                            let accept = pass == 1
                                || if flags.end {
                                    (m.end.line as isize) < sp_line
                                        || (m.end.line as isize == sp_line
                                            && (m.end.col as i64) - 1 < sp_col + extra_col)
                                } else {
                                    (m.start.line as isize) < sp_line
                                        || (m.start.line as isize == sp_line
                                            && (m.start.col as i64) < sp_col + extra_col)
                                };
                            if !accept {
                                break;
                            }
                            ok = true;
                            best = m;
                            if m.end.line > l {
                                break;
                            }
                            let mut matchcol = m.end.col;
                            if matchcol == m.start.col && matchcol < line_len {
                                matchcol += 1;
                            }
                            if matchcol >= line_len {
                                break;
                            }
                            match hay.regexec(pat, l, matchcol) {
                                Some(next) => m = next,
                                None => break,
                            }
                        }
                        m = best;
                    }
                    if ok {
                        let (p, e) = if flags.end && m.start != m.end {
                            let mut p = m.end;
                            if p.col == 0 {
                                if p.line > 0 {
                                    p.line -= 1;
                                    p.col = hay.line_len(p.line);
                                }
                            } else {
                                p.col -= 1;
                            }
                            (p, m.start)
                        } else {
                            (m.start, m.end)
                        };
                        found = Some((p, e));
                        first_match = false;
                        break 'loops;
                    }
                    // A match on this line, but not an acceptable one: on to the next line
                    // (Vim's `continue`, which skips the stop check below).
                    lnum += dir;
                    at_first_line = false;
                    continue;
                }
                if pass == 1 && lnum == start_pos.0 {
                    break;
                }
                lnum += dir;
                at_first_line = false;
            }
            if !flags.wrapscan || found.is_some() || pass == 1 {
                break;
            }
            lnum = if forward { 0 } else { lines - 1 };
            at_first_line = false;
            wrapped = true;
        }
        let Some((p, e)) = found else {
            return Err(if flags.wrapscan {
                NotFound::Nowhere
            } else if lnum < 0 {
                NotFound::HitTop
            } else {
                NotFound::HitBottom
            });
        };
        count -= 1;
        if count == 0 {
            return Ok(Found {
                pos: p,
                end: e,
                wrapped,
            });
        }
        pos = (p.line as isize, p.col);
    }
}

/// Vim's `inc`: one character forward, onto the end of the line, then to the next line.
/// Returns -1 at the end of the buffer, 1 when it moved to another line, 2 when it moved onto
/// the end of a line.
fn inc(hay: &Haystack<'_>, p: &mut (isize, usize)) -> i32 {
    if p.1 != MAXCOL && p.0 >= 0 && (p.0 as usize) < hay.line_count() {
        let len = hay.line_len(p.0 as usize);
        if p.1 < len {
            p.1 += 1;
            return if p.1 < len { 0 } else { 2 };
        }
    }
    if p.0 + 1 < hay.line_count() as isize {
        p.0 += 1;
        p.1 = 0;
        return 1;
    }
    -1
}

fn incl(hay: &Haystack<'_>, p: &mut (isize, usize)) -> i32 {
    let r = inc(hay, p);
    if r >= 1 && p.1 > 0 { inc(hay, p) } else { r }
}

/// Vim's `dec`.
fn dec(hay: &Haystack<'_>, p: &mut (isize, usize)) -> i32 {
    if p.1 == MAXCOL {
        p.1 = hay.line_len(p.0.max(0) as usize);
        return 0;
    }
    if p.1 > 0 {
        p.1 -= 1;
        return 0;
    }
    if p.0 > 0 {
        p.0 -= 1;
        p.1 = hay.line_len(p.0 as usize);
        return 1;
    }
    -1
}

fn decl(hay: &Haystack<'_>, p: &mut (isize, usize)) -> i32 {
    let r = dec(hay, p);
    if r == 1 && p.1 > 0 { dec(hay, p) } else { r }
}

/// Split a search command line into the pattern and what follows its closing `delim` (Vim's
/// `skip_regexp`): a `[...]` collection may contain `delim`, and so may an escape.
pub fn split_pattern(s: &str, delim: char) -> (String, Option<&str>) {
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let mut i = 0;
    let mut pat = String::new();
    while i < chars.len() {
        let (at, c) = chars[i];
        if c == delim {
            return (pat, Some(&s[at + c.len_utf8()..]));
        }
        if c == '[' {
            // Copy a collection whole if it's closed.
            if let Some(len) = collection_len(&chars[i..]) {
                for &(_, ch) in &chars[i..i + len] {
                    pat.push(ch);
                }
                i += len;
                continue;
            }
        }
        if c == '\\' && i + 1 < chars.len() {
            let next = chars[i + 1].1;
            // In a `?` search `\?` stands for `?`.
            if next == delim && delim == '?' {
                pat.push('?');
            } else {
                pat.push('\\');
                pat.push(next);
            }
            i += 2;
            continue;
        }
        pat.push(c);
        i += 1;
    }
    (pat, None)
}

/// The length of a `[...]` collection starting at `chars[0]`, if it's closed.
fn collection_len(chars: &[(usize, char)]) -> Option<usize> {
    let mut i = 1;
    if chars.get(i).map(|c| c.1) == Some('^') {
        i += 1;
    }
    if chars.get(i).map(|c| c.1) == Some(']') {
        i += 1;
    }
    while i < chars.len() {
        match chars[i].1 {
            ']' => return Some(i + 1),
            '\\' => i += 2,
            _ => i += 1,
        }
    }
    None
}

/// Parse `{offset}` (`e`, `s`, `b`, `+N`, `-N`, `eN`, …), returning it and what follows.
fn parse_offset(s: &str) -> (SearchOffset, &str) {
    let mut off = SearchOffset::default();
    let mut rest = s;
    match rest.chars().next() {
        Some('+' | '-' | '0'..='9') => off.line = true,
        Some(c @ ('e' | 's' | 'b')) => {
            off.end = c == 'e';
            rest = &rest[1..];
        }
        _ => {}
    }
    if let Some(c @ ('+' | '-' | '0'..='9')) = rest.chars().next() {
        // `+` and `-` alone mean one.
        let (sign, from) = match c {
            '-' => (-1, 1),
            '+' => (1, 1),
            _ => (1, 0),
        };
        let len = rest[from..]
            .chars()
            .take_while(char::is_ascii_digit)
            .count();
        let n: i64 = if len == 0 {
            1
        } else {
            rest[from..from + len].parse().unwrap_or(0)
        };
        off.off = sign * n;
        rest = &rest[from + len..];
    }
    (off, rest)
}

/// What [`do_search`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchResult {
    pub pos: Cursor,
    /// A line offset was added: the motion is linewise.
    pub linewise: bool,
    /// An `e` offset: the motion is inclusive.
    pub inclusive: bool,
    /// The search went past the end (or start) and continued at the other end.
    pub wrapped: bool,
}

/// What kind of search [`do_search`] runs.
#[derive(Debug, Clone, Copy)]
pub enum SearchCmd<'a> {
    /// `/` or `?` with what was typed on the command line.
    Typed { forward: bool, input: &'a str },
    /// `n` (`reverse` false) or `N`.
    Next { reverse: bool },
    /// `*`, `#` and friends: a pattern that ignores 'smartcase'.
    Word { forward: bool, pattern: &'a str },
}

/// Vim's `do_search` for Normal-mode searches: parse the pattern and offset, search, apply the
/// offset and show `/pattern` with the match count (`[2/5]`) or an error. Doesn't move the
/// cursor.
pub fn do_search(editor: &mut Editor, cmd: SearchCmd<'_>, count: usize) -> Option<SearchResult> {
    // Direction, pattern (empty for the last one) and offset.
    let (forward, typed): (bool, Option<String>) = match cmd {
        SearchCmd::Typed { forward, input } => {
            editor.search.forward = forward;
            let delim = if forward { '/' } else { '?' };
            let (pat, rest) = split_pattern(input, delim);
            if let Some(rest) = rest {
                let (off, _rest) = parse_offset(rest);
                editor.search.offset = off;
            } else if !pat.is_empty() {
                editor.search.offset = SearchOffset::default();
            }
            (forward, Some(pat))
        }
        SearchCmd::Next { reverse } => (editor.search.forward != reverse, None),
        SearchCmd::Word { forward, pattern } => {
            editor.search.forward = forward;
            editor.search.offset = SearchOffset::default();
            (forward, Some(pattern.to_string()))
        }
    };
    let dirc = if forward { '/' } else { '?' };
    let no_scs = matches!(cmd, SearchCmd::Word { .. });
    // The pattern to use, remembering a new one.
    let (pat, no_smartcase) = match typed.filter(|p| !p.is_empty()) {
        Some(p) => {
            editor.search.set_search(&p, no_scs);
            (p, no_scs)
        }
        None => match editor.search.last_pattern() {
            Some(saved) => (saved.pat.clone(), saved.no_smartcase),
            None => {
                editor.error("E35: No previous regular expression");
                return None;
            }
        },
    };
    editor.search.no_hlsearch = false;
    let off = editor.search.offset;
    let shown = editor
        .search
        .search
        .as_ref()
        .map_or(pat.clone(), |s| s.pat.clone());

    let pattern = match compile(editor, &pat, no_smartcase) {
        Ok(p) => p,
        Err(e) => {
            editor.error(e);
            return None;
        }
    };
    let text = editor.text();
    let hay = Haystack::new(text);
    let cur = editor.cursor();
    let mut start = (cur.line as isize, cur.col);
    // Start from the other side of a character offset, so `/pat/e+2` doesn't get stuck.
    if !off.line && off.off != 0 {
        if off.off > 0 {
            for _ in 0..off.off {
                if decl(&hay, &mut start) == -1 {
                    start = (-1, MAXCOL);
                    break;
                }
            }
        } else {
            for _ in 0..(-off.off) {
                if incl(&hay, &mut start) == -1 {
                    start = (hay.line_count() as isize, 0);
                    break;
                }
            }
        }
    }
    let flags = SearchFlags {
        start: false,
        end: off.end,
        wrapscan: editor.options.wrapscan,
    };
    let found = match searchit(&hay, &pattern, start, forward, count, flags) {
        Ok(f) => f,
        Err(why) => {
            let msg = match why {
                NotFound::Nowhere => format!("E486: Pattern not found: {pat}"),
                NotFound::HitTop => format!("E384: Search hit TOP without match for: {pat}"),
                NotFound::HitBottom => {
                    format!("E385: Search hit BOTTOM without match for: {pat}")
                }
            };
            editor.error(msg);
            return None;
        }
    };
    // The offset.
    let mut p = (found.pos.line as isize, found.pos.col);
    let mut linewise = false;
    if off.line {
        let l = (p.0 + off.off as isize).clamp(0, hay.line_count() as isize - 1);
        p = (l, 0);
        linewise = true;
    } else if off.off > 0 {
        for _ in 0..off.off {
            if incl(&hay, &mut p) == -1 {
                break;
            }
        }
    } else if off.off < 0 {
        for _ in 0..(-off.off) {
            if decl(&hay, &mut p) == -1 {
                break;
            }
        }
    }
    let pos = Cursor {
        line: p.0.max(0) as usize,
        col: p.1,
    };

    // `/pattern` (with the offset) and the match count on the command line.
    let mut msg = format!("{dirc}{shown}");
    if off.line || off.end || off.off != 0 {
        msg.push(dirc);
        if off.end {
            msg.push('e');
        } else if !off.line {
            msg.push('s');
        }
        if off.off != 0 || off.line {
            msg.push_str(&format!("{:+}", off.off));
        }
    }
    if let Some(stat) = search_stat(&hay, &pattern, pos, found.wrapped) {
        let width = editor.screen_size().0.saturating_sub(13);
        let used = msg.chars().count();
        if used + stat.len() < width {
            msg.push_str(&" ".repeat(width - used - stat.chars().count()));
            msg.push_str(&stat);
        }
    }
    editor.message = Some(crate::Message {
        text: msg,
        kind: MessageKind::Info,
    });
    editor.keep_msg = true;
    editor.kept_message = editor.message.clone();
    Some(SearchResult {
        pos,
        linewise,
        inclusive: off.end,
        wrapped: found.wrapped,
    })
}

/// A `/pat/` or `?pat?` address in an Ex range (Vim's `get_address`): search from line
/// `from` (1-based; forward from its end, backward from its start) and return the line found
/// (0-based) and how many bytes of `input` (after the first delimiter) were used. Like a
/// search command it remembers the pattern, direction and a line offset. Errors are shown.
#[allow(clippy::result_unit_err)]
pub fn address_search(
    editor: &mut Editor,
    delim: char,
    input: &str,
    from: isize,
) -> Result<(usize, usize), ()> {
    let forward = delim == '/';
    let (pat, rest) = split_pattern(input, delim);
    let mut used = input.len() - rest.map_or(0, str::len);
    editor.search.forward = forward;
    if let Some(rest) = rest.filter(|_| !pat.is_empty()) {
        let (off, after) = parse_offset(rest);
        if off.end || (!off.line && off.off != 0) {
            // Character offsets mean nothing for a line address; Vim doesn't parse them here.
            editor.search.offset = SearchOffset::default();
        } else {
            editor.search.offset = off;
            used += rest.len() - after.len();
        }
    } else if !pat.is_empty() {
        editor.search.offset = SearchOffset::default();
    }
    let (pat, no_scs) = if pat.is_empty() {
        match editor.search.last_pattern() {
            Some(p) => (p.pat.clone(), p.no_smartcase),
            None => {
                editor.error("E35: No previous regular expression");
                return Err(());
            }
        }
    } else {
        editor.search.set_search(&pat, false);
        (pat, false)
    };
    let off = editor.search.offset;
    let line = search_line(editor, &pat, no_scs, forward, from)?;
    let line = if off.line {
        (line as i64 + off.off).clamp(0, editor.text().last_line() as i64) as usize
    } else {
        line
    };
    Ok((line, used))
}

/// `\/`, `\?` (the last search pattern) and `\&` (the last substitute pattern) in an Ex range.
#[allow(clippy::result_unit_err)]
pub fn repeat_address_search(editor: &mut Editor, kind: char, from: isize) -> Result<usize, ()> {
    let saved = if kind == '&' {
        editor.search.substitute.clone()
    } else {
        editor.search.search.clone()
    };
    let Some(saved) = saved else {
        editor.error("E35: No previous regular expression");
        return Err(());
    };
    search_line(editor, &saved.pat, saved.no_smartcase, kind != '?', from)
}

/// The line of the next match of `pat` after (or before) line `from` (1-based), with
/// 'wrapscan'. Errors are shown.
fn search_line(
    editor: &mut Editor,
    pat: &str,
    no_scs: bool,
    forward: bool,
    from: isize,
) -> Result<usize, ()> {
    let pattern = compile(editor, pat, no_scs).map_err(|e| editor.error(e))?;
    let text = editor.text();
    let hay = Haystack::new(text);
    let line = (from - 1).clamp(-1, hay.line_count() as isize);
    let start = if forward { (line, MAXCOL) } else { (line, 0) };
    let flags = SearchFlags {
        start: false,
        end: false,
        wrapscan: editor.options.wrapscan,
    };
    match searchit(&hay, &pattern, start, forward, 1, flags) {
        Ok(found) => Ok(found.pos.line),
        Err(why) => {
            drop(hay);
            editor.error(match why {
                NotFound::Nowhere => format!("E486: Pattern not found: {pat}"),
                NotFound::HitTop => format!("E384: Search hit TOP without match for: {pat}"),
                NotFound::HitBottom => {
                    format!("E385: Search hit BOTTOM without match for: {pat}")
                }
            });
            Err(())
        }
    }
}

/// Neovim's `[cur/total]` search count (with `W ` when the search wrapped), counting up to
/// 'maxsearchcount' (999) matches.
fn search_stat(hay: &Haystack<'_>, pat: &Pattern, pos: Cursor, wrapped: bool) -> Option<String> {
    const MAX: usize = 999;
    let flags = SearchFlags {
        start: false,
        end: false,
        wrapscan: false,
    };
    let (mut cur, mut cnt) = (0, 0);
    let mut last = (-1isize, 0usize);
    while let Ok(f) = searchit(hay, pat, last, true, 1, flags) {
        cnt += 1;
        if (f.pos.line, f.pos.col) <= (pos.line, pos.col) {
            cur = cnt;
        }
        last = (f.pos.line as isize, f.pos.col);
        if cnt > MAX {
            break;
        }
    }
    if cur == 0 {
        return None;
    }
    let mut s = if cnt > MAX && cur > MAX {
        format!("[>{MAX}/>{MAX}]")
    } else if cnt > MAX {
        format!("[{cur}/>{MAX}]")
    } else {
        format!("[{cur}/{cnt}]")
    };
    if wrapped {
        s = format!("W {s}");
    }
    Some(s)
}

/// Matches of the last search pattern in `lines`, for 'hlsearch': `(start, end)` positions.
pub fn highlight_matches(
    editor: &Editor,
    text: &Text,
    lines: std::ops::Range<usize>,
) -> Vec<(Cursor, Cursor)> {
    let Some(saved) = editor.search.last_pattern() else {
        return Vec::new();
    };
    let Ok(pattern) = compile(editor, &saved.pat, saved.no_smartcase) else {
        return Vec::new();
    };
    matches_in(text, &pattern, lines)
}

/// Every match of `pattern` starting in `lines`.
pub fn matches_in(
    text: &Text,
    pattern: &Pattern,
    lines: std::ops::Range<usize>,
) -> Vec<(Cursor, Cursor)> {
    if lines.is_empty() || text.has_no_lines() {
        return Vec::new();
    }
    let rope = text.rope();
    let last = lines.end.min(text.line_count()) - 1;
    let start_byte = rope.line_to_byte(lines.start);
    // Search a little past the last line so matches can run into it.
    let end_line = (last + 1).min(text.line_count());
    let end_byte = if pattern.is_multiline() {
        rope.len_bytes()
    } else if end_line < text.line_count() {
        rope.line_to_byte(end_line)
    } else {
        rope.len_bytes()
    };
    let s = rope.byte_slice(start_byte..end_byte).to_string();
    let limit = if last + 1 < text.line_count() {
        rope.line_to_byte(last + 1) - start_byte
    } else {
        s.len() + 1
    };
    let to_pos = |b: usize| {
        let c = rope.byte_to_char(start_byte + b);
        let (line, col) = text.char_to_pos(c);
        Cursor { line, col }
    };
    let mut out = Vec::new();
    for m in pattern.find_iter(&s) {
        if m.whole_start >= limit {
            break;
        }
        out.push((to_pos(m.start), to_pos(m.end)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &Text, pat: &str, from: (isize, usize), forward: bool) -> Option<(usize, usize)> {
        let p = Pattern::new(pat, PatternOptions::default()).unwrap();
        let hay = Haystack::new(text);
        let flags = SearchFlags {
            wrapscan: true,
            ..Default::default()
        };
        searchit(&hay, &p, from, forward, 1, flags)
            .ok()
            .map(|f| (f.pos.line, f.pos.col))
    }

    #[test]
    fn forward_and_backward_with_wrap() {
        let t = Text::new("foo bar\nbar foo\n");
        assert_eq!(at(&t, "foo", (0, 0), true), Some((1, 4)));
        assert_eq!(at(&t, "foo", (1, 4), true), Some((0, 0)));
        assert_eq!(at(&t, "bar", (1, 4), false), Some((1, 0)));
        assert_eq!(at(&t, "bar", (0, 0), false), Some((1, 0)));
        assert_eq!(at(&t, "zzz", (0, 0), true), None);
    }

    #[test]
    fn overlapping_matches() {
        let t = Text::new("aaaa\n");
        // 'cpoptions' has `c`: the next match is looked for after the end of the last one.
        assert_eq!(at(&t, "aa", (0, 0), true), Some((0, 2)));
        assert_eq!(at(&t, "aa", (0, 2), false), Some((0, 0)));
    }

    #[test]
    fn splitting_and_offsets() {
        assert_eq!(split_pattern("a/b", '/'), ("a".into(), Some("b")));
        assert_eq!(split_pattern("[/]x/e", '/'), ("[/]x".into(), Some("e")));
        assert_eq!(split_pattern("a\\/b", '/'), ("a\\/b".into(), None));
        assert_eq!(split_pattern("a\\?b", '?'), ("a?b".into(), None));
        let (o, _) = parse_offset("e+2");
        assert_eq!(
            o,
            SearchOffset {
                line: false,
                end: true,
                off: 2
            }
        );
        let (o, _) = parse_offset("-");
        assert_eq!(
            o,
            SearchOffset {
                line: true,
                end: false,
                off: -1
            }
        );
        let (o, _) = parse_offset("3");
        assert_eq!(
            o,
            SearchOffset {
                line: true,
                end: false,
                off: 3
            }
        );
        let (o, _) = parse_offset("b-1");
        assert_eq!(
            o,
            SearchOffset {
                line: false,
                end: false,
                off: -1
            }
        );
    }
}
