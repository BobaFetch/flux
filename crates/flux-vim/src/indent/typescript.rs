//! `GetTypescriptIndent()`, ported from Neovim's `runtime/indent/typescript.vim` (yats.vim),
//! which `typescriptreact` buffers use too.
//!
//! The script tests Vim's syntax group names at positions; in `nvim --clean` those come from
//! the legacy `runtime/syntax/shared/typescriptcommon.vim`, whose strings, comments and regexes
//! [`scan`] reproduces. Its names decide what counts: `typescriptString`,
//! `typescriptRegexpString` and the comments are strings or comments to the script, but
//! template literals (`typescriptTemplate`) and escapes (`typescriptSpecial`) are not. Lines
//! are numbered from 1 here, with 0 for none, as in the script.

use super::javascript::{Highlight, Syn, is_blank, is_kw};
use super::{Ctx, cindent};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Region {
    Code,
    Comment,
    Tpl,
    /// `${...}`, with the braces opened inside it.
    Subst(usize),
    /// A string continued on the next line with a backslash.
    Str(u8),
}

/// A `"`/`'` string from `i` (or continuing at `i` when `open`), as typescriptString: escapes
/// (typescriptSpecial), ending at the quote or the end of the line unless a backslash ends the
/// line. Returns where it ends and whether it goes on to the next line.
fn string(b: &[u8], i: usize, q: u8, open: bool, cls: &mut [Syn]) -> (usize, bool) {
    let mut k = i;
    if !open {
        cls[k] = Syn::Str;
        k += 1;
    }
    while k < b.len() {
        if b[k] == b'\\' {
            if k + 1 == b.len() {
                cls[k] = Syn::Special;
                return (b.len(), true);
            }
            cls[k] = Syn::Special;
            cls[k + 1] = Syn::Special;
            k += 2;
            continue;
        }
        cls[k] = Syn::Str;
        k += 1;
        if b[k - 1] == q {
            return (k, false);
        }
    }
    (k, false)
}

/// Whether a `/` at `j` starts a typescriptRegexpString: after `return`, `typeof`, or a
/// character that can't end a value (or at the start of a line), and not `//` or `/*`.
fn regex_start(b: &[u8], j: usize) -> bool {
    if !b.get(j + 1).is_some_and(|&c| c != b'*' && c != b'/') {
        return false;
    }
    let mut p = j;
    while p > 0 && is_blank(b[p - 1]) {
        p -= 1;
    }
    if p == 0 {
        return true;
    }
    let before = &b[..p];
    for w in [&b"return"[..], b"typeof"] {
        if before.ends_with(w) && (p == w.len() || !is_kw(b[p - w.len() - 1])) {
            return true;
        }
    }
    let c = b[p - 1];
    !(c == b')'
        || c == b']'
        || c == b'\''
        || c == b'"'
        || c.is_ascii_alphanumeric()
        || c == b'_'
        || c == b'$')
}

/// Where a regex starting at `j` ends (after its flags), if it ends on the line.
fn regex_end(b: &[u8], j: usize) -> Option<usize> {
    let mut k = j + 1;
    while k < b.len() {
        match b[k] {
            b'\\' => k += 2,
            b'[' => match b[k + 1..].iter().position(|&c| c == b']') {
                Some(p) if p > 0 => k += p + 2,
                _ => k += 1,
            },
            b'/' => {
                let mut e = k + 1;
                while e < b.len() && e - k <= 6 && b"gimyus".contains(&b[e]) {
                    e += 1;
                }
                return Some(e);
            }
            _ => k += 1,
        }
    }
    None
}

/// The TypeScript syntax's strings, comments, templates and regexes.
fn scan(lines: &[String]) -> Highlight {
    let mut stack = vec![Region::Code];
    let mut bytes = Vec::with_capacity(lines.len());
    let mut starts = Vec::with_capacity(lines.len());
    for line in lines {
        let b = line.as_bytes();
        let top = *stack.last().expect("code at the bottom");
        starts.push(match top {
            Region::Comment => Syn::Comment,
            Region::Tpl => Syn::Tpl,
            Region::Str(_) => Syn::Str,
            _ => Syn::None,
        });
        let mut cls = vec![Syn::None; b.len()];
        let mut i = 0;
        if let Region::Str(q) = top {
            stack.pop();
            let (end, more) = string(b, 0, q, true, &mut cls);
            if more {
                stack.push(Region::Str(q));
            }
            i = end;
        }
        while i < b.len() {
            let top = *stack.last().expect("code at the bottom");
            match top {
                Region::Comment => {
                    cls[i] = Syn::Comment;
                    if b[i..].starts_with(b"*/") {
                        cls[i + 1] = Syn::Comment;
                        i += 2;
                        stack.pop();
                    } else {
                        i += 1;
                    }
                }
                Region::Tpl => {
                    if b[i] == b'\\' && i + 1 < b.len() {
                        cls[i] = Syn::Special;
                        cls[i + 1] = Syn::Special;
                        i += 2;
                    } else if b[i] == b'`' {
                        cls[i] = Syn::Tpl;
                        i += 1;
                        stack.pop();
                    } else if b[i..].starts_with(b"${") {
                        i += 2;
                        stack.push(Region::Subst(0));
                    } else {
                        cls[i] = Syn::Tpl;
                        i += 1;
                    }
                }
                Region::Code | Region::Subst(_) | Region::Str(_) => {
                    if b[i..].starts_with(b"/*") {
                        cls[i] = Syn::Comment;
                        cls[i + 1] = Syn::Comment;
                        i += 2;
                        stack.push(Region::Comment);
                    } else if b[i..].starts_with(b"//") {
                        cls[i..].fill(Syn::LineComment);
                        i = b.len();
                    } else if b[i] == b'"' || b[i] == b'\'' {
                        let (end, more) = string(b, i, b[i], false, &mut cls);
                        if more {
                            stack.push(Region::Str(b[i]));
                        }
                        i = end;
                    } else if b[i] == b'`' {
                        cls[i] = Syn::Tpl;
                        i += 1;
                        stack.push(Region::Tpl);
                    } else if b[i] == b'/' && regex_start(b, i) {
                        match regex_end(b, i) {
                            Some(end) => {
                                cls[i..end].fill(Syn::Regex);
                                i = end;
                            }
                            None => i += 1,
                        }
                    } else {
                        if let Region::Subst(depth) = top {
                            match b[i] {
                                b'{' => {
                                    *stack.last_mut().expect("subst") = Region::Subst(depth + 1)
                                }
                                b'}' if depth == 0 => {
                                    stack.pop();
                                }
                                b'}' => {
                                    *stack.last_mut().expect("subst") = Region::Subst(depth - 1)
                                }
                                _ => {}
                            }
                        }
                        i += 1;
                    }
                }
            }
        }
        bytes.push(cls);
    }
    Highlight { bytes, starts }
}

/// 'iskeyword' in TypeScript buffers: the defaults plus `$` and `#`.
fn is_kw_ts(b: u8) -> bool {
    is_kw(b) || b == b'$' || b == b'#'
}

/// `s:line_term` matching at `j`: blanks, then nothing or a `//` comment.
fn line_term(b: &[u8], j: usize) -> bool {
    let mut k = j.min(b.len());
    while k < b.len() && is_blank(b[k]) {
        k += 1;
    }
    k == b.len() || b[k..].starts_with(b"//")
}

/// `match(line, s:continuation_regex)`: where the leftmost match starts.
fn match_continuation(b: &[u8]) -> Option<usize> {
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    (0..b.len()).find(|&i| {
        let c = b[i];
        if b"\\*+/.:".contains(&c) && line_term(b, i + 1) {
            return true;
        }
        if (c == b'=' || c == b'-') && !(i >= 2 && &b[i - 2..i] == b"<%") && line_term(b, i + 1) {
            return true;
        }
        if !word(c) && b.get(i + 1).is_some_and(|n| b"|&?".contains(n)) && line_term(b, i + 2) {
            return true;
        }
        if (b[i..].starts_with(b"||") || b[i..].starts_with(b"&&")) && line_term(b, i + 2) {
            return true;
        }
        c != b'='
            && b.get(i + 1) == Some(&b'=')
            && b.get(i + 2).is_some_and(|&n| n != b'=')
            && (i + 3..b.len()).any(|k| b[k] == b',' && line_term(b, k + 1))
    })
}

/// `match(line, s:one_line_scope_regex)`: `if`, `else`, `for` or `while` with no `{` or `;`
/// after it (other than in a trailing comment).
fn match_one_line_scope(b: &[u8]) -> Option<usize> {
    (0..b.len()).find(|&i| {
        if i > 0 && is_kw_ts(b[i - 1]) {
            return false;
        }
        ["if", "else", "for", "while"].iter().any(|w| {
            let w = w.as_bytes();
            if !b[i..].starts_with(w) || b.get(i + w.len()).is_some_and(|&c| is_kw_ts(c)) {
                return false;
            }
            let rest = &b[i + w.len()..];
            match rest.iter().position(|&c| c == b'{' || c == b';') {
                None => true,
                Some(q) => rest[..q].windows(2).any(|x| x == b"//"),
            }
        })
    })
}

/// `match(line, s:block_regex)`: a `{` or `[` that ends the line, maybe with `|params|`.
fn match_block(b: &[u8]) -> Option<usize> {
    let ident = |b: &[u8], mut k: usize| -> Option<usize> {
        if b.get(k).is_some_and(|&c| c == b'*' || c == b'@') {
            k += 1;
        }
        if !b
            .get(k)
            .is_some_and(|&c| c.is_ascii_alphabetic() || c == b'_')
        {
            return None;
        }
        while b
            .get(k)
            .is_some_and(|&c| c.is_ascii_alphanumeric() || c == b'_')
        {
            k += 1;
        }
        Some(k)
    };
    (0..b.len()).find(|&i| {
        if b[i] != b'{' && b[i] != b'[' {
            return false;
        }
        let mut j = i + 1;
        while j < b.len() && is_blank(b[j]) {
            j += 1;
        }
        if line_term(b, j) {
            return true;
        }
        // `|a, b|`
        if b.get(j) != Some(&b'|') {
            return false;
        }
        let Some(mut k) = ident(b, j + 1) else {
            return false;
        };
        if b.get(k) == Some(&b',') {
            k += 1;
        }
        while b.get(k).is_some_and(|&c| is_blank(c)) {
            k += 1;
        }
        loop {
            if b.get(k) == Some(&b'|') {
                return line_term(b, k + 1);
            }
            if b.get(k) != Some(&b',') {
                return false;
            }
            k += 1;
            while b.get(k).is_some_and(|&c| is_blank(c)) {
                k += 1;
            }
            match ident(b, k) {
                Some(e) => k = e,
                None => return false,
            }
        }
    })
}

struct Ts<'a> {
    ctx: &'a Ctx<'a>,
    lines: Vec<String>,
    syn: Highlight,
}

impl Ts<'_> {
    /// `getline(lnum)`, 1-based.
    fn getline(&self, lnum: usize) -> &[u8] {
        lnum.checked_sub(1)
            .and_then(|l| self.lines.get(l))
            .map_or(&[], |s| s.as_bytes())
    }

    fn indent(&self, lnum: usize) -> isize {
        if lnum == 0 || lnum > self.lines.len() {
            return -1;
        }
        self.ctx.indent(lnum - 1) as isize
    }

    fn sw(&self) -> isize {
        self.ctx.sw() as isize
    }

    /// The syntax at 1-based `col` of line `lnum` (nothing past the end, like `synID()`).
    fn syn(&self, lnum: usize, col: usize) -> Syn {
        if lnum == 0 || col == 0 {
            return Syn::None;
        }
        self.syn.at(lnum - 1, col - 1)
    }

    fn in_string_or_comment(&self, lnum: usize, col: usize) -> bool {
        matches!(
            self.syn(lnum, col),
            Syn::Str | Syn::Regex | Syn::Comment | Syn::LineComment
        )
    }

    fn in_string(&self, lnum: usize, col: usize) -> bool {
        self.syn(lnum, col) == Syn::Regex
    }

    fn in_multiline_comment(&self, lnum: usize, col: usize) -> bool {
        self.syn(lnum, col) == Syn::Comment
    }

    fn is_line_comment(&self, lnum: usize, col: usize) -> bool {
        self.syn(lnum, col) == Syn::LineComment
    }

    /// `prevnonblank(lnum)`.
    fn prevnonblank(&self, lnum: usize) -> usize {
        let mut l = lnum.min(self.lines.len());
        while l > 0 && self.getline(l).iter().all(|&c| is_blank(c)) {
            l -= 1;
        }
        l
    }

    /// `s:PrevNonBlankNonString(lnum)`.
    fn prev_non_blank_non_string(&self, lnum: usize) -> usize {
        let mut in_block = false;
        let mut lnum = self.prevnonblank(lnum);
        while lnum > 0 {
            let line = self.getline(lnum);
            let has = |p: &[u8]| line.windows(2).any(|w| w == p);
            if has(b"/*") {
                if in_block {
                    in_block = false;
                } else {
                    break;
                }
            } else if !in_block && has(b"*/") {
                in_block = true;
            } else if !in_block {
                let start = line
                    .iter()
                    .position(|&c| !is_blank(c))
                    .unwrap_or(line.len());
                let line_comment = line[start..].starts_with(b"//");
                if !line_comment
                    && !(self.in_string_or_comment(lnum, 1)
                        && self.in_string_or_comment(lnum, line.len()))
                {
                    break;
                }
            }
            lnum = self.prevnonblank(lnum - 1);
        }
        lnum
    }

    /// `s:Match(lnum, regex)`: the 1-based column of the leftmost match, if it's not in a string
    /// or comment.
    fn matches(&self, lnum: usize, m: fn(&[u8]) -> Option<usize>) -> usize {
        match m(self.getline(lnum)) {
            Some(i) if !self.in_string_or_comment(lnum, i + 1) => i + 1,
            _ => 0,
        }
    }

    /// `s:GetMSL(lnum, in_one_line_scope)`: the line that starts the statement `lnum` continues.
    fn get_msl(&self, lnum: usize, in_one_line_scope: bool) -> usize {
        let mut msl = lnum;
        let mut l = self.prev_non_blank_non_string(lnum.saturating_sub(1));
        while l > 0 {
            let line = self.getline(l);
            let col = match_continuation(line).map_or(0, |i| i + 1);
            if (col > 0 && !self.in_string_or_comment(l, col)) || self.in_string(l, line.len()) {
                msl = l;
            } else {
                if in_one_line_scope {
                    break;
                }
                if self.matches(l, match_one_line_scope) == 0 {
                    break;
                }
            }
            l = self.prev_non_blank_non_string(l - 1);
        }
        msl
    }

    /// `s:InMultiVarStatement(lnum)`: the `var` line of a multi-line `var` statement.
    fn in_multi_var_statement(&self, lnum: usize) -> usize {
        const KEYWORDS: [&str; 28] = [
            "break",
            "case",
            "catch",
            "continue",
            "debugger",
            "default",
            "delete",
            "do",
            "else",
            "finally",
            "for",
            "function",
            "if",
            "in",
            "instanceof",
            "new",
            "return",
            "switch",
            "this",
            "throw",
            "try",
            "typeof",
            "var",
            "void",
            "while",
            "with",
            "",
            "",
        ];
        let mut l = self.prev_non_blank_non_string(lnum.saturating_sub(1));
        while l > 0 {
            let line = self.getline(l);
            let start = line
                .iter()
                .position(|&c| !is_blank(c))
                .unwrap_or(line.len());
            let rest = &line[start..];
            if KEYWORDS
                .iter()
                .any(|k| !k.is_empty() && rest.starts_with(k.as_bytes()))
            {
                return if rest.starts_with(b"var") { l } else { 0 };
            }
            l = self.prev_non_blank_non_string(l - 1);
        }
        0
    }

    /// `s:LineHasOpeningBrackets(lnum)`: whether `(`, `{` and `[` are left open on the line.
    fn line_has_opening_brackets(&self, lnum: usize) -> [bool; 3] {
        let mut open = [0isize; 3];
        for (pos, &c) in self.getline(lnum).iter().enumerate() {
            let Some(idx) = b"(){}[]".iter().position(|&x| x == c) else {
                continue;
            };
            if self.in_string_or_comment(lnum, pos + 1) {
                continue;
            }
            if idx % 2 == 0 {
                open[idx / 2] += 1;
            } else {
                open[idx / 2] -= 1;
            }
        }
        open.map(|n| n > 0)
    }

    /// `s:IndentWithContinuation(lnum, ind, width)`.
    fn indent_with_continuation(&self, lnum: usize, ind: isize, width: isize) -> isize {
        let p_lnum = lnum;
        let lnum = self.get_msl(lnum, true);
        let line_len = self.getline(lnum).len();
        if p_lnum != lnum
            && (self.matches(p_lnum, match_continuation) > 0 || self.in_string(p_lnum, line_len))
        {
            return ind;
        }
        let msl_ind = self.indent(lnum);
        if self.matches(lnum, match_continuation) > 0 {
            return if lnum == p_lnum {
                msl_ind + width
            } else {
                msl_ind
            };
        }
        ind
    }

    fn in_one_line_scope(&self, lnum: usize) -> usize {
        let msl = self.get_msl(lnum, true);
        if msl > 0 && self.matches(msl, match_one_line_scope) > 0 {
            msl
        } else {
            0
        }
    }

    fn exiting_one_line_scope(&self, lnum: usize) -> usize {
        let msl = self.get_msl(lnum, true);
        if msl > 0 {
            if self.matches(msl, match_one_line_scope) > 0 {
                return 0;
            }
            let prev_msl = self.get_msl(msl - 1, true);
            if self.matches(prev_msl, match_one_line_scope) > 0 {
                return prev_msl;
            }
        }
        0
    }

    /// Vim's `searchpair(open, '', close, 'bW', s:skip_expr)` from `(lnum, col)` (1-based,
    /// the match must be before it): where the unmatched `open` is.
    fn searchpair(&self, open: u8, close: u8, lnum: usize, col: usize) -> Option<(usize, usize)> {
        let mut nest = 1;
        let (mut l, mut c) = (lnum, col.saturating_sub(1));
        loop {
            if c == 0 {
                if l <= 1 {
                    return None;
                }
                l -= 1;
                c = self.getline(l).len();
                continue;
            }
            c -= 1;
            let ch = self.getline(l)[c];
            if ch != open && ch != close {
                continue;
            }
            if self.in_string_or_comment(l, c + 1) {
                continue;
            }
            if ch == open {
                nest -= 1;
                if nest == 0 {
                    return Some((l, c + 1));
                }
            } else {
                nest += 1;
            }
        }
    }

    /// `virtcol()` of 1-based `col` of `lnum`: the last screen column the character takes.
    fn virtcol(&self, lnum: usize, col: usize) -> isize {
        let ts = self.ctx.opts.tabstop.max(1);
        let mut v = 0;
        for &c in &self.getline(lnum)[..col] {
            v = if c == b'\t' { v + ts - v % ts } else { v + 1 };
        }
        v as isize
    }

    fn get_indent(&self) -> isize {
        let v = self.ctx.lnum + 1;
        let line = self.getline(v).to_vec();
        let prevline = self.prevnonblank(v - 1);
        let first = line.iter().position(|&c| !is_blank(c));
        let sw = self.sw();

        // A line starting with a closing bracket: indent like the line with its match.
        if let Some(f) = first.filter(|&f| b"],})".contains(&line[f])) {
            let col = f + 1;
            if !self.in_string_or_comment(v, col) {
                let lvar = self.in_multi_var_statement(v);
                if lvar > 0 && line[f] == b',' {
                    let prev = remove_trailing_comments(self.getline(prevline));
                    let prev = prev.trim_end_matches([' ', '\t']);
                    if prev.ends_with([';', ',']) {
                        return self.indent(self.get_msl(v, false));
                    } else if prev.trim_start_matches([' ', '\t']).starts_with(',') {
                        return self.indent(prevline);
                    } else {
                        return self.indent(lvar) + sw;
                    }
                }
                let (open, close) = match line[f] {
                    b')' => (b'(', b')'),
                    b'}' => (b'{', b'}'),
                    b']' => (b'[', b']'),
                    // A leading comma outside a `var`: an empty pair to look for, which isn't
                    // worth emulating.
                    _ => return -1,
                };
                return match self.searchpair(open, close, v, col) {
                    Some((l, c)) => {
                        if line[f] == b')' && c != self.getline(l).len() {
                            self.virtcol(l, c) - 1
                        } else {
                            self.indent(self.get_msl(l, false))
                        }
                    }
                    None => -1,
                };
            }
        }

        // Comma first: back a level.
        let prev = self.getline(prevline);
        let prev_first = prev.iter().position(|&c| !is_blank(c));
        if prev_first.is_some_and(|f| prev[f] == b',') {
            return self.indent(prevline) - sw;
        }

        // `^\s\+[?|:]`: a ternary's branches.
        let ws = line.iter().take_while(|&&c| is_blank(c)).count();
        if ws > 0 && line.get(ws).is_some_and(|c| b"?|:".contains(c)) {
            let pws = prev.iter().take_while(|&&c| is_blank(c)).count();
            return if pws > 0 && prev.get(pws) == Some(&b'?') {
                self.indent(prevline)
            } else {
                self.indent(prevline) + sw
            };
        }

        // In a multi-line comment, cindent does the right thing.
        if self.in_multiline_comment(v, 1) && !self.is_line_comment(v, 1) {
            return cindent::get_c_indent(self.ctx).map_or(-1, |n| n as isize);
        }

        let blank = first.is_none();
        if blank && self.in_multiline_comment(prevline, 1) {
            return self.indent(prevline) - 1;
        }

        let lnum = self.prev_non_blank_non_string(v - 1);
        if blank && lnum != prevline {
            return self.indent(self.prevnonblank(v));
        }
        if lnum == 0 {
            return 0;
        }

        let pline = self.getline(lnum);
        let ind = self.indent(lnum);

        if self.matches(lnum, match_block) > 0 {
            return self.indent(self.get_msl(lnum, false)) + sw;
        }

        if pline.iter().any(|&c| c == b'[' || c == b'(' || c == b'{') {
            let [paren, brace, bracket] = self.line_has_opening_brackets(lnum);
            if paren && let Some((l, c)) = self.searchpair(b'(', b')', v, 1) {
                return if c == self.getline(l).len() {
                    ind + sw
                } else {
                    self.virtcol(l, c)
                };
            } else if brace || bracket {
                return ind + sw;
            }
        }

        let mut ind = self.indent_with_continuation(lnum, ind, sw);
        let ols = self.in_one_line_scope(lnum);
        if ols > 0 {
            ind += sw;
        } else {
            let mut ols = self.exiting_one_line_scope(lnum);
            while ols > 0 && ind > 0 {
                ind -= sw;
                ols = self.in_one_line_scope(ols - 1);
            }
        }
        ind
    }
}

/// `s:RemoveTrailingComments(content)`.
fn remove_trailing_comments(line: &[u8]) -> String {
    let mut s = String::from_utf8_lossy(line).into_owned();
    if let Some(i) = s.find("//") {
        s.truncate(i);
    }
    if let Some(i) = s.find("/*")
        && s[i..].trim_end().ends_with("*/")
    {
        s.truncate(i);
    }
    s
}

/// The indent for `ctx.lnum`, or `None` to keep it.
pub(super) fn indent(ctx: &Ctx) -> Option<usize> {
    let lines: Vec<String> = (0..ctx.line_count())
        .map(|l| ctx.line(l).into_owned())
        .collect();
    let syn = scan(&lines[..(ctx.lnum + 1).min(lines.len())]);
    let ts = Ts { ctx, lines, syn };
    usize::try_from(ts.get_indent()).ok()
}
