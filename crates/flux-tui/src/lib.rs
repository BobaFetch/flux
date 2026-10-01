//! Terminal rendering for flux: a cell grid, drawing the editor into it, and writing only the
//! cells that changed.

pub mod draw;
pub mod grid;
pub mod renderer;
pub mod theme;

pub use draw::draw;
pub use grid::{Cell, Color, Grid, Style};
pub use renderer::Renderer;
