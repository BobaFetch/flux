//! The engine: routes keys to the current mode, groups edits into undo steps, and records the
//! last change for `.`.

use std::time::SystemTime;

use flux_core::{Change, Edit, Rope, Step};
use flux_view::{Editor, LineShift, Mode, VisualKind};

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
    /// Visual-mode keys of the command being typed.
    pub(crate) visual_pending: Vec<Key>,
    /// Keys typed since `q{reg}` started recording.
    macro_keys: Vec<Key>,
    /// The register `@@` runs again.
    last_executed: Option<char>,
    /// The last command failed (Vim beeps). A running macro stops.
    pub(crate) failed: bool,
    /// The next command-line key names a register to insert (`CTRL-R`).
    cmdline_register: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct Dot {
    pub keys: Vec<Key>,
    pub count: Option<usize>,
    /// For a Visual-mode operator: the selection to repeat it on.
    pub visual: Option<VisualDot>,
}

/// The size of a Visual selection, for repeating its operator with `.` on the same amount of
/// text (Vim's `redo_VIsual`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct VisualDot {
    pub kind: VisualKind,
    pub lines: usize,
    /// Charwise within one line: its width in columns. Over several lines: the end's column.
    pub vcol: usize,
    /// Selected to the end of the line (`$`).
    pub eol: bool,
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

    /// A key typed by the user.
    pub fn handle_key(&mut self, editor: &mut Editor, key: Key) {
        if editor.recording.is_some() {
            self.macro_keys.push(key);
        }
        self.process_key(editor, key);
    }

    /// A key, typed or replayed (macros, `.`).
    pub(crate) fn process_key(&mut self, editor: &mut Editor, key: Key) {
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
            self.process_key(editor, Key::plain(KeyCode::Esc));
            self.process_key(editor, Key::char(c));
            return;
        }
        match editor.mode {
            Mode::Normal => self.normal_key(editor, key),
            Mode::Insert => self.insert_key(editor, key),
            Mode::Visual => self.visual_key(editor, key),
            Mode::CmdLine => {
                self.cmdline_key(editor, key);
                if editor.mode == Mode::Normal && self.ctrl_o.is_some() {
                    self.finish_ctrl_o(editor);
                }
            }
        }
        editor.with_window(|win, m| win.scroll_to_cursor(m));
    }

    fn normal_key(&mut self, editor: &mut Editor, key: Key) {
        self.pending.push(key);
        match parse::parse(&self.pending, editor.recording.is_some()) {
            Parse::Incomplete => {}
            Parse::Invalid => {
                self.pending.clear();
                self.failed = true;
            }
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

    /// Record a finished change for `.`.
    pub(crate) fn set_dot(&mut self, dot: Dot) {
        if self.ctrl_o.is_none() {
            self.dot = Some(dot);
        }
    }

    /// `.`: replay the last change, with `count` replacing its count if given.
    pub(crate) fn repeat(&mut self, editor: &mut Editor, count: Option<usize>) {
        let Some(dot) = self.dot.as_mut() else {
            self.failed = true;
            return;
        };
        // `"1p` repeated puts from `"2`, `"3`, … (`:h redo-register`).
        if let [q, n, ..] = dot.keys.as_mut_slice()
            && *q == Key::char('"')
            && let Some(d @ '1'..='8') = n.typed_char()
        {
            *n = Key::char(char::from(d as u8 + 1));
        }
        let dot = dot.clone();
        let count = count.or(dot.count);
        let mut keys: Vec<Key> = count
            .filter(|_| dot.visual.is_none())
            .map(|c| c.to_string().chars().map(Key::char).collect())
            .unwrap_or_default();
        keys.extend(dot.keys);
        if let Some(v) = dot.visual {
            self.reselect_for_repeat(editor, v);
        }
        for key in keys {
            self.process_key(editor, key);
        }
    }

    /// `q{reg}`: start recording keys into register `name`.
    pub(crate) fn start_recording(&mut self, editor: &mut Editor, name: char) {
        editor.recording = Some(name);
        self.macro_keys.clear();
    }

    /// `q`: stop recording and store the keys, without the `q` that ended it.
    pub(crate) fn stop_recording(&mut self, editor: &mut Editor) {
        let Some(name) = editor.recording.take() else {
            return;
        };
        self.macro_keys.pop();
        let text = crate::key::keys_to_text(&self.macro_keys);
        self.macro_keys.clear();
        editor.registers.yank(
            Some(name),
            flux_view::Register::new(text, flux_view::RegisterKind::Char),
        );
    }

    /// `@{reg}`: run a register's contents as keys, `count` times, stopping at the first
    /// failure like Vim.
    pub(crate) fn execute_register(&mut self, editor: &mut Editor, name: char, count: usize) {
        let name = match name {
            '@' => match self.last_executed {
                Some(n) => n,
                None => {
                    editor.error("E748: No previously used register");
                    return;
                }
            },
            n => n,
        };
        self.last_executed = Some(name);
        if name == ':' {
            let Some(cmd) = editor.register(Some(':')) else {
                editor.error("E30: No previous command line");
                return;
            };
            for _ in 0..count {
                ex::execute(editor, &cmd.text);
            }
            return;
        }
        let Some(reg) = editor.register(Some(name)) else {
            self.failed = true;
            return;
        };
        let mut keys = crate::key::text_to_keys(&reg.text);
        if reg.kind == flux_view::RegisterKind::Line {
            keys.push(Key::plain(KeyCode::Enter));
        }
        let errors = editor.error_count;
        self.failed = false;
        'outer: for _ in 0..count {
            for &key in &keys {
                self.process_key(editor, key);
                if self.failed || editor.error_count != errors {
                    self.pending.clear();
                    self.visual_pending.clear();
                    break 'outer;
                }
            }
        }
    }

    /// Apply `edit` to the current buffer as part of the undo step being built, moving marks
    /// and setting `'[`, `']` and `'.`.
    pub(crate) fn edit(&mut self, editor: &mut Editor, edit: Edit) {
        let cursor = editor.cursor();
        let first = self.change.is_none();
        let shift = LineShift::of(editor.text(), &edit);
        editor.current_buffer_mut().marks.adjust(&shift);
        editor.adjust_other_windows(&shift);
        if let Some(p) = editor.window.pcmark {
            editor.window.pcmark = shift.adjust(p).or(Some(p));
        }
        let text = &mut editor.current_buffer_mut().text;
        let builder = self.change.get_or_insert_with(|| ChangeBuilder {
            before: text.rope().clone(),
            edits: Vec::new(),
            inverse: Vec::new(),
            cursor_before: (cursor.line, cursor.col),
            no_lines_before: text.has_no_lines(),
        });
        let inverse = text.apply(&edit);
        let (sl, sc) = text.char_to_pos(edit.at);
        let inserted = edit.insert.chars().count();
        let (el, ec) = text.char_to_pos(edit.at + inserted.saturating_sub(1));
        let buffer = editor.current_buffer_mut();
        buffer.uncommitted = true;
        let (start, end) = (pos(sl, sc), pos(el, ec));
        let marks = &mut buffer.marks;
        let keep = |name, p: Pos, pick_min: bool| match marks.get(name) {
            Some(old) if !first && (crate::util::before(old, p) == pick_min) => old,
            _ => p,
        };
        let (open, close) = (keep('[', start, true), keep(']', end, false));
        marks.set('[', open);
        marks.set(']', close);
        marks.set('.', start);
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

impl Engine {
    fn cmdline_key(&mut self, editor: &mut Editor, key: Key) {
        if std::mem::take(&mut self.cmdline_register) {
            if let Some(name) = key.typed_char()
                && let Some(reg) = editor.register(Some(name))
            {
                let text = reg.text.trim_end_matches('\n').replace('\n', "\r");
                editor.cmdline.push_str(&text);
            }
            return;
        }
        let leave = |editor: &mut Editor| {
            editor.mode = Mode::Normal;
            editor.cmdline.clear();
        };
        match (key.code, key.mods) {
            (KeyCode::Enter, _) => {
                let line = std::mem::take(&mut editor.cmdline);
                editor.mode = Mode::Normal;
                // The typed command stays visible, as in Vim, unless the command reports
                // something.
                editor.info(format!(":{line}"));
                ex::execute(editor, &line);
                // `":` holds the last command once it has run.
                if !line.trim().is_empty() {
                    editor.registers.set_readonly(':', line);
                }
            }
            (KeyCode::Esc, _) | (KeyCode::Char('c'), Modifiers::CTRL) => leave(editor),
            (KeyCode::Backspace, _) | (KeyCode::Char('h'), Modifiers::CTRL) => {
                if editor.cmdline.pop().is_none() {
                    leave(editor);
                }
            }
            (KeyCode::Char('u'), Modifiers::CTRL) => editor.cmdline.clear(),
            (KeyCode::Char('w'), Modifiers::CTRL) => delete_word_before(&mut editor.cmdline),
            (KeyCode::Char('r'), Modifiers::CTRL) => self.cmdline_register = true,
            (KeyCode::Char(c), Modifiers::NONE) => editor.cmdline.push(c),
            _ => {}
        }
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
    fn deleting_everything_from_far_down_the_file() {
        let text: String = (1..=200).map(|i| format!("{i}\n")).collect();
        for keys in ["GvipdG", "GdggG", "GVggd"] {
            let mut editor = Editor::new(80, 24);
            editor.set_text(&text);
            let mut engine = Engine::new();
            feed(&mut editor, &mut engine, keys);
            assert_eq!(editor.text().line_str(0), "", "{keys}");
            assert_eq!(editor.window.top, 0, "{keys}");
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
