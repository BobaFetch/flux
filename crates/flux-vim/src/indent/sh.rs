//! `GetShIndent()`, ported from Neovim's `runtime/indent/sh.vim` (with its default
//! `b:sh_indent_options`: everything one 'shiftwidth', no extra indent for case breaks).
//!
//! The script's patterns are Vim patterns, so they're kept as they are and matched with
//! flux's Vim pattern engine.
//!
//! Where the script asks Vim's regex syntax (syntax/sh.vim) about a position, flux asks the
//! tree-sitter captures instead:
//! - `searchpair()` skips matches in "comment" and "quote" groups: flux skips positions
//!   captured as a comment or string. Tree-sitter also captures plain command arguments
//!   (`echo fi`) and here-document bodies as strings, which Vim doesn't skip; such `fi` and
//!   braces are rare.
//! - "Is the line in a here-document" (`synstack()` naming a heredoc group) is worked out from
//!   the text: an unclosed `<<WORD` above.
//! - `s:is_bash()` (bash arrays) uses `b:is_bash`, which Neovim's filetype detection sets from
//!   the first line when the file is read; flux looks at the first line as it is now.

use std::cell::RefCell;
use std::collections::HashMap;

use flux_core::{Pattern, PatternOptions};

use super::{Ctx, SynKind};

thread_local! {
    static PATTERNS: RefCell<HashMap<String, Pattern>> = RefCell::default();
}

/// Run `f` with the compiled Vim pattern `pat`.
fn with_pattern<R>(pat: &str, f: impl FnOnce(&Pattern) -> R) -> R {
    PATTERNS.with(|p| {
        let mut p = p.borrow_mut();
        let pattern = p.entry(pat.to_string()).or_insert_with(|| {
            Pattern::new(pat, PatternOptions::default()).expect("valid pattern")
        });
        f(pattern)
    })
}

/// Vim's `s =~ pat` (with 'noignorecase').
fn m(s: &str, pat: &str) -> bool {
    with_pattern(pat, |p| p.find_at(s, 0).is_some())
}

fn is_continuation_line(line: &str) -> bool {
    // A comment can't be continued.
    if m(line, r"^\s*#") {
        return false;
    }
    m(
        line,
        r"\%(\%(^\|[^\\]\)\\\|&&\|||\||\)\s*\({\s*\)\=\(#.*\)\=$",
    )
}

/// The first line of the continued lines ending at `lnum`.
fn find_continued_lnum(ctx: &Ctx, lnum: usize) -> usize {
    let mut i = lnum;
    while i > 0 && is_continuation_line(&ctx.line(i - 1)) {
        i -= 1;
    }
    i
}

fn is_function_definition(line: &str, bash: bool) -> bool {
    m(line, r"^\s*\<\k\+\>\s*()\s*{")
        || m(line, r"^\s*{")
        || m(line, r"^\s*function\s*\k\+\s*\%(()\)\?\s*{")
        || (bash && m(line, r"^\s*function\s*\S\+\s*\%(()\)\?\s*{"))
}

fn is_array(line: &str) -> bool {
    m(
        line,
        r"^\s*\(\(declare\|typeset\|local\)\s\+\(-[Aalrtu]\+\s\+\)\?\)\?\<\k\+\>=(",
    )
}

/// `s:is_case_label(line, pnum)`: `pnum` is the line before it, if any.
fn is_case_label(ctx: &Ctx, line: &str, pnum: Option<usize>) -> bool {
    if !m(line, r"^\s*(\=.*)") {
        return false;
    }
    if let Some(p) = pnum {
        let pine = ctx.line(p);
        if !(is_case(&pine) || is_case_ended(&pine)) {
            return false;
        }
    }
    let trimmed = line.trim_start_matches([' ', '\t']);
    let suffix = trimmed.strip_prefix('(').unwrap_or(trimmed).as_bytes();
    let mut nesting = 0;
    let mut i = 0;
    while i < suffix.len() {
        let c = suffix[i];
        i += 1;
        match c {
            b'\\' => i += 1,
            b'(' => nesting += 1,
            b')' => {
                if nesting == 0 {
                    return true;
                }
                nesting -= 1;
            }
            _ => {}
        }
    }
    false
}

fn is_case(line: &str) -> bool {
    m(line, r"^\s*case\>")
}

fn is_case_break(line: &str) -> bool {
    m(line, r"^\s*;[;&]")
}

fn is_case_ended(line: &str) -> bool {
    is_case_break(line) || m(line, r";[;&]\s*\%(#.*\)\=$")
}

/// `s:is_case_empty()`: the script calls itself with the same line when that line is blank or
/// a comment, which ends in an error in Vim (no indent change): `None`.
fn is_case_empty(line: &str) -> Option<bool> {
    if m(line, r"^\s*$") || m(line, r"^\s*#") {
        None
    } else {
        Some(m(line, r"^\s*case\>"))
    }
}

/// `s:is_here_doc()`: a lone word ending a here-document started above (`<<WORD`).
fn is_here_doc(ctx: &Ctx, line: &str) -> bool {
    if !m(line, r"^\w\+$") {
        return false;
    }
    (0..ctx.lnum).any(|l| {
        let s = ctx.line(l);
        s.strip_suffix(line)
            .is_some_and(|rest| rest.ends_with("<<") || rest.ends_with("<<-"))
    })
}

fn is_empty(line: &str) -> bool {
    m(line, r"^\s*$")
}

fn end_block(line: &str) -> bool {
    m(line, r"^\s*}")
}

fn start_block(line: &str) -> bool {
    m(line, r"^[^#]*[{(]\s*\(#.*\)\?$")
}

fn is_comment(line: &str) -> bool {
    m(line, r"^\s*#")
}

fn is_end_expression(line: &str) -> bool {
    m(line, r"\<\%(fi\|esac\|done\|end\)\>\s*\%(#.*\)\=$")
}

/// `s:is_bash()`: `b:is_bash`, which Neovim sets for a `bash` shebang.
fn is_bash(ctx: &Ctx) -> bool {
    let first = ctx.line(0);
    first == "#!/bin/bash" || m(&first, r"^#!.*\<\(bash\|bash2\)\>")
}

/// Whether line `lnum` is in a here-document's body or is its terminator (where Vim's
/// syntax puts a heredoc group at column 1).
fn in_here_doc(ctx: &Ctx, lnum: usize) -> bool {
    // The terminator being waited for, and whether it may be indented (`<<-`).
    let mut open: Option<(String, bool)> = None;
    for l in 0..=lnum {
        let s = ctx.line(l);
        if let Some((word, dash)) = &open {
            if l == lnum {
                return true;
            }
            let t = if *dash {
                s.trim_start_matches([' ', '\t'])
            } else {
                &s
            };
            if t.trim_end_matches([' ', '\t']) == word {
                open = None;
            }
            continue;
        }
        if l == lnum {
            return false;
        }
        open = here_doc_start(&s);
    }
    false
}

/// The terminator of a here-document started on `line` (outside quotes and comments).
fn here_doc_start(line: &str) -> Option<(String, bool)> {
    let b = line.as_bytes();
    let mut quote = None;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match quote {
            Some(q) => {
                if c == b'\\' && q == b'"' {
                    i += 1;
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                b'\\' => i += 1,
                b'\'' | b'"' => quote = Some(c),
                b'#' if i == 0 || b[i - 1] == b' ' || b[i - 1] == b'\t' => return None,
                b'<' if b.get(i + 1) == Some(&b'<') && b.get(i + 2) != Some(&b'<') => {
                    let mut j = i + 2;
                    let dash = b.get(j) == Some(&b'-');
                    if dash {
                        j += 1;
                    }
                    while b.get(j).is_some_and(|c| *c == b' ' || *c == b'\t') {
                        j += 1;
                    }
                    let rest = line[j..].trim_start_matches(['\'', '"', '\\']);
                    let word: String = rest
                        .chars()
                        .take_while(|c| {
                            !matches!(c, ' ' | '\t' | '|' | '>' | ';' | '&' | '\'' | '"' | ')')
                        })
                        .collect();
                    if !word.is_empty() {
                        return Some((word, dash));
                    }
                    i = j;
                    continue;
                }
                _ => {}
            },
        }
        i += 1;
    }
    None
}

/// Whether a `searchpair()` match at byte `col` of line `l` is skipped: in a comment or a
/// quoted string.
fn skipped(ctx: &Ctx, l: usize, col: usize) -> bool {
    ctx.syn_kind(l, col) != SynKind::None
}

/// Vim's `searchpair(start, '', end, 'nW' or 'bnW', skip)` from the cursor: the line of the
/// matching `start` (backward) or `end` (forward).
fn searchpair(ctx: &Ctx, start: &str, end: &str, backward: bool) -> Option<usize> {
    let pat = format!(r"\m\({start}\m\)\|\({end}\m\)");
    // Every match on a line, in order: its column and whether it's the start pattern.
    let matches_on = |l: usize| -> Vec<(usize, bool)> {
        let s = ctx.line(l);
        with_pattern(&pat, |p| {
            let mut out = Vec::new();
            let mut at = 0;
            while at <= s.len() {
                let Some((m, subs)) = p.captures_at(&s, at) else {
                    break;
                };
                out.push((m.start, subs.get(1).copied().flatten().is_some()));
                at = flux_core::pattern::next_char(&s, m.whole_start);
            }
            out
        })
    };
    let (cl, cc) = (ctx.lnum, ctx.col);
    let mut nest = 1;
    let mut visit = |l: usize, col: usize, is_start: bool| -> Option<usize> {
        if skipped(ctx, l, col) {
            return None;
        }
        // An `end` going backward (a `start` going forward) opens a nested pair.
        if is_start != backward {
            nest += 1;
        } else {
            nest -= 1;
            if nest == 0 {
                return Some(l);
            }
        }
        None
    };
    if backward {
        for l in (0..=cl).rev() {
            let mut ms = matches_on(l);
            if l == cl {
                ms.retain(|&(c, _)| c < cc);
            }
            for (c, is_start) in ms.into_iter().rev() {
                if let Some(found) = visit(l, c, is_start) {
                    return Some(found);
                }
            }
        }
    } else {
        for l in cl..ctx.line_count() {
            let mut ms = matches_on(l);
            if l == cl {
                ms.retain(|&(c, _)| c > cc);
            }
            for (c, is_start) in ms {
                if let Some(found) = visit(l, c, is_start) {
                    return Some(found);
                }
            }
        }
    }
    None
}

/// `s:is_in_block()`: the cursor line is inside `{ … }`.
fn is_in_block(ctx: &Ctx) -> bool {
    let prev = searchpair(ctx, "{", "}", true);
    let next = searchpair(ctx, "{", "}", false);
    // Vim compares 1-based line numbers with 0 for "not found".
    let line = ctx.lnum + 1;
    let prev = prev.map_or(0, |l| l + 1);
    let next = next.map_or(0, |l| l + 1);
    line > prev && line < next
}

/// The indent for `ctx.lnum`, or `None` to keep it.
pub(super) fn indent(ctx: &Ctx) -> Option<usize> {
    let v = ctx.lnum;
    let sw = ctx.sw() as isize;
    let curline = ctx.line(v).into_owned();
    let Some(lnum) = v.checked_sub(1).and_then(|l| ctx.prevnonblank(l)) else {
        return Some(0);
    };
    let line = ctx.line(lnum).into_owned();
    let pnum = lnum.checked_sub(1).and_then(|l| ctx.prevnonblank(l));
    let pline = pnum.map(|p| ctx.line(p).into_owned()).unwrap_or_default();
    let mut ind = ctx.indent(lnum) as isize;

    // What the previous lines open or close.
    if start_block(&line) {
        ind += sw;
    } else if m(
        &line,
        r"^\s*\%(if\|then\|do\|else\|elif\|case\|while\|until\|for\|select\|foreach\)\>\($\|\s\)",
    ) {
        if !is_end_expression(&line) {
            ind += sw;
        }
    } else if is_case_label(ctx, &line, pnum) {
        if !is_case_ended(&line) {
            ind += sw;
        }
    } else if is_function_definition(&line, is_bash(ctx)) {
        if !m(&line, r"}\s*\%(#.*\)\=$") {
            ind += sw;
        }
    } else if is_array(&line) && !m(&line, r")\s*$") && is_bash(ctx) {
        ind += sw;
    } else if m(&curline, r"^\s*)$") {
        ind -= sw;
    } else if is_continuation_line(&line) {
        if pnum.is_none() || !is_continuation_line(&pline) {
            ind += sw;
        }
    } else if end_block(&line) && !start_block(&line) {
        ind -= sw;
    } else if let Some(p) = pnum
        && is_continuation_line(&pline)
        && !end_block(&curline)
        && !is_end_expression(&curline)
    {
        // Only add indent if the line and the one before are in the same block.
        let ind2 = ctx.indent(find_continued_lnum(ctx, p)) as isize;
        let mut i = v;
        while !is_empty(&ctx.line(i)) && i > p {
            i -= 1;
        }
        if i == p && (is_continuation_line(&line) || m(&pline, r"{\s*\(#.*\)\=$")) {
            ind += ind2;
        } else {
            ind = ind2;
        }
    }

    // What the current line is.
    let pine = line;
    let line = curline.as_str();
    if m(line, r"^\s*\%(fi\);\?\s*\%(#.*\)\=$") {
        // An `fi` lines up with its `if`.
        ind = ctx.indent(v) as isize;
        let end = if ctx.insert { r"\<fi\>\zs" } else { r"\<fi\>" };
        if let Some(l) = searchpair(ctx, r"^\s*\<if\>", end, true) {
            ind = ctx.indent(l) as isize;
        }
    } else if m(line, r"^\s*\%(then\|do\|else\|elif\|done\|end\)\>")
        || end_block(line)
        || (m(line, r"^\s*esac\>") && is_case_empty(&ctx.line(v.wrapping_sub(1)))?)
    {
        ind -= sw;
    } else if m(line, r"^\s*esac\>") {
        let statements = if is_case_label(ctx, &pine, Some(lnum)) && is_case_ended(&pine) {
            0
        } else {
            sw
        };
        ind -= statements + sw;
    } else if is_case_label(ctx, line, Some(lnum)) {
        if is_case(&pine) {
            ind = ctx.indent(lnum) as isize + sw;
        } else if !(is_case_label(ctx, &pine, Some(lnum)) && is_case_ended(&pine)) {
            ind -= sw;
        }
    } else if is_case_break(line) {
        // 'case-breaks' is 0.
    } else if is_here_doc(ctx, line) {
        ind = 0;
    } else if in_here_doc(ctx, v) {
        // Here-document text keeps its indent.
        return Some(ctx.indent(v));
    } else if is_comment(line) && is_empty(&ctx.line(v.wrapping_sub(1))) {
        return Some(if is_in_block(ctx) {
            ctx.indent(lnum)
        } else {
            ctx.indent(v)
        });
    }

    // A closing `}` lines up with its `{`.
    if m(line, r"^\s*}\s*$")
        && let Some(l) = searchpair(ctx, "{", "}", true)
    {
        return Some(ctx.indent(l));
    }
    Some(ind.max(0) as usize)
}
