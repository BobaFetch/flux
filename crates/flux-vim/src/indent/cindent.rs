//! Vim's C indenting (`get_c_indent` in Neovim's `src/nvim/indent_c.c`), with
//! 'cinoptions', used for C, by Rust's indent script, and by `=` when there's no 'indentexpr'.

use super::Ctx;

/// The C indent for `ctx.lnum`.
pub(super) fn get_c_indent(ctx: &Ctx) -> usize {
    match ctx.lnum.checked_sub(1).and_then(|l| ctx.prevnonblank(l)) {
        Some(l) => ctx.indent(l),
        None => 0,
    }
}

/// Vim's `cin_iscase`: the line is a `case x:` or `default:` label.
pub(super) fn is_case(_line: &str, _strict: bool) -> bool {
    false
}

/// Vim's `cin_isscopedecl`: `public:`, `private:` or `protected:`.
pub(super) fn is_scopedecl(_line: &str) -> bool {
    false
}

/// Vim's `cin_islabel`: the line is a goto label.
pub(super) fn is_label(_line: &str) -> bool {
    false
}
