//! Text storage and screen layout for flux. No IO, no editor state.

pub mod layout;
pub mod text;

pub use layout::{Glyph, GlyphKind, LineLayout, layout_line};
pub use text::{LineEnding, Text};
