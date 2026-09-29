//! The engine: routes keys to the current mode, groups edits into undo steps, and records the
//! last change for `.`.

use std::time::SystemTime;

use flux_core::{Change, Edit, Rope, Step};
use flux_view::{Editor, Mode};

use crate::ex;
use crate::insert::Insert;
use crate::key::{Key, KeyCode, Modifiers};
use crate::motion::Find;
use crate::parse::{self, Parse};
use crate::util::{Pos, pos};

#[derive(Debug, Default)]
pub struct Engine {
    /// Normal-mode keys of the command being typed.
    pending: Vec<Key>,
    pub(crate) last_find: Option<Find>,
    /// The last change, for `.`.
    dot: Option<Dot>,
    /// The change being recorded while its Insert mode runs.
    pub(crate) recording: Option<Dot>,
    pub(crate) insert: Option<Insert>,
    /// A Normal-mode command typed with `CTRL-O` from Insert mode.
    pub(crate) ctrl_o: Option<CtrlO>,
    /// Edits of the undo step being built.
    change: Option<ChangeBuilder>,
}

#[derive(Debug, Clone)]
pub(crate) struct Dot {
    pub keys: Vec<Key>,
    pub count: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CtrlO {
    /// The cursor was past the end of the line when `CTRL-O` was typed.
    pub at_eol: bool,
    pub line: usize,
}

#[derive(Debug)]
struct ChangeBuilder {
    before: Rope,
    edits: Vec<Edit>,
    inverse: Vec<Edit>,
    cursor_before: (usize, usize),
    no_lines_before: bool,
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    /// The keys of a partly typed Normal-mode command (Vim's 'showcmd').
    pub fn pending_keys(&self) -> &[Key] {
        &self.pending
    }

    pub fn handle_key(&mut self, editor: &mut Editor, key: Key) {
        // The hit-enter prompt: any key dismisses it; <CR>, <Space> and <Esc> do nothing else.
        if editor.hit_enter {
            editor.hit_enter = false;
            editor.message = None;
            if key == Key::plain(KeyCode::Enter)
                || key == Key::char(' ')
                || key == Key::plain(KeyCode::Esc)
            {
                return;
            }
        }
        // An unmapped Alt/Meta key is Esc followed by the key, as in Neovim. Terminals send Esc
        // quickly followed by a key the same way, so this is also what makes typing <Esc>:w
        // fast work.
        if key.mods == Modifiers::ALT
            && let KeyCode::Char(c) = key.code
        {
            self.handle_key(editor, Key::plain(KeyCode::Esc));
            self.handle_key(editor, Key::char(c));
            return;
        }
        match editor.mode {
            Mode::Normal => self.normal_key(editor, key),
            Mode::Insert => self.insert_key(editor, key),
            Mode::CmdLine => {
                cmdline(editor, key);
                if editor.mode == Mode::Normal && self.ctrl_o.is_some() {
                    self.finish_ctrl_o(editor);
                }
            }
        }
        editor.with_window(|win, m| win.scroll_to_cursor(m));
    }

    fn normal_key(&mut self, editor: &mut Editor, key: Key) {
        self.pending.push(key);
        match parse::parse(&self.pending) {
            Parse::Incomplete => {}
            Parse::Invalid => self.pending.clear(),
            Parse::Done(command) => {
                self.pending.clear();
                editor.with_window(|win, m| win.update_curswant(m, false));
                self.run(editor, command);
                if editor.mode == Mode::Normal {
                    self.commit(editor);
                    if self.ctrl_o.is_some() {
                        self.finish_ctrl_o(editor);
                    }
                }
            }
        }
    }

    /// Record `keys` (a finished change) for `.`.
    pub(crate) fn set_dot(&mut self, keys: Vec<Key>, count: Option<usize>) {
        if self.ctrl_o.is_none() {
            self.dot = Some(Dot { keys, count });
        }
    }

    /// `.`: replay the last change, with `count` replacing its count if given.
    pub(crate) fn repeat(&mut self, editor: &mut Editor, count: Option<usize>) {
        let Some(dot) = self.dot.clone() else {
            return;
        };
        let mut keys: Vec<Key> = count
            .or(dot.count)
            .map(|c| c.to_string().chars().map(Key::char).collect())
            .unwrap_or_default();
        keys.extend(dot.keys);
        for key in keys {
            self.handle_key(editor, key);
        }
    }

    /// Apply `edit` to the current buffer as part of the undo step being built.
    pub(crate) fn edit(&mut self, editor: &mut Editor, edit: Edit) {
        let cursor = editor.cursor();
        let text = &mut editor.current_buffer_mut().text;
        let builder = self.change.get_or_insert_with(|| ChangeBuilder {
            before: text.rope().clone(),
            edits: Vec::new(),
            inverse: Vec::new(),
            cursor_before: (cursor.line, cursor.col),
            no_lines_before: text.has_no_lines(),
        });
        let inverse = text.apply(&edit);
        editor.current_buffer_mut().uncommitted = true;
        builder.edits.push(edit);
        builder.inverse.push(inverse);
    }

    /// Finish the undo step being built, if any.
    pub(crate) fn commit(&mut self, editor: &mut Editor) {
        let Some(mut builder) = self.change.take() else {
            return;
        };
        let buffer = editor.current_buffer_mut();
        buffer.uncommitted = false;
        builder.inverse.reverse();
        let (before, after) =
            Change::changed_lines(&builder.before, buffer.text.rope(), &builder.edits);
        buffer.history.record(Change {
            edits: builder.edits,
            inverse: builder.inverse,
            cursor_before: builder.cursor_before,
            before,
            after,
            no_lines_before: builder.no_lines_before,
            no_lines_after: buffer.text.has_no_lines(),
        });
    }

    /// `u` (or `CTRL-R` when `redo`), `count` times.
    pub(crate) fn undo(&mut self, editor: &mut Editor, count: usize, redo: bool) {
        self.commit(editor);
        let mut last: Option<Step> = None;
        let (mut old_lines, mut new_lines) = (0, 0);
        for _ in 0..count {
            let buffer = editor.current_buffer_mut();
            let step = if redo {
                buffer.history.redo()
            } else {
                buffer.history.undo()
            };
            let Some(step) = step else { break };
            let change = &step.change;
            let (edits, region_before, region_after, no_lines) = if redo {
                (
                    &change.edits,
                    &change.before,
                    &change.after,
                    change.no_lines_after,
                )
            } else {
                (
                    &change.inverse,
                    &change.after,
                    &change.before,
                    change.no_lines_before,
                )
            };
            let was_empty = buffer.text.has_no_lines();
            for edit in edits {
                buffer.text.apply(edit);
            }
            buffer.text.set_no_lines(no_lines);
            // A buffer with no lines has 0 lines, not one empty one.
            old_lines += if was_empty { 0 } else { region_before.len() };
            new_lines += if no_lines { 0 } else { region_after.len() };
            let cursor = restore_cursor(editor, region_after.clone(), change.cursor_before);
            editor.window.cursor = cursor;
            last = Some(step);
        }
        let Some(step) = last else {
            editor.error(if redo {
                "Already at newest change"
            } else {
                "Already at oldest change"
            });
            return;
        };
        let diff = old_lines as isize - new_lines as isize;
        let (n, what) = match diff {
            -1 => (1, "more line"),
            d if d < 0 => (-d as usize, "more lines"),
            1 => (1, "line less"),
            d if d > 1 => (d as usize, "fewer lines"),
            _ if new_lines == 1 => (1, "change"),
            _ => (new_lines, "changes"),
        };
        // Whole seconds on both sides, like Vim's `time()` arithmetic.
        let unix = |t: SystemTime| {
            t.duration_since(SystemTime::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs())
        };
        let secs = unix(SystemTime::now()).saturating_sub(unix(step.time));
        let ago = if secs == 1 {
            "1 second ago".to_string()
        } else {
            format!("{secs} seconds ago")
        };
        let (when, seq) = if redo {
            ("after", step.seq)
        } else {
            ("before", step.seq)
        };
        editor.info(format!("{n} {what}; {when} #{seq}  {ago}"));
    }

    pub(crate) fn enter_cmdline(&mut self, editor: &mut Editor) {
        editor.mode = Mode::CmdLine;
        editor.cmdline.clear();
        editor.message = None;
    }
}

/// Where the cursor goes after undo or redo (Vim's `u_undoredo`): back to where it was before
/// the change if that's in or next to the changed lines, otherwise to the first changed line.
fn restore_cursor(editor: &Editor, changed: std::ops::Range<usize>, before: (usize, usize)) -> Pos {
    let text = editor.text();
    let (first, len) = (changed.start, changed.len());
    let last = text.last_line();
    if before.0 + 1 >= first && before.0 <= first + len {
        let line = before.0.min(last);
        let s = text.line_str(line);
        let col = before.1.min(flux_core::chars::last_grapheme(&s));
        return pos(line, col);
    }
    let line = first.min(last);
    pos(
        line,
        editor.metrics().col_for_vcol(line, editor.window.curswant),
    )
}

fn cmdline(editor: &mut Editor, key: Key) {
    let leave = |editor: &mut Editor| {
        editor.mode = Mode::Normal;
        editor.cmdline.clear();
    };
    match (key.code, key.mods) {
        (KeyCode::Enter, _) => {
            let line = std::mem::take(&mut editor.cmdline);
            editor.mode = Mode::Normal;
            // The typed command stays visible, as in Vim, unless the command reports something.
            editor.info(format!(":{line}"));
            ex::execute(editor, &line);
        }
        (KeyCode::Esc, _) | (KeyCode::Char('c'), Modifiers::CTRL) => leave(editor),
        (KeyCode::Backspace, _) | (KeyCode::Char('h'), Modifiers::CTRL) => {
            if editor.cmdline.pop().is_none() {
                leave(editor);
            }
        }
        (KeyCode::Char('u'), Modifiers::CTRL) => editor.cmdline.clear(),
        (KeyCode::Char('w'), Modifiers::CTRL) => delete_word_before(&mut editor.cmdline),
        (KeyCode::Char(c), Modifiers::NONE) => editor.cmdline.push(c),
        _ => {}
    }
}

/// `CTRL-W` on the command line: delete trailing spaces, then a run of keyword or non-keyword
/// characters.
fn delete_word_before(s: &mut String) {
    let trimmed = s.trim_end_matches(' ').len();
    s.truncate(trimmed);
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let Some(last) = s.chars().last() else {
        return;
    };
    let keep = s
        .char_indices()
        .rev()
        .find(|&(_, c)| is_word(c) != is_word(last) || c == ' ')
        .map_or(0, |(i, c)| i + c.len_utf8());
    s.truncate(keep);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_keys;

    fn feed(editor: &mut Editor, engine: &mut Engine, keys: &str) {
        for key in parse_keys(keys) {
            engine.handle_key(editor, key);
        }
    }

    #[test]
    fn cmdline_editing() {
        let mut editor = Editor::new(80, 24);
        let mut engine = Engine::new();
        feed(&mut editor, &mut engine, ":foo bar<C-w>");
        assert_eq!(editor.cmdline, "foo ");
        feed(&mut editor, &mut engine, "<BS><BS><BS><BS>");
        assert_eq!(editor.mode, Mode::CmdLine);
        feed(&mut editor, &mut engine, "<BS>");
        assert_eq!(editor.mode, Mode::Normal);
        feed(&mut editor, &mut engine, ":q<Esc>");
        assert!(!editor.quit);
        feed(&mut editor, &mut engine, ":1<CR>");
        assert_eq!(editor.message.as_ref().unwrap().text, ":1");
        feed(&mut editor, &mut engine, ":q<CR>");
        assert!(editor.quit);
    }

    #[test]
    fn ctrl_w_word_classes() {
        for (before, after) in [("a.b", "a."), ("foo  ", ""), ("x foo..", "x foo"), ("", "")] {
            let mut s = before.to_string();
            delete_word_before(&mut s);
            assert_eq!(s, after, "CTRL-W on {before:?}");
        }
    }

    #[test]
    fn alt_key_is_escape_then_key() {
        let mut editor = Editor::new(80, 24);
        editor.set_text("abc\n");
        let mut engine = Engine::new();
        feed(&mut editor, &mut engine, "iX<M-:>");
        assert_eq!(editor.mode, Mode::CmdLine);
        assert_eq!(editor.text().line_str(0), "Xabc");
    }

    #[test]
    fn undo_messages() {
        let mut editor = Editor::new(80, 24);
        editor.set_text("a\nb\nc\nd\n");
        let mut engine = Engine::new();
        feed(&mut editor, &mut engine, "u");
        assert_eq!(
            editor.message.as_ref().unwrap().text,
            "Already at oldest change"
        );
        feed(&mut editor, &mut engine, "3ddu");
        assert_eq!(
            editor.message.as_ref().unwrap().text,
            "3 more lines; before #1  0 seconds ago"
        );
        feed(&mut editor, &mut engine, "<C-r>");
        assert_eq!(
            editor.message.as_ref().unwrap().text,
            "3 fewer lines; after #1  0 seconds ago"
        );
        feed(&mut editor, &mut engine, "xu");
        assert_eq!(
            editor.message.as_ref().unwrap().text,
            "1 change; before #2  0 seconds ago"
        );
    }
}
