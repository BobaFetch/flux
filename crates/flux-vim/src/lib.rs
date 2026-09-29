//! The Vim modal engine: keys in, editor state changes out. No IO.

pub mod engine;
pub mod ex;
pub mod key;

pub use engine::Engine;
pub use key::{Key, KeyCode, Modifiers, parse_keys};
