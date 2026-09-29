//! A window onto a buffer: cursor, viewport, and Vim's scrolling rules.
//!
//! The rules follow Neovim 0.12 (`update_topline`, `scroll_cursor_halfway`, `scroll_cursor_bot`
//! and `pagescroll` in `move.c`), measured with headless nvim. All distances are in screen rows,
//! so wrapped lines count as more than one.

use flux_core::{Text, layout_line};

use crate::BufferId;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cursor {
    pub line: usize,
    /// Char index within the line.
    pub col: usize,
}

#[derive(Debug, Clone)]
pub struct Window {
    pub buffer: BufferId,
    pub cursor: Cursor,
    /// The virtual column vertical moves aim for (Vim's `curswant`). It survives passing
    /// through shorter lines.
    pub curswant: usize,
    /// First buffer line shown.
    pub top: usize,
    /// Text area size in cells.
    pub width: usize,
    pub height: usize,
    /// Where the cursor sits in the window, as a fraction of its height, so a resize can keep it
    /// there (Vim's `w_fraction` and `w_prev_fraction_row`).
    fraction: usize,
    fraction_row: Option<usize>,
}

/// Vim's fixed-point scale for `w_fraction`.
const FRACTION_MULT: usize = 16384;

/// What the viewport logic needs to know about the buffer shown in a window.
pub struct Metrics<'a> {
    pub text: &'a Text,
    pub tabstop: usize,
    pub width: usize,
    pub height: usize,
}

impl Metrics<'_> {
    /// Screen rows line `line` takes up.
    pub fn rows(&self, line: usize) -> usize {
        layout_line(&self.text.line_str(line), self.tabstop, Some(self.width)).row_count()
    }

    fn last_line(&self) -> usize {
        self.text.line_count() - 1
    }

    /// The character on `line` covering virtual column `vcol`, or the last character when the
    /// line is shorter (Vim's `coladvance` in Normal mode).
    pub fn col_for_vcol(&self, line: usize, vcol: usize) -> usize {
        let layout = layout_line(&self.text.line_str(line), self.tabstop, None);
        let mut start = 0;
        let mut last = 0;
        for glyph in &layout.rows[0] {
            let end = start + usize::from(glyph.width);
            if vcol < end {
                return glyph.char_idx;
            }
            last = glyph.char_idx;
            start = end;
        }
        last
    }

    /// The virtual column where character `col` of `line` starts.
    pub fn vcol_of(&self, line: usize, col: usize) -> usize {
        let layout = layout_line(&self.text.line_str(line), self.tabstop, None);
        layout.rows[0]
            .iter()
            .take_while(|g| g.char_idx < col)
            .map(|g| usize::from(g.width))
            .sum()
    }

    /// Rows taken by `lines`, stopping early once the sum passes `cap`.
    fn rows_between(&self, lines: impl Iterator<Item = usize>, cap: usize) -> usize {
        let mut total = 0;
        for line in lines {
            total += self.rows(line);
            if total > cap {
                break;
            }
        }
        total
    }
}

impl Window {
    pub fn new(buffer: BufferId, width: usize, height: usize) -> Self {
        Self {
            buffer,
            cursor: Cursor::default(),
            curswant: 0,
            top: 0,
            width,
            height,
            fraction: 0,
            fraction_row: None,
        }
    }

    /// Screen row of the cursor within the window.
    fn cursor_row(&self, m: &Metrics) -> usize {
        let above: usize = (self.top..self.cursor.line).map(|l| m.rows(l)).sum();
        let layout = layout_line(&m.text.line_str(self.cursor.line), m.tabstop, Some(m.width));
        above + layout.cursor_position(self.cursor.col).0
    }

    /// Change the window's height, keeping the cursor at the same relative height, the way Vim
    /// does (`win_new_height` and `scroll_to_fraction`). `m` describes the new height.
    pub fn set_height(&mut self, m: &Metrics) {
        let height = m.height;
        if height == self.height || height == 0 {
            self.height = height.max(1);
            return;
        }
        let row = self.cursor_row(m);
        if self.fraction_row != Some(row) && self.height > 1 {
            self.fraction = (row * FRACTION_MULT + FRACTION_MULT / 2) / self.height;
        }
        self.height = height;
        if height < m.text.line_count() || self.top > 0 {
            self.top = self.top_for_fraction(m);
        }
        self.fraction_row = Some(self.cursor_row(m));
    }

    /// Change the window's width. The cursor line may now wrap differently, so scroll it into
    /// view again.
    pub fn set_width(&mut self, m: &Metrics) {
        if m.width != self.width {
            self.width = m.width;
            self.scroll_to_cursor(m);
        }
    }

    /// The top line that puts the cursor at `fraction` of the window height.
    fn top_for_fraction(&self, m: &Metrics) -> usize {
        let height = self.height as isize;
        let cursor_line = self.cursor.line;
        let layout = layout_line(&m.text.line_str(cursor_line), m.tabstop, Some(m.width));
        let mut line_size = layout.cursor_position(self.cursor.col).0 as isize;
        let wrow = ((self.fraction * self.height).saturating_sub(1) / FRACTION_MULT) as isize;
        let mut sline = wrow - line_size;
        if sline >= 0 {
            // Make sure the whole cursor line is visible, if possible.
            let rows = layout.row_count() as isize;
            if sline > height - rows {
                sline = height - rows;
            }
        }
        if sline < 0 {
            return cursor_line;
        }
        let mut line = cursor_line;
        while sline > 0 && line > 0 {
            line -= 1;
            line_size = m.rows(line) as isize;
            sline -= line_size;
        }
        if sline < 0 {
            // That line would go off the top; start at the next one.
            line += 1;
        }
        line
    }

    /// Vim's 'scroll' default: half the window height.
    pub fn scroll_amount(&self) -> usize {
        (self.height / 2).max(1)
    }

    /// Last line shown completely.
    pub fn bottom(&self, m: &Metrics) -> usize {
        let mut used = 0;
        let mut line = self.top;
        loop {
            used += m.rows(line);
            if used > self.height {
                return line.saturating_sub(1).max(self.top);
            }
            if line == m.last_line() {
                return line;
            }
            line += 1;
        }
    }

    /// Move the cursor to `line`, in the column nearest `curswant`, and scroll it into view.
    pub fn set_cursor_line(&mut self, line: usize, m: &Metrics) {
        self.place_on_line(line.min(m.last_line()), m);
        self.scroll_to_cursor(m);
    }

    /// Put the cursor at an exact position, making its column the new `curswant`.
    pub fn set_cursor(&mut self, line: usize, col: usize, m: &Metrics) {
        let line = line.min(m.last_line());
        self.cursor = Cursor { line, col };
        self.curswant = m.vcol_of(line, col);
        self.scroll_to_cursor(m);
    }

    fn place_on_line(&mut self, line: usize, m: &Metrics) {
        self.cursor.line = line;
        self.cursor.col = m.col_for_vcol(line, self.curswant);
    }

    /// Scroll so the cursor line is visible, the way Vim does after a cursor move: a short
    /// distance scrolls just enough, a long one puts the cursor in the middle.
    pub fn scroll_to_cursor(&mut self, m: &Metrics) {
        let cur = self.cursor.line;
        let h = self.height;
        if cur < self.top {
            let distance = m.rows_between(cur..self.top, h);
            let half = (h / 2).saturating_sub(1).max(2);
            self.top = if distance >= half {
                self.halfway(cur, false, m)
            } else {
                cur
            };
        } else if cur > self.bottom(m) {
            let bottom = self.bottom(m);
            let distance = m.rows_between(bottom + 1..=cur, h + 1);
            self.top = if distance > h + 1 {
                self.halfway(cur, false, m)
            } else if distance >= (h + 3) / 2 {
                self.halfway(cur, true, m)
            } else {
                self.top.max(self.top_with_bottom(cur, m))
            };
        }
    }

    /// Smallest top line that shows `line` completely.
    fn top_with_bottom(&self, line: usize, m: &Metrics) -> usize {
        let mut used = m.rows(line);
        let mut top = line;
        while top > 0 {
            used += m.rows(top - 1);
            if used > self.height {
                break;
            }
            top -= 1;
        }
        top
    }

    /// Top line that puts `cur` in the middle of the window. With an odd number of spare rows,
    /// `prefer_above` puts the extra one above the cursor.
    fn halfway(&self, cur: usize, prefer_above: bool, m: &Metrics) -> usize {
        let h = self.height;
        let mut used = m.rows(cur);
        let (mut above, mut below) = (0, 0);
        let (mut top, mut bottom) = (cur, cur);
        while top > 0 {
            let add_below = if prefer_above {
                below < above
            } else {
                below <= above
            };
            if add_below {
                if bottom < m.last_line() {
                    bottom += 1;
                    let rows = m.rows(bottom);
                    used += rows;
                    if used > h {
                        break;
                    }
                    below += rows;
                } else {
                    // Past the end: count a `~` row without using up space.
                    below += 1;
                }
            }
            let add_above = if prefer_above {
                below >= above
            } else {
                below > above
            };
            if add_above {
                let rows = m.rows(top - 1);
                used += rows;
                if used > h {
                    break;
                }
                above += rows;
                top -= 1;
            }
        }
        top
    }

    /// Keep the cursor inside the window after the view moved.
    fn clamp_cursor_to_view(&mut self, m: &Metrics) {
        let line = self.cursor.line.clamp(self.top, self.bottom(m));
        if line != self.cursor.line {
            self.place_on_line(line, m);
        }
    }

    /// Move the cursor `dist` screen rows down (or up), the way `gj`/`gk` do: through the rows
    /// of wrapped lines, keeping the position within the row. Returns false if it hit the end of
    /// the buffer first. This is Neovim's `nv_screengo`, which works on `curswant` alone.
    fn move_screen_rows(&mut self, down: bool, mut dist: usize, m: &Metrics) -> bool {
        let width = m.width.max(1);
        let span = |line: usize| m.rows(line) * width;
        let mut line = self.cursor.line;
        let mut want = self.curswant.min(span(line) - 1);
        let mut ok = true;
        while dist > 0 {
            dist -= 1;
            if down {
                if want + width < span(line) {
                    want += width;
                } else if line < m.last_line() {
                    line += 1;
                    want %= width;
                } else {
                    ok = false;
                    break;
                }
            } else if want >= width {
                want -= width;
            } else if line > 0 {
                line -= 1;
                want += span(line) - width;
            } else {
                ok = false;
                break;
            }
        }
        self.curswant = want;
        self.place_on_line(line, m);
        ok
    }

    /// `CTRL-E`: scroll the text up `n` lines. The last line may scroll up to the top.
    pub fn scroll_lines_down(&mut self, n: usize, m: &Metrics) -> bool {
        if self.top == m.last_line() {
            return false;
        }
        self.top = (self.top + n).min(m.last_line());
        self.clamp_cursor_to_view(m);
        true
    }

    /// `CTRL-Y`: scroll the text down `n` lines.
    pub fn scroll_lines_up(&mut self, n: usize, m: &Metrics) -> bool {
        if self.top == 0 {
            return false;
        }
        self.top = self.top.saturating_sub(n);
        self.clamp_cursor_to_view(m);
        true
    }

    /// `CTRL-D`: scroll down 'scroll' screen rows and move the cursor down as many rows.
    ///
    /// Neovim scrolls with 'smoothscroll' on and, when that leaves the top line partly scrolled
    /// off, backs up to the line's start and moves the cursor one row less. The view stops once
    /// the last line reaches the bottom; the cursor keeps going.
    pub fn scroll_half_down(&mut self, m: &Metrics) -> bool {
        let count = self.scroll_amount();
        let mut cursor_rows = count;
        let rows_to_end = m.rows_between(self.top..=m.last_line(), self.height + count);
        let scroll = if rows_to_end < self.height + count {
            rows_to_end.saturating_sub(self.height)
        } else {
            count
        };
        if scroll > 0 {
            let (mut line, mut sub) = (self.top, 0);
            for _ in 0..scroll {
                sub += 1;
                if sub == m.rows(line) {
                    line += 1;
                    sub = 0;
                }
            }
            if sub > 0 {
                if line != self.top {
                    cursor_rows -= 1;
                } else {
                    cursor_rows += m.rows(line) - sub;
                    line += 1;
                }
            }
            self.top = line.min(m.last_line());
        }
        let moved = self.move_screen_rows(true, cursor_rows, m);
        self.clamp_cursor_to_view(m);
        moved
    }

    /// `CTRL-U`: the mirror image of `CTRL-D`. Scrolling back into a wrapped line that ends up
    /// partly shown moves forward to the next line start instead.
    pub fn scroll_half_up(&mut self, m: &Metrics) -> bool {
        let count = self.scroll_amount();
        let mut cursor_rows = count;
        if self.top > 0 {
            let (mut line, mut sub) = (self.top, 0);
            for _ in 0..count {
                if sub > 0 {
                    sub -= 1;
                } else if line > 0 {
                    line -= 1;
                    sub = m.rows(line) - 1;
                } else {
                    break;
                }
            }
            if sub > 0 {
                if self.top - line > 1 {
                    cursor_rows -= m.rows(line) - sub;
                    line += 1;
                } else {
                    cursor_rows += sub;
                }
            }
            self.top = line;
        }
        let moved = self.move_screen_rows(false, cursor_rows, m);
        self.clamp_cursor_to_view(m);
        moved
    }

    /// `CTRL-F`: scroll forward a page, keeping two lines of overlap, and put the cursor on the top
    /// line. When the last line is already visible, it scrolls to the top of the window.
    pub fn page_down(&mut self, m: &Metrics) -> bool {
        let last = m.last_line();
        if self.top == last {
            return false;
        }
        if self.bottom(m) >= last {
            self.top = last;
        } else {
            self.top = self.advance(self.top, self.height.saturating_sub(2).max(1), m);
        }
        self.place_on_line(self.top, m);
        true
    }

    /// `CTRL-B`: scroll back a page, keeping two lines of overlap, and put the cursor on the bottom
    /// line.
    pub fn page_up(&mut self, m: &Metrics) -> bool {
        if self.top == 0 {
            return false;
        }
        // Measured from nvim: with the last line at the top, CTRL-B goes back a full window.
        let budget = if self.top == m.last_line() {
            self.height
        } else {
            self.height.saturating_sub(2).max(1)
        };
        let mut used = 0;
        while self.top > 0 {
            let rows = m.rows(self.top - 1);
            if used + rows > budget && used > 0 {
                break;
            }
            used += rows;
            self.top -= 1;
        }
        self.place_on_line(self.bottom(m), m);
        true
    }

    /// The line `budget` rows below `line`, moving at least one line.
    fn advance(&self, mut line: usize, budget: usize, m: &Metrics) -> usize {
        let mut used = 0;
        while line < m.last_line() {
            let rows = m.rows(line);
            if used + rows > budget && used > 0 {
                break;
            }
            used += rows;
            line += 1;
        }
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: usize = 22;

    fn numbered(n: usize) -> Text {
        let s: String = (1..=n).map(|i| format!("{i}\n")).collect();
        Text::new(&s)
    }

    /// Lines where every third one is 201 cells wide, so three rows at width 80.
    fn wrapped(n: usize) -> Text {
        let s: String = (1..=n)
            .map(|i| {
                if i % 3 == 0 {
                    format!("{i}{}\n", "x".repeat(200))
                } else {
                    format!("{i}\n")
                }
            })
            .collect();
        Text::new(&s)
    }

    fn metrics(text: &Text) -> Metrics<'_> {
        Metrics {
            text,
            tabstop: 8,
            width: 80,
            height: H,
        }
    }

    /// Window at 1-based `top`/`cur`, as in the nvim probes.
    fn win(top: usize, cur: usize) -> Window {
        let mut w = Window::new(BufferId(0), 80, H);
        w.top = top - 1;
        w.cursor.line = cur - 1;
        w
    }

    fn pos(w: &Window) -> (usize, usize) {
        (w.top + 1, w.cursor.line + 1)
    }

    #[test]
    fn jump_down_scrolls_minimally_then_centers() {
        let text = numbered(200);
        let m = metrics(&text);
        // Window at 50 shows 50..=71. Rows below the bottom: 1..=11 minimal, 12..=23 centered
        // with the extra row above, 24+ centered with it below.
        for (d, row) in [(1, 21), (11, 21), (12, 11), (23, 11), (24, 10), (60, 10)] {
            let mut w = win(50, 50);
            let target = 71 + d;
            w.set_cursor_line(target - 1, &m);
            assert_eq!(target - (w.top + 1), row, "jump {d} rows below");
        }
    }

    #[test]
    fn jump_up_scrolls_minimally_then_centers() {
        let text = numbered(200);
        let m = metrics(&text);
        for (d, row) in [(1, 0), (9, 0), (10, 10), (40, 10)] {
            let mut w = win(100, 100);
            w.set_cursor_line(100 - d - 1, &m);
            assert_eq!(100 - d - (w.top + 1), row, "jump {d} rows above");
        }
    }

    #[test]
    fn jumps_near_the_ends_do_not_overscroll() {
        let text = numbered(100);
        let m = metrics(&text);
        let mut w = win(1, 1);
        w.set_cursor_line(99, &m);
        assert_eq!(pos(&w), (79, 100));
        w.set_cursor_line(0, &m);
        assert_eq!(pos(&w), (1, 1));
        let mut w = win(1, 1);
        w.set_cursor_line(98, &m);
        assert_eq!(pos(&w), (79, 99));
    }

    #[test]
    fn ctrl_d_and_ctrl_u() {
        let text = numbered(200);
        let m = metrics(&text);
        let cases_d = [
            ((1, 1), (12, 12)),
            ((1, 5), (12, 16)),
            ((170, 175), (179, 186)),
            ((179, 179), (179, 190)),
            ((190, 195), (190, 200)),
            ((178, 200), (179, 200)),
        ];
        for (from, to) in cases_d {
            let mut w = win(from.0, from.1);
            w.scroll_half_down(&m);
            assert_eq!(pos(&w), to, "CTRL-D from {from:?}");
        }
        let cases_u = [
            ((1, 5), (1, 1)),
            ((3, 10), (1, 1)),
            ((10, 12), (1, 1)),
            ((10, 31), (1, 20)),
            ((190, 200), (179, 189)),
            ((200, 200), (189, 189)),
        ];
        for (from, to) in cases_u {
            let mut w = win(from.0, from.1);
            w.scroll_half_up(&m);
            assert_eq!(pos(&w), to, "CTRL-U from {from:?}");
        }
    }

    #[test]
    fn ctrl_f_and_ctrl_b() {
        let text = numbered(200);
        let m = metrics(&text);
        for (top, to) in [(1, 21), (170, 190), (179, 200), (190, 200), (199, 200)] {
            let mut w = win(top, top);
            assert!(w.page_down(&m));
            assert_eq!(pos(&w), (to, to), "CTRL-F from {top}");
        }
        assert!(!win(200, 200).page_down(&m));
        let mut w = win(30, 40);
        w.page_down(&m);
        assert_eq!(pos(&w), (50, 50));
        for ((top, cur), to) in [
            ((200, 200), (178, 199)),
            ((195, 195), (175, 196)),
            ((179, 179), (159, 180)),
            ((100, 121), (80, 101)),
            ((2, 3), (1, 22)),
        ] {
            let mut w = win(top, cur);
            assert!(w.page_up(&m));
            assert_eq!(pos(&w), to, "CTRL-B from {top},{cur}");
        }
    }

    #[test]
    fn ctrl_e_and_ctrl_y() {
        let text = numbered(100);
        let m = metrics(&text);
        let mut w = win(1, 1);
        w.scroll_lines_down(1, &m);
        assert_eq!(pos(&w), (2, 2));
        let mut w = win(3, 13);
        w.scroll_lines_up(1, &m);
        assert_eq!(pos(&w), (2, 13));
        let mut w = win(79, 100);
        w.scroll_lines_down(1, &m);
        assert_eq!(pos(&w), (80, 100));
    }

    #[test]
    fn resize_keeps_the_cursor_at_the_same_relative_height() {
        // Measured with nvim in tmux: cursor on the second row of a 22-row window, shrink to 13
        // rows and the cursor line becomes the top; grow back and it returns to the second row.
        let text = numbered(200);
        let mut w = win(19, 20);
        let resized = |height| Metrics {
            text: &text,
            tabstop: 8,
            width: 80,
            height,
        };
        w.set_height(&resized(13));
        assert_eq!(pos(&w), (20, 20));
        w.set_height(&resized(22));
        assert_eq!(pos(&w), (19, 20));
    }

    #[test]
    fn short_file() {
        let text = numbered(10);
        let m = metrics(&text);
        let mut w = win(1, 1);
        w.scroll_half_down(&m);
        assert_eq!(pos(&w), (1, 10));
        w.page_down(&m);
        assert_eq!(pos(&w), (10, 10));
        w.page_up(&m);
        assert_eq!(pos(&w), (1, 10));
    }

    #[test]
    fn wrapped_lines_count_as_rows() {
        let text = wrapped(40);
        let m = metrics(&text);
        let mut w = win(1, 1);
        w.set_cursor_line(20, &m);
        assert_eq!(pos(&w), (15, 21));
        w.set_cursor_line(22, &m);
        assert_eq!(pos(&w), (15, 23));
        w.scroll_half_down(&m);
        assert_eq!(pos(&w), (21, 29));
        w.set_cursor_line(39, &m);
        assert_eq!(pos(&w), (28, 40));
        w.set_cursor_line(0, &m);
        w.page_down(&m);
        assert_eq!(pos(&w), (13, 13));
    }
}
