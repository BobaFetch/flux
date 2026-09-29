//! Keys, and parsing/printing Vim key notation (`<C-w>`, `<CR>`, `<lt>`).

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyCode {
    Char(char),
    Esc,
    Enter,
    Backspace,
    Tab,
    Delete,
    Insert,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Super/Cmd, written `<D-…>` as in Neovim.
    pub meta: bool,
}

impl Modifiers {
    pub const NONE: Self = Self {
        ctrl: false,
        alt: false,
        shift: false,
        meta: false,
    };
    pub const CTRL: Self = Self {
        ctrl: true,
        ..Self::NONE
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub code: KeyCode,
    pub mods: Modifiers,
}

impl Key {
    /// A key, normalized the way Vim compares keys: Shift is folded into printable characters
    /// (`<S-a>` is `A`), and Ctrl with a letter ignores case (`<C-A>` is `<C-a>`).
    pub fn new(code: KeyCode, mut mods: Modifiers) -> Self {
        let code = match code {
            KeyCode::Char(c) => {
                if mods.shift {
                    mods.shift = false;
                    KeyCode::Char(c.to_uppercase().next().unwrap_or(c))
                } else if mods.ctrl && c.is_ascii_uppercase() {
                    KeyCode::Char(c.to_ascii_lowercase())
                } else {
                    KeyCode::Char(c)
                }
            }
            other => other,
        };
        Self { code, mods }
    }

    pub const fn char(c: char) -> Self {
        Self {
            code: KeyCode::Char(c),
            mods: Modifiers::NONE,
        }
    }

    pub fn ctrl(c: char) -> Self {
        Self::new(KeyCode::Char(c), Modifiers::CTRL)
    }

    pub const fn plain(code: KeyCode) -> Self {
        Self {
            code,
            mods: Modifiers::NONE,
        }
    }

    /// The character this key types, if it types one.
    pub fn typed_char(&self) -> Option<char> {
        match (self.code, self.mods) {
            (KeyCode::Char(c), Modifiers::NONE) => Some(c),
            _ => None,
        }
    }
}

const NAMES: &[(&str, KeyCode)] = &[
    ("Esc", KeyCode::Esc),
    ("CR", KeyCode::Enter),
    ("Enter", KeyCode::Enter),
    ("Return", KeyCode::Enter),
    ("BS", KeyCode::Backspace),
    ("Backspace", KeyCode::Backspace),
    ("Tab", KeyCode::Tab),
    ("Del", KeyCode::Delete),
    ("Delete", KeyCode::Delete),
    ("Insert", KeyCode::Insert),
    ("Up", KeyCode::Up),
    ("Down", KeyCode::Down),
    ("Left", KeyCode::Left),
    ("Right", KeyCode::Right),
    ("Home", KeyCode::Home),
    ("End", KeyCode::End),
    ("PageUp", KeyCode::PageUp),
    ("PageDown", KeyCode::PageDown),
    ("Space", KeyCode::Char(' ')),
    ("lt", KeyCode::Char('<')),
    ("Bslash", KeyCode::Char('\\')),
    ("Bar", KeyCode::Char('|')),
];

/// Parse Vim key notation. Anything that isn't a valid `<…>` name is taken literally, as Vim does.
pub fn parse_keys(input: &str) -> Vec<Key> {
    let mut keys = Vec::new();
    let mut rest = input;
    while let Some(c) = rest.chars().next() {
        if c == '<'
            && let Some(end) = rest.find('>')
            && let Some(key) = parse_special(&rest[1..end])
        {
            keys.push(key);
            rest = &rest[end + 1..];
            continue;
        }
        keys.push(Key::char(c));
        rest = &rest[c.len_utf8()..];
    }
    keys
}

fn parse_special(inner: &str) -> Option<Key> {
    let mut mods = Modifiers::NONE;
    let mut name = inner;
    // Modifier prefixes, e.g. `C-S-x`. A trailing `-` is the key itself, as in `<C-->`.
    while name.len() > 2 && name.as_bytes()[1] == b'-' {
        match name.as_bytes()[0].to_ascii_uppercase() {
            b'C' => mods.ctrl = true,
            b'S' => mods.shift = true,
            b'A' | b'M' => mods.alt = true,
            b'D' => mods.meta = true,
            _ => return None,
        }
        name = &name[2..];
    }
    let mut chars = name.chars();
    let code = match (chars.next(), chars.next()) {
        (Some(c), None) if mods != Modifiers::NONE => KeyCode::Char(c),
        _ => {
            if let Some(n) = name
                .strip_prefix(['F', 'f'])
                .and_then(|n| n.parse::<u8>().ok())
                .filter(|n| (1..=37).contains(n))
            {
                KeyCode::F(n)
            } else {
                NAMES
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case(name))
                    .map(|&(_, code)| code)?
            }
        }
    };
    Some(Key::new(code, mods))
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self.code {
            KeyCode::Char(c) if self.mods == Modifiers::NONE => {
                return match c {
                    '<' => f.write_str("<lt>"),
                    ' ' => f.write_str("<Space>"),
                    c => write!(f, "{c}"),
                };
            }
            KeyCode::Char(' ') => "Space".to_string(),
            KeyCode::Char('<') => "lt".to_string(),
            KeyCode::Char(c) => c.to_string(),
            KeyCode::F(n) => format!("F{n}"),
            code => NAMES
                .iter()
                .find(|&&(_, c)| c == code)
                .map(|(n, _)| n.to_string())
                .unwrap_or_default(),
        };
        f.write_str("<")?;
        for (on, prefix) in [
            (self.mods.ctrl, "C-"),
            (self.mods.shift, "S-"),
            (self.mods.alt, "M-"),
            (self.mods.meta, "D-"),
        ] {
            if on {
                f.write_str(prefix)?;
            }
        }
        write!(f, "{name}>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show(keys: &[Key]) -> String {
        keys.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn plain_and_special_keys() {
        let keys = parse_keys("dw<Esc>:q<CR>");
        assert_eq!(keys.len(), 6);
        assert_eq!(keys[2], Key::plain(KeyCode::Esc));
        assert_eq!(keys[5], Key::plain(KeyCode::Enter));
        assert_eq!(show(&keys), "dw<Esc>:q<CR>");
    }

    #[test]
    fn modifiers_normalize_like_vim() {
        assert_eq!(parse_keys("<C-A>"), parse_keys("<c-a>"));
        assert_eq!(parse_keys("<S-a>"), vec![Key::char('A')]);
        assert_eq!(parse_keys("<C-w>"), vec![Key::ctrl('w')]);
        assert_eq!(
            show(&parse_keys("<S-Tab><M-x><D-s><C-->")),
            "<S-Tab><M-x><D-s><C-->"
        );
    }

    #[test]
    fn names_are_case_insensitive() {
        assert_eq!(parse_keys("<esc><cr><bs>"), parse_keys("<Esc><CR><BS>"));
        assert_eq!(parse_keys("<F12>"), vec![Key::plain(KeyCode::F(12))]);
    }

    #[test]
    fn invalid_notation_is_literal() {
        assert_eq!(show(&parse_keys("<foo>")), "<lt>foo>");
        assert_eq!(show(&parse_keys("a<b")), "a<lt>b");
        assert_eq!(parse_keys("<lt>"), vec![Key::char('<')]);
        assert_eq!(parse_keys("<"), vec![Key::char('<')]);
    }

    #[test]
    fn unicode_chars() {
        assert_eq!(parse_keys("é日"), vec![Key::char('é'), Key::char('日')]);
    }
}
