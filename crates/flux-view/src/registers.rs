//! Vim's registers: where yanked and deleted text goes, and where puts come from.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterKind {
    Char,
    Line,
    Block,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Register {
    /// Lines joined with `\n`; a linewise register has no trailing newline.
    pub text: String,
    pub kind: RegisterKind,
}

impl Register {
    pub fn new(text: impl Into<String>, kind: RegisterKind) -> Self {
        Self {
            text: text.into(),
            kind,
        }
    }
}

#[derive(Debug)]
pub struct Registers {
    contents: HashMap<char, Register>,
    /// The register `""` currently refers to: the last one written.
    unnamed: char,
}

impl Default for Registers {
    fn default() -> Self {
        Self {
            contents: HashMap::new(),
            unnamed: '0',
        }
    }
}

/// Registers a command can name after `"`.
pub fn is_valid_name(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '"' | '-' | '_' | '+' | '*' | '.' | ':' | '/' | '%')
}

impl Registers {
    /// The contents of register `name`, or of the unnamed register.
    pub fn get(&self, name: Option<char>) -> Option<&Register> {
        let name = match name {
            None | Some('"') => self.unnamed,
            Some(c) => c.to_ascii_lowercase(),
        };
        self.contents.get(&name)
    }

    /// Store yanked text: in `name` if given, otherwise in `"0`.
    pub fn yank(&mut self, name: Option<char>, reg: Register) {
        match name {
            Some('_') => {}
            None | Some('"') => self.set('0', reg),
            Some(c) => self.write_named(c, reg),
        }
    }

    /// Store deleted text: in `name` if given; otherwise small deletes (within one line) go to
    /// `"-` and others shift through `"1`–`"9`. (Vim's documentation says deletes with `%`, `{`,
    /// `}` and searches always use `"1`, but Neovim 0.12 puts a small `d%` only in `"-`; flux
    /// follows Neovim, so `numbered` currently changes nothing.)
    pub fn delete(&mut self, name: Option<char>, reg: Register, numbered: bool) {
        let _ = numbered;
        let named = match name {
            Some('_') => return,
            None | Some('"') => None,
            Some(c) => Some(c),
        };
        // Like Vim's `op_delete`: a delete of more than a line always shifts into `"1`, even
        // into a named register; a small one goes to `"-` only when no register is named.
        let big = reg.kind != RegisterKind::Char || reg.text.contains('\n');
        if big {
            for n in (1..9).rev() {
                let from = char::from_digit(n, 10).unwrap();
                let to = char::from_digit(n + 1, 10).unwrap();
                if let Some(r) = self.contents.remove(&from) {
                    self.contents.insert(to, r);
                }
            }
            self.set('1', reg.clone());
        } else if named.is_none() {
            self.set('-', reg.clone());
        }
        if let Some(c) = named {
            self.write_named(c, reg);
        }
    }

    /// Write a register the user named, appending for `A`–`Z`.
    fn write_named(&mut self, name: char, reg: Register) {
        let lower = name.to_ascii_lowercase();
        if name.is_ascii_uppercase()
            && let Some(existing) = self.contents.get_mut(&lower)
        {
            if existing.kind == RegisterKind::Line || reg.kind == RegisterKind::Line {
                existing.text = format!("{}\n{}", existing.text, reg.text);
                existing.kind = RegisterKind::Line;
            } else {
                existing.text.push_str(&reg.text);
            }
            self.unnamed = lower;
            return;
        }
        self.set(lower, reg);
    }

    /// Set a read-only register (`".`, `":`), which doesn't change what `""` refers to.
    pub fn set_readonly(&mut self, name: char, text: String) {
        self.contents
            .insert(name, Register::new(text, RegisterKind::Char));
    }

    fn set(&mut self, name: char, reg: Register) {
        self.contents.insert(name, reg);
        self.unnamed = name;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Register {
        Register::new(s, RegisterKind::Char)
    }

    #[test]
    fn yank_and_small_delete() {
        let mut r = Registers::default();
        r.yank(None, chars("y"));
        assert_eq!(r.get(None).unwrap().text, "y");
        r.delete(None, chars("d"), false);
        assert_eq!(r.get(Some('-')).unwrap().text, "d");
        assert_eq!(r.get(None).unwrap().text, "d");
        assert_eq!(r.get(Some('0')).unwrap().text, "y");
    }

    #[test]
    fn numbered_registers_shift() {
        let mut r = Registers::default();
        for text in ["one", "two", "three"] {
            r.delete(None, Register::new(text, RegisterKind::Line), false);
        }
        assert_eq!(r.get(Some('1')).unwrap().text, "three");
        assert_eq!(r.get(Some('3')).unwrap().text, "one");
    }

    #[test]
    fn named_and_append() {
        let mut r = Registers::default();
        r.yank(Some('a'), chars("foo"));
        r.yank(Some('A'), chars("bar"));
        assert_eq!(r.get(Some('a')).unwrap().text, "foobar");
        r.yank(Some('A'), Register::new("line", RegisterKind::Line));
        assert_eq!(
            r.get(Some('a')).unwrap(),
            &Register::new("foobar\nline", RegisterKind::Line)
        );
        assert_eq!(r.get(None).unwrap().text, "foobar\nline");
        r.delete(Some('_'), chars("gone"), false);
        assert_eq!(r.get(None).unwrap().text, "foobar\nline");
    }
}
