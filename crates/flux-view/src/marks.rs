//! Marks, the jumplist, and how edits move them.

use std::collections::HashMap;
use std::ops::Range;

use flux_core::{Edit, Text};

use crate::Cursor;

/// Named and automatic marks of a buffer: `a`–`z` (and, while flux has one buffer, `A`–`Z`),
/// plus Vim's automatic ones (`[`, `]`, `<`, `>`, `.`, `^`).
#[derive(Debug, Clone, Default)]
pub struct Marks {
    marks: HashMap<char, Cursor>,
}

impl Marks {
    pub fn get(&self, name: char) -> Option<Cursor> {
        self.marks.get(&name).copied()
    }

    pub fn set(&mut self, name: char, pos: Cursor) {
        self.marks.insert(name, pos);
    }

    pub fn remove(&mut self, name: char) {
        self.marks.remove(&name);
    }

    /// Move every mark for an edit about to be applied to `text`. Marks on deleted lines are
    /// removed, as in Vim.
    pub fn adjust(&mut self, shift: &LineShift) {
        self.marks.retain(|_, pos| match shift.adjust(*pos) {
            Some(p) => {
                *pos = p;
                true
            }
            None => false,
        });
    }

    /// Marks in name order, for `:marks`.
    pub fn sorted(&self) -> Vec<(char, Cursor)> {
        let mut marks: Vec<_> = self.marks.iter().map(|(&c, &p)| (c, p)).collect();
        marks.sort_by_key(|&(c, _)| c);
        marks
    }
}

/// How an edit moves lines: Vim adjusts marks by line, not by column.
#[derive(Debug, Clone)]
pub struct LineShift {
    line: usize,
    col: usize,
    removed: usize,
    inserted: usize,
    /// Whole lines the edit deletes.
    deleted: Option<Range<usize>>,
    /// The edit inserts whole lines above `line`, which moves `line` down too.
    inserts_above: bool,
}

impl LineShift {
    /// The shift `edit` will cause when applied to `text`.
    pub fn of(text: &Text, edit: &Edit) -> Self {
        let (line, col) = text.char_to_pos(edit.at);
        let removed_text = text.slice(edit.at..edit.at + edit.delete);
        let removed = removed_text.matches('\n').count();
        let inserted = edit.insert.matches('\n').count();
        let pure_delete = edit.insert.is_empty() && removed > 0;
        let deleted = if pure_delete && col == 0 && removed_text.ends_with('\n') {
            Some(line..line + removed)
        } else if pure_delete && removed_text.starts_with('\n') && col == text.line_len(line) {
            Some(line + 1..line + 1 + removed)
        } else {
            None
        };
        let inserts_above = edit.delete == 0 && col == 0 && edit.insert.ends_with('\n');
        Self {
            line,
            col,
            removed,
            inserted,
            deleted,
            inserts_above,
        }
    }

    /// Where a position ends up, or `None` if its line is deleted.
    pub fn adjust(&self, mut p: Cursor) -> Option<Cursor> {
        if let Some(deleted) = &self.deleted {
            if deleted.contains(&p.line) {
                return None;
            }
            if p.line >= deleted.end {
                p.line -= deleted.len();
            }
            return Some(p);
        }
        if p.line < self.line || (p.line == self.line && !self.inserts_above) {
            return Some(p);
        }
        if p.line == self.line || p.line > self.line + self.removed {
            p.line = p.line + self.inserted - self.removed;
            return Some(p);
        }
        // On a line merged into the edit's first line (a join, a multi-line change).
        Some(Cursor {
            line: self.line,
            col: self.col,
        })
    }
}

/// A position in the jumplist: which buffer, and where.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Jump {
    pub buffer: crate::BufferId,
    pub pos: Cursor,
}

/// Positions jumped from (Vim's jumplist), for `CTRL-O` and `CTRL-I`.
#[derive(Debug, Clone, Default)]
pub struct JumpList {
    entries: Vec<Jump>,
    /// Where `CTRL-O`/`CTRL-I` are in the list; `entries.len()` when at the newest end.
    idx: usize,
}

const JUMPLIST_SIZE: usize = 100;

impl JumpList {
    /// Remember a position as a place jumped from (Vim's `setpcmark`).
    pub fn push(&mut self, jump: Jump) {
        self.entries.push(jump);
        if self.entries.len() > JUMPLIST_SIZE {
            self.entries.remove(0);
        }
        self.idx = self.entries.len();
    }

    /// Move `count` entries back (negative: forward) from `current`, returning where to go.
    pub fn jump(&mut self, count: isize, current: Jump) -> Option<Jump> {
        self.cleanup();
        if self.entries.is_empty() {
            return None;
        }
        let target = self.idx as isize - count;
        if target < 0 || target >= self.entries.len() as isize {
            return None;
        }
        if self.idx == self.entries.len() {
            // The first CTRL-O after a jump remembers where it came from, so CTRL-I can return.
            self.push(current);
            self.cleanup();
            self.idx = self.entries.len() - 1;
            if self.idx as isize - count < 0 {
                return None;
            }
        }
        self.idx = (self.idx as isize - count) as usize;
        self.entries.get(self.idx).copied()
    }

    /// Drop older entries on the same line of the same buffer as a newer one, keeping `idx` on
    /// the same entry.
    fn cleanup(&mut self) {
        let mut kept = Vec::with_capacity(self.entries.len());
        let mut new_idx = self.idx;
        for (i, e) in self.entries.iter().enumerate() {
            let later_same_line = self.entries[i + 1..]
                .iter()
                .any(|l| l.buffer == e.buffer && l.pos.line == e.pos.line);
            if later_same_line {
                if i < self.idx {
                    new_idx -= 1;
                }
            } else {
                kept.push(*e);
            }
        }
        self.entries = kept;
        self.idx = new_idx.min(self.entries.len());
    }

    /// Move entries in `buffer` for an edit.
    pub fn adjust(&mut self, buffer: crate::BufferId, shift: &LineShift) {
        for e in self.entries.iter_mut().filter(|e| e.buffer == buffer) {
            e.pos = shift.adjust(e.pos).unwrap_or(Cursor {
                line: shift.line,
                col: 0,
            });
        }
    }

    /// Forget entries in a buffer that no longer exists.
    pub fn remove_buffer(&mut self, buffer: crate::BufferId) {
        let before = self.entries.len();
        self.entries.retain(|e| e.buffer != buffer);
        self.idx = self
            .idx
            .saturating_sub(before - self.entries.len())
            .min(self.entries.len());
    }

    pub fn entries(&mut self) -> (&[Jump], usize) {
        self.cleanup();
        (&self.entries, self.idx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(line: usize, col: usize) -> Cursor {
        Cursor { line, col }
    }

    #[test]
    fn marks_follow_line_deletes_and_inserts() {
        let text = Text::new("1\n2\n3\n4\n");
        let mut marks = Marks::default();
        marks.set('a', p(2, 0));
        marks.set('b', p(1, 0));
        // `dd` on line 1.
        marks.adjust(&LineShift::of(&text, &Edit::delete(2..4)));
        assert_eq!(marks.get('a'), Some(p(1, 0)));
        assert_eq!(marks.get('b'), None);
        // `O` above line 0.
        marks.adjust(&LineShift::of(&text, &Edit::insert(0, "new\n")));
        assert_eq!(marks.get('a'), Some(p(2, 0)));
        // `dd` on the last line deletes "\n4".
        let mut m = Marks::default();
        m.set('z', p(3, 0));
        m.adjust(&LineShift::of(&text, &Edit::delete(5..7)));
        assert_eq!(m.get('z'), None);
    }

    fn j(line: usize, col: usize) -> Jump {
        Jump {
            buffer: crate::BufferId(1),
            pos: p(line, col),
        }
    }

    #[test]
    fn jumplist_back_and_forth() {
        let mut list = JumpList::default();
        list.push(j(0, 0)); // G from line 0
        assert_eq!(list.jump(1, j(4, 0)), Some(j(0, 0)));
        assert_eq!(list.jump(-1, j(0, 0)), Some(j(4, 0)));
        assert_eq!(list.jump(-1, j(4, 0)), None);
    }

    #[test]
    fn jumplist_drops_duplicate_lines() {
        let mut list = JumpList::default();
        list.push(j(1, 0));
        list.push(j(2, 0));
        list.push(j(1, 3));
        assert_eq!(list.jump(1, j(5, 0)), Some(j(1, 3)));
        assert_eq!(list.jump(1, j(1, 3)), Some(j(2, 0)));
    }
}
