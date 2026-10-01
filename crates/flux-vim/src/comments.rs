//! Comment leaders from 'comments', ported from Vim's `get_leader_len`,
//! `get_last_leader_offset` (`change.c`), `skip_comment` (`ops.c`) and `check_linecomment`
//! (`search.c`). Offsets are in bytes, as Vim's are.

/// One entry of 'comments': its flags and its string (`s1:/*` is flags `s1`, string `/*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Part<'a> {
    pub flags: &'a str,
    pub string: &'a str,
}

impl Part<'_> {
    pub fn has(&self, flag: char) -> bool {
        self.flags.contains(flag)
    }
}

/// The entries of a 'comments' value; ones without a `:` are ignored, as in Vim.
pub(crate) fn parts(comments: &str) -> Vec<Part<'_>> {
    let mut out = Vec::new();
    let mut rest = comments;
    while !rest.is_empty() {
        // Vim's `copy_option_part`: up to an unescaped comma.
        let mut end = rest.len();
        let b = rest.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'\\' && b.get(i + 1) == Some(&b',') {
                i += 2;
                continue;
            }
            if b[i] == b',' {
                end = i;
                break;
            }
            i += 1;
        }
        let part = &rest[..end];
        rest = rest.get(end + 1..).unwrap_or("");
        rest = rest.trim_start_matches(' ');
        if let Some((flags, string)) = part.split_once(':') {
            out.push(Part { flags, string });
        }
    }
    out
}

fn is_white(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

/// Vim's `get_leader_len`: the length of the comment leader `line` starts with (after its
/// indent), and the entry it matched. `backward` is for `O` (entries with `O` don't count);
/// `include_space` takes in the white space after the leader.
pub(crate) fn leader_len(
    comments: &str,
    line: &str,
    backward: bool,
    include_space: bool,
) -> (usize, Option<usize>) {
    let parts = parts(comments);
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() && is_white(b[i]) {
        i += 1;
    }
    let mut result = 0;
    let mut got_com = false;
    let mut flags_of: Option<usize> = None;
    // Repeat to match several nested comment strings.
    while i < b.len() {
        let mut found: Option<usize> = None;
        let mut middle_match: Option<(usize, usize)> = None;
        for (k, part) in parts.iter().enumerate() {
            // A middle match found before is used unless this is a middle or end.
            if middle_match.is_some() && !part.has('m') && !part.has('e') {
                break;
            }
            if got_com && !part.has('n') {
                continue;
            }
            if backward && part.has('O') {
                continue;
            }
            let mut string = part.string.as_bytes();
            // A string starting with white space needs some white space in the line.
            if string.first().is_some_and(|&c| is_white(c)) {
                if i == 0 || !is_white(b[i - 1]) {
                    continue;
                }
                while string.first().is_some_and(|&c| is_white(c)) {
                    string = &string[1..];
                }
            }
            if !b[i..].starts_with(string) {
                continue;
            }
            let j = string.len();
            // `b`: white space or the end of the line must follow.
            if part.has('b') && b.get(i + j).is_some_and(|&c| !is_white(c)) {
                continue;
            }
            // A middle part may be a prefix of the end part: keep looking for a longer end.
            if part.has('m') {
                if middle_match.is_none() {
                    middle_match = Some((j, k));
                }
                continue;
            }
            if let Some((mlen, _)) = middle_match
                && j > mlen
            {
                middle_match = None;
            }
            if middle_match.is_none() {
                i += j;
            }
            found = Some(k);
            break;
        }
        let matched = if let Some((mlen, k)) = middle_match {
            i += mlen;
            Some(k)
        } else {
            found
        };
        let Some(k) = matched else {
            break;
        };
        if !got_com {
            flags_of = Some(k);
        }
        result = i;
        while i < b.len() && is_white(b[i]) {
            i += 1;
        }
        if include_space {
            result = i;
        }
        got_com = true;
        if !parts[k].has('n') {
            break;
        }
    }
    (result, flags_of)
}

/// Vim's `get_last_leader_offset`: where the last comment leader in `line` starts, and its
/// entry.
pub(crate) fn last_leader_offset(comments: &str, line: &str) -> Option<(usize, usize)> {
    let parts = parts(comments);
    let b = line.as_bytes();
    let mut result = None;
    let mut lower = 0usize;
    let mut i = b.len();
    while i > lower {
        i -= 1;
        let mut found = None;
        for (k, part) in parts.iter().enumerate() {
            let mut string = part.string.as_bytes();
            if string.first().is_some_and(|&c| is_white(c)) {
                if i == 0 || !is_white(b[i - 1]) {
                    continue;
                }
                while string.first().is_some_and(|&c| is_white(c)) {
                    string = &string[1..];
                }
            }
            if !b[i..].starts_with(string) {
                continue;
            }
            let j = string.len();
            if part.has('b') && b.get(i + j).is_some_and(|&c| !is_white(c)) {
                continue;
            }
            // A middle part counts only with nothing but white space before it.
            if part.has('m') && !b[..i].iter().all(|&c| is_white(c)) {
                continue;
            }
            found = Some(k);
            break;
        }
        let Some(k) = found else {
            continue;
        };
        result = Some((i, k));
        if parts[k].has('n') {
            continue;
        }
        lower = i;
        // The leader found may end another leader: look further back for that one.
        let leader = parts[k].string.trim_start_matches([' ', '\t']).as_bytes();
        for (k2, other) in parts.iter().enumerate() {
            if k2 == k {
                continue;
            }
            let s = other.string.trim_start_matches([' ', '\t']).as_bytes();
            if s.is_empty() {
                continue;
            }
            let mut off = s.len().min(i);
            while off > 0 && off + leader.len() > s.len() {
                off -= 1;
                if leader.starts_with(&s[off..]) {
                    lower = lower.min(i - off);
                }
            }
        }
    }
    result
}

/// Vim's `skip_comment` for joining lines ('formatoptions' `j`): how many bytes of comment
/// leader `line` starts with (0 without `process`), and whether the line ends inside a
/// comment.
pub(crate) fn skip_comment(
    comments: &str,
    line: &str,
    process: bool,
    include_space: bool,
) -> (usize, bool) {
    let parts = parts(comments);
    let is_comment = last_leader_offset(comments, line).is_some_and(|(_, k)| !parts[k].has('e'));
    if !process {
        return (0, is_comment);
    }
    let (len, k) = leader_len(comments, line, false, include_space);
    if len == 0 {
        return (0, is_comment);
    }
    // The end part of a three-part comment isn't removed.
    if k.is_some_and(|k| parts[k].has('e')) {
        return (0, is_comment);
    }
    (len, is_comment)
}

/// Vim's `skip_string`: past a C string or character literal at `p`, or `p` itself.
fn skip_string(b: &[u8], mut p: usize) -> usize {
    loop {
        let at = |k: usize| b.get(k).copied().unwrap_or(0);
        if at(p) == b'\'' {
            if at(p + 1) == 0 {
                break;
            }
            let mut i = 2;
            if at(p + 1) == b'\\' && at(p + 2) != 0 {
                i += 1;
                while at(p + i - 1).is_ascii_digit() {
                    i += 1;
                }
            }
            if at(p + i - 1) != 0 && at(p + i) == b'\'' {
                p += i + 1;
                continue;
            }
        } else if at(p) == b'"' {
            p += 1;
            while at(p) != 0 {
                if at(p) == b'\\' && at(p + 1) != 0 {
                    p += 1;
                } else if at(p) == b'"' {
                    break;
                }
                p += 1;
            }
            if at(p) == b'"' {
                p += 1;
                continue;
            }
        } else if at(p) == b'R' && at(p + 1) == b'"' {
            let delim_start = p + 2;
            if let Some(paren) = b[delim_start.min(b.len())..]
                .iter()
                .position(|&c| c == b'(')
            {
                let delim = &b[delim_start..delim_start + paren];
                p += 3;
                while at(p) != 0 {
                    if at(p) == b')'
                        && b[p + 1..].starts_with(delim)
                        && at(p + 1 + delim.len()) == b'"'
                    {
                        p += delim.len() + 1;
                        break;
                    }
                    p += 1;
                }
                if at(p) == b'"' {
                    p += 1;
                    continue;
                }
            }
        }
        break;
    }
    if p >= b.len() {
        b.len().saturating_sub(1)
    } else {
        p
    }
}

/// Vim's `is_pos_in_string`: byte `col` of `line` is inside a C string.
fn is_pos_in_string(line: &str, col: usize) -> bool {
    let b = line.as_bytes();
    let mut p = 0;
    while p < b.len() && p < col {
        p = skip_string(b, p) + 1;
    }
    p > col
}

/// Vim's `check_linecomment`: where a `//` comment starts in `line`, outside strings.
pub(crate) fn check_linecomment(line: &str) -> Option<usize> {
    let b = line.as_bytes();
    let mut p = 0;
    while let Some(off) = b[p..].iter().position(|&c| c == b'/') {
        p += off;
        let after_star_before_star = p > 0 && b[p - 1] == b'*' && b.get(p + 2) == Some(&b'*');
        if b.get(p + 1) == Some(&b'/') && !after_star_before_star && !is_pos_in_string(line, p) {
            return Some(p);
        }
        p += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const C: &str = "sO:* -,mO:*  ,exO:*/,s1:/*,mb:*,ex:*/,:///,://";
    const RUST: &str = "s0:/*!,ex:*/,s1:/*,mb:*,ex:*/,:///,://!,://";

    #[test]
    fn leaders() {
        assert_eq!(leader_len(C, "// x", false, true), (3, Some(7)));
        assert_eq!(leader_len(C, "  /// x", false, true), (6, Some(6)));
        assert_eq!(leader_len(C, "/* x", false, true), (3, Some(3)));
        assert_eq!(leader_len(C, " * x", false, true), (3, Some(4)));
        assert_eq!(leader_len(C, " */", false, true), (3, Some(2)));
        assert_eq!(leader_len(C, "x = 1;", false, true), (0, None));
        assert_eq!(leader_len(RUST, "//! doc", false, true), (4, Some(6)));
        assert_eq!(leader_len("b:#", "#x", false, true), (0, None));
        assert_eq!(leader_len("b:#", "# x", false, false), (1, Some(0)));
        // `n`: nested leaders.
        assert_eq!(leader_len("n:>", "> > quote", false, true), (4, Some(0)));
    }

    #[test]
    fn line_comments_after_code() {
        assert_eq!(check_linecomment("x = 1; // note"), Some(7));
        assert_eq!(check_linecomment("s = \"a//b\"; // c"), Some(12));
        assert_eq!(check_linecomment("a /* */ b"), None);
    }

    #[test]
    fn skipping_comments_for_join() {
        assert_eq!(skip_comment(C, "// more", true, true), (3, true));
        assert_eq!(skip_comment(C, " */", true, true), (0, false));
        assert_eq!(skip_comment(C, "code", true, true), (0, false));
    }
}
