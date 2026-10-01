//! Tree-sitter parsing and highlighting, with Neovim's queries and its query extensions.

pub mod filetype;
mod highlight;
mod lang;
mod predicate;

pub use filetype::{detect, lang_for_filetype, lang_for_injection};
pub use highlight::{Span, Syntax};
pub use lang::names as languages;
