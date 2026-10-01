//! `gq` and `gw`: formatting lines to 'textwidth', ported from Vim's `textformat.c`
//! (`op_format`, `format_lines`, and `internal_format` as it formats). Paragraphs are joined
//! and broken again at white space before the margin, keeping comment leaders ('formatoptions'
//! `q`). With a language server that formats ranges, `gq` asks it instead (Neovim sets
//! 'formatexpr' to `vim.lsp.formatexpr()`).

use flux_core::Edit;
use flux_view::Editor;

use crate::comments::{self, Part};
use crate::engine::Engine;
use crate::normal::Range;
use crate::open_line::OpenFlags;
use crate::util::{self, Pos, pos};

/// The comment leader a line starts with: its length in chars, and the 'comments' entry.
#[derive(Debug, Clone, Copy, Default)]
struct Leader {
    len: usize,
    part: Option<usize>,
}

/// Vim's `inmacro`: whether `s` (after a `.`) starts with one of the two-letter nroff macros
/// in `opt`.
fn in_macro(opt: &str, s: &[char]) -> bool {
    let m: Vec<char> = opt.chars().collect();
    let s0 = s.first().copied().unwrap_or('\0');
    let s1 = s.get(1).copied().unwrap_or('\0');
    let mut i = 0;
    while i < m.len() {
        let m0 = m[i];
        let m1 = m.get(i + 1).copied().unwrap_or('\0');
        if (m0 == s0 || (m0 == ' ' && (s0 == '\0' || s0 == ' ')))
            && (m1 == s1 || ((m1 == '\0' || m1 == ' ') && (s0 == '\0' || s1 == '\0' || s1 == ' ')))
        {
            return true;
        }
        i += 2;
    }
    false
}

/// Vim's `startPS(lnum, NUL, false)`: an empty line, a form feed, or an nroff paragraph or
/// section macro ('paragraphs' and 'sections' at their defaults).
fn starts_paragraph(line: &str) -> bool {
    let c: Vec<char> = line.chars().collect();
    match c.first() {
        None | Some('\x0c') => true,
        Some('.') => {
            in_macro("SHNHH HUnhsh", &c[1..]) || in_macro("IPLPPPQPP TPHPLIPpLpItpplpipbp", &c[1..])
        }
        _ => false,
    }
}

/// How formatting goes ('formatoptions' and the margin).
struct Format {
    comments: String,
    do_comments: bool,
    white_par: bool,
    textwidth: usize,
}

impl Format {
    fn parts(&self) -> Vec<Part<'_>> {
        comments::parts(&self.comments)
    }

    /// The leader of `line` (Vim's `get_leader_len` with white space included).
    fn leader(&self, line: &str) -> Leader {
        let (bytes, part) = comments::leader_len(&self.comments, line, false, true);
        Leader {
            len: line[..bytes].chars().count(),
            part,
        }
    }

    /// Vim's `fmt_check_par`: whether `line` isn't part of a paragraph (blank, only a leader, a
    /// comment's end), and its leader.
    fn check_par(&self, line: &str) -> (bool, Leader) {
        let leader = if self.do_comments {
            self.leader(line)
        } else {
            Leader::default()
        };
        let rest: String = line.chars().skip(leader.len).collect();
        let ends_comment = leader.len > 0
            && leader
                .part
                .is_some_and(|k| self.parts().get(k).is_some_and(|p| p.has('e')));
        let not_par = rest.trim_start_matches([' ', '\t']).is_empty()
            || ends_comment
            || starts_paragraph(line);
        (not_par, leader)
    }

    /// Vim's `same_leader`: whether `line1` (with leader `l1`) and the next line (with `l2`)
    /// continue the same comment.
    fn same_leader(&self, line1: &str, l1: Leader, line2: &str, l2: Leader) -> bool {
        if l1.len == 0 {
            return l2.len == 0;
        }
        let parts = self.parts();
        if let Some(p) = l1.part.and_then(|k| parts.get(k)) {
            for f in p.flags.chars() {
                match f {
                    'f' => return l2.len == 0,
                    'e' => return false,
                    's' => {
                        if line1.chars().count() <= l1.len || l2.len == 0 {
                            return false;
                        }
                        return l2
                            .part
                            .and_then(|k| parts.get(k))
                            .is_some_and(|p| p.has('m'));
                    }
                    _ => {}
                }
            }
        }
        // The leaders are the same apart from white space.
        let a: Vec<char> = line1.chars().collect();
        let b: Vec<char> = line2.chars().collect();
        let mut i1 = 0;
        while a.get(i1).is_some_and(|&c| util::is_white(c)) {
            i1 += 1;
        }
        let mut i2 = 0;
        while i2 < l2.len {
            let c = b.get(i2).copied().unwrap_or('\0');
            if !util::is_white(c) {
                if a.get(i1).copied().unwrap_or('\0') != c {
                    break;
                }
                i1 += 1;
            } else {
                while a.get(i1).is_some_and(|&c| util::is_white(c)) {
                    i1 += 1;
                }
            }
            i2 += 1;
        }
        i2 == l2.len && i1 == l1.len
    }
}

/// How much of `s` a list item's marker takes, for 'formatlistpat': Markdown's (a number and
/// a period, a bullet, or a footnote) or the default (a number and one of `]:.)}`, a tab or a
/// space), with the blanks after it.
fn list_marker(s: &[char], markdown: bool) -> Option<usize> {
    let white = |i: usize| s.get(i).is_some_and(|&c| c == ' ' || c == '\t');
    let mut i = 0;
    while white(i) {
        i += 1;
    }
    let digits_from = i;
    while s.get(i).is_some_and(char::is_ascii_digit) {
        i += 1;
    }
    let has_digits = i > digits_from;
    if markdown {
        // `^\s*\d\+\.\s\+`, `^\s*[-*+]\s\+`, `^\[^\ze[^\]]\+\]:\&^.\{4\}`.
        let blanks_after = |mut j: usize| {
            let start = j;
            while white(j) {
                j += 1;
            }
            (j > start).then_some(j)
        };
        if has_digits && s.get(i) == Some(&'.') {
            return blanks_after(i + 1);
        }
        if !has_digits && matches!(s.get(i), Some('-' | '*' | '+')) {
            return blanks_after(i + 1);
        }
        if s.first() == Some(&'[') && s.get(1) == Some(&'^') {
            let close = s.iter().skip(2).position(|&c| c == ']')?;
            if close > 0 && s.get(2 + close + 1) == Some(&':') && s.len() >= 4 {
                return Some(4);
            }
        }
        return None;
    }
    if !has_digits || !matches!(s.get(i), Some(']' | ':' | '.' | ')' | '}' | '\t' | ' ')) {
        return None;
    }
    i += 1;
    while white(i) {
        i += 1;
    }
    Some(i)
}

/// Vim's `get_number_indent`: the screen column where a numbered list item's text starts on
/// line `lnum` (after its comment leader with 'formatoptions' `q`).
fn number_indent(editor: &Editor, lnum: usize, f: &Format) -> Option<usize> {
    if lnum > editor.text().last_line() {
        return None;
    }
    let line = util::line(editor, lnum);
    let lead = if f.do_comments {
        f.leader(&line).len
    } else {
        0
    };
    let chars: Vec<char> = line.chars().collect();
    let markdown = editor.buf_opts().filetype == "markdown";
    let end = lead + list_marker(&chars[lead.min(chars.len())..], markdown)?;
    if end >= chars.len() {
        return None;
    }
    Some(editor.metrics().vcol_of(lnum, end))
}

/// Vim's `comp_textwidth(true)`: 'textwidth', or the window's width less one, at most 79.
fn textwidth(editor: &Editor) -> usize {
    let tw = editor.buf_opts().textwidth;
    if tw > 0 {
        return tw;
    }
    let width = editor
        .window_rects()
        .into_iter()
        .find(|(id, _)| *id == editor.window.id)
        .map_or(80, |(_, r)| r.width);
    width.saturating_sub(1).min(79)
}

impl Engine {
    /// `gq` (or `gw`, `keep_cursor`) on the lines of `range`. `cursor_start` is where the
    /// command was typed; `end_adjusted` says the range was cut back a line (an exclusive motion
    /// ending in column 0, `gq}`).
    pub(crate) fn format_op(
        &mut self,
        editor: &mut Editor,
        range: Range,
        keep_cursor: bool,
        cursor_start: Pos,
        end_adjusted: bool,
    ) {
        let (first, last) = (range.start.line, range.end.line);
        // 'formatexpr' (`vim.lsp.formatexpr()`); `gw` never uses it.
        if !keep_cursor && crate::lsp::edits::formats(editor) {
            editor.window.cursor = range.start;
            crate::lsp::edits::format_lines(editor, first, last);
            return;
        }
        let old_count = editor.text().line_count();
        editor.current_buffer_mut().marks.set('[', range.start);
        editor.window.cursor = range.start;
        let mut saved = keep_cursor.then_some(cursor_start);
        let fresh = !self.has_change();
        self.format_lines(editor, last - first + 1, &mut saved);
        // Undo goes back to where the command was typed.
        if fresh {
            self.set_undo_cursor(cursor_start);
        }
        // To the first non-blank of the last line formatted (the next one if the range was cut
        // back, so `.` goes on).
        let mut line = editor.cursor().line;
        if end_adjusted && line < editor.text().last_line() {
            line += 1;
        }
        let col = util::first_non_blank(&util::line(editor, line));
        editor.window.cursor = pos(line, col);
        crate::normal::normalize_cursor(editor);
        let delta = editor.text().line_count() as isize - old_count as isize;
        if let Some(msg) = util::more_lines_message(delta, editor.options.report) {
            editor.more_info(msg);
        }
        let end = editor.cursor();
        editor.current_buffer_mut().marks.set(']', end);
        if let Some(p) = saved {
            let text = editor.text();
            let line = p.line.min(text.last_line());
            editor.window.cursor = pos(line, p.col);
            crate::normal::normalize_cursor(editor);
        }
        editor.window.set_curswant = true;
    }

    /// Vim's `format_lines`: format `line_count` lines from the cursor line. `saved` is a
    /// position kept on the same text (`gw`'s cursor).
    fn format_lines(&mut self, editor: &mut Editor, line_count: usize, saved: &mut Option<Pos>) {
        let o = editor.buf_opts().clone();
        let fo = o.formatoptions.clone();
        let f = Format {
            comments: o.comments.clone(),
            do_comments: fo.contains('q'),
            white_par: fo.contains('w'),
            textwidth: textwidth(editor),
        };
        let do_second_indent = fo.contains('2');
        let do_number_indent = fo.contains('n');
        let max_len = f.textwidth * 3;
        let line_of = |editor: &Editor, l: usize| util::line(editor, l);
        let ends_in_white = |s: &str| s.ends_with([' ', '\t']);

        let first_line = editor.cursor().line;
        let mut lnum = first_line;
        let mut is_not_par = if lnum > 0 {
            f.check_par(&line_of(editor, lnum - 1)).0
        } else {
            true
        };
        let mut leader = Leader::default();
        let (mut next_is_not_par, mut next_leader) = f.check_par(&line_of(editor, lnum));
        let mut is_end_par = is_not_par || next_is_not_par;
        if !is_end_par && f.white_par && lnum > 0 {
            is_end_par = !ends_in_white(&line_of(editor, lnum - 1));
        }
        let mut prev_is_end_par = false;
        let mut next_is_start_par = false;
        let mut advance = true;
        let mut need_set_indent = true;
        let mut force_format = false;
        let mut first_par_line = true;
        // The indent for the second line of a paragraph ('formatoptions' `2` and `n`).
        let mut second_indent: Option<usize> = None;
        let mut com_list = false;
        let mut first = true;
        let mut count = line_count;
        while count != 0 {
            if advance {
                if first {
                    first = false;
                } else {
                    lnum += 1;
                }
                prev_is_end_par = is_end_par;
                is_not_par = next_is_not_par;
                leader = next_leader;
            }
            let last = editor.text().last_line();
            if count == 1 || lnum == last {
                next_is_not_par = true;
                next_leader = Leader::default();
            } else {
                (next_is_not_par, next_leader) = f.check_par(&line_of(editor, lnum + 1));
                if do_number_indent {
                    next_is_start_par = number_indent(editor, lnum + 1, &f).is_some();
                }
            }
            advance = true;
            is_end_par = is_not_par || next_is_not_par || next_is_start_par;
            if !is_end_par && f.white_par {
                is_end_par = !ends_in_white(&line_of(editor, lnum));
            }
            if !is_not_par {
                // A paragraph's second line sets the indent of those after the first.
                if first_par_line
                    && (do_second_indent || do_number_indent)
                    && prev_is_end_par
                    && lnum < editor.text().last_line()
                {
                    let both_plain = leader.len == 0 && next_leader.len == 0;
                    if do_second_indent && !line_of(editor, lnum + 1).is_empty() {
                        if both_plain {
                            let ts = editor.buf_opts().tabstop;
                            second_indent =
                                Some(util::indent_width(&line_of(editor, lnum + 1), ts));
                        } else {
                            second_indent = Some(next_leader.len);
                            com_list = true;
                        }
                    } else if do_number_indent {
                        second_indent = number_indent(editor, lnum, &f);
                        com_list = !both_plain;
                    }
                }
                // A change of comment leader ends the paragraph, unless the next line is a line
                // comment and this one has one after some text.
                if lnum >= editor.text().last_line()
                    || !f.same_leader(
                        &line_of(editor, lnum),
                        leader,
                        &line_of(editor, lnum + 1),
                        next_leader,
                    )
                {
                    // Vim compares the next line's leader flags with `://`; for a line with
                    // no leader they're left at the last entry of 'comments'.
                    let parts = f.parts();
                    let next_part = match next_leader.part {
                        Some(k) => parts.get(k),
                        None if lnum < editor.text().last_line()
                            && !line_of(editor, lnum + 1).trim().is_empty() =>
                        {
                            parts.last()
                        }
                        None => None,
                    };
                    let next_is_line_comment =
                        next_part.is_some_and(|p| p.flags.is_empty() && p.string.starts_with("//"));
                    if !next_is_line_comment
                        || comments::check_linecomment(&line_of(editor, lnum)).is_none()
                    {
                        is_end_par = true;
                    }
                }
                if is_end_par || force_format {
                    if need_set_indent {
                        // The indent is made again of tabs and spaces as the options say: the
                        // first line's as it is, later paragraphs' as the indenter has it.
                        let ts = editor.buf_opts().tabstop;
                        let mut amount = util::indent_width(&line_of(editor, lnum), ts);
                        if lnum != first_line && crate::indent::cindent_on(editor) {
                            editor.window.cursor = pos(lnum, 0);
                            editor.update_syntax_within(None);
                            amount = crate::indent::get_indent(editor, lnum).unwrap_or(amount);
                        }
                        self.set_line_indent(editor, lnum, amount);
                    }
                    // On the last non-blank, then break the line as typing there would.
                    let s: Vec<char> = line_of(editor, lnum).chars().collect();
                    let mut col = s.len().saturating_sub(1);
                    while col > 0 && s[col].is_whitespace() {
                        col -= 1;
                    }
                    editor.window.cursor = pos(lnum, col);
                    self.internal_format(
                        editor,
                        &f,
                        second_indent,
                        f.do_comments && com_list,
                        saved,
                    );
                    lnum = editor.cursor().line;
                    second_indent = None;
                    need_set_indent = is_end_par;
                    if is_end_par {
                        first_par_line = true;
                    }
                    force_format = false;
                }
                if !is_end_par {
                    // Join the next line, without its leader (or, for `2`, its indent).
                    advance = false;
                    let next = lnum + 1;
                    let strip = if next_leader.len > 0 {
                        next_leader.len
                    } else if second_indent.is_some_and(|i| i > 0) {
                        line_of(editor, next)
                            .chars()
                            .take_while(|&c| util::is_white(c))
                            .count()
                    } else {
                        0
                    };
                    if strip > 0 {
                        let start = editor.text().line_start(next);
                        self.edit(editor, Edit::delete(start..start + strip));
                        if let Some(p) = saved.as_mut()
                            && p.line == next
                        {
                            p.col = p.col.saturating_sub(strip);
                        }
                    }
                    self.format_join(editor, lnum, saved);
                    first_par_line = false;
                    force_format = editor.text().line_len(lnum) > max_len;
                }
            }
            count -= 1;
        }
        editor.window.cursor = pos(lnum.min(editor.text().last_line()), 0);
    }

    /// Insert `n` spaces at the cursor and move past them.
    fn insert_spaces(&mut self, editor: &mut Editor, n: usize) {
        if n == 0 {
            return;
        }
        let cur = editor.cursor();
        let at = editor.text().pos_to_char(cur.line, cur.col);
        self.edit(editor, Edit::insert(at, " ".repeat(n)));
        editor.window.cursor = pos(cur.line, cur.col + n);
    }

    /// Vim's `do_join(2, true, …)` without 'formatoptions': line `lnum + 1` is appended to
    /// `lnum`, its leading white space replaced by one space (none after white space or before
    /// a `)`).
    fn format_join(&mut self, editor: &mut Editor, lnum: usize, saved: &mut Option<Pos>) {
        let a = util::line(editor, lnum);
        let b = util::line(editor, lnum + 1);
        let white = b.chars().take_while(|&c| util::is_white(c)).count();
        let rest: String = b.chars().skip(white).collect();
        let space = !rest.is_empty()
            && !rest.starts_with(')')
            && !a.is_empty()
            && !a.ends_with('\t')
            && !a.ends_with(' ');
        let joined_at = a.chars().count() + usize::from(space);
        let t = editor.text();
        let from = t.line_start(lnum) + t.line_len(lnum);
        let to = t.line_start(lnum + 1) + white;
        self.edit(
            editor,
            Edit::replace(from..to, if space { " " } else { "" }.to_string()),
        );
        editor.window.cursor = pos(lnum, joined_at);
        if let Some(p) = saved.as_mut() {
            if p.line == lnum + 1 {
                *p = pos(lnum, joined_at + p.col.saturating_sub(white));
            } else if p.line > lnum + 1 {
                p.line -= 1;
            }
        }
    }

    /// Vim's `internal_format` as `format_lines` calls it: break the cursor line at white space
    /// before the margin until it fits, continuing the comment leader.
    fn internal_format(
        &mut self,
        editor: &mut Editor,
        f: &Format,
        mut second_indent: Option<usize>,
        com_list: bool,
        saved: &mut Option<Pos>,
    ) {
        let o = editor.buf_opts().clone();
        let fo = o.formatoptions.clone();
        let mut no_leader = false;
        let mut first_line = true;
        loop {
            let cur = editor.cursor();
            let line: Vec<char> = util::line(editor, cur.line).chars().collect();
            let m = editor.metrics();
            let under = line.get(cur.col).copied().unwrap_or(' ');
            let cells = unicode_width::UnicodeWidthChar::width(under).unwrap_or(1);
            if m.vcol_of(cur.line, cur.col) + cells <= f.textwidth {
                break;
            }
            let do_comments = f.do_comments && !no_leader;
            let s: String = line.iter().collect();
            let leader_len = if do_comments {
                let (mut l, _) = comments::leader_len(&o.comments, &s, false, true);
                if l == 0
                    && o.cindent
                    && let Some(cs) = comments::check_linecomment(&s)
                {
                    let (l2, _) = comments::leader_len(&o.comments, &s[cs..], false, true);
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
            let startcol = cur.col;
            if startcol == 0 {
                break;
            }
            let wantcol = m.col_for_vcol(cur.line, f.textwidth);
            // The white space to break at: the last before the margin.
            let at = |i: usize| line.get(i).copied().unwrap_or('\0');
            let mut i = startcol;
            let mut foundcol = 0;
            loop {
                let mut cc = at(i);
                if util::is_white(cc) {
                    let mut wcc = 0;
                    while i > 0 && util::is_white(cc) {
                        i -= 1;
                        cc = at(i);
                        wcc = (wcc + 1).min(2);
                    }
                    if i == 0 && util::is_white(cc) {
                        break;
                    }
                    // 'formatoptions' `p`: not after a period with one space.
                    if fo.contains('p') && cc == '.' && wcc < 2 {
                        continue;
                    }
                    if i < leader_len {
                        break;
                    }
                    if fo.contains('1') {
                        // Not after a one-letter word.
                        if i == 0 || i <= leader_len {
                            break;
                        }
                        if util::is_white(at(i - 1)) {
                            i -= 1;
                            continue;
                        }
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
                editor.window.cursor = pos(cur.line, startcol);
                break;
            }
            // The text that moves starts after the white space (some of it stays with `w`).
            let mut word = foundcol;
            while util::is_white(at(word)) && (!f.white_par || word < startcol) {
                word += 1;
            }
            let offset = startcol.saturating_sub(word);
            let split = if f.white_par { word } else { foundcol };
            editor.window.cursor = pos(cur.line, split);
            let opened = self.open_line_vim(
                editor,
                OpenFlags {
                    insert: true,
                    do_com: Some(do_comments),
                    del_spaces: true,
                    ..OpenFlags::default()
                },
            );
            if opened.end_comment_pending.is_some() || leader_len > 0 && do_comments {
                no_leader = false;
            }
            let new_line = util::line(editor, opened.line);
            let got_leader =
                do_comments && comments::leader_len(&o.comments, &new_line, false, true).0 > 0;
            // A list's text lines up after its marker, also behind a comment leader.
            if com_list
                && got_leader
                && let Some(si) = second_indent.filter(|&i| i > 0)
            {
                let at = editor.metrics().vcol_of(opened.line, editor.cursor().col);
                let padding = si.saturating_sub(at);
                self.insert_spaces(editor, padding);
            }
            if first_line && !com_list {
                if second_indent.is_none() && fo.contains('n') {
                    second_indent = number_indent(editor, cur.line, f);
                }
                if let Some(si) = second_indent {
                    if leader_len > 0 && si > leader_len {
                        self.insert_spaces(editor, si - leader_len);
                    } else {
                        self.set_line_indent(editor, opened.line, si);
                    }
                }
            }
            first_line = false;
            let newcol = editor.cursor().col;
            if let Some(p) = saved.as_mut() {
                if p.line == cur.line && p.col >= split {
                    *p = pos(opened.line, newcol + p.col.saturating_sub(word));
                } else if p.line > cur.line {
                    p.line += 1;
                }
            }
            let len = editor.text().line_len(opened.line);
            editor.window.cursor = pos(opened.line, (newcol + offset).min(len));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker(s: &str, markdown: bool) -> Option<usize> {
        list_marker(&s.chars().collect::<Vec<_>>(), markdown)
    }

    #[test]
    fn list_markers() {
        assert_eq!(marker("1. one", false), Some(3));
        assert_eq!(marker("  12)\tx", false), Some(6));
        assert_eq!(marker("- one", false), None);
        assert_eq!(marker("- one", true), Some(2));
        assert_eq!(marker("3. x", true), Some(3));
        assert_eq!(marker("3) x", true), None);
        assert_eq!(marker("[^1]: note", true), Some(4));
    }

    #[test]
    fn nroff_macros_start_paragraphs() {
        assert!(starts_paragraph(""));
        assert!(starts_paragraph(".PP"));
        assert!(starts_paragraph(".SH NAME"));
        assert!(!starts_paragraph(".XY"));
        assert!(!starts_paragraph("text"));
    }
}
