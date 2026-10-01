//! Buffer text, stored as a rope of lines joined by `\n`.
//!
//! Like Vim, the text is a list of lines: the final line terminator isn't part of it, and `\r\n`
//! line endings are converted on load. Both are remembered so the file is written back the same
//! way.

use std::borrow::Cow;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

use ropey::{Rope, RopeSlice};

use crate::Edit;

/// How lines end on disk, like Vim's 'fileformat'.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineEnding {
    #[default]
    Lf,
    /// Every line ends in `\r\n`. A file with mixed endings stays `Lf` and shows its `\r`s as `^M`, as in Vim.
    Crlf,
}

/// A position for a parser: the line, and the byte offset within it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BytePoint {
    pub row: usize,
    pub col: usize,
}

/// An edit in bytes and byte points, as incremental parsers (tree-sitter) want it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteEdit {
    pub start_byte: usize,
    pub old_end_byte: usize,
    pub new_end_byte: usize,
    pub start: BytePoint,
    pub old_end: BytePoint,
    pub new_end: BytePoint,
}

/// The edits made to a [`Text`] since they were last taken, for keeping a parse tree in step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edits {
    /// The text was made from these edits, in order.
    Known(Vec<ByteEdit>),
    /// The text is not the one last seen (or too many edits were made): start over.
    Unknown,
}

/// Edits kept for a reader at most; past this nobody is reading them, and a fresh parse is
/// cheaper than replaying them anyway.
const MAX_LOGGED_EDITS: usize = 10_000;

static NEXT_TEXT_ID: AtomicU64 = AtomicU64::new(1);

fn next_id() -> u64 {
    NEXT_TEXT_ID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug)]
pub struct Text {
    rope: Rope,
    line_ending: LineEnding,
    final_eol: bool,
    /// The buffer has no lines at all, as opposed to one empty line: an empty file, or after
    /// deleting every line. Vim writes such a buffer as an empty file.
    no_lines: bool,
    /// Identifies this text's line of edits: a new or cloned text gets a new id, so a reader
    /// that followed another one knows to start over.
    id: u64,
    /// Edits since the last [`Text::take_edits`], or `None` if they were too many to keep.
    edits: Option<Vec<ByteEdit>>,
}

impl Clone for Text {
    fn clone(&self) -> Self {
        Self {
            rope: self.rope.clone(),
            line_ending: self.line_ending,
            final_eol: self.final_eol,
            no_lines: self.no_lines,
            id: next_id(),
            edits: Some(Vec::new()),
        }
    }
}

impl Default for Text {
    fn default() -> Self {
        Self {
            rope: Rope::new(),
            line_ending: LineEnding::Lf,
            final_eol: true,
            no_lines: true,
            id: next_id(),
            edits: Some(Vec::new()),
        }
    }
}

impl Text {
    /// Text from a file's contents.
    pub fn new(contents: &str) -> Self {
        let line_ending = detect_line_ending(contents);
        let mut text: Cow<'_, str> = match line_ending {
            LineEnding::Crlf => contents.replace("\r\n", "\n").into(),
            LineEnding::Lf => contents.into(),
        };
        let final_eol = text.is_empty() || text.ends_with('\n');
        if text.ends_with('\n') {
            text.to_mut().pop();
        }
        Self {
            rope: Rope::from_str(&text),
            line_ending,
            final_eol,
            no_lines: contents.is_empty(),
            id: next_id(),
            edits: Some(Vec::new()),
        }
    }

    /// Which text this is (see [`Text::take_edits`]).
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The edits made since the last call, for a reader that last saw this text (by
    /// [`Text::id`]) then.
    pub fn take_edits(&mut self) -> Edits {
        match self.edits.replace(Vec::new()) {
            Some(edits) => Edits::Known(edits),
            None => Edits::Unknown,
        }
    }

    /// Where char index `idx` is, as a byte offset and a byte point.
    fn byte_point(&self, idx: usize) -> (usize, BytePoint) {
        let byte = self.rope.char_to_byte(idx);
        let row = self.rope.byte_to_line(byte);
        let col = byte - self.rope.line_to_byte(row);
        (byte, BytePoint { row, col })
    }

    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    /// Whether the file had a final line terminator. Vim reports a missing one as `[noeol]`.
    pub fn has_final_eol(&self) -> bool {
        self.final_eol
    }

    /// No lines at all (see the field). Vim shows `--No lines in buffer--` for this.
    pub fn has_no_lines(&self) -> bool {
        self.no_lines
    }

    pub fn set_no_lines(&mut self, no_lines: bool) {
        self.no_lines = no_lines && self.rope.len_chars() == 0;
    }

    pub fn is_empty(&self) -> bool {
        self.rope.len_chars() == 0
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    /// Number of lines; at least one.
    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    pub fn last_line(&self) -> usize {
        self.line_count() - 1
    }

    /// Line `idx` (0-based) without its line terminator.
    pub fn line(&self, idx: usize) -> RopeSlice<'_> {
        let line = self.rope.line(idx);
        let len = line.len_chars();
        if len > 0 && line.char(len - 1) == '\n' {
            line.slice(..len - 1)
        } else {
            line
        }
    }

    /// Line `idx` as a string, borrowed when the rope stores it contiguously.
    pub fn line_str(&self, idx: usize) -> Cow<'_, str> {
        self.line(idx).into()
    }

    /// Length of line `idx` in chars, without the terminator.
    pub fn line_len(&self, idx: usize) -> usize {
        self.line(idx).len_chars()
    }

    /// Char index where line `idx` starts.
    pub fn line_start(&self, idx: usize) -> usize {
        self.rope.line_to_char(idx)
    }

    /// Char index of column `col` (a char index within the line) on line `line`.
    pub fn pos_to_char(&self, line: usize, col: usize) -> usize {
        self.line_start(line) + col.min(self.line_len(line))
    }

    /// `(line, col)` of char index `idx`.
    pub fn char_to_pos(&self, idx: usize) -> (usize, usize) {
        let line = self.rope.char_to_line(idx.min(self.len_chars()));
        (line, idx - self.line_start(line))
    }

    pub fn slice(&self, range: Range<usize>) -> String {
        self.rope.slice(range).to_string()
    }

    /// Apply `edit`, returning the edit that undoes it.
    pub fn apply(&mut self, edit: &Edit) -> Edit {
        let end = edit.at + edit.delete;
        let removed = self.rope.slice(edit.at..end).to_string();
        let logged = self.edits.is_some();
        let before = logged.then(|| (self.byte_point(edit.at), self.byte_point(end)));
        self.rope.remove(edit.at..end);
        self.rope.insert(edit.at, &edit.insert);
        self.no_lines = false;
        if let Some(((start_byte, start), (old_end_byte, old_end))) = before {
            let (new_end_byte, new_end) = self.byte_point(edit.at + edit.insert.chars().count());
            let log = self.edits.as_mut().expect("logged");
            if log.len() < MAX_LOGGED_EDITS {
                log.push(ByteEdit {
                    start_byte,
                    old_end_byte,
                    new_end_byte,
                    start,
                    old_end,
                    new_end,
                });
            } else {
                self.edits = None;
            }
        }
        Edit {
            at: edit.at,
            delete: edit.insert.chars().count(),
            insert: removed,
        }
    }

    /// The file contents to write: every line terminated (Vim's 'fixendofline'), in the file's
    /// line ending.
    pub fn to_file_contents(&self) -> String {
        if self.no_lines {
            return String::new();
        }
        let eol = match self.line_ending {
            LineEnding::Lf => "\n",
            LineEnding::Crlf => "\r\n",
        };
        let mut out = String::with_capacity(self.rope.len_bytes() + self.line_count() * eol.len());
        for chunk in self.rope.chunks() {
            match self.line_ending {
                LineEnding::Lf => out.push_str(chunk),
                LineEnding::Crlf => out.push_str(&chunk.replace('\n', "\r\n")),
            }
        }
        out.push_str(eol);
        out
    }
}

fn detect_line_ending(contents: &str) -> LineEnding {
    let mut saw_newline = false;
    for line in contents.split_inclusive('\n') {
        if !line.ends_with('\n') {
            continue;
        }
        saw_newline = true;
        if !line.ends_with("\r\n") {
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
    fn edits_are_logged_in_bytes() {
        let mut text = Text::new("héllo\nwörld\n");
        assert_eq!(text.take_edits(), Edits::Known(vec![]));
        text.apply(&Edit::replace(8..9, "OO\nx"));
        let Edits::Known(edits) = text.take_edits() else {
            panic!("known");
        };
        assert_eq!(
            edits,
            [ByteEdit {
                start_byte: 10,
                old_end_byte: 11,
                new_end_byte: 14,
                start: BytePoint { row: 1, col: 3 },
                old_end: BytePoint { row: 1, col: 4 },
                new_end: BytePoint { row: 2, col: 1 },
            }]
        );
        assert_eq!(text.take_edits(), Edits::Known(vec![]));
        let copy = text.clone();
        assert_ne!(copy.id(), text.id());
        for _ in 0..=MAX_LOGGED_EDITS {
            text.apply(&Edit::insert(0, "a"));
        }
        assert_eq!(text.take_edits(), Edits::Unknown);
        assert_eq!(text.take_edits(), Edits::Known(vec![]));
    }

    #[test]
    fn final_newline_terminates_last_line() {
        let text = Text::new("a\nb\n");
        assert_eq!(lines(&text), ["a", "b"]);
        assert!(text.has_final_eol());
        assert_eq!(text.to_file_contents(), "a\nb\n");
    }

    #[test]
    fn missing_final_newline_is_added_on_write() {
        let text = Text::new("a\nb");
        assert_eq!(lines(&text), ["a", "b"]);
        assert!(!text.has_final_eol());
        assert_eq!(text.to_file_contents(), "a\nb\n");
    }

    #[test]
    fn empty_file_has_no_lines_but_one_empty_line() {
        let text = Text::new("");
        assert_eq!(lines(&text), [""]);
        assert!(text.has_no_lines());
        assert_eq!(text.to_file_contents(), "");

        let text = Text::new("\n");
        assert_eq!(lines(&text), [""]);
        assert!(!text.has_no_lines());
        assert_eq!(text.to_file_contents(), "\n");
        assert_eq!(Text::new("\n\n").line_count(), 2);
    }

    #[test]
    fn crlf_is_converted_only_when_consistent() {
        let dos = Text::new("a\r\nb\r\n");
        assert_eq!(dos.line_ending(), LineEnding::Crlf);
        assert_eq!(lines(&dos), ["a", "b"]);
        assert_eq!(dos.to_file_contents(), "a\r\nb\r\n");

        let mixed = Text::new("a\r\nb\n");
        assert_eq!(mixed.line_ending(), LineEnding::Lf);
        assert_eq!(lines(&mixed), ["a\r", "b"]);
    }

    #[test]
    fn only_lf_breaks_lines() {
        assert_eq!(Text::new("a\rb\u{2028}c\n").line_count(), 1);
    }

    #[test]
    fn positions() {
        let text = Text::new("ab\ncde\n");
        assert_eq!(text.pos_to_char(1, 2), 5);
        assert_eq!(text.char_to_pos(5), (1, 2));
        assert_eq!(text.char_to_pos(2), (0, 2));
        assert_eq!(text.line_len(1), 3);
    }

    #[test]
    fn apply_returns_the_inverse() {
        let mut text = Text::new("hello world\n");
        let inverse = text.apply(&Edit::replace(0..5, "bye"));
        assert_eq!(text.line_str(0), "bye world");
        text.apply(&inverse);
        assert_eq!(text.line_str(0), "hello world");
    }
}
