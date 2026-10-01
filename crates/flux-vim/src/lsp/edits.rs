//! The answers to the requests that edit (rename, code actions, formatting), and
//! `vim.lsp.formatexpr()`, which `gq` uses when a server can format a range.

use std::time::{Duration, Instant};

use flux_core::Text;
use flux_view::lsp::{Client, ClientId, ClientState, Pending};
use flux_view::{BufferId, Cursor, Editor};
use serde_json::{Value, json};

use crate::engine::Engine;

/// `lsp_request_all`, saying so when servers are attached but none can answer `method`
/// (Neovim's `buf_request`).
pub(crate) fn request_all(
    editor: &mut Editor,
    method: &str,
    capability: &str,
    params: impl Fn(&Client, &Text, BufferId, Cursor) -> Value,
    data: Value,
) -> usize {
    let n = editor.lsp_request_all(method, capability, params, data);
    if n == 0 && !editor.lsp.clients_for(editor.window.buffer).is_empty() {
        editor.error(format!(
            "vim.lsp: method \"{method}\" is not supported by any server activated for this buffer"
        ));
    }
    n
}

/// An answer to one of the requests made here. Returns whether it was one.
pub(crate) fn answer(
    engine: &mut Engine,
    editor: &mut Editor,
    client: ClientId,
    pending: &Pending,
    msg: &Value,
) -> bool {
    let answer = match msg.get("error") {
        Some(e) => Err(e),
        None => Ok(&msg["result"]),
    };
    match pending.method.as_str() {
        "textDocument/prepareRename" => {
            super::rename::prepared(engine, editor, client, pending, answer)
        }
        "textDocument/rename" => super::rename::renamed(engine, editor, client, pending, answer),
        "codeAction/resolve" => super::code_action::resolved(editor, client, pending, answer),
        // Neovim's handler does nothing with it, apart from reporting an error.
        "workspace/executeCommand" => {
            if let Err(e) = answer {
                handler_error(editor, client, e);
            }
        }
        "textDocument/rangeFormatting" => {
            // An answer that comes after giving up is ignored.
            let id = msg["id"].as_i64();
            if editor
                .lsp
                .waiting
                .is_some_and(|(c, i, _)| c == client && Some(i) == id)
            {
                editor.lsp.waiting = None;
                formatted(editor, client, pending, answer);
            }
        }
        _ => return false,
    }
    true
}

/// What Neovim's default handlers do with an error answer: say so (unless the document had
/// changed, `ContentModified`). Returns whether the handler stops there, which it does for all
/// but `ServerCancelled`.
pub(crate) fn handler_error(editor: &mut Editor, client: ClientId, err: &Value) -> bool {
    let code = err["code"].as_i64();
    if code == Some(-32802) {
        return false;
    }
    if code != Some(-32801) {
        let name = editor
            .lsp
            .client(client)
            .map_or_else(|| format!("client_id={}", client.0), |c| c.name.clone());
        let code = match &err["code"] {
            Value::Number(n) => n.to_string(),
            v => v.to_string(),
        };
        editor.error(format!(
            "{name}: {code}: {}",
            err["message"].as_str().unwrap_or("")
        ));
    }
    true
}

/// The servers attached to the current buffer that can format a range, in order.
fn range_formatters(editor: &Editor) -> Vec<ClientId> {
    let buffer = editor.window.buffer;
    editor
        .lsp
        .clients
        .iter()
        .filter(|c| c.state == ClientState::Running && c.docs.contains_key(&buffer))
        .filter(|c| c.capability(&["documentRangeFormattingProvider"]).is_some())
        .map(|c| c.id)
        .collect()
}

/// Whether `gq` formats with a server ('formatexpr' is `v:lua.vim.lsp.formatexpr()` once a
/// server that can format a range is attached).
pub(crate) fn formats(editor: &Editor) -> bool {
    !range_formatters(editor).is_empty()
}

/// `vim.lsp.formatexpr()` for lines `first..=last`: the first server that formats the range
/// has its edits applied (when the answer comes, where Neovim waits for it).
pub(crate) fn format_lines(editor: &mut Editor, first: usize, last: usize) {
    let clients = range_formatters(editor);
    editor.lsp_sync();
    ask_to_format(editor, editor.window.buffer, clients, first, last);
}

fn ask_to_format(
    editor: &mut Editor,
    buffer: BufferId,
    mut clients: Vec<ClientId>,
    first: usize,
    last: usize,
) {
    if clients.is_empty() {
        return;
    }
    let client = clients.remove(0);
    let Some(c) = editor.lsp.client(client) else {
        return;
    };
    let Some(b) = editor.buffer(buffer) else {
        return;
    };
    let o = &b.opts;
    let end_line = b.text.line_str(last.min(b.text.last_line()));
    let end_col = flux_view::lsp::to_lsp_col(&end_line, end_line.chars().count(), c.encoding);
    let params = json!({
        "textDocument": { "uri": c.docs.get(&buffer).map(|d| d.uri.clone()) },
        "options": {
            "tabSize": if o.shiftwidth == 0 { o.tabstop } else { o.shiftwidth },
            "insertSpaces": o.expandtab,
        },
        "range": {
            "start": { "line": first, "character": 0 },
            "end": { "line": last, "character": end_col },
        },
    });
    let id = editor.lsp.request(
        client,
        params,
        Pending {
            method: "textDocument/rangeFormatting".into(),
            buffer: Some(buffer),
            data: json!({
                "rest": clients.iter().map(|c| c.0).collect::<Vec<_>>(),
                "first": first,
                "last": last,
            }),
        },
    );
    // `vim.lsp.formatexpr()` waits up to 500 ms for the answer.
    editor.lsp.waiting = id.map(|id| (client, id, Instant::now() + Duration::from_millis(500)));
}

/// The answer to `textDocument/rangeFormatting`: its edits, or the next server.
fn formatted(
    editor: &mut Editor,
    client: ClientId,
    pending: &Pending,
    answer: Result<&Value, &Value>,
) {
    let Some(buffer) = pending.buffer else {
        return;
    };
    match answer {
        Ok(Value::Array(edits)) => {
            let enc = super::encoding(editor, client);
            let edits = super::text_edits(editor, buffer, enc, edits);
            editor.apply_buffer_edits(buffer, edits);
        }
        _ => {
            let rest: Vec<ClientId> = pending.data["rest"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_u64().map(|n| ClientId(n as usize)))
                .collect();
            let n = |k: &str| pending.data[k].as_u64().unwrap_or(0) as usize;
            ask_to_format(editor, buffer, rest, n("first"), n("last"));
        }
    }
}
