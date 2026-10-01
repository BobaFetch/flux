//! LSP snippets (`foo(${1:x}, $2)$0`), parsed as Neovim's `vim.lsp._snippet_grammar` does,
//! and expanded as `vim.snippet.expand` inserts them.

/// A snippet's parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Node {
    Text(String),
    Tabstop(usize),
    Placeholder(usize, Box<Node>),
    Choice(usize, Vec<String>),
    Variable(String, Option<Box<Node>>),
    /// Several parts (the whole snippet).
    Snippet(Vec<Node>),
}

impl Node {
    /// The text the snippet stands for: placeholders' text and choices' first value, without
    /// tabstops or variables (Neovim's `tostring` of a parsed snippet).
    pub fn text(&self) -> String {
        match self {
            Node::Text(t) => t.clone(),
            Node::Placeholder(_, v) => v.text(),
            Node::Choice(_, values) => values.first().cloned().unwrap_or_default(),
            Node::Snippet(children) => children.iter().map(Node::text).collect(),
            Node::Tabstop(_) | Node::Variable(..) => String::new(),
        }
    }
}

const ESCAPABLE: &str = "$}\\";

struct Parser<'a> {
    s: &'a [char],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn int(&mut self) -> Option<usize> {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        (self.i > start)
            .then(|| {
                self.s[start..self.i]
                    .iter()
                    .collect::<String>()
                    .parse()
                    .ok()
            })
            .flatten()
    }

    fn var_name(&mut self) -> Option<String> {
        let start = self.i;
        if !self
            .peek()
            .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        {
            return None;
        }
        while self
            .peek()
            .is_some_and(|c| c == '_' || c.is_ascii_alphanumeric())
        {
            self.i += 1;
        }
        Some(self.s[start..self.i].iter().collect())
    }

    /// Text up to one of `stop`, with `\` escaping what's in `escape`.
    fn text(&mut self, escape: &str, stop: &str) -> String {
        let mut out = String::new();
        while let Some(c) = self.peek() {
            if c == '\\'
                && let Some(&n) = self.s.get(self.i + 1)
                && escape.contains(n)
            {
                out.push(n);
                self.i += 2;
                continue;
            }
            if stop.contains(c) {
                break;
            }
            // A backslash before anything else stays.
            out.push(c);
            self.i += 1;
        }
        out
    }

    /// `$…` (Neovim's `any`): a placeholder, tabstop, choice or variable.
    fn any(&mut self) -> Option<Node> {
        let save = self.i;
        let node = self.any_inner();
        if node.is_none() {
            self.i = save;
        }
        node
    }

    fn any_inner(&mut self) -> Option<Node> {
        if !self.eat('$') {
            return None;
        }
        if let Some(n) = self.int() {
            return Some(Node::Tabstop(n));
        }
        if let Some(name) = self.var_name() {
            return Some(Node::Variable(name, None));
        }
        if !self.eat('{') {
            return None;
        }
        if let Some(n) = self.int() {
            if self.eat('}') {
                return Some(Node::Tabstop(n));
            }
            if self.eat(':') {
                let value = self.any_or_text();
                return self.eat('}').then(|| Node::Placeholder(n, Box::new(value)));
            }
            if self.eat('|') {
                let mut values = Vec::new();
                loop {
                    let v = self.text(&format!("{ESCAPABLE},|"), &format!("{ESCAPABLE},|"));
                    if v.is_empty() {
                        return None;
                    }
                    values.push(v);
                    if !self.eat(',') {
                        break;
                    }
                }
                return (self.eat('|') && self.eat('}')).then_some(Node::Choice(n, values));
            }
            return None;
        }
        let name = self.var_name()?;
        if self.eat('}') {
            return Some(Node::Variable(name, None));
        }
        if self.eat(':') {
            let default = self.any_or_text();
            return self
                .eat('}')
                .then(|| Node::Variable(name, Some(Box::new(default))));
        }
        if self.eat('/') {
            // A transform: `${VAR/regex/format/options}`, kept as the variable.
            let _ = self.text("/", "/");
            if !self.eat('/') {
                return None;
            }
            let mut depth = 0;
            while let Some(c) = self.peek() {
                match c {
                    '\\' => self.i += 1,
                    '{' => depth += 1,
                    '}' if depth > 0 => depth -= 1,
                    '/' if depth == 0 => break,
                    _ => {}
                }
                self.i += 1;
            }
            if !self.eat('/') {
                return None;
            }
            while self.peek().is_some_and(|c| "dgimsuvy".contains(c)) {
                self.i += 1;
            }
            return self.eat('}').then_some(Node::Variable(name, None));
        }
        None
    }

    /// A placeholder's value: one `$…`, or text up to an escapable character.
    fn any_or_text(&mut self) -> Node {
        if let Some(n) = self.any() {
            return n;
        }
        Node::Text(self.text(ESCAPABLE, ESCAPABLE))
    }
}

/// Parse `input`; `None` when it isn't a valid snippet.
pub(crate) fn parse(input: &str) -> Option<Node> {
    let chars: Vec<char> = input.chars().collect();
    let mut p = Parser { s: &chars, i: 0 };
    let mut children = Vec::new();
    while p.i < chars.len() {
        if let Some(n) = p.any() {
            children.push(n);
            continue;
        }
        let t = p.text(ESCAPABLE, "$");
        if t.is_empty() {
            return None;
        }
        children.push(Node::Text(t));
    }
    if children.is_empty() {
        return None;
    }
    Some(Node::Snippet(children))
}

/// The text of snippet `input`, or `input` itself when it doesn't parse (Neovim's
/// `parse_snippet`).
pub(crate) fn parse_text(input: &str) -> String {
    parse(input).map_or_else(|| input.to_string(), |n| n.text())
}

/// A snippet expanded for inserting: its text, and where its tabstops are in it (char
/// offsets of their start and end), by tabstop number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Expanded {
    pub text: String,
    pub tabstops: Vec<(usize, usize, usize)>,
}

/// Expand `input` as `vim.snippet.expand` does: placeholders' text, later lines indented as
/// the cursor line (`base_indent`), tabs as 'shiftwidth' spaces with `expand_tab`. Variables
/// are looked up with `var`.
pub(crate) fn expand(
    input: &str,
    base_indent: &str,
    expand_tab: Option<usize>,
    var: &dyn Fn(&str) -> Option<String>,
) -> Option<Expanded> {
    let Node::Snippet(children) = parse(input)? else {
        return None;
    };
    let mut placeholders: Vec<(usize, String)> = Vec::new();
    for child in &children {
        if let Node::Placeholder(n, v) = child
            && !placeholders.iter().any(|(m, _)| m == n)
        {
            placeholders.push((*n, v.text()));
        }
    }
    let placeholder = |n: usize| {
        placeholders
            .iter()
            .find(|(m, _)| *m == n)
            .map(|(_, v)| v.clone())
    };
    let mut text = String::new();
    let mut tabstops = Vec::new();
    let append = |text: &mut String, s: &str| {
        for (i, line) in s.split('\n').enumerate() {
            let line = match expand_tab {
                Some(sw) => line.replace('\t', &" ".repeat(sw)),
                None => line.to_string(),
            };
            if i > 0 {
                text.push('\n');
                text.push_str(base_indent);
            }
            text.push_str(&line);
        }
    };
    let len = |t: &String| t.chars().count();
    for child in &children {
        match child {
            Node::Tabstop(n) => {
                let start = len(&text);
                if let Some(p) = placeholder(*n) {
                    append(&mut text, &p);
                }
                tabstops.push((*n, start, len(&text)));
            }
            Node::Placeholder(n, _) => {
                let start = len(&text);
                append(&mut text, &placeholder(*n).unwrap_or_default());
                tabstops.push((*n, start, len(&text)));
            }
            Node::Choice(n, _) => {
                let start = len(&text);
                tabstops.push((*n, start, start));
            }
            Node::Variable(name, default) => {
                let value = var(name).or_else(|| default.as_ref().map(|d| d.text()));
                match value {
                    Some(v) => append(&mut text, &v),
                    None => {
                        // An unknown variable becomes a tabstop with its name.
                        let n = tabstops.iter().map(|t| t.0).max().unwrap_or(0) + 1;
                        let start = len(&text);
                        append(&mut text, name);
                        tabstops.push((n, start, len(&text)));
                    }
                }
            }
            Node::Text(t) => append(&mut text, t),
            Node::Snippet(_) => {}
        }
    }
    if !tabstops.iter().any(|t| t.0 == 0) {
        let end = len(&text);
        tabstops.push((0, end, end));
    }
    Some(Expanded { text, tabstops })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_text() {
        assert_eq!(parse_text("foo(${1:x}, $2)$0"), "foo(x, )");
        assert_eq!(parse_text("a ${1|one,two|} b"), "a one b");
        assert_eq!(parse_text("\\$1 ${1:\\}}"), "$1 }");
        assert_eq!(parse_text("x $TM_FILENAME y"), "x  y");
        assert_eq!(parse_text("cost $"), "cost $");
        assert_eq!(parse_text("${1:${2:inner}}"), "inner");
    }

    #[test]
    fn expanding() {
        let e = expand("foo(${1:x}, $2)$0", "", None, &|_| None).unwrap();
        assert_eq!(e.text, "foo(x, )");
        assert_eq!(e.tabstops, vec![(1, 4, 5), (2, 7, 7), (0, 8, 8)]);
        let e = expand("if $1 {\n\t$0\n}", "    ", Some(4), &|_| None).unwrap();
        assert_eq!(e.text, "if  {\n        \n    }");
    }
}
