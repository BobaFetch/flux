//! Writes a grid to the terminal, sending only the cells that changed since the last frame.

use std::io::{self, Write};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::queue;
use crossterm::style::{
    Attribute, Color as TermColor, Print, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::terminal::{BeginSynchronizedUpdate, Clear, ClearType, EndSynchronizedUpdate};

use crate::grid::{Color, Grid, Style};

#[derive(Debug, Default)]
pub struct Renderer {
    previous: Option<Grid>,
}

impl Renderer {
    /// Forget what's on screen, so the next frame is drawn in full (after a resize or `CTRL-L`).
    pub fn invalidate(&mut self) {
        self.previous = None;
    }

    /// Draw `grid`, leaving the terminal cursor at `cursor` (or hidden).
    pub fn draw(
        &mut self,
        out: &mut impl Write,
        grid: &Grid,
        cursor: Option<(usize, usize)>,
    ) -> io::Result<()> {
        queue!(out, BeginSynchronizedUpdate, Hide)?;
        let previous = self.previous.take().filter(|p| {
            p.width() == grid.width() && p.height() == grid.height() && p.default == grid.default
        });
        if previous.is_none() {
            queue!(out, SetAttribute(Attribute::Reset), Clear(ClearType::All))?;
        }

        let mut style: Option<Style> = None;
        let mut link: Option<std::sync::Arc<str>> = None;
        let mut at: Option<(usize, usize)> = None;
        for y in 0..grid.height() {
            for x in 0..grid.width() {
                let cell = grid.cell(x, y);
                if cell.width == 0 {
                    continue;
                }
                if let Some(prev) = &previous {
                    let same = prev.cell(x, y) == cell
                        && (cell.width < 2 || prev.cell(x + 1, y) == grid.cell(x + 1, y));
                    if same {
                        continue;
                    }
                }
                if at != Some((x, y)) {
                    queue!(out, MoveTo(x as u16, y as u16))?;
                }
                // OSC 8 hyperlinks, as Neovim sends them for a `url` highlight.
                if link != cell.link {
                    if link.is_some() {
                        queue!(out, Print("\x1b]8;;\x1b\\"))?;
                    }
                    if let Some(url) = &cell.link {
                        queue!(out, Print(format!("\x1b]8;;{url}\x1b\\")))?;
                    }
                    link = cell.link.clone();
                }
                let cell_style = with_default(cell.style, grid.default);
                if style != Some(cell_style) {
                    apply_style(out, cell_style)?;
                    style = Some(cell_style);
                }
                queue!(out, Print(&cell.symbol))?;
                at = Some((x + usize::from(cell.width), y));
            }
        }

        if link.is_some() {
            queue!(out, Print("\x1b]8;;\x1b\\"))?;
        }
        queue!(out, SetAttribute(Attribute::Reset))?;
        if let Some((x, y)) = cursor {
            queue!(out, MoveTo(x as u16, y as u16), Show)?;
        }
        queue!(out, EndSynchronizedUpdate)?;
        out.flush()?;
        self.previous = Some(grid.clone());
        Ok(())
    }
}

/// `style` with its `Reset` colors replaced by the grid's default ones.
fn with_default(style: Style, default: Style) -> Style {
    Style {
        fg: if style.fg == Color::Reset {
            default.fg
        } else {
            style.fg
        },
        bg: if style.bg == Color::Reset {
            default.bg
        } else {
            style.bg
        },
        ..style
    }
}

fn apply_style(out: &mut impl Write, style: Style) -> io::Result<()> {
    queue!(
        out,
        SetAttribute(Attribute::Reset),
        SetForegroundColor(term_color(style.fg)),
        SetBackgroundColor(term_color(style.bg)),
    )?;
    for (on, attribute) in [
        (style.bold, Attribute::Bold),
        (style.italic, Attribute::Italic),
        (style.underline, Attribute::Underlined),
        (style.undercurl, Attribute::Undercurled),
        (style.strikethrough, Attribute::CrossedOut),
        (style.reverse, Attribute::Reverse),
    ] {
        if on {
            queue!(out, SetAttribute(attribute))?;
        }
    }
    Ok(())
}

fn term_color(color: Color) -> TermColor {
    const ANSI: [TermColor; 16] = [
        TermColor::Black,
        TermColor::DarkRed,
        TermColor::DarkGreen,
        TermColor::DarkYellow,
        TermColor::DarkBlue,
        TermColor::DarkMagenta,
        TermColor::DarkCyan,
        TermColor::Grey,
        TermColor::DarkGrey,
        TermColor::Red,
        TermColor::Green,
        TermColor::Yellow,
        TermColor::Blue,
        TermColor::Magenta,
        TermColor::Cyan,
        TermColor::White,
    ];
    match color {
        Color::Reset => TermColor::Reset,
        Color::Ansi(n) => ANSI
            .get(usize::from(n))
            .copied()
            .unwrap_or(TermColor::AnsiValue(n)),
        Color::Rgb(r, g, b) => TermColor::Rgb { r, g, b },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(renderer: &mut Renderer, grid: &Grid) -> String {
        let mut out = Vec::new();
        renderer.draw(&mut out, grid, Some((0, 0))).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn only_changed_cells_are_redrawn() {
        let mut renderer = Renderer::default();
        let mut grid = Grid::new(10, 2);
        grid.put_str(0, 0, "hello", Style::default());
        let first = frame(&mut renderer, &grid);
        assert!(first.contains("hello"));

        let unchanged = frame(&mut renderer, &grid);
        assert!(!unchanged.contains("ello"));

        grid.put_str(0, 1, "Z", Style::default());
        let second = frame(&mut renderer, &grid);
        assert!(second.contains('Z'));
        assert!(!second.contains("hello"));
    }

    #[test]
    fn default_colors_fill_in_and_force_a_redraw() {
        let mut renderer = Renderer::default();
        let mut grid = Grid::new(4, 1);
        grid.put_str(0, 0, "ab", Style::default());
        frame(&mut renderer, &grid);
        grid.default = Style {
            bg: Color::Rgb(1, 2, 3),
            ..Style::default()
        };
        let out = frame(&mut renderer, &grid);
        assert!(out.contains("ab"), "{out:?}");
        assert!(out.contains("48;2;1;2;3"), "{out:?}");
    }

    #[test]
    fn invalidate_redraws_everything() {
        let mut renderer = Renderer::default();
        let mut grid = Grid::new(10, 1);
        grid.put_str(0, 0, "hello", Style::default());
        frame(&mut renderer, &grid);
        renderer.invalidate();
        assert!(frame(&mut renderer, &grid).contains("hello"));
    }
}
