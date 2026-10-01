//! Editor state for flux: buffers, the window and its viewport, and what the renderer needs to know.

pub mod buffer;
pub mod editor;
pub mod explorer;
pub mod filetype;
pub mod highlight;
pub mod layout;
pub mod marks;
pub mod matchparen;
pub mod options;
pub mod registers;
pub mod search;
pub mod window;

pub use buffer::{Buffer, BufferId};
pub use editor::{
    CMDLINE_ROWS, Editor, Message, MessageKind, Mode, Options, Preview, Visual, VisualKind,
};
pub use layout::{Dir, Layout, LayoutTree, Rect, WindowId};
pub use marks::{Jump, JumpList, LineShift, Marks};
pub use registers::{Register, RegisterKind, Registers};
pub use window::{Cursor, Metrics, Window};
