//! Motions: where a key moves the cursor, and how an operator treats the text it covers.
//!
//! The word and paragraph motions follow Vim's `search.c` (`fwd_word`, `end_word`, `bck_word`,
//! `bckend_word`, `findpar`) step for step, including the special cases for operators.

use flux_core::{Text, chars};
use flux_view::Editor;

use crate::util::{self, Pos, before, pos};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    /// `h`, `<Left>`.
    Left,
    /// `l`, `<Right>`.
    Right,
    /// `<BS>`: like `h` but wraps to the previous line ('whichwrap' has `b`).
    BackspaceLeft,
    /// `<Space>`: like `l` but wraps to the next line ('whichwrap' has `s`).
    SpaceRight,
    Down,
    Up,
    /// `0`, `<Home>`.
    LineStart,
    /// `^`.
    FirstNonBlank,
    /// `$`, `<End>`.
    LineEnd,
    /// `|`.
    Column,
    /// `+`, `<CR>`.
    NextLineStart,
    /// `-`.
    PrevLineStart,
    /// `_`.
    CurrentLineStart,
    /// `G`.
    GotoLine,
    /// `gg`.
    GotoFirstLine,
    /// `%`.
    Percent,
    /// `w` / `W`.
    WordForward(bool),
    /// `b` / `B`.
    WordBackward(bool),
    /// `e` / `E`.
    WordEnd(bool),
    /// `ge` / `gE`.
    WordEndBackward(bool),
    /// `f`, `F`, `t`, `T`.
    Find(Find),
    /// `;` (`reverse` false) and `,`.
    RepeatFind {
        reverse: bool,
    },
    /// `}`.
    ParagraphForward,
    /// `{`.
    ParagraphBackward,
    /// `H`.
    WindowTop,
    /// `M`.
    WindowMiddle,
    /// `L`.
    WindowBottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Find {
    pub forward: bool,
    /// `t`/`T`: stop just before the character.
    pub till: bool,
    pub ch: char,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Exclusive,
    Inclusive,
    Linewise,
}

/// What a motion does to the column vertical moves aim for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Want {
    /// Keep it (`j`, `G`, …).
    Keep,
    /// Aim for wherever the cursor lands.
    Column,
    /// The end of every line (`$`).
    End,
    /// A specific screen column (`|`).
    Exact(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub pos: Pos,
    pub kind: Kind,
    pub want: Want,
    /// Deletes with this motion always go to `"1` (Vim does this for `%`, `{`, `}`, …).
    pub numbered_register: bool,
}

/// Which operator, if any, is waiting for the motion. A few motions behave differently then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    None,
    Change,
    Other,
}

pub struct Context<'a> {
    pub editor: &'a Editor,
    pub count: Option<usize>,
    pub pending: Pending,
    pub last_find: Option<Find>,
}

/// Where `motion` goes from the cursor, or `None` when it fails (Vim beeps and cancels any
/// operator).
pub fn eval(motion: Motion, cx: &Context) -> Option<Target> {
    let editor = cx.editor;
    let text = editor.text();
    let cur = editor.cursor();
    let n = cx.count.unwrap_or(1).max(1);
    let op = cx.pending != Pending::None;
    let line = util::line(editor, cur.line);
    let len = line.chars().count();
    let target = |p: Pos, kind: Kind, want: Want| Target {
        pos: p,
        kind,
        want,
        numbered_register: false,
    };

    match motion {
        Motion::Left => {
            if cur.col == 0 {
                return None;
            }
            let mut col = cur.col;
            for _ in 0..n {
                col = chars::prev_grapheme(&line, col);
            }
            Some(target(pos(cur.line, col), Kind::Exclusive, Want::Column))
        }
        Motion::Right => {
            // With an operator the motion may reach the end of the line (`dl` on the last char).
            let limit = if op { len } else { chars::last_grapheme(&line) };
            if len == 0 || cur.col >= limit {
                return None;
            }
            let mut col = cur.col;
            for _ in 0..n {
                col = chars::next_grapheme(&line, col).min(limit);
            }
            Some(target(pos(cur.line, col), Kind::Exclusive, Want::Column))
        }
        Motion::BackspaceLeft => {
            let mut p = cur;
            let mut moved = false;
            for _ in 0..n {
                if p.col > 0 {
                    p.col = chars::prev_grapheme(&util::line(editor, p.line), p.col);
                } else if p.line > 0 {
                    p.line -= 1;
                    let s = util::line(editor, p.line);
                    p.col = if op {
                        s.chars().count()
                    } else {
                        chars::last_grapheme(&s)
                    };
                } else {
                    break;
                }
                moved = true;
            }
            moved.then(|| target(p, Kind::Exclusive, Want::Column))
        }
        Motion::SpaceRight => {
            let mut p = cur;
            let mut moved = false;
            for _ in 0..n {
                let s = util::line(editor, p.line);
                let next = chars::next_grapheme(&s, p.col);
                let limit = if op {
                    s.chars().count()
                } else {
                    chars::last_grapheme(&s)
                };
                if next <= limit && !s.is_empty() && p.col < limit {
                    p.col = next;
                } else if p.line < text.last_line() {
                    p = pos(p.line + 1, 0);
                } else {
                    break;
                }
                moved = true;
            }
            moved.then(|| target(p, Kind::Exclusive, Want::Column))
        }
        Motion::Down => {
            if cur.line == text.last_line() {
                return None;
            }
            let l = (cur.line + n).min(text.last_line());
            Some(target(on_line(editor, l), Kind::Linewise, Want::Keep))
        }
        Motion::Up => {
            if cur.line == 0 {
                return None;
            }
            let l = cur.line.saturating_sub(n);
            Some(target(on_line(editor, l), Kind::Linewise, Want::Keep))
        }
        Motion::LineStart => Some(target(pos(cur.line, 0), Kind::Exclusive, Want::Column)),
        Motion::FirstNonBlank => Some(target(
            pos(cur.line, util::first_non_blank(&line)),
            Kind::Exclusive,
            Want::Column,
        )),
        Motion::LineEnd => {
            let l = cur.line + n - 1;
            if l > text.last_line() {
                return None;
            }
            let s = util::line(editor, l);
            Some(target(
                pos(l, chars::last_grapheme(&s)),
                Kind::Inclusive,
                Want::End,
            ))
        }
        Motion::Column => {
            let vcol = n - 1;
            let col = editor.metrics().col_for_vcol(cur.line, vcol);
            Some(target(
                pos(cur.line, col),
                Kind::Exclusive,
                Want::Exact(vcol),
            ))
        }
        Motion::NextLineStart | Motion::PrevLineStart | Motion::CurrentLineStart => {
            let l = match motion {
                Motion::NextLineStart if cur.line + n <= text.last_line() => cur.line + n,
                Motion::PrevLineStart if cur.line >= n => cur.line - n,
                Motion::CurrentLineStart if cur.line + n - 1 <= text.last_line() => {
                    cur.line + n - 1
                }
                _ => return None,
            };
            let col = util::first_non_blank(&util::line(editor, l));
            Some(target(pos(l, col), Kind::Linewise, Want::Column))
        }
        Motion::GotoLine | Motion::GotoFirstLine => {
            let l = match (cx.count, motion) {
                (Some(c), _) => c.max(1) - 1,
                (None, Motion::GotoLine) => text.last_line(),
                (None, _) => 0,
            }
            .min(text.last_line());
            Some(target(on_line(editor, l), Kind::Linewise, Want::Keep))
        }
        Motion::Percent => {
            if let Some(count) = cx.count {
                if count > 100 {
                    return None;
                }
                let l = ((count * text.line_count()).div_ceil(100)).max(1) - 1;
                return Some(target(on_line(editor, l), Kind::Linewise, Want::Keep));
            }
            let p = match_pair(text, cur)?;
            Some(Target {
                numbered_register: true,
                ..target(p, Kind::Inclusive, Want::Column)
            })
        }
        Motion::WordForward(big) | Motion::WordEnd(big) => {
            let word_end = matches!(motion, Motion::WordEnd(_));
            let (p, inclusive) = word_command(text, cur, n, big, word_end, cx.pending)?;
            let kind = if inclusive {
                Kind::Inclusive
            } else {
                Kind::Exclusive
            };
            Some(target(p, kind, Want::Column))
        }
        Motion::WordBackward(big) => {
            let mut sc = Scanner::new(text, cur, big);
            if !sc.bck_word(n, false) {
                return None;
            }
            Some(target(sc.pos(), Kind::Exclusive, Want::Column))
        }
        Motion::WordEndBackward(big) => {
            let mut sc = Scanner::new(text, cur, big);
            if !sc.bckend_word(n, false) {
                return None;
            }
            Some(target(sc.pos(), Kind::Inclusive, Want::Column))
        }
        Motion::Find(find) => find_char(&line, cur, n, find, false),
        Motion::RepeatFind { reverse } => {
            let mut find = cx.last_find?;
            if reverse {
                find.forward = !find.forward;
            }
            find_char(&line, cur, n, find, true)
        }
        Motion::ParagraphForward | Motion::ParagraphBackward => {
            let forward = motion == Motion::ParagraphForward;
            let (p, inclusive) = find_paragraph(text, cur.line, forward, n)?;
            let kind = if inclusive {
                Kind::Inclusive
            } else {
                Kind::Exclusive
            };
            Some(Target {
                numbered_register: true,
                ..target(p, kind, Want::Column)
            })
        }
        Motion::WindowTop | Motion::WindowMiddle | Motion::WindowBottom => {
            let m = editor.metrics();
            let win = &editor.window;
            let bottom = win.bottom(&m);
            let l = match motion {
                Motion::WindowTop => (win.top + n - 1).min(bottom),
                Motion::WindowBottom => bottom.saturating_sub(n - 1).max(win.top),
                _ => window_middle(editor),
            };
            Some(target(on_line(editor, l), Kind::Linewise, Want::Keep))
        }
    }
}

/// The cursor on line `l` in the column nearest the window's `curswant`.
fn on_line(editor: &Editor, l: usize) -> Pos {
    pos(l, editor.metrics().col_for_vcol(l, editor.window.curswant))
}

/// `M`: the middle of the lines shown, not counting `~` rows below the end.
fn window_middle(editor: &Editor) -> usize {
    let m = editor.metrics();
    let win = &editor.window;
    let text = editor.text();
    let mut shown = 0;
    let mut line = win.top;
    while line <= text.last_line() && shown < win.height {
        shown += m.rows(line);
        line += 1;
    }
    let half = shown.min(win.height).div_ceil(2);
    let mut used = 0;
    let mut n = 0;
    while win.top + n < text.last_line() {
        used += m.rows(win.top + n);
        if used >= half {
            break;
        }
        n += 1;
    }
    win.top + n
}

/// `f`/`t`/`F`/`T` within the current line. When repeating a `t`/`T` (`;`, `,`), a match right
/// next to the cursor is skipped so the repeat makes progress (Vim without `;` in 'cpoptions').
fn find_char(line: &str, cur: Pos, count: usize, find: Find, repeat: bool) -> Option<Target> {
    let chars: Vec<char> = line.chars().collect();
    let mut col = cur.col;
    let skip_adjacent = repeat && find.till && count == 1;
    for i in 0..count {
        loop {
            if find.forward {
                col += 1;
                if col >= chars.len() {
                    return None;
                }
            } else {
                col = col.checked_sub(1)?;
            }
            if chars[col] == find.ch {
                let adjacent = if find.forward {
                    col == cur.col + 1
                } else {
                    col + 1 == cur.col
                };
                if i == 0 && skip_adjacent && adjacent {
                    continue;
                }
                break;
            }
        }
    }
    if find.till {
        col = if find.forward { col - 1 } else { col + 1 };
    }
    let kind = if find.forward {
        Kind::Inclusive
    } else {
        Kind::Exclusive
    };
    Some(Target {
        pos: pos(cur.line, col),
        kind,
        want: Want::Column,
        numbered_register: false,
    })
}

/// `%` without a count: the bracket matching the first one at or after the cursor on this line.
fn match_pair(text: &Text, cur: Pos) -> Option<Pos> {
    const PAIRS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];
    let line: Vec<char> = text.line_str(cur.line).chars().collect();
    let (col, c) = line
        .iter()
        .enumerate()
        .skip(cur.col)
        .find(|(_, c)| PAIRS.iter().any(|&(o, cl)| **c == o || **c == cl))
        .map(|(i, &c)| (i, c))?;
    let (open, close, forward) = PAIRS.iter().find_map(|&(o, cl)| {
        if c == o {
            Some((o, cl, true))
        } else if c == cl {
            Some((o, cl, false))
        } else {
            None
        }
    })?;
    let start = text.pos_to_char(cur.line, col);
    let rope = text.rope();
    let mut depth = 0usize;
    if forward {
        for (i, ch) in rope.chars_at(start).enumerate() {
            if ch == open {
                depth += 1;
            } else if ch == close {
                depth -= 1;
                if depth == 0 {
                    let (l, c) = text.char_to_pos(start + i);
                    return Some(pos(l, c));
                }
            }
        }
    } else {
        let mut idx = start + 1;
        let mut iter = rope.chars_at(idx);
        while let Some(ch) = iter.prev() {
            idx -= 1;
            if ch == close {
                depth += 1;
            } else if ch == open {
                depth -= 1;
                if depth == 0 {
                    let (l, c) = text.char_to_pos(idx);
                    return Some(pos(l, c));
                }
            }
        }
    }
    None
}

/// Vim's `findpar`: the next (or previous) empty line after a non-empty one. Running into the
/// end of the buffer lands on the last character and makes the motion inclusive.
fn find_paragraph(text: &Text, start: usize, forward: bool, count: usize) -> Option<(Pos, bool)> {
    let last = text.last_line();
    let mut curr = start;
    for remaining in (0..count).rev() {
        let mut did_skip = false;
        let mut first = true;
        loop {
            let empty = text.line_len(curr) == 0;
            if !empty {
                did_skip = true;
            }
            if !first && did_skip && empty {
                break;
            }
            let next = if forward {
                curr.checked_add(1).filter(|&l| l <= last)
            } else {
                curr.checked_sub(1)
            };
            match next {
                Some(l) => curr = l,
                None if remaining > 0 => return None,
                None => break,
            }
            first = false;
        }
    }
    if curr == last && forward {
        let s = text.line_str(curr);
        if !s.is_empty() {
            return Some((pos(curr, chars::last_grapheme(&s)), true));
        }
    }
    Some((pos(curr, 0), false))
}

/// `w`/`W`/`e`/`E` with Vim's special cases: `cw` on a word acts like `ce`, and with an operator
/// `w` stops at the end of the line instead of crossing to the next word.
fn word_command(
    text: &Text,
    start: Pos,
    count: usize,
    big: bool,
    mut word_end: bool,
    pending: Pending,
) -> Option<(Pos, bool)> {
    let mut inclusive = word_end;
    let mut stop = false;
    if !word_end && pending == Pending::Change {
        let s = text.line_str(start.line);
        if let Some(c) = s.chars().nth(start.col)
            && !util::is_white(c)
        {
            inclusive = true;
            word_end = true;
            stop = true;
        }
    }
    let mut sc = Scanner::new(text, start, big);
    let ok = if word_end {
        sc.end_word(count, stop, false)
    } else {
        sc.fwd_word(count, pending != Pending::None)
    };
    // Don't leave the cursor on the end of the line, unless it didn't move forward.
    if before(start, sc.pos()) && sc.col > 0 && sc.col == sc.len() {
        sc.col = chars::prev_grapheme(&sc.line_text, sc.col);
        inclusive = true;
    }
    if !ok && pending == Pending::None {
        return None;
    }
    Some((sc.pos(), inclusive))
}

/// A cursor walking the text one grapheme at a time, including the position just past the end
/// of each line (Vim's NUL position), which counts as a blank.
struct Scanner<'a> {
    text: &'a Text,
    big: bool,
    line: usize,
    col: usize,
    line_text: String,
    chars: Vec<char>,
}

impl<'a> Scanner<'a> {
    fn new(text: &'a Text, p: Pos, big: bool) -> Self {
        let mut sc = Self {
            text,
            big,
            line: p.line,
            col: 0,
            line_text: String::new(),
            chars: Vec::new(),
        };
        sc.load(p.line);
        sc.col = p.col.min(sc.len());
        sc
    }

    fn load(&mut self, line: usize) {
        self.line = line;
        self.line_text = self.text.line_str(line).into_owned();
        self.chars = self.line_text.chars().collect();
    }

    fn pos(&self) -> Pos {
        pos(self.line, self.col)
    }

    fn len(&self) -> usize {
        self.chars.len()
    }

    fn line_empty(&self) -> bool {
        self.chars.is_empty()
    }

    fn cls(&self) -> u32 {
        match self.chars.get(self.col) {
            Some(&c) => chars::class(c, self.big),
            None => 0,
        }
    }

    /// Vim's `inc`: 0 within the line, 2 onto the end of the line, 1 onto the next line, -1 at
    /// the end of the buffer.
    fn inc(&mut self) -> i32 {
        if self.col < self.len() {
            self.col = chars::next_grapheme(&self.line_text, self.col);
            return if self.col < self.len() { 0 } else { 2 };
        }
        if self.line < self.text.last_line() {
            self.load(self.line + 1);
            self.col = 0;
            return 1;
        }
        -1
    }

    /// Vim's `dec`: 0 within the line, 1 onto the end of the previous line, -1 at the start.
    fn dec(&mut self) -> i32 {
        if self.col > 0 {
            self.col = chars::prev_grapheme(&self.line_text, self.col);
            return 0;
        }
        if self.line > 0 {
            self.load(self.line - 1);
            self.col = self.len();
            return 1;
        }
        -1
    }

    /// Skip characters of class `class`. True when it ran into the end of the buffer.
    fn skip_chars(&mut self, class: u32, forward: bool) -> bool {
        while self.cls() == class {
            let r = if forward { self.inc() } else { self.dec() };
            if r == -1 {
                return true;
            }
        }
        false
    }

    fn fwd_word(&mut self, mut count: usize, eol: bool) -> bool {
        while count > 0 {
            count -= 1;
            let sclass = self.cls();
            let last_line = self.line == self.text.last_line();
            let mut i = self.inc();
            if i == -1 || (i >= 1 && last_line) {
                return false;
            }
            if i >= 1 && eol && count == 0 {
                return true;
            }
            if sclass != 0 {
                while self.cls() == sclass {
                    i = self.inc();
                    if i == -1 || (i >= 1 && eol && count == 0) {
                        return true;
                    }
                }
            }
            while self.cls() == 0 {
                if self.col == 0 && self.line_empty() {
                    break;
                }
                i = self.inc();
                if i == -1 || (i >= 1 && eol && count == 0) {
                    return true;
                }
            }
        }
        true
    }

    fn end_word(&mut self, mut count: usize, mut stop: bool, empty: bool) -> bool {
        while count > 0 {
            count -= 1;
            let sclass = self.cls();
            if self.inc() == -1 {
                return false;
            }
            let mut finished = false;
            if self.cls() == sclass && sclass != 0 {
                if self.skip_chars(sclass, true) {
                    return false;
                }
            } else if !stop || sclass == 0 {
                while self.cls() == 0 {
                    if self.col == 0 && self.line_empty() && empty {
                        finished = true;
                        break;
                    }
                    if self.inc() == -1 {
                        return false;
                    }
                }
                if !finished && self.skip_chars(self.cls(), true) {
                    return false;
                }
            }
            if !finished {
                self.dec();
            }
            stop = false;
        }
        true
    }

    fn bck_word(&mut self, mut count: usize, mut stop: bool) -> bool {
        while count > 0 {
            count -= 1;
            let sclass = self.cls();
            if self.dec() == -1 {
                return false;
            }
            let mut finished = false;
            if !stop || sclass == self.cls() || sclass == 0 {
                while self.cls() == 0 {
                    if self.col == 0 && self.line_empty() {
                        finished = true;
                        break;
                    }
                    if self.dec() == -1 {
                        return true;
                    }
                }
                if !finished && self.skip_chars(self.cls(), false) {
                    return true;
                }
            }
            if !finished {
                self.inc();
            }
            stop = false;
        }
        true
    }

    fn bckend_word(&mut self, mut count: usize, eol: bool) -> bool {
        while count > 0 {
            count -= 1;
            let sclass = self.cls();
            let i = self.dec();
            if i == -1 {
                return false;
            }
            if eol && i == 1 {
                return true;
            }
            if sclass != 0 {
                while self.cls() == sclass {
                    let i = self.dec();
                    if i == -1 || (eol && i == 1) {
                        return true;
                    }
                }
            }
            while self.cls() == 0 {
                if self.col == 0 && self.line_empty() {
                    break;
                }
                let i = self.dec();
                if i == -1 || (eol && i == 1) {
                    return true;
                }
            }
        }
        true
    }
}
