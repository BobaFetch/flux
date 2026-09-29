//! Key dispatch. M0 handles viewing only: line moves, jumps, scrolling and the command line.
//! M1 replaces the Normal-mode part with the operator/motion state machine.

use flux_view::{Editor, Mode};

use crate::ex;
use crate::key::{Key, KeyCode, Modifiers};

#[derive(Debug, Default)]
pub struct Engine {
    /// A prefix key waiting for the rest of its command, like the first `g` of `gg`.
    pending: Option<char>,
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn handle_key(&mut self, editor: &mut Editor, key: Key) {
        match editor.mode {
            Mode::Normal => self.normal(editor, key),
            Mode::CmdLine => cmdline(editor, key),
        }
    }

    fn normal(&mut self, editor: &mut Editor, key: Key) {
        if let Some(prefix) = self.pending.take() {
            if prefix == 'g' && key == Key::char('g') {
                editor.with_window(|win, m| win.set_cursor_line(0, m));
            }
            return;
        }

        let ctrl = |c| Key::ctrl(c) == key;
        if key == Key::char('j') || key == Key::plain(KeyCode::Down) || ctrl('n') || ctrl('j') {
            editor.with_window(|win, m| {
                let line = win.cursor.line + 1;
                if line < m.text.line_count() {
                    win.set_cursor_line(line, m);
                }
            });
        } else if key == Key::char('k') || key == Key::plain(KeyCode::Up) || ctrl('p') {
            editor.with_window(|win, m| {
                if let Some(line) = win.cursor.line.checked_sub(1) {
                    win.set_cursor_line(line, m);
                }
            });
        } else if key == Key::char('G') {
            editor.with_window(|win, m| win.set_cursor_line(m.text.line_count() - 1, m));
        } else if key == Key::char('g') {
            self.pending = Some('g');
        } else if ctrl('e') {
            editor.with_window(|win, m| win.scroll_lines_down(1, m));
        } else if ctrl('y') {
            editor.with_window(|win, m| win.scroll_lines_up(1, m));
        } else if ctrl('d') {
            editor.with_window(|win, m| win.scroll_half_down(m));
        } else if ctrl('u') {
            editor.with_window(|win, m| win.scroll_half_up(m));
        } else if ctrl('f') || key == Key::plain(KeyCode::PageDown) {
            editor.with_window(|win, m| win.page_down(m));
        } else if ctrl('b') || key == Key::plain(KeyCode::PageUp) {
            editor.with_window(|win, m| win.page_up(m));
        } else if key == Key::char(':') {
            editor.mode = Mode::CmdLine;
            editor.cmdline.clear();
            editor.message = None;
        }
    }
}

fn cmdline(editor: &mut Editor, key: Key) {
    let leave = |editor: &mut Editor| {
        editor.mode = Mode::Normal;
        editor.cmdline.clear();
    };
    match (key.code, key.mods) {
        (KeyCode::Enter, _) => {
            let line = std::mem::take(&mut editor.cmdline);
            editor.mode = Mode::Normal;
            // The typed command stays visible, as in Vim, unless the command reports something.
            editor.info(format!(":{line}"));
            ex::execute(editor, &line);
        }
        (KeyCode::Esc, _) | (KeyCode::Char('c'), Modifiers::CTRL) => leave(editor),
        (KeyCode::Backspace, _) | (KeyCode::Char('h'), Modifiers::CTRL) => {
            if editor.cmdline.pop().is_none() {
                leave(editor);
            }
        }
        (KeyCode::Char('u'), Modifiers::CTRL) => editor.cmdline.clear(),
        (KeyCode::Char('w'), Modifiers::CTRL) => delete_word_before(&mut editor.cmdline),
        (KeyCode::Char(c), Modifiers::NONE) => editor.cmdline.push(c),
        _ => {}
    }
}

/// `CTRL-W` on the command line: delete trailing spaces, then a run of keyword or non-keyword
/// characters.
fn delete_word_before(s: &mut String) {
    let trimmed = s.trim_end_matches(' ').len();
    s.truncate(trimmed);
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let Some(last) = s.chars().last() else {
        return;
    };
    let keep = s
        .char_indices()
        .rev()
        .find(|&(_, c)| is_word(c) != is_word(last) || c == ' ')
        .map_or(0, |(i, c)| i + c.len_utf8());
    s.truncate(keep);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_keys;

    fn feed(editor: &mut Editor, engine: &mut Engine, keys: &str) {
        for key in parse_keys(keys) {
            engine.handle_key(editor, key);
        }
    }

    #[test]
    fn cmdline_editing() {
        let mut editor = Editor::new(80, 24);
        let mut engine = Engine::new();
        feed(&mut editor, &mut engine, ":foo bar<C-w>");
        assert_eq!(editor.cmdline, "foo ");
        feed(&mut editor, &mut engine, "<BS><BS><BS><BS>");
        assert_eq!(editor.mode, Mode::CmdLine);
        feed(&mut editor, &mut engine, "<BS>");
        assert_eq!(editor.mode, Mode::Normal);
        feed(&mut editor, &mut engine, ":q<Esc>");
        assert!(!editor.quit);
        feed(&mut editor, &mut engine, ":1<CR>");
        assert_eq!(editor.message.as_ref().unwrap().text, ":1");
        feed(&mut editor, &mut engine, ":q<CR>");
        assert!(editor.quit);
    }

    #[test]
    fn ctrl_w_word_classes() {
        for (before, after) in [("a.b", "a."), ("foo  ", ""), ("x foo..", "x foo"), ("", "")] {
            let mut s = before.to_string();
            delete_word_before(&mut s);
            assert_eq!(s, after, "CTRL-W on {before:?}");
        }
    }
}
