//! Vim search patterns, translated to the `regex` crate.
//!
//! Vim's pattern syntax (`:h pattern`) is translated into an equivalent Rust regex that runs over
//! the buffer's lines joined by `\n`. Supported: the four magic levels (`\v` `\m` `\M` `\V`),
//! `\c`/`\C`, anchors with Vim's context rules for `^` `$` and `*`, `.` `\_.`, multis (`*` `\+`
//! `\=` `\?` `\{n,m}` `\{-n,m}`), groups `\( \)` and `\%( \)`, alternation `\|`, `\< \>`,
//! collections `[...]` (with `\_[...]`, character classes and escapes), the character classes
//! `\s \d \w \a \l \u \x \o \h \k \i \f \p` and their negations and `\_x` forms, `\n \t \e \r
//! \b`, `\zs`/`\ze`, `\%^ \%$`, `\%d123 \%x2a \%o17 \%u20ac`, and `~` (the last substitute
//! string). Vim features the `regex` crate can't express (lookaround `\@`, back-references
//! `\1`, `\&`, `\%V`, `\%23l` and friends) are reported as errors instead of silently
//! misbehaving.

use std::fmt;

use regex::{Regex, RegexBuilder};

/// How a pattern is compiled.
#[derive(Debug, Clone, Copy, Default)]
pub struct PatternOptions<'a> {
    /// 'ignorecase'.
    pub ignorecase: bool,
    /// 'smartcase': with 'ignorecase', a pattern with an uppercase letter matches case.
    pub smartcase: bool,
    /// The last substitute string, for `~`. `None` if there hasn't been one.
    pub last_substitute: Option<&'a str>,
}

/// Why a pattern can't be used, formatted like Vim's error (`E54: Unmatched \(`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternError(pub String);

impl fmt::Display for PatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PatternError {}

fn err<T>(msg: impl Into<String>) -> Result<T, PatternError> {
    Err(PatternError(msg.into()))
}

/// A compiled pattern.
#[derive(Debug, Clone)]
pub struct Pattern {
    regex: Regex,
    /// Capture groups standing for `\zs` and `\ze`.
    zs: Vec<usize>,
    ze: Vec<usize>,
    /// The regex group of each `\(` (Vim's `\1`, `\2`, …).
    groups: Vec<usize>,
    multiline: bool,
}

/// Byte ranges of a match and its `\(\)` groups (see [`Pattern::captures_at`]).
pub type Submatches = Vec<Option<(usize, usize)>>;

/// A match, as byte offsets into the searched text. With `\zs`/`\ze` it's the part between
/// them; `whole_start` is where the regex match itself began.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub start: usize,
    pub end: usize,
    pub whole_start: usize,
    pub whole_end: usize,
}

impl Pattern {
    pub fn new(pattern: &str, options: PatternOptions<'_>) -> Result<Self, PatternError> {
        let t = Translator::new(pattern, options.last_substitute).translate()?;
        let ignore_case = match t.case {
            Some(ignore) => ignore,
            None => options.ignorecase && !(options.smartcase && has_uppercase(pattern)),
        };
        let regex = RegexBuilder::new(&t.regex)
            .multi_line(true)
            .case_insensitive(ignore_case)
            .build()
            .or_else(|e| err(format!("E486: Invalid pattern: {e}")))?;
        Ok(Self {
            regex,
            zs: t.zs,
            ze: t.ze,
            groups: t.vim_groups,
            multiline: t.multiline,
        })
    }

    /// Whether a match can include a line break, so matching one line at a time isn't enough.
    pub fn is_multiline(&self) -> bool {
        self.multiline
    }

    /// The first match whose regex match starts at or after byte `at` of `hay`.
    pub fn find_at(&self, hay: &str, at: usize) -> Option<Match> {
        if self.zs.is_empty() && self.ze.is_empty() {
            let m = self.regex.find_at(hay, at)?;
            return Some(Match {
                start: m.start(),
                end: m.end(),
                whole_start: m.start(),
                whole_end: m.end(),
            });
        }
        let caps = self.regex.captures_at(hay, at)?;
        let whole = caps.get(0)?;
        // The last `\zs`/`\ze` that took part in the match wins, like in Vim.
        let pos = |groups: &[usize]| {
            groups
                .iter()
                .filter_map(|&g| caps.get(g))
                .map(|m| m.start())
                .max()
        };
        let start = pos(&self.zs).unwrap_or(whole.start());
        let end = pos(&self.ze).unwrap_or(whole.end()).max(start);
        Some(Match {
            start,
            end,
            whole_start: whole.start(),
            whole_end: whole.end(),
        })
    }

    /// The match at or after byte `at` with Vim's submatches: `[0]` is the whole match (the
    /// `\zs`..`\ze` part), `[1]`… are `\(`…`\)` groups (`None` if they didn't take part).
    pub fn captures_at(&self, hay: &str, at: usize) -> Option<(Match, Submatches)> {
        let caps = self.regex.captures_at(hay, at)?;
        let whole = caps.get(0)?;
        let pos = |groups: &[usize]| {
            groups
                .iter()
                .filter_map(|&g| caps.get(g))
                .map(|m| m.start())
                .max()
        };
        let start = pos(&self.zs).unwrap_or(whole.start());
        let end = pos(&self.ze).unwrap_or(whole.end()).max(start);
        let mut subs = vec![Some((start, end))];
        for &g in &self.groups {
            subs.push(caps.get(g).map(|m| (m.start(), m.end())));
        }
        Some((
            Match {
                start,
                end,
                whole_start: whole.start(),
                whole_end: whole.end(),
            },
            subs,
        ))
    }

    /// Every match in `hay`, Vim-style: after a match the next one is looked for from the
    /// character after its start when it was empty, otherwise from its end.
    pub fn find_iter<'h>(&'h self, hay: &'h str) -> impl Iterator<Item = Match> + 'h {
        let mut at = 0;
        std::iter::from_fn(move || {
            if at > hay.len() {
                return None;
            }
            let m = self.find_at(hay, at)?;
            at = if m.whole_end > m.whole_start {
                m.whole_end
            } else {
                next_char(hay, m.whole_end)
            };
            Some(m)
        })
    }
}

/// The byte offset of the character after the one at `at` (or one past the end).
pub fn next_char(s: &str, at: usize) -> usize {
    s[at.min(s.len())..]
        .chars()
        .next()
        .map_or(s.len() + 1, |c| at + c.len_utf8())
}

/// Vim's `pat_has_uppercase`: an uppercase letter that isn't part of an escape like `\S`.
pub fn has_uppercase(pattern: &str) -> bool {
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                // `\_x` and `\%x` are items, not letters.
                if let Some('_' | '%') = chars.next() {
                    chars.next();
                }
            }
            c if c.is_uppercase() => return true,
            _ => {}
        }
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
    VeryNoMagic,
    NoMagic,
    Magic,
    VeryMagic,
}

/// One item of the pattern, with the magic level already applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    /// A literal character.
    Lit(char),
    /// A metacharacter (`^ $ . * [ ~ ( ) | + ? = { @ % < >`), in its special meaning.
    Meta(char),
    /// A backslash item that is special at every magic level: `\s`, `\n`, `\z`, `\_`, …
    Esc(char),
    End,
}

struct Output {
    regex: String,
    case: Option<bool>,
    zs: Vec<usize>,
    ze: Vec<usize>,
    vim_groups: Vec<usize>,
    multiline: bool,
}

struct Translator<'a> {
    chars: Vec<char>,
    pos: usize,
    magic: Level,
    last_substitute: Option<&'a str>,
    out: String,
    case: Option<bool>,
    groups: usize,
    zs: Vec<usize>,
    ze: Vec<usize>,
    vim_groups: Vec<usize>,
    multiline: bool,
    /// Open groups, `true` for capturing ones.
    open: Vec<bool>,
    /// Nothing that can be repeated precedes this point (start of pattern or branch).
    at_start: bool,
    /// The last item was a multi (`*`, `\+`, …), and the one before this item was.
    after_multi: bool,
    prev_multi: bool,
}

impl<'a> Translator<'a> {
    fn new(pattern: &str, last_substitute: Option<&'a str>) -> Self {
        Self {
            chars: pattern.chars().collect(),
            pos: 0,
            magic: Level::Magic,
            last_substitute,
            out: String::new(),
            case: None,
            groups: 0,
            zs: Vec::new(),
            ze: Vec::new(),
            vim_groups: Vec::new(),
            multiline: false,
            open: Vec::new(),
            at_start: true,
            after_multi: false,
            prev_multi: false,
        }
    }

    /// Read the next item at the current magic level.
    fn next_tok(&mut self) -> Tok {
        let Some(&c) = self.chars.get(self.pos) else {
            return Tok::End;
        };
        self.pos += 1;
        if c != '\\' {
            return if self.special_bare(c) {
                Tok::Meta(c)
            } else {
                Tok::Lit(c)
            };
        }
        let Some(&d) = self.chars.get(self.pos) else {
            // A trailing backslash is a literal backslash.
            return Tok::Lit('\\');
        };
        self.pos += 1;
        if is_meta(d) {
            // A backslash flips a metacharacter's meaning.
            return if self.special_bare(d) {
                Tok::Lit(d)
            } else {
                Tok::Meta(d)
            };
        }
        if d.is_ascii_alphanumeric() || d == '_' {
            Tok::Esc(d)
        } else {
            Tok::Lit(d)
        }
    }

    /// Whether metacharacter `c` is special without a backslash at the current level.
    fn special_bare(&self, c: char) -> bool {
        match c {
            '^' | '$' => self.magic >= Level::NoMagic,
            '.' | '*' | '[' | '~' => self.magic >= Level::Magic,
            '(' | ')' | '|' | '+' | '?' | '=' | '{' | '@' | '%' | '<' | '>' => {
                self.magic == Level::VeryMagic
            }
            _ => false,
        }
    }

    fn peek_tok(&mut self) -> Tok {
        let (pos, magic) = (self.pos, self.magic);
        let t = self.next_tok();
        self.pos = pos;
        self.magic = magic;
        t
    }

    fn translate(mut self) -> Result<Output, PatternError> {
        loop {
            let tok = self.next_tok();
            if tok == Tok::End {
                break;
            }
            self.item(tok)?;
        }
        if !self.open.is_empty() {
            return if self.open.last() == Some(&true) {
                err("E54: Unmatched \\(")
            } else {
                err("E53: Unmatched \\%(")
            };
        }
        Ok(Output {
            regex: self.out,
            case: self.case,
            zs: self.zs,
            ze: self.ze,
            vim_groups: self.vim_groups,
            multiline: self.multiline,
        })
    }

    /// Emit an atom that can be repeated.
    fn atom(&mut self, regex: &str) {
        self.out.push_str(regex);
        self.at_start = false;
    }

    fn lit(&mut self, c: char) {
        let mut buf = [0; 4];
        self.atom(&regex::escape(c.encode_utf8(&mut buf)));
    }

    fn item(&mut self, tok: Tok) -> Result<(), PatternError> {
        // A multi sets this again; anything else ends a run of multis.
        self.prev_multi = std::mem::replace(&mut self.after_multi, false);
        match tok {
            Tok::End => {}
            Tok::Lit(c) => self.lit(c),
            Tok::Meta(c) => self.meta(c)?,
            Tok::Esc(c) => self.escape(c)?,
        }
        Ok(())
    }

    fn meta(&mut self, c: char) -> Result<(), PatternError> {
        match c {
            '^' => {
                if self.at_start {
                    self.out.push('^');
                } else {
                    self.lit('^');
                }
            }
            '$' => {
                let next = self.peek_tok();
                let at_end = matches!(next, Tok::End | Tok::Meta('|' | ')') | Tok::Esc('n'));
                if at_end {
                    self.out.push('$');
                    self.at_start = false;
                } else {
                    self.lit('$');
                }
            }
            '.' => self.atom("."),
            '[' => self.collection(false)?,
            '~' => match self.last_substitute {
                Some(s) => {
                    let s = regex::escape(s);
                    self.atom(&format!("(?:{s})"));
                }
                None => return err("E33: No previous substitute regular expression"),
            },
            '*' | '+' | '=' | '?' | '{' => {
                if self.at_start {
                    // Nothing to repeat: a literal, like Vim at the start of a pattern.
                    self.lit(c);
                } else {
                    let multi = match c {
                        '*' => "*".to_string(),
                        '+' => "+".to_string(),
                        '=' | '?' => "?".to_string(),
                        _ => self.brace()?,
                    };
                    if self.prev_multi {
                        return err("E61: Nested *");
                    }
                    self.out.push_str(&multi);
                    self.after_multi = true;
                    return Ok(());
                }
            }
            '@' => return err("E1: flux doesn't support \\@ (lookaround) in patterns yet"),
            '(' => {
                self.groups += 1;
                self.vim_groups.push(self.groups);
                self.open.push(true);
                self.out.push('(');
                self.at_start = true;
            }
            ')' => match self.open.pop() {
                Some(_) => {
                    self.out.push(')');
                    self.at_start = false;
                }
                None => return err("E55: Unmatched \\)"),
            },
            '|' => {
                self.out.push('|');
                self.at_start = true;
            }
            '%' => self.percent()?,
            '<' => self.out.push_str(r"\b{start}"),
            '>' => self.out.push_str(r"\b{end}"),
            _ => self.lit(c),
        }
        Ok(())
    }

    /// `\{n,m}` and friends, after the `{`: returns the Rust repetition.
    fn brace(&mut self) -> Result<String, PatternError> {
        let mut body = String::new();
        loop {
            match self.chars.get(self.pos) {
                Some('}') => {
                    self.pos += 1;
                    break;
                }
                Some('\\') if self.chars.get(self.pos + 1) == Some(&'}') => {
                    self.pos += 2;
                    break;
                }
                Some(&c) if c.is_ascii_digit() || c == ',' || c == '-' => {
                    body.push(c);
                    self.pos += 1;
                }
                _ => return err("E554: Syntax error in \\{...}"),
            }
        }
        let (lazy, body) = match body.strip_prefix('-') {
            Some(rest) => (true, rest.to_string()),
            None => (false, body),
        };
        let rep = match body.split_once(',') {
            None if body.is_empty() => "*".to_string(),
            None => format!("{{{body}}}"),
            Some((a, b)) => {
                let a = if a.is_empty() { "0" } else { a };
                format!("{{{a},{b}}}")
            }
        };
        let rep = match rep.as_str() {
            "{0,}" => "*".to_string(),
            _ => rep,
        };
        Ok(if lazy { format!("{rep}?") } else { rep })
    }

    /// Items after `\%`.
    fn percent(&mut self) -> Result<(), PatternError> {
        let Some(&c) = self.chars.get(self.pos) else {
            return err("E71: Invalid character after \\%");
        };
        self.pos += 1;
        match c {
            '(' => {
                self.open.push(false);
                self.out.push_str("(?:");
                self.at_start = true;
            }
            '^' => self.out.push_str(r"\A"),
            '$' => self.out.push_str(r"\z"),
            'd' | 'x' | 'o' | 'u' | 'U' => {
                let (radix, max) = match c {
                    'd' => (10, 10),
                    'x' => (16, 2),
                    'o' => (8, 4),
                    'u' => (16, 4),
                    _ => (16, 8),
                };
                let n = self.number(radix, max);
                match n.and_then(char::from_u32) {
                    Some(ch) => self.lit(ch),
                    None => return err("E678: Invalid character after \\%[dxouU]"),
                }
            }
            '[' => return err("E1: flux doesn't support \\%[ in patterns yet"),
            _ => return err(format!("E1: flux doesn't support \\%{c} in patterns yet")),
        }
        Ok(())
    }

    fn number(&mut self, radix: u32, max: usize) -> Option<u32> {
        let start = self.pos;
        while self.pos - start < max && self.chars.get(self.pos).is_some_and(|c| c.is_digit(radix))
        {
            self.pos += 1;
        }
        let digits: String = self.chars[start..self.pos].iter().collect();
        u32::from_str_radix(&digits, radix).ok()
    }

    fn escape(&mut self, c: char) -> Result<(), PatternError> {
        match c {
            'n' => {
                self.multiline = true;
                self.atom(r"\n");
            }
            't' => self.atom(r"\t"),
            'e' => self.atom(r"\x1b"),
            'r' => self.atom(r"\r"),
            'b' => self.atom(r"\x08"),
            'c' => self.case = Some(true),
            'C' => {
                if self.case.is_none() {
                    self.case = Some(false);
                }
            }
            'v' => self.magic = Level::VeryMagic,
            'm' => self.magic = Level::Magic,
            'M' => self.magic = Level::NoMagic,
            'V' => self.magic = Level::VeryNoMagic,
            'z' => match self.chars.get(self.pos) {
                Some('s') => {
                    self.pos += 1;
                    self.groups += 1;
                    self.zs.push(self.groups);
                    self.out.push_str("()");
                }
                Some('e') => {
                    self.pos += 1;
                    self.groups += 1;
                    self.ze.push(self.groups);
                    self.out.push_str("()");
                }
                _ => return err("E68: flux doesn't support this \\z item yet"),
            },
            '_' => match self.chars.get(self.pos).copied() {
                Some('^') => {
                    self.pos += 1;
                    self.out.push('^');
                }
                Some('$') => {
                    self.pos += 1;
                    self.out.push('$');
                }
                Some('.') => {
                    self.pos += 1;
                    self.multiline = true;
                    self.atom(r"(?s:.)");
                }
                Some('[') => {
                    self.pos += 1;
                    self.multiline = true;
                    self.collection(true)?;
                }
                Some(k) if class(k).is_some() => {
                    self.pos += 1;
                    self.multiline = true;
                    let (set, negated) = class(k).unwrap();
                    self.atom(&if negated {
                        format!("[^{set}]")
                    } else {
                        format!("[{set}\\n]")
                    });
                }
                _ => return err("E63: Invalid use of \\_"),
            },
            '1'..='9' => {
                return err("E1: flux doesn't support back-references (\\1) in patterns yet");
            }
            k => match class(k) {
                Some((set, false)) => self.atom(&format!("[{set}]")),
                Some((set, true)) => self.atom(&format!("[^{set}\\n]")),
                // An unknown escape is the character itself.
                None => self.lit(k),
            },
        }
        Ok(())
    }

    /// `[...]` after the `[`; `newline` for `\_[...]`. Without a closing `]` the `[` is
    /// literal.
    fn collection(&mut self, newline: bool) -> Result<(), PatternError> {
        let start = self.pos;
        let mut set = String::new();
        let mut negated = false;
        if self.chars.get(self.pos) == Some(&'^') {
            negated = true;
            self.pos += 1;
        }
        let mut first = true;
        loop {
            let Some(&c) = self.chars.get(self.pos) else {
                // No closing `]`: a literal `[`.
                self.pos = start;
                if newline {
                    return err("E769: Missing ] after \\_[");
                }
                self.lit('[');
                return Ok(());
            };
            self.pos += 1;
            match c {
                ']' if !first => break,
                '[' if self.chars.get(self.pos) == Some(&':') => {
                    let rest: String = self.chars[self.pos..].iter().collect();
                    if let Some(end) = rest.find(":]") {
                        let name = &rest[1..end];
                        match char_class(name) {
                            Some(cls) => set.push_str(cls),
                            None => set.push_str(r"\["),
                        }
                        if char_class(name).is_some() {
                            self.pos += end + 2;
                        }
                    } else {
                        set.push_str(r"\[");
                    }
                }
                '[' if matches!(self.chars.get(self.pos), Some('=' | '.')) => {
                    // `[[=a=]]` and `[[.a.]]`: the character itself.
                    let kind = self.chars[self.pos];
                    if self.chars.get(self.pos + 2) == Some(&kind)
                        && self.chars.get(self.pos + 3) == Some(&']')
                    {
                        push_class_char(&mut set, self.chars[self.pos + 1]);
                        self.pos += 4;
                    } else {
                        set.push_str(r"\[");
                    }
                }
                '-' if !first && self.chars.get(self.pos) != Some(&']') => set.push('-'),
                '\\' => {
                    let d = self.chars.get(self.pos).copied();
                    let ch = match d {
                        Some('e') => Some('\x1b'),
                        Some('t') => Some('\t'),
                        Some('r') => Some('\r'),
                        Some('b') => Some('\x08'),
                        Some('n') => Some('\n'),
                        Some('\\') => Some('\\'),
                        Some(']') => Some(']'),
                        Some('^') => Some('^'),
                        Some('-') => Some('-'),
                        _ => None,
                    };
                    match (ch, d) {
                        (Some(ch), _) => {
                            self.pos += 1;
                            if ch == '\n' {
                                self.multiline = true;
                                set.push_str(r"\n");
                            } else {
                                push_class_char(&mut set, ch);
                            }
                        }
                        (None, Some(k @ ('d' | 'o' | 'x' | 'u' | 'U'))) => {
                            self.pos += 1;
                            let (radix, max) = match k {
                                'd' => (10, 10),
                                'o' => (8, 4),
                                'x' => (16, 2),
                                'u' => (16, 4),
                                _ => (16, 8),
                            };
                            match self.number(radix, max).and_then(char::from_u32) {
                                Some(ch) => push_class_char(&mut set, ch),
                                None => {
                                    push_class_char(&mut set, '\\');
                                    push_class_char(&mut set, k);
                                }
                            }
                        }
                        // Any other backslash is literal.
                        _ => push_class_char(&mut set, '\\'),
                    }
                }
                c => push_class_char(&mut set, c),
            }
            first = false;
        }
        if newline && !negated {
            set.push_str(r"\n");
        }
        if negated && !newline {
            set.push_str(r"\n");
        }
        if set.is_empty() {
            self.atom(if negated { r"(?s:.)" } else { r"[^\s\S]" });
        } else {
            let neg = if negated { "^" } else { "" };
            self.atom(&format!("[{neg}{set}]"));
        }
        Ok(())
    }
}

fn is_meta(c: char) -> bool {
    matches!(
        c,
        '^' | '$'
            | '.'
            | '*'
            | '['
            | '~'
            | '('
            | ')'
            | '|'
            | '+'
            | '?'
            | '='
            | '{'
            | '@'
            | '%'
            | '<'
            | '>'
    )
}

/// Vim's character classes (`\s`, `\d`, …) as the inside of a Rust class, and whether it's
/// negated. The negated forms never match a line break.
fn class(c: char) -> Option<(&'static str, bool)> {
    Some(match c {
        's' => (r" \t", false),
        'S' => (r" \t", true),
        'd' => ("0-9", false),
        'D' => ("0-9", true),
        'w' => ("0-9A-Za-z_", false),
        'W' => ("0-9A-Za-z_", true),
        'a' => ("A-Za-z", false),
        'A' => ("A-Za-z", true),
        'l' => ("a-z", false),
        'L' => ("a-z", true),
        'u' => ("A-Z", false),
        'U' => ("A-Z", true),
        'x' => ("0-9A-Fa-f", false),
        'X' => ("0-9A-Fa-f", true),
        'o' => ("0-7", false),
        'O' => ("0-7", true),
        'h' => ("A-Za-z_", false),
        'H' => ("A-Za-z_", true),
        // 'iskeyword' and 'isident' ("@,48-57,_,192-255"); multibyte letters count as word
        // characters too, as in Vim.
        'k' | 'i' => (r"\w\u{c0}-\u{ff}", false),
        'K' | 'I' => (r"\p{L}_\u{c0}-\u{ff}", false),
        // 'isfname' ("@,48-57,/,.,-,_,+,,,#,$,%,~,=").
        'f' => (r"\w/.\-_+,#$%~=", false),
        'F' => (r"\p{L}/.\-_+,#$%~=", false),
        'p' => (r"^\x00-\x1f\x7f", false),
        'P' => (r"^\x00-\x1f\x7f0-9", false),
        _ => return None,
    })
}

/// `[:name:]` inside a collection.
fn char_class(name: &str) -> Option<&'static str> {
    Some(match name {
        "alnum" => "0-9A-Za-z",
        "alpha" => "A-Za-z",
        "blank" => r" \t",
        "cntrl" => r"\x00-\x1f\x7f",
        "digit" => "0-9",
        "graph" => r"!-~",
        "lower" => "a-z",
        "print" => r" -~",
        "punct" => r"!-/:-@\[-`{-~",
        "space" => r"\t\n\x0b\x0c\r ",
        "upper" => "A-Z",
        "xdigit" => "0-9A-Fa-f",
        "return" => r"\r",
        "tab" => r"\t",
        "escape" => r"\x1b",
        "backspace" => r"\x08",
        "ident" | "keyword" => r"\w",
        "fname" => r"\w/.\-_+,#$%~=",
        _ => return None,
    })
}

fn push_class_char(set: &mut String, c: char) {
    if matches!(c, '[' | ']' | '\\' | '^' | '-' | '&' | '~') {
        set.push('\\');
        set.push(c);
    } else if c == '\n' {
        set.push_str(r"\n");
    } else {
        set.push(c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(pat: &str, hay: &str) -> Vec<String> {
        let p = Pattern::new(pat, PatternOptions::default()).unwrap();
        p.find_iter(hay)
            .map(|m| hay[m.start..m.end].to_string())
            .collect()
    }

    fn one(pat: &str, hay: &str) -> Option<String> {
        matches(pat, hay).into_iter().next()
    }

    #[test]
    fn magic_basics() {
        assert_eq!(matches("a.c", "abc a-c ac"), ["abc", "a-c"]);
        assert_eq!(matches("ab*", "a ab abbb"), ["a", "ab", "abbb"]);
        assert_eq!(matches("ab\\+", "a ab abbb"), ["ab", "abbb"]);
        assert_eq!(matches("colou\\=r", "color colour"), ["color", "colour"]);
        assert_eq!(matches("a\\{2}", "aaaaa"), ["aa", "aa"]);
        assert_eq!(matches("a\\{-1,}", "aaa"), ["a", "a", "a"]);
        assert_eq!(matches("\\(ab\\)\\+", "ababab"), ["ababab"]);
        assert_eq!(matches("foo\\|bar", "foo bar"), ["foo", "bar"]);
        assert_eq!(one("a+b", "a+b aab"), Some("a+b".into()));
        assert_eq!(one("a(b)", "a(b)"), Some("a(b)".into()));
    }

    #[test]
    fn magic_levels() {
        assert_eq!(matches("\\v(ab)+", "ababx"), ["abab"]);
        assert_eq!(matches("\\v<is>", "this is"), ["is"]);
        assert_eq!(matches("\\Va.c", "abc a.c"), ["a.c"]);
        assert_eq!(matches("\\Ma*", "aa a*"), ["a*"]);
        assert_eq!(matches("\\M\\.", "a.b"), ["a", ".", "b"]);
        assert_eq!(matches("\\va{2}", "aaa"), ["aa"]);
    }

    #[test]
    fn anchors_depend_on_position() {
        assert_eq!(matches("^a", "a a\na"), ["a", "a"]);
        assert_eq!(matches("a^", "a^"), ["a^"]);
        assert_eq!(matches("a$", "a a\na$"), ["a"]);
        assert_eq!(matches("$a", "$a"), ["$a"]);
        assert_eq!(matches("*a", "*a"), ["*a"]);
        assert_eq!(matches("^*", "*a"), ["*"]);
        assert_eq!(matches("\\(^x\\)", "x\nx"), ["x", "x"]);
    }

    #[test]
    fn words_and_classes() {
        assert_eq!(matches("\\<the\\>", "the other then the"), ["the", "the"]);
        assert_eq!(matches("\\d\\+", "a12 b3"), ["12", "3"]);
        assert_eq!(matches("\\s\\+", "a \t b"), [" \t "]);
        assert_eq!(matches("\\S\\+", "ab cd\nef"), ["ab", "cd", "ef"]);
        assert_eq!(matches("\\u\\l", "AbCd"), ["Ab", "Cd"]);
        assert_eq!(matches("[abc]\\+", "xaabcx"), ["aabc"]);
        assert_eq!(matches("[^a]\\+", "bb\nbb"), ["bb", "bb"]);
        assert_eq!(matches("[]a]", "]a"), ["]", "a"]);
        assert_eq!(matches("[[:digit:]]", "a1"), ["1"]);
        assert_eq!(matches("[a-c-]", "b-"), ["b", "-"]);
        assert_eq!(matches("x[", "x["), ["x["]);
    }

    #[test]
    fn line_breaks() {
        assert_eq!(matches("a\\nb", "a\nb"), ["a\nb"]);
        assert_eq!(matches("a\\_s*b", "a \n b"), ["a \n b"]);
        assert_eq!(matches("a.b", "a\nb"), Vec::<String>::new());
        assert_eq!(matches("a\\_.b", "a\nb"), ["a\nb"]);
        assert!(
            Pattern::new("a\\nb", PatternOptions::default())
                .unwrap()
                .is_multiline()
        );
        assert!(
            !Pattern::new("ab", PatternOptions::default())
                .unwrap()
                .is_multiline()
        );
    }

    #[test]
    fn zs_and_ze() {
        assert_eq!(matches("foo\\zsbar", "foobar bar"), ["bar"]);
        assert_eq!(matches("foo\\zebar", "foobar foo"), ["foo"]);
        assert_eq!(matches("a\\zsb\\zec", "abc"), ["b"]);
    }

    #[test]
    fn case() {
        let ic = PatternOptions {
            ignorecase: true,
            ..Default::default()
        };
        let scs = PatternOptions {
            ignorecase: true,
            smartcase: true,
            ..Default::default()
        };
        let find = |p: &str, o| {
            Pattern::new(p, o)
                .unwrap()
                .find_at("Foo foo", 0)
                .unwrap()
                .start
        };
        assert_eq!(find("foo", ic), 0);
        assert_eq!(find("foo", scs), 0);
        assert_eq!(find("Foo", scs), 0);
        assert_eq!(find("foo\\C", ic), 4);
        assert_eq!(find("\\cFOO", PatternOptions::default()), 0);
        assert!(!has_uppercase("\\Sfoo"));
        assert!(has_uppercase("fOo"));
    }

    #[test]
    fn specials_and_errors() {
        assert_eq!(matches("\\%x41", "BAB"), ["A"]);
        assert_eq!(matches("a\\/b", "a/b"), ["a/b"]);
        assert_eq!(matches("a\\.b", "a.b axb"), ["a.b"]);
        let last = PatternOptions {
            last_substitute: Some("x.y"),
            ..Default::default()
        };
        let p = Pattern::new("a~", last).unwrap();
        assert_eq!(p.find_at("ax.y axzy", 0).map(|m| m.end), Some(4));
        let e = |p| Pattern::new(p, PatternOptions::default()).unwrap_err().0;
        assert_eq!(e("\\(a"), "E54: Unmatched \\(");
        assert_eq!(e("a\\)"), "E55: Unmatched \\)");
        assert_eq!(e("\\%(a"), "E53: Unmatched \\%(");
        assert_eq!(e("~"), "E33: No previous substitute regular expression");
        assert!(e("a\\@=b").starts_with("E1:"));
        assert!(e("\\(a\\)\\1").starts_with("E1:"));
    }

    #[test]
    fn empty_matches_advance() {
        assert_eq!(matches("x*", "ab").len(), 3);
        assert_eq!(matches("^", "a\nb\nc").len(), 3);
    }
}
