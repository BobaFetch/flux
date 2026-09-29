//! Text objects (`iw`, `a"`, `i(`, `ap`, `it`, …), ported from Vim's `textobject.c`.
//!
//! Each returns the text it covers either for an operator (a range and how it's included) or
//! for Visual mode (a new anchor and cursor).

use flux_core::Text;

use crate::motion::{Kind, Scanner};
use crate::util::{self, Pos, before, pos};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    Word { big: bool },
    Quote(char),
    Block(char, char),
    Paragraph,
    Tag,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextObject {
    pub kind: ObjectKind,
    /// `a…` rather than `i…`: include surrounding white space, quotes or brackets.
    pub around: bool,
}

impl TextObject {
    /// The object named by the character after `i` or `a`.
    pub fn from_char(c: char, around: bool) -> Option<Self> {
        let kind = match c {
            'w' => ObjectKind::Word { big: false },
            'W' => ObjectKind::Word { big: true },
            '"' | '\'' | '`' => ObjectKind::Quote(c),
            '(' | ')' | 'b' => ObjectKind::Block('(', ')'),
            '[' | ']' => ObjectKind::Block('[', ']'),
            '{' | '}' | 'B' => ObjectKind::Block('{', '}'),
            '<' | '>' => ObjectKind::Block('<', '>'),
            'p' => ObjectKind::Paragraph,
            't' => ObjectKind::Tag,
            _ => return None,
        };
        Some(Self { kind, around })
    }
}

/// What an object covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub start: Pos,
    pub end: Pos,
    pub kind: Kind,
    /// Skip Vim's adjustment of exclusive ranges that end in column 0 (tag objects).
    pub no_adjust: bool,
}

/// Select `obj` around `cursor`. In Visual mode `anchor` is the other end of the selection,
/// which some objects extend rather than replace.
pub fn select(
    text: &Text,
    obj: TextObject,
    count: usize,
    cursor: Pos,
    anchor: Option<Pos>,
) -> Option<Selection> {
    let count = count.max(1);
    let charwise = |start, end, inclusive| Selection {
        start,
        end,
        kind: if inclusive {
            Kind::Inclusive
        } else {
            Kind::Exclusive
        },
        no_adjust: false,
    };
    match obj.kind {
        ObjectKind::Word { big } => {
            let (start, end, inclusive) =
                current_word(text, cursor, anchor, count, obj.around, big)?;
            Some(charwise(start, end, inclusive))
        }
        ObjectKind::Quote(q) => {
            let (start, end, inclusive) = current_quote(text, cursor, count, obj.around, q)?;
            Some(charwise(start, end, inclusive))
        }
        ObjectKind::Block(open, close) => {
            let (start, end, inclusive) =
                current_block(text, cursor, anchor, count, obj.around, open, close)?;
            Some(charwise(start, end, inclusive))
        }
        ObjectKind::Paragraph => {
            let (first, last) = current_par(text, cursor.line, count, obj.around)?;
            Some(Selection {
                start: pos(first, 0),
                end: pos(last, 0),
                kind: Kind::Linewise,
                no_adjust: false,
            })
        }
        ObjectKind::Tag => {
            let (start, end) = current_tag(text, cursor, count, obj.around)?;
            Some(Selection {
                start,
                end,
                kind: Kind::Exclusive,
                no_adjust: true,
            })
        }
    }
}

/// `iw`, `aw`, `iW`, `aW`. Returns start, end and whether the end is included.
fn current_word(
    text: &Text,
    cursor: Pos,
    anchor: Option<Pos>,
    mut count: usize,
    include: bool,
    big: bool,
) -> Option<(Pos, Pos, bool)> {
    let mut sc = Scanner::new(text, cursor, big);
    let mut start = anchor.unwrap_or(cursor);
    let mut inclusive = true;
    let mut include_white = false;
    if anchor.is_none_or(|a| a == cursor) {
        sc.back_in_line();
        start = sc.pos();
        if (sc.cls() == 0) == include {
            if !sc.end_word(1, true, true) {
                return None;
            }
        } else {
            // Go to the start of the next word; a one-char word at the line end leaves us on the
            // next line, so back up.
            sc.fwd_word(1, true);
            if sc.col() == 0 {
                sc.decl();
            } else {
                sc.oneleft();
            }
            include_white = include;
        }
        count -= 1;
    }
    while count > 0 {
        inclusive = true;
        if anchor.is_some_and(|a| before(sc.pos(), a)) {
            if sc.decl() == -1 {
                return None;
            }
            if include != (sc.cls() != 0) {
                if !sc.bck_word(1, true) {
                    return None;
                }
            } else {
                if !sc.bckend_word(1, true) {
                    return None;
                }
                sc.incl();
            }
        } else {
            if sc.incl() == -1 {
                return None;
            }
            if include != (sc.cls() == 0) {
                if !sc.fwd_word(1, true) && count > 1 {
                    return None;
                }
                if !sc.oneleft() {
                    inclusive = false;
                }
            } else if !sc.end_word(1, true, true) {
                return None;
            }
        }
        count -= 1;
    }
    if include_white && (sc.cls() != 0 || (sc.col() == 0 && !inclusive)) {
        // No white space after the word: take the white space before it instead, but not an
        // indent.
        let end = sc.pos();
        sc.set(start);
        if sc.oneleft() {
            sc.back_in_line();
            if sc.cls() == 0 && sc.col() > 0 {
                start = sc.pos();
            }
        }
        sc.set(end);
    }
    Some((start, sc.pos(), inclusive))
}

/// `i"`, `a"` and the other quotes, within the cursor's line.
fn current_quote(
    text: &Text,
    cursor: Pos,
    count: usize,
    include: bool,
    quote: char,
) -> Option<(Pos, Pos, bool)> {
    let line: Vec<char> = text.line_str(cursor.line).chars().collect();
    let len = line.len();
    let col = cursor.col.min(len);
    let next_quote = |mut c: usize| -> Option<usize> {
        while c < len {
            if line[c] == '\\' {
                c += 1;
            } else if line[c] == quote {
                return Some(c);
            }
            c += 1;
        }
        None
    };
    let prev_quote = |mut c: usize| -> usize {
        while c > 0 {
            c -= 1;
            let escapes = line[..c].iter().rev().take_while(|&&ch| ch == '\\').count();
            if escapes % 2 == 1 {
                c -= escapes;
            } else if line[c] == quote {
                break;
            }
        }
        c
    };

    let (mut col_start, mut col_end);
    if line.get(col) == Some(&quote) {
        // On a quote: count pairs from the start of the line to see if it opens or closes.
        let mut from = 0;
        loop {
            col_start = next_quote(from)?;
            if col_start > col {
                return None;
            }
            col_end = next_quote(col_start + 1)?;
            if col_start <= col && col <= col_end {
                break;
            }
            from = col_end + 1;
        }
    } else {
        col_start = prev_quote(col);
        if line.get(col_start) != Some(&quote) {
            // No quote before the cursor: use the first quoted string after it.
            col_start = next_quote(col_start)?;
        }
        col_end = next_quote(col_start + 1)?;
    }
    if include {
        if line.get(col_end + 1).is_some_and(|&c| util::is_white(c)) {
            while line.get(col_end + 1).is_some_and(|&c| util::is_white(c)) {
                col_end += 1;
            }
        } else {
            while col_start > 0 && util::is_white(line[col_start - 1]) {
                col_start -= 1;
            }
        }
    }
    let l = cursor.line;
    if !include && count < 2 {
        // Between the quotes: up to but not including the closing one.
        Some((pos(l, col_start + 1), pos(l, col_end), false))
    } else if col_end + 1 >= len {
        Some((pos(l, col_start), pos(l, len), true))
    } else {
        Some((pos(l, col_start), pos(l, col_end + 1), false))
    }
}

/// `i(`, `a{`, and the other brackets. Brackets in strings are not treated specially.
fn current_block(
    text: &Text,
    cursor: Pos,
    anchor: Option<Pos>,
    count: usize,
    include: bool,
    open: char,
    close: char,
) -> Option<(Pos, Pos, bool)> {
    let visual = anchor.filter(|&a| a != cursor);
    let (old_start, old_end) = match visual {
        Some(a) if before(a, cursor) => (a, cursor),
        Some(a) => (cursor, a),
        None => (cursor, cursor),
    };
    let mut from = old_start;
    if visual.is_none() {
        let line: Vec<char> = text.line_str(cursor.line).chars().collect();
        if open == '{' {
            // In the indent of a `{` line, start from the first non-blank.
            while indent_len(&line) > from.col && from.col < line.len() {
                from.col += 1;
            }
        }
        if line.get(from.col) == Some(&open) {
            from.col += 1;
        }
    }
    let mut level_from = from;
    let mut start = None;
    for _ in 0..count {
        match find_unmatched(text, level_from, open, close, false) {
            Some(p) => {
                level_from = p;
                start = Some(p);
            }
            None => break,
        }
    }
    let mut start = start?;
    let mut end = find_unmatched(text, start, open, close, true)?;
    if include {
        return Some((start, end, true));
    }
    loop {
        let mut s = Scanner::new(text, start, false);
        s.incl();
        let start_pos = s.pos();
        let mut e = Scanner::new(text, end, false);
        let mut sol = e.col() == 0;
        e.decl();
        while indent_len(&text.line_str(e.pos().line).chars().collect::<Vec<_>>()) > e.col() {
            sol = true;
            if e.decl() != 0 {
                break;
            }
        }
        let end_pos = e.pos();
        // In Visual mode, when this doesn't grow the selection, go out a level.
        if visual.is_some()
            && !before(start_pos, old_start)
            && !before(old_end, end_pos)
            && start_pos != end_pos
        {
            start = find_unmatched(text, start, open, close, false)?;
            end = find_unmatched(text, start, open, close, true)?;
            continue;
        }
        return Some(if sol {
            let mut e = Scanner::new(text, end_pos, false);
            e.incl();
            (start_pos, e.pos(), false)
        } else if !before(end_pos, start_pos) {
            (start_pos, end_pos, true)
        } else {
            // Nothing between the brackets.
            (start_pos, start_pos, false)
        });
    }
}

fn indent_len(line: &[char]) -> usize {
    line.iter().take_while(|&&c| util::is_white(c)).count()
}

/// The unmatched `open` before `from` (or, with `forward`, the unmatched `close` after it),
/// skipping nested pairs.
fn find_unmatched(text: &Text, from: Pos, open: char, close: char, forward: bool) -> Option<Pos> {
    let rope = text.rope();
    let start = text.pos_to_char(from.line, from.col);
    let mut depth = 0usize;
    if forward {
        for (i, ch) in rope.chars_at(start + 1).enumerate() {
            if ch == open {
                depth += 1;
            } else if ch == close {
                if depth == 0 {
                    let (l, c) = text.char_to_pos(start + 1 + i);
                    return Some(pos(l, c));
                }
                depth -= 1;
            }
        }
    } else {
        let mut idx = start;
        let mut iter = rope.chars_at(start);
        while let Some(ch) = iter.prev() {
            idx -= 1;
            if ch == close {
                depth += 1;
            } else if ch == open {
                if depth == 0 {
                    let (l, c) = text.char_to_pos(idx);
                    return Some(pos(l, c));
                }
                depth -= 1;
            }
        }
    }
    None
}

/// `ip` and `ap`: whole lines. Returns the first and last line.
fn current_par(text: &Text, line: usize, count: usize, include: bool) -> Option<(usize, usize)> {
    let last = text.last_line() as isize;
    let white = |l: isize| text.line_str(l as usize).chars().all(util::is_white);
    let mut start = line as isize;
    let white_in_front = white(start);
    while start > 0 {
        if white_in_front {
            if !white(start - 1) {
                break;
            }
        } else if white(start - 1) {
            break;
        }
        start -= 1;
    }
    let mut end = start;
    while end <= last && white(end) {
        end += 1;
    }
    end -= 1;
    let mut i = count as isize;
    if !include && white_in_front {
        i -= 1;
    }
    let mut do_white = false;
    while i > 0 {
        i -= 1;
        if end == last {
            return None;
        }
        if !include {
            do_white = white(end + 1);
        }
        if include || !do_white {
            end += 1;
            while end < last && !white(end + 1) {
                end += 1;
            }
        }
        if i == 0 && white_in_front && include {
            break;
        }
        if include || do_white {
            while end < last && white(end + 1) {
                end += 1;
            }
        }
    }
    if !white_in_front && !white(end) && include {
        while start > 0 && white(start - 1) {
            start -= 1;
        }
    }
    Some((start as usize, end.max(start) as usize))
}

/// `it` and `at`: the element around the cursor, `count` levels out. Returns an exclusive range.
fn current_tag(text: &Text, cursor: Pos, count: usize, include: bool) -> Option<(Pos, Pos)> {
    let s: Vec<char> = text.rope().chars().collect();
    let at = text.pos_to_char(cursor.line, cursor.col);
    // Open tags waiting for their close: name, start of `<`, end after `>`.
    let mut stack: Vec<(String, usize, usize)> = Vec::new();
    // Elements containing the cursor: (outer start, inner start, inner end, outer end).
    let mut around: Vec<(usize, usize, usize, usize)> = Vec::new();
    let mut i = 0;
    while i < s.len() {
        if s[i] != '<' {
            i += 1;
            continue;
        }
        let Some(gt) = s[i..].iter().position(|&c| c == '>').map(|p| i + p) else {
            break;
        };
        let inner: String = s[i + 1..gt].iter().collect();
        let tag_end = gt + 1;
        if let Some(name) = inner.strip_prefix('/') {
            let name = name.trim().to_string();
            if let Some(k) = stack.iter().rposition(|(n, _, _)| *n == name) {
                let (_, open_start, open_end) = stack[k].clone();
                stack.truncate(k);
                if open_start <= at && at < tag_end {
                    around.push((open_start, open_end, i, tag_end));
                }
            }
        } else if !inner.ends_with('/') && inner.chars().next().is_some_and(|c| c.is_alphabetic()) {
            let name: String = inner
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '/')
                .collect();
            stack.push((name, i, tag_end));
        }
        i = tag_end;
    }
    // Innermost element first: the one that starts last.
    around.sort_by_key(|&(start, ..)| std::cmp::Reverse(start));
    let &(outer_start, inner_start, inner_end, outer_end) = around.get(count - 1)?;
    let (from, to) = if include {
        (outer_start, outer_end)
    } else {
        (inner_start, inner_end)
    };
    let p = |idx| {
        let (l, c) = text.char_to_pos(idx);
        pos(l, c)
    };
    Some((p(from), p(to)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(
        text: &str,
        cursor: (usize, usize),
        obj: &str,
        count: usize,
    ) -> Option<(Pos, Pos, Kind)> {
        let t = Text::new(text);
        let mut ch = obj.chars();
        let around = ch.next() == Some('a');
        let o = TextObject::from_char(ch.next().unwrap(), around).unwrap();
        select(&t, o, count, pos(cursor.0, cursor.1), None).map(|s| (s.start, s.end, s.kind))
    }

    #[test]
    fn words() {
        assert_eq!(
            sel("foo bar baz", (0, 5), "iw", 1),
            Some((pos(0, 4), pos(0, 6), Kind::Inclusive))
        );
        assert_eq!(
            sel("foo bar baz", (0, 5), "aw", 1),
            Some((pos(0, 4), pos(0, 7), Kind::Inclusive))
        );
        assert_eq!(
            sel("foo bar baz", (0, 9), "aw", 1),
            Some((pos(0, 7), pos(0, 10), Kind::Inclusive))
        );
    }

    #[test]
    fn quotes_and_blocks() {
        assert_eq!(
            sel("a \"bc\" d", (0, 3), "i\"", 1),
            Some((pos(0, 3), pos(0, 5), Kind::Exclusive))
        );
        assert_eq!(
            sel("f(a, b)", (0, 3), "i(", 1),
            Some((pos(0, 2), pos(0, 5), Kind::Inclusive))
        );
        assert_eq!(
            sel("f(a, b)", (0, 3), "a(", 1),
            Some((pos(0, 1), pos(0, 6), Kind::Inclusive))
        );
        assert_eq!(sel("abc", (0, 1), "i(", 1), None);
    }

    #[test]
    fn paragraphs_and_tags() {
        assert_eq!(
            sel("a\nb\n\nc", (0, 0), "ip", 1),
            Some((pos(0, 0), pos(1, 0), Kind::Linewise))
        );
        assert_eq!(
            sel("<a><b>x</b></a>", (0, 6), "it", 1),
            Some((pos(0, 6), pos(0, 7), Kind::Exclusive))
        );
        assert_eq!(
            sel("<a><b>x</b></a>", (0, 6), "it", 2),
            Some((pos(0, 3), pos(0, 11), Kind::Exclusive))
        );
    }
}
