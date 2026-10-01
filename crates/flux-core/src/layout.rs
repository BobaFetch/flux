//! How one line of text is laid out on screen: tabs, unprintable characters, wide characters and
//! soft wrapping. Follows Vim's display rules so cursor columns and row counts line up with Vim.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphKind {
    Text,
    /// One cell of an expanded tab.
    Tab,
    /// Part of a `^X` or `<hex>` rendering of an unprintable character (Vim's SpecialKey).
    Special,
    /// The `>` Vim shows when a double-width character doesn't fit at the end of a row.
    Filler,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Glyph {
    pub symbol: String,
    /// Screen cells: 1, or 2 for a wide character.
    pub width: u8,
    /// Char index within the line of the character this glyph displays.
    pub char_idx: usize,
    pub kind: GlyphKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineLayout {
    /// Screen rows; always at least one, even for an empty line.
    pub rows: Vec<Vec<Glyph>>,
}

impl LineLayout {
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Screen position `(row, column)` of the character at `char_idx`. On a tab this is the last
    /// cell of the tab where Vim puts the Normal-mode cursor, or the first cell when
    /// `tab_start` (Insert mode). Past the end of the line it is the cell just after the last
    /// glyph.
    pub fn cursor_position(&self, char_idx: usize, tab_start: bool) -> (usize, usize) {
        let mut found = None;
        for (r, row) in self.rows.iter().enumerate() {
            let mut x = 0;
            for glyph in row {
                if glyph.char_idx == char_idx {
                    match glyph.kind {
                        GlyphKind::Filler => {}
                        GlyphKind::Tab if tab_start => return (r, x),
                        GlyphKind::Tab => found = Some((r, x)),
                        GlyphKind::Text | GlyphKind::Special => return (r, x),
                    }
                }
                x += usize::from(glyph.width);
            }
        }
        if let Some(pos) = found {
            return pos;
        }
        let last = self.rows.len() - 1;
        let x = self.rows[last].iter().map(|g| usize::from(g.width)).sum();
        (last, x)
    }
}

/// Lay out `line` (without its line ending). `wrap_width` is the window's text width, or `None`
/// for a single unwrapped row.
pub fn layout_line(line: &str, tabstop: usize, wrap_width: Option<usize>) -> LineLayout {
    let mut builder = Builder {
        rows: vec![Vec::new()],
        x: 0,
        wrap_width: wrap_width.filter(|&w| w > 0),
    };
    let tabstop = tabstop.max(1);
    let mut vcol = 0;
    let mut char_idx = 0;

    for grapheme in line.graphemes(true) {
        let first = grapheme.chars().next().unwrap_or(' ');
        if grapheme == "\t" {
            let width = tabstop - vcol % tabstop;
            for _ in 0..width {
                builder.push(" ".into(), 1, char_idx, GlyphKind::Tab);
            }
            vcol += width;
        } else if let Some(special) = special_rendering(grapheme, first) {
            vcol += special.chars().count();
            for c in special.chars() {
                builder.push(c.to_string(), 1, char_idx, GlyphKind::Special);
            }
        } else {
            let width = grapheme.width().clamp(1, 2);
            if width == 2 && builder.remaining() == Some(1) {
                builder.push(">".into(), 1, char_idx, GlyphKind::Filler);
            }
            builder.push(grapheme.to_owned(), width as u8, char_idx, GlyphKind::Text);
            vcol += width;
        }
        char_idx += grapheme.chars().count();
    }

    LineLayout { rows: builder.rows }
}

/// Like [`layout_line`] wrapping at `width`, with Vim's 'linebreak': a row that would be cut in
/// the middle of a word breaks after the last 'breakat' character (` ^I!@*-+;:,./?`) instead.
pub fn layout_line_linebreak(line: &str, tabstop: usize, width: usize) -> LineLayout {
    let flat = layout_line(line, tabstop, None);
    let glyphs = flat.rows.into_iter().next().unwrap_or_default();
    let chars: Vec<char> = line.chars().collect();
    let breaks_after = |g: &Glyph| {
        g.kind != GlyphKind::Filler
            && chars
                .get(g.char_idx)
                .is_some_and(|c| " \t!@*-+;:,./?".contains(*c))
    };
    let width = width.max(1);
    let mut rows: Vec<Vec<Glyph>> = vec![Vec::new()];
    let mut x = 0;
    for g in glyphs {
        let w = usize::from(g.width);
        if x + w > width && !rows.last().expect("a row").is_empty() {
            let row = rows.last_mut().expect("a row");
            // Break after the last break character, unless that leaves the row empty.
            let cut = row
                .iter()
                .rposition(&breaks_after)
                .map(|i| i + 1)
                .filter(|&i| i < row.len());
            let moved = match cut {
                Some(i) => row.split_off(i),
                None => Vec::new(),
            };
            // The break character stretches over the rest of the row, as in Vim.
            if cut.is_some()
                && let Some(last) = row.last().cloned()
            {
                let used: usize = row.iter().map(|g| usize::from(g.width)).sum();
                for _ in used..width {
                    row.push(Glyph {
                        symbol: " ".into(),
                        width: 1,
                        ..last.clone()
                    });
                }
            }
            x = moved.iter().map(|g| usize::from(g.width)).sum();
            rows.push(moved);
        }
        x += w;
        rows.last_mut().expect("a row").push(g);
    }
    LineLayout { rows }
}

/// How Vim displays characters that can't be printed as themselves: `^X` for C0 controls and
/// DEL, `<hex>` for C1 controls and zero-width characters.
fn special_rendering(grapheme: &str, first: char) -> Option<String> {
    match first {
        '\0'..='\x1f' => Some(format!("^{}", char::from(first as u8 + 64))),
        '\x7f' => Some("^?".into()),
        '\u{80}'..='\u{9f}' => Some(format!("<{:x}>", first as u32)),
        _ if grapheme.width() == 0 => Some(format!("<{:x}>", first as u32)),
        _ => None,
    }
}

struct Builder {
    rows: Vec<Vec<Glyph>>,
    x: usize,
    wrap_width: Option<usize>,
}

impl Builder {
    fn remaining(&self) -> Option<usize> {
        self.wrap_width.map(|w| w - self.x)
    }

    fn push(&mut self, symbol: String, width: u8, char_idx: usize, kind: GlyphKind) {
        if let Some(wrap) = self.wrap_width
            && self.x + usize::from(width) > wrap
        {
            self.rows.push(Vec::new());
            self.x = 0;
        }
        self.x += usize::from(width);
        self.rows.last_mut().unwrap().push(Glyph {
            symbol,
            width,
            char_idx,
            kind,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(layout: &LineLayout) -> Vec<String> {
        layout
            .rows
            .iter()
            .map(|row| row.iter().map(|g| g.symbol.as_str()).collect())
            .collect()
    }

    #[test]
    fn empty_line_is_one_row() {
        let layout = layout_line("", 8, Some(10));
        assert_eq!(layout.row_count(), 1);
        assert_eq!(layout.cursor_position(0, false), (0, 0));
    }

    #[test]
    fn tabs_expand_to_the_next_tabstop() {
        let layout = layout_line("a\tb", 4, None);
        assert_eq!(render(&layout), ["a   b"]);
        // Normal-mode cursor sits on the last cell of the tab.
        assert_eq!(layout.cursor_position(1, false), (0, 3));
        assert_eq!(layout.cursor_position(1, true), (0, 1));
        assert_eq!(layout.cursor_position(2, false), (0, 4));
    }

    #[test]
    fn tab_width_uses_line_column_not_row_column() {
        // The tab starts at virtual column 6: two cells to reach 8, split across the wrap.
        let layout = layout_line("abcdef\tg", 8, Some(7));
        assert_eq!(render(&layout), ["abcdef ", " g"]);
    }

    #[test]
    fn control_characters() {
        assert_eq!(
            render(&layout_line("a\x01\x7f\u{85}", 8, None)),
            ["a^A^?<85>"]
        );
        assert_eq!(render(&layout_line("x\u{200b}y", 8, None)), ["x<200b>y"]);
    }

    #[test]
    fn linebreak_breaks_after_blanks() {
        let l = layout_line_linebreak("aaa bbb ccc", 8, 9);
        assert_eq!(render(&l), ["aaa bbb  ", "ccc"]);
        // A word longer than the row is cut anyway.
        let l = layout_line_linebreak("abcdefghijk", 8, 5);
        assert_eq!(render(&l), ["abcde", "fghij", "k"]);
        let l = layout_line_linebreak("a b", 8, 10);
        assert_eq!(render(&l), ["a b"]);
    }

    #[test]
    fn wrapping_counts_rows() {
        assert_eq!(layout_line(&"x".repeat(10), 8, Some(10)).row_count(), 1);
        assert_eq!(layout_line(&"x".repeat(11), 8, Some(10)).row_count(), 2);
        assert_eq!(layout_line(&"x".repeat(200), 8, Some(80)).row_count(), 3);
        assert_eq!(layout_line(&"x".repeat(200), 8, None).row_count(), 1);
    }

    #[test]
    fn wide_char_that_does_not_fit_gets_a_filler() {
        let layout = layout_line("abc日本", 8, Some(4));
        assert_eq!(render(&layout), ["abc>", "日本"]);
        assert_eq!(layout.cursor_position(3, false), (1, 0));
        assert_eq!(layout.cursor_position(4, false), (1, 2));
    }

    #[test]
    fn combining_sequences_are_one_glyph() {
        let layout = layout_line("e\u{301}x", 8, None);
        assert_eq!(layout.rows[0].len(), 2);
        assert_eq!(layout.cursor_position(2, false), (0, 1));
    }
}
