//! Language server processes for the editor: started and fed from the editor's LSP outbox,
//! their messages handed to the engine.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use flux_lsp::{Event, Server, ServerId};
use flux_view::Editor;
use flux_view::lsp::{ClientId, ClientState, Outgoing};
use flux_vim::Engine;
use tokio::sync::mpsc;

pub struct Servers {
    running: HashMap<ClientId, Server>,
    events: mpsc::UnboundedSender<Event>,
    /// Where servers' stderr goes, like Neovim's `lsp.log`.
    log: Option<PathBuf>,
}

/// `$XDG_STATE_HOME/flux/lsp.log`, or `~/.local/state/flux/lsp.log`.
fn log_path() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(state.join("flux/lsp.log"))
}

impl Servers {
    pub fn new() -> (Self, mpsc::UnboundedReceiver<Event>) {
        let (events, rx) = mpsc::unbounded_channel();
        (
            Self {
                running: HashMap::new(),
                events,
                log: log_path(),
            },
            rx,
        )
    }

    /// Send the documents' changes and whatever else the editor has for the servers.
    pub fn flush(&mut self, editor: &mut Editor) {
        editor.lsp_sync();
        for out in std::mem::take(&mut editor.lsp.outbox) {
            match out {
                Outgoing::Start { client, cmd, cwd } => {
                    match Server::start(
                        ServerId(client.0),
                        &cmd,
                        &cwd,
                        self.log.clone(),
                        self.events.clone(),
                    ) {
                        Ok(server) => {
                            self.running.insert(client, server);
                        }
                        Err(e) => flux_vim::lsp::handle_exit(editor, client, &e),
                    }
                }
                Outgoing::Send { client, message } => {
                    if let Some(s) = self.running.get(&client) {
                        s.send(message);
                    }
                }
                Outgoing::Kill { client } => {
                    if let Some(mut s) = self.running.remove(&client) {
                        // A moment for `exit` to arrive before the process is ended.
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_millis(200)).await;
                            s.kill();
                        });
                    }
                }
            }
        }
    }

    /// Hand a server's event to the engine.
    pub fn handle(&mut self, event: Event, editor: &mut Editor, engine: &mut Engine) {
        match event {
            Event::Message(id, msg) => {
                flux_vim::lsp::handle_message(engine, editor, ClientId(id.0), msg)
            }
            Event::Exited(id, why) => {
                self.running.remove(&ClientId(id.0));
                flux_vim::lsp::handle_exit(editor, ClientId(id.0), &why);
            }
        }
    }

    /// Shut the servers down when quitting: `shutdown` and `exit`, waiting a little for them.
    pub async fn shutdown(
        &mut self,
        editor: &mut Editor,
        engine: &mut Engine,
        rx: &mut mpsc::UnboundedReceiver<Event>,
    ) {
        editor.lsp_stop(None);
        self.flush(editor);
        let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
        while editor
            .lsp
            .clients
            .iter()
            .any(|c| c.state != ClientState::Exited)
        {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(event)) => {
                    self.handle(event, editor, engine);
                    self.flush(editor);
                }
                _ => break,
            }
        }
        for (_, mut s) in self.running.drain() {
            s.kill();
        }
    }
}
