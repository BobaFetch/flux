//! The `:Files`/`:Buffers` picker window (flux-own, centered) and the command-line
//! wildmenu row (Neovim's `show_wildmenu`).

use flux_view::{Editor, PickerKind};
use unicode_width::UnicodeWidthStr;

use crate::grid::Grid;
use crate::theme::Theme;

/// Draw the picker window over the area above the command line; returns the
/// prompt cursor position.
pub(super) fn draw_picker(
    editor: &Editor,
    theme: &Theme,
    grid: &mut Grid,
) -> Option<(usize, usize)> {
    let picker = editor.picker.as_ref()?;
    let (width, height) = (grid.width(), grid.height());
    let avail = height.saturating_sub(flux_view::CMDLINE_ROWS);
    if width == 0 || avail == 0 {
        return None;
    }
    let pmenu = theme.group("Pmenu");
    let selected_style = pmenu.combine(theme.group("PmenuSel"));
    let list_rows = picker.shown_count().min(avail.saturating_sub(1));
    let h = list_rows + 1;
    let w = match width.saturating_sub(4) {
        w if w >= 10 => w,
        _ => width,
    };
    let col = width.saturating_sub(w) / 2;
    let row = avail.saturating_sub(h) / 2;
    for r in 0..h {
        for x in 0..w {
            grid.set(col + x, row + r, " ", 1, pmenu);
        }
    }
    let label = match picker.kind {
        PickerKind::Files => "Files> ",
        PickerKind::Buffers => "Buffers> ",
    };
    let x = grid.put_str_until(col, row, label, pmenu, col + w);
    grid.put_str_until(x, row, &picker.input, pmenu, col + w);
    // The selection stays visible: the first shown row follows it.
    let first = picker.selected.saturating_sub(list_rows.saturating_sub(1));
    let shown = picker.shown_entries();
    for i in 0..list_rows {
        let style = if first + i == picker.selected {
            selected_style
        } else {
            pmenu
        };
        if let Some(entry) = shown.get(first + i) {
            grid.put_str_until(col, row + 1 + i, &entry.text, style, col + w);
        }
    }
    let before: String = picker.input.chars().take(picker.pos).collect();
    let cursor = col + label.len() + UnicodeWidthStr::width(before.as_str());
    Some((cursor.min(col + w.saturating_sub(1)), row))
}

/// Draw the wildmenu row at grid row `y`: the matches across the whole row in
/// StatusLine, the selected one in WildMenu.
pub(super) fn draw_wildmenu(editor: &Editor, theme: &Theme, grid: &mut Grid, y: usize) {
    let Some(wild) = &editor.wildmenu else {
        return;
    };
    if editor.picker.is_some() {
        return;
    }
    let width = grid.width();
    if width == 0 || y >= grid.height() {
        return;
    }
    let status = theme.status_line;
    let selected_style = status.combine(theme.group("WildMenu"));
    grid.fill_row(y, status);
    let items = &wild.items;
    if items.is_empty() {
        return;
    }
    let widths: Vec<usize> = items
        .iter()
        .map(|s| UnicodeWidthStr::width(s.as_str()))
        .collect();
    // The window around the selection that fits: right first, then left.
    let sel = wild.selected.unwrap_or(0).min(items.len() - 1);
    let (mut start, mut end) = (sel, sel);
    let mut used = widths[sel];
    while end + 1 < items.len() && used + 2 + widths[end + 1] <= width {
        end += 1;
        used += 2 + widths[end];
    }
    while start > 0 && used + 2 + widths[start - 1] <= width {
        start -= 1;
        used += 2 + widths[start];
    }
    let mut x = 0;
    for (i, item) in items.iter().enumerate().skip(start).take(end - start + 1) {
        let style = if wild.selected == Some(i) {
            selected_style
        } else {
            status
        };
        x = grid.put_str(x, y, item, style);
        if i < end {
            x = grid.put_str(x, y, "  ", status);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use flux_view::{Mode, Picker, Wildmenu};

    use super::*;

    fn render(editor: &Editor) -> (Grid, Option<(usize, usize)>) {
        let (w, h) = editor.screen_size();
        let mut grid = Grid::new(w, h);
        let cursor = super::super::draw(editor, "", &mut grid);
        (grid, cursor)
    }

    #[test]
    fn wildmenu_row_marks_the_selection() {
        let mut editor = Editor::new(40, 8);
        editor.mode = Mode::CmdLine;
        editor.cmdline_kind = ':';
        editor.cmdline = "set".to_string();
        editor.cmdline_pos = 3;
        editor.wildmenu = Some(Wildmenu {
            items: ["set", "setglobal", "setlocal"]
                .map(str::to_string)
                .to_vec(),
            selected: Some(1),
            original: "se".to_string(),
            start: 0,
        });
        let (grid, cursor) = render(&editor);
        assert_eq!(grid.row_text(6).trim_end(), "set  setglobal  setlocal");
        assert_eq!(grid.row_text(7).trim_end(), ":set");
        assert_eq!(cursor, Some((4, 7)));
        let theme = Theme::new(&editor);
        assert_eq!(grid.cell(0, 6).style, theme.status_line);
        assert_eq!(
            grid.cell(5, 6).style,
            theme.status_line.combine(theme.group("WildMenu"))
        );
    }

    #[test]
    fn picker_window_shows_prompt_and_matches() {
        let mut editor = Editor::new(40, 10);
        editor.mode = Mode::CmdLine;
        editor.cmdline_kind = ':';
        let mut picker = Picker::files(
            ["a.txt", "b.txt", "sub/c.txt"]
                .into_iter()
                .map(PathBuf::from)
                .collect(),
        );
        picker.set_query("b");
        editor.picker = Some(picker);
        let (grid, cursor) = render(&editor);
        // Centered over the 9 rows above the command line: prompt + 2 matches
        // (column 0 keeps the buffer's `~` outside the window).
        assert_eq!(grid.row_text(3).trim_end(), "~ Files> b");
        assert_eq!(grid.row_text(4).trim_end(), "~ b.txt");
        assert_eq!(grid.row_text(5).trim_end(), "~ sub/c.txt");
        assert_eq!(cursor, Some((10, 3)));
        let theme = Theme::new(&editor);
        assert_eq!(grid.cell(2, 3).style, theme.group("Pmenu"));
        assert_eq!(
            grid.cell(2, 4).style,
            theme.group("Pmenu").combine(theme.group("PmenuSel"))
        );
        assert_eq!(grid.cell(2, 5).style, theme.group("Pmenu"));
    }
}
