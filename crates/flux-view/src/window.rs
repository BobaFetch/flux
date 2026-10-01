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
    pub id: crate::WindowId,
    pub buffer: BufferId,
    /// The alternate buffer (`#`, `CTRL-^`).
    pub alt_buffer: Option<BufferId>,
    pub cursor: Cursor,
    /// The virtual column vertical moves aim for (Vim's `curswant`). It survives passing
    /// through shorter lines.
    pub curswant: usize,
    /// `curswant` is stale: recompute it from the cursor before the next vertical move (Vim's
    /// `w_set_curswant`). Horizontal moves set this rather than computing it straight away.
    pub set_curswant: bool,
    /// First buffer line shown.
    pub top: usize,
    /// Columns of the top line scrolled off the top of the window (Vim's `w_skipcol`), for a
    /// cursor line too tall to fit. Only meaningful while `top` is still `skip_top`.
    skipcol: usize,
    skip_top: usize,
    /// Vim's 'scroll': rows `CTRL-D` and `CTRL-U` scroll. A count sets it until the next resize.
    scroll: usize,
    /// Vim's `VALID_TOPLINE` after a half-page scroll that left the cursor on its line: the
    /// view is kept as is (even with the cursor line partly shown) until the cursor changes
    /// lines, the view moves or the text changes. Holds (cursor line, top, line count, rows of
    /// the cursor line).
    keep_view: Option<(usize, usize, usize, usize)>,
    /// While scrolling a page: the cursor line Vim last validated (`w_valid_cursor`), and
    /// whether validating on another line has invalidated the top line since.
    valid_line: usize,
    topline_dirty: bool,
    /// Text area size in cells.
    pub width: usize,
    pub height: usize,
    /// Where the cursor sits in the window, as a fraction of its height, so a resize can keep it
    /// there (Vim's `w_fraction` and `w_prev_fraction_row`).
    fraction: usize,
    fraction_row: Option<usize>,
    pub jumps: crate::JumpList,
    /// Where the last jump came from (the `''` mark).
    pub pcmark: Option<Cursor>,
    /// Window-local options ('number', …).
    pub opts: crate::options::WindowOptions,
}

/// Columns the `<<<` marker covers at the start of a partly shown top line (Neovim's
/// `sms_marker_overlap`).
const SMS_MARKER: usize = 3;

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

    /// Display width of `line`.
    pub fn width_of(&self, line: usize) -> usize {
        layout_line(&self.text.line_str(line), self.tabstop, None).rows[0]
            .iter()
            .map(|g| usize::from(g.width))
            .sum()
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

    /// The virtual column of the Normal-mode cursor on character `col`: the last cell of a tab,
    /// the first cell of anything else. With `insert`, the first cell of a tab too.
    pub fn cursor_vcol(&self, line: usize, col: usize, insert: bool) -> usize {
        let layout = layout_line(&self.text.line_str(line), self.tabstop, None);
        let mut vcol = 0;
        for glyph in &layout.rows[0] {
            if glyph.char_idx == col {
                if glyph.kind != flux_core::GlyphKind::Tab || insert {
                    return vcol;
                }
                let tab_end = layout.rows[0].iter().filter(|g| g.char_idx == col).count();
                return vcol + tab_end - 1;
            }
            if glyph.char_idx > col {
                break;
            }
            vcol += usize::from(glyph.width);
        }
        vcol
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
}

impl Window {
    pub fn new(id: crate::WindowId, buffer: BufferId, width: usize, height: usize) -> Self {
        Self {
            id,
            buffer,
            alt_buffer: None,
            cursor: Cursor::default(),
            curswant: 0,
            set_curswant: true,
            top: 0,
            skipcol: 0,
            skip_top: 0,
            scroll: (height / 2).max(1),
            keep_view: None,
            valid_line: 0,
            topline_dirty: false,
            width,
            height,
            fraction: 0,
            fraction_row: None,
            jumps: Default::default(),
            pcmark: None,
            opts: Default::default(),
        }
    }

    /// Where the cursor is drawn within the window's text area: (row, column). `insert` puts
    /// it at the start of a tab.
    pub fn cursor_screen_offset(&self, m: &Metrics, insert: bool) -> (usize, usize) {
        let layout = layout_line(&m.text.line_str(self.cursor.line), m.tabstop, Some(m.width));
        let (_, x) = layout.cursor_position(self.cursor.col, insert);
        let skipped = if self.cursor.line == self.top {
            self.skip_rows()
        } else {
            0
        };
        (
            self.cursor_row(m).saturating_sub(skipped),
            x.min(m.width.saturating_sub(1)),
        )
    }

    /// Screen row of the cursor within the window.
    fn cursor_row(&self, m: &Metrics) -> usize {
        let above: usize = (self.top..self.cursor.line).map(|l| m.rows(l)).sum();
        let layout = layout_line(&m.text.line_str(self.cursor.line), m.tabstop, Some(m.width));
        above + layout.cursor_position(self.cursor.col, false).0
    }

    /// Change the window's height, keeping the cursor at the same relative height, the way Vim
    /// does (`win_new_height` and `scroll_to_fraction`). `m` describes the new height.
    pub fn set_height(&mut self, m: &Metrics) {
        let height = m.height;
        if height != self.height {
            self.scroll = (height / 2).max(1);
        }
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
        let mut line_size = layout.cursor_position(self.cursor.col, false).0 as isize;
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

    /// Vim's 'scroll': half the window height unless a count to `CTRL-D` or `CTRL-U` set it.
    pub fn scroll_amount(&self) -> usize {
        self.scroll
    }

    /// `skipcol` for the current top line.
    pub fn skipcol(&self) -> usize {
        if self.skip_top == self.top {
            self.skipcol
        } else {
            0
        }
    }

    fn set_skipcol(&mut self, skipcol: usize) {
        self.skipcol = skipcol;
        self.skip_top = self.top;
    }

    /// Screen rows of the top line hidden by `skipcol`.
    pub fn skip_rows(&self) -> usize {
        self.skipcol() / self.width.max(1)
    }

    /// Last line shown completely.
    pub fn bottom(&self, m: &Metrics) -> usize {
        let mut used = 0;
        let mut line = self.top.min(m.last_line());
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

    /// Bring `curswant` up to date if a horizontal move left it stale.
    pub fn update_curswant(&mut self, m: &Metrics, insert: bool) {
        if self.set_curswant {
            self.curswant = m.cursor_vcol(self.cursor.line, self.cursor.col, insert);
            self.set_curswant = false;
        }
    }

    /// Move the cursor to `line`, in the column nearest `curswant`, and scroll it into view.
    pub fn set_cursor_line(&mut self, line: usize, m: &Metrics) {
        self.update_curswant(m, false);
        self.place_on_line(line.min(m.last_line()), m);
        self.scroll_to_cursor(m);
    }

    /// Put the cursor at an exact position; vertical moves will aim for its column.
    pub fn set_cursor(&mut self, line: usize, col: usize, m: &Metrics) {
        let line = line.min(m.last_line());
        self.cursor = Cursor { line, col };
        self.set_curswant = true;
        self.scroll_to_cursor(m);
    }

    fn place_on_line(&mut self, line: usize, m: &Metrics) {
        self.cursor.line = line;
        self.cursor.col = m.col_for_vcol(line, self.curswant);
    }

    /// Scroll so the cursor line is visible, the way Vim does after a cursor move: a short
    /// distance scrolls just enough, a long one puts the cursor in the middle.
    pub fn scroll_to_cursor(&mut self, m: &Metrics) {
        // Lines may have been deleted since the view was last placed.
        self.top = self.top.min(m.last_line());
        self.cursor.line = self.cursor.line.min(m.last_line());
        let cur = self.cursor.line;
        let state = (cur, self.top, m.text.line_count(), m.rows(cur));
        if self.keep_view.take() == Some(state) {
            self.keep_view = Some(state);
            self.update_skipcol(m);
            return;
        }
        // Vim's `update_topline` with 'scrolloff' 0 and 'scrolljump' 1. Distances are in
        // lines; the scroll functions then count rows.
        let hidden_on_top = self.skipcol() > 0
            && cur == self.top
            && self.skipcol() + SMS_MARKER > m.cursor_vcol(cur, self.cursor.col, false);
        if cur < self.top || hidden_on_top {
            let halfheight = (self.height / 2).saturating_sub(1).max(2);
            if self.top - cur >= halfheight {
                self.top = self.halfway(cur, false, m);
            } else {
                self.scroll_cursor_top(m);
            }
        }
        let (botline, _) = self.botline(m);
        if botline < m.text.line_count() && cur >= botline {
            if cur - botline < self.height + 1 {
                self.scroll_cursor_bot(m);
            } else {
                self.top = self.halfway(cur, false, m);
            }
        }
        self.update_skipcol(m);
    }

    /// The part of Vim's `curs_columns` that sets `w_skipcol`: when the cursor is on a top line
    /// too tall for the window, scroll within the line so the cursor is shown; otherwise
    /// nothing is skipped.
    fn update_skipcol(&mut self, m: &Metrics) {
        let cur = self.cursor.line;
        if cur != self.top || self.height == 0 || self.width == 0 {
            // Without 'smoothscroll' nothing is skipped unless the cursor needs it.
            self.set_skipcol(0);
            return;
        }
        let (w, h) = (self.width as isize, self.height as isize);
        let vcol = m.cursor_vcol(cur, self.cursor.col, false) as isize;
        let prev = self.skipcol() as isize;
        let mut skip = prev;
        let mut wcol = vcol;
        let mut wrow = 0;
        let mut did_sub = false;
        if skip > 0 && wcol >= skip {
            wcol -= w * (if skip <= w { 1 } else { (skip - w) / w + 1 });
            did_sub = true;
        }
        if wcol >= w {
            wrow += (wcol - w) / w + 1;
        }
        let plines = m.rows(cur) as isize;
        if !(wrow >= h || (prev > 0 && plines > h)) {
            self.set_skipcol(0);
            return;
        }
        let mut extra = 0;
        if skip > vcol {
            extra = 1;
        }
        let plines = plines - 1;
        let n = if plines > wrow { wrow } else { plines };
        if n >= h + skip / w {
            extra += 2;
        }
        if extra == 3 {
            // Put the cursor in the middle.
            let mut n = vcol / w;
            n = if n > h / 2 { n - h / 2 } else { 0 };
            n = n.min(plines - h + 1);
            skip = if n > 0 { w + (n - 1) * w } else { 0 };
        } else if extra == 1 {
            let mut e = (skip - vcol + w - 1) / w;
            if e > 0 {
                if e * w > skip {
                    e = skip / w;
                }
                skip -= e * w;
            }
        } else if extra == 2 {
            let mut endcol = (n - h + 1) * w;
            while endcol > vcol {
                endcol -= w;
            }
            skip = skip.max(endcol);
        }
        if did_sub {
            wrow -= (skip - prev) / w;
        } else {
            wrow -= skip / w;
        }
        if wrow >= h {
            skip += (wrow - h + 1) * w;
        }
        self.set_skipcol(skip.max(0) as usize);
    }

    /// Vim's `w_botline` and `w_empty_rows`: the first line not completely shown, and the rows
    /// left over below the last one that is (past the end of the buffer, or under a wrapped
    /// line too tall to fit).
    fn botline(&self, m: &Metrics) -> (usize, usize) {
        let mut done = 0;
        let mut line = self.top;
        while line <= m.last_line() {
            let mut n = m.rows(line);
            if line == self.top {
                n -= self.skip_rows().min(n);
            }
            // A line too tall for the window counts as filling it.
            n = n.min(self.height);
            if done + n > self.height {
                break;
            }
            done += n;
            line += 1;
        }
        (line, if done == 0 { 0 } else { self.height - done })
    }

    /// Vim's `scroll_cursor_top` for a cursor above the window: usually the cursor line
    /// becomes the top line.
    fn scroll_cursor_top(&mut self, m: &Metrics) {
        let cur = self.cursor.line;
        let mut used = m.rows(cur);
        let mut scrolled = if cur < self.top { used } else { 0 };
        let mut new_top = cur;
        let mut top = cur;
        while top > 0 {
            let i = m.rows(top - 1);
            if top - 1 < self.top {
                scrolled += i;
            }
            if new_top >= self.top || scrolled > 1 {
                break;
            }
            used += i;
            if used > self.height {
                break;
            }
            new_top = top - 1;
            top -= 1;
        }
        if used > self.height {
            self.top = self.halfway(cur, false, m);
        } else if new_top < self.top {
            self.top = new_top;
        }
    }

    /// Vim's `scroll_cursor_bot` for a cursor a little below the window: scroll just enough
    /// lines to show it, or put it in the middle when that would be a whole window's worth.
    fn scroll_cursor_bot(&mut self, m: &Metrics) {
        const MIN_SCROLL: isize = 1;
        let h = self.height;
        let count = m.text.line_count();
        let rows = |line: usize| {
            if line < count {
                m.rows(line)
            } else {
                usize::MAX / 4
            }
        };
        let cur = self.cursor.line;
        let (botline, empty) = self.botline(m);
        let empty = empty as isize;
        let mut used = rows(cur);
        let mut scrolled: isize = 0;
        if cur >= botline {
            scrolled = used as isize;
            if cur == botline {
                scrolled -= empty;
            }
        }
        let (mut loff, mut boff) = (cur, cur);
        while loff > 0 {
            if (scrolled <= 0 || scrolled >= MIN_SCROLL || boff + 1 >= count) && loff <= botline {
                break;
            }
            loff -= 1;
            let height = rows(loff);
            used += height;
            if used > h {
                break;
            }
            if loff >= botline {
                scrolled += height as isize;
                if loff == botline {
                    scrolled -= empty;
                }
            }
            if boff + 1 < count {
                boff += 1;
                let height = rows(boff);
                used += height;
                if used > h {
                    break;
                }
                if scrolled < MIN_SCROLL && boff >= botline {
                    scrolled += height as isize;
                    if boff == botline {
                        scrolled -= empty;
                    }
                }
            }
        }
        let line_count = if scrolled <= 0 {
            0
        } else if used > h {
            used
        } else {
            // Lines to scroll to move `scrolled` rows off the top.
            let mut n = 0;
            let mut rows_moved = 0;
            let mut line = self.top;
            while (rows_moved as isize) < scrolled && line < botline + 1 {
                rows_moved += rows(line);
                n += 1;
                line += 1;
            }
            if (rows_moved as isize) < scrolled {
                9999
            } else {
                n
            }
        };
        if line_count >= h && line_count as isize > MIN_SCROLL {
            self.top = self.halfway(cur, true, m);
        } else if line_count > 0 {
            self.top = (self.top + line_count).min(m.last_line());
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
    /// `prefer_above` puts the extra one above the cursor. `atend` counts the `~` rows past the
    /// end of the buffer as used, which `zz` does and a jump doesn't.
    fn halfway(&self, cur: usize, prefer_above: bool, m: &Metrics) -> usize {
        self.halfway_at(cur, prefer_above, false, m)
    }

    fn halfway_at(&self, cur: usize, prefer_above: bool, atend: bool, m: &Metrics) -> usize {
        // Neovim's `scroll_cursor_halfway` without 'smoothscroll': each pass adds one line
        // below and one above the cursor (above first with `prefer_above`), whatever their
        // heights, until the window is full.
        let h = self.height;
        let mut used = m.rows(cur);
        let (mut top, mut bottom) = (cur, cur);
        'outer: while top > 0 {
            for round in 1..=2 {
                let (add_below, add_above) = if prefer_above {
                    (round == 2, round == 1)
                } else {
                    (round == 1, round == 1)
                };
                if add_below {
                    if bottom < m.last_line() {
                        bottom += 1;
                        used += m.rows(bottom);
                        if used > h {
                            break 'outer;
                        }
                    } else if atend {
                        // Past the end: a `~` row, which only `atend` counts as used space.
                        used += 1;
                    }
                }
                if add_above {
                    used += m.rows(top - 1);
                    if used > h {
                        break 'outer;
                    }
                    top -= 1;
                }
            }
        }
        top
    }

    /// `zt`, `zz`, `zb`: put the cursor line at the top, middle or bottom of the window.
    pub fn scroll_cursor_to(&mut self, at: char, m: &Metrics) {
        let cur = self.cursor.line;
        self.top = match at {
            't' => cur,
            'b' => self.top_with_bottom(cur, m),
            _ => self.halfway_at(cur, false, true, m),
        };
    }

    /// Keep the cursor inside the window after the view moved.
    fn clamp_cursor_to_view(&mut self, m: &Metrics) {
        self.update_curswant(m, false);
        let line = self.cursor.line.clamp(self.top, self.bottom(m));
        if line != self.cursor.line {
            self.place_on_line(line, m);
        }
    }

    /// Move the cursor `dist` screen rows down (or up), the way `gj`/`gk` do: through the rows
    /// of wrapped lines, keeping the position within the row. Returns false if it hit the end of
    /// the buffer first. This is Neovim's `nv_screengo`, which works on `curswant` alone.
    fn move_screen_rows(&mut self, down: bool, mut dist: usize, m: &Metrics) -> bool {
        self.update_curswant(m, false);
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
        self.set_curswant = false;
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

    /// `CTRL-D`: scroll down 'scroll' screen rows and move the cursor down as many rows. A
    /// count sets 'scroll'.
    pub fn scroll_half_down(&mut self, count: Option<usize>, m: &Metrics) -> bool {
        self.pagescroll(true, count.unwrap_or(0), true, m)
    }

    /// `CTRL-U`: the mirror image of `CTRL-D`.
    pub fn scroll_half_up(&mut self, count: Option<usize>, m: &Metrics) -> bool {
        self.pagescroll(false, count.unwrap_or(0), true, m)
    }

    /// `CTRL-F`: scroll forward `count` pages, keeping up to two lines of overlap, and put the
    /// cursor on the top line.
    pub fn page_down(&mut self, count: Option<usize>, m: &Metrics) -> bool {
        self.pagescroll(true, count.unwrap_or(1), false, m)
    }

    /// `CTRL-B`: scroll back `count` pages and put the cursor on the bottom line.
    pub fn page_up(&mut self, count: Option<usize>, m: &Metrics) -> bool {
        self.pagescroll(false, count.unwrap_or(1), false, m)
    }

    /// Neovim's `pagescroll`. The view scrolls by screen rows as with 'smoothscroll', then
    /// finishes scrolling a partly shown top line. A half page moves the cursor as many screen
    /// rows as the view moved (without revealing rows past the end); a whole page puts it at
    /// the top (forward) or bottom (backward) of the window.
    fn pagescroll(&mut self, forward: bool, count: usize, half: bool, m: &Metrics) -> bool {
        self.update_curswant(m, false);
        let (prev_cursor, prev_curswant) = (self.cursor, self.curswant);
        let h = self.height;
        let mut did_move = false;
        self.valid_line = self.cursor.line;
        self.topline_dirty = false;
        if half {
            if count > 0 {
                self.scroll = count.min(h);
            }
            let mut count = self.scroll.min(h) as isize;
            let mut curscount = count;
            let lines = m.text.line_count();
            if forward && self.top + 1 + h + count as usize > lines {
                let cap = h + count as usize;
                let mut n = m.rows(self.top) - self.skip_rows();
                if (n as isize) - count < h as isize && self.top + 1 < lines {
                    for line in self.top + 1..lines {
                        if n >= cap {
                            break;
                        }
                        n += m.rows(line);
                    }
                    n = n.min(cap);
                }
                if n < cap {
                    count = n as isize - h as isize;
                }
            }
            if count > 0 {
                did_move = self.scroll_with_sms(forward, count as usize, &mut curscount, m);
                self.cursor = prev_cursor;
                self.curswant = prev_curswant;
            }
            self.move_screen_rows(forward, curscount.max(0) as usize, m);
        } else {
            let count = count.max(1) * self.scroll_overlap(forward, m);
            let mut unused = 0;
            did_move = self.scroll_with_sms(forward, count, &mut unused, m);
            if did_move {
                self.cursor.line = if forward {
                    self.top
                } else {
                    self.botline(m).0.saturating_sub(1)
                };
            }
        }
        if !did_move && self.cursor == prev_cursor {
            return false;
        }
        // 'nostartofline': keep the column.
        self.coladvance(m);
        self.check_cursor_moved();
        if self.topline_dirty {
            self.scroll_to_cursor(m);
        } else {
            // Neovim only revalidates the top line when the cursor was seen on another line,
            // so a cursor line left partly shown stays that way.
            let line = self.cursor.line;
            self.keep_view = Some((line, self.top, m.text.line_count(), m.rows(line)));
            self.update_skipcol(m);
        }
        true
    }

    /// Neovim's `scroll_with_sms`: scroll `count` rows, then scroll on (or back, if the top line
    /// already changed) until no line is partly shown at the top, adjusting `curscount` by the
    /// extra rows. True when the view moved.
    fn scroll_with_sms(
        &mut self,
        forward: bool,
        count: usize,
        curscount: &mut isize,
        m: &Metrics,
    ) -> bool {
        let (prev_top, prev_skip) = (self.top, self.skipcol());
        self.sms_scroll(forward, count, m);
        let skipcol = self.skipcol();
        if skipcol > 0 {
            // One line extra going backward so that consuming the partial line is symmetric.
            let fix_forward = if self.top.abs_diff(prev_top) > usize::from(!forward) {
                !forward
            } else {
                forward
            };
            let w = self.width.max(1) as isize;
            let sc = skipcol as isize;
            let count = if fix_forward {
                1 + (m.width_of(self.top) as isize - sc - w + w - 1) / w
            } else {
                1 + (sc - w - 1) / w
            };
            let count = count.max(0);
            self.sms_scroll(fix_forward, count as usize, m);
            *curscount += if fix_forward == forward {
                count
            } else {
                -count
            };
        }
        self.top != prev_top || self.skipcol() != prev_skip
    }

    /// Neovim's `get_scroll_overlap`: rows to scroll for a page, less up to two lines that stay
    /// in view.
    fn scroll_overlap(&self, forward: bool, m: &Metrics) -> usize {
        const TALL: i64 = 1 << 40;
        let h = self.height as i64;
        let min_height = h - 2;
        let count = m.text.line_count() as i64;
        let (botline, _) = self.botline(m);
        if (!forward && self.top == 0) || (forward && botline as i64 >= count) {
            return h as usize;
        }
        let rows = |line: i64| {
            if line < 0 || line >= count {
                TALL
            } else {
                m.rows(line as usize) as i64
            }
        };
        let (start, step) = if forward {
            (botline as i64, -1)
        } else {
            (self.top as i64 - 1, 1)
        };
        let h1 = rows(start);
        if h1 > min_height {
            return h as usize;
        }
        let h2 = rows(start + step);
        if h2 + h1 > min_height {
            return h as usize;
        }
        let h3 = rows(start + 2 * step);
        if h3 + h2 > min_height {
            return h as usize;
        }
        let h4 = rows(start + 3 * step);
        if h4 + h3 + h2 > min_height || h3 + h2 + h1 > min_height {
            (min_height + 1) as usize
        } else {
            min_height.max(0) as usize
        }
    }

    /// Neovim's `scroll_redraw` with 'smoothscroll' on: scroll `count` screen rows, leaving
    /// `skipcol` columns of the top line scrolled off, and keep the cursor in view.
    fn sms_scroll(&mut self, up: bool, count: usize, m: &Metrics) {
        let mut skip = self.skipcol();
        let skipcol = &mut skip;
        let prev_line = self.cursor.line;
        let w = self.width.max(1);
        let last = m.last_line();
        if up {
            // `scrollup`
            let mut size = m.width_of(self.top);
            for _ in 0..count {
                let mut line = self.top;
                *skipcol += w;
                if *skipcol >= size {
                    if line == last {
                        *skipcol -= w;
                        break;
                    }
                    line += 1;
                }
                if line > self.top {
                    self.top = line;
                    *skipcol = 0;
                    size = m.width_of(self.top);
                }
            }
            if self.cursor.line < self.top {
                self.cursor.line = self.top;
                self.coladvance(m);
            }
        } else {
            // `scrolldown`
            self.check_cursor_moved();
            let old_wrow = self.cursor_wrow(m);
            let mut done = 0;
            for _ in 0..count {
                if self.top == 0 && *skipcol < w {
                    break;
                }
                done += 1;
                if *skipcol >= w {
                    *skipcol -= w;
                } else {
                    // The line above comes in showing only its last row.
                    self.top -= 1;
                    *skipcol = 0;
                    let mut size = m.width_of(self.top);
                    if size > w {
                        *skipcol = w;
                        size -= w;
                    }
                    while size > w {
                        *skipcol += w;
                        size -= w;
                    }
                }
            }
            // Move the cursor up until the last row of its line is in the window. Like Vim,
            // line heights are capped at the window height.
            let h = self.height;
            let vcol = m.cursor_vcol(self.cursor.line, self.cursor.col, false);
            let mut wrow = old_wrow + done + m.rows(self.cursor.line).min(h) as isize
                - 1
                - (vcol / w) as isize;
            let mut moved = false;
            while wrow >= h as isize && self.cursor.line > 0 {
                wrow -= m.rows(self.cursor.line).min(h) as isize;
                self.cursor.line -= 1;
                moved = true;
            }
            if moved {
                self.coladvance(m);
            }
            self.cursor.line = self.cursor.line.max(self.top);
        }
        self.set_skipcol(*skipcol);
        self.cursor_correct_sms(*skipcol, m);
        if self.cursor.line != prev_line {
            self.coladvance(m);
        }
    }

    /// Vim's `check_cursor_moved` during a page scroll: seeing the cursor on another line than
    /// the last validated one invalidates the top line.
    fn check_cursor_moved(&mut self) {
        if self.cursor.line != self.valid_line {
            self.topline_dirty = true;
            self.valid_line = self.cursor.line;
        }
    }

    /// The window row of the cursor (Vim's `w_wrow`), with line heights capped at the window
    /// height.
    fn cursor_wrow(&self, m: &Metrics) -> isize {
        let w = self.width.max(1);
        let skip = self.skipcol();
        let mut row = 0;
        for line in self.top..self.cursor.line {
            let mut n = m.rows(line);
            if line == self.top {
                n -= self.skip_rows().min(n);
            }
            row += n.min(self.height);
        }
        let mut wcol = m.cursor_vcol(self.cursor.line, self.cursor.col, false);
        if self.cursor.line == self.top && skip > 0 && wcol >= skip {
            wcol -= w * if skip <= w { 1 } else { (skip - w) / w + 1 };
        }
        if wcol >= w {
            row += (wcol - w) / w + 1;
        }
        row as isize
    }

    /// Neovim's `cursor_correct_sms` with 'scrolloff' 0: on a partly shown top line, move the
    /// cursor to a screen row that is shown, changing `curswant`.
    fn cursor_correct_sms(&mut self, skipcol: usize, m: &Metrics) {
        if self.cursor.line != self.top {
            return;
        }
        self.check_cursor_moved();
        let w = self.width.max(1);
        // The `<<<` marker covers the start of a partly shown line.
        let overlap = if skipcol == 0 { 0 } else { 3 };
        let top = skipcol + overlap;
        let bot = skipcol + w + self.height.saturating_sub(1) * w;
        let vcol = m.cursor_vcol(self.cursor.line, self.cursor.col, false);
        let mut col = vcol;
        if col < top {
            if col < w {
                col += w;
            }
            while col < top {
                col += w;
            }
        } else {
            while col >= bot {
                col -= w;
            }
        }
        if col != vcol {
            self.curswant = col;
            let failed = !self.coladvance(m);
            if failed && skipcol > 0 && self.cursor.line < m.last_line() {
                let vcol = m.cursor_vcol(self.cursor.line, self.cursor.col, false);
                if vcol < skipcol + overlap {
                    // Still not visible: go to the next line instead.
                    self.cursor.line += 1;
                    self.cursor.col = 0;
                    self.curswant = 0;
                }
            }
        }
    }

    /// Vim's `coladvance(curswant)` on the cursor line; false when the line is too short to
    /// reach it.
    fn coladvance(&mut self, m: &Metrics) -> bool {
        self.place_on_line(self.cursor.line, m);
        let width = m.width_of(self.cursor.line);
        self.curswant < width || (width == 0 && self.curswant == 0)
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
        let mut w = Window::new(crate::WindowId(1000), BufferId(1), 80, H);
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
            w.scroll_half_down(None, &m);
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
            w.scroll_half_up(None, &m);
            assert_eq!(pos(&w), to, "CTRL-U from {from:?}");
        }
    }

    #[test]
    fn ctrl_f_and_ctrl_b() {
        let text = numbered(200);
        let m = metrics(&text);
        for (top, to) in [(1, 21), (170, 190), (179, 200), (190, 200), (199, 200)] {
            let mut w = win(top, top);
            assert!(w.page_down(None, &m));
            assert_eq!(pos(&w), (to, to), "CTRL-F from {top}");
        }
        assert!(!win(200, 200).page_down(None, &m));
        let mut w = win(30, 40);
        w.page_down(None, &m);
        assert_eq!(pos(&w), (50, 50));
        for ((top, cur), to) in [
            ((200, 200), (178, 199)),
            ((195, 195), (175, 196)),
            ((179, 179), (159, 180)),
            ((100, 121), (80, 101)),
            ((2, 3), (1, 22)),
        ] {
            let mut w = win(top, cur);
            assert!(w.page_up(None, &m));
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
    fn view_recovers_when_lines_below_it_are_deleted() {
        let mut w = win(90, 100);
        let text = numbered(3);
        let m = metrics(&text);
        w.scroll_to_cursor(&m);
        assert!(w.top <= 2 && w.cursor.line == 2, "{:?}", pos(&w));
    }

    #[test]
    fn short_file() {
        let text = numbered(10);
        let m = metrics(&text);
        let mut w = win(1, 1);
        w.scroll_half_down(None, &m);
        assert_eq!(pos(&w), (1, 10));
        w.page_down(None, &m);
        assert_eq!(pos(&w), (10, 10));
        w.page_up(None, &m);
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
        w.scroll_half_down(None, &m);
        assert_eq!(pos(&w), (21, 29));
        w.set_cursor_line(39, &m);
        assert_eq!(pos(&w), (28, 40));
        w.set_cursor_line(0, &m);
        w.page_down(None, &m);
        assert_eq!(pos(&w), (13, 13));
    }
}
