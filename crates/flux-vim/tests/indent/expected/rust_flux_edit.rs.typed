//! One change to the text: replace `delete` chars at `at` with `insert`.

use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// Char index where the edit happens.
    pub at: usize,
    /// Number of chars removed from `at`.
    pub delete: usize,
    pub insert: String,
}

impl Edit {
    pub fn insert(at: usize, text: impl Into<String>) -> Self {
        Self {
            at,
            delete: 0,
            insert: text.into(),
        }
    }

    pub fn delete(range: Range<usize>) -> Self {
        Self {
            at: range.start,
            delete: range.len(),
            insert: String::new(),
        }
    }

    pub fn replace(range: Range<usize>, text: impl Into<String>) -> Self {
        Self {
            at: range.start,
            delete: range.len(),
            insert: text.into(),
        }
    }
}
