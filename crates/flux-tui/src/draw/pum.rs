//! The completion menu and its info window, drawn as Neovim's `pum_redraw` does.

use flux_core::GlyphKind;
use flux_view::Editor;
use flux_view::pum::{Pum, info_layout};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::Paint;
use crate::grid::{Grid, Style};
use crate::theme::Theme;

/// Draw the info window, then the menu over everything.
pub(super) fn draw(editor: &Editor, theme: &Theme, grid: &mut Grid) {
    let Some(pum) = &editor.completion.pum else {
        return;
    };
    draw_info(editor, theme, grid);
    draw_menu(pum, theme, grid);
}

fn draw_menu(pum: &Pum, theme: &Theme, grid: &mut Grid) {
    let pmenu = theme.group("Pmenu");
    // Each part's highlight combined over Pmenu, normal and selected.
    let norm = [
        pmenu,
        pmenu.combine(theme.group("PmenuKind")),
        pmenu.combine(theme.group("PmenuExtra")),
    ];
    let sel = [
        pmenu.combine(theme.group("PmenuSel")),
        pmenu.combine(theme.group("PmenuKindSel")),
        pmenu.combine(theme.group("PmenuExtraSel")),
    ];
    let match_hl = |selected: bool| {
        let (m, ms, base) = (
            theme.group("PmenuMatch"),
            theme.group("PmenuMatchSel"),
            if selected {
                theme.group("PmenuSel")
            } else {
                pmenu
            },
        );
        let top = if selected { ms } else { m };
        pmenu.combine(base.combine(m.combine(top)))
    };
    let highlight_matches = theme.group("PmenuMatchSel") != theme.group("PmenuSel")
        || theme.group("PmenuMatch") != pmenu;
    let size = pum.items.len();
    let scroll_range = size.saturating_sub(pum.height);
    let first = pum.first.min(scroll_range);
    let (mut thumb_pos, mut thumb_height) = (0, 1);
    if pum.scrollbar && scroll_range > 0 {
        thumb_height = (pum.height * pum.height / size).max(1);
        thumb_pos = (first * (pum.height - thumb_height) + scroll_range / 2) / scroll_range;
    }
    let widths = [pum.base_width, pum.kind_width, pum.extra_width];
    let width = pum.width;
    let left = pum.col;
    for i in 0..pum.height {
        let y = pum.row + i;
        let idx = first + i;
        let Some(item) = pum.items.get(idx) else {
            break;
        };
        let selected = pum.selected == Some(idx);
        let attrs = if selected { sel } else { norm };
        let raw = |j: usize| -> Style {
            match (j, selected) {
                (0, false) => pmenu,
                (0, true) => theme.group("PmenuSel"),
                _ => attrs[j],
            }
        };
        if left > 0 {
            grid.set(left - 1, y, " ", 1, attrs[0]);
        }
        let texts = [
            Some(item.abbr.as_str()),
            item.kind.as_deref(),
            item.menu.as_deref(),
        ];
        let user = [item.abbr_hl.as_deref(), item.kind_hl.as_deref()];
        let mut x = left;
        let mut total = 0;
        let mut truncated = false;
        let mut orig = attrs[0];
        for j in 0..3 {
            let mut attr = attrs[j];
            orig = attr;
            if j < 2
                && let Some(g) = user[j]
            {
                attr = attr.combine(theme.group(g));
            }
            let next_empty = j + 1 >= 3 || texts[j + 1].is_none();
            if let Some(text) = texts[j] {
                // Tabs show as two spaces.
                let segments: Vec<&str> = text.split('\t').collect();
                for (k, seg) in segments.iter().enumerate() {
                    let cells = seg.width();
                    let pad = if next_empty { 0 } else { 2 };
                    if width.saturating_sub(total) < cells + pad {
                        truncated = true;
                    }
                    let shown = if truncated {
                        let room = width.saturating_sub(total);
                        let mut used = 0;
                        let mut s = String::new();
                        for g in seg.graphemes(true) {
                            let w = g.width();
                            if used + w > room {
                                break;
                            }
                            used += w;
                            s.push_str(g);
                        }
                        s
                    } else {
                        seg.to_string()
                    };
                    let match_range = if j == 0 && highlight_matches {
                        match_cells(&shown, &pum.leader)
                    } else {
                        None
                    };
                    let mut cx = x;
                    for (ci, g) in shown.graphemes(true).enumerate() {
                        let w = g.width();
                        let mut style = attr;
                        if let Some((a, b)) = match_range
                            && ci >= a
                            && ci < b
                        {
                            style = match_hl(selected);
                            if let Some(g) = user[0] {
                                style = style.combine(theme.group(g));
                            }
                        }
                        grid.set(cx, y, g, w.clamp(1, 2) as u8, style);
                        cx += w;
                    }
                    x = cx;
                    if k + 1 < segments.len() {
                        grid.put_str(x, y, "  ", attr);
                        x += 2;
                        total += 2;
                    }
                }
            }
            let n = if j > 0 { widths[1] + 1 } else { 1 };
            if j == 2
                || (next_empty && (j == 1 || (j == 0 && texts[2].is_none())))
                || pum.base_width + n >= width
            {
                break;
            }
            let to = left + pum.base_width + n;
            while x < to {
                grid.set(x, y, " ", 1, orig);
                x += 1;
            }
            x = to;
            total = pum.base_width + n;
        }
        let right = left + width;
        while x < right {
            grid.set(x, y, " ", 1, orig);
            x += 1;
        }
        if truncated {
            grid.set(right - 1, y, ">", 1, raw(0));
        }
        if pum.scrollbar {
            let thumb = i >= thumb_pos && i < thumb_pos + thumb_height;
            let style = if thumb {
                theme.group("PmenuThumb")
            } else {
                theme.group("PmenuSbar")
            };
            grid.set(right, y, " ", 1, style);
        }
    }
}

/// The graphemes of `text` that show the leader: the first place it starts, ignoring case
/// (Neovim's `pum_compute_text_attrs`), as many as the leader has.
fn match_cells(text: &str, leader: &str) -> Option<(usize, usize)> {
    if leader.is_empty() || text.is_empty() {
        return None;
    }
    let lower: Vec<String> = text.graphemes(true).map(str::to_lowercase).collect();
    let lead = leader.to_lowercase();
    let n = leader.graphemes(true).count();
    (0..lower.len())
        .find(|&i| lower[i..].concat().starts_with(&lead))
        .map(|i| (i, i + n))
}

fn draw_info(editor: &Editor, theme: &Theme, grid: &mut Grid) {
    let Some(w) = editor.completion.info.as_ref().filter(|w| !w.hidden) else {
        return;
    };
    let Some(buffer) = editor.buffer(w.buffer) else {
        return;
    };
    let base = theme.normal_float;
    for r in 0..w.height {
        for x in 0..w.width {
            grid.set(w.col + x, w.row + r, " ", 1, base);
        }
    }
    let text = &buffer.text;
    let spans = match &buffer.syntax {
        Some(s) if editor.syntax_on && w.markdown => s.highlights(text, 0..text.line_count()),
        _ => Vec::new(),
    };
    let mut row = 0;
    for shown_line in editor.pum_info_lines() {
        if row >= w.height {
            break;
        }
        let layout = info_layout(&shown_line, &text.line_str(shown_line.line), w.width);
        let shown = layout.row_count().min(w.height - row);
        for (r, glyphs) in layout.rows.iter().take(shown).enumerate() {
            let mut x = w.col;
            for glyph in glyphs {
                let style = match glyph.kind {
                    GlyphKind::Text | GlyphKind::Tab => base,
                    GlyphKind::Special => base.combine(theme.special_key),
                    GlyphKind::Filler => base.combine(theme.non_text),
                };
                grid.set(x, w.row + row + r, &glyph.symbol, glyph.width, style);
                x += usize::from(glyph.width);
            }
        }
        let at = Paint {
            layout: &layout,
            left: w.col,
            width: w.width,
            row: w.row + row,
            rows: shown,
        };
        for span in spans.iter().filter(|s| s.line == shown_line.line) {
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
        row += shown;
    }
}
