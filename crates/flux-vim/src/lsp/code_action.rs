//! `gra`: Neovim's `vim.lsp.buf.code_action()`. The servers are asked for the actions at the
//! cursor (or for the Visual selection), with the diagnostics there; the actions are listed with
//! `inputlist()` (`vim.ui.select()`), and the one chosen is resolved if needed, its edit applied
//! and its command run.

use std::collections::HashMap;

use flux_view::lsp::{ClientId, ClientState, Group, Pending, to_lsp_col};
use flux_view::{Cursor, Editor, Mode, VisualKind};
use serde_json::{Value, json};

use crate::engine::Engine;
use crate::prompt::ListFor;

/// The actions offered, with the server each came from.
#[derive(Debug)]
pub(crate) struct Choices {
    actions: Vec<(ClientId, Value)>,
    buffer: flux_view::BufferId,
}

/// `gra`, in Normal or Visual mode.
pub(crate) fn request(editor: &mut Editor) {
    let buffer = editor.window.buffer;
    let supported = editor.lsp.clients.iter().any(|c| {
        c.state == ClientState::Running
            && c.docs.contains_key(&buffer)
            && c.capability(&["codeActionProvider"]).is_some()
    });
    if !supported {
        editor.warning(
            "vim.lsp: method \"textDocument/codeAction\" is not supported by any server activated \
             for this buffer",
        );
        return;
    }
    let cursor = editor.cursor();
    // Neovim's `range_from_selection`: the Visual area from where it started to the cursor.
    let range = (editor.mode == Mode::Visual).then(|| {
        let (mut a, mut b) = (editor.visual.anchor, cursor);
        if b.line < a.line || (b.line == a.line && b.col < a.col) {
            std::mem::swap(&mut a, &mut b);
        }
        let line = editor.visual.kind == VisualKind::Line;
        (a, b, line)
    });
    // The diagnostics each server gave that start on the cursor line (and, without a
    // selection, contain the cursor), as the server sent them.
    let mut diagnostics: HashMap<ClientId, Vec<Value>> = HashMap::new();
    for (s, e, d) in editor.buffer_diagnostics(buffer) {
        if s.line != cursor.line {
            continue;
        }
        let at = |p: Cursor| (p.line, p.col);
        let contains = if s == e {
            cursor == s
        } else {
            at(s) <= at(cursor) && at(cursor) < at(e)
        };
        if range.is_some() || contains {
            diagnostics.entry(d.client).or_default().push(d.lsp.clone());
        }
    }
    editor.lsp_request_all(
        "textDocument/codeAction",
        "codeActionProvider",
        |client, text, buffer, cursor| {
            let enc = client.encoding;
            let units = |at: Cursor| to_lsp_col(&text.line_str(at.line), at.col, enc);
            let range = match range {
                // `make_given_range_params`: the end is one past the last character.
                Some((a, b, linewise)) => {
                    let start = if linewise { 0 } else { units(a) };
                    let end_col = if linewise {
                        text.line_len(b.line).saturating_sub(1)
                    } else {
                        b.col
                    };
                    let end = if linewise && text.line_len(b.line) == 0 {
                        0
                    } else {
                        units(Cursor {
                            line: b.line,
                            col: end_col,
                        }) + 1
                    };
                    json!({
                        "start": { "line": a.line, "character": start },
                        "end": { "line": b.line, "character": end },
                    })
                }
                None => {
                    let p = json!({ "line": cursor.line, "character": units(cursor) });
                    json!({ "start": p, "end": p })
                }
            };
            json!({
                "textDocument": { "uri": client.docs.get(&buffer).map(|d| d.uri.clone()) },
                "range": range,
                "context": {
                    "triggerKind": 1,
                    "diagnostics": diagnostics.get(&client.id).cloned().unwrap_or_default(),
                },
            })
        },
        Value::Null,
    );
}

/// The servers answered: list their actions.
pub(crate) fn show(engine: &mut Engine, editor: &mut Editor, group: Group) {
    let mut results: Vec<(ClientId, Value)> = group
        .results
        .into_iter()
        .filter_map(|(c, r)| r.ok().map(|r| (c, r)))
        .collect();
    // Neovim goes through them by client.
    results.sort_by_key(|(c, _)| c.0);
    let actions: Vec<(ClientId, Value)> = results
        .into_iter()
        .flat_map(|(c, r)| {
            r.as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(move |a| (c, a))
        })
        .collect();
    if actions.is_empty() {
        editor.info("No code actions available");
        return;
    }
    let attached = editor.lsp.clients_for(group.buffer).len();
    let mut lines = vec!["Code actions:".to_string()];
    for (i, (client, action)) in actions.iter().enumerate() {
        let mut title = action["title"]
            .as_str()
            .unwrap_or("")
            .replace("\r\n", "\\r\\n")
            .replace('\n', "\\n");
        if !action["disabled"].is_null() {
            title.push_str(" (disabled)");
        }
        if attached > 1 {
            let name = editor
                .lsp
                .client(*client)
                .map_or(String::new(), |c| c.name.clone());
            title = format!("{title} [{name}]");
        }
        lines.push(format!("{}: {title}", i + 1));
    }
    engine.ask_number(
        editor,
        &lines,
        ListFor::CodeAction(Choices {
            actions,
            buffer: group.buffer,
        }),
    );
}

/// Action `n` (from 1) was chosen; 0 or a number past the list is none.
pub(crate) fn chosen(_engine: &mut Engine, editor: &mut Editor, choices: Choices, n: usize) {
    let Some((client, action)) = n
        .checked_sub(1)
        .and_then(|i| choices.actions.get(i))
        .cloned()
    else {
        return;
    };
    // A Command is run as it is.
    if action["title"].is_string() && action["command"].is_string() {
        apply(editor, client, &action);
        return;
    }
    if let Some(reason) = action["disabled"]["reason"].as_str() {
        editor.error(reason);
        return;
    }
    let complete = !action["edit"].is_null() && !action["command"].is_null();
    let can_resolve = editor.lsp.client(client).is_some_and(|c| {
        c.capability(&["codeActionProvider", "resolveProvider"])
            .is_some()
    });
    if !complete && can_resolve {
        editor.lsp.request(
            client,
            action.clone(),
            Pending {
                method: "codeAction/resolve".into(),
                buffer: Some(choices.buffer),
                data: json!({ "action": action }),
            },
        );
    } else {
        apply(editor, client, &action);
    }
}

/// The answer to `codeAction/resolve`.
pub(crate) fn resolved(
    editor: &mut Editor,
    client: ClientId,
    pending: &Pending,
    answer: Result<&Value, &Value>,
) {
    match answer {
        Ok(action) => apply(editor, client, action),
        Err(err) => {
            let action = &pending.data["action"];
            // The action as it was may still do something.
            if !action["edit"].is_null() || !action["command"].is_null() {
                apply(editor, client, action);
            } else {
                let code = match &err["code"] {
                    Value::Number(n) => n.to_string(),
                    other => other.to_string(),
                };
                editor.error(format!("{code}: {}", err["message"].as_str().unwrap_or("")));
            }
        }
    }
}

/// Neovim's `apply_action`: the edit, then the command.
fn apply(editor: &mut Editor, client: ClientId, action: &Value) {
    if !action["edit"].is_null() {
        super::apply_workspace_edit(editor, client, &action["edit"]);
    }
    match &action["command"] {
        Value::Object(_) => exec_cmd(editor, client, &action["command"]),
        Value::String(_) => exec_cmd(editor, client, action),
        _ => {}
    }
}

/// `Client:exec_cmd`: `workspace/executeCommand`, if the server says it has the command.
fn exec_cmd(editor: &mut Editor, client: ClientId, command: &Value) {
    let Some(c) = editor.lsp.client(client) else {
        return;
    };
    let name = command["command"].as_str().unwrap_or("");
    let known = c
        .capability(&["executeCommandProvider", "commands"])
        .and_then(Value::as_array)
        .is_some_and(|list| list.iter().any(|v| v == name));
    if !known {
        editor.warning(format!(
            "Language server `{}` does not support command `{name}`. This command may require a \
             client extension.",
            c.name
        ));
        return;
    }
    let mut params = json!({ "command": name });
    if let Some(args) = command.get("arguments") {
        params["arguments"] = args.clone();
    }
    editor.lsp.request(
        client,
        params,
        Pending {
            method: "workspace/executeCommand".into(),
            buffer: None,
            data: Value::Null,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_keys;

    #[test]
    fn choosing_a_number() {
        let mut editor = Editor::new(80, 24);
        let mut engine = Engine::new();
        let lines = vec![
            "Code actions:".to_string(),
            "1: One".into(),
            "2: Two".into(),
        ];
        let choices = Choices {
            actions: Vec::new(),
            buffer: editor.window.buffer,
        };
        engine.ask_number(&mut editor, &lines, ListFor::CodeAction(choices));
        assert!(editor.hit_enter);
        for key in parse_keys("12x<BS>") {
            engine.handle_key(&mut editor, key);
        }
        let prompt = editor.number_prompt.clone().unwrap();
        assert!(prompt.ends_with("(q or empty cancels): 1"), "{prompt}");
        // Choosing one that isn't there does nothing; the list goes.
        for key in parse_keys("<CR>") {
            engine.handle_key(&mut editor, key);
        }
        assert!(!editor.hit_enter);
        assert!(editor.number_prompt.is_none());
        assert!(editor.message.is_none());
        assert_eq!(editor.mode, Mode::Normal);
    }
}
