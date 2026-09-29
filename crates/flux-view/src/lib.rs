//! Editor state for flux: buffers, the window and its viewport, and what the renderer needs to know.

pub mod buffer;
pub mod editor;
pub mod window;

pub use buffer::{Buffer, BufferId};
pub use editor::{Editor, Message, Mode, Options};
pub use window::{Cursor, Metrics, Window};
