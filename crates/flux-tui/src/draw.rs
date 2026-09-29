//! Draws the editor into a grid: the window's text, the statusline and the command line.

use flux_core::{GlyphKind, LineLayout, layout_line};
use flux_view::{Cursor, Editor, MessageKind, Mode, VisualKind};
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
const STATUS_LINE: Style = Style {
    reverse: true,
    ..Style::fg(Color::Reset)
};

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
    let text_rows = editor.window.height.min(height);
    let mut cursor = draw_text(editor, grid, text_rows);
    if height >= 2 {
        draw_statusline(editor, grid, height - 2);
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

fn draw_text(editor: &Editor, grid: &mut Grid, text_rows: usize) -> Option<(usize, usize)> {
    let win = &editor.window;
    let text = &editor.current_buffer().text;
    let mut cursor = None;
    let mut row = 0;
    let mut line = win.top;
    while row < text_rows && line < text.line_count() {
        let layout = layout_line(
            &text.line_str(line),
            editor.options.tabstop,
            Some(win.width),
        );
        let fits = row + layout.row_count() <= text_rows;
        if !fits && line != win.top {
            // Vim's `display=lastline`: show what fits and mark the cut with `@@@`.
            draw_rows(grid, &layout, row, text_rows - row);
            for x in grid.width().saturating_sub(3)..grid.width() {
                grid.set(x, text_rows - 1, "@", 1, NON_TEXT);
            }
            row = text_rows;
            break;
        }
        let shown = layout.row_count().min(text_rows - row);
        draw_rows(grid, &layout, row, shown);
        if let Some((from, to)) = selected_columns(editor, line) {
            highlight(grid, &layout, row, shown, from, to);
        }
        if line == win.cursor.line {
            let (r, x) = layout.cursor_position(win.cursor.col, editor.mode == Mode::Insert);
            if r < shown {
                cursor = Some((x.min(grid.width() - 1), row + r));
            }
        }
        row += shown;
        line += 1;
    }
    for r in row..text_rows {
        grid.set(0, r, "~", 1, NON_TEXT);
    }
    cursor
}

fn draw_rows(grid: &mut Grid, layout: &LineLayout, first_row: usize, count: usize) {
    for (r, glyphs) in layout.rows.iter().take(count).enumerate() {
        let mut x = 0;
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

/// The selected chars of `line` as `[from, to)`, with `to` past the end when the line break
/// is selected too.
fn selected_columns(editor: &Editor, line: usize) -> Option<(usize, usize)> {
    if editor.mode != Mode::Visual {
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

/// Paint the selection's background over already drawn glyphs; a selected line break shows as
/// one highlighted cell after the text.
fn highlight(
    grid: &mut Grid,
    layout: &LineLayout,
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
                let cell = grid.cell(x, first_row + r).clone();
                if cell.width > 0 {
                    let style = Style {
                        bg: VISUAL.bg,
                        ..cell.style
                    };
                    grid.set(x, first_row + r, &cell.symbol, cell.width, style);
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
    if to > len && from <= len && end_cell.1 < grid.width() {
        grid.set(end_cell.1, end_cell.0, " ", 1, VISUAL);
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

/// Neovim's default statusline: `%<%f %h%w%m%r%=%-14.(%l,%c%V%) %P`.
fn draw_statusline(editor: &Editor, grid: &mut Grid, y: usize) {
    grid.fill_row(y, STATUS_LINE);
    let buffer = editor.current_buffer();
    let ruler = format!("{:<14} {}", cursor_ruler(editor), relative_position(editor));
    let ruler_width = ruler.chars().count();
    let name_room = grid.width().saturating_sub(ruler_width + 1);
    // `%<%f %h%w%m%r`: the name, a space, then flags; truncated from the start as one piece.
    let flags = if buffer.modified() { "[+]" } else { "" };
    let name = truncate_left(&format!("{} {flags}", buffer.name()), name_room);
    grid.put_str(0, y, &name, STATUS_LINE);
    grid.put_str(
        grid.width().saturating_sub(ruler_width),
        y,
        &ruler,
        STATUS_LINE,
    );
}

/// `%l,%c%V`: line, byte column and, when different, screen column. An empty line is `0-1`.
fn cursor_ruler(editor: &Editor) -> String {
    let cursor = editor.window.cursor;
    let text = &editor.current_buffer().text;
    let line = text.line_str(cursor.line);
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
    let layout = layout_line(&line, editor.options.tabstop, None);
    let screen_col = layout
        .cursor_position(cursor.col, editor.mode == Mode::Insert)
        .1
        + 1;
    if screen_col == byte_col {
        format!("{},{byte_col}", cursor.line + 1)
    } else {
        format!("{},{byte_col}-{screen_col}", cursor.line + 1)
    }
}

/// `%P`: `All`, `Top`, `Bot`, or how far down the window is.
fn relative_position(editor: &Editor) -> String {
    let buffer = editor.current_buffer();
    let win = &editor.window;
    let metrics = flux_view::Metrics {
        text: &buffer.text,
        tabstop: editor.options.tabstop,
        width: win.width,
        height: win.height,
    };
    let above = win.top;
    let below = buffer.text.line_count() - win.bottom(&metrics) - 1;
    if below == 0 {
        if above == 0 { "All" } else { "Bot" }.to_string()
    } else if above == 0 {
        "Top".to_string()
    } else {
        format!("{:>2}%", above * 100 / (above + below))
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
    let len = s.chars().count();
    if len <= room {
        return s.to_owned();
    }
    if room == 0 {
        return String::new();
    }
    let tail: String = s.chars().skip(len - room + 1).collect();
    format!("<{tail}")
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
    fn long_names_keep_their_end() {
        assert_eq!(truncate_left("src/very/long/path.rs", 10), "<g/path.rs");
        assert_eq!(truncate_left("short", 10), "short");
    }
}
