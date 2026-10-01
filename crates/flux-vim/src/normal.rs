//! Running Normal-mode commands: motions, operators, and the commands that change text directly.

use flux_core::{Edit, chars};
use flux_view::{Editor, Mode, Register, RegisterKind};

use crate::engine::{Dot, Engine};
use crate::ex;
use crate::insert::InsertKind;
use crate::motion::{self, Context, Kind, Motion, Pending, Target, Want};
use crate::parse::{Action, Command, InsertAt, OpTarget, Operator, Scroll};
use crate::textobj;
use crate::util::{self, Pos, before, pos};

/// The text an operator works on.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Range {
    pub start: Pos,
    pub end: Pos,
    pub linewise: bool,
    /// Charwise: `end` is included.
    pub inclusive: bool,
    pub numbered_register: bool,
}

impl Engine {
    pub(crate) fn run(&mut self, editor: &mut Editor, cmd: Command) {
        let Command {
            register,
            count,
            action,
            keys,
        } = cmd;
        let dot = Dot {
            keys,
            count,
            visual: None,
        };
        if editor.current_buffer().nomodifiable() && changes_text(&action) {
            editor.error(flux_view::explorer::NOT_MODIFIABLE);
            self.failed = true;
            return;
        }
        let done = match action {
            Action::Move(motion::Motion::Mark { name, exact })
                if self.mark_in_other_buffer(editor, name) =>
            {
                // A global mark in another file: switch to it, then go to the mark.
                let (buffer, p) = editor.global_marks[&name];
                editor.show_buffer(buffer);
                let line = p.line.min(editor.text().last_line());
                let col = if exact {
                    p.col
                } else {
                    util::first_non_blank(&util::line(editor, line))
                };
                editor.window.cursor = pos(line, col);
                normalize_cursor(editor);
                set_want(editor, Want::Column);
                true
            }
            Action::Move(m) => match self.motion(editor, m, count, Pending::None) {
                Some(t) => {
                    self.remember_find(m);
                    if m.is_jump() {
                        set_pcmark(editor);
                    }
                    self.move_to(editor, t);
                    true
                }
                None => {
                    mark_not_set(editor, m);
                    false
                }
            },
            Action::Operate(op, target) => {
                let done = self.operate(editor, op, target, count, register);
                if done && op.changes_text() {
                    if op == Operator::Change {
                        self.recording = Some(dot);
                    } else {
                        self.set_dot(dot);
                    }
                }
                done
            }
            Action::Put { before } => {
                let done = self.put(editor, register, count.unwrap_or(1), before);
                if done {
                    self.set_dot(dot);
                }
                done
            }
            Action::Replace(c) => {
                let done = self.replace(editor, count.unwrap_or(1), c);
                if done {
                    self.set_dot(dot);
                }
                done
            }
            Action::Join { spaces } => {
                let done = self.join(editor, count, spaces);
                if done {
                    self.set_dot(dot);
                }
                done
            }
            Action::ToggleCase => {
                let done = self.toggle_case(editor, count.unwrap_or(1));
                if done {
                    self.set_dot(dot);
                }
                done
            }
            Action::RepeatSubstitute { all } => {
                // Like the Ex command it runs, not repeated by `.`.
                let _ = dot;
                ex::run(self, editor, if all { "%s//~/&" } else { "s" });
                true
            }
            Action::Insert(at) => {
                self.recording = Some(dot);
                self.start_insert(editor, at, count.unwrap_or(1));
                true
            }
            other => {
                self.run_other(editor, other, count);
                true
            }
        };
        if !done {
            self.failed = true;
        }
    }

    fn run_other(&mut self, editor: &mut Editor, action: Action, count: Option<usize>) {
        match action {
            Action::Move(_)
            | Action::Operate(..)
            | Action::Put { .. }
            | Action::Replace(_)
            | Action::Join { .. }
            | Action::ToggleCase
            | Action::RepeatSubstitute { .. }
            | Action::Insert(_) => unreachable!("handled in run"),
            Action::Undo => self.undo(editor, count.unwrap_or(1), false),
            Action::Redo => self.undo(editor, count.unwrap_or(1), true),
            Action::Repeat => self.repeat(editor, count),
            Action::CmdLine => {
                self.enter_cmdline(editor);
                // A count becomes a range of that many lines, as in Vim.
                match count {
                    Some(1) => editor.cmdline.push('.'),
                    Some(n) => editor.cmdline.push_str(&format!(".,.+{}", n - 1)),
                    None => {}
                }
                editor.cmdline_pos = editor.cmdline.chars().count();
            }
            Action::Scroll(s) => scroll(editor, s, count),
            Action::FileInfo => file_info(editor),
            Action::WriteQuit => ex::run(self, editor, "x"),
            Action::QuitDiscard => ex::run(self, editor, "q!"),
            Action::Redraw => {}
            Action::SetMark(name) => {
                let cur = editor.cursor();
                match name {
                    '\'' | '`' => set_pcmark(editor),
                    'A'..='Z' => {
                        let buffer = editor.window.buffer;
                        editor.global_marks.insert(name, (buffer, cur));
                    }
                    _ => editor.current_buffer_mut().marks.set(name, cur),
                }
            }
            Action::Record(name) => self.start_recording(editor, name),
            Action::StopRecord => self.stop_recording(editor),
            Action::Execute(name) => self.execute_register(editor, name, count.unwrap_or(1)),
            Action::Jump { older } => {
                let n = count.unwrap_or(1) as isize;
                let here = flux_view::Jump {
                    buffer: editor.window.buffer,
                    pos: editor.cursor(),
                };
                match editor.window.jumps.jump(if older { n } else { -n }, here) {
                    Some(j) => {
                        if j.buffer != editor.window.buffer {
                            if editor.buffer(j.buffer).is_none() {
                                self.failed = true;
                                return;
                            }
                            editor.switch_buffer(j.buffer, false);
                        }
                        let line = j.pos.line.min(editor.text().last_line());
                        editor.window.cursor = pos(line, j.pos.col);
                        normalize_cursor(editor);
                        set_want(editor, Want::Column);
                    }
                    None => self.failed = true,
                }
            }
            Action::Visual(kind) => self.start_visual(editor, kind),
            Action::Window(cmd) => self.window_command(editor, cmd, count),
            Action::AlternateBuffer => self.alternate_buffer(editor, count),
            Action::ScrollCursor {
                at,
                first_non_blank,
            } => {
                if let Some(n) = count {
                    let line = n.max(1) - 1;
                    editor.with_window(|w, m| w.set_cursor_line(line, m));
                }
                if first_non_blank {
                    let line = editor.cursor().line;
                    editor.window.cursor.col = util::first_non_blank(&util::line(editor, line));
                    set_want(editor, Want::Column);
                }
                let at = match at {
                    crate::parse::ScreenPos::Top => 't',
                    crate::parse::ScreenPos::Middle => 'z',
                    crate::parse::ScreenPos::Bottom => 'b',
                };
                editor.with_window(|w, m| w.scroll_cursor_to(at, m));
            }
            Action::Reselect => self.reselect(editor),
            Action::Lsp(cmd) => self.lsp_command(editor, cmd, count),
            Action::ExCount { cmd, count1 } => {
                let n = count.or(count1.then_some(1));
                let arg = n.map(|n| format!(" {n}")).unwrap_or_default();
                crate::ex::run(self, editor, &format!("{cmd}{arg}"));
            }
            Action::InsertAtLastInsert => {
                if let Some(p) = editor.current_buffer().marks.get('^') {
                    let line = p.line.min(editor.text().last_line());
                    let len = editor.text().line_len(line);
                    editor.window.cursor = pos(line, p.col.min(len));
                }
                self.recording = Some(Dot {
                    keys: vec![crate::key::Key::char('i')],
                    count: None,
                    visual: None,
                });
                self.start_insert(editor, InsertAt::Cursor, count.unwrap_or(1));
            }
        }
    }

    pub(crate) fn motion(
        &mut self,
        editor: &mut Editor,
        motion: Motion,
        count: Option<usize>,
        pending: Pending,
    ) -> Option<Target> {
        if motion.is_search() {
            return self.search_motion(editor, motion, count, pending);
        }
        motion::eval(
            motion,
            &Context {
                editor,
                count,
                pending,
                last_find: self.last_find,
            },
        )
    }

    fn remember_find(&mut self, motion: Motion) {
        if let Motion::Find(f) = motion {
            self.last_find = Some(f);
        }
    }

    /// Move the cursor to a motion's target.
    fn move_to(&mut self, editor: &mut Editor, t: Target) {
        let text = editor.text();
        let line = t.pos.line.min(text.last_line());
        let s = text.line_str(line);
        let col = chars::snap_to_grapheme(&s, t.pos.col.min(chars::last_grapheme(&s)));
        editor.window.cursor = pos(line, col);
        set_want(editor, t.want);
    }

    fn operate(
        &mut self,
        editor: &mut Editor,
        op: Operator,
        target: OpTarget,
        count: Option<usize>,
        register: Option<char>,
    ) -> bool {
        let cur = editor.cursor();
        let t = match target {
            OpTarget::Lines => {
                let n = count.unwrap_or(1).max(1);
                let last = editor.text().last_line();
                // `2dd` on the last line fails, like Vim's `cursor_down`.
                if n > 1 && cur.line == last {
                    return false;
                }
                Target {
                    pos: pos((cur.line + n - 1).min(last), cur.col),
                    kind: Kind::Linewise,
                    want: Want::Keep,
                    numbered_register: false,
                }
            }
            OpTarget::Motion(m) => {
                let pending = if op == Operator::Change {
                    Pending::Change
                } else {
                    Pending::Other
                };
                let Some(t) = self.motion(editor, m, count, pending) else {
                    mark_not_set(editor, m);
                    return false;
                };
                self.remember_find(m);
                if m.is_jump() {
                    set_pcmark(editor);
                }
                t
            }
            OpTarget::Object(obj) => {
                let Some(sel) = textobj::select(editor.text(), obj, count.unwrap_or(1), cur, None)
                else {
                    return false;
                };
                let range = op_range_between(
                    editor,
                    op,
                    sel.start,
                    sel.end,
                    sel.kind,
                    false,
                    sel.no_adjust,
                );
                self.apply_operator(editor, op, range, register);
                return true;
            }
        };
        let range = op_range(editor, op, cur, t);
        self.apply_operator(editor, op, range, register);
        true
    }

    /// Run an operator on a range.
    pub(crate) fn apply_operator(
        &mut self,
        editor: &mut Editor,
        op: Operator,
        range: Range,
        register: Option<char>,
    ) {
        // Vim puts the cursor at the start of the text before operating on it, which is also
        // where undo returns to.
        if op != Operator::Yank {
            editor.window.cursor = range.start;
        }
        match op {
            Operator::Delete => self.delete(editor, range, register),
            Operator::Change => self.change(editor, range, register),
            Operator::Yank => yank(editor, range, register),
            Operator::ShiftRight | Operator::ShiftLeft => self.shift(
                editor,
                range.start.line,
                range.end.line,
                op == Operator::ShiftRight,
            ),
            Operator::Lowercase | Operator::Uppercase | Operator::ToggleCase => {
                self.change_case(editor, range, op)
            }
            Operator::Reindent => self.reindent(editor, range.start.line, range.end.line),
        }
        // Like Vim, the column to aim for is recomputed from wherever the operator leaves the
        // cursor, at the next vertical move.
        editor.window.set_curswant = true;
    }

    pub(crate) fn delete(&mut self, editor: &mut Editor, r: Range, register: Option<char>) {
        if r.linewise {
            let text = lines_text(editor, r.start.line, r.end.line);
            editor
                .registers
                .delete(register, Register::new(text, RegisterKind::Line), true);
            let count = r.end.line - r.start.line + 1;
            self.delete_lines(editor, r.start.line, r.end.line);
            let line = r.start.line.min(editor.text().last_line());
            editor.window.cursor = pos(
                line,
                editor.metrics().col_for_vcol(line, editor.window.curswant),
            );
            if editor.text().has_no_lines() {
                editor.info("--No lines in buffer--");
            } else if let Some(msg) =
                util::more_lines_message(-(count as isize), editor.options.report)
            {
                editor.more_info(msg);
            }
        } else {
            let (from, to) = char_range(editor, &r);
            let deleted = editor.text().slice(from..to);
            editor.registers.delete(
                register,
                Register::new(deleted.clone(), RegisterKind::Char),
                r.numbered_register,
            );
            self.edit(editor, Edit::delete(from..to));
            editor.window.cursor = r.start;
            normalize_cursor(editor);
            set_want(editor, Want::Column);
            let lines = deleted.matches('\n').count();
            if let Some(msg) = util::more_lines_message(-(lines as isize), editor.options.report) {
                editor.more_info(msg);
            }
        }
    }

    /// Delete whole lines `first..=last`.
    pub(crate) fn delete_lines(&mut self, editor: &mut Editor, first: usize, last: usize) {
        let text = editor.text();
        let edit = if last < text.last_line() {
            Edit::delete(text.line_start(first)..text.line_start(last + 1))
        } else if first > 0 {
            Edit::delete(text.line_start(first) - 1..text.len_chars())
        } else {
            Edit::delete(0..text.len_chars())
        };
        let all = first == 0 && last == text.last_line();
        self.edit(editor, edit);
        if all {
            editor.current_buffer_mut().text.set_no_lines(true);
        }
    }

    pub(crate) fn change(&mut self, editor: &mut Editor, r: Range, register: Option<char>) {
        if r.linewise {
            let text = lines_text(editor, r.start.line, r.end.line);
            editor
                .registers
                .delete(register, Register::new(text, RegisterKind::Line), true);
            let first = util::line(editor, r.start.line);
            let indent = if editor.buf_opts().autoindent {
                util::indent_of(&first).to_string()
            } else {
                String::new()
            };
            let t = editor.text();
            let from = t.line_start(r.start.line);
            let to = t.line_start(r.end.line) + t.line_len(r.end.line);
            self.edit(editor, Edit::replace(from..to, indent.clone()));
            let col = indent.chars().count();
            editor.window.cursor = pos(r.start.line, col);
            let ai = (!indent.is_empty()).then_some(r.start.line);
            self.begin_insert(editor, InsertKind::Change, 1, ai);
        } else {
            let (from, to) = char_range(editor, &r);
            let deleted = editor.text().slice(from..to);
            editor.registers.delete(
                register,
                Register::new(deleted, RegisterKind::Char),
                r.numbered_register,
            );
            self.edit(editor, Edit::delete(from..to));
            editor.window.cursor = r.start;
            self.begin_insert(editor, InsertKind::Change, 1, None);
        }
    }

    pub(crate) fn shift(&mut self, editor: &mut Editor, first: usize, last: usize, right: bool) {
        let sw = editor.buf_opts().sw();
        let ts = editor.buf_opts().tabstop;
        for line in first..=last {
            let s = util::line(editor, line);
            if s.is_empty() {
                continue;
            }
            let indent = util::indent_of(&s);
            let width = util::indent_width(&s, ts);
            let new = if right {
                width + sw
            } else {
                width.saturating_sub(sw)
            };
            let new_indent = util::make_indent(new, editor.buf_opts());
            if new_indent != indent {
                let start = editor.text().line_start(line);
                let end = start + indent.chars().count();
                self.edit(editor, Edit::replace(start..end, new_indent));
            }
        }
        editor.window.cursor = pos(
            first,
            editor.metrics().col_for_vcol(first, editor.window.curswant),
        );
        let n = last - first + 1;
        if n > editor.options.report {
            editor.info(format!(
                "{} {}ed 1 time",
                util::lines(n),
                if right { '>' } else { '<' }
            ));
        }
    }

    pub(crate) fn change_case(&mut self, editor: &mut Editor, r: Range, op: Operator) {
        let (from, to) = if r.linewise {
            let t = editor.text();
            (
                t.line_start(r.start.line),
                t.line_start(r.end.line) + t.line_len(r.end.line),
            )
        } else {
            char_range(editor, &r)
        };
        let old = editor.text().slice(from..to);
        let new: String = old
            .chars()
            .map(|c| match op {
                Operator::Lowercase => lower(c),
                Operator::Uppercase => upper(c),
                _ => toggle(c),
            })
            .collect();
        if new != old {
            self.edit(editor, Edit::replace(from..to, new));
        }
        editor.window.cursor = if r.linewise {
            pos(r.start.line, 0)
        } else {
            r.start
        };
        normalize_cursor(editor);
        set_want(editor, Want::Column);
        let n = r.end.line - r.start.line + 1;
        if n > editor.options.report {
            editor.info(format!("{} changed", util::lines(n)));
        }
    }

    fn put(
        &mut self,
        editor: &mut Editor,
        register: Option<char>,
        count: usize,
        before: bool,
    ) -> bool {
        let Some(reg) = editor.register(register) else {
            let name = register.unwrap_or('"');
            editor.error(format!("E353: Nothing in register {name}"));
            return false;
        };
        self.put_register(editor, &reg, count, before);
        true
    }

    /// Put `reg` after (or `before`) the cursor, `count` times.
    pub(crate) fn put_register(
        &mut self,
        editor: &mut Editor,
        reg: &Register,
        count: usize,
        before: bool,
    ) {
        let cur = editor.cursor();
        match reg.kind {
            RegisterKind::Line | RegisterKind::Block => {
                let text = vec![reg.text.as_str(); count].join("\n");
                let t = editor.text();
                let at_line = if before { cur.line } else { cur.line + 1 };
                let edit = if at_line <= t.last_line() {
                    Edit::insert(t.line_start(at_line), format!("{text}\n"))
                } else {
                    Edit::insert(t.len_chars(), format!("\n{text}"))
                };
                self.edit(editor, edit);
                let col = util::first_non_blank(&util::line(editor, at_line));
                editor.window.cursor = pos(at_line, col);
                let added = (reg.text.matches('\n').count() + 1) * count;
                let marks = &mut editor.current_buffer_mut().marks;
                marks.set('[', pos(at_line, 0));
                marks.set(']', pos(at_line + added - 1, usize::MAX));
                if let Some(msg) = util::more_lines_message(added as isize, editor.options.report) {
                    editor.more_info(msg);
                }
            }
            RegisterKind::Char => {
                let text = reg.text.repeat(count);
                let s = util::line(editor, cur.line);
                let col = if before || s.is_empty() {
                    cur.col
                } else {
                    chars::next_grapheme(&s, cur.col)
                };
                let at = editor.text().pos_to_char(cur.line, col);
                let len = text.chars().count();
                let multiline = text.contains('\n');
                self.edit(editor, Edit::insert(at, text));
                // '] is the last character put.
                let (l, c) = editor.text().char_to_pos(at + len.saturating_sub(1));
                let marks = &mut editor.current_buffer_mut().marks;
                marks.set('[', pos(cur.line, col));
                marks.set(']', pos(l, c));
                editor.window.cursor = if multiline {
                    pos(cur.line, col)
                } else {
                    let s = util::line(editor, cur.line);
                    pos(cur.line, chars::prev_grapheme(&s, col + len))
                };
                let added = reg.text.matches('\n').count() * count;
                if let Some(msg) = util::more_lines_message(added as isize, editor.options.report) {
                    editor.more_info(msg);
                }
            }
        }
        set_want(editor, Want::Column);
    }

    fn replace(&mut self, editor: &mut Editor, count: usize, c: char) -> bool {
        let cur = editor.cursor();
        let s = util::line(editor, cur.line);
        let starts = chars::grapheme_starts(&s);
        let Some(first) = starts.iter().position(|&st| st == cur.col) else {
            return false;
        };
        if first + count > starts.len() {
            return false;
        }
        let end_col = starts
            .get(first + count)
            .copied()
            .unwrap_or(s.chars().count());
        let from = editor.text().pos_to_char(cur.line, cur.col);
        let to = editor.text().pos_to_char(cur.line, end_col);
        if c == '\n' {
            // Like typing <CR> in Insert mode, then <Esc>.
            self.edit(editor, Edit::delete(from..to));
            self.insert = Some(crate::insert::Insert::new(
                InsertKind::Replace,
                1,
                editor.cursor(),
                None,
            ));
            self.newline(editor);
            self.insert = None;
            let cur = editor.cursor();
            let s = util::line(editor, cur.line);
            editor.window.cursor.col = chars::prev_grapheme(&s, cur.col);
        } else {
            self.edit(editor, Edit::replace(from..to, c.to_string().repeat(count)));
            editor.window.cursor = pos(cur.line, cur.col + count - 1);
        }
        set_want(editor, Want::Column);
        true
    }

    pub(crate) fn join(&mut self, editor: &mut Editor, count: Option<usize>, spaces: bool) -> bool {
        let cur = editor.cursor();
        let last = editor.text().last_line();
        let mut n = count.unwrap_or(0).max(2);
        if cur.line + n - 1 > last {
            if count.unwrap_or(0) <= 2 {
                return false;
            }
            n = last - cur.line + 1;
        }
        if n < 2 {
            return false;
        }
        let mut joined = util::line(editor, cur.line);
        let mut col = 0;
        // 'formatoptions' `j`: a comment leader goes when joining a comment line onto another
        // (Vim's `skip_comment`).
        let com = editor.buf_opts().comments.clone();
        let remove_comments = editor.buf_opts().formatoptions.contains('j');
        let mut prev_was_comment =
            remove_comments && crate::comments::skip_comment(&com, &joined, false, spaces).1;
        for l in cur.line + 1..cur.line + n {
            let line = util::line(editor, l);
            let mut s = line.as_str();
            if remove_comments {
                let (skip, is_comment) =
                    crate::comments::skip_comment(&com, s, prev_was_comment, spaces);
                prev_was_comment = is_comment;
                s = &s[skip..];
            }
            let next = if spaces {
                s.trim_start_matches([' ', '\t'])
            } else {
                s
            };
            let insert_space = spaces
                && !next.is_empty()
                && !next.starts_with(')')
                && !joined.is_empty()
                && !joined.ends_with([' ', '\t']);
            col = joined.chars().count();
            if insert_space {
                joined.push(' ');
            }
            joined.push_str(next);
        }
        let t = editor.text();
        let from = t.line_start(cur.line);
        let to = t.line_start(cur.line + n - 1) + t.line_len(cur.line + n - 1);
        self.edit(editor, Edit::replace(from..to, joined));
        editor.window.cursor = pos(cur.line, col);
        normalize_cursor(editor);
        set_want(editor, Want::Column);
        true
    }

    fn toggle_case(&mut self, editor: &mut Editor, count: usize) -> bool {
        let cur = editor.cursor();
        let s = util::line(editor, cur.line);
        if s.is_empty() {
            return false;
        }
        let mut end = cur.col;
        for _ in 0..count {
            end = chars::next_grapheme(&s, end);
        }
        let t = editor.text();
        let (from, to) = (
            t.pos_to_char(cur.line, cur.col),
            t.pos_to_char(cur.line, end),
        );
        let old = t.slice(from..to);
        let new: String = old.chars().map(toggle).collect();
        if new != old {
            self.edit(editor, Edit::replace(from..to, new));
        }
        let s = util::line(editor, cur.line);
        editor.window.cursor.col = end.min(chars::last_grapheme(&s));
        set_want(editor, Want::Column);
        true
    }

    fn start_insert(&mut self, editor: &mut Editor, at: InsertAt, count: usize) {
        let cur = editor.cursor();
        let s = util::line(editor, cur.line);
        let len = s.chars().count();
        let mut ai_line = None;
        let mut end_comment = None;
        editor.window.cursor.col = match at {
            InsertAt::Cursor => cur.col,
            InsertAt::After => {
                if len == 0 {
                    0
                } else {
                    chars::next_grapheme(&s, cur.col)
                }
            }
            InsertAt::FirstNonBlank => util::skip_white(&s),
            InsertAt::LineStart => 0,
            InsertAt::LineEnd => len,
            InsertAt::OpenBelow | InsertAt::OpenAbove => {
                let opened = self.open_line_vim(
                    editor,
                    crate::open_line::OpenFlags {
                        backward: at == InsertAt::OpenAbove,
                        ..Default::default()
                    },
                );
                ai_line = opened.did_ai.then_some(opened.line);
                end_comment = opened.end_comment_pending;
                editor.cursor().col
            }
        };
        self.begin_insert(editor, InsertKind::Plain(at), count, ai_line);
        if let Some(ins) = self.insert.as_mut() {
            ins.end_comment_pending = end_comment;
        }
    }
}

/// The command changes the buffer's text (refused in a directory listing).
fn changes_text(action: &Action) -> bool {
    match action {
        Action::Operate(op, _) => op.changes_text(),
        Action::Put { .. }
        | Action::Replace(_)
        | Action::Join { .. }
        | Action::ToggleCase
        | Action::RepeatSubstitute { .. }
        | Action::Insert(_)
        | Action::InsertAtLastInsert => true,
        _ => false,
    }
}

/// Work out the text an operator covers, applying Vim's adjustments for exclusive motions
/// (`:h exclusive-linewise`) and for deletes that end at the end of a line.
fn op_range(editor: &Editor, op: Operator, cur: Pos, t: Target) -> Range {
    op_range_between(editor, op, cur, t.pos, t.kind, t.numbered_register, false)
}

/// The range between two positions, however they're ordered, with Vim's adjustments (skipped
/// with `no_adjust`).
fn op_range_between(
    editor: &Editor,
    op: Operator,
    a: Pos,
    b: Pos,
    kind: Kind,
    numbered_register: bool,
    no_adjust: bool,
) -> Range {
    let (mut start, mut end) = if before(b, a) { (b, a) } else { (a, b) };
    let mut linewise = kind == Kind::Linewise;
    let mut inclusive = kind == Kind::Inclusive;
    if linewise {
        start.col = start
            .col
            .min(util::line(editor, start.line).chars().count());
    }
    if kind == Kind::Exclusive && end.col == 0 && end.line > start.line && !no_adjust {
        end.line -= 1;
        if util::in_indent(&util::line(editor, start.line), start.col) {
            linewise = true;
        } else {
            let s = util::line(editor, end.line);
            if !s.is_empty() {
                end.col = chars::last_grapheme(&s);
                inclusive = true;
            }
        }
    }
    if op == Operator::Delete && !linewise && end.line > start.line && !no_adjust {
        let s = util::line(editor, end.line);
        let after = if inclusive {
            chars::next_grapheme(&s, end.col)
        } else {
            end.col
        };
        if s.chars().skip(after).all(util::is_white)
            && util::in_indent(&util::line(editor, start.line), start.col)
        {
            linewise = true;
        }
    }
    Range {
        start,
        end,
        linewise,
        inclusive,
        numbered_register,
    }
}

/// Char indices `[from, to)` of a charwise range.
pub(crate) fn char_range(editor: &Editor, r: &Range) -> (usize, usize) {
    let t = editor.text();
    let from = t.pos_to_char(r.start.line, r.start.col);
    let to = if r.inclusive {
        let s = t.line_str(r.end.line);
        t.pos_to_char(r.end.line, chars::next_grapheme(&s, r.end.col))
    } else {
        t.pos_to_char(r.end.line, r.end.col)
    };
    (from, to.max(from))
}

pub(crate) fn lines_text(editor: &Editor, first: usize, last: usize) -> String {
    (first..=last)
        .map(|l| util::line(editor, l))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn yank(editor: &mut Editor, r: Range, register: Option<char>) {
    let (text, kind) = if r.linewise {
        (
            lines_text(editor, r.start.line, r.end.line),
            RegisterKind::Line,
        )
    } else {
        let (from, to) = char_range(editor, &r);
        (editor.text().slice(from..to), RegisterKind::Char)
    };
    editor.registers.yank(register, Register::new(text, kind));
    let (open, close) = if r.linewise {
        (pos(r.start.line, 0), pos(r.end.line, usize::MAX))
    } else if r.inclusive {
        (r.start, r.end)
    } else {
        // `']` is on the last character yanked, not just after it.
        let (from, to) = char_range(editor, &r);
        let (l, c) = editor.text().char_to_pos(to.saturating_sub(1).max(from));
        (r.start, pos(l, c))
    };
    let marks = &mut editor.current_buffer_mut().marks;
    marks.set('[', open);
    marks.set(']', close);
    editor.window.cursor = r.start;
    normalize_cursor(editor);
    if !r.linewise {
        set_want(editor, Want::Column);
    }
    let lines = r.end.line - r.start.line + 1;
    if lines > editor.options.report {
        editor.info(format!("{} yanked", util::lines(lines)));
    }
}

/// Keep the Normal-mode cursor on a character: not past the end of the line, not inside a
/// grapheme.
pub(crate) fn normalize_cursor(editor: &mut Editor) {
    let text = editor.text();
    let line = editor.window.cursor.line.min(text.last_line());
    let s = text.line_str(line);
    let col = chars::snap_to_grapheme(&s, editor.window.cursor.col.min(chars::last_grapheme(&s)));
    editor.window.cursor = pos(line, col);
}

/// Remember the cursor as where a jump started: the `''` mark and a jumplist entry.
pub(crate) fn set_pcmark(editor: &mut Editor) {
    if editor.global_busy {
        return;
    }
    let cur = editor.cursor();
    editor.window.pcmark = Some(cur);
    let buffer = editor.window.buffer;
    editor
        .window
        .jumps
        .push(flux_view::Jump { buffer, pos: cur });
}

impl Engine {
    fn mark_in_other_buffer(&self, editor: &Editor, name: char) -> bool {
        name.is_ascii_uppercase()
            && editor
                .global_marks
                .get(&name)
                .is_some_and(|&(b, _)| b != editor.window.buffer)
    }
}

/// Vim's E20 for jumping to a mark that isn't set.
fn mark_not_set(editor: &mut Editor, m: Motion) {
    if let Motion::Mark { .. } = m {
        editor.error("E20: Mark not set");
    }
}

pub(crate) fn set_want(editor: &mut Editor, want: Want) {
    let win = &mut editor.window;
    match want {
        Want::Keep => {}
        Want::Column => win.set_curswant = true,
        Want::End => {
            win.curswant = usize::MAX;
            win.set_curswant = false;
        }
        Want::Exact(v) => {
            win.curswant = v;
            win.set_curswant = false;
        }
    }
}

fn scroll(editor: &mut Editor, s: Scroll, count: Option<usize>) {
    editor.with_window(|win, m| {
        match s {
            Scroll::LinesDown => win.scroll_lines_down(count.unwrap_or(1), m),
            Scroll::LinesUp => win.scroll_lines_up(count.unwrap_or(1), m),
            Scroll::HalfDown => win.scroll_half_down(count, m),
            Scroll::HalfUp => win.scroll_half_up(count, m),
            Scroll::PageDown => win.page_down(count, m),
            Scroll::PageUp => win.page_up(count, m),
        };
    });
}

/// `CTRL-G`: `"name" [Modified] 12 lines --50%--`.
fn file_info(editor: &mut Editor) {
    let buffer = editor.current_buffer();
    let lines = buffer.text.line_count();
    let percent = (editor.cursor().line + 1) * 100 / lines;
    let modified = if buffer.modified() { " [Modified]" } else { "" };
    let noun = if lines == 1 { "line" } else { "lines" };
    let msg = if buffer.text.has_no_lines() {
        format!("\"{}\"{modified} --No lines in buffer--", buffer.name())
    } else {
        format!(
            "\"{}\"{modified} {lines} {noun} --{percent}%--",
            buffer.name()
        )
    };
    // Neovim's CTRL-G is never truncated.
    editor.full_message(msg);
}

/// Case changes that map one char to one char; others (like `ß` to `SS`) are left alone, as in
/// Vim.
fn upper(c: char) -> char {
    let mut u = c.to_uppercase();
    match (u.next(), u.next()) {
        (Some(x), None) => x,
        _ => c,
    }
}

fn lower(c: char) -> char {
    let mut l = c.to_lowercase();
    match (l.next(), l.next()) {
        (Some(x), None) => x,
        _ => c,
    }
}

fn toggle(c: char) -> char {
    if c.is_lowercase() {
        upper(c)
    } else if c.is_uppercase() {
        lower(c)
    } else {
        c
    }
}

impl Engine {
    /// Enter Insert mode for a command that has already positioned the cursor.
    pub(crate) fn begin_insert(
        &mut self,
        editor: &mut Editor,
        kind: InsertKind,
        count: usize,
        ai_line: Option<usize>,
    ) {
        let mut ins = crate::insert::Insert::new(kind, count, editor.cursor(), ai_line);
        let cur = editor.cursor();
        ins.start_textlen = editor
            .metrics()
            .vcol_of(cur.line, editor.text().line_len(cur.line));
        self.insert = Some(ins);
        editor.mode = Mode::Insert;
        editor.message = None;
    }
}
