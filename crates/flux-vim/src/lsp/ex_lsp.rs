//! `:lsp enable|disable|restart|stop [name …]` (Neovim's `ex_cmd.lua`).

use flux_view::Editor;
use flux_view::lsp::{ClientId, ClientState};

/// `:lsp {subcommand} [name …]`.
pub(crate) fn lsp(editor: &mut Editor, args: &str) {
    let mut words = args.split_whitespace();
    let Some(sub) = words.next() else {
        editor.error("E471: Argument required");
        return;
    };
    let names: Vec<String> = words.map(str::to_owned).collect();
    let mut errors: Vec<String> = Vec::new();
    match sub {
        "enable" => enable(editor, names, &mut errors),
        "disable" => disable(editor, names, &mut errors),
        "restart" => {
            for id in clients_from_names(editor, &names, &mut errors) {
                restart(editor, id);
            }
        }
        "stop" => {
            for id in clients_from_names(editor, &names, &mut errors) {
                editor.lsp_stop(Some(id));
            }
        }
        other => errors.push(format!("Invalid subcommand '{other}'")),
    }
    if !errors.is_empty() {
        editor.error(errors.join("\n"));
    }
}

fn active(state: ClientState) -> bool {
    matches!(state, ClientState::Running | ClientState::Initializing)
}

/// The clients named, or those attached to the current buffer.
fn clients_from_names(
    editor: &Editor,
    names: &[String],
    errors: &mut Vec<String>,
) -> Vec<ClientId> {
    let buffer = editor.window.buffer;
    if names.is_empty() {
        let ids: Vec<ClientId> = editor
            .lsp
            .clients
            .iter()
            .filter(|c| active(c.state) && c.docs.contains_key(&buffer))
            .map(|c| c.id)
            .collect();
        if ids.is_empty() {
            errors.push("No clients attached to current buffer".into());
        }
        return ids;
    }
    let mut ids = Vec::new();
    for name in names {
        let named: Vec<ClientId> = editor
            .lsp
            .clients
            .iter()
            .filter(|c| active(c.state) && &c.name == name)
            .map(|c| c.id)
            .collect();
        if named.is_empty() {
            errors.push(format!("No active clients named '{name}'"));
        }
        ids.extend(named);
    }
    ids
}

/// `vim.lsp.enable(names)` for each known config, the others are errors.
fn checked_enable(editor: &mut Editor, names: &[String], enable: bool, errors: &mut Vec<String>) {
    for name in names {
        let Some(config) = editor.lsp.configs.iter().find(|c| &c.name == name).cloned() else {
            errors.push(format!("No client config named '{name}'"));
            continue;
        };
        if enable {
            if !editor.lsp.enabled.iter().any(|c| &c.name == name) {
                editor.lsp.enabled.push(config);
            }
            // Buffers already open start their servers (`:doautoall FileType`).
            let ids: Vec<_> = editor
                .buffers
                .iter()
                .filter(|b| b.loaded && b.listed)
                .map(|b| b.id)
                .collect();
            for id in ids {
                editor.lsp_attach(id);
            }
        } else {
            editor.lsp.enabled.retain(|c| &c.name != name);
            let ids: Vec<ClientId> = editor
                .lsp
                .clients
                .iter()
                .filter(|c| active(c.state) && &c.name == name)
                .map(|c| c.id)
                .collect();
            for id in ids {
                editor.lsp_stop(Some(id));
            }
        }
    }
}

/// `:lsp enable`: the configs named, or every config for the current buffer's filetype.
fn enable(editor: &mut Editor, mut names: Vec<String>, errors: &mut Vec<String>) {
    if names.is_empty() {
        let ft = editor.buf_opts().filetype.clone();
        names = editor
            .lsp
            .configs
            .iter()
            .filter(|c| c.filetypes.is_empty() || c.filetypes.contains(&ft))
            .map(|c| c.name.clone())
            .collect();
        if names.is_empty() {
            errors.push(if ft.is_empty() {
                "Current buffer has no filetype".into()
            } else {
                format!("No configs for filetype '{ft}'")
            });
            return;
        }
    }
    checked_enable(editor, &names, true, errors);
}

/// `:lsp disable`: the configs named, or those of the clients attached to the current buffer.
fn disable(editor: &mut Editor, mut names: Vec<String>, errors: &mut Vec<String>) {
    if names.is_empty() {
        let buffer = editor.window.buffer;
        for c in &editor.lsp.clients {
            if active(c.state)
                && c.docs.contains_key(&buffer)
                && editor.lsp.configs.iter().any(|k| k.name == c.name)
                && !names.contains(&c.name)
            {
                names.push(c.name.clone());
            }
        }
        if names.is_empty() {
            errors.push("No configs with clients attached to current buffer".into());
            return;
        }
    }
    checked_enable(editor, &names, false, errors);
}

/// `Client:_restart()`: stop the client; once it has exited, a new one starts for the same
/// buffers (see [`exited`]).
fn restart(editor: &mut Editor, id: ClientId) {
    let Some(c) = editor.lsp.client(id) else {
        return;
    };
    let buffers: Vec<_> = c.docs.keys().copied().collect();
    editor.lsp.restarts.push((id, buffers));
    editor.lsp_stop(Some(id));
}

/// Client `id` exited: start it again if it was restarting.
pub(crate) fn exited(editor: &mut Editor, id: ClientId) {
    let Some(i) = editor.lsp.restarts.iter().position(|(c, _)| *c == id) else {
        return;
    };
    let (_, buffers) = editor.lsp.restarts.remove(i);
    let Some(c) = editor.lsp.client(id) else {
        return;
    };
    let (config, root) = (c.config.clone(), c.root.clone());
    editor.lsp_restart(config, root, &buffers);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(editor: &Editor) -> String {
        editor
            .message
            .as_ref()
            .map_or(String::new(), |m| m.text.clone())
    }

    #[test]
    fn messages_without_servers() {
        let mut editor = Editor::new(80, 24);
        lsp(&mut editor, "");
        assert_eq!(message(&editor), "E471: Argument required");
        lsp(&mut editor, "foo");
        assert_eq!(message(&editor), "Invalid subcommand 'foo'");
        lsp(&mut editor, "enable");
        assert_eq!(message(&editor), "Current buffer has no filetype");
        lsp(&mut editor, "stop");
        assert_eq!(message(&editor), "No clients attached to current buffer");
        lsp(&mut editor, "disable");
        assert_eq!(
            message(&editor),
            "No configs with clients attached to current buffer"
        );
        lsp(&mut editor, "enable a b");
        assert_eq!(
            message(&editor),
            "No client config named 'a'\nNo client config named 'b'"
        );
        lsp(&mut editor, "restart x");
        assert_eq!(message(&editor), "No active clients named 'x'");
    }
}
