//! A snippet being filled in, as Neovim's `vim.snippet` does it: its tabstops are highlighted
//! (SnippetTabstop, the current one SnippetTabstopActive), `<Tab>` and `<S-Tab>` go from one
//! to the next, selecting a placeholder's text in Select mode, where typing replaces it.
//! Tabstops with the same number mirror each other. The session ends when the cursor leaves
//! the tabstops, or at the last one (`$0`).

use flux_core::Edit;
use flux_view::{BufferId, Cursor, Editor, Mode, Visual, VisualKind};

use crate::engine::Engine;
use crate::insert::InsertKind;
use crate::key::{Key, KeyCode, Modifiers};
use crate::util::pos;

/// How a position moves when text is inserted right at it (an extmark's gravity).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Gravity {
    start_right: bool,
    end_right: bool,
}

#[derive(Debug, Clone)]
struct Tabstop {
    index: usize,
    /// Char offsets in the buffer.
    start: usize,
    end: usize,
    gravity: Gravity,
    /// Where it was in the snippet, to tell which tabstops are next to each other.
    placement: usize,
    choices: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub(crate) struct Session {
    buffer: BufferId,
    tabstops: Vec<Tabstop>,
    /// The whole snippet's text.
    start: usize,
    end: usize,
    current: usize,
}

/// A tabstop of a snippet just inserted: its number, char range, place in the snippet, and a
/// choice's values.
pub(crate) struct NewTabstop {
    pub index: usize,
    pub start: usize,
    pub end: usize,
    pub placement: usize,
    pub choices: Option<Vec<String>>,
}

/// Move offset `p` over `edit`; `right` gravity moves it past text inserted right at it.
/// Offsets in deleted text go to its start (as extmarks do).
fn shift(p: usize, edit: &Edit, right: bool) -> usize {
    let (at, del, new) = (edit.at, edit.delete, edit.insert.chars().count());
    let end = at + del;
    if p < at {
        p
    } else if del == 0 {
        if p == at && !right { p } else { p + new }
    } else if p >= end {
        p + new - del
    } else if right {
        at + new
    } else {
        at
    }
}

impl Session {
    fn ranges(&self) -> impl Iterator<Item = &Tabstop> {
        self.tabstops.iter()
    }

    /// The next tabstop number to go to (Neovim's `get_dest_index`).
    fn dest_index(&self, forward: bool) -> Option<usize> {
        let mut indexes: Vec<usize> = self.tabstops.iter().map(|t| t.index).collect();
        indexes.sort_unstable();
        indexes.dedup();
        let i = indexes.iter().position(|&x| x == self.current)?;
        let dest = if forward {
            indexes.get(i + 1).copied().or(Some(0))
        } else {
            i.checked_sub(1).map(|j| indexes[j])
        };
        dest.filter(|&d| forward || d != 0)
    }

    /// Make the current tabstops grow with what's typed in them, and the others move or stay
    /// (Neovim's `Session:set_gravity`).
    fn set_gravity(&mut self) {
        let current = self.current;
        let dest: Vec<usize> = self
            .tabstops
            .iter()
            .filter(|t| t.index == current)
            .map(|t| t.placement)
            .collect();
        let all: Vec<usize> = self.tabstops.iter().map(|t| t.placement).collect();
        for t in &mut self.tabstops {
            if t.index == current {
                t.gravity = Gravity {
                    start_right: false,
                    end_right: true,
                };
                continue;
            }
            let mut p = t.placement + 1;
            while all.contains(&p) && !dest.contains(&p) {
                p += 1;
            }
            t.gravity = if dest.contains(&p) {
                Gravity {
                    start_right: false,
                    end_right: false,
                }
            } else {
                Gravity {
                    start_right: true,
                    end_right: true,
                }
            };
        }
    }

    /// The highlights to show: each tabstop's range, and whether it's the current one.
    fn highlights(&self, editor: &Editor) -> Vec<(Cursor, Cursor, bool)> {
        let text = editor.text();
        self.ranges()
            .filter(|t| t.end > t.start)
            .map(|t| {
                let (sl, sc) = text.char_to_pos(t.start.min(text.len_chars()));
                let (el, ec) = text.char_to_pos(t.end.min(text.len_chars()));
                (pos(sl, sc), pos(el, ec), t.index == self.current)
            })
            .collect()
    }
}

impl Engine {
    /// Start a session for a snippet just inserted, at its first tabstop (Neovim's
    /// `vim.snippet.expand` and `jump(1)`).
    pub(crate) fn snippet_start(
        &mut self,
        editor: &mut Editor,
        start: usize,
        end: usize,
        tabstops: Vec<NewTabstop>,
    ) {
        let tabstops = tabstops
            .into_iter()
            .map(|t| Tabstop {
                index: t.index,
                start: t.start,
                end: t.end,
                gravity: Gravity {
                    start_right: true,
                    end_right: false,
                },
                placement: t.placement,
                choices: t.choices,
            })
            .collect();
        self.compl.snippet = Some(Session {
            buffer: editor.window.buffer,
            tabstops,
            start,
            end,
            current: 0,
        });
        self.snippet_jump(editor, true);
    }

    /// Go to the next (or previous) tabstop: put the cursor there, or select its text
    /// (Neovim's `vim.snippet.jump` and `select_tabstop`).
    pub(crate) fn snippet_jump(&mut self, editor: &mut Editor, forward: bool) -> bool {
        let Some(s) = self.compl.snippet.as_mut() else {
            return false;
        };
        let Some(dest) = s.dest_index(forward) else {
            return false;
        };
        // The leftmost tabstop with that number.
        let Some(t) = s
            .tabstops
            .iter()
            .filter(|t| t.index == dest)
            .min_by_key(|t| (t.start, t.end))
            .cloned()
        else {
            return false;
        };
        s.current = dest;
        s.set_gravity();
        let text = editor.text();
        let (sl, sc) = text.char_to_pos(t.start.min(text.len_chars()));
        let (el, ec) = text.char_to_pos(t.end.min(text.len_chars()));
        if editor.completion.pum.is_some() {
            self.compl_reset(editor);
        }
        if t.choices.is_some() || dest == 0 || t.start == t.end {
            // Insert mode at the end of the range.
            if editor.mode == Mode::Visual {
                self.exit_visual(editor);
                editor.completion.select = false;
            }
            editor.window.cursor = pos(el, ec);
            if editor.mode != Mode::Insert {
                self.begin_insert(
                    editor,
                    InsertKind::Plain(crate::parse::InsertAt::Cursor),
                    1,
                    None,
                );
            }
            // A choice's values show in the menu (Neovim's `display_choices`).
            if let Some(values) = &t.choices {
                let now: String = editor.text().rope().slice(t.start..t.end).to_string();
                let mut items: Vec<crate::completion::Match> = values
                    .iter()
                    .filter(|v| **v != now)
                    .map(|v| crate::completion::Match::new(v.clone()))
                    .collect();
                if values.contains(&now) {
                    items.insert(0, crate::completion::Match::new(now));
                }
                self.set_completion(editor, sc, items);
            }
        } else {
            // Select the text: Select mode with the cursor at its start.
            if editor.mode == Mode::Insert {
                self.leave_insert(editor);
            } else if editor.mode == Mode::Visual {
                self.exit_visual(editor);
            }
            let last = if ec > 0 { pos(el, ec - 1) } else { pos(el, 0) };
            editor.visual = Visual {
                anchor: last,
                kind: VisualKind::Char,
            };
            editor.window.cursor = pos(sl, sc);
            editor.mode = Mode::Visual;
            editor.completion.select = true;
        }
        if dest == 0 {
            self.snippet_stop(editor);
        } else {
            self.snippet_show(editor);
        }
        true
    }

    /// End the session.
    pub(crate) fn snippet_stop(&mut self, editor: &mut Editor) {
        self.compl.snippet = None;
        editor.completion.snippet.clear();
    }

    fn snippet_show(&mut self, editor: &mut Editor) {
        editor.completion.snippet = self
            .compl
            .snippet
            .as_ref()
            .filter(|s| s.buffer == editor.window.buffer)
            .map(|s| s.highlights(editor))
            .unwrap_or_default();
    }

    /// Move the tabstops over an edit of the current buffer.
    pub(crate) fn snippet_edit(&mut self, editor: &Editor, edit: &Edit) {
        let Some(s) = self.compl.snippet.as_mut() else {
            return;
        };
        if s.buffer != editor.window.buffer {
            return;
        }
        s.start = shift(s.start, edit, false);
        s.end = shift(s.end, edit, true);
        for t in &mut s.tabstops {
            t.start = shift(t.start, edit, t.gravity.start_right);
            t.end = shift(t.end, edit, t.gravity.end_right).max(t.start);
        }
    }

    /// After a key: the session ends when the cursor left the snippet or its tabstops, and
    /// tabstops with the current number get its text (Neovim's CursorMoved and TextChanged
    /// handlers).
    pub(crate) fn snippet_check(&mut self, editor: &mut Editor) {
        if editor.mode != Mode::Visual {
            editor.completion.select = false;
        }
        let Some(s) = self.compl.snippet.as_ref() else {
            return;
        };
        if s.buffer != editor.window.buffer {
            self.snippet_stop(editor);
            return;
        }
        if s.start == s.end || s.end > editor.text().len_chars() {
            self.snippet_stop(editor);
            return;
        }
        // Mirrors.
        let current = s.current;
        let src = s
            .tabstops
            .iter()
            .filter(|t| t.index == current)
            .min_by_key(|t| (t.start, t.end))
            .cloned();
        if let Some(src) = src {
            let text: String = editor.text().rope().slice(src.start..src.end).to_string();
            let others: Vec<(usize, usize)> = s
                .tabstops
                .iter()
                .filter(|t| t.index == current && t.start != src.start)
                .map(|t| (t.start, t.end))
                .collect();
            for (a, b) in others.into_iter().rev() {
                let now: String = editor.text().rope().slice(a..b).to_string();
                if now != text {
                    let cur = editor.cursor();
                    let at = editor.text().pos_to_char(cur.line, cur.col);
                    let edit = Edit::replace(a..b, text.clone());
                    let moved = shift(at, &edit, true);
                    self.edit(editor, edit);
                    let (l, c) = editor
                        .text()
                        .char_to_pos(moved.min(editor.text().len_chars()));
                    editor.window.cursor = pos(l, c);
                }
            }
        }
        if matches!(editor.mode, Mode::Insert | Mode::Visual)
            && let Some(s) = self.compl.snippet.as_ref()
        {
            let cur = editor.cursor();
            let at = editor.text().pos_to_char(cur.line, cur.col);
            let inside = at >= s.start && at <= s.end;
            let on_tabstop = s
                .tabstops
                .iter()
                .any(|t| t.index != 0 && at >= t.start && at <= t.end);
            if !inside || !on_tabstop {
                self.snippet_stop(editor);
                return;
            }
        }
        self.snippet_show(editor);
    }

    /// `<Tab>` and `<S-Tab>` jump while a snippet can go that way (Neovim's default mappings).
    pub(crate) fn snippet_key(&mut self, editor: &mut Editor, key: Key) -> bool {
        if self.compl.snippet.is_none() || key.code != KeyCode::Tab {
            return false;
        }
        let forward = !key.mods.shift;
        let can = self
            .compl
            .snippet
            .as_ref()
            .is_some_and(|s| s.dest_index(forward).is_some());
        can && self.snippet_jump(editor, forward)
    }

    /// A key in Select mode: typing replaces the selection, `<Esc>` stops selecting, `CTRL-G`
    /// goes to Visual mode.
    pub(crate) fn select_key(&mut self, editor: &mut Editor, key: Key) -> bool {
        if !editor.completion.select {
            return false;
        }
        if self.snippet_key(editor, key) {
            return true;
        }
        let printable = key.typed_char().filter(|c| !c.is_control());
        let erase =
            key.mods == Modifiers::NONE && matches!(key.code, KeyCode::Backspace | KeyCode::Delete);
        if key == Key::plain(KeyCode::Esc) || key == Key::ctrl('c') {
            editor.completion.select = false;
            self.exit_visual(editor);
            return true;
        }
        if key == Key::ctrl('g') {
            editor.completion.select = false;
            return true;
        }
        if printable.is_none() && !erase && key != Key::plain(KeyCode::Enter) {
            editor.completion.select = false;
            return false;
        }
        // Delete the selection and type in its place.
        let a = editor.visual.anchor;
        let c = editor.cursor();
        let (from, to) = if (a.line, a.col) <= (c.line, c.col) {
            (a, c)
        } else {
            (c, a)
        };
        let text = editor.text();
        let start = text.pos_to_char(from.line, from.col);
        let end = (text.pos_to_char(to.line, to.col) + 1).min(text.len_chars());
        editor.completion.select = false;
        editor.mode = Mode::Normal;
        self.edit(editor, Edit::delete(start..end));
        editor.window.cursor = from;
        if erase {
            // `<BS>` only deletes.
            self.commit(editor);
            crate::normal::normalize_cursor(editor);
            return true;
        }
        self.begin_insert(editor, InsertKind::Change, 1, None);
        self.insert_key(editor, key);
        true
    }
}
