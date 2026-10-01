//! Visual mode (`v`, `V`).

use flux_core::{Edit, chars};
use flux_view::{Editor, Mode, Register, RegisterKind, Visual, VisualKind};

use crate::engine::{Dot, Engine, VisualDot};
use crate::key::Key;
use crate::motion::{self, Kind, Pending, Want};
use crate::normal::{Range, char_range, normalize_cursor, set_pcmark, set_want, yank};
use crate::parse::{self, Operator, VisualAction, VisualCommand, VisualParse};
use crate::textobj;
use crate::util::{self, Pos, before, pos};

impl Engine {
    pub(crate) fn start_visual(&mut self, editor: &mut Editor, kind: VisualKind) {
        editor.visual = Visual {
            anchor: editor.cursor(),
            kind,
        };
        editor.mode = Mode::Visual;
        editor.message = None;
    }

    pub(crate) fn visual_key(&mut self, editor: &mut Editor, key: Key) {
        // A message shown over the mode (after a search) lasts until the next key.
        editor.message = None;
        self.visual_pending.push(key);
        match parse::parse_visual(&self.visual_pending) {
            VisualParse::Incomplete => {}
            VisualParse::Invalid => {
                self.visual_pending.clear();
                self.failed = true;
            }
            VisualParse::Done(cmd) => {
                self.visual_pending.clear();
                editor.with_window(|win, m| win.update_curswant(m, false));
                if self.search_input.is_none()
                    && let VisualAction::Move(motion::Motion::Search { forward }) = cmd.action
                {
                    self.start_search(editor, forward, crate::search::PendingSearch::Visual(cmd));
                    return;
                }
                self.run_visual(editor, cmd);
                if editor.mode != Mode::Insert {
                    self.commit(editor);
                }
            }
        }
    }

    /// The Visual-mode keys typed so far (for 'showcmd').
    pub fn visual_pending_keys(&self) -> &[Key] {
        &self.visual_pending
    }

    pub(crate) fn run_visual(&mut self, editor: &mut Editor, cmd: VisualCommand) {
        let VisualCommand {
            register,
            count,
            action,
            keys,
        } = cmd;
        match action {
            VisualAction::Move(m) => {
                let t = self.motion(editor, m, count, Pending::None);
                match t {
                    Some(t) => {
                        if let motion::Motion::Find(f) = m {
                            self.last_find = Some(f);
                        }
                        if m.is_jump() {
                            set_pcmark(editor);
                        }
                        let text = editor.text();
                        let line = t.pos.line.min(text.last_line());
                        let s = text.line_str(line);
                        let col = t.pos.col.min(chars::last_grapheme(&s));
                        editor.window.cursor = pos(line, col);
                        set_want(editor, t.want);
                    }
                    None => self.failed = true,
                }
            }
            VisualAction::Object(obj) => {
                let anchor = editor.visual.anchor;
                let cur = editor.cursor();
                match textobj::select(editor.text(), obj, count.unwrap_or(1), cur, Some(anchor)) {
                    Some(sel) => {
                        let end = match sel.kind {
                            Kind::Exclusive if sel.end != sel.start => step_back(editor, sel.end),
                            _ => sel.end,
                        };
                        editor.visual.anchor = sel.start;
                        editor.window.cursor = end;
                        editor.visual.kind = if sel.kind == Kind::Linewise {
                            VisualKind::Line
                        } else {
                            VisualKind::Char
                        };
                        set_want(editor, Want::Column);
                    }
                    None => self.failed = true,
                }
            }
            VisualAction::Operate(op) | VisualAction::OperateLines(op) => {
                let lines = matches!(action, VisualAction::OperateLines(_));
                self.visual_operate(editor, op, lines, count, register, keys);
            }
            VisualAction::Join { spaces } => {
                let (start, end) = self.selection_bounds(editor);
                let size = self.visual_size(editor, false);
                self.exit_visual(editor);
                editor.window.cursor = start;
                let n = (end.line - start.line + 1).max(2);
                if self.join(editor, Some(n), spaces) {
                    self.set_dot(Dot {
                        keys,
                        count: None,
                        visual: Some(size),
                    });
                } else {
                    self.failed = true;
                }
            }
            VisualAction::Replace(c) => {
                if c == '\n' {
                    self.failed = true;
                    return;
                }
                let r = self.visual_range(editor, false);
                let size = self.visual_size(editor, false);
                self.exit_visual(editor);
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
                    .map(|ch| if ch == '\n' { ch } else { c })
                    .collect();
                self.edit(editor, Edit::replace(from..to, new));
                editor.window.cursor = if r.linewise {
                    pos(r.start.line, 0)
                } else {
                    r.start
                };
                normalize_cursor(editor);
                self.set_dot(Dot {
                    keys,
                    count: None,
                    visual: Some(size),
                });
            }
            VisualAction::Put { before } => {
                self.visual_put(editor, register, count.unwrap_or(1), before)
            }
            VisualAction::SwapEnds => {
                let anchor = editor.visual.anchor;
                editor.visual.anchor = editor.cursor();
                editor.window.cursor = anchor;
                set_want(editor, Want::Column);
            }
            VisualAction::Reselect => {
                let current = (
                    editor.visual.anchor,
                    editor.cursor(),
                    editor.visual.kind,
                    false,
                );
                if let Some((a, c, kind, eol)) = editor.current_buffer().last_visual {
                    editor.current_buffer_mut().last_visual = Some(current);
                    editor.visual = Visual { anchor: a, kind };
                    editor.window.cursor = c;
                    if eol {
                        editor.window.curswant = usize::MAX;
                    }
                }
            }
            VisualAction::Switch(kind) => {
                if editor.visual.kind == kind {
                    self.exit_visual(editor);
                } else {
                    editor.visual.kind = kind;
                }
            }
            VisualAction::Exit => self.exit_visual(editor),
            VisualAction::CmdLine => {
                self.exit_visual(editor);
                self.enter_cmdline(editor);
                editor.cmdline = "'<,'>".into();
                editor.cmdline_pos = 5;
            }
            VisualAction::Scroll(s) => {
                editor.with_window(|win, m| {
                    match s {
                        parse::Scroll::LinesDown => win.scroll_lines_down(count.unwrap_or(1), m),
                        parse::Scroll::LinesUp => win.scroll_lines_up(count.unwrap_or(1), m),
                        parse::Scroll::HalfDown => win.scroll_half_down(count, m),
                        parse::Scroll::HalfUp => win.scroll_half_up(count, m),
                        parse::Scroll::PageDown => win.page_down(count, m),
                        parse::Scroll::PageUp => win.page_up(count, m),
                    };
                });
            }
            VisualAction::SetMark(name) => {
                let cur = editor.cursor();
                editor.current_buffer_mut().marks.set(name, cur);
            }
        }
    }

    /// `gv` from Normal mode: select the last Visual area again.
    pub(crate) fn reselect(&mut self, editor: &mut Editor) {
        let Some((anchor, cursor, kind, eol)) = editor.current_buffer().last_visual else {
            self.failed = true;
            return;
        };
        let clamp = |editor: &Editor, p: Pos| {
            let line = p.line.min(editor.text().last_line());
            let s = editor.text().line_str(line);
            pos(line, p.col.min(chars::last_grapheme(&s)))
        };
        editor.visual = Visual {
            anchor: clamp(editor, anchor),
            kind,
        };
        editor.window.cursor = clamp(editor, cursor);
        editor.mode = Mode::Visual;
        editor.message = None;
        if eol {
            editor.window.curswant = usize::MAX;
            editor.window.set_curswant = false;
        }
    }

    /// Leave Visual mode, remembering the selection for `gv` and the `'<`/`'>` marks.
    pub(crate) fn exit_visual(&mut self, editor: &mut Editor) {
        let anchor = editor.visual.anchor;
        let cursor = editor.cursor();
        let kind = editor.visual.kind;
        let eol = editor.window.curswant == usize::MAX;
        let (start, end) = if before(cursor, anchor) {
            (cursor, anchor)
        } else {
            (anchor, cursor)
        };
        let buffer = editor.current_buffer_mut();
        buffer.last_visual = Some((anchor, cursor, kind, eol));
        let (open, close) = match kind {
            VisualKind::Char => (start, end),
            VisualKind::Line => (pos(start.line, 0), pos(end.line, usize::MAX)),
        };
        buffer.marks.set('<', open);
        buffer.marks.set('>', close);
        editor.mode = Mode::Normal;
        normalize_cursor(editor);
    }

    fn selection_bounds(&self, editor: &Editor) -> (Pos, Pos) {
        let a = editor.visual.anchor;
        let c = editor.cursor();
        if before(c, a) { (c, a) } else { (a, c) }
    }

    /// The selection as an operator range. Charwise selections include the character under
    /// the end, and the line break when the selection reaches past the end of a line (`$`, or
    /// an empty line).
    pub(crate) fn visual_range(&self, editor: &Editor, force_lines: bool) -> Range {
        let (start, end) = self.selection_bounds(editor);
        if force_lines || editor.visual.kind == VisualKind::Line {
            return Range {
                start: pos(start.line, 0),
                end: pos(end.line, 0),
                linewise: true,
                inclusive: false,
                numbered_register: false,
            };
        }
        let text = editor.text();
        let cursor_at_end = !before(editor.cursor(), editor.visual.anchor);
        let dollar = editor.window.curswant == usize::MAX && cursor_at_end;
        let end_len = text.line_len(end.line);
        if dollar || end.col >= end_len {
            let end = if end.line < text.last_line() {
                pos(end.line + 1, 0)
            } else {
                pos(end.line, end_len)
            };
            return Range {
                start,
                end,
                linewise: false,
                inclusive: false,
                numbered_register: false,
            };
        }
        Range {
            start,
            end,
            linewise: false,
            inclusive: true,
            numbered_register: false,
        }
    }

    /// The selection's size, for `.` (Vim's `redo_VIsual`).
    fn visual_size(&self, editor: &Editor, force_lines: bool) -> VisualDot {
        let (start, end) = self.selection_bounds(editor);
        let m = editor.metrics();
        let lines = end.line - start.line + 1;
        let vcol = if lines == 1 {
            m.cursor_vcol(end.line, end.col, false) + 1 - m.vcol_of(start.line, start.col)
        } else {
            m.cursor_vcol(end.line, end.col, false)
        };
        VisualDot {
            kind: if force_lines {
                VisualKind::Line
            } else {
                editor.visual.kind
            },
            lines,
            vcol,
            eol: editor.window.curswant == usize::MAX,
        }
    }

    fn visual_operate(
        &mut self,
        editor: &mut Editor,
        op: Operator,
        lines: bool,
        count: Option<usize>,
        register: Option<char>,
        keys: Vec<Key>,
    ) {
        let range = self.visual_range(editor, lines);
        let size = self.visual_size(editor, lines);
        self.exit_visual(editor);
        let dot = Dot {
            keys,
            count: None,
            visual: Some(size),
        };
        match op {
            Operator::ShiftRight | Operator::ShiftLeft => {
                for _ in 0..count.unwrap_or(1) {
                    self.shift(
                        editor,
                        range.start.line,
                        range.end.line,
                        op == Operator::ShiftRight,
                    );
                }
                let n = range.end.line - range.start.line + 1;
                let times = count.unwrap_or(1);
                if n > editor.options.report {
                    let dir = if op == Operator::ShiftRight { '>' } else { '<' };
                    let plural = if times == 1 { "time" } else { "times" };
                    editor.info(format!("{} {dir}ed {times} {plural}", util::lines(n)));
                }
                editor.window.set_curswant = true;
            }
            Operator::Yank => {
                yank(editor, range, register);
                if range.linewise {
                    editor.window.cursor = pos(range.start.line, editor.cursor().col);
                    normalize_cursor(editor);
                }
            }
            _ => self.apply_operator(editor, op, range, register),
        }
        if op.changes_text() {
            if op == Operator::Change {
                self.recording = Some(dot);
            } else {
                self.set_dot(dot);
            }
        }
    }

    /// `p`/`P` in Visual mode: replace the selection with a register. `p` puts the replaced
    /// text in the registers as a delete would; `P` leaves them alone.
    fn visual_put(
        &mut self,
        editor: &mut Editor,
        register: Option<char>,
        count: usize,
        before_: bool,
    ) {
        let Some(reg) = editor.register(register) else {
            self.exit_visual(editor);
            editor.error(format!(
                "E353: Nothing in register {}",
                register.unwrap_or('"')
            ));
            return;
        };
        let range = self.visual_range(editor, false);
        self.exit_visual(editor);
        let sink = if before_ { Some('_') } else { None };
        let deleted_all_lines =
            range.linewise && range.start.line == 0 && range.end.line == editor.text().last_line();
        self.delete(editor, range, sink);
        let text_block = |r: &Register| {
            vec![r.text.as_str(); count].join(if r.kind == RegisterKind::Char {
                ""
            } else {
                "\n"
            })
        };
        match (range.linewise, reg.kind) {
            (false, RegisterKind::Char) => {
                editor.window.cursor = range.start;
                self.put_register(editor, &reg, count, true);
            }
            (false, _) => {
                // Linewise text replacing part of a line splits the line around it.
                let at = editor.text().pos_to_char(range.start.line, range.start.col);
                self.edit(
                    editor,
                    Edit::insert(at, format!("\n{}\n", text_block(&reg))),
                );
                let line = range.start.line + 1;
                let col = util::first_non_blank(&util::line(editor, line));
                editor.window.cursor = pos(line, col);
            }
            (true, kind) => {
                let lines_reg = Register::new(text_block(&reg), RegisterKind::Line);
                if deleted_all_lines {
                    // Nothing left but one empty line: replace it.
                    let len = editor.text().len_chars();
                    self.edit(editor, Edit::replace(0..len, lines_reg.text.clone()));
                    editor.window.cursor = pos(0, util::first_non_blank(&util::line(editor, 0)));
                } else {
                    let line = range.start.line;
                    let last = editor.text().last_line();
                    if line > last {
                        editor.window.cursor = pos(last, 0);
                        self.put_register(editor, &lines_reg, 1, false);
                    } else {
                        editor.window.cursor = pos(line, 0);
                        self.put_register(editor, &lines_reg, 1, true);
                    }
                }
                let _ = kind;
            }
        }
        normalize_cursor(editor);
        editor.window.set_curswant = true;
    }

    /// Select the same amount of text as the last Visual operator, starting at the cursor.
    pub(crate) fn reselect_for_repeat(&mut self, editor: &mut Editor, v: VisualDot) {
        let cur = editor.cursor();
        editor.visual = Visual {
            anchor: cur,
            kind: v.kind,
        };
        editor.mode = Mode::Visual;
        editor.message = None;
        let last = editor.text().last_line();
        let line = (cur.line + v.lines - 1).min(last);
        if v.kind == VisualKind::Line {
            // Only the lines matter; keep the column the operator restores afterwards.
            editor.window.cursor.line = line;
            return;
        }
        let m = editor.metrics();
        let want = if v.eol {
            usize::MAX
        } else if v.lines <= 1 {
            m.vcol_of(cur.line, cur.col) + v.vcol - 1
        } else {
            v.vcol
        };
        // Past the last character, the selection ends on the line break, which Visual mode
        // allows (Vim's `coladvance` with 'selection' inclusive).
        let len = editor.text().line_len(line);
        let col = if len > 0 && m.vcol_of(line, len) <= want && want != usize::MAX {
            len
        } else {
            m.col_for_vcol(line, want)
        };
        editor.window.cursor = pos(line, col);
        editor.window.curswant = want;
        editor.window.set_curswant = false;
    }
}

/// The position just before `p`: the previous character, or the end of the previous line.
fn step_back(editor: &Editor, p: Pos) -> Pos {
    if p.col > 0 {
        let s = util::line(editor, p.line);
        return pos(p.line, chars::prev_grapheme(&s, p.col));
    }
    if p.line == 0 {
        return p;
    }
    let s = util::line(editor, p.line - 1);
    pos(p.line - 1, s.chars().count())
}
