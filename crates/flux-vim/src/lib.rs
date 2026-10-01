//! The Vim modal engine: keys in, editor state changes out. No IO except Ex commands that read
//! and write files.

mod comments;
mod completion;
pub mod engine;
pub mod ex;
mod ex_lines;
mod format;
mod global;
mod indent;
mod insert;
pub mod key;
pub mod lsp;
pub mod motion;
mod normal;
mod open_line;
pub mod parse;
mod prompt;
mod quickfix;
mod search;
mod set;
mod snippet;
mod substitute;
pub mod textobj;
mod util;
mod visual;
mod windows;

pub use engine::Engine;
pub use key::{Key, KeyCode, Modifiers, keys_to_text, parse_keys, text_to_keys};
