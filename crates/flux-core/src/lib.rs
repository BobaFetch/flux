//! Text storage, editing and screen layout for flux. No IO.

pub mod chars;
pub mod edit;
pub mod history;
pub mod layout;
pub mod pattern;
pub mod text;

pub use edit::Edit;
pub use history::{Change, History, Step};
pub use layout::{Glyph, GlyphKind, LineLayout, layout_line, layout_line_linebreak};
pub use pattern::{Match, Pattern, PatternError, PatternOptions};
pub use ropey::Rope;
pub use text::{ByteEdit, BytePoint, LineEnding, LoggedEdit, Revision, Text};
