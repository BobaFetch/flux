//! Buffer text, stored as a rope.

use std::borrow::Cow;

use ropey::{Rope, RopeSlice};

/// How lines end on disk, like Vim's 'fileformat'.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineEnding {
    #[default]
    Lf,
    /// Every line ends in `\r\n`. A file with mixed endings stays `Lf` and shows its `\r`s as `^M`, as in Vim.
    Crlf,
}

#[derive(Debug, Clone, Default)]
pub struct Text {
    rope: Rope,
    line_ending: LineEnding,
}

impl Text {
    pub fn new(s: &str) -> Self {
        let rope = Rope::from_str(s);
        let line_ending = detect_line_ending(&rope);
        Self { rope, line_ending }
    }

    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    pub fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }

    pub fn is_empty(&self) -> bool {
        self.rope.len_chars() == 0
    }

    /// Whether the last line has a terminator. Vim reports a missing one as `[noeol]`.
    pub fn has_final_eol(&self) -> bool {
        self.is_empty() || self.ends_with_newline()
    }

    /// Number of lines as Vim counts them: a final line terminator ends the last line rather than
    /// starting a new one, and empty text still has one empty line.
    pub fn line_count(&self) -> usize {
        let n = self.rope.len_lines();
        if n > 1 && self.ends_with_newline() {
            n - 1
        } else {
            n
        }
    }

    /// Line `idx` (0-based) without its line ending.
    pub fn line(&self, idx: usize) -> RopeSlice<'_> {
        debug_assert!(idx < self.line_count(), "line {idx} out of range");
        let line = self.rope.line(idx);
        let mut end = line.len_chars();
        if end > 0 && line.char(end - 1) == '\n' {
            end -= 1;
            if self.line_ending == LineEnding::Crlf && end > 0 && line.char(end - 1) == '\r' {
                end -= 1;
            }
        }
        line.slice(..end)
    }

    /// Line `idx` as a string, borrowed when the rope stores it contiguously.
    pub fn line_str(&self, idx: usize) -> Cow<'_, str> {
        self.line(idx).into()
    }

    fn ends_with_newline(&self) -> bool {
        let len = self.rope.len_chars();
        len > 0 && self.rope.char(len - 1) == '\n'
    }
}

fn detect_line_ending(rope: &Rope) -> LineEnding {
    let mut saw_newline = false;
    for line in rope.lines() {
        let len = line.len_chars();
        if len == 0 || line.char(len - 1) != '\n' {
            continue;
        }
        saw_newline = true;
        if len < 2 || line.char(len - 2) != '\r' {
            return LineEnding::Lf;
        }
    }
    if saw_newline {
        LineEnding::Crlf
    } else {
        LineEnding::Lf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &Text) -> Vec<String> {
        (0..text.line_count())
            .map(|i| text.line_str(i).into_owned())
            .collect()
    }

    #[test]
    fn final_newline_terminates_last_line() {
        let text = Text::new("a\nb\n");
        assert_eq!(text.line_count(), 2);
        assert_eq!(lines(&text), ["a", "b"]);
        assert!(text.has_final_eol());
    }

    #[test]
    fn missing_final_newline() {
        let text = Text::new("a\nb");
        assert_eq!(lines(&text), ["a", "b"]);
        assert!(!text.has_final_eol());
    }

    #[test]
    fn empty_text_has_one_empty_line() {
        let text = Text::new("");
        assert_eq!(lines(&text), [""]);
        assert!(text.has_final_eol());
        assert_eq!(Text::new("\n").line_count(), 1);
        assert_eq!(Text::new("\n\n").line_count(), 2);
    }

    #[test]
    fn crlf_is_stripped_only_when_consistent() {
        let dos = Text::new("a\r\nb\r\n");
        assert_eq!(dos.line_ending(), LineEnding::Crlf);
        assert_eq!(lines(&dos), ["a", "b"]);

        let mixed = Text::new("a\r\nb\n");
        assert_eq!(mixed.line_ending(), LineEnding::Lf);
        assert_eq!(lines(&mixed), ["a\r", "b"]);
    }

    #[test]
    fn only_lf_breaks_lines() {
        let text = Text::new("a\rb\u{2028}c\n");
        assert_eq!(text.line_count(), 1);
    }
}
