//! Buffer text, stored as a rope of lines joined by `\n`.
//!
//! Like Vim, the text is a list of lines: the final line terminator isn't part of it, and `\r\n`
//! line endings are converted on load. Both are remembered so the file is written back the same
//! way.

use std::borrow::Cow;
use std::collections::VecDeque;
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

/// One edit as readers of a text's history see it: in bytes and byte points (for tree-sitter
/// and language servers), and the text inserted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggedEdit {
    pub bytes: ByteEdit,
    pub text: String,
}

/// A text's place in its history, for readers that follow its edits (a parse tree, a
/// language server): see [`Text::edits_since`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Revision {
    id: u64,
    n: u64,
}

/// Edits kept for readers at most; a reader further behind starts over, which is cheaper than
/// replaying that many anyway.
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
    /// How many edits were made.
    revision: u64,
    /// The latest edits; the first one made the text revision `revision - edits.len()`.
    edits: VecDeque<LoggedEdit>,
}

impl Clone for Text {
    fn clone(&self) -> Self {
        Self {
            rope: self.rope.clone(),
            line_ending: self.line_ending,
            final_eol: self.final_eol,
            no_lines: self.no_lines,
            id: next_id(),
            revision: 0,
            edits: VecDeque::new(),
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
            revision: 0,
            edits: VecDeque::new(),
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
            revision: 0,
            edits: VecDeque::new(),
        }
    }

    /// Which text this is: a new or cloned text gets a new id.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Where the text is in its history (see [`Text::edits_since`]).
    pub fn revision(&self) -> Revision {
        Revision {
            id: self.id,
            n: self.revision,
        }
    }

    /// The edits that made this text from what it was at `since`, in order; `None` when that
    /// isn't known (another text, or edits too long ago): the reader starts over.
    pub fn edits_since(
        &self,
        since: Revision,
    ) -> Option<std::collections::vec_deque::Iter<'_, LoggedEdit>> {
        let first = self.revision - self.edits.len() as u64;
        if since.id != self.id || since.n < first || since.n > self.revision {
            return None;
        }
        Some(self.edits.range((since.n - first) as usize..))
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
        let ((start_byte, start), (old_end_byte, old_end)) =
            (self.byte_point(edit.at), self.byte_point(end));
        self.rope.remove(edit.at..end);
        self.rope.insert(edit.at, &edit.insert);
        self.no_lines = false;
        let (new_end_byte, new_end) = self.byte_point(edit.at + edit.insert.chars().count());
        if self.edits.len() == MAX_LOGGED_EDITS {
            self.edits.pop_front();
        }
        self.edits.push_back(LoggedEdit {
            bytes: ByteEdit {
                start_byte,
                old_end_byte,
                new_end_byte,
                start,
                old_end,
                new_end,
            },
            text: edit.insert.clone(),
        });
        self.revision += 1;
        Edit {
            at: edit.at,
            delete: edit.insert.chars().count(),
            insert: removed,
        }
    }

    /// The whole text as a language server is sent it: lines ending in `\n`, the last one
    /// too (Neovim's `_buf_get_full_text`).
    pub fn to_lsp_text(&self) -> String {
        let mut s = self.rope.to_string();
        s.push('\n');
        s
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
        let r0 = text.revision();
        assert_eq!(text.edits_since(r0).unwrap().count(), 0);
        text.apply(&Edit::replace(8..9, "OO\nx"));
        let edits: Vec<ByteEdit> = text.edits_since(r0).unwrap().map(|e| e.bytes).collect();
        assert_eq!(text.edits_since(r0).unwrap().next().unwrap().text, "OO\nx");
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
        // Each reader follows at its own pace.
        let r1 = text.revision();
        assert_eq!(text.edits_since(r1).unwrap().count(), 0);
        assert_eq!(text.edits_since(r0).unwrap().count(), 1);
        let copy = text.clone();
        assert!(copy.edits_since(r1).is_none());
        for _ in 0..MAX_LOGGED_EDITS {
            text.apply(&Edit::insert(0, "a"));
        }
        assert!(text.edits_since(r0).is_none());
        assert_eq!(text.edits_since(r1).unwrap().count(), MAX_LOGGED_EDITS);
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
