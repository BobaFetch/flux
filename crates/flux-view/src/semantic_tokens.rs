//! LSP semantic tokens, as Neovim's `vim.lsp.semantic_tokens` highlights them by default: for
//! each buffer and server, `textDocument/semanticTokens/full` (or `full/delta` once there's a
//! result to diff against), and `range` for the windows' lines while there's no full result
//! yet. Tokens are decoded with the server's legend into ranges; the ones a window shows are
//! placed as highlights (`@lsp.type.<type>.<ft>` at priority 125, `@lsp.mod.<mod>.<ft>` at 126
//! and `@lsp.typemod.<type>.<mod>.<ft>` at 127) that move with edits like extmarks, until an
//! answer for the text as it is replaces them.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use flux_core::{LineEnding, Revision, Text};
use serde_json::{Value, json};

use crate::lsp::{ClientId, ClientState, Encoding, Pending, follow_edit};
use crate::{BufferId, Cursor, Editor, WindowId};

/// Neovim's `debounce` for requests after a refresh or a scroll.
pub const DEBOUNCE: Duration = Duration::from_millis(200);
/// Neovim asks again right after `didChange`, which it sends 150 ms after the last change
/// (`debounce_text_changes`); flux sends `didChange` at once, so the request waits instead.
pub const CHANGE_DEBOUNCE: Duration = Duration::from_millis(150);
/// `vim.hl.priorities.semantic_tokens`.
pub const PRIORITY: u8 = 125;

pub const FULL: &str = "textDocument/semanticTokens/full";
pub const DELTA: &str = "textDocument/semanticTokens/full/delta";
pub const RANGE: &str = "textDocument/semanticTokens/range";

/// A decoded token: (line, byte column) points, as the text was when it was decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
    pub ty: String,
    pub modifiers: Vec<String>,
    /// Placed as highlights already.
    pub marked: bool,
}

/// A placed highlight (Neovim's extmark): where it was in the text at `revision`, as (line,
/// byte column) points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    pub revision: Revision,
    pub start: (usize, usize),
    pub end: (usize, usize),
    pub group: String,
    pub priority: u8,
}

/// A request in flight: flux's own number for it, the LSP id and the text revision.
#[derive(Debug, Clone, Default)]
struct Active {
    seq: Option<u64>,
    id: Option<i64>,
    revision: Option<Revision>,
}

#[derive(Debug, Clone, Default)]
struct Current {
    revision: Option<Revision>,
    result_id: Option<String>,
    tokens: Vec<u64>,
    highlights: Option<Vec<Token>>,
    namespace_cleared: bool,
}

/// One server's tokens for one buffer (Neovim's `STClientState`).
#[derive(Debug, Clone, Default)]
struct State {
    supports_range: bool,
    supports_delta: bool,
    active: Active,
    active_range: Active,
    current: Current,
    has_full_result: bool,
    marks: Vec<Mark>,
    /// The document version the last `didChange` gave.
    notified: i64,
}

/// What a request was for, until its answer comes.
#[derive(Debug, Clone, Copy)]
struct Sent {
    revision: Revision,
    range: bool,
}

/// Semantic token state for every buffer.
#[derive(Debug, Default)]
pub struct SemanticTokens {
    states: HashMap<(BufferId, ClientId), State>,
    /// When to ask again, per buffer (Neovim's debounce timer).
    timers: HashMap<BufferId, Instant>,
    /// The windows showing each buffer (id, top line, height), to notice scrolling.
    views: HashMap<BufferId, Vec<(WindowId, usize, usize)>>,
    sent: HashMap<u64, Sent>,
    next_seq: u64,
}

impl SemanticTokens {
    /// When a debounced request is due.
    pub fn deadline(&self) -> Option<Instant> {
        self.timers.values().min().copied()
    }

    /// The highlights placed in `buffer`, by every server.
    pub fn marks(&self, buffer: BufferId) -> impl Iterator<Item = &Mark> {
        self.states
            .iter()
            .filter(move |((b, _), _)| *b == buffer)
            .flat_map(|(_, s)| &s.marks)
    }
}

/// Neovim's `modifiers_from_number`: the modifiers whose bits are set in `bits`.
fn modifiers_from_number(mut bits: u64, legend: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while bits > 0 {
        if bits & 1 == 1
            && let Some(m) = legend.get(i)
        {
            out.push(m.clone());
        }
        bits >>= 1;
        i += 1;
    }
    out
}

/// Length of `line` in `enc` units.
fn units(line: &str, enc: Encoding) -> usize {
    match enc {
        Encoding::Utf8 => line.len(),
        Encoding::Utf16 => line.chars().map(char::len_utf16).sum(),
        Encoding::Utf32 => line.chars().count(),
    }
}

/// The byte index of `character` (`enc` units) in `line`, clamped to the line (Neovim's
/// `str_byteindex` without strict indexing).
fn byte_index(line: &str, character: usize, enc: Encoding) -> usize {
    let mut n = 0;
    for (i, c) in line.char_indices() {
        if n >= character {
            return i;
        }
        n += match enc {
            Encoding::Utf8 => c.len_utf8(),
            Encoding::Utf16 => c.len_utf16(),
            Encoding::Utf32 => 1,
        };
    }
    line.len()
}

/// The legend's token types and modifiers.
pub struct Legend {
    pub types: Vec<String>,
    pub modifiers: Vec<String>,
}

impl Legend {
    pub fn from_capability(provider: &Value) -> Self {
        let list = |v: &Value| -> Vec<String> {
            v.as_array()
                .into_iter()
                .flatten()
                .map(|s| s.as_str().unwrap_or("").to_string())
                .collect()
        };
        Self {
            types: list(&provider["legend"]["tokenTypes"]),
            modifiers: list(&provider["legend"]["tokenModifiers"]),
        }
    }
}

/// Neovim's `tokens_to_ranges`: decode the relative token list `data` against `text`, merging
/// into `ranges` (the tokens of an earlier answer for the same text, kept in order and without
/// duplicates).
pub fn tokens_to_ranges(
    data: &[u64],
    text: &Text,
    legend: &Legend,
    enc: Encoding,
    mut ranges: Vec<Token>,
) -> Vec<Token> {
    let line_count = text.line_count();
    let line_at = |n: usize| -> String {
        if n < line_count {
            text.line_str(n).into_owned()
        } else {
            String::new()
        }
    };
    // `\r\n` counts as two units, `\n` as one.
    let eol_offset = if text.line_ending() == LineEnding::Crlf {
        2
    } else {
        1
    };
    let mut last_insert = 0;
    let mut line: Option<usize> = None;
    let mut start_char = 0usize;
    for t in data.as_chunks::<5>().0 {
        let (delta_line, delta_start) = (t[0] as usize, t[1] as usize);
        let l = line.map_or(delta_line, |l| l + delta_line);
        line = Some(l);
        start_char = if delta_line == 0 {
            start_char + delta_start
        } else {
            delta_start
        };
        let Some(ty) = legend.types.get(t[3] as usize) else {
            continue;
        };
        let modifiers = modifiers_from_number(t[4], &legend.modifiers);
        let mut end_char = start_char as i64 + t[2] as i64;
        let mut buf_line = line_at(l);
        let mut end_line = l;
        let start_col = byte_index(&buf_line, start_char, enc);
        // A token going past its line goes on into the next ones.
        let mut new_end = end_char - units(&buf_line, enc) as i64 - eol_offset;
        while new_end > 0 && end_line + 1 < line_count {
            end_char = new_end;
            end_line += 1;
            buf_line = line_at(end_line);
            new_end -= units(&buf_line, enc) as i64 + eol_offset;
        }
        let end_col = byte_index(&buf_line, end_char.max(0) as usize, enc);
        let range = Token {
            line: l,
            start_col,
            end_line,
            end_col,
            ty: ty.clone(),
            modifiers,
            marked: false,
        };
        if last_insert + 1 < ranges.len() {
            let mut needs_insert = true;
            let mut idx = last_insert + ranges[last_insert..].partition_point(|r| r.line < l);
            while idx < ranges.len() {
                let token = &ranges[idx];
                if token.line > l || (token.line == l && token.start_col > start_col) {
                    break;
                }
                if token.line == l
                    && token.start_col == start_col
                    && token.end_line == end_line
                    && token.end_col == end_col
                    && token.ty == range.ty
                {
                    needs_insert = false;
                    break;
                }
                idx += 1;
            }
            last_insert = idx;
            if needs_insert {
                ranges.insert(idx, range);
            }
        } else {
            last_insert = ranges.len();
            ranges.push(range);
        }
    }
    ranges
}

/// A `full/delta` answer's edits applied to the tokens of the answer it's relative to.
pub fn apply_delta(old: &[u64], edits: &[Value]) -> Vec<u64> {
    let mut edits: Vec<&Value> = edits.iter().collect();
    edits.sort_by_key(|e| e["start"].as_u64().unwrap_or(0));
    let mut tokens = Vec::with_capacity(old.len());
    let mut idx = 0usize;
    for e in edits {
        let start = e["start"].as_u64().unwrap_or(0) as usize;
        if idx < start {
            tokens.extend_from_slice(&old[idx.min(old.len())..start.min(old.len())]);
        }
        tokens.extend(
            e["data"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_u64),
        );
        idx = start + e["deleteCount"].as_u64().unwrap_or(0) as usize;
    }
    tokens.extend_from_slice(&old[idx.min(old.len())..]);
    tokens
}

/// Neovim's `mark_dirty`/`reset` cancelling: a `$/cancelRequest` for a request in flight.
fn cancel(editor: &mut Editor, client: ClientId, active: &mut Active) {
    if let Some(id) = active.id.take() {
        let pending = editor
            .lsp
            .client(client)
            .is_some_and(|c| c.pending.contains_key(&id));
        if pending {
            editor
                .lsp
                .notify(client, "$/cancelRequest", json!({ "id": id }));
        }
    }
    active.seq = None;
    active.revision = None;
}

impl Editor {
    /// Keep semantic tokens going (once per frame, before drawing): attach to servers that
    /// have them, ask again after changes, scrolling and refreshes when it's time, and place
    /// the tokens the windows show.
    pub fn semantic_tokens_update(&mut self, now: Instant) {
        self.lsp_sync();
        // The (buffer, client) pairs with semantic tokens, and their document versions.
        let mut attached = Vec::new();
        for c in &self.lsp.clients {
            if c.state != ClientState::Running {
                continue;
            }
            let Some(provider) = c.capability(&["semanticTokensProvider"]) else {
                continue;
            };
            let range = provider.get("range").is_some_and(truthy);
            let delta = provider
                .get("full")
                .and_then(|f| f.get("delta"))
                .is_some_and(truthy);
            for (b, doc) in &c.docs {
                attached.push((*b, c.id, doc.version, range, delta));
            }
        }
        let st = &mut self.lsp.semantic_tokens;
        st.states
            .retain(|k, _| attached.iter().any(|a| (a.0, a.1) == *k));
        st.timers.retain(|b, _| attached.iter().any(|a| a.0 == *b));
        let mut now_buffers = Vec::new();
        for &(b, c, version, range, delta) in &attached {
            match st.states.get_mut(&(b, c)) {
                None => {
                    st.states.insert(
                        (b, c),
                        State {
                            supports_range: range,
                            supports_delta: delta,
                            notified: version,
                            ..State::default()
                        },
                    );
                    now_buffers.push(b);
                }
                Some(s) if s.notified != version => {
                    s.notified = version;
                    st.timers.insert(b, now + CHANGE_DEBOUNCE);
                }
                Some(_) => {}
            }
        }
        // Windows showing the buffers: shown again (BufWinEnter) asks now; scrolled
        // (WinScrolled) asks a moment later when the server takes ranges.
        let mut buffers: Vec<BufferId> = attached.iter().map(|a| a.0).collect();
        buffers.sort_by_key(|b| b.0);
        buffers.dedup();
        for b in buffers {
            let views: Vec<(WindowId, usize, usize)> = self
                .all_windows()
                .filter(|w| w.buffer == b)
                .map(|w| (w.id, w.top, w.height))
                .collect();
            let st = &mut self.lsp.semantic_tokens;
            let before = st.views.insert(b, views.clone()).unwrap_or_default();
            if before.is_empty() && !views.is_empty() {
                now_buffers.push(b);
            } else if before != views
                && !views.is_empty()
                && st
                    .states
                    .iter()
                    .any(|((sb, _), s)| *sb == b && s.supports_range)
            {
                st.timers.insert(b, now + DEBOUNCE);
            }
        }
        let due: Vec<BufferId> = self
            .lsp
            .semantic_tokens
            .timers
            .iter()
            .filter(|(_, t)| **t <= now)
            .map(|(b, _)| *b)
            .collect();
        now_buffers.extend(due);
        now_buffers.sort_by_key(|b| b.0);
        now_buffers.dedup();
        for b in now_buffers {
            self.semantic_tokens_request(b);
        }
        self.semantic_tokens_on_win();
    }

    /// Every window: the current one and the others.
    fn all_windows(&self) -> impl Iterator<Item = &crate::Window> {
        std::iter::once(&self.window).chain(&self.windows)
    }

    /// Neovim's `send_request`: a range request while there's no full result, and a full (or
    /// delta) one when there's no result for the text as it is and none on the way.
    fn semantic_tokens_request(&mut self, buffer: BufferId) {
        self.lsp.semantic_tokens.timers.remove(&buffer);
        let Some(revision) = self.buffer(buffer).map(|b| b.text.revision()) else {
            return;
        };
        let clients: Vec<ClientId> = self
            .lsp
            .semantic_tokens
            .states
            .keys()
            .filter(|(b, _)| *b == buffer)
            .map(|(_, c)| *c)
            .collect();
        for client in clients {
            let Some(s) = self.lsp.semantic_tokens.states.get(&(buffer, client)) else {
                continue;
            };
            if s.supports_range && !s.has_full_result {
                self.semantic_tokens_send(buffer, client, revision, true);
            }
            let Some(s) = self.lsp.semantic_tokens.states.get(&(buffer, client)) else {
                continue;
            };
            if (!s.has_full_result || s.current.revision != Some(revision))
                && s.active.revision != Some(revision)
            {
                self.semantic_tokens_send(buffer, client, revision, false);
            }
        }
    }

    fn semantic_tokens_send(
        &mut self,
        buffer: BufferId,
        client: ClientId,
        revision: Revision,
        range: bool,
    ) {
        let key = (buffer, client);
        let Some(mut s) = self.lsp.semantic_tokens.states.remove(&key) else {
            return;
        };
        // A request for an older text is cancelled.
        let mut active = std::mem::take(if range {
            &mut s.active_range
        } else {
            &mut s.active
        });
        cancel(self, client, &mut active);
        let uri = self
            .lsp
            .client(client)
            .and_then(|c| c.docs.get(&buffer))
            .map(|d| d.uri.clone());
        let mut params = json!({ "textDocument": { "uri": uri } });
        let method = if range {
            params["range"] = self.semantic_tokens_overscan(buffer);
            RANGE
        } else if s.supports_delta
            && let Some(id) = &s.current.result_id
        {
            params["previousResultId"] = json!(id);
            DELTA
        } else {
            FULL
        };
        let st = &mut self.lsp.semantic_tokens;
        let seq = st.next_seq;
        st.next_seq += 1;
        st.sent.insert(seq, Sent { revision, range });
        let id = self.lsp.request(
            client,
            params,
            Pending {
                method: method.into(),
                buffer: Some(buffer),
                data: json!({ "seq": seq }),
            },
        );
        if id.is_some() {
            active = Active {
                seq: Some(seq),
                id,
                revision: Some(revision),
            };
        }
        if range {
            s.active_range = active;
        } else {
            s.active = active;
        }
        self.lsp.semantic_tokens.states.insert(key, s);
    }

    /// Neovim's `get_overscan_range`: the lines the windows showing `buffer` show, and as many
    /// again above and below.
    fn semantic_tokens_overscan(&self, buffer: BufferId) -> Value {
        let lines = self.buffer(buffer).map_or(0, |b| b.text.line_count());
        let mut span: Option<(usize, usize)> = None;
        for w in self.all_windows().filter(|w| w.buffer == buffer) {
            let start = w.top.saturating_sub(w.height);
            let end = (w.top + w.height + w.height).min(lines);
            span = Some(span.map_or((start, end), |(s, e)| (s.min(start), e.max(end))));
        }
        let (start, end) = span.unwrap_or((0, 0));
        json!({
            "start": { "line": start, "character": 0 },
            "end": { "line": end, "character": 0 },
        })
    }

    /// An answer to a semantic tokens request (`None` for an error).
    pub fn semantic_tokens_response(
        &mut self,
        client: ClientId,
        pending: &Pending,
        result: Option<&Value>,
    ) {
        let Some(seq) = pending.data["seq"].as_u64() else {
            return;
        };
        let Some(sent) = self.lsp.semantic_tokens.sent.remove(&seq) else {
            return;
        };
        let Some(buffer) = pending.buffer else {
            return;
        };
        let key = (buffer, client);
        let Some(s) = self.lsp.semantic_tokens.states.get_mut(&key) else {
            return;
        };
        let result = result.filter(|r| !r.is_null());
        // An error, no answer, or (for a range) a full result already in.
        let Some(response) = result.filter(|_| !(sent.range && s.has_full_result)) else {
            let active = if sent.range {
                &mut s.active_range
            } else {
                &mut s.active
            };
            active.seq = None;
            active.id = None;
            active.revision = None;
            return;
        };
        let active = if sent.range {
            &s.active_range
        } else {
            &s.active
        };
        // A stale answer.
        if active.seq.is_some_and(|a| a != seq) {
            return;
        }
        let Some(c) = self.lsp.client(client) else {
            return;
        };
        let legend = Legend::from_capability(&c.capabilities["semanticTokensProvider"]);
        let enc = c.encoding;
        let Some(text) = self
            .buffers
            .iter()
            .find(|b| b.id == buffer)
            .map(|b| &b.text)
        else {
            return;
        };
        let Some(s) = self.lsp.semantic_tokens.states.get_mut(&key) else {
            return;
        };
        let tokens: Vec<u64> = match response.get("edits").and_then(Value::as_array) {
            Some(edits) => apply_delta(&s.current.tokens, edits),
            None => response["data"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_u64)
                .collect(),
        };
        let changed = s.current.revision != Some(sent.revision);
        let earlier = match s.current.highlights.take() {
            Some(h) if !changed => h,
            _ => Vec::new(),
        };
        let highlights = tokens_to_ranges(&tokens, text, &legend, enc, earlier);
        if !sent.range {
            s.has_full_result = true;
        }
        let active = if sent.range {
            &mut s.active_range
        } else {
            &mut s.active
        };
        *active = Active::default();
        s.current.revision = Some(sent.revision);
        s.current.result_id = if sent.range {
            None
        } else {
            response["resultId"].as_str().map(str::to_owned)
        };
        s.current.tokens = tokens;
        s.current.highlights = Some(highlights);
        if changed {
            s.current.namespace_cleared = false;
        }
    }

    /// `workspace/semanticTokens/refresh` from `client` (Neovim's `_refresh`): its results
    /// are out of date; buffers in windows ask again after a moment.
    pub fn semantic_tokens_refresh(&mut self, client: ClientId, now: Instant) {
        let buffers: Vec<BufferId> = self
            .lsp
            .semantic_tokens
            .states
            .keys()
            .filter(|(_, c)| *c == client)
            .map(|(b, _)| *b)
            .collect();
        for b in buffers {
            let Some(mut s) = self.lsp.semantic_tokens.states.remove(&(b, client)) else {
                continue;
            };
            // Neovim's `mark_dirty`.
            s.current.revision = None;
            s.has_full_result = false;
            cancel(self, client, &mut s.active_range);
            cancel(self, client, &mut s.active);
            self.lsp.semantic_tokens.states.insert((b, client), s);
            if self.all_windows().any(|w| w.buffer == b) {
                self.lsp.semantic_tokens.timers.insert(b, now + DEBOUNCE);
            }
        }
    }

    /// Neovim's `on_win`: with a result for the text as it is, the highlights of the last one
    /// go, and the tokens the windows show are placed.
    fn semantic_tokens_on_win(&mut self) {
        let views: Vec<(BufferId, usize, usize)> = self
            .all_windows()
            .map(|w| (w.buffer, w.top, w.top + w.height))
            .collect();
        for (buffer, top, bottom) in views {
            let Some(b) = self.buffers.iter().find(|b| b.id == buffer) else {
                continue;
            };
            let revision = b.text.revision();
            let ft = b.opts.filetype.clone();
            for ((sb, _), s) in self.lsp.semantic_tokens.states.iter_mut() {
                if *sb != buffer || s.current.revision != Some(revision) {
                    continue;
                }
                if !s.current.namespace_cleared {
                    s.marks.clear();
                    s.current.namespace_cleared = true;
                }
                let Some(highlights) = s.current.highlights.as_mut() else {
                    continue;
                };
                let first = highlights.partition_point(|h| h.end_line < top);
                let last = first + highlights[first..].partition_point(|h| h.line <= bottom);
                for token in &mut highlights[first..last] {
                    if token.marked {
                        continue;
                    }
                    let mut mark = |group: String, priority: u8| {
                        s.marks.push(Mark {
                            revision,
                            start: (token.line, token.start_col),
                            end: (token.end_line, token.end_col),
                            group,
                            priority,
                        });
                    };
                    mark(format!("@lsp.type.{}.{ft}", token.ty), PRIORITY);
                    for m in &token.modifiers {
                        mark(format!("@lsp.mod.{m}.{ft}"), PRIORITY + 1);
                        mark(format!("@lsp.typemod.{}.{m}.{ft}", token.ty), PRIORITY + 2);
                    }
                    token.marked = true;
                }
            }
        }
    }

    /// The semantic token highlights of buffer `id` as chars of its text, moved with the edits
    /// since they were placed: `(start, end, group, priority)`, in the order to paint them.
    pub fn semantic_token_highlights(&self, id: BufferId) -> Vec<(Cursor, Cursor, &str, u8)> {
        let Some(buffer) = self.buffer(id) else {
            return Vec::new();
        };
        let text = &buffer.text;
        let cursor = |(line, byte): (usize, usize)| {
            let line = line.min(text.last_line());
            let s = text.line_str(line);
            let col = s.char_indices().take_while(|&(i, _)| i < byte).count();
            Cursor { line, col }
        };
        let mut out: Vec<(Cursor, Cursor, &str, u8)> = self
            .lsp
            .semantic_tokens
            .marks(id)
            .filter_map(|m| {
                let (mut s, mut e) = (m.start, m.end);
                for edit in text.edits_since(m.revision)? {
                    s = follow_edit(s, &edit.bytes, true);
                    e = follow_edit(e, &edit.bytes, false);
                }
                Some((cursor(s), cursor(e.max(s)), m.group.as_str(), m.priority))
            })
            .collect();
        out.sort_by_key(|h| h.3);
        out
    }
}

fn truthy(v: &Value) -> bool {
    !matches!(v, Value::Null | Value::Bool(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legend() -> Legend {
        Legend {
            types: ["function", "variable", "string"]
                .map(String::from)
                .to_vec(),
            modifiers: ["declaration", "readonly"].map(String::from).to_vec(),
        }
    }

    /// A token as (start line, start col, end line, end col, type, modifiers).
    type Flat<'a> = (usize, usize, usize, usize, &'a str, Vec<&'a str>);

    fn spans(tokens: &[Token]) -> Vec<Flat<'_>> {
        tokens
            .iter()
            .map(|t| {
                (
                    t.line,
                    t.start_col,
                    t.end_line,
                    t.end_col,
                    t.ty.as_str(),
                    t.modifiers.iter().map(String::as_str).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn decodes_relative_tokens_with_the_legend() {
        let text = Text::new("fn main() {\n    let x = 1;\n}\n");
        let data = [0, 3, 4, 0, 1, 1, 8, 1, 1, 3, 0, 1, 1, 9, 0];
        let got = tokens_to_ranges(&data, &text, &legend(), Encoding::Utf16, Vec::new());
        assert_eq!(
            spans(&got),
            [
                (0, 3, 0, 7, "function", vec!["declaration"]),
                (1, 8, 1, 9, "variable", vec!["declaration", "readonly"]),
            ]
        );
    }

    #[test]
    fn positions_count_in_the_servers_encoding() {
        // "é" is 2 bytes, 1 UTF-16 unit; "𝕏" is 4 bytes, 2 UTF-16 units.
        let text = Text::new("é𝕏 ab\n");
        let utf16 = tokens_to_ranges(&[0, 4, 2, 1, 0], &text, &legend(), Encoding::Utf16, vec![]);
        assert_eq!((utf16[0].start_col, utf16[0].end_col), (7, 9));
        let utf8 = tokens_to_ranges(&[0, 7, 2, 1, 0], &text, &legend(), Encoding::Utf8, vec![]);
        assert_eq!((utf8[0].start_col, utf8[0].end_col), (7, 9));
        let utf32 = tokens_to_ranges(&[0, 3, 2, 1, 0], &text, &legend(), Encoding::Utf32, vec![]);
        assert_eq!((utf32[0].start_col, utf32[0].end_col), (7, 9));
    }

    #[test]
    fn multiline_tokens_go_on_into_the_next_lines() {
        let text = Text::new("a = \"one\ntwo\nthree\" b\n");
        // From col 4 of line 0: `"one` (4) + \n + `two` (3) + \n + `three"` (6) = 15.
        let got = tokens_to_ranges(&[0, 4, 15, 2, 0], &text, &legend(), Encoding::Utf16, vec![]);
        assert_eq!(spans(&got), [(0, 4, 2, 6, "string", vec![])]);
        let dos = Text::new("a = \"one\r\ntwo\r\nthree\" b\r\n");
        let got = tokens_to_ranges(&[0, 4, 17, 2, 0], &dos, &legend(), Encoding::Utf16, vec![]);
        assert_eq!(spans(&got), [(0, 4, 2, 6, "string", vec![])]);
    }

    #[test]
    fn unknown_types_are_skipped_but_still_move_the_position() {
        let text = Text::new("aa bb cc\n");
        let got = tokens_to_ranges(
            &[0, 0, 2, 9, 0, 0, 3, 2, 1, 0, 0, 3, 2, 0, 4],
            &text,
            &legend(),
            Encoding::Utf16,
            vec![],
        );
        assert_eq!(
            spans(&got),
            [
                (0, 3, 0, 5, "variable", vec![]),
                (0, 6, 0, 8, "function", vec![])
            ]
        );
    }

    #[test]
    fn a_later_answer_merges_in_order_without_duplicates() {
        let text = Text::new("aa bb cc dd\n");
        let l = legend();
        let range = tokens_to_ranges(
            &[0, 0, 2, 0, 0, 0, 6, 2, 0, 0],
            &text,
            &l,
            Encoding::Utf16,
            vec![],
        );
        let full = tokens_to_ranges(
            &[0, 0, 2, 0, 0, 0, 3, 2, 1, 0, 0, 3, 2, 0, 0, 0, 3, 2, 1, 0],
            &text,
            &l,
            Encoding::Utf16,
            range,
        );
        let cols: Vec<usize> = full.iter().map(|t| t.start_col).collect();
        assert_eq!(cols, [0, 3, 6, 9]);
    }

    #[test]
    fn deltas_edit_the_previous_tokens() {
        let old = [0, 0, 2, 0, 0, 0, 3, 2, 1, 0, 1, 0, 1, 2, 0];
        let edits = [
            json!({ "start": 10, "deleteCount": 5, "data": [2, 0, 1, 2, 0] }),
            json!({ "start": 5, "deleteCount": 0, "data": [0, 1, 1, 1, 0] }),
        ];
        assert_eq!(
            apply_delta(&old, &edits),
            [0, 0, 2, 0, 0, 0, 1, 1, 1, 0, 0, 3, 2, 1, 0, 2, 0, 1, 2, 0]
        );
        assert_eq!(apply_delta(&old, &[]), old);
    }

    #[test]
    fn placed_tokens_follow_edits() {
        let mut editor = Editor::new(80, 24);
        let id = editor.window.buffer;
        editor.current_buffer_mut().text = Text::new("let total = 1;\n");
        let revision = editor.current_buffer().text.revision();
        let state = State {
            marks: vec![Mark {
                revision,
                start: (0, 4),
                end: (0, 9),
                group: "@lsp.type.variable.rust".into(),
                priority: PRIORITY,
            }],
            ..State::default()
        };
        editor
            .lsp
            .semantic_tokens
            .states
            .insert((id, ClientId(1)), state);
        // Insert in front of the token, then inside it.
        let text = &mut editor.current_buffer_mut().text;
        text.apply(&flux_core::Edit::insert(0, "  "));
        text.apply(&flux_core::Edit::insert(8, "xx"));
        let got = editor.semantic_token_highlights(id);
        assert_eq!(
            got,
            [(
                Cursor { line: 0, col: 6 },
                Cursor { line: 0, col: 13 },
                "@lsp.type.variable.rust",
                PRIORITY
            )]
        );
    }
}
