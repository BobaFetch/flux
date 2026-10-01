//! Search motions: `/`, `?`, `n`, `N`, `*`, `#`, `g*`, `g#`, and 'incsearch' while typing.

use flux_core::chars;
use flux_view::search::{self, SearchCmd};
use flux_view::{Editor, Mode};

use crate::engine::Engine;
use crate::key::Key;
use crate::motion::{Kind, Motion, Pending, Target, Want};
use crate::normal::set_pcmark;
use crate::parse::{Command, VisualCommand};
use crate::util::{self, pos};

/// A command waiting for its search pattern to be typed.
#[derive(Debug, Clone)]
pub(crate) enum PendingSearch {
    Normal(Command),
    Visual(VisualCommand),
}

/// The view before 'incsearch' moved it, to go back to.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SavedView {
    pub(crate) top: usize,
    pub(crate) cursor: flux_view::Cursor,
}

impl SavedView {
    pub(crate) fn of(editor: &Editor) -> Self {
        Self {
            top: editor.window.top,
            cursor: editor.cursor(),
        }
    }
}

impl Engine {
    /// Run a search motion, returning its target. Remembers the pattern, shows `/pattern` and
    /// the match count, or an error.
    pub(crate) fn search_motion(
        &mut self,
        editor: &mut Editor,
        motion: Motion,
        count: Option<usize>,
        _pending: Pending,
    ) -> Option<Target> {
        let n = count.unwrap_or(1).max(1);
        let result = match motion {
            Motion::Search { forward } => {
                let input = self.search_input.take().unwrap_or_default();
                search::do_search(
                    editor,
                    SearchCmd::Typed {
                        forward,
                        input: &input,
                    },
                    n,
                )
            }
            Motion::SearchNext { reverse } => {
                let before = editor.cursor();
                let r = search::do_search(editor, SearchCmd::Next { reverse }, n);
                // Stuck on the cursor (an offset at the end of the buffer): try one further.
                match r {
                    Some(found)
                        if !found.wrapped
                            && (found.pos.line, found.pos.col) == (before.line, before.col) =>
                    {
                        search::do_search(editor, SearchCmd::Next { reverse }, n + 1)
                    }
                    other => other,
                }
            }
            Motion::SearchWord { forward, whole } => self.search_word(editor, forward, whole, n),
            _ => unreachable!("not a search motion"),
        }?;
        // `/$` finds the end of the line; the cursor stays on a character.
        let text = editor.text();
        let line = result.pos.line.min(text.last_line());
        let s = text.line_str(line);
        let col = result.pos.col.min(chars::last_grapheme(&s));
        let kind = if result.linewise {
            Kind::Linewise
        } else if result.inclusive {
            Kind::Inclusive
        } else {
            Kind::Exclusive
        };
        Some(Target {
            pos: pos(line, col),
            kind,
            want: Want::Column,
            numbered_register: true,
        })
    }

    /// `*`, `#`, `g*`, `g#` (Vim's `nv_ident`): search for the keyword under or after the
    /// cursor. The jump is remembered and the cursor moved to the word's start first, even if
    /// the search then fails.
    fn search_word(
        &mut self,
        editor: &mut Editor,
        forward: bool,
        whole: bool,
        count: usize,
    ) -> Option<search::SearchResult> {
        let cur = editor.cursor();
        let line = util::line(editor, cur.line);
        let (start, end) = match find_ident(&line, cur.col) {
            Ok(r) => r,
            Err(e) => {
                editor.error(e);
                return None;
            }
        };
        set_pcmark(editor);
        editor.window.cursor.col = start;
        let word: Vec<char> = line.chars().skip(start).take(end - start).collect();
        let escape: &[char] = if forward {
            &['/', '.', '*', '~', '[', '^', '$', '\\']
        } else {
            &['/', '?', '.', '*', '~', '[', '^', '$', '\\']
        };
        let mut pat = String::new();
        if whole && chars::is_keyword(word[0]) {
            pat.push_str("\\<");
        }
        for &c in &word {
            if escape.contains(&c) {
                pat.push('\\');
            }
            pat.push(c);
        }
        if whole && chars::is_keyword(word[word.len() - 1]) {
            pat.push_str("\\>");
        }
        editor.search.add_history(true, &pat);
        search::do_search(
            editor,
            SearchCmd::Word {
                forward,
                pattern: &pat,
            },
            count,
        )
    }

    /// Open the search command line for a command whose motion is `/` or `?`.
    pub(crate) fn start_search(
        &mut self,
        editor: &mut Editor,
        forward: bool,
        pending: PendingSearch,
    ) {
        editor.cmdline_return = if matches!(pending, PendingSearch::Visual(_)) {
            Mode::Visual
        } else {
            Mode::Normal
        };
        self.search_cmd = Some(pending);
        self.enter_cmdline(editor);
        editor.cmdline_kind = if forward { '/' } else { '?' };
    }

    /// Leave the search command line: with the typed text, run the waiting command.
    pub(crate) fn finish_search(&mut self, editor: &mut Editor, typed: Option<String>) {
        self.restore_view(editor);
        editor.incsearch = None;
        editor.incsearch_pattern = None;
        editor.mode = editor.cmdline_return;
        editor.cmdline_return = Mode::Normal;
        let Some(pending) = self.search_cmd.take() else {
            return;
        };
        // Escaped: back to Normal or Visual mode as it was.
        let Some(typed) = typed else {
            return;
        };
        editor.search.add_history(true, &typed);
        // `.` and macros replay the typed pattern too.
        let mut typed_keys: Vec<Key> = typed.chars().map(Key::char).collect();
        typed_keys.push(Key::plain(crate::key::KeyCode::Enter));
        self.search_input = Some(typed);
        match pending {
            PendingSearch::Normal(mut cmd) => {
                cmd.keys.extend(typed_keys);
                self.run(editor, cmd);
                if editor.mode == Mode::Normal {
                    self.commit(editor);
                    if self.ctrl_o.is_some() {
                        self.finish_ctrl_o(editor);
                    }
                }
            }
            PendingSearch::Visual(mut cmd) => {
                cmd.keys.extend(typed_keys);
                self.run_visual(editor, cmd);
                if editor.mode != Mode::Insert {
                    self.commit(editor);
                }
            }
        }
        self.search_input = None;
    }

    pub(crate) fn restore_view(&mut self, editor: &mut Editor) {
        if let Some(v) = self.saved_view.take() {
            editor.window.top = v.top;
            editor.window.cursor = v.cursor;
        }
    }

    /// 'incsearch': show where the pattern typed so far matches, scrolling the window to it.
    pub(crate) fn update_incsearch(&mut self, editor: &mut Editor) {
        if editor.cmdline_kind == ':' || !editor.options.incsearch {
            return;
        }
        let Some(saved) = self.saved_view else {
            return;
        };
        editor.window.top = saved.top;
        editor.window.cursor = saved.cursor;
        editor.incsearch = None;
        let forward = editor.cmdline_kind == '/';
        let (pat, _) = search::split_pattern(&editor.cmdline, editor.cmdline_kind);
        editor.incsearch_pattern = (!pat.is_empty()).then(|| pat.clone());
        if pat.is_empty() {
            return;
        }
        let Ok(pattern) = search::compile(editor, &pat, false) else {
            return;
        };
        let hay = search::Haystack::new(editor.text());
        let flags = search::SearchFlags {
            start: false,
            end: false,
            wrapscan: editor.options.wrapscan,
        };
        let cur = saved.cursor;
        let Ok(found) = search::searchit(
            &hay,
            &pattern,
            (cur.line as isize, cur.col),
            forward,
            1,
            flags,
        ) else {
            return;
        };
        drop(hay);
        editor.incsearch = Some((found.pos, found.end));
        // Scroll as the search itself would.
        let text = editor.text();
        let line = found.pos.line.min(text.last_line());
        let col = found
            .pos
            .col
            .min(chars::last_grapheme(&text.line_str(line)));
        // Like Neovim, the cursor (and so the ruler) shows the match until the search ends.
        editor.window.cursor = pos(line, col);
        editor.with_window(|w, m| w.scroll_to_cursor(m));
    }
}

/// Vim's `find_ident_under_cursor` with `FIND_IDENT | FIND_STRING`: the keyword under or
/// after the cursor, or else the non-blank text there. Char offsets `[start, end)`.
pub(crate) fn find_ident(line: &str, cursor: usize) -> Result<(usize, usize), &'static str> {
    let chars: Vec<char> = line.chars().collect();
    let class = |i: usize| chars.get(i).map_or(0, |&c| chars::class(c, false));
    let mut col = 0;
    let mut this_class = 0;
    let mut found_i = 0;
    for i in 0..2 {
        // 1. Skip to the start of a keyword (i == 0) or of any non-blank text.
        col = cursor;
        while col < chars.len() {
            let c = class(col);
            if c != 0 && (i == 1 || c != 1) {
                break;
            }
            col += 1;
        }
        // 2. Back up to its start.
        this_class = class(col);
        while col > 0 && this_class != 0 {
            let prev = class(col - 1);
            if this_class != prev {
                break;
            }
            col -= 1;
        }
        if this_class > 2 {
            this_class = 2;
        }
        found_i = i;
        if this_class == 2 {
            break;
        }
    }
    if col >= chars.len() || (found_i == 0 && this_class != 2) {
        return Err("E348: No string under cursor");
    }
    // 3. Find its end.
    let start = col;
    let first_class = class(start);
    let mut end = start;
    while end < chars.len() {
        let c = class(end);
        let same = if found_i == 0 {
            c == first_class
        } else {
            c != 0
        };
        if !same {
            break;
        }
        end += 1;
    }
    Ok((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ident_under_cursor() {
        assert_eq!(find_ident("foo bar", 1), Ok((0, 3)));
        assert_eq!(find_ident("foo bar", 3), Ok((4, 7)));
        assert_eq!(find_ident("  (x)", 0), Ok((3, 4)));
        assert_eq!(find_ident("a ++ ", 2), Ok((2, 4)));
        assert_eq!(find_ident("   ", 0), Err("E348: No string under cursor"));
    }
}
