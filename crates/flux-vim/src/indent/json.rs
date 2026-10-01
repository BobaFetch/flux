//! `GetJSONIndent()`, ported from Neovim's `runtime/indent/json.vim`.
//!
//! The script only treats `jsonString` syntax items specially, and those never begin a line or
//! hold a line's first bracket in valid JSON (keys are `jsonKeyword`, quotes `jsonQuote`, and
//! strings can't span lines), so flux doesn't need the syntax.

use super::Ctx;

/// The indent for `ctx.lnum`, or `None` to keep it.
pub(super) fn indent(ctx: &Ctx) -> Option<usize> {
    let line = ctx.line(ctx.lnum);
    let ws = line.len() - line.trim_start_matches([' ', '\t']).len();
    // A closing bracket first on the line: line up with the line of its match.
    if let Some(close @ (b']' | b'}')) = line.as_bytes().get(ws).copied() {
        let open = if close == b'}' { b'{' } else { b'[' };
        return Some(match find_open(ctx, ws, open, close) {
            Some(l) => ctx.indent(l),
            None => ctx.indent(ctx.lnum),
        });
    }
    let Some(prev) = ctx.lnum.checked_sub(1).and_then(|l| ctx.prevnonblank(l)) else {
        return Some(0);
    };
    let pline = ctx.line(prev);
    let ind = ctx.indent(prev);
    // Still inside a bracket the previous line opened: one more level.
    if pline.contains(['[', '(', '{']) && has_opening_brackets(&pline) {
        return Some(ind + ctx.sw());
    }
    Some(ind)
}

/// Vim's `searchpair(open, '', close, 'bW')` from the bracket at byte `col` of the current
/// line: the line of the unmatched `open` before it.
fn find_open(ctx: &Ctx, col: usize, open: u8, close: u8) -> Option<usize> {
    let mut depth = 0usize;
    for l in (0..=ctx.lnum).rev() {
        let s = ctx.line(l);
        let end = if l == ctx.lnum { col } else { s.len() };
        for &b in s.as_bytes()[..end].iter().rev() {
            if b == close {
                depth += 1;
            } else if b == open {
                if depth == 0 {
                    return Some(l);
                }
                depth -= 1;
            }
        }
    }
    None
}

/// `s:LineHasOpeningBrackets`: more of some kind of bracket opened than closed on the line.
fn has_opening_brackets(line: &str) -> bool {
    let mut open = [0isize; 3];
    for b in line.bytes() {
        if let Some(i) = b"(){}[]".iter().position(|&c| c == b) {
            open[i / 2] += if i % 2 == 0 { 1 } else { -1 };
        }
    }
    open.iter().any(|&n| n > 0)
}
