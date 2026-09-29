//! The Vim modal engine: keys in, editor state changes out. No IO except Ex commands that read
//! and write files.

pub mod engine;
pub mod ex;
mod insert;
pub mod key;
pub mod motion;
mod normal;
pub mod parse;
mod util;

pub use engine::Engine;
pub use key::{Key, KeyCode, Modifiers, parse_keys};
