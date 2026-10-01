//! A buffer's syntax tree, kept in step with its text, and the highlights of a range of it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::{ControlFlow, Range};
use std::time::{Duration, Instant};

use flux_core::{Revision, Text};
use streaming_iterator::StreamingIterator;
use tree_sitter::{
    InputEdit, Node, ParseOptions, ParseState, Parser, Point, QueryCursor, Range as TsRange, Tree,
};

use crate::lang::{self, Lang};
use crate::predicate::node_text;

/// How long parsing an injected region may take; one that takes longer isn't highlighted.
const INJECTION_BUDGET: Duration = Duration::from_millis(100);
/// How deep injections nest (Markdown → a Rust code block → a macro's arguments).
const MAX_INJECTION_DEPTH: usize = 3;
/// Neovim's default highlight priority.
const DEFAULT_PRIORITY: u32 = 100;

/// A highlighted range of a line: chars `start..end` (columns) of line `line`, with the
/// capture name to look up (`@keyword.function` without the `@`; empty for a span that only
/// carries a link), and the URL the text links to (Neovim's `url` metadata, shown as an OSC 8
/// hyperlink).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub capture: &'static str,
    pub url: Option<std::sync::Arc<str>>,
    /// Neovim's `conceal` metadata: with 'conceallevel', the text shows as this instead
    /// (nothing when it's empty).
    pub conceal: Option<std::sync::Arc<str>>,
    /// Neovim's `conceal_lines`: the whole line is hidden.
    pub conceal_lines: bool,
}

/// A capture in bytes, before sorting into paint order.
struct ByteSpan {
    range: Range<usize>,
    capture: &'static str,
    priority: u32,
    url: Option<std::sync::Arc<str>>,
    conceal: Option<std::sync::Arc<str>>,
    conceal_lines: bool,
}

/// A parse tree for one buffer.
pub struct Syntax {
    lang: &'static Lang,
    parser: Parser,
    tree: Option<Tree>,
    /// The revision of the text the tree follows.
    seen: Revision,
    /// Edits were applied to the tree's positions but it hasn't been parsed since.
    stale: bool,
    /// A parse ran out of time; the parser keeps its state to resume it.
    resuming: bool,
    /// Injected trees for the regions looked at so far, until the next edit.
    injections: RefCell<InjectionCache>,
}

/// An injected region: its language, and its byte ranges.
type Region = (&'static str, Vec<(usize, usize)>);

#[derive(Default)]
struct InjectionCache {
    parsers: HashMap<&'static str, Parser>,
    trees: HashMap<Region, Option<Tree>>,
}

impl std::fmt::Debug for Syntax {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Syntax")
            .field("lang", &self.lang.name)
            .field("parsed", &self.tree.is_some())
            .field("stale", &self.stale)
            .finish()
    }
}

/// Feed `text` to the parser in rope chunks. Like Neovim, the last line ends with a line
/// break too (a Markdown code fence on the last line closes its block).
fn parse(parser: &mut Parser, text: &Text, old: Option<&Tree>, deadline: Instant) -> Option<Tree> {
    let rope = text.rope();
    let len = rope.len_bytes();
    let mut input = |byte: usize, _: Point| -> &[u8] {
        if byte == len {
            return b"\n";
        }
        if byte > len {
            return &[];
        }
        let (chunk, start, _, _) = rope.chunk_at_byte(byte);
        &chunk.as_bytes()[byte - start..]
    };
    let mut progress = |_: &ParseState| {
        if Instant::now() > deadline {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let options = ParseOptions::new().progress_callback(&mut progress);
    parser.parse_with_options(&mut input, old, Some(options))
}

fn point(p: flux_core::BytePoint) -> Point {
    Point {
        row: p.row,
        column: p.col,
    }
}

impl Syntax {
    /// A parser for language `lang` (a parser name, see [`crate::lang_for_filetype`]).
    pub fn new(lang: &str) -> Option<Self> {
        let lang = lang::get(lang)?;
        let mut parser = Parser::new();
        parser.set_language(&lang.language).ok()?;
        Some(Self {
            lang,
            parser,
            tree: None,
            seen: Revision::default(),
            stale: true,
            resuming: false,
            injections: RefCell::default(),
        })
    }

    /// The tree is up to date with the text last given to [`Syntax::update`].
    pub fn is_parsed(&self) -> bool {
        !self.stale && self.tree.is_some()
    }

    /// The parser name.
    pub fn lang(&self) -> &'static str {
        self.lang.name
    }

    /// Follow `text`'s edits since the last call: the tree's positions move with them, without
    /// reparsing (enough to keep highlights and string/comment lookups in place after small
    /// edits, such as reindenting).
    fn follow(&mut self, text: &Text) {
        if text.revision() == self.seen {
            return;
        }
        match (self.tree.as_mut(), text.edits_since(self.seen)) {
            (Some(tree), Some(edits)) => {
                for e in edits.map(|e| e.bytes) {
                    tree.edit(&InputEdit {
                        start_byte: e.start_byte,
                        old_end_byte: e.old_end_byte,
                        new_end_byte: e.new_end_byte,
                        start_position: point(e.start),
                        old_end_position: point(e.old_end),
                        new_end_position: point(e.new_end),
                    });
                }
            }
            (None, Some(_)) => {}
            // Another text, or edits that weren't kept: start over.
            (_, None) => self.tree = None,
        }
        self.seen = text.revision();
        self.stale = true;
        if self.resuming {
            // The parse that ran out of time was of older text.
            self.parser.reset();
            self.resuming = false;
        }
        self.injections.get_mut().trees.clear();
    }

    /// Bring the tree up to date with `text`, reusing the old tree for the edits made since,
    /// parsing for at most `budget` (with `None`, only [`Syntax::follow`] the edits). Returns
    /// whether the tree is up to date; if not, the next call goes on from where this one
    /// stopped, and the tree as it was (moved by the edits) is used meanwhile.
    pub fn update(&mut self, text: &Text, budget: Option<Duration>) -> bool {
        self.follow(text);
        if !self.stale {
            return true;
        }
        let Some(budget) = budget else {
            return false;
        };
        match parse(
            &mut self.parser,
            text,
            self.tree.as_ref(),
            Instant::now() + budget,
        ) {
            Some(tree) => {
                self.tree = Some(tree);
                self.stale = false;
                self.resuming = false;
                self.injections.get_mut().trees.clear();
                true
            }
            None => {
                self.resuming = true;
                false
            }
        }
    }

    /// The syntax nodes containing byte `col` of line `line`, innermost first: their kinds
    /// and the (line, byte column) where they start. Empty without a tree.
    pub fn nodes_at(
        &self,
        text: &Text,
        line: usize,
        col: usize,
    ) -> Vec<(&'static str, (usize, usize))> {
        let Some(tree) = &self.tree else {
            return Vec::new();
        };
        if text.revision() != self.seen {
            return Vec::new();
        }
        let p = Point {
            row: line,
            column: col,
        };
        let mut node = tree.root_node().descendant_for_point_range(p, p);
        let mut out = Vec::new();
        while let Some(n) = node {
            let s = n.start_position();
            let kind = self
                .lang
                .language
                .node_kind_for_id(n.kind_id())
                .unwrap_or("");
            out.push((kind, (s.row, s.column)));
            node = n.parent();
        }
        out
    }

    /// The highlights on lines `lines` of `text` (which the tree must be up to date with), in
    /// the order to paint them: a later span's attributes override an earlier one's.
    pub fn highlights(&self, text: &Text, lines: Range<usize>) -> Vec<Span> {
        let Some(tree) = &self.tree else {
            return Vec::new();
        };
        if text.revision() != self.seen || lines.start >= text.line_count() {
            return Vec::new();
        }
        let rope = text.rope();
        let start = rope.line_to_byte(lines.start);
        let end = rope.line_to_byte(lines.end.min(text.line_count()));
        let mut spans = Vec::new();
        let mut cache = self.injections.borrow_mut();
        layer(self.lang, tree, text, start..end, 0, &mut cache, &mut spans);
        // Stable: equal priorities keep query order, so later captures win.
        spans.sort_by_key(|s| s.priority);
        let mut out = Vec::with_capacity(spans.len());
        for s in spans {
            let (from, to) = (s.range.start.max(start), s.range.end.min(end));
            if from >= to {
                continue;
            }
            split_lines(text, from..to, &s, &mut out);
        }
        out
    }
}

/// Split a byte range into per-line char spans.
fn split_lines(text: &Text, range: Range<usize>, s: &ByteSpan, out: &mut Vec<Span>) {
    let rope = text.rope();
    let first = rope.byte_to_line(range.start);
    let last = rope.byte_to_line(range.end);
    for line in first..=last {
        let line_start = rope.line_to_char(line);
        let line_len = text.line_len(line);
        let from = if line == first {
            rope.byte_to_char(range.start) - line_start
        } else {
            0
        };
        let to = if line == last {
            rope.byte_to_char(range.end) - line_start
        } else {
            // Through the line break: a highlighted line break shows past the text in Neovim
            // only for some groups; flux paints text only.
            line_len
        };
        let to = to.min(line_len);
        if from < to || s.conceal_lines {
            out.push(Span {
                line,
                start: from,
                end: to.max(from),
                capture: s.capture,
                url: s.url.clone(),
                conceal: s.conceal.clone(),
                conceal_lines: s.conceal_lines,
            });
        }
    }
}

/// Byte range of `node`, moved by an `#offset!` directive.
fn offset_range(node: Node<'_>, offset: Option<[i64; 4]>, text: &Text) -> Range<usize> {
    let Some([sr, sc, er, ec]) = offset else {
        return node.byte_range();
    };
    let rope = text.rope();
    let at = |p: Point, dr: i64, dc: i64| -> usize {
        let row = (p.row as i64 + dr).clamp(0, rope.len_lines() as i64 - 1) as usize;
        let line_start = rope.line_to_byte(row);
        let line_end = if row + 1 < rope.len_lines() {
            rope.line_to_byte(row + 1)
        } else {
            rope.len_bytes()
        };
        let col = (p.column as i64 + dc).max(0) as usize;
        (line_start + col).min(line_end)
    };
    let start = at(node.start_position(), sr, sc);
    let end = at(node.end_position(), er, ec);
    start..end.max(start)
}

/// Collect the highlights of one tree (and, recursively, the trees injected into it) over
/// bytes `range`.
fn layer(
    lang: &'static Lang,
    tree: &Tree,
    text: &Text,
    range: Range<usize>,
    depth: usize,
    cache: &mut InjectionCache,
    out: &mut Vec<ByteSpan>,
) {
    let rope = text.rope();
    let provider = |n: Node<'_>| {
        let r = n.byte_range();
        let end = r.end.min(rope.len_bytes());
        rope.byte_slice(r.start.min(end)..end)
            .chunks()
            .map(str::as_bytes)
    };
    let q = &lang.highlights;
    let names = q.query.capture_names();
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(range.clone());
    let mut captures = cursor.captures(&q.query, tree.root_node(), provider);
    while let Some((m, i)) = captures.next() {
        let pattern = q.patterns.get(m.pattern_index);
        if !pattern.satisfied(m, text) {
            continue;
        }
        let cap = m.captures()[*i];
        let name: &'static str = names[cap.index as usize];
        // A link: its URL is a literal, or the text of another capture.
        let url = pattern
            .settings
            .iter()
            .rev()
            .find(|s| s.key == "url" && s.capture == Some(cap.index))
            .and_then(|s| match s.value_capture {
                Some(c) => m.nodes_for_capture_index(c).next().map(|n| {
                    let r = offset_range(n, pattern.offset(c), text);
                    let rope = text.rope();
                    let end = r.end.min(rope.len_bytes());
                    String::from(rope.byte_slice(r.start.min(end)..end))
                }),
                None => s.value.clone(),
            })
            .map(std::sync::Arc::from);
        let conceal = pattern
            .setting(Some(cap.index), "conceal")
            .map(|s| std::sync::Arc::from(s.value.as_deref().unwrap_or("")));
        let conceal_lines = pattern.setting(Some(cap.index), "conceal_lines").is_some();
        let hidden = name.starts_with('_') || matches!(name, "spell" | "nospell" | "conceal");
        if hidden && url.is_none() && conceal.is_none() && !conceal_lines {
            continue;
        }
        let priority = pattern
            .setting(Some(cap.index), "priority")
            .and_then(|s| s.value.as_deref()?.parse().ok())
            .unwrap_or(DEFAULT_PRIORITY);
        out.push(ByteSpan {
            range: offset_range(cap.node, pattern.offset(cap.index), text),
            capture: if hidden { "" } else { name },
            priority,
            url,
            conceal,
            conceal_lines,
        });
    }
    if depth >= MAX_INJECTION_DEPTH {
        return;
    }
    for (inj_lang, ranges) in injections(lang, tree, text, range.clone()) {
        let key = (
            inj_lang.name,
            ranges.iter().map(|r| (r.start_byte, r.end_byte)).collect(),
        );
        if !cache.trees.contains_key(&key) {
            let parser = cache.parsers.entry(inj_lang.name).or_insert_with(|| {
                let mut p = Parser::new();
                p.set_language(&inj_lang.language).ok();
                p
            });
            let tree = parser
                .set_included_ranges(&ranges)
                .ok()
                .and_then(|()| parse(parser, text, None, Instant::now() + INJECTION_BUDGET));
            cache.trees.insert(key.clone(), tree);
        }
        if let Some(tree) = cache.trees.get(&key).cloned().flatten() {
            layer(inj_lang, &tree, text, range.clone(), depth + 1, cache, out);
        }
    }
}

/// The regions of `tree` within `range` that hold another language, from the language's
/// injections query.
fn injections(
    lang: &'static Lang,
    tree: &Tree,
    text: &Text,
    range: Range<usize>,
) -> Vec<(&'static Lang, Vec<TsRange>)> {
    let Some(q) = &lang.injections else {
        return Vec::new();
    };
    let rope = text.rope();
    let provider = |n: Node<'_>| {
        let r = n.byte_range();
        let end = r.end.min(rope.len_bytes());
        rope.byte_slice(r.start.min(end)..end)
            .chunks()
            .map(str::as_bytes)
    };
    let names = q.query.capture_names();
    let content_idx = names.iter().position(|&n| n == "injection.content");
    let lang_idx = names.iter().position(|&n| n == "injection.language");
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(range);
    let mut matches = cursor.matches(&q.query, tree.root_node(), provider);
    let mut out = Vec::new();
    while let Some(m) = matches.next() {
        let pattern = q.patterns.get(m.pattern_index);
        if !pattern.satisfied(m, text) {
            continue;
        }
        let name = match pattern.setting(None, "injection.language") {
            Some(s) => s.value.clone(),
            None => lang_idx.and_then(|i| {
                m.nodes_for_capture_index(i as u32)
                    .next()
                    .map(|n| node_text(n, text).into_owned())
            }),
        };
        let name = if pattern.setting(None, "injection.self").is_some() {
            Some(lang.name.to_string())
        } else {
            name
        };
        let Some(inj) = name
            .as_deref()
            .and_then(crate::filetype::lang_for_injection)
            .and_then(lang::get)
        else {
            continue;
        };
        let Some(ci) = content_idx else { continue };
        let include_children = pattern
            .setting(None, "injection.include-children")
            .is_some();
        for node in m.nodes_for_capture_index(ci as u32) {
            let r = offset_range(node, pattern.offset(ci as u32), text);
            let ranges = if include_children {
                vec![r]
            } else {
                exclude_children(node, r)
            };
            let ranges: Vec<TsRange> = ranges
                .into_iter()
                .filter(|r| r.start < r.end)
                .map(|r| ts_range(text, r))
                .collect();
            if !ranges.is_empty() {
                out.push((inj, ranges));
            }
        }
    }
    out
}

/// `range` (of `node`) without the ranges of the node's named children, as Neovim's
/// `get_node_ranges` does.
fn exclude_children(node: Node<'_>, range: Range<usize>) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut at = range.start;
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let r = child.byte_range();
        if r.start > at {
            out.push(at..r.start.min(range.end));
        }
        at = at.max(r.end);
    }
    if at < range.end {
        out.push(at..range.end);
    }
    out
}

fn ts_range(text: &Text, r: Range<usize>) -> TsRange {
    let rope = text.rope();
    let pt = |b: usize| {
        let row = rope.byte_to_line(b);
        Point {
            row,
            column: b - rope.line_to_byte(row),
        }
    };
    TsRange {
        start_byte: r.start,
        end_byte: r.end,
        start_point: pt(r.start),
        end_point: pt(r.end),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flux_core::Edit;

    const FULL: Option<Duration> = Some(Duration::from_secs(10));

    fn spans(syntax: &Syntax, text: &Text) -> Vec<(usize, String, &'static str)> {
        syntax
            .highlights(text, 0..text.line_count())
            .into_iter()
            .map(|s| {
                let line = text.line_str(s.line);
                let word: String = line.chars().skip(s.start).take(s.end - s.start).collect();
                (s.line, word, s.capture)
            })
            .collect()
    }

    fn has(spans: &[(usize, String, &str)], word: &str, capture: &str) -> bool {
        spans.iter().any(|(_, w, c)| w == word && *c == capture)
    }

    #[test]
    fn rust_highlights() {
        let mut text = Text::new("fn main() {\n    let x = \"hi\"; // note\n}\n");
        let mut syntax = Syntax::new("rust").unwrap();
        syntax.update(&mut text, FULL);
        let s = spans(&syntax, &text);
        assert!(has(&s, "fn", "keyword.function"), "{s:?}");
        assert!(has(&s, "main", "function"), "{s:?}");
        assert!(has(&s, "\"hi\"", "string"), "{s:?}");
        assert!(has(&s, "// note", "comment"), "{s:?}");
    }

    #[test]
    fn incremental_update_follows_edits() {
        let mut text = Text::new("fn main() {}\n");
        let mut syntax = Syntax::new("rust").unwrap();
        syntax.update(&mut text, FULL);
        // `fn` becomes `let x = 1; fn`, on its own line.
        text.apply(&Edit::insert(0, "let x = 1;\n"));
        syntax.update(&mut text, FULL);
        let s = spans(&syntax, &text);
        assert!(has(&s, "let", "keyword"), "{s:?}");
        assert!(has(&s, "fn", "keyword.function"), "{s:?}");
        assert_eq!(syntax.tree.as_ref().unwrap().root_node().to_sexp(), {
            let mut fresh = Syntax::new("rust").unwrap();
            let mut t = text.clone();
            fresh.update(&mut t, FULL);
            fresh.tree.unwrap().root_node().to_sexp()
        });
    }

    #[test]
    fn parsing_resumes_after_running_out_of_time() {
        let src: String = (0..3000)
            .map(|i| format!("fn f{i}() {{ let x = vec![{i}, 2]; }}\n"))
            .collect();
        let mut text = Text::new(&src);
        let mut syntax = Syntax::new("rust").unwrap();
        let mut rounds = 0;
        while !syntax.update(&mut text, Some(Duration::from_micros(200))) {
            rounds += 1;
            assert!(rounds < 100_000);
        }
        assert!(rounds > 0, "the budget should have run out at least once");
        let mut fresh = Syntax::new("rust").unwrap();
        fresh.update(&mut text.clone(), FULL);
        assert_eq!(
            syntax.tree.as_ref().unwrap().root_node().to_sexp(),
            fresh.tree.unwrap().root_node().to_sexp()
        );
        // Edits during a resumed parse restart it on the new text.
        text.apply(&Edit::insert(0, "struct S;\n"));
        while !syntax.update(&mut text, Some(Duration::from_micros(200))) {}
        assert!(
            syntax
                .tree
                .as_ref()
                .unwrap()
                .root_node()
                .to_sexp()
                .starts_with("(source_file (struct_item")
        );
    }

    #[test]
    fn injections_highlight_embedded_code() {
        let mut text = Text::new("# Title\n\nSome *em* text.\n\n```rust\nlet x = 1;\n```\n");
        let mut syntax = Syntax::new("markdown").unwrap();
        syntax.update(&mut text, FULL);
        let s = spans(&syntax, &text);
        assert!(has(&s, "# Title", "markup.heading.1"), "{s:?}");
        assert!(has(&s, "*em*", "markup.italic"), "{s:?}");
        assert!(has(&s, "let", "keyword"), "{s:?}");
        // Rust macros' arguments are Rust.
        let mut text = Text::new("fn f() { println!(\"{}\", x.len()); }\n");
        let mut syntax = Syntax::new("rust").unwrap();
        syntax.update(&mut text, FULL);
        let s = spans(&syntax, &text);
        assert!(has(&s, "len", "function.call"), "{s:?}");
    }
}
