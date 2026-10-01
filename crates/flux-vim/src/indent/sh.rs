//! `GetShIndent()`, ported from Neovim's `runtime/indent/sh.vim`.

use super::Ctx;

/// The indent for `ctx.lnum`, or `None` to keep it.
pub(super) fn indent(_ctx: &Ctx) -> Option<usize> {
    None
}
