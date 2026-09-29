//! Draws the editor into a grid: the window's text, the statusline and the command line.

use flux_core::{GlyphKind, LineLayout, layout_line};
use flux_view::{
    Buffer, CMDLINE_ROWS, Cursor, Editor, MessageKind, Mode, Rect, VisualKind, Window,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::grid::{Color, Grid, Style};

/// Vim's NonText and SpecialKey groups: `~` past the end, `@@@`, `>` fillers, `^X`.
const NON_TEXT: Style = Style::fg(Color::Ansi(8));
const ERROR: Style = Style::fg(Color::Ansi(9));
/// The Visual selection: a grey background that works on light and dark terminals.
const VISUAL: Style = Style {
    bg: Color::Ansi(8),
    ..Style::fg(Color::Reset)
};
/// Vim's StatusLine (the current window) and StatusLineNC (the others).
const STATUS_LINE: Style = Style {
    reverse: true,
    bold: true,
    ..Style::fg(Color::Reset)
};
const STATUS_LINE_NC: Style = Style {
    reverse: true,
    ..Style::fg(Color::Reset)
};
/// Vim's WinSeparator.
const SEPARATOR: Style = Style::fg(Color::Reset);

/// Vim's MoreMsg highlight.
const MORE_MSG: Style = Style {
    bold: true,
    ..Style::fg(Color::Ansi(10))
};
const MODE_MSG: Style = Style {
    bold: true,
    ..Style::fg(Color::Reset)
};

/// Draw `editor` into `grid`, returning where the terminal cursor should go. `showcmd` is a
/// partly typed command, shown at the bottom right as Vim's 'showcmd' does.
pub fn draw(editor: &Editor, showcmd: &str, grid: &mut Grid) -> Option<(usize, usize)> {
    let (width, height) = (grid.width(), grid.height());
    if width == 0 || height == 0 {
        return None;
    }
    let mut cursor = None;
    for (id, rect) in editor.window_rects() {
        let pane = Pane {
            editor,
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
    if let Some(pos) = draw_cmdline(editor, grid, height - 1) {
        cursor = Some(pos);
    }
    if editor.hit_enter
        && let Some(pos) = draw_hit_enter(editor, grid)
    {
        return Some(pos);
    }
    let size;
    let showcmd = if showcmd.is_empty() && editor.mode == Mode::Visual {
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
    win: &'a Window,
    rect: Rect,
}

impl Pane<'_> {
    fn is_current(&self) -> bool {
        self.win.id == self.editor.window.id
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
        let text = &self.buffer().text;
        let (top_row, left) = (self.rect.row, self.rect.col);
        let text_rows = self.text_rows(grid);
        let width = self.rect.width;
        let insert = self.is_current() && self.editor.mode == Mode::Insert;
        let mut cursor = None;
        let mut row = 0;
        let mut line = win.top;
        while row < text_rows && line < text.line_count() {
            let mut layout = layout_line(
                &text.line_str(line),
                self.editor.options.tabstop,
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
                draw_rows(grid, &layout, left, top_row + row, text_rows - row);
                for x in width.saturating_sub(3)..width {
                    grid.set(left + x, top_row + text_rows - 1, "@", 1, NON_TEXT);
                }
                row = text_rows;
                break;
            }
            let shown = layout.row_count().min(text_rows - row);
            draw_rows(grid, &layout, left, top_row + row, shown);
            if skip > 0 {
                for x in 0..3.min(width) {
                    grid.set(left + x, top_row + row, "<", 1, NON_TEXT);
                }
            }
            if let Some((from, to)) = self.selected_columns(line) {
                highlight(grid, &layout, left, width, top_row + row, shown, from, to);
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
        for r in row..text_rows {
            grid.set(left, top_row + r, "~", 1, NON_TEXT);
        }
        cursor
    }

    /// The selected chars of `line` as `[from, to)`, with `to` past the end when the line break
    /// is selected too. Like Vim, every window showing the current buffer shows the selection.
    fn selected_columns(&self, line: usize) -> Option<(usize, usize)> {
        let editor = self.editor;
        if editor.mode != Mode::Visual || self.win.buffer != editor.window.buffer {
            return None;
        }
        let a = editor.visual.anchor;
        let c = editor.window.cursor;
        let (start, end) = if (c.line, c.col) < (a.line, a.col) {
            (c, a)
        } else {
            (a, c)
        };
        if line < start.line || line > end.line {
            return None;
        }
        let len = editor.current_buffer().text.line_len(line);
        if editor.visual.kind == VisualKind::Line {
            return Some((0, len.max(1)));
        }
        let from = if line == start.line { start.col } else { 0 };
        let eol = editor.window.curswant == usize::MAX && c == end;
        let to = if line == end.line && !eol {
            end.col + 1
        } else {
            len + 1
        };
        Some((from, to))
    }

    /// Neovim's default statusline: `%<%f %h%w%m%r %=%-14.(%l,%c%V%) %P`. The cell below a
    /// vertical separator belongs to the statusline when another one continues to its right.
    fn draw_statusline(&self, grid: &mut Grid) {
        let y = self.rect.row + self.rect.height;
        if y >= grid.height().saturating_sub(CMDLINE_ROWS) {
            return;
        }
        let style = if self.is_current() {
            STATUS_LINE
        } else {
            STATUS_LINE_NC
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
                grid.set(left + width, y, "│", 1, SEPARATOR);
            }
        }
        let buffer = self.buffer();
        let ruler = format!("{:<14} {}", self.cursor_ruler(), self.relative_position());
        // `%f %h%w%m%r `: the name, a space, the flags and another space.
        let flags = if buffer.modified() { "[+]" } else { "" };
        let name = format!("{} {flags} ", buffer.name());
        let (name_width, ruler_width) = (
            UnicodeWidthStr::width(name.as_str()),
            UnicodeWidthStr::width(ruler.as_str()),
        );
        if name_width + ruler_width <= width {
            // `%=` pushes the ruler to the right edge.
            grid.put_str_until(left, y, &name, style, left + width);
            grid.put_str_until(left + width - ruler_width, y, &ruler, style, left + width);
        } else {
            // Too wide: `%<` at the start cuts the front off everything, marked with `<`.
            let text = truncate_left(&format!("{name}{ruler}"), width);
            grid.put_str_until(left, y, &text, style, left + width);
        }
    }

    /// The `│` column to the right of the window.
    fn draw_separator(&self, grid: &mut Grid) {
        let x = self.rect.col + self.rect.width;
        for r in 0..self.text_rows(grid) {
            grid.set(x, self.rect.row + r, "│", 1, SEPARATOR);
        }
    }

    /// `%l,%c%V`: line, byte column and, when different, screen column. An empty line is `0-1`.
    fn cursor_ruler(&self) -> String {
        let cursor = self.win.cursor;
        let text = &self.buffer().text;
        let line = text.line_str(cursor.line.min(text.line_count().saturating_sub(1)));
        if line.is_empty() {
            // A buffer with no lines at all shows line 0.
            let n = if text.has_no_lines() {
                0
            } else {
                cursor.line + 1
            };
            return format!("{n},0-1");
        }
        let byte_col = line
            .char_indices()
            .nth(cursor.col)
            .map_or(line.len(), |(i, _)| i)
            + 1;
        let layout = layout_line(&line, self.editor.options.tabstop, None);
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
        let buffer = self.buffer();
        let win = self.win;
        let metrics = flux_view::Metrics {
            text: &buffer.text,
            tabstop: self.editor.options.tabstop,
            width: win.width,
            height: win.height,
        };
        let above = win.top;
        let below = buffer
            .text
            .line_count()
            .saturating_sub(win.bottom(&metrics) + 1);
        if below == 0 {
            if above == 0 { "All" } else { "Bot" }.to_string()
        } else if above == 0 {
            "Top".to_string()
        } else {
            format!("{:>2}%", above * 100 / (above + below))
        }
    }
}

fn draw_rows(grid: &mut Grid, layout: &LineLayout, left: usize, first_row: usize, count: usize) {
    for (r, glyphs) in layout.rows.iter().take(count).enumerate() {
        let mut x = left;
        for glyph in glyphs {
            let style = match glyph.kind {
                GlyphKind::Text | GlyphKind::Tab => Style::default(),
                GlyphKind::Special | GlyphKind::Filler => NON_TEXT,
            };
            grid.set(x, first_row + r, &glyph.symbol, glyph.width, style);
            x += usize::from(glyph.width);
        }
    }
}

/// Paint the selection's background over already drawn glyphs; a selected line break shows as
/// one highlighted cell after the text.
#[allow(clippy::too_many_arguments)]
fn highlight(
    grid: &mut Grid,
    layout: &LineLayout,
    left: usize,
    width: usize,
    first_row: usize,
    rows: usize,
    from: usize,
    to: usize,
) {
    let mut end_cell = (first_row, 0);
    for (r, glyphs) in layout.rows.iter().take(rows).enumerate() {
        let mut x = 0;
        for glyph in glyphs {
            if glyph.char_idx >= from && glyph.char_idx < to {
                let cell = grid.cell(left + x, first_row + r).clone();
                if cell.width > 0 {
                    let style = Style {
                        bg: VISUAL.bg,
                        ..cell.style
                    };
                    grid.set(left + x, first_row + r, &cell.symbol, cell.width, style);
                }
            }
            x += usize::from(glyph.width);
        }
        end_cell = (first_row + r, x);
    }
    let len = layout
        .rows
        .iter()
        .flatten()
        .map(|g| g.char_idx + 1)
        .max()
        .unwrap_or(0);
    if to > len && from <= len && end_cell.1 < width {
        grid.set(left + end_cell.1, end_cell.0, " ", 1, VISUAL);
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

/// Keep the end of `s`, as Vim's `%<` does, marking the cut with `<`.
fn truncate_left(s: &str, room: usize) -> String {
    if UnicodeWidthStr::width(s) <= room {
        return s.to_owned();
    }
    if room == 0 {
        return String::new();
    }
    let mut tail: Vec<&str> = Vec::new();
    let mut used = 1;
    for g in s.graphemes(true).rev() {
        let w = UnicodeWidthStr::width(g);
        if used + w > room {
            break;
        }
        tail.push(g);
        used += w;
    }
    tail.reverse();
    format!("<{}", tail.concat())
}

fn draw_cmdline(editor: &Editor, grid: &mut Grid, y: usize) -> Option<(usize, usize)> {
    match editor.mode {
        Mode::CmdLine => {
            let end = grid.put_str(0, y, &format!(":{}", editor.cmdline), Style::default());
            Some((end.min(grid.width() - 1), y))
        }
        Mode::Insert | Mode::Visual => {
            let mode = match (editor.mode, editor.visual.kind) {
                (Mode::Insert, _) => "-- INSERT --",
                (_, VisualKind::Char) => "-- VISUAL --",
                (_, VisualKind::Line) => "-- VISUAL LINE --",
            };
            let end = grid.put_str(0, y, mode, MODE_MSG);
            draw_recording(editor, grid, end, y);
            None
        }
        Mode::Normal if editor.insert_pending => {
            let end = grid.put_str(0, y, "-- (insert) --", MODE_MSG);
            draw_recording(editor, grid, end, y);
            None
        }
        Mode::Normal if editor.message.is_none() => {
            draw_recording(editor, grid, 0, y);
            None
        }
        Mode::Normal => {
            if let Some(message) = &editor.message {
                let style = if message.is_error() {
                    ERROR
                } else {
                    Style::default()
                };
                // Room up to the showcmd column, like Vim's `msg_may_trunc`/`msg_strtrunc`.
                let room = grid.width().saturating_sub(12).max(1);
                let text = match message.kind {
                    MessageKind::Info => truncate_middle(&message.text, room),
                    MessageKind::File => truncate_start(&message.text, room),
                    MessageKind::Full | MessageKind::Error => message.text.clone(),
                };
                grid.put_str(0, y, &text, style);
            }
            None
        }
    }
}

/// Vim's `recording @a` after the mode message.
fn draw_recording(editor: &Editor, grid: &mut Grid, x: usize, y: usize) {
    if let Some(reg) = editor.recording {
        grid.put_str(x, y, &format!("recording @{reg}"), MODE_MSG);
    }
}

/// Neovim's hit-enter prompt for a message longer than one line: a blank separator row, the
/// message, and the prompt, drawn over the bottom of the screen.
fn draw_hit_enter(editor: &Editor, grid: &mut Grid) -> Option<(usize, usize)> {
    let message = editor.message.as_ref()?;
    let width = grid.width().max(1);
    let wrapped: Vec<String> = message
        .text
        .lines()
        .flat_map(|line| wrap(line, width))
        .collect();
    let lines: Vec<&str> = wrapped.iter().map(String::as_str).collect();
    let height = grid.height();
    let rows = (lines.len() + 2).min(height);
    let first = height - rows;
    for y in first..height {
        grid.fill_row(y, Style::default());
    }
    let style = if message.is_error() {
        ERROR
    } else {
        Style::default()
    };
    let shown = rows.saturating_sub(2);
    for (i, line) in lines[lines.len() - shown..].iter().enumerate() {
        grid.put_str(0, first + 1 + i, line, style);
    }
    let end = grid.put_str(
        0,
        height - 1,
        "Press ENTER or type command to continue",
        MORE_MSG,
    );
    Some((end.min(grid.width() - 1), height - 1))
}

/// Split `line` into pieces at most `width` cells wide.
fn wrap(line: &str, width: usize) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut used = 0;
    for g in line.graphemes(true) {
        let w = UnicodeWidthStr::width(g);
        if used + w > width {
            out.push(String::new());
            used = 0;
        }
        out.last_mut().unwrap().push_str(g);
        used += w;
    }
    out
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

    #[test]
    fn long_names_keep_their_end() {
        assert_eq!(truncate_left("src/very/long/path.rs", 10), "<g/path.rs");
        assert_eq!(truncate_left("short", 10), "short");
    }
}
