//! Language server messages: answering the server's requests, using its notifications, and
//! the answers to the requests flux's commands make (`K`, `grr`, …), as Neovim's
//! `vim.lsp.handlers` and `vim.lsp.buf` do.

use flux_core::Edit;
use flux_view::Editor;
use flux_view::lsp::{ClientId, ClientState, Encoding, Outgoing, Pending, from_lsp, uri_to_path};
use serde_json::{Value, json};

use crate::engine::Engine;

mod commands;
pub(crate) mod completion;
mod hover;
mod locations;
mod snippet;

/// Something a server sent.
pub fn handle_message(engine: &mut Engine, editor: &mut Editor, client: ClientId, msg: Value) {
    message(engine, editor, client, msg);
    // Diagnostics may have brought the sign column, which narrows the text.
    editor.refresh_window_widths();
    editor.with_window(|w, m| w.scroll_to_cursor(m));
    editor.pum_ruler_check();
}

fn message(engine: &mut Engine, editor: &mut Editor, client: ClientId, msg: Value) {
    let method = msg["method"].as_str().map(str::to_owned);
    let id = msg.get("id").cloned();
    match (method, id) {
        (Some(method), Some(id)) => server_request(editor, client, &method, id, &msg["params"]),
        (Some(method), None) => notification(editor, client, &method, &msg["params"]),
        (None, Some(id)) => {
            let Some(id) = id.as_i64() else {
                return;
            };
            let Some(pending) = editor
                .lsp
                .client_mut(client)
                .and_then(|c| c.pending.remove(&id))
            else {
                return;
            };
            // An answer in a group waits for the others.
            if let Some(group) = pending.data["group"].as_u64() {
                let answer = match msg.get("error") {
                    Some(e) => Err(e.clone()),
                    None => Ok(msg["result"].clone()),
                };
                if let Some(done) = editor.lsp.group_answer(group, client, answer) {
                    group_done(engine, editor, done);
                }
                return;
            }
            match msg.get("error") {
                Some(error) => response_error(editor, client, &pending, error),
                None => response(engine, editor, client, pending, &msg["result"]),
            }
        }
        (None, None) => {}
    }
}

/// The server process ended. Like Neovim, an unexpected end is reported.
pub fn handle_exit(editor: &mut Editor, client: ClientId, why: &str) {
    let Some(c) = editor.lsp.client(client) else {
        return;
    };
    let expected = c.state == ClientState::Stopping;
    let name = c.name.clone();
    editor.lsp_exited(client);
    if !expected {
        editor.error(format!("Client {name} quit: {why}"));
    }
}

fn server_request(editor: &mut Editor, client: ClientId, method: &str, id: Value, params: &Value) {
    let result = match method {
        "workspace/configuration" => {
            let settings = editor
                .lsp
                .client(client)
                .map(|c| c.config.settings.clone())
                .unwrap_or(Value::Null);
            let items = params["items"].as_array().cloned().unwrap_or_default();
            Ok(Value::Array(
                items
                    .iter()
                    .map(|item| match item["section"].as_str() {
                        Some(section) => lookup_section(&settings, section)
                            .or_else(|| section.is_empty().then(|| settings.clone()))
                            .unwrap_or(Value::Null),
                        None => settings.clone(),
                    })
                    .collect(),
            ))
        }
        "workspace/workspaceFolders" => Ok(editor
            .lsp
            .client(client)
            .and_then(|c| c.root.clone())
            .map_or(Value::Null, |r| {
                json!([{ "uri": flux_view::lsp::path_to_uri(&r), "name": r.to_string_lossy() }])
            })),
        "workspace/applyEdit" => {
            let applied = apply_workspace_edit(editor, client, &params["edit"]);
            Ok(json!({ "applied": applied }))
        }
        "workspace/semanticTokens/refresh" => {
            editor.semantic_tokens_refresh(client, std::time::Instant::now());
            Ok(Value::Null)
        }
        "window/workDoneProgress/create"
        | "client/registerCapability"
        | "client/unregisterCapability"
        | "workspace/inlayHint/refresh"
        | "workspace/codeLens/refresh"
        | "workspace/diagnostic/refresh"
        | "window/showMessageRequest" => Ok(Value::Null),
        _ => Err((-32601, format!("method not found: {method}"))),
    };
    editor.lsp.respond(client, id, result);
}

/// Neovim's `lookup_section`: `a.b.c` in nested settings.
fn lookup_section(settings: &Value, section: &str) -> Option<Value> {
    if let Some(v) = settings.get(section) {
        return Some(v.clone());
    }
    let mut v = settings;
    for key in section.split('.') {
        v = v.get(key)?;
    }
    Some(v.clone())
}

fn notification(editor: &mut Editor, client: ClientId, method: &str, params: &Value) {
    match method {
        "textDocument/publishDiagnostics" => editor.lsp_publish_diagnostics(client, params),
        "window/showMessage" => {
            let name = editor
                .lsp
                .client(client)
                .map_or(String::new(), |c| c.name.clone());
            let text = format!("LSP[{name}] {}", params["message"].as_str().unwrap_or(""));
            match params["type"].as_i64() {
                Some(1) => editor.error(text),
                _ => editor.info(text),
            }
        }
        // Progress and log messages aren't shown by default (Neovim keeps them in its log).
        _ => {}
    }
}

fn response_error(editor: &mut Editor, client: ClientId, pending: &Pending, error: &Value) {
    // Neovim only logs these.
    if pending.method.starts_with("textDocument/semanticTokens/") {
        editor.semantic_tokens_response(client, pending, None);
        return;
    }
    // A cancelled request (the document changed) isn't worth a message.
    if matches!(error["code"].as_i64(), Some(-32800 | -32801)) {
        return;
    }
    let name = editor
        .lsp
        .client(client)
        .map_or(String::new(), |c| c.name.clone());
    let msg = error["message"].as_str().unwrap_or("error");
    editor.error(format!("{}: {name}: {msg}", pending.method));
}

fn response(
    engine: &mut Engine,
    editor: &mut Editor,
    client: ClientId,
    pending: Pending,
    result: &Value,
) {
    match pending.method.as_str() {
        "initialize" => editor.lsp_initialized(client, result),
        m if m.starts_with("textDocument/semanticTokens/") => {
            editor.semantic_tokens_response(client, &pending, Some(result));
        }
        "completionItem/resolve" => completion::resolved(engine, editor, client, &pending, result),
        "shutdown" => {
            editor.lsp.notify(client, "exit", Value::Null);
            editor.lsp.outbox.push(Outgoing::Kill { client });
        }
        _ => {}
    }
}

/// All the servers asked have answered.
fn group_done(engine: &mut Engine, editor: &mut Editor, group: flux_view::lsp::Group) {
    match group.method.as_str() {
        "textDocument/hover" => hover::show(editor, group),
        "textDocument/completion" => completion::show(engine, editor, group),
        "textDocument/references"
        | "textDocument/implementation"
        | "textDocument/typeDefinition"
        | "textDocument/documentSymbol"
        | "textDocument/definition" => locations::done(editor, group),
        _ => {}
    }
}

/// The encoding `client` counts positions in.
pub(crate) fn encoding(editor: &Editor, client: ClientId) -> Encoding {
    editor
        .lsp
        .client(client)
        .map_or(Encoding::Utf16, |c| c.encoding)
}

/// LSP text edits as edits of buffer `buffer`'s text.
pub(crate) fn text_edits(
    editor: &Editor,
    buffer: flux_view::BufferId,
    enc: Encoding,
    edits: &[Value],
) -> Vec<Edit> {
    let Some(b) = editor.buffer(buffer) else {
        return Vec::new();
    };
    let text = &b.text;
    edits
        .iter()
        .filter_map(|e| {
            let range = &e["range"];
            let pos = |p: &Value| -> Option<flux_view::Cursor> {
                Some(from_lsp(
                    text,
                    p["line"].as_u64()? as usize,
                    p["character"].as_u64()? as usize,
                    enc,
                ))
            };
            let at = |p: &Value| -> Option<usize> {
                let line = p["line"].as_u64()? as usize;
                // A position past the last line is the end of the text.
                if line > text.last_line() {
                    return Some(text.len_chars());
                }
                let c = pos(p)?;
                Some(text.pos_to_char(c.line, c.col))
            };
            let start = at(&range["start"])?;
            let end = at(&range["end"])?.max(start);
            let new = e["newText"].as_str().unwrap_or("").replace("\r\n", "\n");
            Some(Edit::replace(start..end, new))
        })
        .collect()
}

/// Apply a `WorkspaceEdit` (Neovim's `apply_workspace_edit`): edits to open buffers, and to
/// files that aren't open, which are read into hidden buffers.
pub(crate) fn apply_workspace_edit(editor: &mut Editor, client: ClientId, edit: &Value) -> bool {
    let enc = encoding(editor, client);
    let mut by_uri: Vec<(String, Vec<Value>)> = Vec::new();
    if let Some(changes) = edit["documentChanges"].as_array() {
        for change in changes {
            if let Some(kind) = change["kind"].as_str() {
                resource_operation(editor, kind, change);
                continue;
            }
            let uri = change["textDocument"]["uri"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let edits = change["edits"].as_array().cloned().unwrap_or_default();
            by_uri.push((uri, edits));
        }
    } else if let Some(changes) = edit["changes"].as_object() {
        for (uri, edits) in changes {
            by_uri.push((uri.clone(), edits.as_array().cloned().unwrap_or_default()));
        }
    }
    for (uri, edits) in by_uri {
        let Some(path) = uri_to_path(&uri) else {
            continue;
        };
        let Ok(buffer) = editor.buffer_for_path(&path) else {
            continue;
        };
        let edits = text_edits(editor, buffer, enc, &edits);
        editor.apply_buffer_edits(buffer, edits);
    }
    true
}

/// A file created, renamed or deleted as part of a workspace edit.
fn resource_operation(editor: &mut Editor, kind: &str, op: &Value) {
    let path = |key: &str| op[key].as_str().and_then(uri_to_path);
    match kind {
        "create" => {
            if let Some(p) = path("uri")
                && !p.exists()
            {
                if let Some(dir) = p.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(&p, "");
                let _ = editor.buffer_for_path(&p);
            }
        }
        "rename" => {
            if let (Some(from), Some(to)) = (path("oldUri"), path("newUri")) {
                if let Some(dir) = to.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::rename(&from, &to);
                if let Some(id) = editor.find_buffer(&from)
                    && let Some(b) = editor.buffer_mut(id)
                {
                    b.path = Some(to);
                }
            }
        }
        "delete" => {
            if let Some(p) = path("uri") {
                let _ = if p.is_dir() {
                    std::fs::remove_dir_all(&p)
                } else {
                    std::fs::remove_file(&p)
                };
                if let Some(id) = editor.find_buffer(&p) {
                    let _ = editor.delete_buffer(id, true, true);
                }
            }
        }
        _ => {}
    }
}
