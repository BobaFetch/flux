//! `python#GetIndent()`, ported from Neovim's `runtime/autoload/python.vim`.

use super::Ctx;

/// The indent for `ctx.lnum`, or `None` to keep it.
pub(super) fn indent(_ctx: &Ctx) -> Option<usize> {
    None
}
