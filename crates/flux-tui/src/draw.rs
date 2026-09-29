//! Draws the editor into a grid: the window's text, the statusline and the command line.

use flux_core::{GlyphKind, LineLayout, layout_line};
use flux_view::{Editor, Mode};

use crate::grid::{Color, Grid, Style};

/// Vim's NonText and SpecialKey groups: `~` past the end, `@@@`, `>` fillers, `^X`.
const NON_TEXT: Style = Style::fg(Color::Ansi(8));
const ERROR: Style = Style::fg(Color::Ansi(9));
const STATUS_LINE: Style = Style {
    reverse: true,
    ..Style::fg(Color::Reset)
};

/// Draw `editor` into `grid`, returning where the terminal cursor should go.
pub fn draw(editor: &Editor, grid: &mut Grid) -> Option<(usize, usize)> {
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
        if line == win.cursor.line {
            let (r, x) = layout.cursor_position(win.cursor.col);
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

/// Neovim's default statusline: `%<%f %h%w%m%r%=%-14.(%l,%c%V%) %P`.
fn draw_statusline(editor: &Editor, grid: &mut Grid, y: usize) {
    grid.fill_row(y, STATUS_LINE);
    let buffer = editor.current_buffer();
    let ruler = format!("{:<14} {}", cursor_ruler(editor), relative_position(editor));
    let ruler_width = ruler.chars().count();
    let name_room = grid.width().saturating_sub(ruler_width + 1);
    let name = truncate_left(&buffer.name(), name_room);
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
    let line = editor.current_buffer().text.line_str(cursor.line);
    if line.is_empty() {
        return format!("{},0-1", cursor.line + 1);
    }
    let byte_col = line
        .char_indices()
        .nth(cursor.col)
        .map_or(line.len(), |(i, _)| i)
        + 1;
    let layout = layout_line(&line, editor.options.tabstop, None);
    let screen_col = layout.cursor_position(cursor.col).1 + 1;
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
        Mode::Normal => {
            if let Some(message) = &editor.message {
                let style = if message.is_error {
                    ERROR
                } else {
                    Style::default()
                };
                grid.put_str(0, y, &message.text, style);
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(editor: &Editor) -> (Vec<String>, Option<(usize, usize)>) {
        let (w, h) = editor.screen_size();
        let mut grid = Grid::new(w, h);
        let cursor = draw(editor, &mut grid);
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
