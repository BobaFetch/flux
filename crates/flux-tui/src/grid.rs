//! A screen-sized grid of styled cells.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    /// The terminal's default.
    #[default]
    Reset,
    /// A terminal color number: the 16 basic colors (which follow the terminal's theme), or
    /// one of the 256.
    Ansi(u8),
    Rgb(u8, u8, u8),
}

/// How a cell looks. A `Reset` color is the grid's default (Neovim's Normal group).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub undercurl: bool,
    pub strikethrough: bool,
    pub reverse: bool,
}

impl Style {
    pub const fn fg(color: Color) -> Self {
        Self {
            fg: color,
            bg: Color::Reset,
            bold: false,
            italic: false,
            underline: false,
            undercurl: false,
            strikethrough: false,
            reverse: false,
        }
    }

    /// `top` drawn over `self`, as Neovim combines highlights (`hl_combine_attr`): its colors
    /// replace ours where it has them, and attributes add up.
    pub fn combine(self, top: Style) -> Style {
        let pick = |a: Color, b: Color| if b == Color::Reset { a } else { b };
        Style {
            fg: pick(self.fg, top.fg),
            bg: pick(self.bg, top.bg),
            bold: self.bold || top.bold,
            italic: self.italic || top.italic,
            underline: self.underline || top.underline,
            undercurl: self.undercurl || top.undercurl,
            strikethrough: self.strikethrough || top.strikethrough,
            reverse: self.reverse || top.reverse,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub symbol: String,
    /// 1, or 2 for a wide character. 0 marks the right half of the wide character to its left.
    pub width: u8,
    pub style: Style,
    /// The URL the cell links to (an OSC 8 hyperlink).
    pub link: Option<std::sync::Arc<str>>,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            symbol: " ".into(),
            width: 1,
            style: Style::default(),
            link: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    width: usize,
    height: usize,
    cells: Vec<Cell>,
    /// The colors a cell's `Reset` colors stand for.
    pub default: Style,
}

impl Grid {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            cells: vec![Cell::default(); width * height],
            default: Style::default(),
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn cell(&self, x: usize, y: usize) -> &Cell {
        &self.cells[y * self.width + x]
    }

    /// Put one grapheme at `(x, y)`. A wide grapheme that would hang off the right edge becomes a
    /// space. Overwriting half of an existing wide character blanks its other half.
    pub fn set(&mut self, x: usize, y: usize, symbol: &str, width: u8, style: Style) {
        if x >= self.width || y >= self.height {
            return;
        }
        self.clear_wide_neighbors(x, y);
        if width == 2 && x + 1 >= self.width {
            self.cells[y * self.width + x] = Cell {
                symbol: " ".into(),
                width: 1,
                style,
                link: None,
            };
            return;
        }
        self.cells[y * self.width + x] = Cell {
            symbol: symbol.to_owned(),
            width,
            style,
            link: None,
        };
        if width == 2 {
            self.clear_wide_neighbors(x + 1, y);
            self.cells[y * self.width + x + 1] = Cell {
                symbol: String::new(),
                width: 0,
                style,
                link: None,
            };
        }
    }

    /// Make the cell at `(x, y)` a link to `url`.
    pub fn set_link(&mut self, x: usize, y: usize, url: Option<std::sync::Arc<str>>) {
        if x < self.width && y < self.height {
            self.cells[y * self.width + x].link = url;
        }
    }

    fn clear_wide_neighbors(&mut self, x: usize, y: usize) {
        let i = y * self.width + x;
        if self.cells[i].width == 0 && x > 0 {
            let style = self.cells[i - 1].style;
            self.cells[i - 1] = Cell {
                style,
                ..Cell::default()
            };
        }
        if self.cells[i].width == 2 && x + 1 < self.width {
            let style = self.cells[i + 1].style;
            self.cells[i + 1] = Cell {
                style,
                ..Cell::default()
            };
        }
    }

    /// Write `s` from `(x, y)`, clipped at the right edge. Returns the column after the text.
    pub fn put_str(&mut self, x: usize, y: usize, s: &str, style: Style) -> usize {
        self.put_str_until(x, y, s, style, self.width)
    }

    /// Write `s` from `(x, y)`, clipped before column `end`. Returns the column after the text.
    pub fn put_str_until(
        &mut self,
        mut x: usize,
        y: usize,
        s: &str,
        style: Style,
        end: usize,
    ) -> usize {
        let end = end.min(self.width);
        for grapheme in s.graphemes(true) {
            let width = grapheme.width().clamp(1, 2);
            if x + width > end {
                break;
            }
            self.set(x, y, grapheme, width as u8, style);
            x += width;
        }
        x
    }

    /// Paint a whole row with `style`, clearing its text.
    pub fn fill_row(&mut self, y: usize, style: Style) {
        for x in 0..self.width {
            self.cells[y * self.width + x] = Cell {
                style,
                ..Cell::default()
            };
        }
    }

    /// The text of row `y`, for tests.
    pub fn row_text(&self, y: usize) -> String {
        (0..self.width)
            .map(|x| self.cell(x, y).symbol.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_characters_take_two_cells() {
        let mut grid = Grid::new(5, 1);
        assert_eq!(grid.put_str(0, 0, "a日b", Style::default()), 4);
        assert_eq!(grid.row_text(0), "a日b ");
        assert_eq!(grid.cell(2, 0).width, 0);
    }

    #[test]
    fn overwriting_half_a_wide_character_blanks_the_other_half() {
        let mut grid = Grid::new(4, 1);
        grid.put_str(0, 0, "日本", Style::default());
        grid.set(1, 0, "x", 1, Style::default());
        assert_eq!(grid.row_text(0), " x本");
        grid.set(2, 0, "y", 1, Style::default());
        assert_eq!(grid.row_text(0), " xy ");
    }

    #[test]
    fn wide_character_at_right_edge_is_clipped() {
        let mut grid = Grid::new(3, 1);
        assert_eq!(grid.put_str(0, 0, "ab日", Style::default()), 2);
        grid.set(2, 0, "日", 2, Style::default());
        assert_eq!(grid.row_text(0), "ab ");
    }
}
