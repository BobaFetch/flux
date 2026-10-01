//! The predicates and directives Neovim adds to tree-sitter queries (`#lua-match?`,
//! `#has-ancestor?`, `#set!`, `#offset!`, …). Tree-sitter itself evaluates `#eq?`, `#match?`
//! and `#any-of?`.

use std::borrow::Cow;

use flux_core::Text;
use regex::bytes::Regex;
use tree_sitter::{Node, Query, QueryMatch, QueryPredicateArg};

enum Test {
    LuaMatch(Regex),
    Contains(Vec<String>),
    HasAncestor(Vec<String>),
    HasParent(Vec<String>),
    KindEq(Vec<String>),
}

struct Predicate {
    capture: u32,
    test: Test,
    negate: bool,
}

/// A `#set!`: on the whole match, or on one capture.
pub(crate) struct Setting {
    pub capture: Option<u32>,
    pub key: String,
    pub value: Option<String>,
}

/// What a pattern adds to the plain tree match.
#[derive(Default)]
pub(crate) struct Pattern {
    predicates: Vec<Predicate>,
    pub settings: Vec<Setting>,
    /// `#offset!`: a capture's range moved by rows and columns (start row, start column, end
    /// row, end column).
    pub offsets: Vec<(u32, [i64; 4])>,
}

impl Pattern {
    /// The value of `#set! key value` for the match, or for `capture`.
    pub fn setting(&self, capture: Option<u32>, key: &str) -> Option<&Setting> {
        self.settings
            .iter()
            .rev()
            .find(|s| s.key == key && (s.capture.is_none() || s.capture == capture))
    }

    pub fn offset(&self, capture: u32) -> Option<[i64; 4]> {
        self.offsets
            .iter()
            .rev()
            .find(|(c, _)| *c == capture)
            .map(|&(_, o)| o)
    }

    /// Whether `m` passes the pattern's predicates.
    pub fn satisfied(&self, m: &QueryMatch<'_, '_>, text: &Text) -> bool {
        self.predicates.iter().all(|p| {
            let mut nodes = m.nodes_for_capture_index(p.capture).peekable();
            if nodes.peek().is_none() {
                // Neovim: a predicate on a capture that didn't match (an optional one) holds.
                return true;
            }
            nodes.all(|node| p.test.holds(node, text) != p.negate)
        })
    }
}

impl Test {
    fn holds(&self, node: Node<'_>, text: &Text) -> bool {
        match self {
            Test::LuaMatch(re) => re.is_match(node_text(node, text).as_bytes()),
            Test::Contains(needles) => {
                let s = node_text(node, text);
                needles.iter().any(|n| s.contains(n.as_str()))
            }
            Test::HasAncestor(kinds) => {
                let mut n = node.parent();
                while let Some(p) = n {
                    if kinds.iter().any(|k| k == p.kind()) {
                        return true;
                    }
                    n = p.parent();
                }
                false
            }
            Test::HasParent(kinds) => node
                .parent()
                .is_some_and(|p| kinds.iter().any(|k| k == p.kind())),
            Test::KindEq(kinds) => kinds.iter().any(|k| k == node.kind()),
        }
    }
}

/// The text of `node`.
pub(crate) fn node_text<'a>(node: Node<'_>, text: &'a Text) -> Cow<'a, str> {
    let rope = text.rope();
    let range = node.byte_range();
    let end = range.end.min(rope.len_bytes());
    rope.byte_slice(range.start.min(end)..end).into()
}

/// Every pattern of a query.
pub(crate) struct Patterns(Vec<Pattern>);

impl Patterns {
    pub fn new(query: &Query) -> Result<Self, String> {
        (0..query.pattern_count())
            .map(|i| parse_pattern(query, i))
            .collect::<Result<_, _>>()
            .map(Self)
    }

    pub fn get(&self, i: usize) -> &Pattern {
        &self.0[i]
    }
}

fn parse_pattern(query: &Query, index: usize) -> Result<Pattern, String> {
    let mut pattern = Pattern::default();
    for p in query.general_predicates(index) {
        let op = &*p.operator;
        let capture = match p.args.first() {
            Some(QueryPredicateArg::Capture(c)) => Some(*c),
            _ => None,
        };
        let strings: Vec<String> = p
            .args
            .iter()
            .filter_map(|a| match a {
                QueryPredicateArg::String(s) => Some(s.to_string()),
                QueryPredicateArg::Capture(_) => None,
            })
            .collect();
        let (name, negate) = match op.strip_prefix("not-") {
            Some(rest) => (rest, true),
            None => (op, false),
        };
        let test = match name {
            "lua-match?" => {
                let pat = strings.first().ok_or("#lua-match? needs a pattern")?;
                let re = lua_pattern_regex(pat).map_err(|e| format!("#lua-match? {pat:?}: {e}"))?;
                Some(Test::LuaMatch(re))
            }
            "contains?" => Some(Test::Contains(strings.clone())),
            "has-ancestor?" => Some(Test::HasAncestor(strings.clone())),
            "has-parent?" => Some(Test::HasParent(strings.clone())),
            "kind-eq?" => Some(Test::KindEq(strings.clone())),
            "nvim-set!" => {
                let (key, value) = match strings.as_slice() {
                    [key] => (key.clone(), None),
                    [key, value, ..] => (key.clone(), Some(value.clone())),
                    [] => return Err("#set! needs a key".into()),
                };
                pattern.settings.push(Setting {
                    capture,
                    key,
                    value,
                });
                None
            }
            "offset!" => {
                let nums: Vec<i64> = strings.iter().filter_map(|s| s.parse().ok()).collect();
                if let (Some(c), [a, b, c2, d]) = (capture, nums.as_slice()) {
                    pattern.offsets.push((c, [*a, *b, *c2, *d]));
                }
                None
            }
            // Directives that change text for other uses (injections, conceal); highlighting
            // doesn't need them.
            _ if op.ends_with('!') => None,
            _ => return Err(format!("unknown predicate #{op}")),
        };
        if let Some(test) = test {
            let capture = capture.ok_or_else(|| format!("#{op} needs a capture"))?;
            pattern.predicates.push(Predicate {
                capture,
                test,
                negate,
            });
        }
    }
    Ok(pattern)
}

/// Translate a Lua pattern (as `#lua-match?` takes) into a byte regex with the same meaning.
pub(crate) fn lua_pattern_regex(pat: &str) -> Result<Regex, String> {
    let mut out = String::from("(?s-u)");
    let chars: Vec<char> = pat.chars().collect();
    let mut i = 0;
    // Each Lua item is translated as a unit, so quantifiers apply to the whole of it.
    while i < chars.len() {
        let c = chars[i];
        match c {
            '^' if i == 0 => {
                out.push('^');
                i += 1;
                continue;
            }
            '$' if i == chars.len() - 1 => {
                out.push('$');
                i += 1;
                continue;
            }
            '(' | ')' => {
                out.push(c);
                i += 1;
                continue;
            }
            _ => {}
        }
        let item = match c {
            '.' => {
                i += 1;
                ".".to_string()
            }
            '%' => {
                let Some(&next) = chars.get(i + 1) else {
                    return Err("pattern ends with '%'".into());
                };
                i += 2;
                match next {
                    'b' | 'f' => return Err(format!("%{next} is not supported")),
                    n if n.is_ascii_digit() => {
                        return Err("back-references are not supported".into());
                    }
                    n => match class(n) {
                        Some(set) => format!("[{set}]"),
                        None => escape(n),
                    },
                }
            }
            '[' => {
                let (set, end) = bracket(&chars, i)?;
                i = end;
                set
            }
            _ => {
                i += 1;
                escape(c)
            }
        };
        out.push_str(&item);
        match chars.get(i) {
            Some('*' | '+' | '?') => {
                out.push(chars[i]);
                i += 1;
            }
            Some('-') => {
                out.push_str("*?");
                i += 1;
            }
            _ => {}
        }
    }
    Regex::new(&out).map_err(|e| e.to_string())
}

/// The contents of a regex class for Lua's `%x` (`None` if `x` isn't a class letter).
fn class(c: char) -> Option<String> {
    let set = match c.to_ascii_lowercase() {
        'a' => "A-Za-z",
        'd' => "0-9",
        'l' => "a-z",
        's' => r" \t\n\r\x0B\x0C",
        'u' => "A-Z",
        'w' => "A-Za-z0-9",
        'x' => "0-9A-Fa-f",
        'p' => r"!-/:-@\[-`{-~",
        'c' => r"\x00-\x1F\x7F",
        'g' => r"!-~",
        _ => return None,
    };
    Some(if c.is_ascii_uppercase() {
        format!("^{set}")
    } else {
        set.to_string()
    })
}

/// A Lua `[set]` starting at `chars[start]`, and the index after it.
fn bracket(chars: &[char], start: usize) -> Result<(String, usize), String> {
    let mut i = start + 1;
    let mut out = String::from("[");
    if chars.get(i) == Some(&'^') {
        out.push('^');
        i += 1;
    }
    let mut first = true;
    loop {
        let Some(&c) = chars.get(i) else {
            return Err("missing ']'".into());
        };
        if c == ']' && !first {
            out.push(']');
            return Ok((out, i + 1));
        }
        first = false;
        if c == '%' {
            let Some(&n) = chars.get(i + 1) else {
                return Err("pattern ends with '%'".into());
            };
            match class(n) {
                // A negated class inside a set has no regex equivalent as a range list; use
                // a nested class.
                Some(set) => out.push_str(&format!("[{set}]")),
                None => out.push_str(&escape(n)),
            }
            i += 2;
            continue;
        }
        if chars.get(i + 1) == Some(&'-') && chars.get(i + 2).is_some_and(|&e| e != ']') {
            out.push_str(&escape(c));
            out.push('-');
            out.push_str(&escape(chars[i + 2]));
            i += 3;
            continue;
        }
        out.push_str(&escape(c));
        i += 1;
    }
}

/// `c` as a regex literal, working byte by byte (the regex is not Unicode-aware, like Lua).
fn escape(c: char) -> String {
    if c.is_ascii_alphanumeric() || c == '_' {
        c.to_string()
    } else if c.is_ascii() {
        format!(r"\x{:02X}", c as u32)
    } else {
        let mut buf = [0; 4];
        c.encode_utf8(&mut buf)
            .bytes()
            .map(|b| format!(r"\x{b:02X}"))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pat: &str, s: &str) -> bool {
        lua_pattern_regex(pat).unwrap().is_match(s.as_bytes())
    }

    #[test]
    fn lua_patterns() {
        assert!(m("^[A-Z][A-Z0-9_]+$", "MAX_LEN"));
        assert!(!m("^[A-Z][A-Z0-9_]+$", "Max"));
        assert!(m("^%u", "Foo"));
        assert!(!m("^%u", "foo"));
        assert!(m("^---", "--- doc"));
        assert!(m("^%s*$", "  \t"));
        assert!(m("^[-+*]$", "-"));
        assert!(m("^#!/", "#!/bin/sh"));
        assert!(m("^_*[A-Z][A-Z%d_]*$", "__FOO_1"));
        assert!(m("a.-b", "axxb"));
        assert!(m("^%.", "."));
        assert!(!m("^%.", "x"));
        assert!(m("[%w_]+", "é_"));
        assert!(m("^[^%s]", "x"));
        assert!(!m("^[^%s]", " x"));
        assert!(m("^%[", "["));
        assert!(m("é", "café"));
        assert!(m("a$b", "a$b"), "$ is literal unless last");
        assert!(lua_pattern_regex("%b()").is_err());
    }
}
