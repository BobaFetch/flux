//! The undo tree.
//!
//! Every change is a node whose parent is the state it was made from. Undo moves to the parent,
//! redo to the child that was most recently undone (or created), so making a new change after
//! undoing starts a new branch rather than discarding the old one.

use std::ops::Range;
use std::time::SystemTime;

use ropey::Rope;

use crate::Edit;

/// One undoable change: everything a single command did.
#[derive(Debug, Clone)]
pub struct Change {
    /// Edits in the order they were made.
    pub edits: Vec<Edit>,
    /// Edits that undo them, in the order to apply.
    pub inverse: Vec<Edit>,
    /// Cursor `(line, col)` before the change.
    pub cursor_before: (usize, usize),
    /// Lines that differ, in the text before the change (what undo restores).
    pub before: Range<usize>,
    /// Lines that differ, in the text after the change (what redo restores).
    pub after: Range<usize>,
    /// Whether the text had no lines at all before and after (see `Text::has_no_lines`).
    pub no_lines_before: bool,
    pub no_lines_after: bool,
    /// Lines saved for undo, when not simply `before`/`after` (Vim saves each line `:s`
    /// changes on its own): the count before and after the change.
    pub saved_lines: Option<(usize, usize)>,
}

impl Change {
    /// Work out which lines differ between `before` and `after`, the text before and after
    /// `edits`. Only lines around the edits are compared.
    pub fn changed_lines(
        before: &Rope,
        after: &Rope,
        edits: &[Edit],
    ) -> (Range<usize>, Range<usize>) {
        // Everything before the first edit and after the last one is unchanged.
        let first_at = edits.iter().map(|e| e.at).min().unwrap_or(0);
        let mut len = before.len_chars();
        let mut tail = len;
        for edit in edits {
            len = len - edit.delete + edit.insert.chars().count();
            tail = tail.min(len - (edit.at + edit.insert.chars().count()));
        }
        let lines = |rope: &Rope| rope.len_lines();
        let (old_lines, new_lines) = (lines(before), lines(after));

        let mut prefix = before.char_to_line(first_at.min(before.len_chars()));
        let max_prefix = old_lines.min(new_lines);
        while prefix < max_prefix && same_line(before, prefix, after, prefix) {
            prefix += 1;
        }
        // Whole lines inside the untouched tail are equal; start comparing just above them.
        let tail_lines = after
            .len_chars()
            .checked_sub(tail)
            .map_or(0, |start| lines(after) - 1 - after.char_to_line(start));
        let mut suffix = tail_lines.min(old_lines - prefix).min(new_lines - prefix);
        while suffix < old_lines - prefix
            && suffix < new_lines - prefix
            && same_line(
                before,
                old_lines - 1 - suffix,
                after,
                new_lines - 1 - suffix,
            )
        {
            suffix += 1;
        }
        (prefix..old_lines - suffix, prefix..new_lines - suffix)
    }
}

/// Line contents are equal, ignoring whether a terminator follows (the last line has none).
fn same_line(a: &Rope, i: usize, b: &Rope, j: usize) -> bool {
    strip_newline(a.line(i)) == strip_newline(b.line(j))
}

fn strip_newline(line: ropey::RopeSlice<'_>) -> ropey::RopeSlice<'_> {
    let len = line.len_chars();
    if len > 0 && line.char(len - 1) == '\n' {
        line.slice(..len - 1)
    } else {
        line
    }
}

/// A change being undone or redone, with its number and when it was made (for Vim's
/// `1 change; before #3  2 seconds ago` message).
#[derive(Debug, Clone)]
pub struct Step {
    pub seq: usize,
    pub time: SystemTime,
    pub change: Change,
}

#[derive(Debug)]
struct Node {
    parent: usize,
    change: Option<Change>,
    /// The child redo goes to.
    redo: Option<usize>,
    time: SystemTime,
}

#[derive(Debug)]
pub struct History {
    nodes: Vec<Node>,
    current: usize,
    saved: Option<usize>,
}

impl Default for History {
    fn default() -> Self {
        Self {
            nodes: vec![Node {
                parent: 0,
                change: None,
                redo: None,
                time: SystemTime::now(),
            }],
            current: 0,
            saved: Some(0),
        }
    }
}

impl History {
    pub fn record(&mut self, change: Change) {
        let id = self.nodes.len();
        self.nodes.push(Node {
            parent: self.current,
            change: Some(change),
            redo: None,
            time: SystemTime::now(),
        });
        self.nodes[self.current].redo = Some(id);
        self.current = id;
    }

    /// Step back, returning the change to revert.
    pub fn undo(&mut self) -> Option<Step> {
        if self.current == 0 {
            return None;
        }
        let node = self.current;
        let parent = self.nodes[node].parent;
        self.nodes[parent].redo = Some(node);
        self.current = parent;
        Some(self.step(node))
    }

    /// Step forward again, returning the change to reapply.
    pub fn redo(&mut self) -> Option<Step> {
        let child = self.nodes[self.current].redo?;
        self.current = child;
        Some(self.step(child))
    }

    fn step(&self, node: usize) -> Step {
        Step {
            seq: node,
            time: self.nodes[node].time,
            change: self.nodes[node]
                .change
                .clone()
                .expect("only the root has no change"),
        }
    }

    /// Vim's change number: 0 for the original text.
    pub fn seq(&self) -> usize {
        self.current
    }

    /// When the current state was reached by a change.
    pub fn time(&self) -> SystemTime {
        self.nodes[self.current].time
    }

    pub fn mark_saved(&mut self) {
        self.saved = Some(self.current);
    }

    pub fn is_modified(&self) -> bool {
        self.saved != Some(self.current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Text;

    fn change(text: &mut Text, edits: Vec<Edit>) -> Change {
        let before = text.rope().clone();
        let mut inverse: Vec<Edit> = edits.iter().map(|e| text.apply(e)).collect();
        inverse.reverse();
        let (b, a) = Change::changed_lines(&before, text.rope(), &edits);
        Change {
            edits,
            inverse,
            cursor_before: (0, 0),
            before: b,
            after: a,
            no_lines_before: false,
            no_lines_after: false,
            saved_lines: None,
        }
    }

    #[test]
    fn changed_lines() {
        let mut text = Text::new("a\nb\nc\nd\n");
        // Delete line "b".
        let c = change(&mut text, vec![Edit::delete(2..4)]);
        assert_eq!((c.before, c.after), (1..2, 1..1));
        // Change a char in "c" (now line 1).
        let c = change(&mut text, vec![Edit::replace(2..3, "x")]);
        assert_eq!((c.before, c.after), (1..2, 1..2));
        // Insert a line after the last one.
        let c = change(&mut text, vec![Edit::insert(5, "\ne")]);
        assert_eq!((c.before, c.after), (3..3, 3..4));
    }

    #[test]
    fn undo_redo_and_branches() {
        let mut h = History::default();
        let c = || Change {
            edits: vec![],
            inverse: vec![],
            cursor_before: (0, 0),
            before: 0..0,
            after: 0..0,
            no_lines_before: false,
            no_lines_after: false,
            saved_lines: None,
        };
        assert!(!h.is_modified());
        h.record(c());
        h.record(c());
        assert!(h.is_modified());
        assert!(h.undo().is_some());
        assert!(h.undo().is_some());
        assert!(h.undo().is_none());
        assert!(!h.is_modified());
        assert!(h.redo().is_some());
        h.record(c()); // a new branch from state 1
        assert!(h.redo().is_none());
        assert_eq!(h.seq(), 3);
    }
}
