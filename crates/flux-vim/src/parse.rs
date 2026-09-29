//! Normal-mode command parsing: keys typed so far in, a command (or "need more keys") out.
//!
//! The grammar is Vim's: `["x][count]{command}`, where an operator command takes
//! `[count]{motion}` or repeats itself (`dd`) to work on lines. Parsing is a pure function of
//! the keys, which is what makes `.` (replay the recorded keys) and count handling simple.

use crate::key::{Key, KeyCode, Modifiers};
use crate::motion::{Find, Motion};
use crate::textobj::TextObject;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    Delete,
    Change,
    Yank,
    ShiftRight,
    ShiftLeft,
    Lowercase,
    Uppercase,
    ToggleCase,
}

impl Operator {
    /// Operators that change text, and so are repeated by `.`.
    pub fn changes_text(self) -> bool {
        self != Operator::Yank
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpTarget {
    Motion(Motion),
    /// The operator repeated (`dd`, `>>`, `gUU`): `count` whole lines.
    Lines,
    Object(TextObject),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertAt {
    /// `i`
    Cursor,
    /// `a`
    After,
    /// `I`
    FirstNonBlank,
    /// `gI`
    LineStart,
    /// `A`
    LineEnd,
    /// `o`
    OpenBelow,
    /// `O`
    OpenAbove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scroll {
    LinesDown,
    LinesUp,
    HalfDown,
    HalfUp,
    PageDown,
    PageUp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Move(Motion),
    Operate(Operator, OpTarget),
    Put {
        before: bool,
    },
    /// `r`: the replacement character (`\n` for `r<CR>`).
    Replace(char),
    Join {
        spaces: bool,
    },
    /// `~`
    ToggleCase,
    Insert(InsertAt),
    Undo,
    Redo,
    /// `.`
    Repeat,
    CmdLine,
    Scroll(Scroll),
    /// `CTRL-G`
    FileInfo,
    /// `ZZ`
    WriteQuit,
    /// `ZQ`
    QuitDiscard,
    /// `CTRL-L`: handled by the frontend; nothing to do here.
    Redraw,
    /// `m{a-zA-Z…}`
    SetMark(char),
    /// `q{reg}`: start recording a macro.
    Record(char),
    /// `q` while recording.
    StopRecord,
    /// `@{reg}`; `@@` is the last one run, `@:` the last command line.
    Execute(char),
    /// `CTRL-O` (`true`) and `CTRL-I`/`<Tab>`.
    Jump {
        older: bool,
    },
    /// `v`, `V`.
    Visual(flux_view::VisualKind),
    /// `gv`
    Reselect,
    /// `gi`
    InsertAtLastInsert,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub register: Option<char>,
    /// The count, with an operator's two counts multiplied (`2d3w` is 6). `None` if none typed.
    pub count: Option<usize>,
    pub action: Action,
    /// The keys without their counts, for `.` to replay with a new count.
    pub keys: Vec<Key>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parse {
    /// A valid prefix; wait for more keys.
    Incomplete,
    /// Not a command. Vim beeps and forgets the keys.
    Invalid,
    Done(Command),
}

struct Keys<'a> {
    keys: &'a [Key],
    i: usize,
    recorded: Vec<Key>,
}

/// Marks running out of keys: the command isn't complete yet.
struct NeedMore;

impl Keys<'_> {
    fn next(&mut self) -> Result<Key, NeedMore> {
        let key = *self.keys.get(self.i).ok_or(NeedMore)?;
        self.i += 1;
        self.recorded.push(key);
        Ok(key)
    }

    fn peek(&self) -> Option<Key> {
        self.keys.get(self.i).copied()
    }

    /// Digits of a count. They're left out of the recorded keys.
    fn count(&mut self) -> Option<usize> {
        let mut count: Option<usize> = None;
        while let Some(c) = self.peek().and_then(|k| k.typed_char()) {
            let Some(d) = c.to_digit(10) else { break };
            if d == 0 && count.is_none() {
                break; // `0` is a motion unless it continues a count
            }
            count = Some(
                count
                    .unwrap_or(0)
                    .saturating_mul(10)
                    .saturating_add(d as usize),
            );
            self.i += 1;
        }
        count
    }
}

/// Parse a Normal-mode command. `recording` says whether a macro is being recorded, which makes
/// `q` alone stop it.
pub fn parse(keys: &[Key], recording: bool) -> Parse {
    let mut k = Keys {
        keys,
        i: 0,
        recorded: Vec::new(),
    };
    match parse_command(&mut k, recording) {
        Err(NeedMore) => Parse::Incomplete,
        Ok(None) => Parse::Invalid,
        Ok(Some((register, count, action))) => Parse::Done(Command {
            register,
            count,
            action,
            keys: k.recorded,
        }),
    }
}

type Parsed = Option<(Option<char>, Option<usize>, Action)>;

fn parse_command(k: &mut Keys, recording: bool) -> Result<Parsed, NeedMore> {
    let mut register = None;
    let mut count: Option<usize> = None;
    // `"x` and a count, in either order (`"a3yy`, `3"ayy`).
    loop {
        if let Some(c) = k.count() {
            count = Some(count.unwrap_or(1).saturating_mul(c));
            continue;
        }
        if k.peek() == Some(Key::char('"')) {
            k.next()?;
            match k.next()?.typed_char() {
                Some(c) if flux_view::registers::is_valid_name(c) => register = Some(c),
                _ => return Ok(None),
            }
            continue;
        }
        break;
    }

    let key = k.next()?;
    let action = match key.typed_char() {
        Some(c) => match c {
            'd' => operator(k, Operator::Delete, 'd', &mut count)?,
            'c' => operator(k, Operator::Change, 'c', &mut count)?,
            'y' => operator(k, Operator::Yank, 'y', &mut count)?,
            '>' => operator(k, Operator::ShiftRight, '>', &mut count)?,
            '<' => operator(k, Operator::ShiftLeft, '<', &mut count)?,
            'x' => Some(Action::Operate(
                Operator::Delete,
                OpTarget::Motion(Motion::Right),
            )),
            'X' => Some(Action::Operate(
                Operator::Delete,
                OpTarget::Motion(Motion::Left),
            )),
            'D' => Some(Action::Operate(
                Operator::Delete,
                OpTarget::Motion(Motion::LineEnd),
            )),
            'C' => Some(Action::Operate(
                Operator::Change,
                OpTarget::Motion(Motion::LineEnd),
            )),
            's' => Some(Action::Operate(
                Operator::Change,
                OpTarget::Motion(Motion::Right),
            )),
            'S' => Some(Action::Operate(Operator::Change, OpTarget::Lines)),
            // Neovim's default `Y` is `y$`.
            'Y' => Some(Action::Operate(
                Operator::Yank,
                OpTarget::Motion(Motion::LineEnd),
            )),
            'p' => Some(Action::Put { before: false }),
            'P' => Some(Action::Put { before: true }),
            'r' => match k.next()? {
                key if key == Key::plain(KeyCode::Enter) => Some(Action::Replace('\n')),
                key if key == Key::plain(KeyCode::Tab) => Some(Action::Replace('\t')),
                key => key.typed_char().map(Action::Replace),
            },
            'J' => Some(Action::Join { spaces: true }),
            '~' => Some(Action::ToggleCase),
            'i' => Some(Action::Insert(InsertAt::Cursor)),
            'a' => Some(Action::Insert(InsertAt::After)),
            'I' => Some(Action::Insert(InsertAt::FirstNonBlank)),
            'A' => Some(Action::Insert(InsertAt::LineEnd)),
            'o' => Some(Action::Insert(InsertAt::OpenBelow)),
            'O' => Some(Action::Insert(InsertAt::OpenAbove)),
            'u' => Some(Action::Undo),
            '.' => Some(Action::Repeat),
            ':' => Some(Action::CmdLine),
            'v' => Some(Action::Visual(flux_view::VisualKind::Char)),
            'V' => Some(Action::Visual(flux_view::VisualKind::Line)),
            'm' => match k.next()?.typed_char() {
                Some(c) if c.is_ascii_alphabetic() || "'`[]<>".contains(c) => {
                    Some(Action::SetMark(c))
                }
                _ => None,
            },
            'q' if recording => Some(Action::StopRecord),
            'q' => match k.next()?.typed_char() {
                Some(c) if c.is_ascii_alphanumeric() || c == '"' => Some(Action::Record(c)),
                _ => None,
            },
            '@' => match k.next()?.typed_char() {
                Some(c) if c.is_ascii_alphanumeric() || "@:\"-".contains(c) => {
                    Some(Action::Execute(c))
                }
                _ => None,
            },
            'Z' => match k.next()?.typed_char() {
                Some('Z') => Some(Action::WriteQuit),
                Some('Q') => Some(Action::QuitDiscard),
                _ => None,
            },
            'g' => match k.next()?.typed_char() {
                Some('~') => operator(k, Operator::ToggleCase, '~', &mut count)?,
                Some('u') => operator(k, Operator::Lowercase, 'u', &mut count)?,
                Some('U') => operator(k, Operator::Uppercase, 'U', &mut count)?,
                Some('J') => Some(Action::Join { spaces: false }),
                Some('I') => Some(Action::Insert(InsertAt::LineStart)),
                Some('v') => Some(Action::Reselect),
                Some('i') => Some(Action::InsertAtLastInsert),
                Some(c) => g_motion(c).map(Action::Move),
                None => None,
            },
            _ => motion(k, key)?.map(Action::Move),
        },
        None => control_key(key).or(motion(k, key)?.map(Action::Move)),
    };
    Ok(action.map(|a| (register, count, a)))
}

fn control_key(key: Key) -> Option<Action> {
    if key.mods == Modifiers::CTRL {
        return match key.code {
            KeyCode::Char('r') => Some(Action::Redo),
            KeyCode::Char('e') => Some(Action::Scroll(Scroll::LinesDown)),
            KeyCode::Char('y') => Some(Action::Scroll(Scroll::LinesUp)),
            KeyCode::Char('d') => Some(Action::Scroll(Scroll::HalfDown)),
            KeyCode::Char('u') => Some(Action::Scroll(Scroll::HalfUp)),
            KeyCode::Char('f') => Some(Action::Scroll(Scroll::PageDown)),
            KeyCode::Char('b') => Some(Action::Scroll(Scroll::PageUp)),
            KeyCode::Char('g') => Some(Action::FileInfo),
            KeyCode::Char('l') => Some(Action::Redraw),
            KeyCode::Char('o') => Some(Action::Jump { older: true }),
            KeyCode::Char('i') => Some(Action::Jump { older: false }),
            _ => None,
        };
    }
    match key.code {
        // Terminals send CTRL-I as <Tab>.
        KeyCode::Tab if key.mods == Modifiers::NONE => Some(Action::Jump { older: false }),
        KeyCode::PageDown if key.mods == Modifiers::NONE => Some(Action::Scroll(Scroll::PageDown)),
        KeyCode::PageUp if key.mods == Modifiers::NONE => Some(Action::Scroll(Scroll::PageUp)),
        _ => None,
    }
}

/// After an operator key: `[count]{motion}`, or the operator again for whole lines.
fn operator(
    k: &mut Keys,
    op: Operator,
    repeat: char,
    count: &mut Option<usize>,
) -> Result<Option<Action>, NeedMore> {
    if let Some(c) = k.count() {
        *count = Some(count.unwrap_or(1).saturating_mul(c));
    }
    let key = k.next()?;
    let is_g_op = matches!(
        op,
        Operator::ToggleCase | Operator::Lowercase | Operator::Uppercase
    );
    if key.typed_char() == Some(repeat) {
        return Ok(Some(Action::Operate(op, OpTarget::Lines)));
    }
    if key.typed_char() == Some('g') {
        let next = k.next()?.typed_char();
        if is_g_op && next == Some(repeat) {
            return Ok(Some(Action::Operate(op, OpTarget::Lines)));
        }
        return Ok(next
            .and_then(g_motion)
            .map(|m| Action::Operate(op, OpTarget::Motion(m))));
    }
    if let Some(c @ ('i' | 'a')) = key.typed_char() {
        let obj = k
            .next()?
            .typed_char()
            .and_then(|o| TextObject::from_char(o, c == 'a'));
        return Ok(obj.map(|o| Action::Operate(op, OpTarget::Object(o))));
    }
    Ok(motion(k, key)?.map(|m| Action::Operate(op, OpTarget::Motion(m))))
}

fn g_motion(c: char) -> Option<Motion> {
    match c {
        'g' => Some(Motion::GotoFirstLine),
        'e' => Some(Motion::WordEndBackward(false)),
        'E' => Some(Motion::WordEndBackward(true)),
        _ => None,
    }
}

fn motion(k: &mut Keys, key: Key) -> Result<Option<Motion>, NeedMore> {
    let ctrl = |c| key == Key::ctrl(c);
    let plain = |code| key == Key::plain(code);
    if let Some(c) = key.typed_char() {
        return Ok(match c {
            'h' => Some(Motion::Left),
            'l' => Some(Motion::Right),
            'j' => Some(Motion::Down),
            'k' => Some(Motion::Up),
            ' ' => Some(Motion::SpaceRight),
            '0' => Some(Motion::LineStart),
            '^' => Some(Motion::FirstNonBlank),
            '$' => Some(Motion::LineEnd),
            '|' => Some(Motion::Column),
            '+' => Some(Motion::NextLineStart),
            '-' => Some(Motion::PrevLineStart),
            '_' => Some(Motion::CurrentLineStart),
            'G' => Some(Motion::GotoLine),
            '%' => Some(Motion::Percent),
            'w' => Some(Motion::WordForward(false)),
            'W' => Some(Motion::WordForward(true)),
            'b' => Some(Motion::WordBackward(false)),
            'B' => Some(Motion::WordBackward(true)),
            'e' => Some(Motion::WordEnd(false)),
            'E' => Some(Motion::WordEnd(true)),
            ';' => Some(Motion::RepeatFind { reverse: false }),
            ',' => Some(Motion::RepeatFind { reverse: true }),
            '}' => Some(Motion::ParagraphForward),
            '{' => Some(Motion::ParagraphBackward),
            'H' => Some(Motion::WindowTop),
            'M' => Some(Motion::WindowMiddle),
            'L' => Some(Motion::WindowBottom),
            '\'' | '`' => {
                let name = k.next()?.typed_char();
                name.filter(|&n| n.is_ascii_alphabetic() || "'`[]<>.^\"".contains(n))
                    .map(|name| Motion::Mark {
                        name,
                        exact: c == '`',
                    })
            }
            'f' | 'F' | 't' | 'T' => {
                let target = k.next()?;
                let ch = match target.code {
                    KeyCode::Tab if target.mods == Modifiers::NONE => Some('\t'),
                    _ => target.typed_char(),
                };
                ch.map(|ch| {
                    Motion::Find(Find {
                        forward: c == 'f' || c == 't',
                        till: c == 't' || c == 'T',
                        ch,
                    })
                })
            }
            _ => None,
        });
    }
    Ok(if plain(KeyCode::Left) || ctrl('h') {
        Some(Motion::Left)
    } else if plain(KeyCode::Right) {
        Some(Motion::Right)
    } else if plain(KeyCode::Down) || ctrl('j') || ctrl('n') {
        Some(Motion::Down)
    } else if plain(KeyCode::Up) || ctrl('p') {
        Some(Motion::Up)
    } else if plain(KeyCode::Backspace) {
        Some(Motion::BackspaceLeft)
    } else if plain(KeyCode::Home) {
        Some(Motion::LineStart)
    } else if plain(KeyCode::End) {
        Some(Motion::LineEnd)
    } else if plain(KeyCode::Enter) || ctrl('m') {
        Some(Motion::NextLineStart)
    } else {
        None
    })
}

/// A command typed in Visual mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualAction {
    Move(Motion),
    Object(TextObject),
    /// An operator on the selection (`d`, `c`, `y`, `>`, `~`, `u`, …).
    Operate(Operator),
    /// An operator on the selected lines, whatever the selection kind (`D`, `X`, `C`, `S`, `R`,
    /// `Y`).
    OperateLines(Operator),
    Join {
        spaces: bool,
    },
    Replace(char),
    Put {
        before: bool,
    },
    /// `o`, `O`
    SwapEnds,
    /// `gv`
    Reselect,
    /// `v`, `V`: switch kind, or leave when it's the current one.
    Switch(flux_view::VisualKind),
    /// `<Esc>`, `CTRL-C`
    Exit,
    CmdLine,
    Scroll(Scroll),
    SetMark(char),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualCommand {
    pub register: Option<char>,
    pub count: Option<usize>,
    pub action: VisualAction,
    /// The keys without the count, for `.` on a Visual operator.
    pub keys: Vec<Key>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisualParse {
    Incomplete,
    Invalid,
    Done(VisualCommand),
}

pub fn parse_visual(keys: &[Key]) -> VisualParse {
    let mut k = Keys {
        keys,
        i: 0,
        recorded: Vec::new(),
    };
    match parse_visual_command(&mut k) {
        Err(NeedMore) => VisualParse::Incomplete,
        Ok(None) => VisualParse::Invalid,
        Ok(Some((register, count, action))) => VisualParse::Done(VisualCommand {
            register,
            count,
            action,
            keys: k.recorded,
        }),
    }
}

type ParsedVisual = Option<(Option<char>, Option<usize>, VisualAction)>;

fn parse_visual_command(k: &mut Keys) -> Result<ParsedVisual, NeedMore> {
    use VisualAction as V;
    let mut register = None;
    let mut count = None;
    loop {
        if let Some(c) = k.count() {
            count = Some(c);
            continue;
        }
        if k.peek() == Some(Key::char('"')) {
            k.next()?;
            match k.next()?.typed_char() {
                Some(c) if flux_view::registers::is_valid_name(c) => register = Some(c),
                _ => return Ok(None),
            }
            continue;
        }
        break;
    }
    let key = k.next()?;
    if key == Key::plain(KeyCode::Esc) || key == Key::ctrl('c') {
        return Ok(Some((register, count, V::Exit)));
    }
    let action = match key.typed_char() {
        Some(c) => match c {
            'd' | 'x' => Some(V::Operate(Operator::Delete)),
            'c' | 's' => Some(V::Operate(Operator::Change)),
            'y' => Some(V::Operate(Operator::Yank)),
            '>' => Some(V::Operate(Operator::ShiftRight)),
            '<' => Some(V::Operate(Operator::ShiftLeft)),
            '~' => Some(V::Operate(Operator::ToggleCase)),
            'u' => Some(V::Operate(Operator::Lowercase)),
            'U' => Some(V::Operate(Operator::Uppercase)),
            'D' | 'X' => Some(V::OperateLines(Operator::Delete)),
            'C' | 'S' | 'R' => Some(V::OperateLines(Operator::Change)),
            'Y' => Some(V::OperateLines(Operator::Yank)),
            'J' => Some(V::Join { spaces: true }),
            'p' => Some(V::Put { before: false }),
            'P' => Some(V::Put { before: true }),
            'o' | 'O' => Some(V::SwapEnds),
            'v' => Some(V::Switch(flux_view::VisualKind::Char)),
            'V' => Some(V::Switch(flux_view::VisualKind::Line)),
            ':' => Some(V::CmdLine),
            'r' => match k.next()? {
                key if key == Key::plain(KeyCode::Enter) => Some(V::Replace('\n')),
                key => key.typed_char().map(V::Replace),
            },
            'm' => k
                .next()?
                .typed_char()
                .filter(|c| c.is_ascii_alphabetic())
                .map(V::SetMark),
            'i' | 'a' => k
                .next()?
                .typed_char()
                .and_then(|o| TextObject::from_char(o, c == 'a'))
                .map(V::Object),
            'g' => match k.next()?.typed_char() {
                Some('v') => Some(V::Reselect),
                Some('J') => Some(V::Join { spaces: false }),
                Some('~') => Some(V::Operate(Operator::ToggleCase)),
                Some('u') => Some(V::Operate(Operator::Lowercase)),
                Some('U') => Some(V::Operate(Operator::Uppercase)),
                Some(c) => g_motion(c).map(V::Move),
                None => None,
            },
            _ => motion(k, key)?.map(V::Move),
        },
        None => match control_key(key) {
            Some(Action::Scroll(s)) => Some(V::Scroll(s)),
            _ if key == Key::plain(KeyCode::Delete) => Some(V::Operate(Operator::Delete)),
            _ => motion(k, key)?.map(V::Move),
        },
    };
    Ok(action.map(|a| (register, count, a)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_keys;

    fn done(keys: &str) -> Command {
        match parse(&parse_keys(keys), false) {
            Parse::Done(c) => c,
            other => panic!("{keys}: {other:?}"),
        }
    }

    #[test]
    fn counts_multiply_and_are_left_out_of_the_keys() {
        let c = done("2d3w");
        assert_eq!(c.count, Some(6));
        assert_eq!(
            c.action,
            Action::Operate(
                Operator::Delete,
                OpTarget::Motion(Motion::WordForward(false))
            )
        );
        assert_eq!(c.keys, parse_keys("dw"));
        assert_eq!(done("d10w").count, Some(10));
        assert_eq!(done("10x").count, Some(10));
    }

    #[test]
    fn zero_is_a_motion_unless_it_continues_a_count() {
        assert_eq!(done("0").action, Action::Move(Motion::LineStart));
        assert_eq!(
            done("d0").action,
            Action::Operate(Operator::Delete, OpTarget::Motion(Motion::LineStart))
        );
        assert_eq!(done("10j").count, Some(10));
    }

    #[test]
    fn registers() {
        let c = done("\"a3yy");
        assert_eq!((c.register, c.count), (Some('a'), Some(3)));
        let c = done("3\"Ayy");
        assert_eq!((c.register, c.count), (Some('A'), Some(3)));
        assert_eq!(c.keys, parse_keys("\"Ayy"));
    }

    #[test]
    fn incomplete_and_invalid() {
        for keys in ["", "d", "2d3", "\"", "g", "f", "dt", "gu", "gug", "Z", "r"] {
            assert_eq!(parse(&parse_keys(keys), false), Parse::Incomplete, "{keys}");
        }
        for keys in ["dz", "Q", "d<Esc>", "f<Esc>", "\"=", "gq", "<Esc>", "dis"] {
            assert_eq!(parse(&parse_keys(keys), false), Parse::Invalid, "{keys}");
        }
    }

    #[test]
    fn operator_forms() {
        assert_eq!(
            done("dd").action,
            Action::Operate(Operator::Delete, OpTarget::Lines)
        );
        assert_eq!(
            done("gUU").action,
            Action::Operate(Operator::Uppercase, OpTarget::Lines)
        );
        assert_eq!(
            done("gugu").action,
            Action::Operate(Operator::Lowercase, OpTarget::Lines)
        );
        assert_eq!(
            done("dge").action,
            Action::Operate(
                Operator::Delete,
                OpTarget::Motion(Motion::WordEndBackward(false))
            )
        );
        assert_eq!(
            done("dgg").action,
            Action::Operate(Operator::Delete, OpTarget::Motion(Motion::GotoFirstLine))
        );
        assert_eq!(
            done("df,").action,
            Action::Operate(
                Operator::Delete,
                OpTarget::Motion(Motion::Find(Find {
                    forward: true,
                    till: false,
                    ch: ','
                }))
            )
        );
        assert_eq!(done("r<CR>").action, Action::Replace('\n'));
    }
}
