//! The Insert-mode completion popup menu, placed and scrolled as Neovim's `popupmenu.c` does,
//! with its info window ('completeopt' `popup`), and the completion mode message. What goes in
//! the menu is decided by `flux_vim::completion`.

use unicode_width::UnicodeWidthStr;

use crate::float::DisplayLine;
use crate::{Buffer, BufferId, Cursor, Editor};

/// Neovim's `PUM_DEF_HEIGHT`.
const DEF_HEIGHT: usize = 10;

/// One line of the menu: Vim's `abbr`, `kind` and `menu` of a match.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PumItem {
    pub abbr: String,
    pub kind: Option<String>,
    pub menu: Option<String>,
    /// Text for the info window.
    pub info: Option<String>,
    /// Highlight groups combined into the abbreviation and the kind (`abbr_hlgroup`,
    /// `kind_hlgroup`).
    pub abbr_hl: Option<String>,
    pub kind_hl: Option<String>,
}

/// The menu on screen.
#[derive(Debug, Clone, Default)]
pub struct Pum {
    pub items: Vec<PumItem>,
    pub selected: Option<usize>,
    /// The text completed so far: where items contain it, it's shown in PmenuMatch.
    pub leader: String,
    /// The top row and the column of the first item's text (a space is drawn before it when
    /// there's room).
    pub row: usize,
    pub col: usize,
    /// Columns for the text, not counting that space or the scrollbar.
    pub width: usize,
    pub height: usize,
    /// The first item shown.
    pub first: usize,
    pub scrollbar: bool,
    pub above: bool,
    /// The widest abbreviation, kind and menu text (each but the first with a space before it).
    pub base_width: usize,
    pub kind_width: usize,
    pub extra_width: usize,
}

/// The info window next to the menu: the selected item's `info`, in a scratch buffer.
#[derive(Debug, Clone)]
pub struct InfoWindow {
    pub buffer: BufferId,
    pub row: usize,
    pub col: usize,
    pub width: usize,
    pub height: usize,
    pub hidden: bool,
    /// Shown as Markdown: concealed, with its syntax (Neovim's `update_popup_window`).
    pub markdown: bool,
    /// The height is to be fitted to the text as shown, once its syntax is known.
    pub fit: bool,
}

/// The completion mode message: `-- Omni completion (^O^N^P)` and what follows it (`match 1 of
/// 5` in its highlight group).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Submode {
    pub text: Option<String>,
    pub extra: Option<(String, Option<&'static str>)>,
}

#[derive(Debug, Clone, Default)]
pub struct CompletionView {
    pub pum: Option<Pum>,
    pub info: Option<InfoWindow>,
    /// Shown instead of `-- INSERT --` while completing (Vim's `edit_submode`).
    pub submode: Submode,
    /// The cursor the ruler shows, when it isn't the real one: opening the info window for a
    /// selected item redraws the statusline with the cursor at the start of the completed
    /// text, and Neovim doesn't redraw it again until the cursor moves.
    pub ruler_cursor: Option<Cursor>,
    /// An error from starting completion shows instead of the mode until the next key, with
    /// the cursor after it (as Neovim leaves it after an error in Insert mode).
    pub show_error: bool,
    /// What the statusline showed when last checked (Neovim's `w_stl_cursor` and friends).
    stl: Option<(Cursor, usize, usize, crate::Mode)>,
}

/// The display width of the item texts.
fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

impl Editor {
    /// Whether 'completeopt' has `flag`.
    pub fn completeopt(&self, flag: &str) -> bool {
        self.options.completeopt.split(',').any(|f| f == flag)
    }

    /// Show the menu with `items` (or with the items shown, when `None`), `selected` selected,
    /// below or above the text being completed that starts at column `start` of the cursor
    /// line (Neovim's `pum_display`). The menu isn't shown when there's no room.
    pub fn pum_display(
        &mut self,
        items: Option<Vec<PumItem>>,
        selected: Option<usize>,
        leader: &str,
        start: usize,
    ) {
        let mut pum = match (items, self.completion.pum.take()) {
            (Some(items), Some(old)) => Pum { items, ..old },
            (Some(items), None) => Pum {
                items,
                ..Pum::default()
            },
            (None, Some(old)) => old,
            (None, None) => return,
        };
        pum.leader = leader.to_string();
        let Some(rect) = self
            .window_rects()
            .into_iter()
            .find(|(id, _)| *id == self.window.id)
            .map(|(_, r)| r)
        else {
            return;
        };
        // Where the completed text starts on screen, and the rows of its line.
        let m = self.metrics();
        let mut win = self.window.clone();
        win.cursor.col = start;
        let (wrow, vcol) = win.cursor_screen_offset(&m, true);
        let layout = flux_core::layout_line(
            &m.text.line_str(win.cursor.line),
            m.tabstop,
            Some(m.width.max(1)),
        );
        let (row_in_line, _) = layout.cursor_position(start, true);
        let cline_row = wrow.saturating_sub(row_in_line);
        let cline_height = layout.row_count();
        let gutter = rect.width.saturating_sub(self.window.width);
        let pum_win_row = rect.row + wrow;
        let cursor_col = rect.col + gutter + vcol;
        let (columns, rows) = self.screen_size();
        let below_row = rows.saturating_sub(1);
        let above_row = 0;
        let ph = self.options.pumheight;
        let pw = self.options.pumwidth;
        let size = pum.items.len();

        // Vertical placement (`pum_compute_vertical_placement`).
        let mut height = size.min(DEF_HEIGHT);
        if ph > 0 && height > ph {
            height = ph;
        }
        let row;
        if pum_win_row + 2 >= below_row.saturating_sub(height)
            && pum_win_row - above_row > (below_row - above_row) / 2
        {
            pum.above = true;
            let context = 2.min(wrow - cline_row);
            let mut r;
            if pum_win_row >= size + context {
                r = pum_win_row - size - context;
                height = size;
            } else {
                r = 0;
                height = pum_win_row.saturating_sub(context);
            }
            if ph > 0 && height > ph {
                r += height - ph;
                height = ph;
            }
            row = r;
        } else {
            pum.above = false;
            let context = 3.min((cline_row + cline_height).saturating_sub(wrow));
            row = pum_win_row + context;
            height = below_row.saturating_sub(row).min(size);
            if ph > 0 && height > ph {
                height = ph;
            }
        }
        // No room for more than one line of several items: no menu.
        if height < 1 || (height == 1 && size > 1) {
            self.pum_undisplay();
            return;
        }
        pum.row = row;
        pum.height = height;

        // The widths (`pum_compute_size`).
        pum.base_width = pum.items.iter().map(|i| width(&i.abbr)).max().unwrap_or(0);
        pum.kind_width = pum
            .items
            .iter()
            .filter_map(|i| i.kind.as_deref())
            .map(|k| width(k) + 1)
            .max()
            .unwrap_or(0);
        pum.extra_width = pum
            .items
            .iter()
            .filter_map(|i| i.menu.as_deref())
            .map(|k| width(k) + 1)
            .max()
            .unwrap_or(0);
        pum.scrollbar = height < size;
        let sb = usize::from(pum.scrollbar);

        // Horizontal placement (`pum_compute_horizontal_placement`).
        let desired = pum.base_width + pum.kind_width + pum.extra_width;
        let mut available = columns.saturating_sub(cursor_col + sb);
        pum.col = cursor_col;
        let (mut w, mut end_padding) = (desired, true);
        if w < pw {
            w = pw;
            end_padding = false;
        }
        let aligned = w + usize::from(end_padding && w >= pw);
        if available >= aligned {
            pum.width = aligned;
        } else if available > pw {
            pum.width = available;
        } else {
            available += cursor_col;
            if available > pw {
                pum.width = pw + 1;
                pum.col = columns.saturating_sub(pum.width + sb);
            } else {
                pum.col = 0;
                pum.width = columns.saturating_sub(sb);
            }
        }
        self.completion.pum = Some(pum);
        self.pum_set_selected(selected, start);
    }

    /// Select item `n` (none: the original text), scrolling the menu to show it with some
    /// context, and show its info (Neovim's `pum_set_selected`).
    fn pum_set_selected(&mut self, n: Option<usize>, start: usize) {
        let popup = self.completeopt("popup");
        let Some(pum) = self.completion.pum.as_mut() else {
            return;
        };
        pum.selected = n;
        let info = n
            .and_then(|n| pum.items.get(n))
            .and_then(|i| i.info.clone());
        if popup
            && info.is_none()
            && let Some(w) = self.completion.info.as_mut()
        {
            w.hidden = true;
        }
        let Some(sel) = n.filter(|&n| n < pum.items.len()) else {
            return;
        };
        let (height, size) = (pum.height as isize, pum.items.len() as isize);
        let (sel, mut first) = (sel as isize, pum.first as isize);
        let scroll_offset = sel - height;
        if first > sel - 4 {
            // Scroll down; when jumping it's probably a PageUp, scroll a page.
            if first > sel - 2 {
                first -= height - 2;
                if first < 0 {
                    first = 0;
                } else if first > sel {
                    first = sel;
                }
            } else {
                first = sel;
            }
        } else if first < scroll_offset + 5 {
            // Scroll up.
            if first < scroll_offset + 3 {
                first = (first + height - 2).max(scroll_offset + 1);
            } else {
                first = scroll_offset + 1;
            }
        }
        // A few lines of context when possible.
        let context = (height / 2).min(3);
        if height > 2 {
            if first > sel - context {
                first = (sel - context).max(0);
            } else if first < sel + context - height + 1 {
                first = sel + context - height + 1;
            }
        }
        first = first.min(size - height).max(0);
        pum.first = first as usize;
        let (_, rows) = self.screen_size();
        if let Some(info) = info
            && rows > 10
            && popup
        {
            self.pum_set_info(&info);
            self.completion.ruler_cursor = Some(Cursor {
                line: self.window.cursor.line,
                col: start,
            });
        }
    }

    /// After a key: the statusline is drawn again when what it shows of the cursor changed
    /// (Neovim's `show_cursor_info_later`).
    pub fn pum_ruler_check(&mut self) {
        let now = (
            self.window.cursor,
            self.window.top,
            self.text().line_count(),
            self.mode,
        );
        if self.completion.stl != Some(now) {
            self.completion.ruler_cursor = None;
        }
        self.completion.stl = Some(now);
    }

    /// Put `info` in the info window and place it next to the menu (Neovim's
    /// `pum_preview_set_text` and `pum_adjust_info_position`).
    pub fn pum_set_info(&mut self, info: &str) {
        let Some(pum) = self.completion.pum.as_ref() else {
            return;
        };
        let (pum_row, pum_col, pum_width, sb) =
            (pum.row, pum.col, pum.width, usize::from(pum.scrollbar));
        let mut lines: Vec<&str> = info.split('\n').collect();
        // A last empty line is left out.
        if lines.len() > 1 && lines.last() == Some(&"") {
            lines.pop();
        }
        let ts = 8;
        let max_width = lines
            .iter()
            .map(|l| {
                flux_core::layout_line(l, ts, None).rows[0]
                    .iter()
                    .map(|g| usize::from(g.width))
                    .sum::<usize>()
            })
            .max()
            .unwrap_or(0);
        let text = format!("{}\n", lines.join("\n"));
        let buffer = match self.completion.info.as_ref() {
            Some(w) => w.buffer,
            None => {
                let mut b = Buffer::scratch(BufferId(0));
                b.listed = false;
                let id = self.add_buffer_hidden(b);
                self.completion.info = Some(InfoWindow {
                    buffer: id,
                    row: 0,
                    col: 0,
                    width: 0,
                    height: 0,
                    hidden: true,
                    markdown: false,
                    fit: false,
                });
                id
            }
        };
        if let Some(b) = self.buffer_mut(buffer) {
            b.text = flux_core::Text::new(&text);
        }
        let (columns, rows) = self.screen_size();
        let col = pum_col + pum_width + 1 + sb;
        let right_extra = columns as isize - col as isize;
        let left_extra = pum_col as isize - 2;
        let max_extra = right_extra.max(left_extra);
        let Some(w) = self.completion.info.as_mut() else {
            return;
        };
        if max_extra < 10 {
            w.hidden = true;
            return;
        }
        let max_width = max_width as isize;
        let (width, wcol) = if right_extra > max_width {
            (max_width, col as isize - 1)
        } else if left_extra > max_width {
            (max_width, pum_col as isize - max_width - 1)
        } else if right_extra > left_extra {
            (max_extra, col as isize - 1)
        } else {
            (max_extra, pum_col as isize - max_extra - 1)
        };
        w.width = width.max(1) as usize;
        w.col = wcol.max(0) as usize;
        w.row = pum_row;
        w.height = lines
            .iter()
            .map(|l| flux_core::layout_line(l, ts, Some(w.width)).row_count())
            .sum::<usize>()
            .min(rows);
        w.hidden = false;
        w.fit = w.markdown;
    }

    /// Show the info window's text as Markdown and fit its height to the text as shown
    /// (Neovim's `update_popup_window`).
    pub fn pum_info_markdown(&mut self) {
        let Some(w) = self.completion.info.as_mut() else {
            return;
        };
        w.fit = true;
        if w.markdown {
            return;
        }
        w.markdown = true;
        let buffer = w.buffer;
        if let Some(b) = self.buffer_mut(buffer) {
            b.opts.filetype = "markdown".into();
            b.syntax =
                flux_syntax::lang_for_filetype("markdown").and_then(flux_syntax::Syntax::new);
        }
    }

    /// The info window's lines as shown: concealed when it shows Markdown.
    pub fn pum_info_lines(&self) -> Vec<DisplayLine> {
        let Some(w) = self.completion.info.as_ref() else {
            return Vec::new();
        };
        let Some(buffer) = self.buffer(w.buffer) else {
            return Vec::new();
        };
        let text = &buffer.text;
        let spans = match &buffer.syntax {
            Some(s) if self.syntax_on && w.markdown => s.highlights(text, 0..text.line_count()),
            _ => Vec::new(),
        };
        (0..text.line_count())
            .filter(|&line| !spans.iter().any(|s| s.line == line && s.conceal_lines))
            .map(|line| DisplayLine {
                line,
                conceals: spans
                    .iter()
                    .filter(|s| s.line == line)
                    .filter_map(|s| Some((s.start, s.end, s.conceal.as_deref()?.chars().next())))
                    .collect(),
            })
            .collect()
    }

    /// Fit the info window's height to its text once its syntax is known.
    pub fn fit_pum_info(&mut self) {
        let Some(w) = self.completion.info.as_ref() else {
            return;
        };
        if !w.fit || w.hidden {
            return;
        }
        let parsed = self
            .buffer(w.buffer)
            .and_then(|b| b.syntax.as_ref())
            .is_none_or(|s| s.is_parsed());
        if !parsed {
            return;
        }
        let Some(text) = self.buffer(w.buffer).map(|b| &b.text) else {
            return;
        };
        let width = w.width;
        let rows: usize = self
            .pum_info_lines()
            .iter()
            .map(|l| info_layout(l, &text.line_str(l.line), width).row_count())
            .sum();
        let (_, screen_rows) = self.screen_size();
        if let Some(w) = self.completion.info.as_mut() {
            w.height = rows.min(screen_rows);
            w.fit = false;
        }
    }

    /// Take the menu and its info window off the screen.
    pub fn pum_undisplay(&mut self) {
        self.completion.pum = None;
        if let Some(w) = self.completion.info.take() {
            self.buffers.retain(|b| b.id != w.buffer);
        }
    }

    /// The buffer of the info window, while it's shown (its syntax is kept up to date).
    pub fn pum_info_buffer(&self) -> Option<BufferId> {
        self.completion
            .info
            .as_ref()
            .filter(|w| !w.hidden && self.completion.pum.is_some())
            .map(|w| w.buffer)
    }

    /// The cursor line's text from column `start` to the cursor.
    pub fn text_before_cursor(&self, start: Cursor) -> String {
        let cur = self.window.cursor;
        let line = self.text().line_str(cur.line);
        line.chars()
            .skip(start.col)
            .take(cur.col.saturating_sub(start.col))
            .collect()
    }
}

/// An info window line laid out in `width` columns: wrapped anywhere ('wrap' without
/// 'linebreak'), with concealed text taken out.
pub fn info_layout(line: &DisplayLine, text: &str, width: usize) -> flux_core::LineLayout {
    let mut layout = flux_core::layout_line(text, 8, Some(width.max(1)));
    let mut replaced = std::collections::HashSet::new();
    for row in &mut layout.rows {
        row.retain_mut(|g| {
            let Some(&(start, _, with)) = line
                .conceals
                .iter()
                .find(|(s, e, _)| *s <= g.char_idx && g.char_idx < *e)
            else {
                return true;
            };
            match with {
                Some(c) if replaced.insert(start) => {
                    g.symbol = c.to_string();
                    g.width = 1;
                    g.kind = flux_core::GlyphKind::Text;
                    true
                }
                _ => false,
            }
        });
    }
    layout
}
