//! Indenting by filetype: the indenters Neovim's indent scripts provide ('indentexpr'), Vim's
//! C indenting ('cindent', also what `=` uses without an 'indentexpr'), the keys that reindent
//! a line while typing ('indentkeys' / 'cinkeys'), and the `=` operator.
//!
//! Each indenter is a port of the Neovim 0.12 runtime script it is named after, checked against
//! Neovim with the corpus in `tests/indent` (`cargo xtask indent gen|check`). Lines are 0-based
//! here, unlike Vim script.

// The helpers are for the indenters being ported.
#![allow(dead_code)]

mod cindent;
mod javascript;
mod json;
mod lua;
mod python;
mod rust;
mod sh;
mod typescript;

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;

use flux_core::{Edit, Text};
use flux_view::Editor;
use flux_view::options::BufferOptions;

use crate::engine::Engine;
use crate::util::{self, pos};

/// What the syntax says is at a position, as far as indent scripts care (they test Vim's
/// syntax group names for "Comment", "String", …; flux asks the tree-sitter captures).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SynKind {
    /// Nothing an indenter treats specially, or no syntax information.
    None,
    Comment,
    String,
}

/// What an indenter sees: the buffer, its options, and the line to indent (the cursor line,
/// as Vim puts the cursor there before calling the indenter).
pub(crate) struct Ctx<'a> {
    pub text: &'a Text,
    pub opts: &'a BufferOptions,
    /// The line being indented (Vim's `v:lnum`).
    pub lnum: usize,
    /// The cursor's byte column on `lnum` (0 when indenting with `=`), where searches from
    /// the cursor (Vim's `searchpair()`) start.
    pub col: usize,
    /// In Insert mode (Vim's `mode()` is `i`).
    pub insert: bool,
    /// The cursor is on `lnum`.
    on_cursor_line: bool,
    syntax: Option<&'a flux_syntax::Syntax>,
    spans: RefCell<HashMap<usize, Vec<flux_syntax::Span>>>,
}

impl<'a> Ctx<'a> {
    fn new(editor: &'a Editor, lnum: usize) -> Self {
        let buffer = editor.current_buffer();
        let cur = editor.cursor();
        let col = if cur.line == lnum {
            let s = buffer.text.line_str(lnum);
            s.char_indices().nth(cur.col).map_or(s.len(), |(i, _)| i)
        } else {
            0
        };
        Self {
            col,
            insert: editor.mode == flux_view::Mode::Insert,
            text: &buffer.text,
            opts: &buffer.opts,
            lnum,
            on_cursor_line: cur.line == lnum,
            syntax: buffer.syntax.as_ref().filter(|_| editor.syntax_on),
            spans: RefCell::default(),
        }
    }

    /// A context for tests: `text` with `opts`, no syntax.
    #[cfg(test)]
    pub fn for_test(text: &'a Text, opts: &'a BufferOptions, lnum: usize) -> Self {
        Self {
            text,
            opts,
            lnum,
            col: 0,
            insert: false,
            on_cursor_line: false,
            syntax: None,
            spans: RefCell::default(),
        }
    }

    /// The cursor's byte column when indenting the cursor line in Insert mode.
    pub fn cursor_col(&self) -> Option<usize> {
        (self.insert && self.on_cursor_line).then_some(self.col)
    }

    pub fn line_count(&self) -> usize {
        self.text.line_count()
    }

    /// Line `n` (Vim's `getline()`), empty past the end.
    pub fn line(&self, n: usize) -> Cow<'a, str> {
        if n < self.text.line_count() {
            self.text.line_str(n)
        } else {
            Cow::Borrowed("")
        }
    }

    /// The indent of line `n` in screen columns (Vim's `indent()`).
    pub fn indent(&self, n: usize) -> usize {
        util::indent_width(&self.line(n), self.opts.tabstop)
    }

    /// Vim's `shiftwidth()`.
    pub fn sw(&self) -> usize {
        self.opts.sw()
    }

    /// The last line at or above `n` that isn't blank (Vim's `prevnonblank()`).
    pub fn prevnonblank(&self, n: usize) -> Option<usize> {
        (0..=n.min(self.line_count().saturating_sub(1)))
            .rev()
            .find(|&l| !self.line(l).trim().is_empty())
    }

    /// The first line at or below `n` that isn't blank (Vim's `nextnonblank()`).
    pub fn nextnonblank(&self, n: usize) -> Option<usize> {
        (n..self.line_count()).find(|&l| !self.line(l).trim().is_empty())
    }

    /// Whether syntax information is available (Vim's `has('syntax_items')` with syntax on).
    pub fn has_syntax(&self) -> bool {
        self.syntax.is_some()
    }

    /// The tree-sitter captures at byte `col` of line `line`, innermost last.
    pub fn captures_at(&self, line: usize, col: usize) -> Vec<&'static str> {
        let Some(syntax) = self.syntax else {
            return Vec::new();
        };
        let mut spans = self.spans.borrow_mut();
        let spans = spans
            .entry(line)
            .or_insert_with(|| syntax.highlights(self.text, line..line + 1));
        let s = self.line(line);
        let char_col = s[..col.min(s.len())].chars().count();
        spans
            .iter()
            .filter(|sp| sp.start <= char_col && char_col < sp.end && !sp.capture.is_empty())
            .map(|sp| sp.capture)
            .collect()
    }

    /// What kind of syntax item byte `col` of line `line` is in.
    pub fn syn_kind(&self, line: usize, col: usize) -> SynKind {
        let caps = self.captures_at(line, col);
        if caps.iter().any(|c| c.contains("comment")) {
            SynKind::Comment
        } else if caps
            .iter()
            .any(|c| c.contains("string") || c.contains("character"))
        {
            SynKind::String
        } else {
            SynKind::None
        }
    }
}

/// The indent for line `lnum` of the current buffer: from 'indentexpr' if set, otherwise C
/// indenting (Vim's `=` does that even without 'cindent'). `None` keeps the current indent
/// (an 'indentexpr' returning -1).
pub(crate) fn get_indent(editor: &Editor, lnum: usize) -> Option<usize> {
    let ctx = Ctx::new(editor, lnum);
    let expr = ctx.opts.indentexpr.as_str();
    if expr.is_empty() {
        return cindent::get_c_indent(&ctx);
    }
    match expr {
        "GetRustIndent(v:lnum)" => rust::indent(&ctx),
        "python#GetIndent(v:lnum)" => python::indent(&ctx),
        "GetLuaIndent()" => lua::indent(&ctx),
        "GetShIndent()" => sh::indent(&ctx),
        "GetJavascriptIndent()" => javascript::indent(&ctx),
        "GetTypescriptIndent()" => typescript::indent(&ctx),
        "GetJSONIndent(v:lnum)" => json::indent(&ctx),
        // An expression flux can't evaluate: keep the indent.
        _ => None,
    }
}

/// Vim's `cindent_on()`: typing reindents lines ('cindent' or an 'indentexpr').
pub(crate) fn cindent_on(editor: &Editor) -> bool {
    let o = editor.buf_opts();
    o.cindent || !o.indentexpr.is_empty()
}

/// A key as 'indentkeys' sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Typed {
    Char(char),
    /// A new line opened below (`o`, `<CR>`): the `o` entry.
    OpenBelow,
    /// A new line opened above (`O`): the `O` entry.
    OpenAbove,
}

/// When an 'indentkeys' entry applies: `*` before inserting the key, `!` instead of
/// inserting it, ` ` after inserting it (Vim's `in_cinkeys` `when`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum When {
    Before,
    Instead,
    After,
}

/// Vim's `in_cinkeys`: whether `typed` reindents the current line. `line_is_empty` is true when
/// only blanks precede the cursor.
pub(crate) fn in_cinkeys(editor: &Editor, typed: Typed, when: When, line_is_empty: bool) -> bool {
    let o = editor.buf_opts();
    let keys = if o.indentexpr.is_empty() {
        o.cinkeys.as_str()
    } else {
        o.indentkeys.as_str()
    };
    let cur = editor.cursor();
    let line = editor.text().line_str(cur.line);
    // The cursor as a byte offset, as Vim works with it.
    let col = line
        .char_indices()
        .nth(cur.col)
        .map_or(line.len(), |(i, _)| i);
    let look = keys.as_bytes();
    let mut i = 0;
    while i < look.len() {
        let mut try_match = match when {
            When::Before => look[i] == b'*',
            When::Instead => look[i] == b'!',
            When::After => look[i] != b'*',
        };
        if look[i] == b'*' || look[i] == b'!' {
            i += 1;
        }
        let try_match_word;
        if look.get(i) == Some(&b'0') {
            try_match_word = try_match;
            if !line_is_empty {
                try_match = false;
            }
            i += 1;
        } else {
            try_match_word = false;
        }
        let at = |k: usize| look.get(k).copied().unwrap_or(0);
        if at(i) == b'^' && (b'?'..=b'_').contains(&at(i + 1)) {
            // A control character: `^F`.
            let ctrl = char::from(at(i + 1) ^ 0x40);
            if try_match && typed == Typed::Char(ctrl) {
                return true;
            }
            i += 2;
        } else if at(i) == b'o' {
            if try_match && typed == Typed::OpenBelow {
                return true;
            }
            i += 1;
        } else if at(i) == b'O' {
            if try_match && typed == Typed::OpenAbove {
                return true;
            }
            i += 1;
        } else if at(i) == b'e' {
            // `else` at the start of the line, just typed.
            if try_match && typed == Typed::Char('e') && col >= 4 {
                let first = line.len() - line.trim_start_matches([' ', '\t']).len();
                if first == col - 4 && line[col - 4..].starts_with("else") {
                    return true;
                }
            }
            i += 1;
        } else if at(i) == b':' {
            // At the end of a label or case, or of `class::method`.
            if try_match && typed == Typed::Char(':') {
                let ctx = Ctx::new(editor, cur.line);
                if cindent::is_case(&ctx, &line, false)
                    || cindent::is_scopedecl(&ctx, &line)
                    || cindent::is_label(&ctx, &line)
                {
                    return true;
                }
                let b = line.as_bytes();
                if col > 2 && b[col - 1] == b':' && b[col - 2] == b':' {
                    let mut l = line.to_string();
                    l.replace_range(col - 1..col, " ");
                    if cindent::is_case(&ctx, &l, false)
                        || cindent::is_scopedecl(&ctx, &l)
                        || cindent::is_label(&ctx, &l)
                    {
                        return true;
                    }
                }
            }
            i += 1;
        } else if at(i) == b'<' {
            // `<:>`, `<>>`, …: a key that otherwise has a meaning here.
            if try_match
                && b"<>!*oOe0:".contains(&at(i + 1))
                && at(i + 2) == b'>'
                && typed == Typed::Char(char::from(at(i + 1)))
            {
                return true;
            }
            while i < look.len() && look[i] != b'>' {
                i += 1;
            }
            while i < look.len() && look[i] == b'>' {
                i += 1;
            }
        } else if at(i) == b'=' && at(i + 1) != b',' && at(i + 1) != 0 {
            // `=word`: the word just typed (at the start of the line for `0=word`).
            i += 1;
            let icase = at(i) == b'~';
            if icase {
                i += 1;
            }
            let end = look[i..]
                .iter()
                .position(|&b| b == b',')
                .map_or(look.len(), |p| i + p);
            let word = &keys[i..end];
            let n = word.len();
            if (try_match || try_match_word)
                && col >= n
                && let Typed::Char(c) = typed
            {
                let last = word.chars().last().unwrap_or('\0');
                let same = |a: char, b: char| {
                    if icase {
                        a.eq_ignore_ascii_case(&b)
                    } else {
                        a == b
                    }
                };
                if same(c, last) {
                    let before = &line[..col];
                    let typed_word = before.get(col - n..).unwrap_or("");
                    let word_start = col == n
                        || !before[..col - n]
                            .chars()
                            .last()
                            .is_some_and(flux_core::chars::is_keyword);
                    let mut matched = word_start
                        && if icase {
                            typed_word.eq_ignore_ascii_case(word)
                        } else {
                            typed_word == word
                        };
                    if matched && try_match_word && !try_match {
                        // `0=word`: only blanks before the word.
                        let blanks = line.len() - line.trim_start_matches([' ', '\t']).len();
                        matched = blanks == col - n;
                    }
                    if matched {
                        return true;
                    }
                }
            }
            i = end;
        } else {
            // A plain character.
            if try_match && typed == Typed::Char(char::from(at(i))) {
                return true;
            }
            if i < look.len() {
                i += 1;
            }
        }
        // Skip to the next entry.
        while i < look.len() && look[i] != b',' {
            i += 1;
        }
        while i < look.len() && (look[i] == b',' || look[i] == b' ') {
            i += 1;
        }
    }
    false
}

impl Engine {
    /// Set the indent of line `lnum` to `amount` columns (Vim's `set_indent`), made of tabs and
    /// spaces as 'expandtab' says. Returns whether the line changed. The cursor, if on the
    /// line, stays on the same text (or at the end of the indent).
    pub(crate) fn set_line_indent(
        &mut self,
        editor: &mut Editor,
        lnum: usize,
        amount: usize,
    ) -> bool {
        let s = util::line(editor, lnum);
        let old = util::indent_of(&s);
        let new = util::make_indent(amount, editor.buf_opts());
        if old == new {
            return false;
        }
        let old_len = old.chars().count();
        let start = editor.text().line_start(lnum);
        let cur = editor.cursor();
        self.edit(editor, Edit::replace(start..start + old_len, new.clone()));
        if cur.line == lnum {
            let new_len = new.chars().count();
            let col = if cur.col >= old_len {
                cur.col - old_len + new_len
            } else {
                new_len
            };
            editor.window.cursor = pos(lnum, col);
        }
        true
    }

    /// Vim's `do_c_expr_indent`: reindent the cursor line as the indenter says. Returns whether
    /// the line is left holding only its indent, which goes again if nothing is typed (Vim's
    /// `fixthisline` setting `did_ai`).
    pub(crate) fn fix_this_line(&mut self, editor: &mut Editor) -> bool {
        editor.update_syntax();
        let lnum = editor.cursor().line;
        let Some(amount) = get_indent(editor, lnum) else {
            return false;
        };
        self.set_line_indent(editor, lnum, amount);
        let s = util::line(editor, lnum);
        !s.is_empty() && s.chars().all(util::is_white)
    }

    /// The `=` operator on lines `first..=last` (Vim's `op_reindent`): each line gets the
    /// indent the indenter gives it, blank lines become empty.
    pub(crate) fn reindent(&mut self, editor: &mut Editor, first: usize, last: usize) {
        let want = editor.window.curswant;
        for lnum in first..=last {
            editor.window.cursor = pos(lnum, 0);
            let s = util::line(editor, lnum);
            let amount = if s.chars().all(util::is_white) {
                Some(0)
            } else {
                editor.update_syntax();
                get_indent(editor, lnum)
            };
            if let Some(amount) = amount {
                self.set_line_indent(editor, lnum, amount);
            }
        }
        // To the first line, in the column wanted ('nostartofline').
        let col = editor
            .metrics()
            .col_for_vcol(first, want)
            .min(editor.text().line_len(first).saturating_sub(1));
        editor.window.cursor = pos(first, col);
        let n = last - first + 1;
        if n > editor.options.report {
            let lines = if n == 1 { "line" } else { "lines" };
            editor.info(format!("{n} {lines} indented "));
        }
    }
}
