//! `GetRustIndent()`, ported from Neovim's `runtime/indent/rust.vim`.
//!
//! In Neovim, Rust buffers are highlighted with the regex syntax, and the script asks it whether
//! a position is in a comment or string (`rustCommentLine`, `rustString`, …). flux asks the
//! tree-sitter captures instead.

use super::{Ctx, SynKind, cindent};

/// `synIDattr(synID(line, col, 1), "name") =~? 'Comment\|Todo'`, `col` a byte column.
fn in_comment(ctx: &Ctx, line: usize, col: usize) -> bool {
    ctx.syn_kind(line, col) == SynKind::Comment
}

/// `s:is_string_comment`: the syntax stack has `rustString` or a `rustComment…` item (a
/// character literal is neither).
fn is_string_comment(ctx: &Ctx, line: usize, col: usize) -> bool {
    ctx.captures_at(line, col)
        .iter()
        .any(|c| c.contains("comment") || c.starts_with("string"))
}

/// `s:get_line_trimmed`: line `n` without a trailing comment. The script's
/// `substitute(line, "\s*$", "", "")` is in a double-quoted string, where `"\s"` is just `s`,
/// so it removes trailing `s` characters, not white space; this does the same.
fn get_line_trimmed(ctx: &Ctx, n: Option<usize>) -> String {
    let Some(n) = n else {
        return String::new();
    };
    let mut line = ctx.line(n).into_owned();
    let len = line.len();
    if len > 0 && in_comment(ctx, n, len - 1) {
        let (mut min, mut max) = (1, len);
        while min < max {
            let col = (min + max) / 2;
            if in_comment(ctx, n, col - 1) {
                max = col;
            } else {
                min = col + 1;
            }
        }
        line.truncate(min - 1);
    }
    line.trim_end_matches('s').to_string()
}

/// Vim's `\s`.
fn is_blank(c: char) -> bool {
    c == ' ' || c == '\t'
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn skip_blanks(s: &str) -> &str {
    s.trim_start_matches(is_blank)
}

/// `line =~# '\V\^\s\*WORD\s\*\$'`.
fn standalone(line: &str, word: &str) -> bool {
    skip_blanks(line)
        .strip_prefix(word)
        .is_some_and(|r| r.chars().all(is_blank))
}

/// `line =~# '^\s*WORD\s'`.
fn starts_with_word(line: &str, word: &str) -> bool {
    skip_blanks(line)
        .strip_prefix(word)
        .is_some_and(|r| r.starts_with(is_blank))
}

/// `searchpos('\<\(fn\|if\)\>', 'bW')` from `(line, col)`: the closest `fn` or `if` word that
/// starts before the position.
fn search_fn_if_back(ctx: &Ctx, line: usize, col: usize) -> Option<(usize, usize)> {
    for l in (0..=line).rev() {
        let s = ctx.line(l);
        let limit = if l == line { col } else { usize::MAX };
        let mut best = None;
        for (i, _) in s.char_indices() {
            if i >= limit {
                break;
            }
            if !(s[i..].starts_with("fn") || s[i..].starts_with("if")) {
                continue;
            }
            let before_ok = !s[..i].chars().last().is_some_and(is_word);
            let after_ok = !s[i + 2..].chars().next().is_some_and(is_word);
            if before_ok && after_ok {
                best = Some(i);
            }
        }
        if let Some(i) = best {
            return Some((l, i));
        }
    }
    None
}

/// `searchpair('{\|(', '', '}\|)', 'nbW', 's:is_string_comment(...)')` from the start of line
/// `line`: the line of the unmatched `{` or `(` before it.
fn scope_start(ctx: &Ctx, line: usize) -> Option<usize> {
    let mut depth = 0;
    for l in (0..line).rev() {
        let s = ctx.line(l);
        for (i, c) in s.char_indices().rev() {
            if !matches!(c, '{' | '(' | '}' | ')') || is_string_comment(ctx, l, i) {
                continue;
            }
            if matches!(c, '}' | ')') {
                depth += 1;
            } else if depth == 0 {
                return Some(l);
            } else {
                depth -= 1;
            }
        }
    }
    None
}

/// `prevline =~# '([^()]\+,$'`.
fn open_paren_args_comma(s: &str) -> bool {
    let Some(body) = s.strip_suffix(',') else {
        return false;
    };
    match body.rfind(['(', ')']) {
        Some(i) => body.as_bytes()[i] == b'(' && i + 1 < body.len(),
        None => false,
    }
}

/// `line =~# '^\s*\S\+\s*=>'`.
fn is_match_arm(s: &str) -> bool {
    let rest = skip_blanks(s);
    let ends = rest
        .char_indices()
        .skip(1)
        .map(|(i, _)| i)
        .chain(std::iter::once(rest.len()));
    for k in ends {
        if rest[..k].contains(is_blank) {
            break;
        }
        if skip_blanks(&rest[k..]).starts_with("=>") {
            return true;
        }
    }
    false
}

/// Whether line `line` starts inside a string literal that began on an earlier line (where
/// Vim's syntax has `rustString` at the first column; a string that starts at the first column
/// has `rustStringDelimiter` there). Found by lexing from the top of the buffer, as tree-sitter
/// captures don't tell a string's delimiter from its contents.
fn starts_in_string(ctx: &Ctx, line: usize) -> bool {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Code,
        Str,
        Raw(usize),
        Block(usize),
    }
    let mut state = State::Code;
    for l in 0..line {
        let s = ctx.line(l);
        let b = s.as_bytes();
        let at = |i: usize| b.get(i).copied().unwrap_or(0);
        let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80;
        let mut i = 0;
        while i < b.len() {
            match state {
                State::Str => {
                    if b[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if b[i] == b'"' {
                        state = State::Code;
                    }
                    i += 1;
                }
                State::Raw(n) => {
                    if b[i] == b'"' && (1..=n).all(|k| at(i + k) == b'#') {
                        state = State::Code;
                        i += n + 1;
                    } else {
                        i += 1;
                    }
                }
                State::Block(d) => {
                    if b[i] == b'/' && at(i + 1) == b'*' {
                        state = State::Block(d + 1);
                        i += 2;
                    } else if b[i] == b'*' && at(i + 1) == b'/' {
                        state = if d == 1 {
                            State::Code
                        } else {
                            State::Block(d - 1)
                        };
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                State::Code => {
                    let c = b[i];
                    if c == b'/' && at(i + 1) == b'/' {
                        break;
                    } else if c == b'/' && at(i + 1) == b'*' {
                        state = State::Block(1);
                        i += 2;
                    } else if c == b'"' {
                        state = State::Str;
                        i += 1;
                    } else if (c == b'r' || (c == b'b' && at(i + 1) == b'r'))
                        && (i == 0 || !ident(b[i - 1]))
                    {
                        let mut j = i + if c == b'b' { 2 } else { 1 };
                        let mut hashes = 0;
                        while at(j) == b'#' {
                            hashes += 1;
                            j += 1;
                        }
                        if at(j) == b'"' {
                            state = State::Raw(hashes);
                            i = j + 1;
                        } else {
                            i += 1;
                        }
                    } else if c == b'\'' {
                        if at(i + 1) == b'\\' {
                            // An escaped character literal: to its closing quote.
                            let mut j = i + 2;
                            while j < b.len() && b[j] != b'\'' {
                                j += 1;
                            }
                            i = j + 1;
                        } else {
                            let next = s[i + 1..].chars().next().map_or(0, char::len_utf8);
                            if next > 0 && at(i + 1 + next) == b'\'' {
                                i += next + 2;
                            } else {
                                // A lifetime or label.
                                i += 1;
                            }
                        }
                    } else {
                        i += 1;
                    }
                }
            }
        }
    }
    matches!(state, State::Str | State::Raw(_))
}

/// The indent for `ctx.lnum`, or `None` to keep it.
pub(super) fn indent(ctx: &Ctx) -> Option<usize> {
    let lnum = ctx.lnum;
    let line = ctx.line(lnum).into_owned();

    // The syntax item at the first column.
    if starts_in_string(ctx, lnum) {
        return None;
    }
    if in_comment(ctx, lnum, 0) && !skip_blanks(&line).starts_with("/*") {
        return cindent::get_c_indent(ctx);
    }

    // Vim's `indent()` of line 0 (no line) is -1, which keeps the indent.
    let indent_of = |n: Option<usize>| n.map(|n| ctx.indent(n));

    let mut prevlinenum = lnum.checked_sub(1).and_then(|l| ctx.prevnonblank(l));
    let mut prevline = get_line_trimmed(ctx, prevlinenum);
    while prevlinenum.is_some_and(|p| p > 0) && !prevline.chars().any(|c| !is_blank(c)) {
        prevlinenum = prevlinenum
            .and_then(|p| p.checked_sub(1))
            .and_then(|p| ctx.prevnonblank(p));
        prevline = get_line_trimmed(ctx, prevlinenum);
    }

    let standalone_open = standalone(&line, "{");
    let standalone_close = standalone(&line, "}");
    let standalone_where = standalone(&line, "where");
    if standalone_open || standalone_close || standalone_where {
        let mut pos = (lnum, ctx.cursor_col.unwrap_or(0));
        let mut found = None;
        for _ in 0..10 {
            let Some(p) = search_fn_if_back(ctx, pos.0, pos.1) else {
                break;
            };
            if !is_string_comment(ctx, p.0, p.1) {
                found = Some(p);
                break;
            }
            pos = p;
        }
        if let Some((found_line, col)) = found {
            let (mut opens, mut closes) = (0, 0);
            let mut search_line = ctx.line(found_line)[col..].to_string();
            let mut i = found_line;
            while i < lnum {
                opens += search_line.matches('{').count();
                closes += search_line.matches('}').count();
                i += 1;
                search_line = ctx.line(i).into_owned();
            }
            if standalone_open || standalone_where {
                if opens == closes {
                    return Some(ctx.indent(found_line));
                }
            } else if opens == closes + 1 {
                return Some(ctx.indent(found_line));
            }
        }
    }

    if standalone(&prevline, "where") {
        return indent_of(prevlinenum).map(|i| i + ctx.sw());
    }

    let last = prevline.chars().last();
    if last == Some(',') && starts_with_word(&prevline, "where") {
        return indent_of(prevlinenum).map(|i| i + 6);
    }

    // `prevline =~# '\V\^\s\*.'`: in very nomagic mode the `.` is a literal dot (a method
    // chain line).
    if skip_blanks(&prevline).starts_with('.')
        && last == Some(';')
        && let Some(start) = lnum.checked_sub(1).and_then(|l| scope_start(ctx, l))
        && start < lnum
    {
        return Some(ctx.indent(start) + ctx.sw());
    }

    if skip_blanks(&prevline).starts_with(']')
        && last == Some(';')
        && !skip_blanks(&line).starts_with('}')
    {
        return indent_of(prevlinenum);
    }

    let cur_trimmed = get_line_trimmed(ctx, Some(lnum));
    let cur_first = skip_blanks(&cur_trimmed).chars().next();
    if last == Some(',')
        && !matches!(cur_first, Some('[' | ']' | '{' | '}' | ')'))
        && !starts_with_word(&prevline, "fn")
        && !open_paren_args_comma(&prevline)
        && !is_match_arm(&cur_trimmed)
    {
        return indent_of(prevlinenum);
    }

    cindent::get_c_indent(ctx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns() {
        assert!(open_paren_args_comma("foo(a,"));
        assert!(!open_paren_args_comma("foo(,"));
        assert!(!open_paren_args_comma("foo(a), b,"));
        assert!(is_match_arm("    Some(x) => {"));
        assert!(is_match_arm("_=>"));
        assert!(!is_match_arm("let x = y;"));
        assert!(standalone("    }  ", "}"));
        assert!(!standalone("} else {", "}"));
    }
}
