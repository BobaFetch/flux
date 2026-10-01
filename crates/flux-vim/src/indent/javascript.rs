//! `GetJavascriptIndent()`, ported from Neovim's `runtime/indent/javascript.vim`
//! (vim-javascript), which `javascriptreact` buffers use too.
//!
//! The script asks Vim's syntax highlighting whether a position is in a string, comment or
//! regex. In `nvim --clean` that is the legacy `runtime/syntax/javascript.vim`, whose regions
//! [`scan`] reproduces (tree-sitter sees the code differently: it knows a `/.../` after `(` is
//! a regex, the legacy syntax doesn't). `b:js_cache`, a performance cache that carries results
//! from one line to the next, isn't kept: each line is worked out from scratch.

use std::collections::HashMap;

use super::{Ctx, cindent};

/// What the legacy syntax says a byte is, as far as the indent scripts care.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Syn {
    None,
    /// A `"..."` or `'...'` string.
    Str,
    /// A template literal, outside `${...}`.
    Tpl,
    /// A backslash escape in a string or template.
    Special,
    /// A `/* */` comment (and `javaScriptCommentSkip`).
    Comment,
    /// A `//` comment.
    LineComment,
    Regex,
}

/// Every byte's [`Syn`], and what is going on where each line starts (for an empty line).
pub(super) struct Highlight {
    pub bytes: Vec<Vec<Syn>>,
    pub starts: Vec<Syn>,
}

impl Highlight {
    pub fn at(&self, line: usize, col: usize) -> Syn {
        self.bytes
            .get(line)
            .and_then(|l| l.get(col))
            .copied()
            .unwrap_or(Syn::None)
    }
}

pub(super) fn is_blank(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

/// Vim's default 'iskeyword' (`@,48-57,_,192-255`); multibyte characters count as letters.
pub(super) fn is_kw(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Region {
    Code,
    Comment,
    Tpl,
    Embed,
}

/// A `"` or `'` string from `i` (javaScriptStringD/S: ends at the quote or the end of the line,
/// with javaScriptSpecial escapes). Returns where it ends.
fn string(b: &[u8], i: usize, cls: &mut [Syn]) -> usize {
    let q = b[i];
    cls[i] = Syn::Str;
    let mut k = i + 1;
    while k < b.len() {
        if b[k] == b'\\' && k + 1 < b.len() {
            cls[k] = Syn::Special;
            cls[k + 1] = Syn::Special;
            k += 2;
            continue;
        }
        cls[k] = Syn::Str;
        k += 1;
        if b[k - 1] == q {
            break;
        }
    }
    k
}

/// The end of a javaScriptRegexpString starting with the `/` at `j`, if it ends on this line.
fn regex_end(b: &[u8], j: usize) -> Option<usize> {
    let mut k = j + 1;
    while k < b.len() {
        if b[k] == b'\\' && matches!(b.get(k + 1), Some(b'\\' | b'/')) {
            k += 2;
            continue;
        }
        if b[k] == b'/' {
            // `/[gimuys]\{0,2}` then `\s*$`, `\s*[+;.,)\]}]` or `\s\+\/`.
            let mut flags = 0;
            while flags < 2 && b.get(k + 1 + flags).is_some_and(|c| b"gimuys".contains(c)) {
                flags += 1;
            }
            for f in (0..=flags).rev() {
                let after = k + 1 + f;
                let mut m = after;
                while m < b.len() && is_blank(b[m]) {
                    m += 1;
                }
                if m == b.len() {
                    return Some(b.len());
                }
                if b"+;.,)]}".contains(&b[m]) || (m > after && b[m] == b'/') {
                    return Some(m);
                }
            }
        }
        k += 1;
    }
    None
}

/// The legacy JavaScript syntax's strings, comments and regexes (`runtime/syntax/javascript.vim`,
/// `syn sync fromstart`).
pub(super) fn scan(lines: &[String]) -> Highlight {
    let mut stack = vec![Region::Code];
    let mut bytes = Vec::with_capacity(lines.len());
    let mut starts = Vec::with_capacity(lines.len());
    for line in lines {
        let b = line.as_bytes();
        let top = *stack.last().expect("code at the bottom");
        starts.push(match top {
            Region::Comment => Syn::Comment,
            Region::Tpl => Syn::Tpl,
            _ => Syn::None,
        });
        let mut cls = vec![Syn::None; b.len()];
        let mut i = 0;
        // javaScriptCommentSkip: `^[ \t]*\*\($\|[ \t]\+\)` outside other items.
        if top == Region::Code {
            let ws = b.iter().take_while(|&&c| is_blank(c)).count();
            if b.get(ws) == Some(&b'*') {
                let rest = b[ws + 1..].iter().take_while(|&&c| is_blank(c)).count();
                if ws + 1 == b.len() || rest > 0 {
                    let end = ws + 1 + rest;
                    cls[..end].fill(Syn::Comment);
                    i = end;
                }
            }
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
                        stack.push(Region::Embed);
                    } else {
                        cls[i] = Syn::Tpl;
                        i += 1;
                    }
                }
                // javaScriptEmbed: strings and templates, up to the first `}`.
                Region::Embed => match b[i] {
                    b'}' => {
                        i += 1;
                        stack.pop();
                    }
                    b'"' | b'\'' => i = string(b, i, &mut cls),
                    b'`' => {
                        cls[i] = Syn::Tpl;
                        i += 1;
                        stack.push(Region::Tpl);
                    }
                    _ => i += 1,
                },
                Region::Code => {
                    if b[i..].starts_with(b"/*") {
                        cls[i] = Syn::Comment;
                        cls[i + 1] = Syn::Comment;
                        i += 2;
                        stack.push(Region::Comment);
                    } else if b[i..].starts_with(b"//") {
                        cls[i..].fill(Syn::LineComment);
                        i = b.len();
                    } else if b[i] == b'"' || b[i] == b'\'' {
                        i = string(b, i, &mut cls);
                    } else if b[i] == b'`' {
                        cls[i] = Syn::Tpl;
                        i += 1;
                        stack.push(Region::Tpl);
                    } else if b",=+".contains(&b[i]) {
                        // javaScriptRegexpString starts after `,`, `=` or `+` (after `(`,
                        // javaScriptParens, defined later, wins).
                        let mut j = i + 1;
                        while j < b.len() && is_blank(b[j]) {
                            j += 1;
                        }
                        let starts_regex = b.get(j) == Some(&b'/')
                            && b.get(j + 1).is_some_and(|c| *c != b'/' && *c != b'*');
                        match starts_regex.then(|| regex_end(b, j)).flatten() {
                            Some(end) => {
                                cls[j..end].fill(Syn::Regex);
                                i = end;
                            }
                            None => i += 1,
                        }
                    } else {
                        i += 1;
                    }
                }
            }
        }
        bytes.push(cls);
    }
    Highlight { bytes, starts }
}

/// A `throw 'out of bounds'` from `s:SkipFunc()`.
struct OutOfBounds;

type R<T> = Result<T, OutOfBounds>;

#[derive(Clone, Copy)]
enum Skip {
    /// `s:skip_expr`: in a string, comment, regex, ….
    Expr,
    /// `s:in_comm`: in a comment.
    InComm,
    /// `s:SkipFunc()`: `s:skip_expr`, checked only where it might matter.
    Func,
}

/// What a searchpair() pattern matches at a position.
type Pat = fn(&Js, usize, usize) -> bool;

fn never(_: &Js, _: usize, _: usize) -> bool {
    false
}

fn bracket_pat(b: u8) -> Pat {
    match b {
        b'[' => |js, l, c| js.byte(l, c) == Some(b'['),
        b']' => |js, l, c| js.byte(l, c) == Some(b']'),
        b'(' => |js, l, c| js.byte(l, c) == Some(b'('),
        b')' => |js, l, c| js.byte(l, c) == Some(b')'),
        b'{' => |js, l, c| js.byte(l, c) == Some(b'{'),
        _ => |js, l, c| js.byte(l, c) == Some(b'}'),
    }
}

fn closing(open: u8) -> u8 {
    match open {
        b'[' => b']',
        b'(' => b')',
        _ => b'}',
    }
}

/// Whether `b[c..]` starts with the word `w` (followed by a non-keyword character).
fn word_at(b: &[u8], c: usize, w: &str) -> bool {
    b.get(c..).is_some_and(|r| r.starts_with(w.as_bytes()))
        && !b.get(c + w.len()).is_some_and(|&x| is_kw(x))
}

struct Js<'a> {
    ctx: &'a Ctx<'a>,
    lines: Vec<String>,
    syn: Highlight,
    /// `s:synid_cache`.
    synid_cache: HashMap<(usize, usize), Syn>,
    /// The cursor: line and byte, 0-based.
    cur: (usize, usize),
    /// `s:l1` (1-based): searches stop above this line.
    l1: usize,
    /// `s:looksyn` (0-based), `s:top_col` (1-based, 0 for none) and `s:check_in`.
    looksyn: usize,
    top_col: usize,
    check_in: bool,
}

impl Js<'_> {
    fn byte(&self, l: usize, c: usize) -> Option<u8> {
        self.lines.get(l).and_then(|s| s.as_bytes().get(c)).copied()
    }

    /// `getline('.')[col('.') - 1 + offset]`, nothing when out of range.
    fn rel(&self, offset: isize) -> Option<u8> {
        let (l, c) = self.cur;
        let i = c as isize + offset;
        if i < 0 {
            None
        } else {
            self.byte(l, i as usize)
        }
    }

    fn line(&self, l: usize) -> &[u8] {
        self.lines.get(l).map_or(&[], |s| s.as_bytes())
    }

    fn indent(&self, l: usize) -> usize {
        self.ctx.indent(l)
    }

    fn sw(&self) -> usize {
        self.ctx.sw()
    }

    /// `s:SynAt(l, c)` (0-based).
    fn syn_at(&mut self, l: usize, c: usize) -> Syn {
        let syn = &self.syn;
        *self
            .synid_cache
            .entry((l, c))
            .or_insert_with(|| syn.at(l, c))
    }

    fn is_comment(s: Syn) -> bool {
        matches!(s, Syn::Comment | Syn::LineComment)
    }

    /// `s:LookingAt()`.
    fn looking_at(&self) -> Option<u8> {
        self.byte(self.cur.0, self.cur.1)
    }

    /// `s:Token()`: the word under the cursor, or the character.
    fn token(&self) -> String {
        let (l, c) = self.cur;
        let b = self.line(l);
        match b.get(c) {
            Some(&x) if is_kw(x) => {
                let mut s = c;
                while s > 0 && is_kw(b[s - 1]) {
                    s -= 1;
                }
                let mut e = c;
                while e < b.len() && is_kw(b[e]) {
                    e += 1;
                }
                String::from_utf8_lossy(&b[s..e]).into_owned()
            }
            Some(&x) => char::from(x).to_string(),
            None => String::new(),
        }
    }

    /// `search('\m\k\{1,}\|\S','ebW')`: to the end of the token before the cursor.
    fn search_prev_token_end(&mut self) -> bool {
        let (l, c) = self.cur;
        let b = self.line(l);
        let mut p = c.min(b.len());
        while p > 0 {
            p -= 1;
            if is_blank(b[p]) {
                continue;
            }
            let end = if is_kw(b[p]) {
                let mut e = p;
                while e + 1 < b.len() && is_kw(b[e + 1]) {
                    e += 1;
                }
                e
            } else {
                p
            };
            if end < c {
                self.cur = (l, end);
                return true;
            }
        }
        for pl in (0..l).rev() {
            if let Some(p) = self.line(pl).iter().rposition(|&x| !is_blank(x)) {
                self.cur = (pl, p);
                return true;
            }
        }
        false
    }

    /// `s:PreviousToken()`.
    fn previous_token(&mut self, recursed: bool) -> String {
        let pos = self.cur;
        let mut tok = String::new();
        if self.search_prev_token_end() {
            let (l, c) = self.cur;
            if c >= 1 && self.byte(l, c - 1) == Some(b'*') && self.byte(l, c) == Some(b'/') {
                let in_comm = Self::is_comment(self.syn_at(l, c));
                if in_comm && !self.search_code_before_comment() {
                    self.cur = pos;
                } else {
                    tok = self.token();
                }
            } else {
                let two = if recursed || l != pos.0 {
                    // `strridx(getline('.')[:col('.')], '//') + 1`
                    let b = self.line(l);
                    let upto = (c + 2).min(b.len());
                    b[..upto]
                        .windows(2)
                        .rposition(|w| w == b"//")
                        .map_or(0, |i| i + 1)
                } else {
                    0
                };
                if two > 0 && Self::is_comment(self.syn_at(l, c)) {
                    self.cur = (l, two - 1);
                    tok = self.previous_token(true);
                    if tok.is_empty() {
                        self.cur = pos;
                    }
                } else {
                    tok = self.token();
                }
            }
        }
        tok
    }

    /// `s:SearchLoop('\S\ze\_s*\/[/*]','bW',s:in_comm)`: back to the code before a comment.
    fn search_code_before_comment(&mut self) -> bool {
        fn before_comment(js: &Js, l: usize, c: usize) -> bool {
            if js.byte(l, c).is_none_or(is_blank) {
                return false;
            }
            let (mut l, mut c) = (l, c + 1);
            loop {
                let b = js.line(l);
                while c < b.len() && is_blank(b[c]) {
                    c += 1;
                }
                if c < b.len() {
                    return b[c] == b'/' && matches!(b.get(c + 1), Some(b'/' | b'*'));
                }
                if l + 1 >= js.lines.len() {
                    return false;
                }
                l += 1;
                c = 0;
            }
        }
        matches!(
            self.searchpair(before_comment, never, Skip::InComm),
            Ok(true)
        )
    }

    /// `s:Pure(f)`: run `f`, then put the cursor back.
    fn pure<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let pos = self.cur;
        let r = f(self);
        self.cur = pos;
        r
    }

    /// The `{skip}` of a searchpair(), evaluated at the cursor.
    fn skip(&mut self, skip: Skip) -> R<bool> {
        let (l, c) = self.cur;
        Ok(match skip {
            Skip::Expr => self.syn_at(l, c) != Syn::None,
            Skip::InComm => Self::is_comment(self.syn_at(l, c)),
            Skip::Func => self.skip_func()?,
        })
    }

    /// `s:SkipFunc()`: skip a match in a string or comment, checking the syntax only when
    /// something on the way looks like it might be one. A match in column 1 is taken to be
    /// outside everything: the search ends there.
    fn skip_func(&mut self) -> R<bool> {
        let (l, c) = self.cur;
        if self.top_col == 1 {
            return Err(OutOfBounds);
        } else if self.check_in {
            if self.skip(Skip::Expr)? {
                return Ok(true);
            }
            self.check_in = false;
        } else if self.suspicious_line(l, c) {
            if self.skip(Skip::Expr)? {
                return Ok(true);
            }
        } else if self.template_or_comment_end_ahead() {
            if self.skip(Skip::Expr)? {
                self.check_in = true;
                return Ok(true);
            }
        } else {
            self.synid_cache.insert((l, c), Syn::None);
        }
        self.looksyn = l;
        self.top_col = c + 1;
        Ok(false)
    }

    /// `getline('.') =~ '\%<'.col('.').'c\/.\{-}\/\|\%>'.col('.').'c[''"]\|\\$'`.
    fn suspicious_line(&self, l: usize, c: usize) -> bool {
        let b = self.line(l);
        let slashes = b
            .iter()
            .position(|&x| x == b'/')
            .is_some_and(|i| i < c && b[i + 1..].contains(&b'/'));
        let quote_after = b.iter().skip(c + 1).any(|&x| x == b'\'' || x == b'"');
        slashes || quote_after || b.last() == Some(&b'\\')
    }

    /// `search('\m`\|\${\|\*\/','nW',s:looksyn)`: a backtick, `${` or `*/` after the cursor.
    fn template_or_comment_end_ahead(&self) -> bool {
        let (l, c) = self.cur;
        for pl in l..=self.looksyn.min(self.lines.len().saturating_sub(1)) {
            let b = self.line(pl);
            let from = if pl == l { c + 1 } else { 0 };
            for i in from..b.len() {
                if b[i] == b'`' || b[i..].starts_with(b"${") || b[i..].starts_with(b"*/") {
                    return true;
                }
            }
        }
        false
    }

    /// Vim's `searchpair(start, '', end, 'bW', skip, s:l1)`: back to the unmatched `start`,
    /// moving the cursor there.
    fn searchpair(&mut self, start: Pat, end: Pat, skip: Skip) -> R<bool> {
        let mut nest = 1;
        let (mut l, mut c) = self.cur;
        loop {
            // The previous position where either pattern matches.
            let found = loop {
                if c == 0 {
                    if l == 0 || l < self.l1 {
                        break None;
                    }
                    l -= 1;
                    c = self.line(l).len();
                    continue;
                }
                c -= 1;
                if start(self, l, c) || end(self, l, c) {
                    break Some((l, c));
                }
            };
            let Some(pos) = found else {
                return Ok(false);
            };
            let is_start = start(self, pos.0, pos.1);
            let saved = self.cur;
            self.cur = pos;
            let skipped = self.skip(skip);
            self.cur = saved;
            if skipped? {
                continue;
            }
            if is_start {
                nest -= 1;
            } else {
                nest += 1;
            }
            if nest == 0 {
                self.cur = pos;
                return Ok(true);
            }
        }
    }

    /// `s:GetPair(open, close, 'bW', skip)` for a bracket pair.
    fn get_pair(&mut self, open: u8, skip: Skip) -> R<bool> {
        self.searchpair(bracket_pat(open), bracket_pat(closing(open)), skip)
    }

    /// `s:AlternatePair()`: back to what contains the cursor, past whole statements.
    fn alternate_pair(&mut self) -> R<()> {
        let mut semis = 2;
        let mut pat: Pat = |js, l, c| js.byte(l, c).is_some_and(|b| b"[](){};".contains(&b));
        while self.searchpair(pat, never, Skip::Func)? {
            if self.looking_at() == Some(b';') {
                if semis == 0 {
                    if self.get_pair(b'{', Skip::Func)? {
                        return Ok(());
                    }
                    break;
                }
                pat = |js, l, c| js.byte(l, c).is_some_and(|b| b"{}();".contains(&b));
                semis -= 1;
            } else {
                let open = match self.looking_at() {
                    Some(b']') => b'[',
                    Some(b')') => b'(',
                    Some(b'}') => b'{',
                    _ => return Ok(()),
                };
                if !self.get_pair(open, Skip::Func)? {
                    break;
                }
            }
        }
        Err(OutOfBounds)
    }

    /// `s:ExprCol()`: whether the `:` at the cursor is part of an expression (a ternary or an
    /// object key) rather than a label.
    fn expr_col(&mut self) -> bool {
        if self.rel(-1) == Some(b':') {
            return true;
        }
        let mut bal = 0;
        let pat: Pat = |js, l, c| js.byte(l, c).is_some_and(|b| b"{}?:".contains(&b));
        while matches!(self.searchpair(pat, never, Skip::Expr), Ok(true)) {
            match self.looking_at() {
                Some(b':') => {
                    if self.rel(-1) == Some(b':') {
                        self.cur.1 -= 1;
                        continue;
                    }
                    bal -= 1;
                }
                Some(b'?') => {
                    // `?.` (but not `?.5`) is optional chaining.
                    if self.rel(1) == Some(b'.') && !self.rel(2).is_some_and(|b| b.is_ascii_digit())
                    {
                        continue;
                    } else if bal == 0 {
                        return true;
                    }
                    bal += 1;
                }
                Some(b'{') => return !self.is_block(),
                _ => {
                    if !matches!(self.get_pair(b'{', Skip::Expr), Ok(true)) {
                        break;
                    }
                }
            }
        }
        false
    }

    /// `s:Continues()`: whether the line ending at the cursor continues on the next.
    fn continues(&mut self) -> bool {
        let (l, c) = self.cur;
        let b = self.line(l);
        let end = (c + 1).min(b.len());
        let w = &b[end.saturating_sub(15)..end];
        match continuation_token(w).as_deref() {
            None => false,
            Some(":") => self.expr_col(),
            Some(t) if t.bytes().any(|x| x.is_ascii_lowercase()) => {
                self.previous_token(false) != "."
            }
            Some("/") => self.syn_at(l, c) != Syn::Regex,
            Some(_) => true,
        }
    }

    /// `s:OneScope()`: whether what ends at the cursor takes a statement without braces
    /// (`if (x)`, `else`, `=>`).
    fn one_scope(&mut self) -> bool {
        if self.looking_at() == Some(b')') && matches!(self.get_pair(b'(', Skip::Expr), Ok(true)) {
            let tok = self.previous_token(false);
            let head = ["for", "if", "let", "while", "with"].contains(&tok.as_str())
                || ((tok == "await" || tok == "each") && self.previous_token(false) == "for");
            return head
                && self.pure(|js| js.previous_token(false)) != "."
                && !(tok == "while" && self.do_while());
        }
        let tok = self.token();
        if tok == "else" || tok == "do" {
            return self.pure(|js| js.previous_token(false)) != ".";
        }
        if self.rel(-1) == Some(b'=') && self.looking_at() == Some(b'>') {
            self.cur.1 -= 1;
            if self.previous_token(false) == ")" {
                return matches!(self.get_pair(b'(', Skip::Expr), Ok(true));
            }
            return true;
        }
        false
    }

    /// `s:DoWhile()`: whether the `while` at the cursor ends a `do` loop.
    fn do_while(&mut self) -> bool {
        // searchpos('\m\<','cbW'): back to the start of the word.
        let word_start = |b: &[u8], i: usize| is_kw(b[i]) && (i == 0 || !is_kw(b[i - 1]));
        let (l, c) = self.cur;
        let mut cpos = None;
        for pl in (0..=l).rev() {
            let b = self.line(pl);
            let upto = if pl == l {
                (c + 1).min(b.len())
            } else {
                b.len()
            };
            if let Some(i) = (0..upto).rev().find(|&i| word_start(b, i)) {
                cpos = Some((pl, i));
                break;
            }
        }
        let Some(cpos) = cpos else {
            return false;
        };
        self.cur = cpos;
        let pat: Pat = |js, l, c| {
            let b = js.line(l);
            match b.get(c) {
                Some(b'{' | b'}') => true,
                _ => {
                    (c == 0 || !is_kw(b[c - 1])) && (word_at(b, c, "do") || word_at(b, c, "while"))
                }
            }
        };
        while matches!(self.searchpair(pat, never, Skip::Expr), Ok(true)) {
            if self.looking_at().is_some_and(|b| b.is_ascii_alphabetic()) {
                if self.pure(|js| js.is_block()) {
                    if self.looking_at() == Some(b'd') {
                        return true;
                    }
                    break;
                }
            } else if self.looking_at() != Some(b'}')
                || !matches!(self.get_pair(b'{', Skip::Expr), Ok(true))
            {
                break;
            }
        }
        self.cur = cpos;
        false
    }

    /// `s:IsContOne(cont)`: how many statements without braces the line is inside.
    fn is_cont_one(&mut self, num: Option<usize>, cont: bool) -> usize {
        let firstline = self.cur.0;
        // `b:js_cache[1] + !b:js_cache[1]`: line 1 when there is no containing bracket.
        let num_l = num.unwrap_or(0);
        let pind = num.map_or(0, |n| self.indent(n) + self.sw());
        let mut ind = self.indent(self.cur.0) + usize::from(!cont);
        let mut b_l = 0;
        while (self.cur.0 > num_l && ind > pind) || self.cur.0 == num_l {
            if self.indent(self.cur.0) < ind && self.one_scope() {
                b_l += 1;
            } else if !cont || b_l > 0 || ind < self.indent(firstline) {
                break;
            } else {
                self.cur.1 = 0;
            }
            ind = ind.min(self.indent(self.cur.0));
            if self.previous_token(false).is_empty() {
                break;
            }
        }
        b_l
    }

    /// `s:IsSwitch()`: whether the `{` at `pos` is followed by `case` or `default`.
    fn is_switch(&mut self, pos: (usize, usize)) -> bool {
        self.cur = pos;
        let (mut l, mut c) = (pos.0, pos.1 + 1);
        loop {
            let b = self.line(l);
            while c < b.len() && is_blank(b[c]) {
                c += 1;
            }
            if c >= b.len() || b[c..].starts_with(b"//") {
                if l + 1 >= self.lines.len() {
                    return false;
                }
                l += 1;
                c = 0;
                continue;
            }
            if b[c..].starts_with(b"/*") {
                // To the end of the comment.
                let (mut el, mut ec) = (l, c + 2);
                loop {
                    let eb = self.line(el);
                    let from = ec.min(eb.len());
                    if let Some(p) = eb[from..].windows(2).position(|w| w == b"*/") {
                        l = el;
                        c = from + p + 2;
                        break;
                    }
                    if el + 1 >= self.lines.len() {
                        return false;
                    }
                    el += 1;
                    ec = 0;
                }
                continue;
            }
            return word_at(b, c, "case") || word_at(b, c, "default");
        }
    }

    /// `s:IsBlock()`: whether the `{` at the cursor starts a block (not an object literal).
    fn is_block(&mut self) -> bool {
        let firstline = self.cur.0;
        let tok = self.previous_token(false);
        if tok.bytes().next().is_some_and(is_kw) {
            if tok == "type" {
                return self.pure(|js| {
                    let t = js.previous_token(false);
                    !(t == "import" || t == "export") || js.previous_token(false) == "."
                });
            } else if tok == "of" {
                return self.pure(|js| {
                    let open: Pat = |js, l, c| js.byte(l, c).is_some_and(|b| b"[({".contains(&b));
                    let close: Pat = |js, l, c| js.byte(l, c).is_some_and(|b| b"])}".contains(&b));
                    if !matches!(js.searchpair(open, close, Skip::Expr), Ok(true))
                        || js.looking_at() != Some(b'(')
                    {
                        return true;
                    }
                    let t = js.previous_token(false);
                    let t = if t == "await" {
                        js.previous_token(false)
                    } else {
                        js.token()
                    };
                    t != "for" || js.previous_token(false) == "."
                });
            }
            const WORDS: [&str; 18] = [
                "return",
                "const",
                "let",
                "import",
                "export",
                "extends",
                "yield",
                "default",
                "delete",
                "var",
                "await",
                "void",
                "typeof",
                "throw",
                "case",
                "new",
                "in",
                "instanceof",
            ];
            let index = WORDS
                .iter()
                .position(|w| *w == tok)
                .map_or(-1, |i| i as isize);
            let other_line = isize::from(self.cur.0 != firstline);
            return index < other_line || self.pure(|js| js.previous_token(false)) == ".";
        }
        match tok.as_str() {
            "" => false,
            ">" => self.rel(-1) == Some(b'='),
            "*" => self.pure(|js| js.previous_token(false)) == ":",
            ":" => self.pure(|js| {
                let t = js.previous_token(false);
                let ident = t
                    .bytes()
                    .next()
                    .is_some_and(|b| is_kw(b) && !b.is_ascii_digit())
                    && t.bytes().all(is_kw);
                ident && !js.expr_col()
            }),
            "/" => {
                let (l, c) = self.cur;
                self.syn_at(l, c) == Syn::Regex
            }
            t if !t.bytes().any(|b| b"=~!<,.?^%|&([".contains(&b)) => {
                !(t == "-" || t == "+")
                    || (self.cur.0 != firstline && self.rel(-1) == t.bytes().next())
            }
            _ => false,
        }
    }
}

/// The match of `s:continuation` (anchored at the end) in `w`, or `None`.
fn continuation_token(w: &[u8]) -> Option<String> {
    let n = w.len();
    let last = *w.last()?;
    for word in [
        "typeof",
        "new",
        "delete",
        "void",
        "in",
        "instanceof",
        "await",
    ] {
        let k = word.len();
        if w.ends_with(word.as_bytes()) && (n == k || !is_kw(w[n - k - 1])) {
            return Some(word.to_string());
        }
    }
    let before = (n >= 2).then(|| w[n - 2]);
    let ok = match last {
        b'<' | b'=' | b',' | b'.' | b'~' | b'!' | b'?' | b'/' | b'*' | b'^' | b'%' | b'|'
        | b'&' | b':' => true,
        b'+' => before != Some(b'+'),
        b'-' => before != Some(b'-'),
        b'>' => before != Some(b'='),
        _ => false,
    };
    ok.then(|| char::from(last).to_string())
}

/// `matchstr(line, s:opfirst)`: the operator the line starts with.
fn opfirst(line: &str) -> Option<String> {
    let b = line.as_bytes();
    let first = *b.first()?;
    if b"<>=,.?^%|/&".contains(&first) {
        return Some(char::from(first).to_string());
    }
    if b"-:+".contains(&first) {
        return (b.get(1) != Some(&first)).then(|| char::from(first).to_string());
    }
    if first == b'*' {
        return Some("*".repeat(b.iter().take_while(|&&x| x == b'*').count()));
    }
    if b.starts_with(b"!=") {
        return Some("!=".into());
    }
    ["instanceof", "in"]
        .into_iter()
        .find(|w| word_at(b, 0, w))
        .map(str::to_owned)
}

/// The indent for `ctx.lnum`, or `None` to keep it.
pub(super) fn indent(ctx: &Ctx) -> Option<usize> {
    let lines: Vec<String> = (0..ctx.line_count())
        .map(|l| ctx.line(l).into_owned())
        .collect();
    let syn = scan(&lines[..(ctx.lnum + 1).min(lines.len())]);
    let mut js = Js {
        ctx,
        lines,
        syn,
        synid_cache: HashMap::new(),
        cur: (ctx.lnum, 0),
        l1: 1,
        looksyn: 0,
        top_col: 0,
        check_in: false,
    };
    js.get_indent()
}

impl Js<'_> {
    fn get_indent(&mut self) -> Option<usize> {
        let ctx = self.ctx;
        let v = ctx.lnum;
        let raw = self.lines.get(v).cloned().unwrap_or_default();
        // synstack(v:lnum, 1): what the line starts in.
        let stack = if raw.is_empty() {
            self.syn.starts.get(v).copied().unwrap_or(Syn::None)
        } else {
            self.syn.at(v, 0)
        };
        let trimmed = raw.trim_start_matches([' ', '\t']);
        if Self::is_comment(stack) {
            if trimmed.starts_with('*') {
                return Some(cindent::get_c_indent(ctx));
            } else if !(trimmed.starts_with("//") || trimmed.starts_with("/*")) {
                return None;
            }
        } else if stack != Syn::None {
            return None;
        }

        // `s:l1`: searches look back up to 2000 lines.
        let prev = ctx.prevnonblank(v).map_or(0, |l| l + 1);
        self.l1 = prev.saturating_sub(2000).max(1);
        self.cur = (v, 0);
        if self.previous_token(false).is_empty() {
            return Some(0);
        }
        let l_lnum = self.cur.0;
        let pline_len = self.cur.1 + 1;
        let pline_last = self.line(l_lnum)[self.cur.1];

        let mut line = trimmed.to_string();
        if line.starts_with("/*") {
            // Leading complete `/* */` comments go.
            while line.starts_with("/*") {
                let Some(end) = line[2..].find("*/") else {
                    break;
                };
                line = line[end + 4..].trim_start_matches([' ', '\t']).to_string();
            }
        }
        if line.starts_with("//") || line.starts_with("/*") {
            line.clear();
        }

        // The bracket the line is in.
        self.cur = (v, 0);
        let idx = match line.bytes().next() {
            Some(b']') => Some(b'['),
            Some(b')') => Some(b'('),
            Some(b'}') => Some(b'{'),
            _ => None,
        };
        self.looksyn = v.saturating_sub(1);
        self.top_col = 0;
        self.check_in = false;
        let found = match idx {
            Some(open) => self.get_pair(open, Skip::Func).map(|_| ()),
            None => self.alternate_pair(),
        };
        if found.is_err() {
            self.cur = (v, 0);
        }
        let container = self.cur;
        let mut num = (self.cur.0 != v).then_some(self.cur.0);

        let mut num_ind = num.map_or(0, |n| self.indent(n));
        let mut is_op = 0;
        let mut b_l = 0;
        let mut switch_offset = 0;
        if num.is_none() || (self.looking_at() == Some(b'{') && self.is_block()) {
            let ilnum = self.cur.0;
            if num.is_some()
                && self.looking_at() == Some(b')')
                && matches!(self.get_pair(b'(', Skip::Expr), Ok(true))
            {
                if Some(ilnum) == num {
                    num = Some(self.cur.0);
                    num_ind = self.indent(self.cur.0);
                }
                if idx.is_none()
                    && self.previous_token(false) == "switch"
                    && self.is_switch(container)
                {
                    switch_offset = self.sw();
                    let case = word_at(line.as_bytes(), 0, "case")
                        || word_at(line.as_bytes(), 0, "default");
                    if pline_last != b'.' && case {
                        return Some(num_ind + switch_offset);
                    }
                }
            }
            if idx.is_none() && pline_last != b'{' && pline_last != b';' {
                self.cur = (l_lnum, pline_len - 1);
                let sol = opfirst(&line);
                let regex_first = sol.as_deref() == Some("/") && {
                    let col = raw.len() - line.len();
                    self.syn_at(v, col) == Syn::Regex
                };
                if sol.is_none() || regex_first {
                    if self.continues() {
                        is_op = self.sw();
                    }
                } else if num.is_some()
                    && sol
                        .as_deref()
                        .is_some_and(|s| s == "in" || s == "instanceof" || s.starts_with('*'))
                    && self.looking_at() == Some(b'}')
                    && matches!(self.get_pair(b'{', Skip::Expr), Ok(true))
                    && self.previous_token(false) == ")"
                    && matches!(self.get_pair(b'(', Skip::Expr), Ok(true))
                    && {
                        let t = self.previous_token(false);
                        t == "]"
                            || (self.looking_at().is_some_and(is_kw) && {
                                let t2 = if t == "*" {
                                    self.previous_token(false)
                                } else {
                                    self.token()
                                };
                                t2 != "function"
                            })
                    }
                {
                    return Some(num_ind + self.sw());
                } else {
                    is_op = self.sw();
                }
                self.cur = (l_lnum, pline_len - 1);
                let cont = self.is_cont_one(num, is_op > 0) as isize
                    - isize::from(is_op == 0 && line.starts_with('{'));
                b_l = cont.max(0) as usize * self.sw();
            }
        }

        if line.starts_with([']', ')', '}']) || line.starts_with("|}") {
            return Some(num_ind);
        }
        if num.is_some() {
            return Some(num_ind + self.sw() + switch_offset + b_l + is_op);
        }
        Some(b_l + is_op)
    }
}
