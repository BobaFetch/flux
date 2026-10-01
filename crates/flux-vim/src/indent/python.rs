//! `python#GetIndent()`, ported from Neovim's `runtime/autoload/python.vim` with the defaults
//! `g:python_indent` gets there (open paren and continuation lines `shiftwidth() * 2`, nested
//! paren `shiftwidth()`, closing parens aligned with the last line).
//!
//! Neovim highlights Python with its regex syntax, whose items the script asks about (strings,
//! comments); flux asks the tree-sitter captures instead.

use super::{Ctx, SynKind};

/// How far back `s:SearchBracket` looks for an open bracket.
const MAXOFF: usize = 50;

/// The indent for `ctx.lnum`, or `None` to keep it.
pub(super) fn indent(ctx: &Ctx) -> Option<usize> {
    let lnum = ctx.lnum;
    let sw = ctx.sw() as isize;
    let line_above = |n: usize| -> String {
        lnum.checked_sub(n)
            .map(|l| ctx.line(l).into_owned())
            .unwrap_or_default()
    };
    // An explicitly joined line: line up with the previous one if it was joined too,
    // otherwise two 'shiftwidth' more.
    if line_above(1).ends_with('\\') {
        if lnum > 1 && line_above(2).ends_with('\\') {
            return Some(ctx.indent(lnum - 1));
        }
        return Some(ctx.indent(lnum - 1) + 2 * ctx.sw());
    }
    // The start of the line in a string: leave it alone.
    if starts_in_string(ctx, lnum) {
        return None;
    }
    // The first non-blank line gets no indent.
    let Some(plnum) = lnum.checked_sub(1).and_then(|l| ctx.prevnonblank(l)) else {
        return Some(0);
    };

    // Inside brackets: line up with the open bracket unless it ends its line.
    if let Some((pl, pc)) = search_bracket(ctx, lnum, (lnum, 0)) {
        let parcol = pc + 1;
        if parcol != ctx.line(pl).len() {
            return Some(parcol);
        }
    }
    // The previous line inside brackets: use the indent of the line that opened them.
    let parlnum = search_bracket(ctx, plnum, (plnum, 0)).map(|(l, _)| l);
    let (plindent, plnumstart) = match parlnum {
        Some(p) => (ctx.indent(p), p),
        None => (ctx.indent(plnum), plnum),
    };
    if let Some((p, pc)) = search_bracket(ctx, lnum, (lnum, 0)) {
        if p == plnum {
            // The first line inside the brackets: two 'shiftwidth', or one when the brackets
            // are themselves inside brackets.
            if search_bracket(ctx, lnum, (p, pc)).is_some() {
                return Some(ctx.indent(plnum) + ctx.sw());
            }
            return Some(ctx.indent(plnum) + 2 * ctx.sw());
        }
        if plnumstart == p {
            return Some(ctx.indent(plnum));
        }
        return Some(plindent);
    }

    // The previous line without a trailing comment.
    let mut pline = ctx.line(plnum).into_owned();
    let len = pline.len();
    if ctx.has_syntax() {
        let comment = |col: usize| ctx.syn_kind(plnum, col - 1) == SynKind::Comment;
        if len > 0 && comment(len) {
            let (mut min, mut max) = (1, len);
            while min < max {
                let col = (min + max) / 2;
                if comment(col) {
                    max = col;
                } else {
                    min = col + 1;
                }
            }
            pline.truncate(min - 1);
        }
    } else if let Some(i) = pline.find('#') {
        pline.truncate(i);
    }

    // After a colon: one more level.
    if pline.trim_end_matches([' ', '\t']).ends_with(':') {
        return Some(plindent + ctx.sw());
    }

    let dedented = |expected: isize| ctx.indent(lnum) as isize <= expected - sw;
    let to_indent = |n: isize| usize::try_from(n).ok();

    // After a statement that stops execution: one level less, unless already dedented.
    let pfull = ctx.line(plnum);
    if starts_with_word(&pfull, &["break", "continue", "raise", "return", "pass"]) {
        if dedented(ctx.indent(plnum) as isize) {
            return None;
        }
        return to_indent(ctx.indent(plnum) as isize - sw);
    }

    let line = ctx.line(lnum);
    // `except` and `finally` line up with the `try` or `except` above.
    if starts_with_word(&line, &["except", "finally"]) {
        for l in (0..lnum).rev() {
            if starts_with_word(&ctx.line(l), &["try", "except"]) {
                let ind = ctx.indent(l);
                if ind >= ctx.indent(lnum) {
                    return None;
                }
                return Some(ind);
            }
        }
        return None;
    }

    // `elif` and `else` dedent, unless after a one-liner or already dedented.
    if starts_with_word(&line, &["elif", "else"]) {
        if starts_with_word(&ctx.line(plnumstart), &["for", "if", "elif", "try"]) {
            return Some(plindent);
        }
        if dedented(plindent as isize) {
            return None;
        }
        return to_indent(plindent as isize - sw);
    }

    // After brackets, back to the line that opened them, unless already dedented.
    if parlnum.is_some() {
        if dedented(plindent as isize) {
            return None;
        }
        return Some(plindent);
    }
    None
}

/// `line =~ '^\s*\(word\|…\)\>'`.
fn starts_with_word(line: &str, words: &[&str]) -> bool {
    let s = line.trim_start_matches([' ', '\t']);
    words.iter().any(|w| {
        s.strip_prefix(w).is_some_and(|rest| {
            !rest
                .chars()
                .next()
                .is_some_and(flux_core::chars::is_keyword)
        })
    })
}

/// Whether the first byte of line `l` is in a string (the script's `synID(lnum, 1, 1)` named
/// `…String`). Quotes are separate syntax items in Vim (`pythonQuotes`), so a line starting
/// with a quote isn't in a string.
fn starts_in_string(ctx: &Ctx, l: usize) -> bool {
    let line = ctx.line(l);
    let rest = line.trim_start_matches(['r', 'R', 'b', 'B', 'u', 'U', 'f', 'F']);
    if line.len() - rest.len() <= 2 && rest.starts_with(['"', '\'']) {
        return false;
    }
    ctx.syn_kind(l, 0) == SynKind::String
}

/// `s:SearchBracket`: Vim's `searchpairpos('[[({]', '', '[])}]', 'bW', skip, stopline)` back
/// from `from` (exclusive), skipping brackets in comments and strings, looking no further than
/// `MAXOFF` lines above line `limit`. Any kind of bracket closes any other.
fn search_bracket(ctx: &Ctx, limit: usize, from: (usize, usize)) -> Option<(usize, usize)> {
    let stop = limit.saturating_sub(MAXOFF);
    let mut depth = 0usize;
    for l in (stop..=from.0).rev() {
        let s = ctx.line(l);
        let end = if l == from.0 {
            from.1.min(s.len())
        } else {
            s.len()
        };
        for (i, &b) in s.as_bytes()[..end].iter().enumerate().rev() {
            let open = matches!(b, b'(' | b'[' | b'{');
            let close = matches!(b, b')' | b']' | b'}');
            if !open && !close || ctx.syn_kind(l, i) != SynKind::None {
                continue;
            }
            if close {
                depth += 1;
            } else if depth == 0 {
                return Some((l, i));
            } else {
                depth -= 1;
            }
        }
    }
    None
}
