//! Asking for an answer the way Neovim's default `vim.ui.input()` and `vim.ui.select()` do: Vim's
//! `input()`, a line typed on the command line after a prompt, and `inputlist()`, a numbered
//! list with a number typed below it.

use flux_view::{Editor, Mode};

use crate::engine::Engine;
use crate::key::{Key, KeyCode, Modifiers};
use crate::lsp::{code_action, rename};

/// What an answer is for.
#[derive(Debug)]
pub(crate) enum Prompt {
    /// `input()`: the command line, with `editor.cmdline_kind` `@`.
    Input(InputFor),
    /// `inputlist()`: the list is the message on screen.
    Number {
        typed: String,
        prompt: String,
        list: ListFor,
    },
}

#[derive(Debug)]
pub(crate) enum InputFor {
    Rename(rename::Asked),
}

#[derive(Debug)]
pub(crate) enum ListFor {
    CodeAction(code_action::Choices),
}

/// Vim's prompt below an `inputlist()`.
const NUMBER_PROMPT: &str =
    "Type number and <Enter> or click with the mouse (q or empty cancels): ";

impl Engine {
    /// Ask for a line of text after `prompt`, starting with `default` (`input()`).
    pub(crate) fn ask_input(
        &mut self,
        editor: &mut Editor,
        prompt: &str,
        default: &str,
        what: InputFor,
    ) {
        editor.mode = Mode::CmdLine;
        editor.cmdline_kind = '@';
        editor.cmdline_prompt = prompt.to_string();
        editor.cmdline = default.to_string();
        editor.cmdline_pos = default.chars().count();
        editor.message = None;
        self.prompt = Some(Prompt::Input(what));
    }

    /// Show `lines` (a title, then the choices numbered from 1) and ask for a number
    /// (`inputlist()`).
    pub(crate) fn ask_number(&mut self, editor: &mut Editor, lines: &[String], what: ListFor) {
        editor.full_message(lines.join("\n"));
        editor.hit_enter = true;
        editor.number_prompt = Some(NUMBER_PROMPT.to_string());
        self.prompt = Some(Prompt::Number {
            typed: String::new(),
            prompt: NUMBER_PROMPT.to_string(),
            list: what,
        });
    }

    /// A key typed while a prompt waits. Returns whether the prompt took it.
    pub(crate) fn prompt_key(&mut self, editor: &mut Editor, key: Key) -> bool {
        match self.prompt.take() {
            None => false,
            Some(Prompt::Input(what)) => {
                if editor.mode != Mode::CmdLine || editor.cmdline_kind != '@' {
                    return false;
                }
                self.input_key(editor, key, what);
                true
            }
            Some(Prompt::Number {
                mut typed,
                prompt,
                list: list_for,
            }) => {
                let ch = (key.mods == Modifiers::NONE)
                    .then(|| key.typed_char())
                    .flatten();
                // Vim's `get_number`: digits, backspace, and what ends it; other keys are
                // ignored.
                let done = match (key.code, ch) {
                    (_, Some(c)) if c.is_ascii_digit() => {
                        typed.push(c);
                        None
                    }
                    (KeyCode::Backspace | KeyCode::Delete, _) => {
                        typed.pop();
                        None
                    }
                    _ if key == Key::ctrl('h') => {
                        typed.pop();
                        None
                    }
                    (KeyCode::Esc, _) | (_, Some('q')) => Some(0),
                    _ if key == Key::ctrl('c') => Some(0),
                    (KeyCode::Enter, _) => Some(typed.parse::<usize>().unwrap_or(0)),
                    _ if key == Key::ctrl('j') || key == Key::ctrl('m') => {
                        Some(typed.parse::<usize>().unwrap_or(0))
                    }
                    _ => None,
                };
                match done {
                    None => {
                        editor.number_prompt = Some(format!("{prompt}{typed}"));
                        self.prompt = Some(Prompt::Number {
                            typed,
                            prompt,
                            list: list_for,
                        });
                    }
                    Some(n) => {
                        let list = editor.message.take().map(|m| m.text).unwrap_or_default();
                        let answered = format!("{list}\n{prompt}{typed}");
                        editor.number_prompt = None;
                        editor.hit_enter = false;
                        match list_for {
                            ListFor::CodeAction(choices) => {
                                code_action::chosen(self, editor, choices, n)
                            }
                        }
                        // A message given right away goes below the list, which is still on
                        // the screen, and waits for a key.
                        if editor.message.is_some() {
                            editor.hit_enter = true;
                            editor.message_above = Some(answered);
                        }
                    }
                }
                true
            }
        }
    }

    fn input_key(&mut self, editor: &mut Editor, key: Key, what: InputFor) {
        let enter =
            key == Key::plain(KeyCode::Enter) || key == Key::ctrl('m') || key == Key::ctrl('j');
        let cancel = key == Key::plain(KeyCode::Esc)
            || key == Key::ctrl('c')
            // Backspace on an empty line leaves, as on any command line.
            || (editor.cmdline.is_empty()
                && (key == Key::plain(KeyCode::Backspace) || key == Key::ctrl('h')));
        if !enter && !cancel {
            // Its own history isn't kept: the arrows do nothing.
            if !matches!(
                key.code,
                KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown
            ) {
                self.cmdline_edit(editor, key);
            }
            self.prompt = Some(Prompt::Input(what));
            return;
        }
        let line = std::mem::take(&mut editor.cmdline);
        let prompt = std::mem::take(&mut editor.cmdline_prompt);
        editor.cmdline_pos = 0;
        editor.cmdline_kind = ':';
        editor.mode = Mode::Normal;
        let answer = if enter {
            // What was typed stays on the command line.
            editor.info(format!("{prompt}{line}"));
            Some(line)
        } else {
            None
        };
        match what {
            InputFor::Rename(asked) => rename::answered(editor, asked, answer),
        }
    }
}
