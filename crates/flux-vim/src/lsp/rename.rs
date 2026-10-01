//! `grn`: Neovim's `vim.lsp.buf.rename()`. Each server that can rename is asked in turn: if it
//! can say what would be renamed (`textDocument/prepareRename`), that text is highlighted and
//! offered as the default new name; the name is asked for with `input()`, and the server's
//! `WorkspaceEdit` is applied.

use flux_view::lsp::{ClientId, ClientState, Pending, from_lsp, position_params};
use flux_view::{BufferId, Cursor, Editor};
use serde_json::{Value, json};

use crate::engine::Engine;
use crate::prompt::InputFor;

/// A rename waiting for its new name.
#[derive(Debug)]
pub(crate) struct Asked {
    buffer: BufferId,
    client: ClientId,
    /// The servers to ask after this one.
    rest: Vec<ClientId>,
    cword: String,
}

/// The servers attached to the current buffer that can rename.
fn clients(editor: &Editor) -> Vec<ClientId> {
    let buffer = editor.window.buffer;
    editor
        .lsp
        .clients
        .iter()
        .filter(|c| c.state == ClientState::Running && c.docs.contains_key(&buffer))
        .filter(|c| c.capability(&["renameProvider"]).is_some())
        .map(|c| c.id)
        .collect()
}

/// `expand('<cword>')`.
fn cword(editor: &Editor) -> String {
    let cur = editor.cursor();
    let line = editor.text().line_str(cur.line);
    match crate::search::find_ident(&line, cur.col) {
        Ok((s, e)) => line.chars().skip(s).take(e - s).collect(),
        Err(_) => String::new(),
    }
}

/// `grn`.
pub(crate) fn start(engine: &mut Engine, editor: &mut Editor) {
    let clients = clients(editor);
    if clients.is_empty() {
        editor.info("[LSP] Rename, no matching language servers with rename capability.");
        return;
    }
    let cword = cword(editor);
    editor.lsp_sync();
    let buffer = editor.window.buffer;
    try_client(engine, editor, buffer, clients, cword);
}

/// Ask the first of `clients`.
fn try_client(
    engine: &mut Engine,
    editor: &mut Editor,
    buffer: BufferId,
    mut clients: Vec<ClientId>,
    cword: String,
) {
    if clients.is_empty() {
        return;
    }
    let client = clients.remove(0);
    let Some(c) = editor.lsp.client(client) else {
        return;
    };
    let prepare = c
        .capability(&["renameProvider", "prepareProvider"])
        .is_some();
    let params = position_params(c, editor.text(), buffer, editor.cursor());
    let data = json!({
        "rest": clients.iter().map(|c| c.0).collect::<Vec<_>>(),
        "cword": cword,
    });
    if prepare {
        editor.lsp.request(
            client,
            params,
            Pending {
                method: "textDocument/prepareRename".into(),
                buffer: Some(buffer),
                data,
            },
        );
    } else {
        let asked = Asked {
            buffer,
            client,
            rest: clients,
            cword: cword.clone(),
        };
        engine.ask_input(editor, "New Name: ", &cword, InputFor::Rename(asked));
    }
}

fn rest_of(data: &Value) -> Vec<ClientId> {
    data["rest"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_u64().map(|n| ClientId(n as usize)))
        .collect()
}

/// The answer to `textDocument/prepareRename` (`Err` with the error).
pub(crate) fn prepared(
    engine: &mut Engine,
    editor: &mut Editor,
    client: ClientId,
    pending: &Pending,
    answer: Result<&Value, &Value>,
) {
    let Some(buffer) = pending.buffer else {
        return;
    };
    let rest = rest_of(&pending.data);
    let cword = pending.data["cword"].as_str().unwrap_or("").to_string();
    let result = match answer {
        Ok(r) if !r.is_null() => r,
        _ => {
            if !rest.is_empty() {
                try_client(engine, editor, buffer, rest, cword);
            } else {
                let msg = match answer {
                    Err(e) => format!(
                        "Error on prepareRename: {}",
                        e["message"].as_str().unwrap_or("")
                    ),
                    Ok(_) => "Nothing to rename".to_string(),
                };
                editor.info(msg);
            }
            return;
        }
    };
    let range = if result.get("start").is_some() {
        Some(result)
    } else {
        result.get("range")
    };
    let enc = super::encoding(editor, client);
    let Some(b) = editor.buffer(buffer) else {
        return;
    };
    let pos = |p: &Value| -> Cursor {
        from_lsp(
            &b.text,
            p["line"].as_u64().unwrap_or(0) as usize,
            p["character"].as_u64().unwrap_or(0) as usize,
            enc,
        )
    };
    let span = range.map(|r| (pos(&r["start"]), pos(&r["end"])));
    let default = if let Some(p) = result["placeholder"].as_str() {
        p.to_string()
    } else if let Some((s, e)) = span {
        // The range's text on its first line.
        let line = b.text.line_str(s.line);
        let end = if e.line == s.line {
            e.col
        } else {
            line.chars().count()
        };
        line.chars()
            .skip(s.col)
            .take(end.saturating_sub(s.col))
            .collect()
    } else {
        cword.clone()
    };
    // Neovim highlights the range (LspReferenceTarget) while the name is typed, but the
    // screen isn't redrawn under the prompt, so it doesn't show.
    let asked = Asked {
        buffer,
        client,
        rest,
        cword,
    };
    engine.ask_input(editor, "New Name: ", &default, InputFor::Rename(asked));
}

/// The new name was typed (`None` if the prompt was left).
pub(crate) fn answered(editor: &mut Editor, asked: Asked, name: Option<String>) {
    let Some(name) = name.filter(|n| !n.is_empty()) else {
        return;
    };
    editor.lsp_sync();
    let Some(c) = editor.lsp.client(asked.client) else {
        return;
    };
    let mut params = position_params(c, editor.text(), asked.buffer, editor.cursor());
    params["newName"] = json!(name);
    let data = json!({
        "rest": asked.rest.iter().map(|c| c.0).collect::<Vec<_>>(),
        "cword": asked.cword,
    });
    editor.lsp.request(
        asked.client,
        params,
        Pending {
            method: "textDocument/rename".into(),
            buffer: Some(asked.buffer),
            data,
        },
    );
}

/// The answer to `textDocument/rename`: apply it, then ask the next server.
pub(crate) fn renamed(
    engine: &mut Engine,
    editor: &mut Editor,
    client: ClientId,
    pending: &Pending,
    answer: Result<&Value, &Value>,
) {
    match answer {
        Err(e) if super::edits::handler_error(editor, client, e) => {}
        Ok(edit) if !edit.is_null() => {
            super::apply_workspace_edit(editor, client, edit);
        }
        _ => editor.info("Language server couldn't provide rename result"),
    }
    let rest = rest_of(&pending.data);
    if let Some(buffer) = pending.buffer
        && !rest.is_empty()
    {
        let cword = pending.data["cword"].as_str().unwrap_or("").to_string();
        try_client(engine, editor, buffer, rest, cword);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_keys;

    #[test]
    fn the_new_name_is_typed_after_the_prompt() {
        let mut editor = Editor::new(80, 24);
        let mut engine = Engine::new();
        let asked = Asked {
            buffer: editor.window.buffer,
            client: ClientId(9),
            rest: Vec::new(),
            cword: "total".into(),
        };
        engine.ask_input(&mut editor, "New Name: ", "total", InputFor::Rename(asked));
        assert_eq!(editor.mode, flux_view::Mode::CmdLine);
        for key in parse_keys("<C-w>sum<Left>x") {
            engine.handle_key(&mut editor, key);
        }
        assert_eq!(editor.cmdline, "suxm");
        for key in parse_keys("<CR>") {
            engine.handle_key(&mut editor, key);
        }
        assert_eq!(editor.mode, flux_view::Mode::Normal);
        assert_eq!(editor.cmdline_kind, ':');
        // What was typed stays on the command line.
        assert_eq!(editor.message.as_ref().unwrap().text, "New Name: suxm");
    }
}
