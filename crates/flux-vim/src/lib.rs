//! The Vim modal engine: keys in, editor state changes out. No IO except Ex commands that read
//! and write files.

pub mod engine;
pub mod ex;
mod insert;
pub mod key;
pub mod motion;
mod normal;
pub mod parse;
pub mod textobj;
mod util;
mod visual;

pub use engine::Engine;
pub use key::{Key, KeyCode, Modifiers, keys_to_text, parse_keys, text_to_keys};
