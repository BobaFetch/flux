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
    /// Insert-mode completion (see [`crate::completion`]).
    pub(crate) compl: crate::completion::Completion,
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
    /// The next command-line key is inserted literally (`CTRL-V`).
    cmdline_literal: bool,
    /// Browsing the command-line history: the entry shown (counting back from the newest) and
    /// the text typed before browsing, which entries must start with.
    history_at: Option<(usize, String)>,
    /// A command whose `/` or `?` motion is waiting for the pattern being typed.
    pub(crate) search_cmd: Option<crate::search::PendingSearch>,
    /// The pattern (and offset) typed for that command, while it runs.
    pub(crate) search_input: Option<String>,
    /// The view before 'incsearch' scrolled it.
    pub(crate) saved_view: Option<crate::search::SavedView>,
    /// While positive, edits keep going into the current undo step (`:normal`, `:g`).
    pub(crate) hold_undo: usize,
    /// An Ex command waiting for a yes/no answer (a backwards range to swap).
    pub(crate) confirm_swap: Option<String>,
    /// Swap a backwards range without asking (the answer was yes).
    pub(crate) swap_range: bool,
    /// A `:s///c` waiting for answers.
    pub(crate) confirm_sub: Option<crate::substitute::ConfirmSession>,
    /// Lines saved for undo by the change being built, when it isn't one range (`:s`).
    pub(crate) saved_lines: Option<(usize, usize)>,
    /// `:g` is running: its marked lines, in order (`None` once deleted).
    pub(crate) global_lines: Option<crate::global::MarkedLines>,
    /// While `:g` runs, `:s` adds up what it did here instead of reporting.
    pub(crate) global_subs: (usize, usize),
    /// The command line being run, as typed (output that starts below it shows it too).
    pub(crate) typed_cmdline: Option<String>,
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

    /// A Normal or Visual command is partly typed.
    pub(crate) fn has_pending(&self) -> bool {
        !self.pending.is_empty() || !self.visual_pending.is_empty()
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
        editor.keep_msg = false;
        if self.confirm_sub.is_some() {
            self.confirm_key(editor, key);
            editor.with_window(|win, m| win.scroll_to_cursor(m));
            return;
        }
        // A yes/no question (Vim's `ask_yesno`): only y, n, <Esc> and CTRL-C answer it.
        if let Some(line) = self.confirm_swap.take() {
            let answer = match (key.code, key.mods) {
                (KeyCode::Char('y'), Modifiers::NONE) => Some(true),
                (KeyCode::Char('n'), Modifiers::NONE) | (KeyCode::Esc, _) => Some(false),
                (KeyCode::Char('c'), Modifiers::CTRL) => Some(false),
                _ => None,
            };
            match answer {
                None => self.confirm_swap = Some(line),
                Some(yes) => {
                    // The answer is echoed after the question; what the command then reports
                    // goes below it.
                    let question = editor.message.take().map(|m| m.text).unwrap_or_default();
                    let echoed = format!("{question}{}", if yes { 'y' } else { 'n' });
                    if yes {
                        self.swap_range = true;
                        ex::run(self, editor, &line);
                        self.swap_range = false;
                        self.commit(editor);
                    }
                    match editor.message.take() {
                        Some(m) => editor.full_message(format!("{echoed}\n{}", m.text)),
                        None => editor.info(echoed),
                    }
                }
            }
            return;
        }
        // Paging through a long message (Vim's `-- More --`).
        if let Some(top) = editor.more_top {
            let page = editor.screen_size().1.saturating_sub(1).max(1);
            let last_top = editor.message_lines().len().saturating_sub(page);
            let at_end = top >= last_top;
            let ch = if key.mods == Modifiers::NONE {
                key.typed_char()
            } else {
                None
            };
            let back = match (key.code, ch) {
                (KeyCode::Up, _) | (_, Some('k')) => Some(top.saturating_sub(1)),
                (KeyCode::PageUp, _) | (_, Some('b')) => Some(top.saturating_sub(page)),
                (_, Some('u')) => Some(top.saturating_sub(editor.screen_size().1 / 2)),
                (_, Some('g')) => Some(0),
                _ => None,
            };
            editor.more_help = false;
            editor.more_max_row = editor.more_max_row.max(top + page);
            if let Some(t) = back {
                editor.more_top = Some(t);
                return;
            }
            if !at_end {
                let next = match (key.code, ch) {
                    (KeyCode::PageDown, _) | (_, Some(' ' | 'f')) => Some(top + page),
                    (KeyCode::Enter | KeyCode::Down, _) | (_, Some('j')) => Some(top + 1),
                    (_, Some('d')) => Some(top + editor.screen_size().1 / 2),
                    (_, Some('G')) => Some(last_top),
                    _ => None,
                };
                if let Some(t) = next {
                    let t = t.min(last_top);
                    editor.more_top = Some(t);
                    // Reaching the last page lets the command finish.
                    editor.more_max_row = if t >= last_top {
                        usize::MAX / 2
                    } else {
                        editor.more_max_row.max(t + page)
                    };
                    return;
                }
                let quit =
                    matches!(key.code, KeyCode::Esc) || ch == Some('q') || key == Key::ctrl('c');
                if quit || ch == Some(':') {
                    // Vim pages while the command runs, so quitting stops it at the first
                    // line not shown yet (scrolling back doesn't undo what ran).
                    let row = editor.more_max_row.max(top + page);
                    let line = editor.message_line_at(row);
                    let pos = editor
                        .message_positions
                        .get(..=line)
                        .and_then(|p| p.iter().rev().find_map(|c| *c));
                    if let Some(p) = pos {
                        // The view never followed the lines the command didn't reach.
                        if let Some(top) = editor.more_restore_top {
                            editor.window.top = top;
                        }
                        editor.window.cursor = p;
                        editor.with_window(|w, m| w.scroll_to_cursor(m));
                    }
                    editor.more_top = None;
                    editor.hit_enter = false;
                    editor.message = None;
                    if quit {
                        return;
                    }
                    // The command line comes up over the message, which stays until the
                    // command runs.
                    editor.stale_screen = true;
                } else {
                    editor.more_help = true;
                    return;
                }
            } else {
                editor.more_top = None;
            }
        }
        // The hit-enter prompt: any key dismisses it; <CR>, <Space> and <Esc> do nothing else.
        if editor.hit_enter {
            editor.hit_enter = false;
            editor.message = None;
            if key == Key::char(':') {
                editor.stale_screen = true;
            }
            if key == Key::plain(KeyCode::Enter)
                || key == Key::char(' ')
                || key == Key::char('q')
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
        // A kept message comes back when its command is over.
        if editor.mode == Mode::Normal
            && self.pending.is_empty()
            && self.insert.is_none()
            && let Some(m) = editor.kept_message.take()
            && !editor.hit_enter
        {
            editor.message = Some(m);
        }
        // Floats close when the cursor moves or another buffer is shown.
        editor.check_floats();
        // The number column may have grown or shrunk.
        editor.refresh_window_widths();
        editor.with_window(|win, m| win.scroll_to_cursor(m));
        editor.pum_ruler_check();
        self.snippet_check(editor);
    }

    fn normal_key(&mut self, editor: &mut Editor, key: Key) {
        if self.pending.is_empty()
            && editor.current_buffer().directory
            && self.explorer_key(editor, key)
        {
            return;
        }
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
                if self.search_input.is_none()
                    && let Some(forward) = typed_search(&command.action)
                {
                    self.start_search(
                        editor,
                        forward,
                        crate::search::PendingSearch::Normal(command),
                    );
                    return;
                }
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

    /// The keys of a directory listing, like netrw's: `<CR>` opens the entry under the cursor,
    /// `o` / `v` open it in a new window, `-` goes up a directory. Other keys work as usual.
    fn explorer_key(&mut self, editor: &mut Editor, key: Key) -> bool {
        let result = if key == Key::plain(KeyCode::Enter) {
            editor.open_entry()
        } else if key == Key::char('-') {
            editor.open_parent()
        } else if key == Key::char('o') || key == Key::char('v') {
            editor.open_entry_in_split(key == Key::char('v'))
        } else {
            return false;
        };
        if let Err(e) = result {
            editor.error(e);
            self.failed = true;
        }
        true
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
                ex::run(self, editor, &cmd.text);
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
        // Commands that would change a listing are refused before they get here; this catches
        // the rest (Ex commands, Insert mode entered some other way).
        if editor.current_buffer().directory {
            editor.error(flux_view::explorer::NOT_MODIFIABLE);
            self.failed = true;
            return;
        }
        let cursor = editor.cursor();
        let first = self.change.is_none();
        self.snippet_edit(editor, &edit);
        let shift = LineShift::of(editor.text(), &edit);
        editor.current_buffer_mut().marks.adjust(&shift);
        // Lines `:g` has yet to visit move along (and are forgotten when deleted).
        if let Some(lines) = self.global_lines.as_mut() {
            lines.adjust(&shift);
        }
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
        if self.hold_undo > 0 {
            return;
        }
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
            saved_lines: self.saved_lines.take(),
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
            let (saved_before, saved_after) = match change.saved_lines {
                Some((b, a)) if redo => (b, a),
                Some((b, a)) => (a, b),
                None => (region_before.len(), region_after.len()),
            };
            old_lines += if was_empty { 0 } else { saved_before };
            new_lines += if no_lines { 0 } else { saved_after };
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
        // The view to go back to after 'incsearch' or 'inccommand' moved it.
        self.saved_view = Some(crate::search::SavedView::of(editor));
        editor.mode = Mode::CmdLine;
        editor.cmdline.clear();
        editor.cmdline_pos = 0;
        editor.cmdline_kind = ':';
        editor.message = None;
        self.history_at = None;
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
        let before = editor.cmdline.clone();
        self.cmdline_edit(editor, key);
        if editor.mode == Mode::CmdLine && editor.cmdline != before {
            self.update_incsearch(editor);
        }
        // 'inccommand': preview `:s` while it's typed, with the cursor on its first match.
        editor.preview = None;
        if editor.mode == Mode::CmdLine && editor.cmdline_kind == ':' {
            if let Some(v) = self.saved_view {
                editor.window.top = v.top;
                editor.window.cursor = v.cursor;
            }
            let botline = editor.with_window(|w, m| w.bottom(m) + 1);
            editor.preview = crate::substitute::preview(editor, &editor.cmdline, botline);
            // 'incsearch' for the pattern even if the rest doesn't parse.
            let cursor = match &editor.preview {
                Some(p) => p.first_match,
                None => crate::substitute::pattern_match(editor, &editor.cmdline),
            };
            if let Some(p) = cursor {
                let line = p.line.min(editor.text().last_line());
                // The cursor may be past the end of the line here (a match of `\n`).
                let col = p.col.min(editor.text().line_len(line));
                editor.window.cursor = crate::util::pos(line, col);
                editor.with_window(|w, m| w.scroll_to_cursor(m));
            }
        }
    }

    fn cmdline_edit(&mut self, editor: &mut Editor, key: Key) {
        if std::mem::take(&mut self.cmdline_register) {
            if let Some(name) = key.typed_char()
                && let Some(reg) = editor.register(Some(name))
            {
                let text = reg.text.trim_end_matches('\n').replace('\n', "\r");
                insert_at_cursor(editor, &text);
            }
            return;
        }
        if std::mem::take(&mut self.cmdline_literal) {
            if let Some(c) = literal_char(key) {
                insert_at_cursor(editor, &c.to_string());
            }
            return;
        }
        let pos = editor.cmdline_pos.min(editor.cmdline.chars().count());
        let len = editor.cmdline.chars().count();
        if !matches!(
            key.code,
            KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown
        ) && !matches!(
            (key.code, key.mods),
            (KeyCode::Char('p' | 'n'), Modifiers::CTRL)
        ) {
            self.history_at = None;
        }
        match (key.code, key.mods) {
            (KeyCode::Enter, _) | (KeyCode::Char('m' | 'j'), Modifiers::CTRL) => {
                let line = std::mem::take(&mut editor.cmdline);
                editor.cmdline_pos = 0;
                editor.mode = Mode::Normal;
                if editor.cmdline_kind == ':' {
                    self.restore_view(editor);
                }
                if editor.cmdline_kind != ':' {
                    self.finish_search(editor, Some(line));
                    return;
                }
                editor.search.add_history(false, &line);
                // The typed command stays visible, as in Vim, unless the command reports
                // something.
                editor.info(format!(":{line}"));
                self.typed_cmdline = Some(line.clone());
                ex::run(self, editor, &line);
                self.typed_cmdline = None;
                self.commit(editor);
                // `":` holds the last command once it has run.
                if !line.trim().is_empty() {
                    editor.registers.set_readonly(':', line);
                }
            }
            (KeyCode::Esc, _) | (KeyCode::Char('c'), Modifiers::CTRL) => self.leave_cmdline(editor),
            (KeyCode::Backspace, _) | (KeyCode::Char('h'), Modifiers::CTRL) => {
                if editor.cmdline.is_empty() {
                    self.leave_cmdline(editor);
                } else if pos > 0 {
                    remove_chars(editor, pos - 1, pos);
                }
            }
            (KeyCode::Delete, _) => {
                if pos < len {
                    remove_chars(editor, pos, pos + 1);
                } else if pos > 0 {
                    remove_chars(editor, pos - 1, pos);
                } else if editor.cmdline.is_empty() {
                    self.leave_cmdline(editor);
                }
            }
            (KeyCode::Char('u'), Modifiers::CTRL) => remove_chars(editor, 0, pos),
            (KeyCode::Char('w'), Modifiers::CTRL) => {
                let head: String = editor.cmdline.chars().take(pos).collect();
                let mut cut = head.clone();
                delete_word_before(&mut cut);
                let from = cut.chars().count();
                remove_chars(editor, from, pos);
            }
            (KeyCode::Char('r'), Modifiers::CTRL) => self.cmdline_register = true,
            (KeyCode::Char('v' | 'q'), Modifiers::CTRL) => self.cmdline_literal = true,
            (KeyCode::Left, m) if m == Modifiers::NONE => {
                editor.cmdline_pos = pos.saturating_sub(1)
            }
            (KeyCode::Right, m) if m == Modifiers::NONE => editor.cmdline_pos = (pos + 1).min(len),
            (KeyCode::Left, _) => editor.cmdline_pos = word_left(&editor.cmdline, pos),
            (KeyCode::Right, _) => editor.cmdline_pos = word_right(&editor.cmdline, pos),
            (KeyCode::Home, _) | (KeyCode::Char('b'), Modifiers::CTRL) => editor.cmdline_pos = 0,
            (KeyCode::End, _) | (KeyCode::Char('e'), Modifiers::CTRL) => editor.cmdline_pos = len,
            (KeyCode::Up, m) | (KeyCode::Down, m) => {
                let older = key.code == KeyCode::Up;
                // Up/Down match what was typed; Shift or CTRL-P/CTRL-N don't.
                self.browse_history(editor, older, m == Modifiers::NONE);
            }
            (KeyCode::PageUp, _) => self.browse_history(editor, true, false),
            (KeyCode::PageDown, _) => self.browse_history(editor, false, false),
            (KeyCode::Char('p'), Modifiers::CTRL) => self.browse_history(editor, true, false),
            (KeyCode::Char('n'), Modifiers::CTRL) => self.browse_history(editor, false, false),
            (KeyCode::Tab, m) if m == Modifiers::NONE => insert_at_cursor(editor, "\t"),
            (KeyCode::Char(c), Modifiers::NONE) => insert_at_cursor(editor, &c.to_string()),
            _ => {}
        }
    }

    /// `<Esc>` on the command line.
    fn leave_cmdline(&mut self, editor: &mut Editor) {
        editor.mode = Mode::Normal;
        editor.cmdline.clear();
        editor.cmdline_pos = 0;
        if editor.cmdline_kind == ':' {
            self.restore_view(editor);
        }
        if editor.cmdline_kind != ':' {
            self.finish_search(editor, None);
        }
    }

    /// `<Up>`/`<Down>` (matching the typed prefix) and `<S-Up>`/`CTRL-P`, … on the command
    /// line.
    fn browse_history(&mut self, editor: &mut Editor, older: bool, prefix: bool) {
        let history = if editor.cmdline_kind == ':' {
            &editor.search.cmd_history
        } else {
            &editor.search.search_history
        };
        let (at, typed) = self
            .history_at
            .clone()
            .unwrap_or((0, editor.cmdline.clone()));
        let matches = |e: &String| !prefix || e.starts_with(&typed);
        // `at` counts back from the newest entry; 0 is the typed text itself.
        let mut next = at;
        let found = loop {
            if older {
                next += 1;
                if next > history.len() {
                    break None;
                }
            } else {
                if next == 0 {
                    break None;
                }
                next -= 1;
                if next == 0 {
                    break Some(typed.clone());
                }
            }
            let entry = &history[history.len() - next];
            if matches(entry) {
                break Some(entry.clone());
            }
        };
        if let Some(text) = found {
            editor.cmdline_pos = text.chars().count();
            editor.cmdline = text;
            self.history_at = Some((next, typed));
            self.update_incsearch(editor);
        }
    }
}

fn insert_at_cursor(editor: &mut Editor, s: &str) {
    let pos = editor.cmdline_pos.min(editor.cmdline.chars().count());
    let at = editor
        .cmdline
        .char_indices()
        .nth(pos)
        .map_or(editor.cmdline.len(), |(i, _)| i);
    editor.cmdline.insert_str(at, s);
    editor.cmdline_pos = pos + s.chars().count();
}

/// Remove chars `[from, to)` of the command line, leaving the cursor at `from`.
fn remove_chars(editor: &mut Editor, from: usize, to: usize) {
    let s: String = editor
        .cmdline
        .chars()
        .enumerate()
        .filter(|&(i, _)| i < from || i >= to)
        .map(|(_, c)| c)
        .collect();
    editor.cmdline = s;
    editor.cmdline_pos = from;
}

/// `<S-Left>` on the command line: to the start of the previous word.
fn word_left(s: &str, pos: usize) -> usize {
    let chars: Vec<char> = s.chars().collect();
    let mut i = pos;
    while i > 0 && chars[i - 1] == ' ' {
        i -= 1;
    }
    while i > 0 && chars[i - 1] != ' ' {
        i -= 1;
    }
    i
}

/// `<S-Right>`: past the next word and the blanks after it.
fn word_right(s: &str, pos: usize) -> usize {
    let chars: Vec<char> = s.chars().collect();
    let mut i = pos;
    while i < chars.len() && chars[i] != ' ' {
        i += 1;
    }
    while i < chars.len() && chars[i] == ' ' {
        i += 1;
    }
    i
}

/// The character `CTRL-V {key}` inserts.
fn literal_char(key: Key) -> Option<char> {
    match (key.code, key.mods) {
        (KeyCode::Char(c), m) if m == Modifiers::CTRL && c.is_ascii_alphabetic() => {
            Some(char::from(c.to_ascii_uppercase() as u8 - b'@'))
        }
        (KeyCode::Char(c), _) => Some(c),
        (KeyCode::Tab, _) => Some('\t'),
        (KeyCode::Enter, _) => Some('\r'),
        (KeyCode::Esc, _) => Some('\x1b'),
        _ => None,
    }
}

/// The direction of a `/` or `?` in a command, which needs a pattern typed first.
fn typed_search(action: &parse::Action) -> Option<bool> {
    use crate::motion::Motion;
    use parse::{Action, OpTarget};
    match action {
        Action::Move(Motion::Search { forward })
        | Action::Operate(_, OpTarget::Motion(Motion::Search { forward })) => Some(*forward),
        _ => None,
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

    /// A directory tree for the listing tests: `dir/{b.txt, a.txt, sub/c.txt}`.
    fn temp_tree(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("flux-explorer-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("b.txt"), "bee\n").unwrap();
        std::fs::write(dir.join("a.txt"), "ay\n").unwrap();
        std::fs::write(dir.join("sub/c.txt"), "sea\n").unwrap();
        dir
    }

    fn lines(editor: &Editor) -> Vec<String> {
        let text = editor.text();
        (0..text.line_count())
            .map(|l| text.line_str(l).into_owned())
            .collect()
    }

    #[test]
    fn browsing_a_directory() {
        let dir = temp_tree("browse");
        let mut editor = Editor::new(80, 24);
        editor.cwd = dir.clone();
        editor.open_args(&[".".into()]);
        let mut engine = Engine::new();
        assert!(editor.current_buffer().directory);
        assert_eq!(lines(&editor), ["../", "sub/", "a.txt", "b.txt"]);
        assert_eq!(editor.current_buffer().path.as_deref(), Some(dir.as_path()));
        // Into a directory and a file, then back up with `-` and `:Ex`.
        feed(&mut editor, &mut engine, "j<CR>");
        assert_eq!(lines(&editor), ["../", "c.txt"]);
        feed(&mut editor, &mut engine, "j<CR>");
        assert_eq!(lines(&editor), ["sea"]);
        assert_eq!(editor.current_buffer().name(), "sub/c.txt");
        feed(&mut editor, &mut engine, ":Ex<CR>");
        assert_eq!(lines(&editor), ["../", "c.txt"]);
        assert_eq!(editor.cursor().line, 1, "on the file just left");
        feed(&mut editor, &mut engine, "-");
        assert_eq!(lines(&editor), ["../", "sub/", "a.txt", "b.txt"]);
        assert_eq!(editor.cursor().line, 1, "on the directory just left");
        // `../` goes up as well.
        feed(&mut editor, &mut engine, "gg<CR>");
        assert_eq!(editor.current_buffer().path.as_deref(), dir.parent());
        // Listings are read again when shown, and aren't listed by `:ls`.
        std::fs::write(dir.join("new.txt"), "").unwrap();
        feed(
            &mut editor,
            &mut engine,
            &format!(":e {}<CR>", dir.display()),
        );
        assert_eq!(lines(&editor), ["../", "sub/", "a.txt", "b.txt", "new.txt"]);
        assert!(
            editor
                .listed_buffers()
                .iter()
                .all(|&b| !editor.buffer(b).unwrap().directory)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn listings_cannot_be_changed() {
        let dir = temp_tree("readonly");
        let mut editor = Editor::new(80, 24);
        editor.cwd = dir.clone();
        editor.open_args(&[".".into()]);
        let mut engine = Engine::new();
        let before = lines(&editor);
        for keys in [
            "dd",
            "x",
            "ixy<Esc>",
            "Vjd",
            "p",
            ":%s/a/b/<CR>",
            ":1d<CR>",
            "J",
        ] {
            feed(&mut editor, &mut engine, keys);
            assert_eq!(lines(&editor), before, "{keys}");
            assert_eq!(editor.mode, Mode::Normal, "{keys}");
        }
        assert!(editor.message.as_ref().unwrap().text.starts_with("E21"));
        feed(&mut editor, &mut engine, ":w<CR>");
        assert!(editor.message.as_ref().unwrap().text.starts_with("E502"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn opening_entries_in_splits() {
        let dir = temp_tree("split");
        let mut editor = Editor::new(80, 24);
        editor.cwd = dir.clone();
        editor.open_args(&["a.txt".into()]);
        let mut engine = Engine::new();
        feed(&mut editor, &mut engine, ":Vex<CR>");
        assert_eq!(editor.window_ids().len(), 2);
        assert_eq!(editor.cursor().line, 2, "on a.txt");
        feed(&mut editor, &mut engine, "jo");
        assert_eq!(editor.window_ids().len(), 3);
        assert_eq!(lines(&editor), ["bee"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn filetype_and_syntax_commands() {
        let mut editor = Editor::new(80, 24);
        let mut engine = Engine::new();
        let msg = |e: &Editor| {
            e.message
                .as_ref()
                .map(|m| m.text.clone())
                .unwrap_or_default()
        };
        feed(&mut editor, &mut engine, ":filetype<CR>");
        assert_eq!(msg(&editor), "filetype detection:ON  plugin:ON  indent:ON");
        feed(
            &mut editor,
            &mut engine,
            ":filetype indent off<CR>:filetype<CR>",
        );
        assert_eq!(msg(&editor), "filetype detection:ON  plugin:ON  indent:OFF");
        feed(
            &mut editor,
            &mut engine,
            ":filetype plugin indent on<CR>:filetype off<CR>:filet<CR>",
        );
        assert_eq!(
            msg(&editor),
            "filetype detection:OFF  plugin:(on)  indent:(on)"
        );
        feed(&mut editor, &mut engine, ":filetype bogus<CR>");
        assert_eq!(msg(&editor), "E475: Invalid argument: bogus");
        feed(&mut editor, &mut engine, ":syntax<CR>");
        assert_eq!(msg(&editor), "No Syntax items defined for this buffer");
        feed(&mut editor, &mut engine, ":syntax bogus<CR>");
        assert_eq!(msg(&editor), "E410: Invalid :syntax subcommand: bogus");
        feed(&mut editor, &mut engine, ":syntax off<CR>");
        assert!(!editor.syntax_on);
        feed(&mut editor, &mut engine, ":sy on<CR>");
        assert!(editor.syntax_on);
        // Setting 'filetype' applies the filetype's settings (Vim's FileType event).
        feed(&mut editor, &mut engine, ":set ft=rust<CR>");
        assert_eq!(editor.buf_opts().shiftwidth, 4);
        assert!(editor.buf_opts().expandtab);
        assert_eq!(editor.buf_opts().indentexpr, "GetRustIndent(v:lnum)");
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
