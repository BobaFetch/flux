//! Floating windows like Neovim's `vim.lsp.util.open_floating_preview` (hover, diagnostics,
//! signature help): a scratch buffer shown over the windows next to the cursor, closed when the
//! cursor moves, a character is typed, or another buffer is shown.

use crate::{Buffer, BufferId, Cursor, Editor, WindowId};

/// A highlight on a float's line: chars `start..end` in group `group`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FloatHighlight {
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub group: String,
}

#[derive(Debug, Clone)]
pub struct Float {
    /// The scratch buffer holding the lines (unlisted; its filetype gives its syntax).
    pub buffer: BufferId,
    /// Screen position and size.
    pub row: usize,
    pub col: usize,
    pub width: usize,
    pub height: usize,
    pub highlights: Vec<FloatHighlight>,
    /// What opened it (`cursor` for `CTRL-W d`, `textDocument/hover`, …): asking again for the
    /// same thing while it's open doesn't open another.
    pub focus_id: String,
    /// Where the cursor was when it opened: it closes when the cursor leaves.
    pub window: WindowId,
    pub source: BufferId,
    pub cursor: Cursor,
}

impl Editor {
    /// Open a float showing `lines` next to the cursor, as `open_floating_preview` places it.
    /// `filetype` sets the scratch buffer's filetype (`markdown` for hover). Replaces a float
    /// already open.
    pub fn open_float(
        &mut self,
        lines: Vec<String>,
        filetype: Option<&str>,
        highlights: Vec<FloatHighlight>,
        focus_id: &str,
    ) {
        self.close_floats();
        if lines.is_empty() {
            return;
        }
        let rect = self
            .window_rects()
            .into_iter()
            .find(|(id, _)| *id == self.window.id)
            .map(|(_, r)| r);
        let Some(rect) = rect else {
            return;
        };
        // Width: the longest line, at most the window's; lines wrap at that width.
        let widths: Vec<usize> = lines
            .iter()
            .map(|l| unicode_width::UnicodeWidthStr::width(l.as_str()))
            .collect();
        let win_width = rect.width;
        let width = widths
            .iter()
            .copied()
            .max()
            .unwrap_or(0)
            .min(win_width)
            .max(1);
        let mut height: usize = if width >= win_width {
            widths.iter().map(|&w| w.div_ceil(width).max(1)).sum()
        } else {
            lines.len()
        };
        // The cursor's place on screen.
        let (cur_row, cur_col) = self.cursor_screen_position();
        let lines_above = cur_row.saturating_sub(rect.row);
        let lines_below = rect.height.saturating_sub(lines_above);
        let row = if lines_below > lines_above {
            height = height.min(lines_below.saturating_sub(1)).max(1);
            cur_row + 1
        } else {
            height = height.min(lines_above).max(1);
            cur_row - height
        };
        let col = if cur_col + 1 + width <= self.screen_size().0 {
            cur_col
        } else {
            (cur_col + 1).saturating_sub(width)
        };
        let mut buffer = Buffer::scratch(BufferId(0));
        buffer.text = flux_core::Text::new(&format!("{}\n", lines.join("\n")));
        buffer.listed = false;
        let id = self.add_buffer_hidden(buffer);
        if let Some(ft) = filetype
            && let Some(b) = self.buffer_mut(id)
        {
            b.opts.filetype = ft.to_string();
            let lang = flux_syntax::lang_for_filetype(ft);
            b.syntax = lang.and_then(flux_syntax::Syntax::new);
        }
        self.floats.push(Float {
            buffer: id,
            row,
            col,
            width,
            height,
            highlights,
            focus_id: focus_id.to_string(),
            window: self.window.id,
            source: self.window.buffer,
            cursor: self.window.cursor,
        });
    }

    /// Close every float, removing its scratch buffer.
    pub fn close_floats(&mut self) {
        for f in std::mem::take(&mut self.floats) {
            self.buffers.retain(|b| b.id != f.buffer);
        }
    }

    /// Close floats whose reason to be is gone (Neovim's close events: CursorMoved,
    /// CursorMovedI, BufLeave).
    pub fn check_floats(&mut self) {
        let moved = self.floats.iter().any(|f| {
            f.window != self.window.id
                || f.source != self.window.buffer
                || f.cursor != self.window.cursor
        });
        if moved {
            self.close_floats();
        }
    }

    /// Where the cursor is on the screen (row, column), as the renderer places it.
    pub fn cursor_screen_position(&self) -> (usize, usize) {
        let rect = self
            .window_rects()
            .into_iter()
            .find(|(id, _)| *id == self.window.id)
            .map(|(_, r)| r)
            .unwrap_or_default();
        let m = self.metrics();
        let insert = self.mode == crate::Mode::Insert;
        let (row, vcol) = self.window.cursor_screen_offset(&m, insert);
        let gutter = rect.width.saturating_sub(self.window.width);
        (rect.row + row, rect.col + gutter + vcol)
    }
}
