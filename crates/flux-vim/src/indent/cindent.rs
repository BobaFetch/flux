//! Vim's C indenting (`get_c_indent`), ported from Vim's `indent_c.c` as Neovim 0.12 has it,
//! with the parts of `findmatchlimit` (`search.c`) it uses. It indents C, does most of the work
//! for Rust's indent script, and is what `=` uses when there's no 'indentexpr'.
//!
//! The port keeps Vim's shape: 1-based line numbers, byte columns, and a "cursor" that the
//! searches start from and that the code moves around, so it can be checked against the C
//! function by function. Like Vim, it finds comments and strings by scanning the text, not
//! from the syntax.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use unicode_width::UnicodeWidthChar;

use super::Ctx;

/// Vim's `MAXCOL` for amounts.
const MAXCOL: i32 = i32::MAX;
/// `check_linecomment`'s "no comment".
const NOCOL: usize = usize::MAX;

const FM_BACKWARD: u32 = 1;
const FM_FORWARD: u32 = 2;
const FM_BLOCKSTOP: u32 = 4;

const LOOKFOR_INITIAL: i32 = 0;
const LOOKFOR_IF: i32 = 1;
const LOOKFOR_DO: i32 = 2;
const LOOKFOR_CASE: i32 = 3;
const LOOKFOR_ANY: i32 = 4;
const LOOKFOR_TERM: i32 = 5;
const LOOKFOR_UNTERM: i32 = 6;
const LOOKFOR_SCOPEDECL: i32 = 7;
const LOOKFOR_NOBREAK: i32 = 8;
const LOOKFOR_CPP_BASECLASS: i32 = 9;
const LOOKFOR_ENUM_OR_INIT: i32 = 10;
const LOOKFOR_JS_KEY: i32 = 11;
const LOOKFOR_COMMA: i32 = 12;

const BRACE_IN_COL0: i32 = 1;
const BRACE_AT_START: i32 = 2;
const BRACE_AT_END: i32 = 3;

/// A position: 1-based line, byte column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pos {
    lnum: usize,
    col: usize,
}

fn lt(a: Pos, b: Pos) -> bool {
    (a.lnum, a.col) < (b.lnum, b.col)
}

/// The byte at `i`, or NUL past the end (C strings).
fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

fn is_white(c: u8) -> bool {
    c == b' ' || c == b'\t'
}

fn skipwhite(s: &[u8], mut i: usize) -> usize {
    while is_white(at(s, i)) {
        i += 1;
    }
    i
}

fn skiptowhite(s: &[u8], mut i: usize) -> usize {
    while at(s, i) != 0 && !is_white(at(s, i)) {
        i += 1;
    }
    i
}

/// Vim's `vim_isIDc` / `vim_iswordc` with the default 'isident' / 'iskeyword'.
fn is_idc(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 192
}

/// `strncmp(s + i, w, strlen(w)) == 0`.
fn starts(s: &[u8], i: usize, w: &[u8]) -> bool {
    s.get(i..).is_some_and(|r| r.starts_with(w))
}

/// `strncmp(a, b, n) == 0` with C string semantics (both NUL-terminated).
fn strneq(a: &[u8], b: &[u8], n: usize) -> bool {
    for k in 0..n {
        let (ca, cb) = (at(a, k), at(b, k));
        if ca != cb {
            return false;
        }
        if ca == 0 {
            return true;
        }
    }
    true
}

/// `vim_strchr(s + i, c)`.
fn strchr(s: &[u8], i: usize, c: u8) -> Option<usize> {
    s.get(i..)?.iter().position(|&b| b == c).map(|p| i + p)
}

/// Length of the UTF-8 character starting with byte `b`.
fn char_len(b: u8) -> usize {
    match b {
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xff => 4,
        _ => 1,
    }
}

/// The character at byte `i` (NUL past the end).
fn char_at(s: &[u8], i: usize) -> char {
    if i >= s.len() {
        return '\0';
    }
    let end = (i + char_len(s[i])).min(s.len());
    std::str::from_utf8(&s[i..end])
        .ok()
        .and_then(|t| t.chars().next())
        .unwrap_or(char::from(s[i]))
}

/// Vim's `skip_string`: skip a "string" or 'c' character (several, when concatenated).
fn skip_string(s: &[u8], mut p: usize) -> usize {
    loop {
        if at(s, p) == b'\'' {
            if at(s, p + 1) == 0 {
                break;
            }
            let mut i = 2;
            if at(s, p + 1) == b'\\' && at(s, p + 2) != 0 {
                i += 1;
                while at(s, p + i - 1).is_ascii_digit() {
                    i += 1;
                }
            }
            if at(s, p + i - 1) != 0 && at(s, p + i) == b'\'' {
                p += i + 1;
                continue;
            }
        } else if at(s, p) == b'"' {
            p += 1;
            while at(s, p) != 0 {
                if at(s, p) == b'\\' && at(s, p + 1) != 0 {
                    p += 1;
                } else if at(s, p) == b'"' {
                    break;
                }
                p += 1;
            }
            if at(s, p) == b'"' {
                p += 1;
                continue;
            }
        } else if at(s, p) == b'R' && at(s, p + 1) == b'"' {
            let delim = p + 2;
            if let Some(paren) = strchr(s, delim, b'(') {
                let dlen = paren - delim;
                p += 3;
                while at(s, p) != 0 {
                    if at(s, p) == b')'
                        && starts(s, p + 1, &s[delim..delim + dlen])
                        && at(s, p + dlen + 1) == b'"'
                    {
                        p += dlen + 1;
                        break;
                    }
                    p += 1;
                }
                if at(s, p) == b'"' {
                    p += 1;
                    continue;
                }
            }
        }
        break;
    }
    if at(s, p) == 0 {
        p = p.saturating_sub(1);
    }
    p
}

/// Vim's `is_pos_in_string`: `line[col]` is inside a C string.
fn is_pos_in_string(line: &[u8], col: usize) -> bool {
    let mut p = 0;
    while at(line, p) != 0 && p < col {
        p = skip_string(line, p);
        p += 1;
    }
    p > col
}

/// Vim's `check_linecomment`: the column of a `//` comment, or `NOCOL`.
fn check_linecomment(line: &[u8]) -> usize {
    let mut p = 0;
    while let Some(i) = strchr(line, p, b'/') {
        if at(line, i + 1) == b'/'
            && (i == 0 || line[i - 1] != b'*' || at(line, i + 2) != b'*')
            && !is_pos_in_string(line, i)
        {
            return i;
        }
        p = i + 1;
    }
    NOCOL
}

fn cin_iscomment(s: &[u8], i: usize) -> bool {
    at(s, i) == b'/' && (at(s, i + 1) == b'*' || at(s, i + 1) == b'/')
}

fn cin_islinecomment(s: &[u8], i: usize) -> bool {
    at(s, i) == b'/' && at(s, i + 1) == b'/'
}

fn cin_ispreproc(s: &[u8], i: usize) -> bool {
    at(s, skipwhite(s, i)) == b'#'
}

fn cin_starts_with(s: &[u8], i: usize, word: &[u8]) -> bool {
    starts(s, i, word) && !is_idc(at(s, i + word.len()))
}

fn cin_isif(s: &[u8], i: usize) -> bool {
    starts(s, i, b"if") && !is_idc(at(s, i + 2))
}

fn cin_isdo(s: &[u8], i: usize) -> bool {
    starts(s, i, b"do") && !is_idc(at(s, i + 2))
}

fn cin_isbreak(s: &[u8], i: usize) -> bool {
    starts(s, i, b"break") && !is_idc(at(s, i + 5))
}

/// Ends in a backslash (a continued line).
fn ends_in_backslash(s: &[u8]) -> bool {
    s.last() == Some(&b'\\')
}

/// Vim's `cin_is_if_for_while_before_offset`.
fn cin_is_if_for_while_before_offset(line: &[u8], poffset: &mut usize) -> bool {
    let mut offset = *poffset as isize;
    if offset < 2 {
        return false;
    }
    offset -= 1;
    while offset > 2 && is_white(at(line, offset as usize)) {
        offset -= 1;
    }
    offset -= 1;
    let found = |o: isize, w: &[u8]| o >= 0 && starts(line, o as usize, w);
    let mut ok = found(offset, b"if");
    if !ok && offset >= 1 {
        offset -= 1;
        ok = found(offset, b"for");
        if !ok && offset >= 2 {
            offset -= 2;
            ok = found(offset, b"while");
        }
    }
    if !ok {
        return false;
    }
    if offset == 0 || !is_idc(at(line, offset as usize - 1)) {
        *poffset = offset as usize;
        return true;
    }
    false
}

/// A pointer into a line: the line and a byte offset.
#[derive(Clone)]
struct Lp {
    line: Rc<[u8]>,
    off: usize,
}

impl Lp {
    fn s(&self) -> &[u8] {
        &self.line
    }
    fn c(&self) -> u8 {
        at(&self.line, self.off)
    }
}

/// 'cinoptions', parsed (Vim's `b_ind_*`).
#[derive(Debug, Clone)]
struct Cino {
    level: i32,
    open_imag: i32,
    no_brace: i32,
    first_open: i32,
    open_extra: i32,
    close_extra: i32,
    open_left_imag: i32,
    jump_label: i32,
    case: i32,
    case_code: i32,
    case_break: i32,
    scopedecl: i32,
    scopedecl_code: i32,
    param: i32,
    func_type: i32,
    cpp_baseclass: i32,
    continuation: i32,
    unclosed: i32,
    unclosed2: i32,
    unclosed_noignore: i32,
    unclosed_wrapped: i32,
    unclosed_whiteok: i32,
    matching_paren: i32,
    paren_prev: i32,
    comment: i32,
    in_comment: i32,
    in_comment2: i32,
    maxparen: i32,
    maxcomment: i32,
    java: i32,
    js: i32,
    keep_case_label: i32,
    cpp_namespace: i32,
    if_for_while: i32,
    hash_comment: i32,
    cpp_extern_c: i32,
    pragma: i32,
}

/// Vim's `parse_cino`.
fn parse_cino(cino: &str, sw: i32) -> Cino {
    let mut c = Cino {
        level: sw,
        open_imag: 0,
        no_brace: 0,
        first_open: 0,
        open_extra: 0,
        close_extra: 0,
        open_left_imag: 0,
        jump_label: -1,
        case: sw,
        case_code: sw,
        case_break: 0,
        scopedecl: sw,
        scopedecl_code: sw,
        param: sw,
        func_type: sw,
        cpp_baseclass: sw,
        continuation: sw,
        unclosed: sw * 2,
        unclosed2: sw,
        unclosed_noignore: 0,
        unclosed_wrapped: 0,
        unclosed_whiteok: 0,
        matching_paren: 0,
        paren_prev: 0,
        comment: 0,
        in_comment: 3,
        in_comment2: 0,
        maxparen: 20,
        maxcomment: 70,
        java: 0,
        js: 0,
        keep_case_label: 0,
        cpp_namespace: 0,
        if_for_while: 0,
        hash_comment: 0,
        cpp_extern_c: 0,
        pragma: 0,
    };
    let p = cino.as_bytes();
    let mut i = 0;
    let mut fraction: i64 = 0;
    while i < p.len() {
        let l = i;
        i += 1;
        if at(p, i) == b'-' {
            i += 1;
        }
        let digits_start = i;
        let mut n: i64 = 0;
        while at(p, i).is_ascii_digit() {
            n = n.saturating_mul(10).saturating_add(i64::from(p[i] - b'0'));
            i += 1;
        }
        let mut divider: i64 = 0;
        if at(p, i) == b'.' {
            i += 1;
            let mut f: i64 = 0;
            let mut k = i;
            while at(p, k).is_ascii_digit() {
                f = f * 10 + i64::from(p[k] - b'0');
                k += 1;
            }
            fraction = f;
            while at(p, i).is_ascii_digit() {
                i += 1;
                divider = if divider != 0 { divider * 10 } else { 10 };
            }
        }
        if at(p, i) == b's' {
            if i == digits_start {
                n = i64::from(sw);
            } else {
                n *= i64::from(sw);
                if divider != 0 {
                    n += (i64::from(sw) * fraction + divider / 2) / divider;
                }
            }
            i += 1;
        }
        if at(p, l + 1) == b'-' {
            n = -n;
        }
        let n = n.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        match p[l] {
            b'>' => c.level = n,
            b'e' => c.open_imag = n,
            b'n' => c.no_brace = n,
            b'f' => c.first_open = n,
            b'{' => c.open_extra = n,
            b'}' => c.close_extra = n,
            b'^' => c.open_left_imag = n,
            b'L' => c.jump_label = n,
            b':' => c.case = n,
            b'=' => c.case_code = n,
            b'b' => c.case_break = n,
            b'p' => c.param = n,
            b't' => c.func_type = n,
            b'/' => c.comment = n,
            b'c' => c.in_comment = n,
            b'C' => c.in_comment2 = n,
            b'i' => c.cpp_baseclass = n,
            b'+' => c.continuation = n,
            b'(' => c.unclosed = n,
            b'u' => c.unclosed2 = n,
            b'U' => c.unclosed_noignore = n,
            b'W' => c.unclosed_wrapped = n,
            b'w' => c.unclosed_whiteok = n,
            b'm' => c.matching_paren = n,
            b'M' => c.paren_prev = n,
            b')' => c.maxparen = n,
            b'*' => c.maxcomment = n,
            b'g' => c.scopedecl = n,
            b'h' => c.scopedecl_code = n,
            b'j' => c.java = n,
            b'J' => c.js = n,
            b'l' => c.keep_case_label = n,
            b'#' => c.hash_comment = n,
            b'N' => c.cpp_namespace = n,
            b'k' => c.if_for_while = n,
            b'E' => c.cpp_extern_c = n,
            b'P' => c.pragma = n,
            _ => {}
        }
        if at(p, i) == b',' {
            i += 1;
        }
    }
    c
}

/// Whether 'cinkeys' (or 'indentkeys') makes `#` reindent at the start of a line
/// (`in_cinkeys('#', ' ', true)`).
fn hash_in_keys(keys: &str) -> bool {
    keys.split(',').any(|e| {
        if e.starts_with('*') {
            return false;
        }
        let e = e.trim_start_matches('!');
        let e = e.strip_prefix('0').unwrap_or(e);
        e == "#"
    })
}

/// `cin_is_cpp_baseclass`'s cache.
struct BaseclassCache {
    found: bool,
    lnum: usize,
    col: usize,
}

/// The C indenter's state: the buffer, the options, and Vim's cursor.
struct Cin<'a> {
    ctx: &'a Ctx<'a>,
    cino: Cino,
    cursor: Pos,
    /// The cursor column on the line being indented in Insert mode.
    insert_col: Option<usize>,
    hash_in_keys: bool,
    lines: RefCell<HashMap<usize, Rc<[u8]>>>,
}

impl<'a> Cin<'a> {
    fn new(ctx: &'a Ctx<'a>) -> Self {
        let sw = ctx.sw() as i32;
        let keys = if ctx.opts.indentexpr.is_empty() {
            &ctx.opts.cinkeys
        } else {
            &ctx.opts.indentkeys
        };
        Cin {
            ctx,
            cino: parse_cino(&ctx.opts.cinoptions, sw),
            cursor: Pos {
                lnum: ctx.lnum + 1,
                col: ctx.cursor_col().unwrap_or(0),
            },
            insert_col: ctx.cursor_col(),
            hash_in_keys: hash_in_keys(keys),
            lines: RefCell::default(),
        }
    }

    fn line_count(&self) -> usize {
        self.ctx.line_count()
    }

    /// Line `lnum` (1-based), as Vim's `ml_get`.
    fn ml(&self, lnum: usize) -> Rc<[u8]> {
        if lnum == 0 || lnum > self.line_count() {
            return Rc::from(&b""[..]);
        }
        self.lines
            .borrow_mut()
            .entry(lnum)
            .or_insert_with(|| Rc::from(self.ctx.line(lnum - 1).as_bytes()))
            .clone()
    }

    fn cur_line(&self) -> Rc<[u8]> {
        self.ml(self.cursor.lnum)
    }

    fn get_indent_lnum(&self, lnum: usize) -> i32 {
        self.ctx.indent(lnum.saturating_sub(1)) as i32
    }

    fn get_indent(&self) -> i32 {
        self.get_indent_lnum(self.cursor.lnum)
    }

    /// The screen column where the character at byte `col` of line `lnum` starts.
    fn getvcol(&self, lnum: usize, col: usize) -> i32 {
        let line = self.ml(lnum);
        let ts = self.ctx.opts.tabstop.max(1);
        let text = String::from_utf8_lossy(&line[..col.min(line.len())]).into_owned();
        let mut v = 0;
        for c in text.chars() {
            v += match c {
                '\t' => ts - v % ts,
                c if (c as u32) < 0x20 || c == '\u{7f}' => 2,
                c => c.width().unwrap_or(1).max(1),
            };
        }
        v as i32
    }

    fn linewhite(&self, lnum: usize) -> bool {
        let l = self.ml(lnum);
        at(&l, skipwhite(&l, 0)) == 0
    }

    // ---- Scanning the text --------------------------------------------------------------

    /// Vim's `cin_skipcomment`: skip white space and C comments (and `#` comments with
    /// 'cino' `#`).
    fn skipcomment(&self, s: &[u8], mut i: usize) -> usize {
        while at(s, i) != 0 {
            let prev = i;
            i = skipwhite(s, i);
            if self.cino.hash_comment != 0 && i != prev && at(s, i) == b'#' {
                return s.len();
            }
            if at(s, i) != b'/' {
                break;
            }
            i += 1;
            if at(s, i) == b'/' {
                return s.len();
            }
            if at(s, i) != b'*' {
                break;
            }
            i += 1;
            while at(s, i) != 0 {
                if at(s, i) == b'*' && at(s, i + 1) == b'/' {
                    i += 2;
                    break;
                }
                i += 1;
            }
        }
        i
    }

    fn nocode(&self, s: &[u8], i: usize) -> bool {
        at(s, self.skipcomment(s, i)) == 0
    }

    fn skip_comment_and_string(&self, s: &[u8], mut p: usize) -> usize {
        loop {
            let r = p;
            p = self.skipcomment(s, p);
            if at(s, p) != 0 {
                p = skip_string(s, p);
            }
            if p == r {
                return p;
            }
        }
    }

    /// Vim's `cin_ends_in`.
    fn ends_in(&self, s: &[u8], find: &[u8]) -> bool {
        let mut p = 0;
        while at(s, p) != 0 {
            p = self.skipcomment(s, p);
            if starts(s, p, find) {
                let r = skipwhite(s, p + find.len());
                if self.nocode(s, r) {
                    return true;
                }
            }
            if at(s, p) != 0 {
                p += 1;
            }
        }
        false
    }

    fn cin_iselse(&self, s: &[u8], mut i: usize) -> bool {
        if at(s, i) == b'}' {
            i = self.skipcomment(s, i + 1);
        }
        starts(s, i, b"else") && !is_idc(at(s, i + 4))
    }

    fn is_cinword(&self, s: &[u8], i: usize) -> bool {
        let i = skipwhite(s, i);
        self.ctx.opts.cinwords.split(',').any(|w| {
            let w = w.as_bytes();
            let len = w.len();
            !w.is_empty()
                && starts(s, i, w)
                && (!is_idc(at(s, i + len)) || !is_idc(at(s, i + len - 1)))
        })
    }

    fn has_js_key(&self, s: &[u8], i: usize) -> bool {
        let mut i = skipwhite(s, i);
        let mut quote = 0;
        if at(s, i) == b'\'' || at(s, i) == b'"' {
            quote = at(s, i);
            i += 1;
        }
        if !is_idc(at(s, i)) {
            return false;
        }
        while is_idc(at(s, i)) {
            i += 1;
        }
        if at(s, i) != 0 && at(s, i) == quote {
            i += 1;
        }
        i = self.skipcomment(s, i);
        at(s, i) == b':' && at(s, i + 1) != b':'
    }

    /// Vim's `cin_islabel_skip`: `label:` at `*i`; moves past the `:`.
    fn islabel_skip(&self, s: &[u8], i: &mut usize) -> bool {
        if !is_idc(at(s, *i)) {
            return false;
        }
        while is_idc(at(s, *i)) {
            *i += char_len(at(s, *i));
        }
        *i = self.skipcomment(s, *i);
        if at(s, *i) == b':' {
            *i += 1;
            return at(s, *i) != b':';
        }
        false
    }

    fn cin_isdefault(&self, s: &[u8], i: usize) -> bool {
        if !starts(s, i, b"default") {
            return false;
        }
        let j = self.skipcomment(s, i + 7);
        at(s, j) == b':' && at(s, j + 1) != b':'
    }

    /// Vim's `cin_iscase`.
    fn iscase(&self, s: &[u8], i: usize, strict: bool) -> bool {
        let mut p = self.skipcomment(s, i);
        if cin_starts_with(s, p, b"case") {
            p += 4;
            while at(s, p) != 0 {
                p = self.skipcomment(s, p);
                if at(s, p) == 0 {
                    break;
                }
                if at(s, p) == b':' {
                    if at(s, p + 1) == b':' {
                        p += 1;
                    } else {
                        return true;
                    }
                }
                if at(s, p) == b'\'' && at(s, p + 1) != 0 && at(s, p + 2) == b'\'' {
                    p += 2;
                } else if at(s, p) == b'/' && (at(s, p + 1) == b'*' || at(s, p + 1) == b'/') {
                    return false;
                } else if at(s, p) == b'"' {
                    return !strict;
                }
                p += 1;
            }
            return false;
        }
        self.cin_isdefault(s, p)
    }

    /// Vim's `cin_isscopedecl` with the default 'cinscopedecls'.
    fn isscopedecl(&self, s: &[u8], i: usize) -> bool {
        let i = self.skipcomment(s, i);
        ["public", "protected", "private"].iter().any(|w| {
            starts(s, i, w.as_bytes()) && {
                let j = self.skipcomment(s, i + w.len());
                at(s, j) == b':' && at(s, j + 1) != b':'
            }
        })
    }

    fn is_cpp_namespace(&self, s: &[u8], i: usize) -> bool {
        let mut i = self.skipcomment(s, i);
        while (starts(s, i, b"inline") || starts(s, i, b"export"))
            && (at(s, i + 6) == 0 || !is_idc(at(s, i + 6)))
        {
            i = self.skipcomment(s, skipwhite(s, i + 6));
        }
        if starts(s, i, b"namespace") && (at(s, i + 9) == 0 || !is_idc(at(s, i + 9))) {
            let mut p = self.skipcomment(s, skipwhite(s, i + 9));
            let mut has_name = false;
            let mut has_name_start = false;
            while at(s, p) != 0 {
                if is_white(at(s, p)) {
                    has_name = true;
                    p = self.skipcomment(s, skipwhite(s, p));
                } else if at(s, p) == b'{' {
                    break;
                } else if is_idc(at(s, p)) {
                    has_name_start = true;
                    if has_name {
                        return false;
                    }
                    p += 1;
                } else if at(s, p) == b':' && at(s, p + 1) == b':' && is_idc(at(s, p + 2)) {
                    if !has_name_start || has_name {
                        return false;
                    }
                    p += 3;
                } else {
                    return false;
                }
            }
            return true;
        }
        false
    }

    fn is_cpp_extern_c(&self, s: &[u8], i: usize) -> bool {
        let i = self.skipcomment(s, i);
        if starts(s, i, b"extern") && (at(s, i + 6) == 0 || !is_idc(at(s, i + 6))) {
            let mut p = self.skipcomment(s, skipwhite(s, i + 6));
            let mut has = false;
            while at(s, p) != 0 {
                if is_white(at(s, p)) {
                    p = self.skipcomment(s, skipwhite(s, p));
                } else if at(s, p) == b'{' {
                    break;
                } else if starts(s, p, b"\"C\"") {
                    if has {
                        return false;
                    }
                    has = true;
                    p += 3;
                } else if starts(s, p, b"\"C++\"") {
                    if has {
                        return false;
                    }
                    has = true;
                    p += 5;
                } else {
                    return false;
                }
            }
            return has;
        }
        false
    }

    /// Vim's `after_label`: the first code after a `label:`.
    fn after_label(&self, l: &[u8]) -> Option<usize> {
        let mut i = 0;
        while at(l, i) != 0 {
            if at(l, i) == b':' {
                if at(l, i + 1) == b':' {
                    i += 1;
                } else if !self.iscase(l, i + 1, false) {
                    break;
                }
            } else if at(l, i) == b'\'' && at(l, i + 1) != 0 && at(l, i + 2) == b'\'' {
                i += 2;
            }
            i += 1;
        }
        if at(l, i) == 0 {
            return None;
        }
        let j = self.skipcomment(l, i + 1);
        if at(l, j) == 0 { None } else { Some(j) }
    }

    fn get_indent_nolabel(&self, lnum: usize) -> i32 {
        let l = self.ml(lnum);
        match self.after_label(&l) {
            None => 0,
            Some(p) => self.getvcol(lnum, p),
        }
    }

    /// Vim's `skip_label`: the indent of line `lnum` ignoring a label, and its text after it.
    fn skip_label(&mut self, lnum: usize) -> (i32, Lp) {
        let save = self.cursor;
        self.cursor.lnum = lnum;
        let l = self.cur_line();
        let result = if self.iscase(&l, 0, false) || self.isscopedecl(&l, 0) || self.cin_islabel() {
            let amount = self.get_indent_nolabel(lnum);
            let line = self.cur_line();
            let off = self.after_label(&line).unwrap_or(0);
            (amount, Lp { line, off })
        } else {
            (
                self.get_indent(),
                Lp {
                    line: self.cur_line(),
                    off: 0,
                },
            )
        };
        self.cursor = save;
        result
    }

    /// Vim's `cin_first_id_amount`.
    fn first_id_amount(&self) -> i32 {
        let line = self.cur_line();
        let mut p = skipwhite(&line, 0);
        let mut len = skiptowhite(&line, p) - p;
        if len == 6 && starts(&line, p, b"static") {
            p = skipwhite(&line, p + 6);
            len = skiptowhite(&line, p) - p;
        }
        if len == 6 && starts(&line, p, b"struct") {
            p = skipwhite(&line, p + 6);
        } else if len == 4 && starts(&line, p, b"enum") {
            p = skipwhite(&line, p + 4);
        } else if (len == 8 && starts(&line, p, b"unsigned"))
            || (len == 6 && starts(&line, p, b"signed"))
        {
            let s = skipwhite(&line, p + len);
            if (starts(&line, s, b"int") && is_white(at(&line, s + 3)))
                || (starts(&line, s, b"long") && is_white(at(&line, s + 4)))
                || (starts(&line, s, b"short") && is_white(at(&line, s + 5)))
                || (starts(&line, s, b"char") && is_white(at(&line, s + 4)))
            {
                p = s;
            }
        }
        let mut len = 0;
        while is_idc(at(&line, p + len)) {
            len += 1;
        }
        if len == 0 || !is_white(at(&line, p + len)) || self.nocode(&line, p) {
            return 0;
        }
        let p = skipwhite(&line, p + len);
        self.getvcol(self.cursor.lnum, p)
    }

    /// Vim's `cin_get_equal_amount`.
    fn get_equal_amount(&self, lnum: usize) -> i32 {
        if lnum > 1 && ends_in_backslash(&self.ml(lnum - 1)) {
            return -1;
        }
        let line = self.ml(lnum);
        let mut s = 0;
        while at(&line, s) != 0 && !b"=;{}\"'".contains(&at(&line, s)) {
            if cin_iscomment(&line, s) {
                s = self.skipcomment(&line, s);
            } else {
                s += 1;
            }
        }
        if at(&line, s) != b'=' {
            return 0;
        }
        s = skipwhite(&line, s + 1);
        if self.nocode(&line, s) {
            return 0;
        }
        if at(&line, s) == b'"' {
            s += 1;
        }
        self.getvcol(lnum, s)
    }

    /// Vim's `cin_ispreproc_cont`: line `*l` at `*lnum` is (part of) a preprocessor
    /// statement; moves `*lnum` to its start.
    fn ispreproc_cont(&self, l: &mut Lp, lnum: &mut usize, amount: &mut i32) -> bool {
        let mut line = l.clone();
        let mut n = *lnum;
        let mut retval = false;
        let mut candidate = *amount;
        if ends_in_backslash(&line.s()[line.off..]) {
            candidate = self.get_indent_lnum(n);
        }
        loop {
            if cin_ispreproc(line.s(), line.off) {
                retval = true;
                *lnum = n;
                break;
            }
            if n == 1 {
                break;
            }
            n -= 1;
            line = Lp {
                line: self.ml(n),
                off: 0,
            };
            if !ends_in_backslash(line.s()) {
                break;
            }
        }
        if n != *lnum {
            *l = Lp {
                line: self.ml(*lnum),
                off: 0,
            };
        }
        if retval {
            *amount = candidate;
        }
        retval
    }

    /// Vim's `cin_isterminated`.
    fn isterminated(&self, s: &[u8], i: usize, incl_open: bool, incl_comma: bool) -> u8 {
        let mut found_start = 0;
        let mut n_open = 0u32;
        let mut is_else = false;
        let mut p = self.skipcomment(s, i);
        if at(s, p) == b'{' || (at(s, p) == b'}' && !self.cin_iselse(s, p)) {
            found_start = at(s, p);
        }
        if found_start == 0 {
            is_else = self.cin_iselse(s, p);
        }
        while at(s, p) != 0 {
            p = skip_string(s, self.skipcomment(s, p));
            let c = at(s, p);
            if c == b'}' && n_open > 0 {
                n_open -= 1;
            }
            if (!is_else || n_open == 0)
                && (c == b';' || c == b'}' || (incl_comma && c == b','))
                && self.nocode(s, p + 1)
            {
                return c;
            } else if c == b'{' {
                if incl_open && self.nocode(s, p + 1) {
                    return c;
                }
                n_open += 1;
            }
            if at(s, p) != 0 {
                p += 1;
            }
        }
        found_start
    }

    fn is_compound_init(&self, s: &[u8], i: usize) -> bool {
        let mut p = i;
        let mut r = None;
        while at(s, p) != 0 {
            if at(s, p) == b'=' {
                p = self.skipcomment(s, p + 1);
                r = Some(p);
            } else if starts(s, p, b"return")
                && !is_idc(at(s, p + 6))
                && (p == i || (p > i && !is_idc(s[p - 1])))
            {
                p = self.skipcomment(s, p + 6);
                r = Some(p);
            } else {
                p = self.skip_comment_and_string(s, p + 1);
            }
        }
        let Some(mut p) = r else {
            return false;
        };
        if self.nocode(s, p) {
            return true;
        }
        if at(s, p) == b'&' {
            p = self.skipcomment(s, p + 1);
        }
        if at(s, p) == b'(' {
            let mut open = 1i32;
            loop {
                p = self.skip_comment_and_string(s, p + 1);
                if self.nocode(s, p) {
                    return true;
                }
                open += i32::from(at(s, p) == b'(') - i32::from(at(s, p) == b')');
                if open == 0 {
                    break;
                }
            }
            p = self.skipcomment(s, p + 1);
            if self.nocode(s, p) {
                return true;
            }
        }
        while at(s, p) == b'{' {
            p = self.skipcomment(s, p + 1);
        }
        self.nocode(s, p)
    }

    fn isinit(&self) -> bool {
        let line = self.cur_line();
        let mut s = self.skipcomment(&line, 0);
        if cin_starts_with(&line, s, b"typedef") {
            s = self.skipcomment(&line, s + 7);
        }
        loop {
            let mut skipped = false;
            for w in ["static", "public", "protected", "private"] {
                if cin_starts_with(&line, s, w.as_bytes()) {
                    s = self.skipcomment(&line, s + w.len());
                    skipped = true;
                    break;
                }
            }
            if !skipped {
                break;
            }
        }
        if cin_starts_with(&line, s, b"enum") {
            return true;
        }
        self.is_compound_init(&line, s)
    }

    // ---- Searching --------------------------------------------------------------------

    /// The 'matchpairs' partner of `initc` (Vim's `find_mps_values`): `(initc, findc,
    /// backwards)`.
    fn find_mps_values(&self, initc: char, switchit: bool) -> Option<(char, char, bool)> {
        for pair in self.ctx.opts.matchpairs.split(',') {
            let mut cs = pair.chars();
            let (Some(a), Some(_), Some(b)) = (cs.next(), cs.next(), cs.next()) else {
                continue;
            };
            if a == initc {
                return Some(if switchit {
                    (b, a, true)
                } else {
                    (a, b, false)
                });
            }
            if b == initc {
                return Some(if switchit {
                    (a, b, false)
                } else {
                    (b, a, true)
                });
            }
        }
        None
    }

    fn find_rawstring_end(&self, linep: &[u8], start: Pos, end: Pos) -> bool {
        let mut p = start.col + 1;
        while at(linep, p) != 0 && at(linep, p) != b'(' {
            p += 1;
        }
        let delim: Vec<u8> = linep[start.col + 1..p].to_vec();
        for lnum in start.lnum..=end.lnum {
            let line = self.ml(lnum);
            let mut q = if lnum == start.lnum { start.col + 1 } else { 0 };
            while at(&line, q) != 0 {
                if lnum == end.lnum && q >= end.col {
                    break;
                }
                if at(&line, q) == b')'
                    && starts(&line, q + 1, &delim)
                    && at(&line, q + delim.len() + 1) == b'"'
                {
                    return true;
                }
                q += 1;
            }
        }
        false
    }

    /// The parts of Vim's `findmatchlimit` that C indenting uses: matching brackets from the
    /// cursor (`initc` NUL for the one under or after it), and finding the start of a comment
    /// (`*`) or raw string (`R`) backwards. 'cpoptions' has neither `%` nor `M`.
    fn findmatchlimit(&self, initc_in: u8, flags: u32, maxtravel: i64) -> Option<Pos> {
        let mut pos = self.cursor;
        let mut linep = self.ml(pos.lnum);
        let dir: i32 = if flags & FM_BACKWARD != 0 {
            -1
        } else if flags & FM_FORWARD != 0 {
            1
        } else {
            0
        };
        let mut initc: char = '\0';
        let mut findc: char = '\0';
        let mut backwards = false;
        let mut raw_string = false;
        let mut comment_dir = 0;
        let mut ignore_cend = false;
        let mut match_escaped = 0;
        let mut count = 0i32;
        let mut traveled = 0i64;
        let mut inquote = false;
        let mut comment_col = NOCOL;

        if matches!(initc_in, b'/' | b'*' | b'R') {
            comment_dir = dir;
            ignore_cend = initc_in == b'/';
            backwards = dir != 1;
            raw_string = initc_in == b'R';
        } else if initc_in != 0 {
            let (i, f, b) = self.find_mps_values(char::from(initc_in), true)?;
            initc = i;
            findc = f;
            backwards = b;
            if dir != 0 {
                backwards = dir == -1;
            }
        } else {
            // The bracket under or after the cursor (or a comment end).
            let mut hash_dir = 0;
            let ptr = skipwhite(&linep, 0);
            if at(&linep, ptr) == b'#' && pos.col <= ptr {
                let p2 = skipwhite(&linep, ptr + 1);
                if starts(&linep, p2, b"if")
                    || starts(&linep, p2, b"endif")
                    || starts(&linep, p2, b"el")
                {
                    hash_dir = 1;
                }
            } else if at(&linep, pos.col) == b'/' {
                if at(&linep, pos.col + 1) == b'*' {
                    comment_dir = 1;
                    backwards = false;
                    pos.col += 1;
                } else if pos.col > 0 && at(&linep, pos.col - 1) == b'*' {
                    comment_dir = -1;
                    backwards = true;
                    pos.col -= 1;
                }
            } else if at(&linep, pos.col) == b'*' {
                if at(&linep, pos.col + 1) == b'/' {
                    comment_dir = -1;
                    backwards = true;
                } else if pos.col > 0 && at(&linep, pos.col - 1) == b'/' {
                    comment_dir = 1;
                    backwards = false;
                }
            }
            if hash_dir == 0 && comment_dir == 0 {
                if at(&linep, pos.col) == 0 && pos.col > 0 {
                    pos.col -= 1;
                }
                loop {
                    initc = char_at(&linep, pos.col);
                    if initc == '\0' {
                        break;
                    }
                    if let Some((i, f, b)) = self.find_mps_values(initc, false) {
                        initc = i;
                        findc = f;
                        backwards = b;
                        break;
                    }
                    pos.col += char_len(linep[pos.col]);
                }
                if findc == '\0' {
                    if at(&linep, skipwhite(&linep, 0)) == b'#' {
                        hash_dir = 1;
                    } else {
                        return None;
                    }
                } else {
                    let mut bslcnt = 0;
                    let mut col = pos.col;
                    while col > 0 && linep[col - 1] == b'\\' {
                        bslcnt += 1;
                        col -= 1;
                    }
                    match_escaped = bslcnt & 1;
                }
            }
            if hash_dir != 0 {
                // Matching `#if`/`#endif` isn't needed for indenting.
                return None;
            }
        }

        let mut do_quotes: i32 = -1;
        let mut start_in_quotes: Option<bool> = None;
        let mut match_pos = Pos { lnum: 0, col: 0 };
        if backwards && comment_dir != 0 {
            comment_col = check_linecomment(&linep);
        }

        loop {
            if backwards {
                if pos.col == 0 {
                    if pos.lnum == 1 {
                        break;
                    }
                    pos.lnum -= 1;
                    if maxtravel > 0 {
                        traveled += 1;
                        if traveled > maxtravel {
                            break;
                        }
                    }
                    linep = self.ml(pos.lnum);
                    pos.col = linep.len();
                    do_quotes = -1;
                    if comment_dir != 0 {
                        comment_col = check_linecomment(&linep);
                    }
                } else {
                    pos.col -= 1;
                    while pos.col > 0 && (0x80..0xc0).contains(&linep[pos.col]) {
                        pos.col -= 1;
                    }
                }
            } else if at(&linep, pos.col) == 0 {
                if pos.lnum == self.line_count() {
                    break;
                }
                pos.lnum += 1;
                if maxtravel != 0 {
                    let t = traveled;
                    traveled += 1;
                    if t > maxtravel {
                        break;
                    }
                }
                linep = self.ml(pos.lnum);
                pos.col = 0;
                do_quotes = -1;
            } else {
                pos.col += char_len(linep[pos.col]);
            }

            if pos.col == 0
                && flags & FM_BLOCKSTOP != 0
                && (at(&linep, 0) == b'{' || at(&linep, 0) == b'}')
            {
                if char::from(at(&linep, 0)) == findc && count == 0 {
                    return Some(pos);
                }
                break;
            }

            if comment_dir != 0 {
                if comment_dir == 1 {
                    if at(&linep, pos.col) == b'*' && at(&linep, pos.col + 1) == b'/' {
                        pos.col += 1;
                        return Some(pos);
                    }
                } else {
                    let c = pos.col;
                    if c == 0 {
                        continue;
                    } else if raw_string {
                        if at(&linep, c - 1) == b'R'
                            && at(&linep, c) == b'"'
                            && strchr(&linep, c + 1, b'(').is_some()
                        {
                            let end = if count > 0 { match_pos } else { self.cursor };
                            if !self.find_rawstring_end(&linep, pos, end) {
                                count += 1;
                                match_pos = pos;
                                match_pos.col -= 1;
                            }
                        }
                    } else if at(&linep, c - 1) == b'/'
                        && at(&linep, c) == b'*'
                        && (c == 1 || at(&linep, c - 2) != b'*')
                        && c < comment_col
                    {
                        count += 1;
                        match_pos = pos;
                        match_pos.col -= 1;
                    } else if at(&linep, c - 1) == b'*' && at(&linep, c) == b'/' {
                        if count > 0 {
                            pos = match_pos;
                        } else if c > 1 && at(&linep, c - 2) == b'/' && c <= comment_col {
                            pos.col -= 2;
                        } else if ignore_cend {
                            continue;
                        } else {
                            return None;
                        }
                        return Some(pos);
                    }
                }
                continue;
            }

            if do_quotes == -1 {
                // Count the quotes in the line, skipping \" and '"'.
                let mut at_start = do_quotes;
                let mut p = 0;
                while p < linep.len() {
                    if p == pos.col + usize::from(backwards) {
                        at_start = do_quotes & 1;
                    }
                    if linep[p] == b'"'
                        && (p == 0 || linep[p - 1] != b'\'' || at(&linep, p + 1) != b'\'')
                    {
                        do_quotes += 1;
                    }
                    if linep[p] == b'\\' && at(&linep, p + 1) != 0 {
                        p += 1;
                    }
                    p += 1;
                }
                if p == pos.col + usize::from(backwards) {
                    at_start = do_quotes & 1;
                }
                do_quotes &= 1;
                if do_quotes == 0 {
                    inquote = false;
                    if ends_in_backslash(&linep) {
                        do_quotes = 1;
                        if start_in_quotes.is_none() {
                            inquote = true;
                            start_in_quotes = Some(true);
                        } else if backwards {
                            inquote = true;
                        }
                    }
                    if pos.lnum > 1 && ends_in_backslash(&self.ml(pos.lnum - 1)) {
                        do_quotes = 1;
                        if start_in_quotes.is_none() {
                            inquote = at_start != 0;
                            if inquote {
                                start_in_quotes = Some(true);
                            }
                        } else if !backwards {
                            inquote = true;
                        }
                    }
                }
            }
            if start_in_quotes.is_none() {
                start_in_quotes = Some(false);
            }

            let c = char_at(&linep, pos.col);
            let mut check = true;
            match c {
                '\0' => {
                    check = false;
                    if pos.col == 0 || linep[pos.col - 1] != b'\\' {
                        inquote = false;
                        start_in_quotes = Some(false);
                    }
                }
                '"' => {
                    check = false;
                    if do_quotes != 0 {
                        let mut col = pos.col as isize - 1;
                        while col >= 0 && linep[col as usize] == b'\\' {
                            col -= 1;
                        }
                        if ((pos.col as isize - 1 - col) & 1) == 0 {
                            inquote = !inquote;
                            start_in_quotes = Some(false);
                        }
                    }
                }
                '\'' if initc != '\'' && findc != '\'' => {
                    let col = pos.col;
                    if backwards {
                        if col > 1 {
                            if at(&linep, col - 2) == b'\'' {
                                pos.col -= 2;
                                check = false;
                            } else if at(&linep, col - 2) == b'\\'
                                && col > 2
                                && at(&linep, col - 3) == b'\''
                            {
                                pos.col -= 3;
                                check = false;
                            }
                        }
                    } else if at(&linep, col + 1) != 0 {
                        if at(&linep, col + 1) == b'\\'
                            && at(&linep, col + 2) != 0
                            && at(&linep, col + 3) == b'\''
                        {
                            pos.col += 3;
                            check = false;
                        } else if at(&linep, col + 2) == b'\'' {
                            pos.col += 2;
                            check = false;
                        }
                    }
                }
                _ => {}
            }
            if check && (!inquote || start_in_quotes == Some(true)) && (c == initc || c == findc) {
                let mut bslcnt = 0;
                let mut col = pos.col;
                while col > 0 && linep[col - 1] == b'\\' {
                    bslcnt += 1;
                    col -= 1;
                }
                if (bslcnt & 1) == match_escaped {
                    if c == initc {
                        count += 1;
                    } else {
                        if count == 0 {
                            return Some(pos);
                        }
                        count -= 1;
                    }
                }
            }
        }
        if comment_dir == -1 && count > 0 {
            return Some(match_pos);
        }
        None
    }

    /// Vim's `find_start_comment`.
    fn find_start_comment(&self, ind_maxcomment: i32) -> Option<Pos> {
        let mut cur_max = i64::from(ind_maxcomment);
        loop {
            let pos = self.findmatchlimit(b'*', FM_BACKWARD, cur_max)?;
            if !is_pos_in_string(&self.ml(pos.lnum), pos.col) {
                return Some(pos);
            }
            cur_max = self.cursor.lnum as i64 - pos.lnum as i64 - 1;
            if cur_max <= 0 {
                return None;
            }
        }
    }

    fn find_start_rawstring(&self, ind_maxcomment: i32) -> Option<Pos> {
        let mut cur_max = i64::from(ind_maxcomment);
        loop {
            let pos = self.findmatchlimit(b'R', FM_BACKWARD, cur_max)?;
            if !is_pos_in_string(&self.ml(pos.lnum), pos.col) {
                return Some(pos);
            }
            cur_max = self.cursor.lnum as i64 - pos.lnum as i64 - 1;
            if cur_max <= 0 {
                return None;
            }
        }
    }

    fn ind_find_start_comment(&self) -> Option<Pos> {
        self.find_start_comment(self.cino.maxcomment)
    }

    /// Vim's `ind_find_start_CORS`: the start of the comment or raw string the cursor is in.
    fn ind_find_start_cors(&self, is_raw: Option<&mut usize>) -> Option<Pos> {
        let comment_pos = self.find_start_comment(self.cino.maxcomment);
        let rs_pos = self.find_start_rawstring(self.cino.maxcomment);
        match comment_pos {
            Some(c) if !rs_pos.is_some_and(|r| lt(r, c)) => Some(c),
            _ => {
                if let (Some(raw), Some(r)) = (is_raw, rs_pos) {
                    *raw = r.lnum;
                }
                rs_pos
            }
        }
    }

    /// Vim's `cin_skip2pos`.
    fn skip2pos(&self, trypos: Pos) -> usize {
        let line = self.ml(trypos.lnum);
        let mut p = 0;
        while at(&line, p) != 0 && p < trypos.col {
            if cin_iscomment(&line, p) {
                p = self.skipcomment(&line, p);
            } else {
                let n = skip_string(&line, p);
                p = if n == p { p + 1 } else { n };
            }
        }
        p
    }

    /// Vim's `find_start_brace`: the `{` of the block the cursor is in.
    fn find_start_brace(&mut self) -> Option<Pos> {
        let save = self.cursor;
        let mut trypos;
        loop {
            trypos = self.findmatchlimit(b'{', FM_BLOCKSTOP, 0);
            let Some(t) = trypos else { break };
            self.cursor = t;
            let mut pos = None;
            if self.skip2pos(t) == t.col {
                pos = self.ind_find_start_cors(None);
                if pos.is_none() {
                    break;
                }
            }
            if let Some(p) = pos {
                self.cursor = p;
            }
        }
        self.cursor = save;
        trypos
    }

    fn find_match_paren(&mut self, ind_maxparen: i32) -> Option<Pos> {
        self.find_match_char(b'(', ind_maxparen)
    }

    /// Vim's `find_match_char`: the unmatched `c` before the cursor, not in a comment.
    fn find_match_char(&mut self, c: u8, ind_maxparen: i32) -> Option<Pos> {
        let save = self.cursor;
        let mut maxp = ind_maxparen;
        let result = loop {
            let Some(t) = self.findmatchlimit(c, 0, i64::from(maxp)) else {
                break None;
            };
            if self.skip2pos(t) > t.col {
                // In a `//` comment.
                maxp = ind_maxparen - (save.lnum as i32 - t.lnum as i32);
                if maxp > 0 {
                    self.cursor = t;
                    self.cursor.col = 0;
                    continue;
                }
                break None;
            }
            self.cursor = t;
            if let Some(wk) = self.ind_find_start_cors(None) {
                maxp = ind_maxparen - (save.lnum as i32 - wk.lnum as i32);
                if maxp > 0 {
                    self.cursor = wk;
                    continue;
                }
                break None;
            }
            break Some(t);
        };
        self.cursor = save;
        result
    }

    fn find_match_paren_after_brace(&mut self, ind_maxparen: i32) -> Option<Pos> {
        let t = self.find_match_paren(ind_maxparen)?;
        if let Some(b) = self.find_start_brace()
            && (if t.lnum != b.lnum {
                t.lnum < b.lnum
            } else {
                t.col < b.col
            })
        {
            return None;
        }
        Some(t)
    }

    fn corr_ind_maxparen(&self, startpos: Pos) -> i32 {
        let n = startpos.lnum as i32 - self.cursor.lnum as i32;
        if n > 0 && n < self.cino.maxparen / 2 {
            self.cino.maxparen - n
        } else {
            self.cino.maxparen
        }
    }

    /// Vim's `find_last_paren`: put the cursor on the last unmatched `end` in `l`.
    fn find_last_paren(&mut self, l: &[u8], start: u8, end: u8) -> bool {
        let mut retval = false;
        let mut open = 0;
        self.cursor.col = 0;
        let mut i = 0;
        while at(l, i) != 0 {
            i = self.skipcomment(l, i);
            i = skip_string(l, i);
            if at(l, i) == start {
                open += 1;
            } else if at(l, i) == end {
                if open > 0 {
                    open -= 1;
                } else {
                    self.cursor.col = i;
                    retval = true;
                }
            }
            i += 1;
        }
        retval
    }

    /// Vim's `find_line_comment`: a `//` comment in the previous lines, skipping blank ones.
    fn find_line_comment(&self) -> Option<Pos> {
        let mut lnum = self.cursor.lnum;
        while lnum > 1 {
            lnum -= 1;
            let line = self.ml(lnum);
            let p = skipwhite(&line, 0);
            if cin_islinecomment(&line, p) {
                return Some(Pos { lnum, col: p });
            }
            if at(&line, p) != 0 {
                break;
            }
        }
        None
    }

    // ---- Recognizing statements ---------------------------------------------------------

    /// Vim's `cin_islabel` for the cursor line.
    fn cin_islabel(&mut self) -> bool {
        let line = self.cur_line();
        self.islabel_line(&line)
    }

    /// `cin_islabel` with `line` as the cursor line's text.
    fn islabel_line(&mut self, line: &[u8]) -> bool {
        let mut s = self.skipcomment(line, 0);
        if self.cin_isdefault(line, s) || self.isscopedecl(line, s) {
            return false;
        }
        if !self.islabel_skip(line, &mut s) {
            return false;
        }
        if self.ind_find_start_cors(None).is_some() {
            return false;
        }
        let save = self.cursor;
        while self.cursor.lnum > 1 {
            self.cursor.lnum -= 1;
            self.cursor.col = 0;
            if let Some(t) = self.ind_find_start_cors(None) {
                self.cursor = t;
            }
            let l = self.cur_line();
            if cin_ispreproc(&l, 0) {
                continue;
            }
            let i = self.skipcomment(&l, 0);
            if at(&l, i) == 0 {
                continue;
            }
            self.cursor = save;
            let mut j = i;
            return self.isterminated(&l, i, true, false) != 0
                || self.isscopedecl(&l, i)
                || self.iscase(&l, i, true)
                || (self.islabel_skip(&l, &mut j) && self.nocode(&l, j));
        }
        self.cursor = save;
        true
    }

    /// Vim's `cin_isfuncdecl`.
    fn isfuncdecl(&mut self, sp: Option<&mut Lp>, first_lnum: usize, min_lnum: usize) -> bool {
        let mut lnum = first_lnum;
        let save_lnum = self.cursor.lnum;
        let mut retval = false;
        let mut just_started = true;
        let mut s: Lp = match &sp {
            Some(l) => (*l).clone(),
            None => Lp {
                line: self.ml(lnum),
                off: 0,
            },
        };
        self.cursor.lnum = lnum;
        let sline = s.line.clone();
        // Like Vim, the column is relative to `s`.
        if self.find_last_paren(&sline[s.off..], b'(', b')')
            && let Some(t) = self.find_match_paren(self.cino.maxparen)
        {
            lnum = t.lnum;
            if lnum < min_lnum {
                self.cursor.lnum = save_lnum;
                return false;
            }
            s = Lp {
                line: self.ml(lnum),
                off: 0,
            };
        }
        self.cursor.lnum = save_lnum;
        if cin_ispreproc(s.s(), s.off) {
            return false;
        }
        let mut line = s.line.clone();
        let mut i = s.off;
        while at(&line, i) != 0
            && at(&line, i) != b'('
            && at(&line, i) != b';'
            && at(&line, i) != b'\''
            && at(&line, i) != b'"'
        {
            if cin_iscomment(&line, i) {
                i = self.skipcomment(&line, i);
            } else if at(&line, i) == b':' {
                if at(&line, i + 1) == b':' {
                    i += 2;
                } else {
                    return false;
                }
            } else {
                i += 1;
            }
        }
        if at(&line, i) != b'(' {
            return false;
        }
        while at(&line, i) != 0
            && at(&line, i) != b';'
            && at(&line, i) != b'\''
            && at(&line, i) != b'"'
        {
            if at(&line, i) == b')' && self.nocode(&line, i + 1) {
                lnum = first_lnum.saturating_sub(1);
                let prev = self.ml(lnum);
                if !ends_in_backslash(&prev) {
                    retval = true;
                }
                break;
            }
            if (at(&line, i) == b',' && self.nocode(&line, i + 1))
                || at(&line, i + 1) == 0
                || self.nocode(&line, i)
            {
                let comma = at(&line, i) == b',';
                loop {
                    if lnum >= self.line_count() {
                        break;
                    }
                    lnum += 1;
                    line = self.ml(lnum);
                    if !cin_ispreproc(&line, 0) {
                        break;
                    }
                }
                if lnum >= self.line_count() {
                    break;
                }
                i = skipwhite(&line, 0);
                if !just_started && !comma && at(&line, i) != b',' && at(&line, i) != b')' {
                    break;
                }
                just_started = false;
            } else if cin_iscomment(&line, i) {
                i = self.skipcomment(&line, i);
            } else {
                i += 1;
                just_started = false;
            }
        }
        if lnum != first_lnum
            && let Some(sp) = sp
        {
            *sp = Lp {
                line: self.ml(first_lnum),
                off: 0,
            };
        }
        retval
    }

    /// Vim's `cin_iswhileofdo`.
    fn iswhileofdo(&mut self, s: &[u8], i: usize, lnum: usize) -> bool {
        let mut p = self.skipcomment(s, i);
        if at(s, p) == b'}' {
            p = self.skipcomment(s, p + 1);
        }
        if !cin_starts_with(s, p, b"while") {
            return false;
        }
        let save = self.cursor;
        self.cursor = Pos { lnum, col: 0 };
        let line = self.cur_line();
        let mut col = 0;
        while at(&line, col) != 0 && at(&line, col) != b'w' {
            col += 1;
        }
        self.cursor.col = col;
        let mut retval = false;
        if let Some(t) = self.findmatchlimit(0, 0, i64::from(self.cino.maxparen)) {
            let l = self.ml(t.lnum);
            if at(&l, self.skipcomment(&l, t.col + 1)) == b';' {
                retval = true;
            }
        }
        self.cursor = save;
        retval
    }

    /// Vim's `cin_iswhileofdo_end`: moves the cursor to the `while` line.
    fn iswhileofdo_end(&mut self, terminated: u8) -> bool {
        if terminated != b';' {
            return false;
        }
        let line = self.cur_line();
        let mut p = 0;
        while at(&line, p) != 0 {
            p = self.skipcomment(&line, p);
            if at(&line, p) == b')' {
                let s = skipwhite(&line, p + 1);
                if at(&line, s) == b';' && self.nocode(&line, s + 1) {
                    self.cursor.col = p;
                    if let Some(t) = self.find_match_paren(self.cino.maxparen) {
                        let l = self.ml(t.lnum);
                        let mut s = self.skipcomment(&l, 0);
                        if at(&l, s) == b'}' {
                            s = self.skipcomment(&l, s + 1);
                        }
                        if cin_starts_with(&l, s, b"while") {
                            self.cursor.lnum = t.lnum;
                            return true;
                        }
                    }
                }
            }
            if at(&line, p) != 0 {
                p += 1;
            }
        }
        false
    }

    /// Vim's `cin_is_cpp_baseclass`.
    fn is_cpp_baseclass(&mut self, cache: &mut BaseclassCache) -> bool {
        let mut lnum = self.cursor.lnum;
        if cache.lnum <= lnum {
            return cache.found;
        }
        cache.col = 0;
        let line = self.cur_line();
        let s = skipwhite(&line, 0);
        if at(&line, s) == b'#' {
            return false;
        }
        let s = self.skipcomment(&line, s);
        if at(&line, s) == 0 {
            return false;
        }
        let mut cpp_base_class = false;
        let mut lookfor_ctor_init = false;
        let mut class_or_struct = false;
        while lnum > 1 {
            let line = self.ml(lnum - 1);
            let mut s = skipwhite(&line, 0);
            if at(&line, s) == b'#' || at(&line, s) == 0 {
                break;
            }
            while at(&line, s) != 0 {
                s = self.skipcomment(&line, s);
                if at(&line, s) == b'{'
                    || at(&line, s) == b'}'
                    || (at(&line, s) == b';' && self.nocode(&line, s + 1))
                {
                    break;
                }
                if at(&line, s) != 0 {
                    s += 1;
                }
            }
            if at(&line, s) != 0 {
                break;
            }
            lnum -= 1;
        }
        cache.lnum = lnum;
        let mut line = self.ml(lnum);
        let mut s = 0;
        loop {
            if at(&line, s) == 0 {
                if lnum == self.cursor.lnum {
                    break;
                }
                lnum += 1;
                line = self.ml(lnum);
                s = 0;
            }
            if s == 0 {
                if self.iscase(&line, 0, false) {
                    break;
                }
                s = self.skipcomment(&line, 0);
                if at(&line, s) == 0 {
                    continue;
                }
            }
            if at(&line, s) == b'"' || (at(&line, s) == b'R' && at(&line, s + 1) == b'"') {
                s = skip_string(&line, s) + 1;
            } else if at(&line, s) == b':' {
                if at(&line, s + 1) == b':' {
                    lookfor_ctor_init = false;
                    s = self.skipcomment(&line, s + 2);
                } else if lookfor_ctor_init || class_or_struct {
                    cpp_base_class = true;
                    lookfor_ctor_init = false;
                    class_or_struct = false;
                    cache.col = 0;
                    s = self.skipcomment(&line, s + 1);
                } else {
                    s = self.skipcomment(&line, s + 1);
                }
            } else if (starts(&line, s, b"class") && !is_idc(at(&line, s + 5)))
                || (starts(&line, s, b"struct") && !is_idc(at(&line, s + 6)))
            {
                class_or_struct = true;
                lookfor_ctor_init = false;
                s = self.skipcomment(&line, s + if at(&line, s) == b'c' { 5 } else { 6 });
            } else {
                let c = at(&line, s);
                if c == b'{' || c == b'}' || c == b';' {
                    cpp_base_class = false;
                    lookfor_ctor_init = false;
                    class_or_struct = false;
                } else if c == b')' {
                    class_or_struct = false;
                    lookfor_ctor_init = true;
                } else if c == b'?' {
                    return false;
                } else if !is_idc(c) {
                    class_or_struct = false;
                    lookfor_ctor_init = false;
                } else if cache.col == 0 {
                    lookfor_ctor_init = false;
                    if cpp_base_class {
                        cache.col = s;
                    }
                }
                if lnum == self.cursor.lnum && c == b',' && self.nocode(&line, s + 1) {
                    cache.col = 0;
                }
                s = self.skipcomment(&line, s + 1);
            }
        }
        cache.found = cpp_base_class;
        if cpp_base_class {
            cache.lnum = lnum;
        }
        cpp_base_class
    }

    fn get_baseclass_amount(&mut self, col: usize) -> i32 {
        let mut amount;
        if col == 0 {
            amount = self.get_indent();
            let line = self.cur_line();
            if self.find_last_paren(&line, b'(', b')')
                && let Some(t) = self.find_match_paren(self.cino.maxparen)
            {
                amount = self.get_indent_lnum(t.lnum);
            }
            if !self.ends_in(&self.cur_line(), b",") {
                amount += self.cino.cpp_baseclass;
            }
        } else {
            self.cursor.col = col;
            amount = self.getvcol(self.cursor.lnum, col);
        }
        amount.max(self.cino.cpp_baseclass)
    }

    /// Vim's `find_match`: the `if` for an `else`, or the `do` for a `while`.
    fn find_match(&mut self, lookfor: i32, ourscope: usize) -> bool {
        let (mut elselevel, mut whilelevel) = if lookfor == LOOKFOR_IF {
            (1, 0)
        } else {
            (0, 1)
        };
        self.cursor.col = 0;
        while self.cursor.lnum > ourscope + 1 {
            self.cursor.lnum -= 1;
            self.cursor.col = 0;
            let line = self.cur_line();
            let look = self.skipcomment(&line, 0);
            if !self.cin_iselse(&line, look)
                && !cin_isif(&line, look)
                && !cin_isdo(&line, look)
                && !self.iswhileofdo(&line, look, self.cursor.lnum)
            {
                continue;
            }
            let Some(theirscope) = self.find_start_brace() else {
                break;
            };
            if theirscope.lnum < ourscope {
                break;
            }
            if theirscope.lnum > ourscope {
                continue;
            }
            let line = self.cur_line();
            let look = self.skipcomment(&line, 0);
            if !(lookfor == LOOKFOR_IF && whilelevel != 0) {
                if self.cin_iselse(&line, look) {
                    let mightbeif = self.skipcomment(&line, look + 4);
                    if !cin_isif(&line, mightbeif) {
                        elselevel += 1;
                    }
                    continue;
                }
                if cin_isif(&line, look) {
                    elselevel -= 1;
                    if elselevel == 0 && lookfor == LOOKFOR_IF {
                        whilelevel = 0;
                    }
                }
            }
            if self.iswhileofdo(&line, look, self.cursor.lnum) {
                whilelevel += 1;
                continue;
            }
            if cin_isdo(&line, look) {
                whilelevel -= 1;
            }
            if elselevel <= 0 && whilelevel <= 0 {
                return true;
            }
        }
        false
    }

    // ---- get_c_indent -------------------------------------------------------------------

    /// Vim's `get_c_indent`. `None` leaves the indent alone (inside a raw string).
    fn get_c_indent(&mut self) -> Option<i32> {
        let cur_curpos = self.cursor;
        let result = self.c_indent();
        self.cursor = cur_curpos;
        result.map(|a| a.max(0))
    }

    fn c_indent(&mut self) -> Option<i32> {
        let cur_curpos = self.cursor;
        if cur_curpos.lnum == 1 {
            return Some(0);
        }
        let mut linecopy = self.ml(cur_curpos.lnum).to_vec();
        // In Insert mode on a ')': don't line up with the matching '('.
        if let Some(col) = self.insert_col
            && col < linecopy.len()
            && linecopy[col] == b')'
        {
            linecopy.truncate(col);
        }
        let theline: Vec<u8> = linecopy[skipwhite(&linecopy, 0)..].to_vec();
        let theline = &theline[..];
        let t0 = at(theline, 0);
        self.cursor.col = 0;
        let original_line_islabel = self.cin_islabel();

        let comment_pos = self.ind_find_start_comment();
        if let Some(t) = self.find_start_rawstring(self.cino.maxcomment)
            && comment_pos.is_none_or(|c| lt(t, c))
        {
            return None;
        }

        // `#` lines go to the left when '#' is in 'cinkeys'.
        if t0 == b'#' && (at(&linecopy, 0) == b'#' || self.hash_in_keys) {
            let d = skipwhite(theline, 1);
            if self.cino.pragma == 0 || !starts(theline, d, b"pragma") {
                return Some(self.cino.hash_comment);
            }
        }

        // A non-case label goes to the left margin.
        if original_line_islabel && self.cino.js == 0 && self.cino.jump_label < 0 {
            return Some(0);
        }

        // A `//` comment below another one lines up with it.
        if cin_islinecomment(theline, 0) {
            let mut trypos = self.find_line_comment();
            if trypos.is_none() && self.cursor.lnum > 1 {
                let c = check_linecomment(&self.ml(self.cursor.lnum - 1));
                if c != NOCOL {
                    trypos = Some(Pos {
                        lnum: self.cursor.lnum - 1,
                        col: c,
                    });
                }
            }
            if let Some(t) = trypos {
                return Some(self.getvcol(t.lnum, t.col));
            }
        }

        // Inside a comment: 'comments' says how.
        if !cin_iscomment(theline, 0)
            && let Some(cp) = comment_pos
        {
            return Some(self.in_comment_indent(theline, cp, cur_curpos));
        }

        // A ']' lines up with the line of its '['.
        if at(theline, skipwhite(theline, 0)) == b']'
            && let Some(t) = self.find_match_char(b'[', self.cino.maxparen)
        {
            return Some(self.get_indent_lnum(t.lnum));
        }

        // Inside parentheses or braces?
        let mut trypos = self.find_match_paren(self.cino.maxparen);
        let mut trypos_brace = None;
        let inside = if trypos.is_some() && self.cino.java == 0 {
            true
        } else {
            trypos_brace = self.find_start_brace();
            trypos_brace.is_some() || trypos.is_some()
        };
        if inside {
            if let (Some(t), Some(b)) = (trypos, trypos_brace) {
                let paren_first = if t.lnum != b.lnum {
                    t.lnum < b.lnum
                } else {
                    t.col < b.col
                };
                if paren_first {
                    trypos = None;
                } else {
                    trypos_brace = None;
                }
            }
            let mut amount = match trypos {
                Some(t) => self.paren_indent(theline, t, cur_curpos),
                None => {
                    let brace = trypos_brace.expect("inside braces");
                    match self.brace_indent(theline, brace, cur_curpos) {
                        Ok(a) => return Some(a),
                        Err(a) => a,
                    }
                }
            };
            if cin_iscomment(theline, 0) {
                amount += self.cino.comment;
            }
            if self.cino.jump_label > 0 && original_line_islabel {
                amount -= self.cino.jump_label;
            }
            return Some(amount);
        }

        Some(self.top_level_indent(theline, cur_curpos))
    }

    /// The indent of a line inside a `/* */` comment (not its first line).
    fn in_comment_indent(&mut self, theline: &[u8], cp: Pos, cur_curpos: Pos) -> i32 {
        let mut amount = self.getvcol(cp.lnum, cp.col);
        let mut lead_start: Vec<u8> = Vec::new();
        let mut lead_start_len = 2;
        let mut lead_middle: Vec<u8> = Vec::new();
        let mut lead_middle_len = 1;
        let mut start_align = 0u8;
        let mut start_off = 0i32;
        let mut done = false;
        let com = self.ctx.opts.comments.clone();
        let p = com.as_bytes();
        let mut i = 0;
        while at(p, i) != 0 {
            let mut align = 0u8;
            let mut off = 0i32;
            let mut what = 0u8;
            while at(p, i) != 0 && at(p, i) != b':' {
                let c = p[i];
                if c == b's' || c == b'e' || c == b'm' {
                    what = c;
                    i += 1;
                } else if c == b'l' || c == b'r' {
                    align = c;
                    i += 1;
                } else if c.is_ascii_digit() || c == b'-' {
                    let neg = c == b'-';
                    if neg {
                        i += 1;
                    }
                    let mut n = 0i32;
                    while at(p, i).is_ascii_digit() {
                        n = n * 10 + i32::from(p[i] - b'0');
                        i += 1;
                    }
                    off = if neg { -n } else { n };
                } else {
                    i += 1;
                }
            }
            if at(p, i) == b':' {
                i += 1;
            }
            // copy_option_part: up to ',' (a backslash keeps a ',').
            let mut lead_end: Vec<u8> = Vec::new();
            while at(p, i) != 0 && at(p, i) != b',' {
                if at(p, i) == b'\\' && at(p, i + 1) == b',' {
                    i += 1;
                }
                lead_end.push(p[i]);
                i += 1;
            }
            while at(p, i) == b' ' || at(p, i) == b',' {
                i += 1;
            }
            let lead_end_len = lead_end.len();
            if what == b's' {
                lead_start = lead_end.clone();
                lead_start_len = lead_end_len;
                start_off = off;
                start_align = align;
            } else if what == b'm' {
                lead_middle = lead_end.clone();
                lead_middle_len = lead_end_len;
            } else if what == b'e' {
                if strneq(theline, &lead_middle, lead_middle_len)
                    && !strneq(theline, &lead_end, lead_end_len)
                {
                    done = true;
                    if self.cursor.lnum > 1 {
                        let prev = self.ml(self.cursor.lnum - 1);
                        let look = &prev[skipwhite(&prev, 0)..];
                        if strneq(look, &lead_start, lead_start_len) {
                            amount = self.get_indent_lnum(self.cursor.lnum - 1);
                        } else if strneq(look, &lead_middle, lead_middle_len) {
                            amount = self.get_indent_lnum(self.cursor.lnum - 1);
                            break;
                        } else {
                            let cl = self.ml(cp.lnum);
                            if !strneq(&cl[cp.col.min(cl.len())..], &lead_start, lead_start_len) {
                                continue;
                            }
                        }
                    }
                    if start_off != 0 {
                        amount += start_off;
                    } else if start_align == b'r' {
                        amount += lead_start.len() as i32 - lead_middle.len() as i32;
                    }
                    break;
                }
                if !strneq(theline, &lead_middle, lead_middle_len)
                    && strneq(theline, &lead_end, lead_end_len)
                {
                    amount = self.get_indent_lnum(self.cursor.lnum - 1);
                    if off != 0 {
                        amount += off;
                    } else if align == b'r' {
                        amount += lead_start.len() as i32 - lead_middle.len() as i32;
                    }
                    done = true;
                    break;
                }
            }
        }
        if done {
            return amount;
        }
        if at(theline, 0) == b'*' {
            return amount + 1;
        }
        amount = -1;
        let mut lnum = cur_curpos.lnum - 1;
        while lnum > cp.lnum {
            if !self.linewhite(lnum) {
                amount = self.get_indent_lnum(lnum);
                break;
            }
            lnum -= 1;
        }
        if amount == -1 {
            let start = self.ml(cp.lnum);
            let look = cp.col + 2;
            let mut col = cp.col;
            if self.cino.in_comment2 == 0 && at(&start, look) != 0 {
                col = skipwhite(&start, look);
            }
            amount = self.getvcol(cp.lnum, col);
            if self.cino.in_comment2 != 0 || at(&start, look) == 0 {
                amount += self.cino.in_comment;
            }
        }
        amount
    }

    /// The indent inside an unclosed `(` at `trypos`.
    fn paren_indent(&mut self, theline: &[u8], trypos: Pos, cur_curpos: Pos) -> i32 {
        let t0 = at(theline, 0);
        let mut our_paren_pos = trypos;
        let mut cur_amount = MAXCOL;
        let mut amount;
        if t0 == b')' && self.cino.paren_prev != 0 {
            amount = self.get_indent_lnum(self.cursor.lnum - 1);
        } else {
            amount = -1;
            let mut lnum = cur_curpos.lnum - 1;
            while lnum > our_paren_pos.lnum {
                let line = self.ml(lnum);
                let mut l = Lp {
                    off: skipwhite(&line, 0),
                    line,
                };
                if self.nocode(l.s(), l.off) {
                    lnum -= 1;
                    continue;
                }
                if self.ispreproc_cont(&mut l, &mut lnum, &mut amount) {
                    lnum -= 1;
                    continue;
                }
                self.cursor.lnum = lnum;
                if let Some(t) = self.ind_find_start_cors(None) {
                    lnum = t.lnum;
                    continue;
                }
                let maxp = self.corr_ind_maxparen(cur_curpos);
                if let Some(t) = self.find_match_paren(maxp)
                    && t == our_paren_pos
                {
                    amount = self.get_indent_lnum(lnum);
                    if t0 == b')' {
                        if our_paren_pos.lnum != lnum && cur_amount > amount {
                            cur_amount = amount;
                        }
                        amount = -1;
                    }
                    break;
                }
                lnum -= 1;
            }
        }

        if amount == -1 {
            let mut ignore_paren_col = 0usize;
            let mut is_if_for_while = false;
            if self.cino.if_for_while != 0 {
                let save = self.cursor;
                let mut outermost;
                let mut t = Some(our_paren_pos);
                loop {
                    outermost = t.expect("set");
                    self.cursor = outermost;
                    t = self.find_match_paren(self.cino.maxparen);
                    if !t.is_some_and(|t| t.lnum == outermost.lnum) {
                        break;
                    }
                }
                self.cursor = save;
                let line = self.ml(outermost.lnum);
                let mut c = outermost.col;
                is_if_for_while = cin_is_if_for_while_before_offset(&line, &mut c);
            }
            let (a, look) = self.skip_label(our_paren_pos.lnum);
            amount = a;
            let mut look = Lp {
                off: skipwhite(look.s(), look.off),
                line: look.line,
            };
            if look.c() == b'(' {
                let save_lnum = self.cursor.lnum;
                self.cursor.lnum = our_paren_pos.lnum;
                let look_col = look.off;
                self.cursor.col = look_col + 1;
                if let Some(t) = self.findmatchlimit(b')', 0, i64::from(self.cino.maxparen))
                    && t.lnum == our_paren_pos.lnum
                    && t.col < our_paren_pos.col
                {
                    ignore_paren_col = t.col + 1;
                }
                self.cursor.lnum = save_lnum;
                look = Lp {
                    line: self.ml(our_paren_pos.lnum),
                    off: look_col,
                };
            }
            let lookparen = look.c() == b'(';
            let noignore = self.cino.unclosed_noignore == 0 && lookparen && ignore_paren_col == 0;
            if t0 == b')' || (self.cino.unclosed == 0 && !is_if_for_while) || noignore {
                if t0 != b')' {
                    cur_amount = MAXCOL;
                    let l = self.ml(our_paren_pos.lnum);
                    if self.cino.unclosed_wrapped != 0 && self.ends_in(&l, b"(") {
                        let mut n = 1;
                        for col in 0..our_paren_pos.col {
                            match at(&l, col) {
                                b'(' | b'{' => n += 1,
                                b')' | b'}' if n > 1 => {
                                    n -= 1;
                                }
                                _ => {}
                            }
                        }
                        our_paren_pos.col = 0;
                        amount += n * self.cino.unclosed_wrapped;
                    } else if self.cino.unclosed_whiteok != 0 {
                        our_paren_pos.col += 1;
                    } else {
                        let mut col = our_paren_pos.col + 1;
                        while is_white(at(&l, col)) {
                            col += 1;
                        }
                        if at(&l, col) != 0 {
                            our_paren_pos.col = col;
                        } else {
                            our_paren_pos.col += 1;
                        }
                    }
                }
                if our_paren_pos.col > 0 {
                    let col = self.getvcol(our_paren_pos.lnum, our_paren_pos.col);
                    if cur_amount > col {
                        cur_amount = col;
                    }
                }
            }

            if t0 == b')' && self.cino.matching_paren != 0 {
                // Line up with the start of the matching paren's line.
            } else if (self.cino.unclosed == 0 && !is_if_for_while) || noignore {
                if cur_amount != MAXCOL {
                    amount = cur_amount;
                }
            } else {
                // One b_ind_unclosed2 for each '(' before ours.
                let line = self.ml(our_paren_pos.lnum);
                let mut col: Option<usize> = Some(our_paren_pos.col);
                while our_paren_pos.col > ignore_paren_col {
                    our_paren_pos.col -= 1;
                    match at(&line, our_paren_pos.col) {
                        b'(' => {
                            amount += self.cino.unclosed2;
                            col = Some(our_paren_pos.col);
                        }
                        b')' => {
                            amount -= self.cino.unclosed2;
                            col = None;
                        }
                        _ => {}
                    }
                }
                match col {
                    None => amount += self.cino.unclosed,
                    Some(col) => {
                        self.cursor = Pos {
                            lnum: our_paren_pos.lnum,
                            col,
                        };
                        if self
                            .find_match_paren_after_brace(self.cino.maxparen)
                            .is_some()
                        {
                            amount += self.cino.unclosed2;
                        } else if is_if_for_while {
                            amount += self.cino.if_for_while;
                        } else {
                            amount += self.cino.unclosed;
                        }
                    }
                }
                if cur_amount < amount {
                    amount = cur_amount;
                }
            }
        }
        if cin_iscomment(theline, 0) {
            amount += self.cino.comment;
        }
        amount
    }

    /// The indent inside the `{` at `brace`. `Ok` is final (Vim's `goto theend`), `Err`
    /// still gets the comment and label adjustments.
    fn brace_indent(&mut self, theline: &[u8], brace: Pos, cur_curpos: Pos) -> Result<i32, i32> {
        let t0 = at(theline, 0);
        let trypos_brace = brace;
        let mut ourscope = brace.lnum;
        let start = self.ml(ourscope);
        let mut amount;
        let start_brace;
        let mut lookfor_cpp_namespace = false;
        let look = skipwhite(&start, 0);
        if at(&start, look) == b'{' {
            amount = self.getvcol(brace.lnum, brace.col);
            start_brace = if at(&start, 0) == b'{' {
                BRACE_IN_COL0
            } else {
                BRACE_AT_START
            };
        } else {
            self.cursor.lnum = ourscope;
            let mut lnum = ourscope;
            if self.find_last_paren(&start, b'(', b')')
                && let Some(t) = self.find_match_paren(self.cino.maxparen)
            {
                lnum = t.lnum;
            }
            let cl = self.cur_line();
            if (self.cino.js != 0 || self.cino.keep_case_label != 0)
                && self.iscase(&cl, skipwhite(&cl, 0), false)
            {
                amount = self.get_indent();
            } else if self.cino.js != 0 {
                amount = self.get_indent_lnum(lnum);
            } else {
                amount = self.skip_label(lnum).0;
            }
            start_brace = BRACE_AT_END;
        }

        let mut js_cur_has_key = self.cino.js != 0 && self.has_js_key(theline, 0);

        if t0 == b'}' {
            amount += self.cino.close_extra;
            return Err(amount);
        }

        let mut lookfor = LOOKFOR_INITIAL;
        if self.cin_iselse(theline, 0) {
            lookfor = LOOKFOR_IF;
        } else if self.iswhileofdo(theline, 0, cur_curpos.lnum) {
            lookfor = LOOKFOR_DO;
        }
        if lookfor != LOOKFOR_INITIAL {
            self.cursor.lnum = cur_curpos.lnum;
            if self.find_match(lookfor, ourscope) {
                return Ok(self.get_indent());
            }
        }

        if start_brace == BRACE_IN_COL0 {
            amount = self.cino.open_left_imag;
            lookfor_cpp_namespace = true;
        } else if start_brace == BRACE_AT_END {
            amount += self.cino.open_imag;
            let cl = self.cur_line();
            let l = skipwhite(&cl, 0);
            if self.is_cpp_namespace(&cl, l) {
                amount += self.cino.cpp_namespace;
            } else if self.is_cpp_extern_c(&cl, l) {
                amount += self.cino.cpp_extern_c;
            }
        } else {
            amount -= self.cino.open_extra;
            if amount < 0 {
                amount = 0;
            }
        }

        let mut lookfor_break = false;
        if self.iscase(theline, 0, false) {
            lookfor = LOOKFOR_CASE;
            amount += self.cino.case;
        } else if self.isscopedecl(theline, 0) {
            lookfor = LOOKFOR_SCOPEDECL;
            amount += self.cino.scopedecl;
        } else {
            if self.cino.case_break != 0 && cin_isbreak(theline, 0) {
                lookfor_break = true;
            }
            lookfor = LOOKFOR_INITIAL;
            amount += self.cino.level;
        }
        let mut scope_amount = amount;
        let mut whilelevel = 0;
        let mut cont_amount = 0;
        let mut added_to_amount = 0;
        let mut raw_string_start = 0usize;
        let mut ind_continuation = self.cino.continuation;
        let mut cache = BaseclassCache {
            found: false,
            lnum: usize::MAX,
            col: 0,
        };
        let mut cur_amount;

        self.cursor = cur_curpos;
        'search: loop {
            self.cursor.lnum -= 1;
            self.cursor.col = 0;

            if self.cursor.lnum <= ourscope {
                // Reached the start of the scope.
                if lookfor == LOOKFOR_ENUM_OR_INIT {
                    if self.cursor.lnum == 0
                        || (self.cursor.lnum as i64)
                            < ourscope as i64 - i64::from(self.cino.maxparen)
                    {
                        if cont_amount > 0 {
                            amount = cont_amount;
                        } else if self.cino.js == 0 {
                            amount += ind_continuation;
                        }
                        break;
                    }
                    if let Some(t) = self.ind_find_start_cors(None) {
                        self.cursor.lnum = t.lnum + 1;
                        self.cursor.col = 0;
                        continue;
                    }
                    let mut l = Lp {
                        line: self.cur_line(),
                        off: 0,
                    };
                    let mut lnum = self.cursor.lnum;
                    let pre = self.ispreproc_cont(&mut l, &mut lnum, &mut amount);
                    self.cursor.lnum = lnum;
                    if pre || self.nocode(l.s(), l.off) {
                        continue;
                    }
                    let terminated = self.isterminated(l.s(), l.off, false, true);
                    let cl = self.cursor.lnum;
                    if start_brace != BRACE_IN_COL0 || !self.isfuncdecl(Some(&mut l), cl, 0) {
                        if terminated == b',' {
                            break;
                        }
                        if terminated != b';' && self.isinit() {
                            break;
                        }
                        if terminated == 0 || terminated == b'{' {
                            continue;
                        }
                    }
                    if terminated != b';' {
                        let mut trypos = None;
                        let ls = l.line.clone();
                        if self.find_last_paren(&ls, b'(', b')') {
                            trypos = self.find_match_paren(self.cino.maxparen);
                        }
                        if trypos.is_none() && self.find_last_paren(&ls, b'{', b'}') {
                            trypos = self.find_start_brace();
                        }
                        if let Some(t) = trypos {
                            self.cursor.lnum = t.lnum + 1;
                            self.cursor.col = 0;
                            continue;
                        }
                    }
                    if cont_amount > 0 {
                        amount = cont_amount;
                    } else {
                        amount += ind_continuation;
                    }
                } else if lookfor == LOOKFOR_UNTERM {
                    if cont_amount > 0 {
                        amount = cont_amount;
                    } else {
                        amount += ind_continuation;
                    }
                } else {
                    if lookfor != LOOKFOR_TERM
                        && lookfor != LOOKFOR_CPP_BASECLASS
                        && lookfor != LOOKFOR_COMMA
                    {
                        amount = scope_amount;
                        if t0 == b'{' {
                            amount += self.cino.open_extra;
                            added_to_amount = self.cino.open_extra;
                        }
                    }
                    if lookfor_cpp_namespace {
                        if self.cursor.lnum == ourscope {
                            continue;
                        }
                        if self.cursor.lnum == 0 || (self.cursor.lnum as i64) < ourscope as i64 - 20
                        {
                            break;
                        }
                        if let Some(t) = self.ind_find_start_cors(None) {
                            self.cursor.lnum = t.lnum + 1;
                            self.cursor.col = 0;
                            continue;
                        }
                        let mut l = Lp {
                            line: self.cur_line(),
                            off: 0,
                        };
                        let mut lnum = self.cursor.lnum;
                        let pre = self.ispreproc_cont(&mut l, &mut lnum, &mut amount);
                        self.cursor.lnum = lnum;
                        if pre {
                            continue;
                        }
                        if self.is_cpp_namespace(l.s(), l.off) {
                            amount += self.cino.cpp_namespace - added_to_amount;
                            break;
                        } else if self.is_cpp_extern_c(l.s(), l.off) {
                            amount += self.cino.cpp_extern_c - added_to_amount;
                            break;
                        }
                        if self.nocode(l.s(), l.off) {
                            continue;
                        }
                    }
                }
                break;
            }

            // In a comment or raw string: skip to its start.
            if let Some(t) = self.ind_find_start_cors(Some(&mut raw_string_start)) {
                self.cursor.lnum = t.lnum + 1;
                self.cursor.col = 0;
                continue;
            }

            let l = self.cur_line();

            let iscase = self.iscase(&l, 0, false);
            if iscase || self.isscopedecl(&l, 0) {
                if lookfor == LOOKFOR_CPP_BASECLASS {
                    break;
                }
                if whilelevel > 0 {
                    continue;
                }
                if lookfor == LOOKFOR_UNTERM || lookfor == LOOKFOR_ENUM_OR_INIT {
                    if cont_amount > 0 {
                        amount = cont_amount;
                    } else {
                        amount += ind_continuation;
                    }
                    break;
                }
                if (iscase && lookfor == LOOKFOR_CASE)
                    || (iscase && lookfor_break)
                    || (!iscase && lookfor == LOOKFOR_SCOPEDECL)
                {
                    match self.find_start_brace() {
                        Some(t) if t.lnum != ourscope => continue,
                        _ => {
                            amount = self.get_indent();
                            break;
                        }
                    }
                }
                let n = self.get_indent_nolabel(self.cursor.lnum);
                if lookfor == LOOKFOR_TERM {
                    if n != 0 {
                        amount = n;
                    }
                    if !lookfor_break {
                        break;
                    }
                }
                if n != 0 {
                    amount = n;
                    let cl = self.cur_line();
                    if let Some(p) = self.after_label(&cl)
                        && self.is_cinword(&cl, p)
                    {
                        if t0 == b'{' {
                            amount += self.cino.open_extra;
                        } else {
                            amount += self.cino.level + self.cino.no_brace;
                        }
                    }
                    break;
                }
                scope_amount = self.get_indent()
                    + if iscase {
                        self.cino.case_code
                    } else {
                        self.cino.scopedecl_code
                    };
                lookfor = if self.cino.case_break != 0 {
                    LOOKFOR_NOBREAK
                } else {
                    LOOKFOR_ANY
                };
                continue;
            }

            if lookfor == LOOKFOR_CASE || lookfor == LOOKFOR_SCOPEDECL {
                if self.find_last_paren(&l, b'{', b'}')
                    && let Some(t) = self.find_start_brace()
                {
                    self.cursor.lnum = t.lnum + 1;
                    self.cursor.col = 0;
                }
                continue;
            }

            // Jump labels with nothing after them.
            if self.cino.js == 0 && self.cin_islabel() {
                let cl = self.cur_line();
                match self.after_label(&cl) {
                    None => continue,
                    Some(p) if self.nocode(&cl, p) => continue,
                    _ => {}
                }
            }

            let mut l = Lp {
                line: self.cur_line(),
                off: 0,
            };
            let mut lnum = self.cursor.lnum;
            let pre = self.ispreproc_cont(&mut l, &mut lnum, &mut amount);
            self.cursor.lnum = lnum;
            if pre || self.nocode(l.s(), l.off) {
                continue;
            }

            // A C++ base class declaration or constructor initialization?
            let mut n = false;
            if lookfor != LOOKFOR_TERM && self.cino.cpp_baseclass > 0 {
                n = self.is_cpp_baseclass(&mut cache);
                l = Lp {
                    line: self.cur_line(),
                    off: 0,
                };
            }
            if n {
                if lookfor == LOOKFOR_UNTERM {
                    if cont_amount > 0 {
                        amount = cont_amount;
                    } else {
                        amount += ind_continuation;
                    }
                } else if t0 == b'{' {
                    lookfor = LOOKFOR_UNTERM;
                    ind_continuation = 0;
                    continue;
                } else {
                    amount = self.get_baseclass_amount(cache.col);
                }
                break;
            } else if lookfor == LOOKFOR_CPP_BASECLASS {
                if self.isterminated(l.s(), l.off, true, false) != 0 {
                    break;
                }
                continue;
            }

            let terminated = self.isterminated(l.s(), l.off, false, true);

            if js_cur_has_key {
                js_cur_has_key = false;
                if self.cino.js != 0 && terminated == b',' {
                    lookfor = LOOKFOR_JS_KEY;
                }
            }
            if lookfor == LOOKFOR_JS_KEY && self.has_js_key(l.s(), l.off) {
                amount = self.get_indent();
                break;
            }
            if lookfor == LOOKFOR_COMMA {
                if trypos_brace.lnum >= self.cursor.lnum {
                    break;
                }
                if terminated == b',' {
                    break;
                }
                amount = self.get_indent();
                if self.cursor.lnum - 1 == ourscope {
                    break;
                }
            }

            if terminated == 0 || (lookfor != LOOKFOR_UNTERM && terminated == b',') {
                let ls = &l.s()[l.off..];
                if lookfor != LOOKFOR_ENUM_OR_INIT
                    && (at(ls, skipwhite(ls, 0)) == b'[' || ls.last() == Some(&b'['))
                {
                    amount += ind_continuation;
                }
                // Back to the line that starts a paren thing.
                let lline = l.line.clone();
                self.find_last_paren(&lline[l.off..], b'(', b')');
                let maxp = self.corr_ind_maxparen(cur_curpos);
                let mut trypos = self.find_match_paren(maxp);
                if let Some(t) = trypos
                    && (t.lnum < trypos_brace.lnum
                        || (t.lnum == trypos_brace.lnum && t.col < trypos_brace.col))
                {
                    trypos = None;
                }
                if trypos.is_none() && terminated == b',' {
                    let cl = self.cur_line();
                    if self.find_last_paren(&cl, b'{', b'}') {
                        trypos = self.find_start_brace();
                    }
                }
                if let Some(t) = trypos {
                    self.cursor = t;
                    let cl = self.cur_line();
                    if self.iscase(&cl, 0, false) || self.isscopedecl(&cl, 0) {
                        self.cursor.lnum += 1;
                        self.cursor.col = 0;
                        continue;
                    }
                }
                if terminated == b',' {
                    while self.cursor.lnum > 1 {
                        if !ends_in_backslash(&self.ml(self.cursor.lnum - 1)) {
                            break;
                        }
                        self.cursor.lnum -= 1;
                        self.cursor.col = 0;
                    }
                }
                let lp;
                if self.cino.js != 0 {
                    cur_amount = self.get_indent();
                    lp = Lp {
                        line: self.cur_line(),
                        off: 0,
                    };
                } else {
                    let (a, p) = self.skip_label(self.cursor.lnum);
                    cur_amount = a;
                    lp = p;
                }
                if terminated != b',' && lookfor != LOOKFOR_TERM && t0 == b'{' {
                    amount = cur_amount;
                    if at(lp.s(), skipwhite(lp.s(), lp.off)) != b'{' {
                        amount += self.cino.open_extra;
                    }
                    if self.cino.cpp_baseclass != 0 && self.cino.js == 0 {
                        lookfor = LOOKFOR_CPP_BASECLASS;
                        continue;
                    }
                    break;
                }

                if self.is_cinword(lp.s(), lp.off)
                    || self.cin_iselse(lp.s(), skipwhite(lp.s(), lp.off))
                {
                    if lookfor == LOOKFOR_UNTERM || lookfor == LOOKFOR_ENUM_OR_INIT {
                        if cont_amount > 0 {
                            amount = cont_amount;
                        } else {
                            amount += ind_continuation;
                        }
                        break;
                    }
                    amount = cur_amount;
                    if t0 == b'{' {
                        amount += self.cino.open_extra;
                    }
                    if lookfor != LOOKFOR_TERM {
                        amount += self.cino.level + self.cino.no_brace;
                        break;
                    }
                    let cl = self.cur_line();
                    let l2 = skipwhite(&cl, 0);
                    if cin_isdo(&cl, l2) {
                        if whilelevel == 0 {
                            break;
                        }
                        whilelevel -= 1;
                    }
                    if self.cin_iselse(&cl, l2) && whilelevel == 0 {
                        if at(&cl, l2) == b'}' {
                            self.cursor.col = l2 + 1;
                        }
                        match self.find_start_brace() {
                            None => break,
                            Some(t) => {
                                if !self.find_match(LOOKFOR_IF, t.lnum) {
                                    break;
                                }
                            }
                        }
                    }
                } else {
                    if lookfor == LOOKFOR_UNTERM {
                        if terminated == b',' {
                            amount += ind_continuation;
                        }
                        break;
                    }
                    if lookfor == LOOKFOR_ENUM_OR_INIT {
                        if terminated == b',' {
                            if self.cino.cpp_baseclass == 0 {
                                break;
                            }
                            lookfor = LOOKFOR_CPP_BASECLASS;
                            continue;
                        }
                        if amount > cur_amount {
                            amount = cur_amount;
                        }
                    } else {
                        let cl = self.cur_line();
                        amount = cur_amount;
                        let n = cl.len();
                        if self.cino.js != 0
                            && terminated == b','
                            && (at(&cl, skipwhite(&cl, 0)) == b']' || (n >= 2 && cl[n - 2] == b']'))
                        {
                            break;
                        }
                        if lookfor == LOOKFOR_INITIAL && terminated == b',' {
                            if self.cino.js != 0 {
                                if cin_iscomment(&cl, skipwhite(&cl, 0)) {
                                    break;
                                }
                                lookfor = LOOKFOR_COMMA;
                                if let Some(t) = self.find_match_char(b'[', self.cino.maxparen) {
                                    if t.lnum == self.cursor.lnum - 1 {
                                        break;
                                    }
                                    ourscope = t.lnum;
                                }
                            } else {
                                lookfor = LOOKFOR_ENUM_OR_INIT;
                                cont_amount = self.first_id_amount();
                            }
                        } else {
                            if lookfor == LOOKFOR_INITIAL && ends_in_backslash(&cl) {
                                cont_amount = self.get_equal_amount(self.cursor.lnum);
                            }
                            if lookfor != LOOKFOR_TERM
                                && lookfor != LOOKFOR_JS_KEY
                                && lookfor != LOOKFOR_COMMA
                                && raw_string_start != self.cursor.lnum
                            {
                                lookfor = LOOKFOR_UNTERM;
                            }
                        }
                    }
                }
            } else if self.iswhileofdo_end(terminated) {
                if lookfor == LOOKFOR_UNTERM || lookfor == LOOKFOR_ENUM_OR_INIT {
                    if cont_amount > 0 {
                        amount = cont_amount;
                    } else {
                        amount += ind_continuation;
                    }
                    break;
                }
                if whilelevel == 0 {
                    lookfor = LOOKFOR_TERM;
                    amount = self.get_indent();
                    if t0 == b'{' {
                        amount += self.cino.open_extra;
                    }
                }
                whilelevel += 1;
            } else {
                // After a "normal" statement.
                let cl = self.cur_line();
                if lookfor == LOOKFOR_NOBREAK && cin_isbreak(&cl, skipwhite(&cl, 0)) {
                    lookfor = LOOKFOR_ANY;
                    continue;
                }
                if whilelevel > 0 {
                    let lc = self.skipcomment(&cl, 0);
                    if cin_isdo(&cl, lc) {
                        amount = self.get_indent();
                        whilelevel -= 1;
                        continue;
                    }
                }
                if lookfor == LOOKFOR_UNTERM || lookfor == LOOKFOR_ENUM_OR_INIT {
                    if cont_amount > 0 {
                        amount = cont_amount;
                    } else {
                        amount += ind_continuation;
                    }
                    break;
                }
                if lookfor == LOOKFOR_TERM {
                    if !lookfor_break && whilelevel == 0 {
                        break;
                    }
                } else {
                    // Vim's `term_again`.
                    loop {
                        let cl = self.cur_line();
                        if self.find_last_paren(&cl, b'(', b')')
                            && let Some(t) = self.find_match_paren(self.cino.maxparen)
                        {
                            self.cursor = t;
                            let cl = self.cur_line();
                            if self.iscase(&cl, 0, false) || self.isscopedecl(&cl, 0) {
                                self.cursor.lnum += 1;
                                self.cursor.col = 0;
                                continue 'search;
                            }
                        }
                        let cl = self.cur_line();
                        let iscase = self.cino.keep_case_label != 0 && self.iscase(&cl, 0, false);
                        let (a, lp) = self.skip_label(self.cursor.lnum);
                        amount = a;
                        if t0 == b'{' {
                            amount += self.cino.open_extra;
                        }
                        let lw = skipwhite(lp.s(), lp.off);
                        if at(lp.s(), lw) == b'{' {
                            amount -= self.cino.open_extra;
                        }
                        lookfor = if iscase { LOOKFOR_ANY } else { LOOKFOR_TERM };
                        if lookfor == LOOKFOR_TERM
                            && at(lp.s(), lw) != b'}'
                            && self.cin_iselse(lp.s(), lw)
                            && whilelevel == 0
                        {
                            match self.find_start_brace() {
                                None => break 'search,
                                Some(t) => {
                                    if !self.find_match(LOOKFOR_IF, t.lnum) {
                                        break 'search;
                                    }
                                }
                            }
                            continue 'search;
                        }
                        let cl = self.cur_line();
                        if self.find_last_paren(&cl, b'{', b'}')
                            && let Some(t) = self.find_start_brace()
                        {
                            self.cursor = t;
                            let cl = self.cur_line();
                            let lc = self.skipcomment(&cl, 0);
                            if at(&cl, lc) == b'}' || !self.cin_iselse(&cl, lc) {
                                continue;
                            }
                            self.cursor.lnum += 1;
                            self.cursor.col = 0;
                        }
                        break;
                    }
                }
            }
        }
        Err(amount)
    }

    /// The indent outside any parentheses or braces.
    fn top_level_indent(&mut self, theline: &[u8], cur_curpos: Pos) -> i32 {
        let t0 = at(theline, 0);
        let ind_continuation = self.cino.continuation;
        let mut amount;
        let mut cache = BaseclassCache {
            found: false,
            lnum: usize::MAX,
            col: 0,
        };
        if t0 == b'{' {
            return self.cino.first_open;
        }
        // The next line is a function declaration: this is its type.
        if cur_curpos.lnum < self.line_count()
            && !self.nocode(theline, 0)
            && !theline.contains(&b'{')
            && !theline.contains(&b'}')
            && !self.ends_in(theline, b":")
            && !self.ends_in(theline, b",")
            && self.isfuncdecl(None, cur_curpos.lnum + 1, cur_curpos.lnum + 1)
            && self.isterminated(theline, 0, false, true) == 0
        {
            return self.cino.func_type;
        }

        amount = 0;
        self.cursor = cur_curpos;
        while self.cursor.lnum > 1 {
            self.cursor.lnum -= 1;
            self.cursor.col = 0;
            if let Some(t) = self.ind_find_start_cors(None) {
                self.cursor.lnum = t.lnum + 1;
                self.cursor.col = 0;
                continue;
            }
            if self.cino.cpp_baseclass != 0 && self.is_cpp_baseclass(&mut cache) {
                amount = self.get_baseclass_amount(cache.col);
                break;
            }
            let mut l = Lp {
                line: self.cur_line(),
                off: 0,
            };
            let mut lnum = self.cursor.lnum;
            let pre = self.ispreproc_cont(&mut l, &mut lnum, &mut amount);
            self.cursor.lnum = lnum;
            if pre {
                continue;
            }
            if self.nocode(l.s(), l.off) {
                continue;
            }
            let ls: Rc<[u8]> = Rc::from(&l.s()[l.off..]);
            let backslash = ends_in_backslash(&ls);
            if self.ends_in(&ls, b",") || backslash {
                if self.find_last_paren(&ls, b'(', b')')
                    && let Some(t) = self.find_match_paren(self.cino.maxparen)
                {
                    self.cursor = t;
                }
                // In C `n` is the last character only when the line ends in a backslash.
                while !backslash && self.cursor.lnum > 1 {
                    if !ends_in_backslash(&self.ml(self.cursor.lnum - 1)) {
                        break;
                    }
                    self.cursor.lnum -= 1;
                    self.cursor.col = 0;
                }
                amount = self.get_indent();
                if amount == 0 {
                    amount = self.first_id_amount();
                }
                if amount == 0 {
                    amount = ind_continuation;
                }
                break;
            }
            if self.isfuncdecl(None, cur_curpos.lnum, 0) {
                break;
            }
            let ls = self.cur_line();
            if at(&ls, skipwhite(&ls, 0)) == b'}' {
                break;
            }
            if self.ends_in(&ls, b"};") {
                break;
            }
            if self.ends_in(&ls, b"[") {
                amount = self.get_indent() + ind_continuation;
                break;
            }
            let look = skipwhite(&ls, 0);
            if at(&ls, look) == b';' && self.nocode(&ls, look + 1) {
                let save = self.cursor;
                let mut look_line = ls.clone();
                while self.cursor.lnum > 1 {
                    self.cursor.lnum -= 1;
                    let mut lp = Lp {
                        line: self.cur_line(),
                        off: 0,
                    };
                    let mut lnum = self.cursor.lnum;
                    let pre = self.ispreproc_cont(&mut lp, &mut lnum, &mut amount);
                    self.cursor.lnum = lnum;
                    look_line = lp.line.clone();
                    if !(self.nocode(&look_line, 0) || pre) {
                        break;
                    }
                }
                if self.cursor.lnum > 0 && self.ends_in(&look_line, b"}") {
                    break;
                }
                self.cursor = save;
            }
            let mut lp = Lp {
                line: self.cur_line(),
                off: 0,
            };
            let cl = self.cursor.lnum;
            if self.isfuncdecl(Some(&mut lp), cl, 0) {
                amount = self.cino.param;
                break;
            }
            let mut ls = lp.line.clone();
            if self.ends_in(&ls, b";") {
                let pl = self.ml(self.cursor.lnum - 1);
                if self.ends_in(&pl, b",") || ends_in_backslash(&pl) {
                    break;
                }
                ls = self.cur_line();
            }
            self.find_last_paren(&ls, b'(', b')');
            if let Some(t) = self.find_match_paren(self.cino.maxparen) {
                self.cursor = t;
            }
            amount = self.get_indent();
            break;
        }

        if cin_iscomment(theline, 0) {
            amount += self.cino.comment;
        }
        if cur_curpos.lnum > 1 && ends_in_backslash(&self.ml(cur_curpos.lnum - 1)) {
            let cur_amount = self.get_equal_amount(cur_curpos.lnum - 1);
            if cur_amount > 0 {
                amount = cur_amount;
            } else if cur_amount == 0 {
                amount += ind_continuation;
            }
        }
        amount
    }
}

/// The C indent for `ctx.lnum`, `None` inside a raw string.
pub(super) fn get_c_indent(ctx: &Ctx) -> Option<usize> {
    Cin::new(ctx).get_c_indent().map(|a| a as usize)
}

/// Vim's `cin_iscase`: the line is a `case x:` or `default:` label.
pub(super) fn is_case(ctx: &Ctx, line: &str, strict: bool) -> bool {
    Cin::new(ctx).iscase(line.as_bytes(), 0, strict)
}

/// Vim's `cin_isscopedecl`: `public:`, `private:` or `protected:`.
pub(super) fn is_scopedecl(ctx: &Ctx, line: &str) -> bool {
    Cin::new(ctx).isscopedecl(line.as_bytes(), 0)
}

/// Vim's `cin_islabel` for the cursor line, whose text is `line`: a goto label.
pub(super) fn is_label(ctx: &Ctx, line: &str) -> bool {
    Cin::new(ctx).islabel_line(line.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cinoptions() {
        let c = parse_cino("L0,(s,Ws,J1,j1,m1", 4);
        assert_eq!(
            (
                c.jump_label,
                c.unclosed,
                c.unclosed_wrapped,
                c.js,
                c.java,
                c.matching_paren
            ),
            (0, 4, 4, 1, 1, 1)
        );
        let c = parse_cino(">2s,:-1,(.5s", 8);
        assert_eq!((c.level, c.case, c.unclosed), (16, -1, 4));
    }

    #[test]
    fn strings_and_comments() {
        assert!(is_pos_in_string(b"x = \"a(b\";", 6));
        assert!(!is_pos_in_string(b"x = \"a(b\";", 2));
        // Like Vim's, the character after the closing quote counts as inside.
        assert!(is_pos_in_string(b"x = \"a(b\";", 9));
        assert_eq!(check_linecomment(b"a = \"//\"; // x"), 10);
        assert_eq!(check_linecomment(b"a = 1;"), NOCOL);
    }
}
