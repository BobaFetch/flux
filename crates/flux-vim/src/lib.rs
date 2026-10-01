//! The Vim modal engine: keys in, editor state changes out. No IO except Ex commands that read
//! and write files.

pub mod engine;
pub mod ex;
mod ex_lines;
mod global;
mod insert;
pub mod key;
pub mod motion;
mod normal;
pub mod parse;
mod search;
mod set;
mod substitute;
pub mod textobj;
mod util;
mod visual;
mod windows;

pub use engine::Engine;
pub use key::{Key, KeyCode, Modifiers, keys_to_text, parse_keys, text_to_keys};
