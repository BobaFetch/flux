//! Opening a new line (`<CR>` in Insert mode, `o`, `O`), ported from Vim's `open_line` in
//! `change.c`: 'autoindent', the comment leader ('comments' with 'formatoptions' `r` and `o`),
//! and then the indenter ('indentkeys' `o` / `O`).

use flux_core::Edit;
use flux_view::Editor;

use crate::comments::{self, Part};
use crate::engine::Engine;
use crate::indent::{self, Typed, When};
use crate::util::{self, pos};

fn is_white(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

/// The byte offset of char column `col` in `s`.
fn byte_col(s: &str, col: usize) -> usize {
    s.char_indices().nth(col).map_or(s.len(), |(i, _)| i)
}

/// Screen width of `s` (tabs as in 'tabstop' from column 0).
fn width(s: &str, ts: usize) -> usize {
    let mut w = 0;
    for c in s.chars() {
        w += if c == '\t' {
            ts - w % ts
        } else {
            unicode_width::UnicodeWidthChar::width(c).unwrap_or(1)
        };
    }
    w
}

/// How to open the line (Vim's `open_line` flags).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct OpenFlags {
    /// `O`, not `o` / `<CR>`.
    pub backward: bool,
    /// `<CR>` in Insert mode: the text after the cursor goes to the new line.
    pub insert: bool,
    /// The current line holds only an automatic indent (Vim's `did_ai`).
    pub did_ai: bool,
    /// Repeat the comment leader (`OPENLINE_DO_COM`); `None` asks 'formatoptions' (`r` for
    /// `<CR>`, `o` for `o` and `O`).
    pub do_com: Option<bool>,
    /// Drop the blanks the moved text starts with even without 'autoindent'
    /// (`OPENLINE_DELSPACES`).
    pub del_spaces: bool,
}

/// What opening a line found out, for the caller.
pub(crate) struct Opened {
    /// The new line.
    pub line: usize,
    /// Its indent or comment leader was added automatically (Vim's `did_ai`): it's removed
    /// again if nothing is typed.
    pub did_ai: bool,
    /// The last character of a comment's end, when typing it first on the new line ends the
    /// comment ('comments' flag `x`, Vim's `end_comment_pending`).
    pub end_comment_pending: Option<char>,
}

/// The comment leader to put on the new line, from the line `saved` that starts with a
/// leader of `lead_len` bytes (entry `k` of 'comments'), and the indent it implies.
struct Leader {
    text: String,
    /// The new indent (Vim's `newindent`) when the leader changes it.
    indent: Option<usize>,
}

impl Engine {
    /// Vim's `open_line`. `forward` is `o` and `<CR>`, otherwise `O`; `insert` is `<CR>` in
    /// Insert mode, which moves the text after the cursor to the new line.
    pub(crate) fn open_line_vim(&mut self, editor: &mut Editor, flags: OpenFlags) -> Opened {
        let forward = !flags.backward;
        let insert = flags.insert;
        let did_ai_before = flags.did_ai;
        let o = editor.buf_opts().clone();
        let ts = o.tabstop;
        let cur = editor.cursor();
        let full = util::line(editor, cur.line);
        let bcol = byte_col(&full, cur.col);
        let (saved, extra): (String, Option<String>) = if insert {
            (full[..bcol].to_string(), Some(full[bcol..].to_string()))
        } else {
            (full.clone(), None)
        };
        let trunc_line = forward && did_ai_before;
        let mut newindent = if o.autoindent {
            util::indent_width(&saved, ts)
        } else {
            0
        };
        let mut did_ai = o.autoindent;
        let typed = if forward {
            Typed::OpenBelow
        } else {
            Typed::OpenAbove
        };
        let line_white = full.chars().all(util::is_white);
        let do_cindent = indent::cindent_on(editor)
            && indent::in_cinkeys(editor, typed, When::After, line_white);

        // The comment leader the line starts with, if it is to be repeated.
        let fo = &o.formatoptions;
        let do_com = flags.do_com.unwrap_or(if insert {
            fo.contains('r')
        } else {
            fo.contains('o')
        });
        let parts = comments::parts(&o.comments);
        let (mut lead_len, mut lead_k) = if do_com {
            comments::leader_len(&o.comments, &saved, !forward, true)
        } else {
            (0, None)
        };
        let mut comment_start = 0;
        if do_com && lead_len == 0 && o.cindent && do_cindent && forward && !fo.contains('/') {
            // A `//` comment after code.
            if let Some(cs) = comments::check_linecomment(&saved) {
                let (l, k) = comments::leader_len(&o.comments, &saved[cs..], false, true);
                if l != 0 {
                    lead_len = l + cs;
                    lead_k = k;
                    comment_start = cs;
                }
            }
        }
        let mut end_comment_pending = None;
        let mut leader: Option<Leader> = None;
        if lead_len > 0
            && let Some(k) = lead_k
        {
            let found = leader_for(
                &parts,
                k,
                &saved,
                &extra,
                bcol,
                lead_len,
                comment_start,
                forward,
                o.autoindent,
                ts,
                &mut end_comment_pending,
            );
            match found {
                LeaderResult::Leader(l) => leader = Some(l),
                LeaderResult::None => {}
                LeaderResult::CommentEnd(at) => {
                    // A finished C comment: line up with the line the comment started on.
                    if saved[at..].starts_with("*/") && o.autoindent {
                        let text = editor.text();
                        let start = (0..=cur.line)
                            .rev()
                            .find(|&l| text.line_str(l).contains("/*"));
                        if let Some(l) = start {
                            newindent = util::indent_width(&text.line_str(l), ts);
                        }
                    }
                }
            }
        }

        // The text that goes to the new line: the leader, then (for `<CR>`) the text after
        // the cursor, without its leading blanks with 'autoindent'.
        let mut extra = extra.unwrap_or_default();
        if o.autoindent || flags.del_spaces {
            extra = extra.trim_start_matches([' ', '\t']).to_string();
        }
        let mut newcol;
        let new_text = match &leader {
            Some(l) => {
                did_ai = true;
                if let Some(ind) = l.indent {
                    newindent = ind;
                }
                let mut text = l.text.clone();
                newcol = text.chars().count();
                // The indent is set below: take it out of the leader.
                if newindent > 0 {
                    let trimmed = text.trim_start_matches([' ', '\t']).to_string();
                    newcol -= text.chars().count() - trimmed.chars().count();
                    text = trimmed;
                }
                text.push_str(&extra);
                text
            }
            None => {
                newcol = 0;
                end_comment_pending = None;
                extra
            }
        };
        let indent_str = if newindent > 0 {
            util::make_indent(newindent, &o)
        } else {
            String::new()
        };
        newcol += indent_str.chars().count();
        let new_line = format!("{indent_str}{new_text}");

        let t = editor.text();
        let line_start = t.line_start(cur.line);
        let line_end = line_start + t.line_len(cur.line);
        let new_lnum = if forward {
            // The current line keeps the text before the cursor, without trailing blanks
            // when it held only an automatic indent.
            let mut left = saved.clone();
            if trunc_line {
                let keep = left.trim_end_matches([' ', '\t']).len();
                left.truncate(keep);
            }
            let replace_from = if insert || trunc_line {
                line_start
            } else {
                line_end
            };
            let replacement = if insert || trunc_line {
                format!("{left}\n{new_line}")
            } else {
                format!("\n{new_line}")
            };
            self.edit(editor, Edit::replace(replace_from..line_end, replacement));
            cur.line + 1
        } else {
            self.edit(editor, Edit::insert(line_start, format!("{new_line}\n")));
            cur.line
        };
        editor.window.cursor = pos(new_lnum, newcol);

        if do_cindent && self.fix_this_line(editor) {
            did_ai = true;
        }
        Opened {
            line: new_lnum,
            did_ai,
            end_comment_pending,
        }
    }
}

enum LeaderResult {
    Leader(Leader),
    /// No leader to repeat.
    None,
    /// The comment ends on this line, at this byte (Vim's `comment_end`).
    CommentEnd(usize),
}

/// The leader-flags part of Vim's `open_line`.
#[allow(clippy::too_many_arguments)]
fn leader_for(
    parts: &[Part<'_>],
    k: usize,
    saved: &str,
    extra: &Option<String>,
    bcol: usize,
    lead_len: usize,
    comment_start: usize,
    forward: bool,
    autoindent: bool,
    ts: usize,
    end_comment_pending: &mut Option<char>,
) -> LeaderResult {
    let part = parts[k];
    let sb = saved.as_bytes();
    let mut lead_repl: Option<String> = None;
    let mut extra_space = false;
    let mut require_blank = false;
    for flag in part.flags.chars() {
        match flag {
            'b' => require_blank = true,
            's' | 'm' => {
                if flag == 's' && !forward {
                    // `O` on the start of a comment: no leader.
                    return LeaderResult::None;
                }
                let (middle, end) = if flag == 's' {
                    require_blank = false;
                    (parts.get(k + 1), parts.get(k + 2))
                } else {
                    (Some(&parts[k]), parts.get(k + 1))
                };
                let (Some(middle), Some(end)) = (middle, end) else {
                    return LeaderResult::None;
                };
                if flag == 's' && middle.has('b') {
                    require_blank = true;
                }
                if end.has('x') {
                    *end_comment_pending = end.string.chars().last();
                }
                // The comment ends on the same line: no leader.
                if forward
                    && !end.string.is_empty()
                    && let Some(at) = saved[lead_len.min(saved.len())..].find(end.string)
                {
                    return LeaderResult::CommentEnd(lead_len + at);
                }
                if flag == 's' {
                    lead_repl = Some(middle.string.to_string());
                }
                // `<CR>` right after the start leader: a space after the middle one.
                if !is_white(sb[lead_len - 1])
                    && ((extra.is_some() && bcol == lead_len)
                        || (extra.is_none() && saved.len() == lead_len)
                        || require_blank)
                {
                    extra_space = true;
                }
                break;
            }
            'e' => {
                if forward {
                    // `o` on the end of a comment: no leader.
                    let first = saved.len() - saved.trim_start_matches([' ', '\t']).len();
                    return LeaderResult::CommentEnd(first);
                }
                // `O` on the end of a comment: the middle leader.
                lead_repl = Some(
                    parts
                        .get(k.wrapping_sub(1))
                        .map_or("", |p| p.string)
                        .to_string(),
                );
                extra_space = true;
                if part.has('x') {
                    *end_comment_pending = part.string.chars().last();
                }
                break;
            }
            'f' => {
                if !forward {
                    return LeaderResult::None;
                }
                lead_repl = Some(String::new());
                break;
            }
            _ => {}
        }
    }

    let mut leader: Vec<u8> = sb[..lead_len].to_vec();
    // Code before a `//` comment becomes blanks.
    for b in leader.iter_mut().take(comment_start) {
        if !is_white(*b) {
            *b = b' ';
        }
    }
    let mut indent = None;
    if let Some(repl) = &lead_repl {
        // The offset and alignment flags of the entry.
        let mut off: isize = 0;
        let mut right = false;
        let mut chars = part.flags.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                'r' => right = true,
                'l' => right = false,
                c if c.is_ascii_digit() || c == '-' => {
                    let mut s = c.to_string();
                    while let Some(&d) = chars.peek() {
                        if d.is_ascii_digit() {
                            s.push(d);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    off = s.parse().unwrap_or(0);
                }
                _ => {}
            }
        }
        let text = String::from_utf8_lossy(&leader).into_owned();
        let indent_len = text.len() - text.trim_start_matches([' ', '\t']).len();
        let repl_size = width(repl, ts);
        let body: Vec<char> = text[indent_len..].chars().collect();
        let mut new_text = text[..indent_len].to_string();
        if right {
            // Right-adjusted: the replacement ends where the old leader's text ended.
            let last = body
                .iter()
                .rposition(|c| !util::is_white(*c))
                .map_or(0, |i| i + 1);
            let start = last.saturating_sub(repl.chars().count());
            let mut rest: Vec<char> = body.clone();
            let mut rebuilt: String = rest[..start].iter().map(|_| ' ').collect();
            rebuilt.push_str(repl);
            rebuilt.extend(rest.drain(last.max(start)..));
            new_text.push_str(&rebuilt);
        } else {
            // Left-adjusted: replace as many characters as the replacement is wide, then
            // blank out what's left of the old leader (keeping tabs).
            let mut i = 0;
            let mut used = 0;
            while i < body.len() {
                let w = width(&body[i].to_string(), ts);
                if used + w > repl_size {
                    break;
                }
                used += w;
                i += 1;
            }
            new_text.push_str(repl);
            let mut rest: Vec<char> = body[i..].to_vec();
            let mut j = 0;
            while j < rest.len() {
                if !util::is_white(rest[j]) {
                    if rest.get(j + 1) == Some(&'\t') {
                        rest.remove(j);
                        continue;
                    }
                    rest[j] = ' ';
                }
                j += 1;
            }
            new_text.extend(rest);
        }
        // The indent the leader gives, with the entry's offset.
        let mut newindent = if autoindent {
            util::indent_width(&new_text, ts) as isize
        } else {
            0
        };
        if newindent + off < 0 {
            off = -newindent;
            newindent = 0;
        } else {
            newindent += off;
        }
        // Trailing spaces make up for the offset, so the text stays aligned.
        let has_tab = new_text.trim_start_matches([' ', '\t']).contains('\t');
        while off > 0 && new_text.ends_with(' ') && !has_tab {
            new_text.pop();
            off -= 1;
        }
        if new_text.ends_with([' ', '\t']) {
            extra_space = false;
        }
        if autoindent {
            indent = Some(newindent.max(0) as usize);
        }
        leader = new_text.into_bytes();
    }
    let mut text = String::from_utf8_lossy(&leader).into_owned();
    if extra_space {
        text.push(' ');
    }
    LeaderResult::Leader(Leader { text, indent })
}
