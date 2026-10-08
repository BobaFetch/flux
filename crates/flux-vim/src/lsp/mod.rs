//! Language server messages: answering the server's requests, using its notifications, and
//! the answers to the requests flux's commands make (`K`, `grr`, …), as Neovim's
//! `vim.lsp.handlers` and `vim.lsp.buf` do.

use std::path::Path;

use flux_core::Edit;
use flux_view::Editor;
use flux_view::lsp::{ClientId, ClientState, Encoding, Outgoing, Pending, from_lsp, uri_to_path};
use serde_json::{Value, json};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::engine::Engine;

pub(crate) mod code_action;
mod commands;
pub(crate) mod completion;
pub(crate) mod edits;
pub(crate) mod ex_lsp;
mod hover;
mod locations;
pub(crate) mod rename;
pub(crate) mod signature;
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
            if edits::answer(engine, editor, client, &pending, &msg) {
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

/// The server process ended. Auto-enabled servers that never initialized warn once; all
/// other unexpected exits keep Neovim's error message.
pub fn handle_exit(editor: &mut Editor, client: ClientId, why: &str, log: Option<&Path>) {
    let Some(c) = editor.lsp.client(client) else {
        return;
    };
    let expected = c.state == ClientState::Stopping;
    let initializing = c.state == ClientState::Initializing;
    let name = c.name.clone();
    editor.lsp_exited(client);
    if !expected {
        if initializing && editor.lsp.auto_enabled.contains(&name) {
            editor.lsp.enabled.retain(|c| c.name != name);
            if !editor.lsp.failed.contains(&name) {
                editor.lsp.failed.push(name);
                let warning = startup_warning(&editor.lsp.failed, log, editor.screen_size().0);
                editor.warning_after_waiting(warning);
            }
        } else {
            editor.error(format!("Client {name} quit {why}"));
        }
    }
    ex_lsp::exited(editor, client);
}

/// Pick the first warning that fits without wrapping. The fallback cuts by display columns,
/// never in the middle of a multibyte character.
fn startup_warning(names: &[String], log: Option<&Path>, width: usize) -> String {
    let names = names.join(", ");
    if let Some(log) = log {
        let text = format!("{names} failed to start; see {}", log.display());
        if UnicodeWidthStr::width(text.as_str()) < width {
            return text;
        }
    }
    let text = format!("{names} failed to start; see lsp.log");
    if UnicodeWidthStr::width(text.as_str()) < width {
        return text;
    }
    let available = width.saturating_sub(1);
    if available == 0 {
        return String::new();
    }
    let mut short = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let columns = ch.width().unwrap_or(0);
        if used + columns >= available {
            break;
        }
        short.push(ch);
        used += columns;
    }
    short.push('…');
    short
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
        "textDocument/codeAction" => code_action::show(engine, editor, group),
        "textDocument/signatureHelp" => signature::show(editor, group),
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

#[cfg(test)]
mod tests {
    use super::*;
    use flux_lsp::ServerConfig;
    use flux_view::MessageKind;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    const WHY: &str = "with exit code 1 and signal 0. Check log for errors: /tmp/x/lsp.log";

    fn setup(width: usize, names: &[&str]) -> (Editor, Vec<ClientId>, PathBuf) {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "flux-lsp-startup-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("main.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();
        let mut editor = Editor::new(width, 24);
        for &name in names {
            let config = ServerConfig {
                name: name.into(),
                cmd: vec!["fake".into()],
                filetypes: vec!["rust".into()],
                root_markers: vec![],
                settings: json!({}),
                init_options: Value::Null,
            };
            editor.lsp.configs.push(config.clone());
            editor.lsp.enabled.push(config);
        }
        editor.open(&file);
        let ids = editor.lsp.clients.iter().map(|c| c.id).collect();
        (editor, ids, dir)
    }

    fn fail(editor: &mut Editor, id: ClientId) {
        handle_exit(editor, id, WHY, Some(Path::new("/tmp/x/lsp.log")));
    }

    #[test]
    fn auto_failure_is_one_line_without_prompt() {
        let (mut editor, ids, _) = setup(80, &["fake"]);
        editor.lsp.auto_enabled.insert("fake".into());
        let errors = editor.error_count;
        fail(&mut editor, ids[0]);
        assert!(!editor.hit_enter);
        assert_eq!(editor.error_count, errors);
        let message = editor.message.as_ref().unwrap();
        assert_eq!(message.kind, MessageKind::Warning);
        assert_eq!(message.text, "fake failed to start; see /tmp/x/lsp.log");
        assert_eq!(editor.lsp.failed, ["fake"]);
    }

    #[test]
    fn auto_failure_disables_config_for_session() {
        let (mut editor, ids, dir) = setup(80, &["fake"]);
        editor.lsp.auto_enabled.insert("fake".into());
        fail(&mut editor, ids[0]);
        assert!(editor.lsp.enabled.is_empty());
        assert_eq!(editor.lsp.configs.len(), 1);
        let client_count = editor.lsp.clients.len();
        editor.lsp.outbox.clear();
        let other = dir.join("other.rs");
        std::fs::write(&other, "fn other() {}\n").unwrap();
        editor.open(&other);
        assert_eq!(editor.lsp.clients.len(), client_count);
        assert!(
            !editor
                .lsp
                .outbox
                .iter()
                .any(|o| matches!(o, Outgoing::Start { .. }))
        );
    }

    #[test]
    fn two_auto_failures_share_one_line() {
        let (mut editor, ids, _) = setup(80, &["a", "b"]);
        editor.lsp.auto_enabled.extend(["a".into(), "b".into()]);
        fail(&mut editor, ids[0]);
        fail(&mut editor, ids[1]);
        assert_eq!(editor.lsp.failed, ["a", "b"]);
        assert!(
            editor
                .message
                .as_ref()
                .unwrap()
                .text
                .starts_with("a, b failed to start")
        );
        assert!(!editor.hit_enter);
    }

    #[test]
    fn warning_fits_narrow_screens() {
        for width in [80, 30, 20, 2, 1] {
            let (mut editor, ids, _) = setup(width, &["fake"]);
            editor.lsp.auto_enabled.insert("fake".into());
            handle_exit(&mut editor, ids[0], WHY, Some(Path::new("/tmp/宽/lsp.log")));
            let text = &editor.message.as_ref().unwrap().text;
            assert!(
                UnicodeWidthStr::width(text.as_str()) < width,
                "{width}: {text:?}"
            );
            assert!(!editor.hit_enter, "{width}: {text:?}");
            match width {
                80 => assert_eq!(text, "fake failed to start; see /tmp/宽/lsp.log"),
                30 | 20 => assert!(text.ends_with('…')),
                2 => assert_eq!(text, "…"),
                1 => assert!(text.is_empty()), // Nothing nonempty fits below one column.
                _ => unreachable!(),
            }
        }
        let (mut editor, ids, _) = setup(80, &["fake"]);
        editor.lsp.auto_enabled.insert("fake".into());
        handle_exit(
            &mut editor,
            ids[0],
            WHY,
            Some(Path::new(
                "/a/very/long/directory/name/that/does/not/fit/in/the/screen/lsp.log",
            )),
        );
        assert_eq!(
            editor.message.unwrap().text,
            "fake failed to start; see lsp.log"
        );
        let (mut editor, ids, _) = setup(80, &["fake"]);
        editor.lsp.auto_enabled.insert("fake".into());
        handle_exit(&mut editor, ids[0], "with error: spawn failed", None);
        assert_eq!(
            editor.message.unwrap().text,
            "fake failed to start; see lsp.log"
        );
    }

    #[test]
    fn explicit_failure_keeps_neovim_message() {
        let (mut editor, ids, _) = setup(80, &["fake"]);
        fail(&mut editor, ids[0]);
        assert_eq!(
            editor.message.unwrap().text,
            format!("Client fake quit {WHY}")
        );
        assert_eq!(editor.error_count, 1);
        assert!(editor.hit_enter);
    }

    #[test]
    fn running_crash_keeps_neovim_message() {
        let (mut editor, ids, _) = setup(80, &["fake"]);
        editor.lsp.auto_enabled.insert("fake".into());
        editor.lsp.client_mut(ids[0]).unwrap().state = ClientState::Running;
        fail(&mut editor, ids[0]);
        assert_eq!(
            editor.message.unwrap().text,
            format!("Client fake quit {WHY}")
        );
        assert_eq!(editor.error_count, 1);
        assert_eq!(editor.lsp.enabled.len(), 1);
    }

    #[test]
    fn waiting_message_is_not_dismissed() {
        let (mut editor, ids, _) = setup(80, &["fake"]);
        editor.lsp.auto_enabled.insert("fake".into());
        editor.error("line one\nline two");
        fail(&mut editor, ids[0]);
        assert!(editor.hit_enter);
        let message = editor.message.unwrap();
        assert_eq!(message.kind, MessageKind::Error);
        assert_eq!(
            message.text,
            "line one\nline two\nfake failed to start; see /tmp/x/lsp.log"
        );
        assert_eq!(editor.error_count, 1);
    }

    #[test]
    fn lsp_enable_makes_it_explicit() {
        let (mut editor, ids, _) = setup(80, &["fake"]);
        editor.lsp.auto_enabled.insert("fake".into());
        fail(&mut editor, ids[0]);
        crate::ex::execute(&mut editor, ":lsp enable fake");
        assert_eq!(editor.lsp.enabled.len(), 1);
        assert!(!editor.lsp.auto_enabled.contains("fake"));
        assert!(editor.lsp.failed.is_empty());
        let new = editor.lsp.clients.last().unwrap().id;
        assert_ne!(new, ids[0]);
        fail(&mut editor, new);
        assert!(
            editor
                .message
                .as_ref()
                .unwrap()
                .text
                .starts_with("Client fake quit")
        );
    }

    #[test]
    fn lsp_enable_without_a_name_makes_it_explicit() {
        let (mut editor, ids, _) = setup(80, &["fake"]);
        editor.lsp.auto_enabled.insert("fake".into());
        fail(&mut editor, ids[0]);
        crate::ex::execute(&mut editor, ":lsp enable");
        assert_eq!(editor.lsp.enabled.len(), 1);
        assert!(!editor.lsp.auto_enabled.contains("fake"));
        assert!(editor.lsp.failed.is_empty());
        assert_eq!(editor.lsp.clients.len(), 2);
    }

    #[test]
    fn stopping_during_initialize_is_silent() {
        let (mut editor, ids, _) = setup(80, &["fake"]);
        editor.lsp.auto_enabled.insert("fake".into());
        editor.lsp_stop(Some(ids[0]));
        fail(&mut editor, ids[0]);
        assert!(editor.message.is_none());
        assert!(editor.lsp.failed.is_empty());
        assert_eq!(editor.lsp.enabled.len(), 1);
    }

    #[test]
    fn restart_counts_as_explicit() {
        let (mut editor, ids, _) = setup(80, &["fake"]);
        editor.lsp.auto_enabled.insert("fake".into());
        crate::ex::execute(&mut editor, ":lsp restart fake");
        assert!(!editor.lsp.auto_enabled.contains("fake"));
        assert_eq!(
            editor.lsp.client(ids[0]).unwrap().state,
            ClientState::Stopping
        );
        fail(&mut editor, ids[0]);
        let new = editor.lsp.clients.last().unwrap().id;
        assert_ne!(new, ids[0]);
        fail(&mut editor, new);
        assert!(
            editor
                .message
                .as_ref()
                .unwrap()
                .text
                .starts_with("Client fake quit")
        );
    }

    #[test]
    fn duplicate_auto_failure_warns_only_once() {
        let (mut editor, ids, dir) = setup(80, &["fake"]);
        editor.lsp.auto_enabled.insert("fake".into());
        let other = dir.join("other.rs");
        std::fs::write(&other, "fn other() {}\n").unwrap();
        editor.open(&other);
        // A second root can leave another client of the same config already initializing.
        // Simulate that client so both exits are handled even after disabling the config.
        let mut second = editor.lsp.client(ids[0]).unwrap().clone();
        second.id = ClientId(ids[0].0 + 100);
        editor.lsp.clients.push(second.clone());
        fail(&mut editor, ids[0]);
        editor.message = None;
        fail(&mut editor, second.id);
        assert_eq!(editor.lsp.failed, ["fake"]);
        assert!(editor.message.is_none());
    }
}
