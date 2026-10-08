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
    let prompt_end = grid.put_str_until(x, row, &picker.input, pmenu, col + w);
    let before: String = picker.input.chars().take(picker.pos).collect();
    // Same cursor as an untruncated prompt: label bytes plus the query width,
    // clamped to the window. The marker must not move it.
    let cursor = (col + label.len() + UnicodeWidthStr::width(before.as_str()))
        .min(col + w.saturating_sub(1));
    // `<listed>+` right-aligned, only in columns the label, query, and cursor
    // do not occupy. Dropped when it would cover them or exceed the window,
    // so a long query is never shifted or clipped to make room.
    if picker.truncated {
        let marker = format!("{}+", picker.entry_count());
        let marker_width = UnicodeWidthStr::width(marker.as_str());
        let occupied_end = prompt_end.max(cursor.saturating_add(1)).min(col + w);
        let room = (col + w).saturating_sub(occupied_end);
        if marker_width > 0 && marker_width <= room {
            grid.put_str_until(col + w - marker_width, row, &marker, pmenu, col + w);
        }
    }
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
    Some((cursor, row))
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

    #[test]
    fn picker_window_marks_a_truncated_list() {
        let mut editor = Editor::new(40, 10);
        editor.mode = Mode::CmdLine;
        editor.cmdline_kind = ':';
        let paths: Vec<PathBuf> = (0..5000)
            .map(|i| PathBuf::from(format!("f{i}.txt")))
            .collect();
        // "zzz" matches nothing, so the shown count is not the listed count.
        let mut full = Picker::files(paths.clone());
        full.set_query("zzz");
        let mut cut = Picker::files_truncated(paths, true);
        cut.set_query("zzz");
        assert_eq!(cut.entry_count(), 5000);
        let shown = cut.shown_count();
        assert_eq!(shown, 0);

        editor.picker = Some(full);
        let (_, cursor_full) = render(&editor);
        editor.picker = Some(cut);
        let (grid, cursor) = render(&editor);

        let (width, height) = editor.screen_size();
        let avail = height.saturating_sub(flux_view::CMDLINE_ROWS);
        let list_rows = shown.min(avail.saturating_sub(1));
        let h = list_rows + 1;
        let w = match width.saturating_sub(4) {
            n if n >= 10 => n,
            _ => width,
        };
        let col = width.saturating_sub(w) / 2;
        let row = avail.saturating_sub(h) / 2;
        let marker = "5000+";
        let start = col + w - marker.len();
        let theme = Theme::new(&editor);
        let pmenu = theme.group("Pmenu");
        for (i, ch) in marker.chars().enumerate() {
            let cell = grid.cell(start + i, row);
            assert_eq!(cell.symbol, ch.to_string());
            assert_eq!(cell.style, pmenu);
        }
        assert_eq!(grid.cell(start - 1, row).symbol, " ");
        assert_eq!(grid.cell(start - 1, row).style, pmenu);
        let label = "Files> ";
        for (i, ch) in label.chars().enumerate() {
            assert_eq!(grid.cell(col + i, row).symbol, ch.to_string());
            assert_eq!(grid.cell(col + i, row).style, pmenu);
        }
        assert_eq!(grid.cell(col + label.len(), row).symbol, "z");
        let expected = col + label.len() + "zzz".len();
        assert_eq!(cursor, Some((expected.min(col + w - 1), row)));
        assert_eq!(cursor, cursor_full);
    }

    #[test]
    fn picker_window_keeps_a_long_query_clear_of_the_truncation_marker() {
        let paths: Vec<PathBuf> = (0..5000)
            .map(|i| PathBuf::from(format!("f{i}.txt")))
            .collect();
        let render_query = |query: &str| {
            let mut editor = Editor::new(40, 10);
            editor.mode = Mode::CmdLine;
            editor.cmdline_kind = ':';
            let mut full = Picker::files(paths.clone());
            full.set_query(query);
            let mut cut = Picker::files_truncated(paths.clone(), true);
            cut.set_query(query);
            editor.picker = Some(full);
            let (grid_full, cursor_full) = render(&editor);
            editor.picker = Some(cut);
            let (grid_cut, cursor_cut) = render(&editor);
            (grid_full, cursor_full, grid_cut, cursor_cut)
        };

        // 40 columns, query longer than the prompt row: the marker is dropped.
        // Label, query text, and cursor match the untruncated picker.
        let long = "z".repeat(40);
        let (grid_full, cursor_full, grid_cut, cursor_cut) = render_query(&long);
        let row = cursor_cut.expect("cursor").1;
        assert_eq!(cursor_cut, cursor_full);
        assert_eq!(grid_cut.row_text(row), grid_full.row_text(row));
        assert!(!grid_cut.row_text(row).contains("5000+"));
        assert!(grid_cut.row_text(row).contains("Files> zzzz"));

        // Same width, short query: the marker fits to the right of the prompt.
        let (grid_full, cursor_full, grid_cut, cursor_cut) = render_query("ab");
        let row = cursor_cut.expect("cursor").1;
        let full_row = grid_full.row_text(row);
        let cut_row = grid_cut.row_text(row);
        let prompt = "Files> ab";
        let prompt_at = cut_row.find(prompt).expect("prompt");
        assert_eq!(cursor_cut, cursor_full);
        assert_eq!(&full_row[prompt_at..prompt_at + prompt.len()], prompt);
        assert!(cut_row.trim_end().ends_with("5000+"));
        let marker_at = cut_row.find("5000+").expect("marker");
        assert!(marker_at >= prompt_at + prompt.len());
        assert!(!full_row.contains("5000+"));
    }
}
