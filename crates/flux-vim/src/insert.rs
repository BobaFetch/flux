//! Insert mode.

use flux_core::{Edit, chars};
use flux_view::{Editor, Mode, RegisterKind};

use crate::engine::{CtrlO, Dot, Engine};
use crate::indent::{self, Typed, When};
use crate::key::{Key, KeyCode, Modifiers};
use crate::motion::Want;
use crate::normal::{normalize_cursor, set_want};
use crate::open_line::OpenFlags;
use crate::parse::InsertAt;
use crate::util::{self, Pos, pos};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InsertKind {
    Plain(InsertAt),
    Change,
    Replace,
}

#[derive(Debug)]
pub(crate) struct Insert {
    kind: InsertKind,
    /// How many times the inserted text is entered (`3ix<Esc>`).
    count: usize,
    /// Where this insert started (Vim's `Insstart`): `CTRL-W` and `CTRL-U` stop here once.
    start: Pos,
    /// A line whose indent was added automatically and that nothing has been typed on yet. The
    /// indent is removed again if the line is left empty.
    pub(crate) ai_line: Option<usize>,
    /// Typing this first on a line that got a comment's middle leader ends the comment
    /// ('comments' flag `x`, Vim's `end_comment_pending`).
    pub(crate) end_comment_pending: Option<char>,
    /// The width of the line when the insert started ('formatoptions' `l`).
    pub(crate) start_textlen: usize,
    /// Keys typed, to repeat them for a count.
    typed: Vec<Key>,
    /// The text inserted, for the `".` register.
    inserted: String,
    /// The next key is literal (`CTRL-V`) or names a register to insert (`CTRL-R`).
    prefix: Option<char>,
    repeating: bool,
}

impl Insert {
    pub fn new(kind: InsertKind, count: usize, start: Pos, ai_line: Option<usize>) -> Self {
        Self {
            kind,
            count: count.max(1),
            start,
            ai_line,
            end_comment_pending: None,
            start_textlen: 0,
            typed: Vec::new(),
            inserted: String::new(),
            prefix: None,
            repeating: false,
        }
    }
}

impl Engine {
    pub(crate) fn insert_key(&mut self, editor: &mut Editor, key: Key) {
        let Some(ins) = self.insert.as_mut() else {
            editor.mode = Mode::Normal;
            return;
        };
        if !ins.repeating {
            if key != Key::ctrl('o')
                && let Some(rec) = self.recording.as_mut()
            {
                rec.keys.push(key);
            }
            ins.typed.push(key);
        }

        if let Some(prefix) = ins.prefix.take() {
            match prefix {
                'v' => {
                    let c = match key.code {
                        KeyCode::Tab => Some('\t'),
                        KeyCode::Enter => Some('\r'),
                        KeyCode::Esc => Some('\u{1b}'),
                        KeyCode::Char(c) if key.mods == Modifiers::NONE => Some(c),
                        KeyCode::Char(c)
                            if key.mods == Modifiers::CTRL && c.is_ascii_lowercase() =>
                        {
                            Some(char::from(c as u8 - b'a' + 1))
                        }
                        _ => None,
                    };
                    if let Some(c) = c {
                        self.insert_text(editor, &c.to_string());
                    }
                }
                _ => {
                    if let Some(name) = key.typed_char()
                        && let Some(reg) = editor.register(Some(name))
                    {
                        let mut text = reg.text;
                        if reg.kind == RegisterKind::Line {
                            text.push('\n');
                        }
                        self.insert_text(editor, &text);
                    }
                }
            }
            return;
        }

        // Completion keys, and keys typed while completing (see `crate::completion`).
        if self.insert_completion_key(editor, key) {
            return;
        }

        let ctrl = |c| key == Key::ctrl(c);
        match key.code {
            KeyCode::Esc => self.leave_insert(editor),
            _ if ctrl('c') || ctrl('[') => self.leave_insert(editor),
            KeyCode::Enter => self.newline(editor),
            _ if ctrl('j') || ctrl('m') => self.newline(editor),
            KeyCode::Backspace => self.backspace(editor),
            _ if ctrl('h') => self.backspace(editor),
            KeyCode::Delete => self.delete_under(editor),
            _ if ctrl('w') => self.delete_back(editor, true),
            _ if ctrl('u') => self.delete_back(editor, false),
            KeyCode::Tab if key.mods == Modifiers::NONE => self.tab(editor),
            _ if ctrl('t') => self.shift_line(editor, true),
            _ if ctrl('d') => self.shift_line(editor, false),
            _ if ctrl('v') || ctrl('q') => self.set_prefix('v'),
            _ if ctrl('r') => self.set_prefix('r'),
            _ if ctrl('o') => self.ctrl_o(editor),
            KeyCode::Left
            | KeyCode::Right
            | KeyCode::Up
            | KeyCode::Down
            | KeyCode::Home
            | KeyCode::End => self.arrow(editor, key.code),
            _ if ctrl('f') => self.ctrl_f(editor),
            // Neovim maps it to `vim.lsp.buf.signature_help()`.
            _ if ctrl('s') => crate::lsp::signature::request(editor),
            KeyCode::Char(c) if key.mods == Modifiers::NONE => self.insert_char(editor, c),
            _ => {}
        }
    }

    /// A typed character, which may reindent the line before or after it goes in
    /// ('indentkeys').
    fn insert_char(&mut self, editor: &mut Editor, c: char) {
        // Floats close when a character is typed (Neovim's InsertCharPre).
        editor.close_floats();
        let cindent = indent::cindent_on(editor);
        let cur = editor.cursor();
        let white = util::in_indent(&util::line(editor, cur.line), cur.col);
        let typed = Typed::Char(c);
        if cindent && indent::in_cinkeys(editor, typed, When::Instead, white) {
            self.reindent_typed(editor);
            return;
        }
        if cindent && indent::in_cinkeys(editor, typed, When::Before, white) {
            self.reindent_typed(editor);
        }
        self.auto_format(editor, c);
        self.end_comment(editor, c);
        self.insert_text(editor, &c.to_string());
        if cindent && indent::in_cinkeys(editor, typed, When::After, white) {
            self.reindent_typed(editor);
        }
    }

    /// Reindent the line being typed; one left blank keeps its indent only until something
    /// else is typed (Vim's `fixthisline` setting `did_ai`).
    fn reindent_typed(&mut self, editor: &mut Editor) {
        if self.fix_this_line(editor) {
            let line = editor.cursor().line;
            self.state().ai_line = Some(line);
        }
    }

    /// Auto-wrap at 'textwidth' before inserting `c` (Vim's `insertchar` and
    /// `internal_format`): with 'formatoptions' `t` for text and `c` for comments, the line is
    /// broken at the last blank before the margin, continuing the comment leader.
    fn auto_format(&mut self, editor: &mut Editor, c: char) {
        let o = editor.buf_opts().clone();
        let tw = o.textwidth;
        if tw == 0 || util::is_white(c) {
            return;
        }
        let fo = o.formatoptions.clone();
        let ins = self.state();
        let (start, start_textlen) = (ins.start, ins.start_textlen);
        // 'formatoptions' `l`: a line already too long when the insert started isn't broken.
        if editor.cursor().line == start.line && fo.contains('l') && start_textlen > tw {
            return;
        }
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(1);
        let mut no_leader = false;
        loop {
            let cur = editor.cursor();
            let m = editor.metrics();
            if m.vcol_of(cur.line, cur.col) + cw <= tw {
                break;
            }
            let do_comments = !no_leader && fo.contains('c');
            let line: Vec<char> = util::line(editor, cur.line).chars().collect();
            let s: String = line.iter().collect();
            // Don't break before the end of the comment leader.
            let leader_len = if do_comments {
                let (mut l, _) = crate::comments::leader_len(&o.comments, &s, false, true);
                if l == 0
                    && o.cindent
                    && let Some(cs) = crate::comments::check_linecomment(&s)
                {
                    let (l2, _) = crate::comments::leader_len(&o.comments, &s[cs..], false, true);
                    if l2 != 0 {
                        l = cs + l2;
                    }
                }
                s[..l].chars().count()
            } else {
                0
            };
            if leader_len == 0 {
                no_leader = true;
            }
            if leader_len == 0 && !fo.contains('t') {
                break;
            }
            let startcol = cur.col;
            if startcol == 0 {
                break;
            }
            let wantcol = m.col_for_vcol(cur.line, tw);
            // Find the blank to break at: the last one before the margin.
            let at = |i: usize| {
                if i == startcol {
                    c
                } else {
                    line.get(i).copied().unwrap_or('\0')
                }
            };
            let mut i = startcol;
            let mut foundcol = 0;
            loop {
                let mut cc = at(i);
                if util::is_white(cc) {
                    while i > 0 && util::is_white(cc) {
                        i -= 1;
                        cc = at(i);
                    }
                    if i == 0 && util::is_white(cc) {
                        break;
                    }
                    if i < leader_len {
                        break;
                    }
                    i += 1;
                    foundcol = i;
                    if i <= wantcol {
                        break;
                    }
                }
                if i == 0 {
                    break;
                }
                i -= 1;
            }
            if foundcol == 0 {
                break;
            }
            // The cursor's offset into the text that moves to the new line.
            let mut word = foundcol;
            while word < line.len() && util::is_white(line[word]) {
                word += 1;
            }
            let offset = startcol.saturating_sub(word);
            editor.window.cursor.col = foundcol;
            let opened = self.open_line_vim(
                editor,
                OpenFlags {
                    insert: true,
                    do_com: Some(do_comments),
                    del_spaces: true,
                    ..OpenFlags::default()
                },
            );
            if opened.end_comment_pending.is_some() || leader_len > 0 {
                no_leader = false;
            }
            let len = editor.text().line_len(opened.line);
            editor.window.cursor.col = (editor.cursor().col + offset).min(len);
            self.state().ai_line = None;
        }
    }

    /// Typing the last character of a comment's end first on a line that got the middle
    /// leader replaces the middle leader with the end (`*` then `/` makes `*/`).
    fn end_comment(&mut self, editor: &mut Editor, c: char) {
        let cur = editor.cursor();
        let ins = self.state();
        let pending = ins.end_comment_pending.take();
        if pending != Some(c) || ins.ai_line != Some(cur.line) {
            return;
        }
        let com = editor.buf_opts().comments.clone();
        let parts = crate::comments::parts(&com);
        let line = util::line(editor, cur.line);
        let (len, k) = crate::comments::leader_len(&com, &line, false, true);
        let Some(k) = k.filter(|&k| len > 0 && parts[k].has('m')) else {
            return;
        };
        let middle = parts[k].string.trim_end_matches([' ', '\t']);
        let Some(end) = parts.get(k + 1).map(|p| p.string) else {
            return;
        };
        let b = line.as_bytes();
        let bcol = line
            .char_indices()
            .nth(cur.col)
            .map_or(line.len(), |(i, _)| i);
        let mut j = bcol;
        while j > 0 && (b[j - 1] == b' ' || b[j - 1] == b'\t') {
            j -= 1;
        }
        if j < middle.len() || !end.ends_with(c) {
            return;
        }
        let j = j - middle.len();
        let start = editor.text().line_start(cur.line);
        let from = start + line[..j].chars().count();
        let replacement: String = end.chars().take(end.chars().count() - 1).collect();
        self.edit(
            editor,
            Edit::replace(from..start + cur.col, replacement.clone()),
        );
        editor.window.cursor.col = line[..j].chars().count() + replacement.chars().count();
    }

    /// `CTRL-F`: reindent the line when 'indentkeys' has `!^F` (the default).
    fn ctrl_f(&mut self, editor: &mut Editor) {
        let cur = editor.cursor();
        let white = util::in_indent(&util::line(editor, cur.line), cur.col);
        if indent::cindent_on(editor)
            && indent::in_cinkeys(editor, Typed::Char('\u{6}'), When::Instead, white)
        {
            self.reindent_typed(editor);
        }
    }

    /// Let completion handle `key` first. A key it uses up isn't recorded for `.`: what it
    /// changed is (see [`Engine::redo_retype`]).
    fn insert_completion_key(&mut self, editor: &mut Editor, key: Key) -> bool {
        // `<Tab>` jumps in a snippet.
        if self.snippet_key(editor, key) {
            return true;
        }
        let recorded = !self.state().repeating;
        let mut in_dot = false;
        if recorded {
            self.state().typed.pop();
            if key != Key::ctrl('o')
                && let Some(rec) = self.recording.as_mut()
                && rec.keys.last() == Some(&key)
            {
                rec.keys.pop();
                in_dot = true;
            }
        }
        let used = self.completion_key(editor, key);
        if recorded && !used {
            if let Some(ins) = self.insert.as_mut() {
                ins.typed.push(key);
            }
            if in_dot && let Some(rec) = self.recording.as_mut() {
                rec.keys.push(key);
            }
        }
        used
    }

    /// Record that the typed text `base` became `now`: as many `<BS>` as needed to get back
    /// to what they share, and the rest typed.
    pub(crate) fn redo_retype(&mut self, base: &str, now: &str) {
        let common = base
            .chars()
            .zip(now.chars())
            .take_while(|(a, b)| a == b)
            .count();
        let back = base.chars().count() - common;
        let rest: String = now.chars().skip(common).collect();
        let mut keys = vec![Key::plain(KeyCode::Backspace); back];
        keys.extend(rest.chars().map(Key::char));
        if let Some(ins) = self.insert.as_mut() {
            if ins.repeating {
                return;
            }
            ins.typed.extend(keys.iter().copied());
            for _ in 0..back {
                ins.inserted.pop();
            }
            ins.inserted.push_str(&rest);
        }
        if let Some(rec) = self.recording.as_mut() {
            rec.keys.extend(keys);
        }
    }

    fn set_prefix(&mut self, prefix: char) {
        if let Some(ins) = self.insert.as_mut() {
            ins.prefix = Some(prefix);
        }
    }

    fn state(&mut self) -> &mut Insert {
        self.insert.as_mut().expect("in Insert mode")
    }

    /// Insert `text` at the cursor and move past it.
    fn insert_text(&mut self, editor: &mut Editor, text: &str) {
        let cur = editor.cursor();
        let at = editor.text().pos_to_char(cur.line, cur.col);
        self.edit(editor, Edit::insert(at, text));
        let end = at + text.chars().count();
        let (line, col) = editor.text().char_to_pos(end);
        editor.window.cursor = pos(line, col);
        let ins = self.state();
        if ins.ai_line == Some(cur.line) {
            ins.ai_line = None;
        }
        ins.inserted.push_str(text);
        set_want(editor, Want::Column);
    }

    /// `<CR>`: split the line (Vim's `open_line`, see [`crate::open_line`]): the new line gets
    /// the indent and comment leader, and the text after the cursor without its leading
    /// blanks. A line that holds only an automatic indent is emptied.
    pub(crate) fn newline(&mut self, editor: &mut Editor) {
        let cur = editor.cursor();
        let did_ai = self.insert.as_ref().and_then(|i| i.ai_line) == Some(cur.line);
        let opened = self.open_line_vim(
            editor,
            OpenFlags {
                insert: true,
                did_ai,
                ..OpenFlags::default()
            },
        );
        if let Some(ins) = self.insert.as_mut() {
            ins.ai_line = opened.did_ai.then_some(opened.line);
            ins.end_comment_pending = opened.end_comment_pending;
            ins.inserted.push('\n');
        }
        set_want(editor, Want::Column);
    }

    /// `<BS>`: delete the character before the cursor; at the start of a line, join it to the
    /// previous one ('backspace' has `eol`). In the indent with 'smarttab', go back to the
    /// previous 'shiftwidth' stop.
    fn backspace(&mut self, editor: &mut Editor) {
        self.state().inserted.pop();
        let cur = editor.cursor();
        if cur.col == 0 {
            self.join_previous(editor);
            return;
        }
        let s = util::line(editor, cur.line);
        let line_start = editor.text().line_start(cur.line);
        let o = editor.buf_opts();
        let in_indent = util::in_indent(&s, cur.col);
        let prev_white = s
            .chars()
            .nth(cur.col - 1)
            .is_some_and(|c| c == ' ' || c == '\t');
        let step = if editor.options.smarttab && in_indent {
            o.sw()
        } else if o.sts() != 0 && prev_white {
            o.sts()
        } else {
            0
        };
        if step > 0 {
            let m = editor.metrics();
            let vcol = m.vcol_of(cur.line, cur.col);
            let sw = step.max(1);
            let want = (vcol - 1) / sw * sw;
            let mut col = cur.col;
            while col > 0 && m.vcol_of(cur.line, col) > want {
                col -= 1;
            }
            let fill = want.saturating_sub(m.vcol_of(cur.line, col));
            self.edit(
                editor,
                Edit::replace(line_start + col..line_start + cur.col, " ".repeat(fill)),
            );
            editor.window.cursor.col = col + fill;
        } else {
            let prev = chars::prev_grapheme(&s, cur.col);
            self.edit(
                editor,
                Edit::delete(line_start + prev..line_start + cur.col),
            );
            editor.window.cursor.col = prev;
        }
        set_want(editor, Want::Column);
    }

    fn join_previous(&mut self, editor: &mut Editor) {
        let cur = editor.cursor();
        if cur.line == 0 {
            return;
        }
        let prev_len = editor.text().line_len(cur.line - 1);
        let at = editor.text().line_start(cur.line) - 1;
        self.edit(editor, Edit::delete(at..at + 1));
        editor.window.cursor = pos(cur.line - 1, prev_len);
        set_want(editor, Want::Column);
    }

    /// `<Del>`: delete the character under the cursor, or join the next line at the end.
    fn delete_under(&mut self, editor: &mut Editor) {
        let cur = editor.cursor();
        let s = util::line(editor, cur.line);
        let len = s.chars().count();
        let start = editor.text().line_start(cur.line);
        if cur.col < len {
            let next = chars::next_grapheme(&s, cur.col);
            self.edit(editor, Edit::delete(start + cur.col..start + next));
        } else if cur.line < editor.text().last_line() {
            self.edit(editor, Edit::delete(start + len..start + len + 1));
        }
    }

    /// `CTRL-W` (`word`) and `CTRL-U`: delete back to the start of the word, or of the typed text
    /// or the indent. Both stop once at the point where the insert started.
    fn delete_back(&mut self, editor: &mut Editor, word: bool) {
        let cur = editor.cursor();
        if cur.col == 0 {
            self.join_previous(editor);
            return;
        }
        let s = util::line(editor, cur.line);
        let chars: Vec<char> = s.chars().collect();
        let start = self.state().start;
        let mut min_col = 0;
        if !word && editor.buf_opts().autoindent {
            let first = util::skip_white(&s);
            if first < cur.col {
                min_col = first;
            }
        }
        let mut col = cur.col;
        let mut in_word = false;
        let mut word_is_keyword = false;
        loop {
            let prev = chars::prev_grapheme(&s, col);
            if word {
                let c = chars[prev];
                if !in_word && !util::is_white(c) {
                    in_word = true;
                    word_is_keyword = flux_core::chars::is_keyword(c);
                } else if in_word
                    && (util::is_white(c) || flux_core::chars::is_keyword(c) != word_is_keyword)
                {
                    break;
                }
            }
            col = prev;
            if col <= min_col || (cur.line == start.line && col == start.col) {
                break;
            }
        }
        let line_start = editor.text().line_start(cur.line);
        self.edit(editor, Edit::delete(line_start + col..line_start + cur.col));
        editor.window.cursor.col = col;
        set_want(editor, Want::Column);
    }

    /// `<Tab>` (Vim's `ins_tab`): white space to the next stop — 'shiftwidth' in the indent
    /// with 'smarttab', else 'softtabstop', else 'tabstop'. Without 'expandtab' the white
    /// space before the cursor is then made of as many tabs as fit.
    fn tab(&mut self, editor: &mut Editor) {
        let o = editor.buf_opts().clone();
        let cur = editor.cursor();
        let s = util::line(editor, cur.line);
        let in_indent = util::in_indent(&s, cur.col);
        let smart = editor.options.smarttab && in_indent;
        if !o.expandtab && !(smart && o.sw() != o.tabstop) && o.sts() == 0 {
            self.insert_text(editor, "\t");
            return;
        }
        let step = if smart {
            o.sw()
        } else if o.sts() != 0 {
            o.sts()
        } else {
            o.tabstop
        }
        .max(1);
        let vcol = editor.metrics().vcol_of(cur.line, cur.col);
        let spaces = step - vcol % step;
        self.insert_text(editor, &" ".repeat(spaces));
        if o.expandtab {
            return;
        }
        // Rebuild the white space before the cursor with tabs where they fit.
        let cur = editor.cursor();
        let s = util::line(editor, cur.line);
        let chars: Vec<char> = s.chars().collect();
        let mut start = cur.col;
        while start > 0 && matches!(chars[start - 1], ' ' | '\t') {
            start -= 1;
        }
        let m = editor.metrics();
        let (from, to) = (m.vcol_of(cur.line, start), m.vcol_of(cur.line, cur.col));
        let ts = o.tabstop.max(1);
        let mut white = String::new();
        let mut v = from;
        while (v / ts + 1) * ts <= to {
            white.push('\t');
            v = (v / ts + 1) * ts;
        }
        white.push_str(&" ".repeat(to - v));
        let old: String = chars[start..cur.col].iter().collect();
        if white != old {
            let line_start = editor.text().line_start(cur.line);
            self.edit(
                editor,
                Edit::replace(line_start + start..line_start + cur.col, white.clone()),
            );
            editor.window.cursor.col = start + white.chars().count();
        }
    }

    /// `CTRL-T` / `CTRL-D`: shift the current line by 'shiftwidth', keeping the cursor on the
    /// same text.
    fn shift_line(&mut self, editor: &mut Editor, right: bool) {
        let cur = editor.cursor();
        let s = util::line(editor, cur.line);
        let indent = util::indent_of(&s);
        let indent_len = indent.chars().count();
        let width = util::indent_width(&s, editor.buf_opts().tabstop);
        let sw = editor.buf_opts().sw().max(1);
        let new = if right {
            (width / sw + 1) * sw
        } else {
            width.saturating_sub(1) / sw * sw
        };
        let new_indent = util::make_indent(
            if !right && width == 0 { 0 } else { new },
            editor.buf_opts(),
        );
        let start = editor.text().line_start(cur.line);
        let new_len = new_indent.chars().count();
        self.edit(editor, Edit::replace(start..start + indent_len, new_indent));
        let col = if cur.col >= indent_len {
            cur.col - indent_len + new_len
        } else {
            new_len
        };
        editor.window.cursor.col = col;
        set_want(editor, Want::Column);
    }

    /// `CTRL-O`: run one Normal-mode command, then come back.
    fn ctrl_o(&mut self, editor: &mut Editor) {
        self.commit(editor);
        let cur = editor.cursor();
        let s = util::line(editor, cur.line);
        let at_eol = !s.is_empty() && cur.col >= s.chars().count();
        if at_eol {
            editor.window.cursor.col = chars::last_grapheme(&s);
        }
        self.ctrl_o = Some(CtrlO {
            at_eol,
            line: cur.line,
        });
        editor.mode = Mode::Normal;
        editor.insert_pending = true;
    }

    /// Back to Insert mode after a `CTRL-O` command. Like Vim, the cursor goes past the end of
    /// the line if it was there before or the command aimed there (`$`).
    pub(crate) fn finish_ctrl_o(&mut self, editor: &mut Editor) {
        let Some(c) = self.ctrl_o.take() else {
            return;
        };
        editor.insert_pending = false;
        if self.insert.is_none() || editor.mode != Mode::Normal {
            return;
        }
        let cur = editor.cursor();
        let s = util::line(editor, cur.line);
        if !s.is_empty() && cur.col == chars::last_grapheme(&s) {
            let vcol = editor.metrics().vcol_of(cur.line, cur.col);
            if (c.at_eol && cur.line == c.line) || editor.window.curswant > vcol {
                editor.window.cursor.col = s.chars().count();
            }
        }
        editor.mode = Mode::Insert;
    }

    /// Cursor keys: move, and start a new undo step and a new insert for `.` (as Vim does).
    fn arrow(&mut self, editor: &mut Editor, code: KeyCode) {
        self.commit(editor);
        let cur = editor.cursor();
        let s = util::line(editor, cur.line);
        let len = s.chars().count();
        let last = editor.text().last_line();
        editor.with_window(|win, m| win.update_curswant(m, true));
        let target = match code {
            KeyCode::Left => {
                (cur.col > 0).then(|| pos(cur.line, chars::prev_grapheme(&s, cur.col)))
            }
            KeyCode::Right => {
                (cur.col < len).then(|| pos(cur.line, chars::next_grapheme(&s, cur.col)))
            }
            KeyCode::Home => Some(pos(cur.line, 0)),
            KeyCode::End => Some(pos(cur.line, len)),
            KeyCode::Up if cur.line > 0 => {
                Some(pos(cur.line - 1, insert_col(editor, cur.line - 1)))
            }
            KeyCode::Down if cur.line < last => {
                Some(pos(cur.line + 1, insert_col(editor, cur.line + 1)))
            }
            _ => None,
        };
        if let Some(p) = target {
            editor.window.cursor = p;
            match code {
                KeyCode::Up | KeyCode::Down => {}
                KeyCode::End => editor.window.curswant = usize::MAX,
                _ => set_want(editor, Want::Column),
            }
        }
        let cur = editor.cursor();
        let ins = self.state();
        ins.start = cur;
        ins.start_textlen = cur.col;
        ins.count = 1;
        ins.typed.clear();
        ins.ai_line = None;
        self.recording = Some(Dot {
            keys: vec![Key::char('i')],
            count: None,
            visual: None,
        });
    }

    pub(crate) fn leave_insert(&mut self, editor: &mut Editor) {
        let Some(mut ins) = self.insert.take() else {
            return;
        };
        if ins.count > 1 && !ins.repeating {
            let keys: Vec<Key> = ins
                .typed
                .iter()
                .copied()
                .filter(|k| !is_escape(*k))
                .collect();
            ins.repeating = true;
            let open = matches!(
                ins.kind,
                InsertKind::Plain(InsertAt::OpenBelow | InsertAt::OpenAbove)
            );
            let times = ins.count - 1;
            self.insert = Some(ins);
            for _ in 0..times {
                if open {
                    let opened = self.open_line_vim(editor, OpenFlags::default());
                    let ins = self.state();
                    ins.ai_line = opened.did_ai.then_some(opened.line);
                    ins.end_comment_pending = opened.end_comment_pending;
                }
                for &key in &keys {
                    self.insert_key(editor, key);
                }
            }
            ins = self.insert.take().expect("still inserting");
        }
        // An automatic indent or comment leader that nothing was typed after loses its
        // trailing blanks (Vim's `stop_insert` with `did_ai`).
        let cur = editor.cursor();
        if ins.ai_line == Some(cur.line) {
            let s: Vec<char> = util::line(editor, cur.line).chars().collect();
            let mut col = cur.col.min(s.len());
            if col == s.len() && col > 0 {
                let mut from = col;
                while from > 0 && util::is_white(s[from - 1]) {
                    from -= 1;
                }
                if from < col {
                    let start = editor.text().line_start(cur.line);
                    self.edit(editor, Edit::delete(start + from..start + col));
                    col = from;
                }
                editor.window.cursor.col = col;
            }
        }
        self.commit(editor);
        editor.lsp_show_held_diagnostics();
        if let Some(dot) = self.recording.take() {
            self.set_dot(dot);
        }
        // The `".` register holds what was typed, and `'^` where Insert mode ended.
        if ins.kind != InsertKind::Replace {
            editor.registers.set_readonly('.', ins.inserted.clone());
        }
        let cur = editor.cursor();
        editor.current_buffer_mut().marks.set('^', cur);
        if cur.col > 0 {
            let s = util::line(editor, cur.line);
            editor.window.cursor.col = chars::prev_grapheme(&s, cur.col);
        }
        editor.mode = Mode::Normal;
        editor.message = None;
        normalize_cursor(editor);
        set_want(editor, Want::Column);
    }
}

fn is_escape(key: Key) -> bool {
    key == Key::plain(KeyCode::Esc) || key == Key::ctrl('c') || key == Key::ctrl('[')
}

/// The Insert-mode column for `curswant` on `line`: like Normal mode, but it may be past the
/// last character.
fn insert_col(editor: &Editor, line: usize) -> usize {
    let want = editor.window.curswant;
    let s = util::line(editor, line);
    let m = editor.metrics();
    let col = m.col_for_vcol(line, want);
    if !s.is_empty() && m.vcol_of(line, s.chars().count()) <= want {
        s.chars().count()
    } else {
        col
    }
}
