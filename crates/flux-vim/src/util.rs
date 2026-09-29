//! Small text helpers shared by motions, operators and Insert mode.

use flux_core::chars;
use flux_view::{Cursor, Editor, Options};

pub type Pos = Cursor;

pub fn pos(line: usize, col: usize) -> Pos {
    Pos { line, col }
}

pub fn before(a: Pos, b: Pos) -> bool {
    (a.line, a.col) < (b.line, b.col)
}

pub fn line(editor: &Editor, line: usize) -> String {
    editor.text().line_str(line).into_owned()
}

pub fn is_white(c: char) -> bool {
    c == ' ' || c == '\t'
}

/// Column of the first non-blank character, or of the last character when the line is all
/// blanks (where `^` goes).
pub fn first_non_blank(s: &str) -> usize {
    match s.chars().position(|c| !is_white(c)) {
        Some(col) => col,
        None => chars::last_grapheme(s),
    }
}

/// Column after the leading blanks, which is the line length for an all-blank line (where `I`
/// inserts).
pub fn skip_white(s: &str) -> usize {
    s.chars().take_while(|&c| is_white(c)).count()
}

pub fn indent_of(s: &str) -> &str {
    let end = s.len() - s.trim_start_matches([' ', '\t']).len();
    &s[..end]
}

/// Screen width of the leading blanks.
pub fn indent_width(s: &str, tabstop: usize) -> usize {
    let mut width = 0;
    for c in s.chars() {
        match c {
            ' ' => width += 1,
            '\t' => width += tabstop - width % tabstop,
            _ => break,
        }
    }
    width
}

/// Blanks that indent to `width`: tabs then spaces, or only spaces with 'expandtab'.
pub fn make_indent(width: usize, options: &Options) -> String {
    if options.expandtab {
        " ".repeat(width)
    } else {
        let tabs = width / options.tabstop;
        "\t".repeat(tabs) + &" ".repeat(width - tabs * options.tabstop)
    }
}

/// The cursor is in the indent: everything before it is blank (Vim's `inindent(0)`).
pub fn in_indent(s: &str, col: usize) -> bool {
    s.chars().take(col).all(is_white)
}

/// Vim's messages for line counts, shown when more than 'report' (2) lines are involved.
pub fn more_lines_message(delta: isize) -> Option<String> {
    match delta {
        d if d > 2 => Some(format!("{d} more lines")),
        d if d < -2 => Some(format!("{} fewer lines", -d)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indents() {
        assert_eq!(first_non_blank("  ab"), 2);
        assert_eq!(first_non_blank("    "), 3);
        assert_eq!(skip_white("    "), 4);
        assert_eq!(indent_of("\t x"), "\t ");
        assert_eq!(indent_width("  \t    a", 8), 12);
        let options = Options::default();
        assert_eq!(make_indent(12, &options), "\t    ");
        assert!(in_indent("  x", 2));
        assert!(!in_indent("  x", 3));
    }
}
