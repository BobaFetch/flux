//! Draws the editor into a grid: the window's text, the statusline and the command line.

use flux_core::{GlyphKind, LineLayout, layout_line};
use flux_view::search;
use flux_view::{
    Buffer, CMDLINE_ROWS, Cursor, Editor, MessageKind, Mode, Rect, VisualKind, Window,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::grid::{Grid, Style};
use crate::theme::Theme;

mod pum;

/// Draw `editor` into `grid`, returning where the terminal cursor should go. `showcmd` is a
/// partly typed command, shown at the bottom right as Vim's 'showcmd' does.
pub fn draw(editor: &Editor, showcmd: &str, grid: &mut Grid) -> Option<(usize, usize)> {
    let (width, height) = (grid.width(), grid.height());
    if width == 0 || height == 0 {
        return None;
    }
    let theme = Theme::new(editor);
    grid.default = theme.normal;
    let mut cursor = None;
    for (id, rect) in editor.window_rects() {
        let pane = Pane {
            editor,
            theme: &theme,
            win: editor.window_ref(id),
            rect,
        };
        let pos = pane.draw_text(grid);
        if pane.is_current() {
            cursor = pos;
        }
        pane.draw_statusline(grid);
        if rect.vsep {
            pane.draw_separator(grid);
        }
    }
    for float in &editor.floats {
        draw_float(editor, &theme, float, grid);
    }
    pum::draw(editor, &theme, grid);
    if let Some(pos) = draw_cmdline(editor, &theme, grid, height - 1) {
        cursor = Some(pos);
    }
    if editor.hit_enter
        && let Some(pos) = hit_enter(editor, &theme, grid)
    {
        return Some(pos);
    }
    let size;
    let showcmd = if showcmd.is_empty() && editor.mode == Mode::Visual && editor.message.is_none() {
        size = selection_size(editor);
        size.as_str()
    } else {
        showcmd
    };
    if !showcmd.is_empty() && width > 11 && editor.mode != Mode::CmdLine {
        let text: String = showcmd
            .chars()
            .rev()
            .take(10)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        grid.put_str(width - 11, height - 1, &text, Style::default());
    }
    cursor
}

/// One window and where it is on screen.
struct Pane<'a> {
    editor: &'a Editor,
    theme: &'a Theme,
    win: &'a Window,
    rect: Rect,
}

impl Pane<'_> {
    fn is_current(&self) -> bool {
        self.win.id == self.editor.window.id
    }

    /// The text to show: the buffer's, or the 'inccommand' preview of it.
    fn text(&self) -> &flux_core::Text {
        match &self.editor.preview {
            Some(p) if self.win.buffer == self.editor.window.buffer => &p.text,
            _ => &self.buffer().text,
        }
    }

    fn buffer(&self) -> &Buffer {
        self.editor
            .buffer(self.win.buffer)
            .unwrap_or_else(|| self.editor.current_buffer())
    }

    /// The window's text rows, clipped to the grid.
    fn text_rows(&self, grid: &Grid) -> usize {
        self.rect
            .height
            .min(grid.height().saturating_sub(self.rect.row))
    }

    fn draw_text(&self, grid: &mut Grid) -> Option<(usize, usize)> {
        let win = self.win;
        let text = self.text();
        let text_rows = self.text_rows(grid);
        // The sign column and the number column come first; the text starts after them.
        let diagnostics = self.editor.buffer_diagnostics(win.buffer);
        let signw = flux_view::editor::sign_width(&win.opts.signcolumn, !diagnostics.is_empty());
        let numw = win
            .opts
            .number_width(text.line_count(), self.rect.height.max(1));
        let (top_row, left) = (self.rect.row, self.rect.col + signw + numw);
        let width = self.rect.width.saturating_sub(signw + numw).max(1);
        let insert = self.is_current() && self.editor.mode == Mode::Insert;
        let mut cursor = None;
        // The sign column is SignColumn and the number column LineNr, also where a wrapped line
        // has no sign or number.
        for r in 0..text_rows.min(self.rect.height) {
            for x in 0..signw {
                grid.set(
                    self.rect.col + x,
                    top_row + r,
                    " ",
                    1,
                    self.theme.sign_column,
                );
            }
            for x in signw..signw + numw {
                grid.set(self.rect.col + x, top_row + r, " ", 1, self.theme.line_nr);
            }
        }
        // A diagnostic's sign goes on its first line. They all have the same priority, so the
        // last one placed shows.
        let mut signs: std::collections::HashMap<usize, u8> = std::collections::HashMap::new();
        for (start, _, d) in &diagnostics {
            signs.insert(start.line, d.severity);
        }
        let numx = self.rect.col + signw;
        let mut row = 0;
        let mut line = win.top;
        let shown_lines = win.top..(win.top + text_rows).min(text.line_count());
        let matches = self.search_matches(shown_lines.clone());
        let syntax = self.syntax_spans(shown_lines);
        while row < text_rows && line < text.line_count() {
            let mut layout = layout_line(
                &text.line_str(line),
                self.buffer().opts.tabstop,
                Some(width),
            );
            // Rows of a too-tall top line scrolled off above the window (Vim's `w_skipcol`).
            let skip = if line == win.top {
                win.skip_rows().min(layout.row_count() - 1)
            } else {
                0
            };
            let full = layout.clone();
            layout.rows.drain(..skip);
            let fits = row + layout.row_count() <= text_rows;
            if !fits && line != win.top {
                // Vim's `display=lastline`: show what fits and mark the cut with `@@@`.
                if numw > 0 {
                    self.draw_number(grid, line, numx, top_row + row, numw);
                }
                draw_rows(
                    grid,
                    self.theme,
                    &layout,
                    left,
                    top_row + row,
                    text_rows - row,
                );
                for x in width.saturating_sub(3)..width {
                    let non_text = self.theme.non_text;
                    grid.set(left + x, top_row + text_rows - 1, "@", 1, non_text);
                }
                row = text_rows;
                break;
            }
            let shown = layout.row_count().min(text_rows - row);
            if numw > 0 {
                self.draw_number(grid, line, numx, top_row + row, numw);
            }
            if signw > 0
                && let Some(&severity) = signs.get(&line)
            {
                let sign = ["E ", "W ", "I ", "H "][usize::from(severity - 1)];
                let style = self.theme.diagnostic_sign[usize::from(severity - 1)];
                grid.put_str_until(
                    self.rect.col,
                    top_row + row,
                    sign,
                    style,
                    self.rect.col + signw,
                );
            }
            draw_rows(grid, self.theme, &layout, left, top_row + row, shown);
            let at = Paint {
                layout: &layout,
                left,
                width,
                row: top_row + row,
                rows: shown,
            };
            for span in syntax.iter().filter(|s| s.line == line) {
                if !span.capture.is_empty() {
                    let style = self.theme.capture(span.capture);
                    at.paint(grid, (span.start, span.end), style, false);
                }
                if span.url.is_some() {
                    at.link(grid, (span.start, span.end), &span.url);
                }
            }
            // A snippet's tabstops.
            if win.id == self.editor.window.id {
                for &(s, e, active) in &self.editor.completion.snippet {
                    if s.line <= line && line <= e.line {
                        let from = if s.line == line { s.col } else { 0 };
                        let to = if e.line == line {
                            e.col
                        } else {
                            text.line_len(line)
                        };
                        let group = if active {
                            "SnippetTabstopActive"
                        } else {
                            "SnippetTabstop"
                        };
                        at.paint(grid, (from, to), self.theme.group(group), false);
                    }
                }
            }
            // A hover float's range (LspReferenceTarget).
            for f in &self.editor.floats {
                if f.window != win.id {
                    continue;
                }
                if let Some((s, e)) = f.target
                    && s.line <= line
                    && line <= e.line
                {
                    let from = if s.line == line { s.col } else { 0 };
                    let to = if e.line == line {
                        e.col
                    } else {
                        text.line_len(line)
                    };
                    at.paint(
                        grid,
                        (from, to),
                        self.theme.group("LspReferenceTarget"),
                        false,
                    );
                }
            }
            // Diagnostics are underlined, the most severe last.
            let mut on_line: Vec<_> = diagnostics
                .iter()
                .filter(|(s, e, _)| s.line <= line && line <= e.line)
                .collect();
            on_line.sort_by_key(|(_, _, d)| std::cmp::Reverse(d.severity));
            for (s, e, d) in on_line {
                let from = if s.line == line { s.col } else { 0 };
                let to = if e.line == line {
                    e.col
                } else {
                    text.line_len(line)
                };
                let style = self.theme.diagnostic_underline[usize::from(d.severity - 1)];
                at.paint(grid, (from, to.max(from)), style, false);
            }
            if self.buffer().directory && text.line_str(line).ends_with('/') {
                let len = text.line_len(line);
                at.paint(grid, (0, len), self.theme.directory, false);
            }
            if skip > 0 {
                for x in 0..3.min(width) {
                    grid.set(left + x, top_row + row, "<", 1, self.theme.non_text);
                }
            }
            for &(start, end, current) in &matches {
                if start.line > line || end.line < line {
                    continue;
                }
                let from = if start.line == line { start.col } else { 0 };
                let to = if end.line == line {
                    end.col
                } else {
                    text.line_len(line) + 1
                };
                if from < to {
                    let style = match current {
                        Match::Current => self.theme.cur_search,
                        Match::Incremental => self.theme.inc_search,
                        Match::Preview => self.theme.substitute,
                        Match::Other => self.theme.search,
                    };
                    at.paint(grid, (from, to), style, true);
                }
            }
            // MatchParen (a match of priority 10) goes over Search; Visual over both.
            if self.is_current()
                && let Some(brackets) = self.editor.matchparen
            {
                for b in brackets.iter().filter(|b| b.line == line) {
                    at.paint(grid, (b.col, b.col + 1), self.theme.match_paren, false);
                }
            }
            for range in self.selected_columns(line) {
                at.paint(grid, range, self.theme.visual, true);
            }
            if line == win.cursor.line {
                let (r, x) = full.cursor_position(win.cursor.col, insert);
                let r = r.wrapping_sub(skip);
                if r < shown {
                    cursor = Some((left + x.min(width.max(1) - 1), top_row + row + r));
                }
            }
            row += shown;
            line += 1;
        }
        // Past the end: `~`, and the rest of the row in the same highlight (EndOfBuffer).
        for r in row..text_rows {
            let eob = self.theme.end_of_buffer;
            grid.set(self.rect.col, top_row + r, "~", 1, eob);
            for x in 1..self.rect.width {
                grid.set(self.rect.col + x, top_row + r, " ", 1, eob);
            }
        }
        cursor
    }

    /// The number column for `line`: its number ('number'), its distance from the cursor line
    /// ('relativenumber'), or both (the cursor line's own number, left-aligned).
    fn draw_number(&self, grid: &mut Grid, line: usize, x: usize, y: usize, numw: usize) {
        let opts = &self.win.opts;
        let cur = self.win.cursor.line;
        let digits = numw - 1;
        let s = if opts.relativenumber {
            if line == cur && opts.number {
                format!("{:<digits$} ", line + 1)
            } else {
                format!("{:>digits$} ", line.abs_diff(cur))
            }
        } else {
            format!("{:>digits$} ", line + 1)
        };
        // CursorLineNr is only for 'cursorline', which flux doesn't have.
        let style = self.theme.line_nr;
        grid.put_str_until(x, y, &s, style, x + numw);
    }

    /// Matches to highlight in `lines`: every match of the last search pattern with
    /// 'hlsearch' (or of the pattern being typed, with 'incsearch'), and whether it's the
    /// current one (CurSearch/IncSearch).
    fn search_matches(
        &self,
        lines: std::ops::Range<usize>,
    ) -> Vec<(flux_view::Cursor, flux_view::Cursor, Match)> {
        let editor = self.editor;
        if let Some(p) = &editor.preview
            && self.win.buffer == editor.window.buffer
        {
            return p
                .highlights
                .iter()
                .filter(|(s, e)| e.line >= lines.start && s.line < lines.end)
                .map(|&(s, e)| (s, e, Match::Preview))
                .collect();
        }
        let text = &self.buffer().text;
        let typing = editor.mode == Mode::CmdLine && editor.cmdline_kind != ':';
        let incsearch = if typing && self.is_current() {
            editor.incsearch
        } else {
            None
        };
        let hls = editor.options.hlsearch && !editor.search.no_hlsearch;
        let mut out = Vec::new();
        let pattern = match (&editor.incsearch_pattern, typing) {
            (Some(p), true) if hls => search::compile(editor, p, false).ok(),
            _ if hls => editor
                .search
                .last_pattern()
                .and_then(|p| search::compile(editor, &p.pat, p.no_smartcase).ok()),
            _ => None,
        };
        if let Some(pattern) = pattern {
            let cur = self.win.cursor;
            for (s, e) in search::matches_in(text, &pattern, lines) {
                let at_cursor = incsearch.is_none()
                    && (s.line, s.col) <= (cur.line, cur.col)
                    && (cur.line, cur.col) < (e.line, e.col);
                out.push((
                    s,
                    e,
                    if at_cursor {
                        Match::Current
                    } else {
                        Match::Other
                    },
                ));
            }
        }
        if let Some((s, e)) = incsearch {
            out.push((s, e, Match::Incremental));
        }
        out
    }

    /// Syntax highlights for `lines`, in paint order. While `:s` previews a change the text on
    /// screen isn't the buffer's, and isn't highlighted.
    fn syntax_spans(&self, lines: std::ops::Range<usize>) -> Vec<flux_syntax::Span> {
        let editor = self.editor;
        let previewed = self.win.buffer == editor.window.buffer
            && editor.preview.as_ref().is_some_and(|p| p.changed);
        if !editor.syntax_on || previewed {
            return Vec::new();
        }
        let buffer = self.buffer();
        match &buffer.syntax {
            Some(syntax) => syntax.highlights(&buffer.text, lines),
            None => Vec::new(),
        }
    }

    /// The selected chars of `line` as `[from, to)` ranges, drawn the way Neovim draws them:
    /// the line break shows as a cell past the text only where the selection starts at it (an
    /// empty line), and in the current window the cell under the (block) cursor isn't
    /// highlighted (`noinvcur`). Like Vim, every window showing the buffer shows the selection.
    fn selected_columns(&self, line: usize) -> Vec<(usize, usize)> {
        let editor = self.editor;
        if !editor.visual_active() || self.win.buffer != editor.window.buffer {
            return Vec::new();
        }
        let a = editor.visual.anchor;
        let c = editor.window.cursor;
        let (start, end) = if (c.line, c.col) < (a.line, a.col) {
            (c, a)
        } else {
            (a, c)
        };
        if line < start.line || line > end.line {
            return Vec::new();
        }
        let len = editor.current_buffer().text.line_len(line);
        let eol = editor.window.curswant == usize::MAX && c == end;
        let (from, to, breaks) = match editor.visual.kind {
            VisualKind::Line => (0, len, true),
            VisualKind::Char => {
                let from = if line == start.line { start.col } else { 0 };
                if line == end.line && !eol {
                    (from, end.col + 1, end.col >= len)
                } else {
                    (from, len, true)
                }
            }
        };
        let to = if breaks && from >= len {
            len + 1
        } else {
            to.min(len)
        };
        if from >= to {
            return Vec::new();
        }
        if self.is_current() && line == c.line && (from..to).contains(&c.col) {
            return [(from, c.col), (c.col + 1, to)]
                .into_iter()
                .filter(|(f, t)| f < t)
                .collect();
        }
        vec![(from, to)]
    }

    /// Neovim's default statusline: `%<%f %h%w%m%r %=%-14.(%l,%c%V%) %P`. The cell below a
    /// vertical separator belongs to the statusline when another one continues to its right.
    fn draw_statusline(&self, grid: &mut Grid) {
        let y = self.rect.row + self.rect.height;
        if y >= grid.height().saturating_sub(CMDLINE_ROWS) {
            return;
        }
        let style = if self.is_current() {
            self.theme.status_line
        } else {
            self.theme.status_line_nc
        };
        let left = self.rect.col;
        let width = self.rect.width;
        for x in left..(left + width).min(grid.width()) {
            grid.set(x, y, " ", 1, style);
        }
        if self.rect.vsep {
            // The separator continues past the statusline unless another statusline follows.
            if self.editor.stl_connected(self.win.id) {
                grid.set(left + width, y, " ", 1, style);
            } else {
                grid.set(left + width, y, "│", 1, self.theme.win_separator);
            }
        }
        let buffer = self.buffer();
        let ruler = format!("{:<14} {}", self.cursor_ruler(), self.relative_position());
        // `%f %h%w%m%r `: the name, a space, the flags and another space.
        let current = self.win.buffer == self.editor.window.buffer;
        let hidden = self.editor.hide_modified && current;
        let previewed = current && self.editor.preview.as_ref().is_some_and(|p| p.changed);
        let flags = if (buffer.modified() && !hidden) || previewed {
            "[+]"
        } else {
            ""
        };
        let name = format!("{} {flags} ", buffer.name());
        // Neovim's default statusline shows the buffer's diagnostics before the ruler
        // (`vim.diagnostic.status()`): `E:1 W:2 `, each in its sign's color.
        let mut right: Vec<(String, Style)> = Vec::new();
        let mut counts = [0usize; 4];
        for (_, _, d) in self.editor.buffer_diagnostics(self.win.buffer) {
            counts[usize::from(d.severity - 1)] += 1;
        }
        let mut first = true;
        for (i, n) in counts.iter().enumerate().filter(|(_, n)| **n > 0) {
            let seg_style = style.combine(self.theme.diagnostic_sign[i]);
            if !first && let Some(last) = right.last_mut() {
                last.0.push(' ');
            }
            first = false;
            right.push((format!("{}:{n}", ["E", "W", "I", "H"][i]), seg_style));
        }
        if !right.is_empty() {
            right.push((" ".into(), style));
        }
        right.push((ruler, style));
        let name_width = UnicodeWidthStr::width(name.as_str());
        let right_width: usize = right
            .iter()
            .map(|(t, _)| UnicodeWidthStr::width(t.as_str()))
            .sum();
        if name_width + right_width <= width {
            // `%=` pushes the ruler to the right edge.
            grid.put_str_until(left, y, &name, style, left + width);
            let mut x = left + width - right_width;
            for (t, st) in &right {
                x = grid.put_str_until(x, y, t, *st, left + width);
            }
        } else {
            // Too wide: `%<` at the start cuts the front off everything, marked with `<`.
            let mut cells: Vec<(&str, Style)> = name.graphemes(true).map(|g| (g, style)).collect();
            for (t, st) in &right {
                cells.extend(t.graphemes(true).map(|g| (g, *st)));
            }
            let mut kept = Vec::new();
            let mut used = 1;
            for &(g, st) in cells.iter().rev() {
                let w = UnicodeWidthStr::width(g);
                if used + w > width {
                    break;
                }
                kept.push((g, st));
                used += w;
            }
            if width > 0 {
                let mut x = grid.put_str_until(left, y, "<", style, left + width);
                for (g, st) in kept.into_iter().rev() {
                    x = grid.put_str_until(x, y, g, st, left + width);
                }
            }
        }
    }

    /// The `│` column to the right of the window.
    fn draw_separator(&self, grid: &mut Grid) {
        let x = self.rect.col + self.rect.width;
        for r in 0..self.text_rows(grid) {
            grid.set(x, self.rect.row + r, "│", 1, self.theme.win_separator);
        }
    }

    /// `%l,%c%V`: line, byte column and, when different, screen column. An empty line is `0-1`.
    fn cursor_ruler(&self) -> String {
        let cursor = match self.editor.completion.ruler_cursor {
            Some(c) if self.is_current() => c,
            _ => self.win.cursor,
        };
        let text = self.text();
        let line = text.line_str(cursor.line.min(text.line_count().saturating_sub(1)));
        if line.is_empty() {
            // A buffer with no lines at all shows line 0.
            let n = if text.has_no_lines() {
                0
            } else {
                cursor.line + 1
            };
            // In Insert mode an empty line's column is 1 (Neovim's `empty_line` is only for
            // the other modes).
            if self.is_current() && self.editor.mode == Mode::Insert {
                return format!("{n},1");
            }
            return format!("{n},0-1");
        }
        let byte_col = line
            .char_indices()
            .nth(cursor.col)
            .map_or(line.len(), |(i, _)| i)
            + 1;
        let layout = layout_line(&line, self.buffer().opts.tabstop, None);
        let insert = self.is_current() && self.editor.mode == Mode::Insert;
        let screen_col = layout.cursor_position(cursor.col, insert).1 + 1;
        if screen_col == byte_col {
            format!("{},{byte_col}", cursor.line + 1)
        } else {
            format!("{},{byte_col}-{screen_col}", cursor.line + 1)
        }
    }

    /// `%P`: `All`, `Top`, `Bot`, or how far down the window is.
    fn relative_position(&self) -> String {
        let text = self.text();
        let win = self.win;
        let metrics = flux_view::Metrics {
            text,
            tabstop: self.buffer().opts.tabstop,
            width: win.width,
            height: win.height,
        };
        let above = win.top;
        let below = text.line_count().saturating_sub(win.bottom(&metrics) + 1);
        if below == 0 {
            if above == 0 { "All" } else { "Bot" }.to_string()
        } else if above == 0 {
            "Top".to_string()
        } else {
            format!("{:>2}%", above * 100 / (above + below))
        }
    }
}

/// A floating window: its lines in NormalFloat, concealed and wrapped (with 'linebreak') at
/// its width, with its buffer's syntax and its own highlights.
fn draw_float(editor: &Editor, theme: &Theme, float: &flux_view::float::Float, grid: &mut Grid) {
    let Some(buffer) = editor.buffer(float.buffer) else {
        return;
    };
    let base = theme.normal_float;
    for r in 0..float.height {
        for x in 0..float.width {
            grid.set(float.col + x, float.row + r, " ", 1, base);
        }
    }
    let text = &buffer.text;
    let spans = match &buffer.syntax {
        Some(s) if editor.syntax_on => s.highlights(text, 0..text.line_count()),
        _ => Vec::new(),
    };
    let mut row = 0;
    for shown_line in editor.float_lines(float) {
        if row >= float.height {
            break;
        }
        let layout = shown_line.layout(
            &text.line_str(shown_line.line),
            buffer.opts.tabstop,
            float.width,
        );
        let shown = layout.row_count().min(float.height - row);
        for (r, glyphs) in layout.rows.iter().take(shown).enumerate() {
            let mut x = float.col;
            for glyph in glyphs {
                let style = match glyph.kind {
                    GlyphKind::Text | GlyphKind::Tab => base,
                    GlyphKind::Special => base.combine(theme.special_key),
                    GlyphKind::Filler => base.combine(theme.non_text),
                };
                grid.set(x, float.row + row + r, &glyph.symbol, glyph.width, style);
                x += usize::from(glyph.width);
            }
        }
        let at = Paint {
            layout: &layout,
            left: float.col,
            width: float.width,
            row: float.row + row,
            rows: shown,
        };
        let line = shown_line.line;
        for span in spans.iter().filter(|s| s.line == line) {
            if !span.capture.is_empty() {
                at.paint(
                    grid,
                    (span.start, span.end),
                    theme.capture(span.capture),
                    false,
                );
            }
            if span.url.is_some() {
                at.link(grid, (span.start, span.end), &span.url);
            }
        }
        for h in float.highlights.iter().filter(|h| h.line == line) {
            at.paint(grid, (h.start, h.end), theme.group(&h.group), false);
        }
        row += shown;
    }
}

/// How a search match is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Match {
    /// A match of the last search pattern ('hlsearch'): Search.
    Other,
    /// The match under the cursor: CurSearch.
    Current,
    /// The match 'incsearch' shows while typing: IncSearch.
    Incremental,
    /// What `:s` would change, while typing it ('inccommand'): Substitute.
    Preview,
}

fn draw_rows(
    grid: &mut Grid,
    theme: &Theme,
    layout: &LineLayout,
    left: usize,
    first_row: usize,
    count: usize,
) {
    for (r, glyphs) in layout.rows.iter().take(count).enumerate() {
        let mut x = left;
        for glyph in glyphs {
            let style = match glyph.kind {
                GlyphKind::Text | GlyphKind::Tab => Style::default(),
                GlyphKind::Special => theme.special_key,
                GlyphKind::Filler => theme.non_text,
            };
            grid.set(x, first_row + r, &glyph.symbol, glyph.width, style);
            x += usize::from(glyph.width);
        }
    }
}

/// Where a drawn line is on screen, for painting highlights over it.
struct Paint<'a> {
    layout: &'a LineLayout,
    left: usize,
    width: usize,
    row: usize,
    rows: usize,
}

impl Theme {
    /// The style of a message: ErrorMsg, WarningMsg, or none.
    fn message(&self, message: &flux_view::Message) -> Style {
        match message.kind {
            MessageKind::Error => self.error_msg,
            MessageKind::Warning => self.warning_msg,
            _ => Style::default(),
        }
    }
}

impl Paint<'_> {
    /// Make chars `[from, to)` of the line a link.
    fn link(&self, grid: &mut Grid, (from, to): (usize, usize), url: &Option<std::sync::Arc<str>>) {
        for (r, glyphs) in self.layout.rows.iter().take(self.rows).enumerate() {
            let mut x = 0;
            for glyph in glyphs {
                if glyph.char_idx >= from && glyph.char_idx < to {
                    grid.set_link(self.left + x, self.row + r, url.clone());
                }
                x += usize::from(glyph.width);
            }
        }
    }

    /// Combine `style` into chars `[from, to)` of the line. With `eol`, a line break in the
    /// range shows as one painted cell after the text (a selection, a search match); without
    /// it, characters drawn specially (`^X`) keep their own look (syntax).
    fn paint(&self, grid: &mut Grid, (from, to): (usize, usize), style: Style, eol: bool) {
        let mut end_cell = (self.row, 0);
        for (r, glyphs) in self.layout.rows.iter().take(self.rows).enumerate() {
            let mut x = 0;
            for glyph in glyphs {
                let text = matches!(glyph.kind, GlyphKind::Text | GlyphKind::Tab);
                if glyph.char_idx >= from && glyph.char_idx < to && (eol || text) {
                    let cell = grid.cell(self.left + x, self.row + r).clone();
                    if cell.width > 0 {
                        let combined = cell.style.combine(style);
                        grid.set(
                            self.left + x,
                            self.row + r,
                            &cell.symbol,
                            cell.width,
                            combined,
                        );
                        grid.set_link(self.left + x, self.row + r, cell.link.clone());
                    }
                }
                x += usize::from(glyph.width);
            }
            end_cell = (self.row + r, x);
        }
        let len = self
            .layout
            .rows
            .iter()
            .flatten()
            .map(|g| g.char_idx + 1)
            .max()
            .unwrap_or(0);
        if eol && to > len && from <= len && end_cell.1 < self.width {
            grid.set(self.left + end_cell.1, end_cell.0, " ", 1, style);
        }
    }
}

/// Vim's 'showcmd' in Visual mode: the selection's size, `chars` (or `chars-bytes`) within a
/// line, otherwise lines.
fn selection_size(editor: &Editor) -> String {
    let a: Cursor = editor.visual.anchor;
    let c = editor.window.cursor;
    let lines = a.line.abs_diff(c.line) + 1;
    if editor.visual.kind == VisualKind::Line || lines > 1 {
        return lines.to_string();
    }
    let text = &editor.current_buffer().text;
    let s = text.line_str(c.line);
    let (from, to) = (a.col.min(c.col), a.col.max(c.col));
    let selected: String = s.chars().skip(from).take(to + 1 - from).collect();
    let chars = selected.chars().count().max(1);
    let bytes = selected.len().max(1);
    if chars == bytes {
        chars.to_string()
    } else {
        format!("{chars}-{bytes}")
    }
}

/// Vim's 'shortmess' `t`: drop the start of a message too wide for `room`, marking it with `<`.
fn truncate_start(s: &str, room: usize) -> String {
    let width = UnicodeWidthStr::width(s);
    if width <= room {
        return s.to_owned();
    }
    let mut size = width;
    let mut graphemes = s.graphemes(true);
    while size >= room {
        match graphemes.next() {
            Some(g) => size -= UnicodeWidthStr::width(g),
            None => break,
        }
    }
    format!("<{}", graphemes.collect::<String>())
}

/// Vim's 'shortmess' `T`: cut the middle out of a message too wide for `room`, leaving `...`.
fn truncate_middle(s: &str, room: usize) -> String {
    if UnicodeWidthStr::width(s) <= room || room < 4 {
        return s.to_owned();
    }
    let room = room - 3;
    let half = room / 2;
    let mut head = String::new();
    let mut used = 0;
    for g in s.graphemes(true) {
        let w = UnicodeWidthStr::width(g);
        if used + w > half {
            break;
        }
        head.push_str(g);
        used += w;
    }
    let mut tail: Vec<&str> = Vec::new();
    let mut tail_used = 0;
    for g in s.graphemes(true).rev() {
        let w = UnicodeWidthStr::width(g);
        if used + tail_used + w > room {
            break;
        }
        tail.push(g);
        tail_used += w;
    }
    tail.reverse();
    format!("{head}...{}", tail.concat())
}

/// Just the command line, over a screen left as it was (see `Editor::stale_screen`).
pub fn draw_cmdline_only(editor: &Editor, grid: &mut Grid) -> Option<(usize, usize)> {
    let y = grid.height().checked_sub(1)?;
    let theme = Theme::new(editor);
    grid.default = theme.normal;
    grid.fill_row(y, Style::default());
    draw_cmdline(editor, &theme, grid, y)
}

fn draw_cmdline(
    editor: &Editor,
    theme: &Theme,
    grid: &mut Grid,
    y: usize,
) -> Option<(usize, usize)> {
    match editor.mode {
        Mode::CmdLine => {
            let line = format!("{}{}", editor.cmdline_kind, editor.cmdline);
            let width = grid.width().max(1);
            // A command line too long for one row takes more, growing upward over the
            // screen with a blank row above it (Neovim's 'msgsep').
            let rows = UnicodeWidthStr::width(line.as_str()) / width + 1;
            let first = (y + 1).saturating_sub(rows);
            if rows > 1 {
                for r in first.saturating_sub(1)..=y {
                    grid.fill_row(r, Style::default());
                }
            }
            for (row, piece) in (first..).zip(flux_view::editor::wrap(&line, width)) {
                grid.put_str(0, row, &piece, Style::default());
            }
            let before: String = line.chars().take(editor.cmdline_pos + 1).collect();
            let x = UnicodeWidthStr::width(before.as_str());
            Some((x % width, (first + x / width).min(y)))
        }
        // In Visual mode a message (from a search) shows instead of the mode until the next
        // key.
        Mode::Visual if editor.message.is_some() => {
            let message = editor.message.as_ref()?;
            let style = theme.message(message);
            grid.put_str(0, y, &message.text, style);
            None
        }
        Mode::Insert if editor.completion.show_error && editor.message.is_some() => {
            let message = editor.message.as_ref()?;
            let end = grid.put_str(0, y, &message.text, theme.message(message));
            Some((end.min(grid.width() - 1), y))
        }
        Mode::Insert if editor.completion.submode.text.is_some() => {
            let submode = &editor.completion.submode;
            let mut end = grid.put_str(0, y, "--", theme.mode_msg);
            end = grid.put_str(
                end,
                y,
                submode.text.as_deref().unwrap_or(""),
                theme.mode_msg,
            );
            if let Some((extra, group)) = &submode.extra {
                end = grid.put_str(end, y, " ", theme.mode_msg);
                let style = group.map_or(theme.mode_msg, |g| theme.group(g));
                grid.put_str(end, y, extra, style);
            }
            None
        }
        Mode::Insert | Mode::Visual => {
            let mode = match (editor.mode, editor.visual.kind) {
                (Mode::Insert, _) => "-- INSERT --",
                (_, VisualKind::Char) if editor.completion.select => "-- SELECT --",
                (_, VisualKind::Line) if editor.completion.select => "-- SELECT LINE --",
                (_, VisualKind::Char) => "-- VISUAL --",
                (_, VisualKind::Line) => "-- VISUAL LINE --",
            };
            let end = grid.put_str(0, y, mode, theme.mode_msg);
            draw_recording(editor, theme, grid, end, y);
            None
        }
        Mode::Normal if editor.insert_pending => {
            let end = grid.put_str(0, y, "-- (insert) --", theme.mode_msg);
            draw_recording(editor, theme, grid, end, y);
            None
        }
        Mode::Normal if editor.message.is_none() => {
            draw_recording(editor, theme, grid, 0, y);
            None
        }
        Mode::Normal
            if editor
                .message
                .as_ref()
                .is_some_and(|m| m.kind == MessageKind::Question) =>
        {
            let text = &editor.message.as_ref()?.text;
            let end = grid.put_str(0, y, text, theme.question);
            Some((end.min(grid.width() - 1), y))
        }
        Mode::Normal => {
            if let Some(message) = &editor.message {
                let style = theme.message(message);
                // Room up to the showcmd column, like Vim's `msg_may_trunc`/`msg_strtrunc`.
                let room = grid.width().saturating_sub(12).max(1);
                let text = match message.kind {
                    MessageKind::Info => truncate_middle(&message.text, room),
                    MessageKind::File => truncate_start(&message.text, room),
                    MessageKind::Full
                    | MessageKind::Error
                    | MessageKind::Warning
                    | MessageKind::Question => message.text.clone(),
                };
                grid.put_str(0, y, &text, style);
            }
            None
        }
    }
}

/// Vim's `recording @a` after the mode message.
fn draw_recording(editor: &Editor, theme: &Theme, grid: &mut Grid, x: usize, y: usize) {
    if let Some(reg) = editor.recording {
        grid.put_str(x, y, &format!("recording @{reg}"), theme.mode_msg);
    }
}

/// Neovim's hit-enter prompt for a message longer than one line: a blank separator row, the
/// message, and the prompt, drawn over the bottom of the screen.
pub fn draw_hit_enter(editor: &Editor, grid: &mut Grid) -> Option<(usize, usize)> {
    let theme = Theme::new(editor);
    grid.default = theme.normal;
    hit_enter(editor, &theme, grid)
}

fn hit_enter(editor: &Editor, theme: &Theme, grid: &mut Grid) -> Option<(usize, usize)> {
    let message = editor.message.as_ref()?;
    let wrapped = editor.message_lines();
    let lines: Vec<&str> = wrapped.iter().map(String::as_str).collect();
    let height = grid.height();
    let style = theme.message(message);
    if let Some(top) = editor.more_top {
        // A page of a long message, then `-- More --` (or the prompt on the last page).
        let page = height.saturating_sub(1);
        let last_top = lines.len().saturating_sub(page);
        let top = top.min(last_top);
        for y in 0..height {
            grid.fill_row(y, Style::default());
        }
        for (i, line) in lines[top..(top + page).min(lines.len())].iter().enumerate() {
            grid.put_str(0, i, line, style);
        }
        let (prompt, prompt_style) = if top < last_top && editor.more_help {
            (
                "-- More -- SPACE/d/j: screen/page/line down, b/u/k: up, q: quit ",
                theme.more_msg,
            )
        } else if top < last_top {
            ("-- More --", theme.more_msg)
        } else {
            ("Press ENTER or type command to continue", theme.question)
        };
        let end = grid.put_str(0, height - 1, prompt, prompt_style);
        return Some((end.min(grid.width() - 1), height - 1));
    }
    let rows = (lines.len() + 2).min(height);
    let first = height - rows;
    for y in first..height {
        grid.fill_row(y, Style::default());
    }
    let shown = rows.saturating_sub(2);
    for (i, line) in lines[lines.len() - shown..].iter().enumerate() {
        grid.put_str(0, first + 1 + i, line, style);
    }
    let end = grid.put_str(
        0,
        height - 1,
        "Press ENTER or type command to continue",
        theme.question,
    );
    Some((end.min(grid.width() - 1), height - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(editor: &Editor) -> (Vec<String>, Option<(usize, usize)>) {
        let (w, h) = editor.screen_size();
        let mut grid = Grid::new(w, h);
        let cursor = draw(editor, "", &mut grid);
        let rows = (0..h)
            .map(|y| grid.row_text(y).trim_end().to_owned())
            .collect();
        (rows, cursor)
    }

    #[test]
    fn short_file_with_tildes_and_statusline() {
        let mut editor = Editor::new(30, 6);
        editor.set_text("one\n\ttwo\n");
        let (rows, cursor) = render(&editor);
        assert_eq!(
            rows,
            [
                "one",
                "        two",
                "~",
                "~",
                "[No Name]   1,1            All",
                "",
            ]
        );
        assert_eq!(cursor, Some((0, 0)));
    }

    #[test]
    fn ruler_shows_screen_column_on_tabs_and_empty_lines() {
        let mut editor = Editor::new(40, 5);
        editor.set_text("\tx\n\n");
        assert!(render(&editor).0[3].ends_with("1,1-8          All"));
        editor.window.cursor.line = 1;
        assert!(render(&editor).0[3].ends_with("2,0-1          All"));
    }

    #[test]
    fn wrapped_lines_and_lastline_marker() {
        let mut editor = Editor::new(10, 5);
        editor.set_text("abcdefghijklmn\n0123456789abcdefghijklmnopqrstuvwxyz\n");
        let (rows, _) = render(&editor);
        assert_eq!(rows[0], "abcdefghij");
        assert_eq!(rows[1], "klmn");
        assert_eq!(
            rows[2],
            "01234567@@@".chars().take(7).collect::<String>() + "@@@"
        );
    }

    #[test]
    fn percentage_and_cmdline() {
        let mut editor = Editor::new(40, 6);
        let text: String = (1..=100).map(|i| format!("{i}\n")).collect();
        editor.set_text(&text);
        editor.window.top = 50;
        editor.window.cursor.line = 50;
        assert_eq!(
            render(&editor).0[4],
            format!("{:<22}{:<14} 52%", "[No Name]", "51,1")
        );

        editor.mode = Mode::CmdLine;
        editor.cmdline = "q".into();
        editor.cmdline_pos = 1;
        let (rows, cursor) = render(&editor);
        assert_eq!(rows[5], ":q");
        assert_eq!(cursor, Some((2, 5)));
    }

    #[test]
    fn split_windows_have_separators_and_statuslines() {
        let mut editor = Editor::new(80, 24);
        editor.set_text("alpha 1\nalpha 2\n");
        editor.split(false, None);
        editor.split(true, None);
        let (rows, cursor) = render(&editor);
        assert_eq!(rows[0], format!("{:<40}│alpha 1", "alpha 1"));
        assert_eq!(rows[2], format!("{:<40}│~", "~"));
        // Side by side statuslines join below the separator.
        assert_eq!(
            rows[11],
            format!(
                "{:<22}{:<14} All {:<21}{:<14} All",
                "[No Name]", "1,1", "[No Name]", "1,1"
            )
        );
        assert_eq!(rows[12], "alpha 1");
        assert_eq!(rows[22], format!("{:<62}{:<14} All", "[No Name]", "1,1"));
        assert_eq!(cursor, Some((0, 0)));
    }

    #[test]
    fn separator_continues_past_a_statusline_above_another_window() {
        let mut editor = Editor::new(80, 24);
        editor.set_text("x\n");
        editor.split(true, None);
        editor.split(false, None);
        let (rows, _) = render(&editor);
        // The left column is split: its top statusline has the separator beside it.
        assert_eq!(rows[11].chars().nth(40), Some('│'));
        assert_eq!(rows[22].chars().nth(40), Some(' '));
    }

    #[test]
    fn narrow_statusline_keeps_its_end() {
        let mut editor = Editor::new(80, 24);
        editor.set_text("x\n");
        editor.split(true, Some(10));
        let (rows, _) = render(&editor);
        assert_eq!(&rows[22][..10], "<      All");
    }

    #[test]
    fn partly_shown_top_line_is_marked() {
        let mut editor = Editor::new(20, 6);
        editor.set_text(&format!("{}\n", "x".repeat(100)));
        editor.with_window(|w, m| w.set_cursor(0, 99, m));
        let (rows, cursor) = render(&editor);
        assert_eq!(rows[0], format!("<<<{}", "x".repeat(17)));
        assert_eq!(cursor, Some((19, 3)));
    }
}
