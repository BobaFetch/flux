//! Character classes and grapheme stepping, as Vim's word motions and cursor moves see them.

use unicode_segmentation::UnicodeSegmentation;

/// Vim's character class for word motions: 0 for blanks, 1 for punctuation, 2 for keyword
/// characters, and other values for scripts Vim treats as separate words (CJK, emoji, …). A
/// "WORD" motion folds every non-blank class into 1.
pub fn class(c: char, bigword: bool) -> u32 {
    if is_blank(c) {
        return 0;
    }
    if bigword {
        return 1;
    }
    let cp = c as u32;
    if cp < 0x100 {
        return if is_keyword(c) { 2 } else { 1 };
    }
    // A condensed version of Vim's `utf_class` table.
    match cp {
        0x037e
        | 0x0387
        | 0x055a..=0x055f
        | 0x0589
        | 0x05be
        | 0x05c0
        | 0x05c3
        | 0x05f3..=0x05f4
        | 0x060c
        | 0x061b
        | 0x061f
        | 0x066a..=0x066d
        | 0x06d4
        | 0x0700..=0x070d
        | 0x0964..=0x0965
        | 0x0970
        | 0x0df4
        | 0x0e4f
        | 0x0e5a..=0x0e5b
        | 0x0f04..=0x0f12
        | 0x0f3a..=0x0f3d
        | 0x0f85
        | 0x104a..=0x104f
        | 0x10fb
        | 0x1361..=0x1368
        | 0x166d..=0x166e
        | 0x169b..=0x169c
        | 0x16eb..=0x16ed
        | 0x1735..=0x1736
        | 0x17d4..=0x17dc
        | 0x1800..=0x180a
        | 0x2010..=0x2027
        | 0x2030..=0x205e
        | 0x20a0..=0x27ff
        | 0x2900..=0x2998
        | 0x29d8..=0x29db
        | 0x29fc..=0x29fd
        | 0x2e00..=0x2e7f
        | 0x3001..=0x3020
        | 0x3030
        | 0x303d
        | 0xfd3e..=0xfd3f
        | 0xfe30..=0xfe6b
        | 0xff00..=0xff0f
        | 0xff1a..=0xff20
        | 0xff3b..=0xff40
        | 0xff5b..=0xff65
        | 0x1d000..=0x1d24f
        | 0x1d400..=0x1d7ff => 1,
        0x2000..=0x200b | 0x2028..=0x2029 | 0x202f | 0x205f | 0x3000 => 0,
        0x2070..=0x207f => 0x2070,
        0x2080..=0x2094 => 0x2080,
        0x2800..=0x28ff => 0x2800,
        0x3040..=0x309f => 0x3040,
        0x30a0..=0x30ff => 0x30a0,
        0x3300..=0x9fff | 0xf900..=0xfaff | 0x20000..=0x2fa1f => 0x4e00,
        0xac00..=0xd7a3 => 0xac00,
        0x1f000..=0x1faff => 3,
        _ => 2,
    }
}

/// Space, tab or no-break space.
pub fn is_blank(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\u{a0}')
}

/// Vim's default 'iskeyword' (`@,48-57,_,192-255`), extended to letters and digits beyond
/// Latin-1 the way Vim treats them.
pub fn is_keyword(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || matches!(c as u32, 0xc0..=0xff if c != '×' && c != '÷')
}

/// Char offsets where each grapheme of `line` starts.
pub fn grapheme_starts(line: &str) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut col = 0;
    for g in line.graphemes(true) {
        starts.push(col);
        col += g.chars().count();
    }
    starts
}

/// The start of the grapheme after the one at `col`, or the line length at the last one.
pub fn next_grapheme(line: &str, col: usize) -> usize {
    let mut pos = 0;
    for g in line.graphemes(true) {
        let len = g.chars().count();
        if pos + len > col {
            return pos + len;
        }
        pos += len;
    }
    pos
}

/// The start of the grapheme before `col`, or 0.
pub fn prev_grapheme(line: &str, col: usize) -> usize {
    let mut prev = 0;
    let mut pos = 0;
    for g in line.graphemes(true) {
        if pos >= col {
            break;
        }
        prev = pos;
        pos += g.chars().count();
    }
    prev
}

/// The start of the grapheme containing `col`.
pub fn snap_to_grapheme(line: &str, col: usize) -> usize {
    let mut pos = 0;
    for g in line.graphemes(true) {
        let len = g.chars().count();
        if pos + len > col {
            return pos;
        }
        pos += len;
    }
    pos
}

/// Start of the last grapheme, where the Normal-mode cursor sits at the end of a line; 0 when
/// the line is empty.
pub fn last_grapheme(line: &str) -> usize {
    grapheme_starts(line).last().copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes() {
        assert_eq!(class('a', false), 2);
        assert_eq!(class('_', false), 2);
        assert_eq!(class('é', false), 2);
        assert_eq!(class('.', false), 1);
        assert_eq!(class(' ', false), 0);
        assert_eq!(class('日', false), 0x4e00);
        assert_eq!(class('.', true), 1);
        assert_eq!(class('a', true), 1);
    }

    #[test]
    fn grapheme_steps() {
        let s = "e\u{301}x日";
        assert_eq!(grapheme_starts(s), [0, 2, 3]);
        assert_eq!(next_grapheme(s, 0), 2);
        assert_eq!(next_grapheme(s, 3), 4);
        assert_eq!(prev_grapheme(s, 3), 2);
        assert_eq!(prev_grapheme(s, 2), 0);
        assert_eq!(snap_to_grapheme(s, 1), 0);
        assert_eq!(last_grapheme(s), 3);
        assert_eq!(last_grapheme(""), 0);
    }
}
