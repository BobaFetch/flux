//! `GetLuaIndent()`, ported from Neovim's `runtime/indent/lua.vim`.
//!
//! Neovim highlights Lua with tree-sitter, so there are no syntax items for the script's
//! `luaComment` checks to see: they never match, and flux doesn't look at the syntax either.

use std::cell::RefCell;
use std::collections::HashMap;

use flux_core::{Pattern, PatternOptions};

use super::Ctx;

/// Vim's `match()`: the byte offset of the first match of Vim pattern `pat` in `s`.
fn vim_match(pat: &'static str, s: &str) -> Option<usize> {
    thread_local! {
        static CACHE: RefCell<HashMap<&'static str, Pattern>> = RefCell::default();
    }
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let p = c
            .entry(pat)
            .or_insert_with(|| Pattern::new(pat, PatternOptions::default()).expect("valid"));
        p.find_at(s, 0).map(|m| m.start)
    })
}

/// The indent for `ctx.lnum`, or `None` to keep it.
pub(super) fn indent(ctx: &Ctx) -> Option<usize> {
    // Hit the start of the file, use zero indent.
    let Some(prev) = ctx.lnum.checked_sub(1).and_then(|l| ctx.prevnonblank(l)) else {
        return Some(0);
    };
    let sw = ctx.sw() as isize;
    let mut ind = ctx.indent(prev) as isize;
    let prevline = ctx.line(prev);
    // Add a 'shiftwidth' after lines that start a block, unless an `end` or `until` closes it
    // on the same line.
    let opens = vim_match(
        r"^\s*\%(if\>\|for\>\|while\>\|repeat\>\|else\>\|elseif\>\|do\>\|then\>\)",
        &prevline,
    )
    .or_else(|| vim_match(r"\%({\|(\)\s*\%(--\%([^[].*\)\?\)\?$", &prevline))
    .or_else(|| vim_match(r"\<function\>\s*\%(\k\|[.:]\)\{-}\s*(", &prevline))
    .is_some();
    if opens && vim_match(r"\<end\>\|\<until\>", &prevline).is_none() {
        ind += sw;
    }
    // Subtract a 'shiftwidth' on end, else, elseif, until, '}' and ')'.
    if vim_match(
        r"^\s*\%(end\>\|else\>\|elseif\>\|until\>\|}\|)\)",
        &ctx.line(ctx.lnum),
    )
    .is_some()
    {
        ind -= sw;
    }
    // A negative indent keeps the line's indent (Vim's `get_expr_indent`).
    usize::try_from(ind).ok()
}
