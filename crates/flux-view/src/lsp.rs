//! The editor's side of the Language Server Protocol: which servers run for which buffers,
//! keeping their copies of the documents in step, requests waiting for answers, and the
//! diagnostics they report. Pure state: the binary runs the processes (`flux_lsp`), carries
//! [`Outgoing`] messages out and brings their messages back in (`flux_vim::lsp`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use flux_core::{Revision, Text};
use flux_lsp::ServerConfig;
use serde_json::{Value, json};

use crate::{BufferId, Cursor, Editor};

/// A language server client (Neovim's client id).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClientId(pub usize);

/// How LSP positions count characters within a line ('positionEncoding').
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf16,
    Utf32,
}

/// What the binary is to do with the servers.
#[derive(Debug, Clone, PartialEq)]
pub enum Outgoing {
    Start {
        client: ClientId,
        cmd: Vec<String>,
        cwd: PathBuf,
    },
    Send {
        client: ClientId,
        message: Value,
    },
    /// End the process (after `exit`, or when it's stuck).
    Kill {
        client: ClientId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientState {
    /// Started, `initialize` not answered yet.
    Initializing,
    Running,
    /// `shutdown` sent.
    Stopping,
    Exited,
}

/// A document as a server has it.
#[derive(Debug, Clone)]
pub struct Doc {
    pub uri: String,
    pub version: i64,
    /// The text revision the server has.
    pub synced: Revision,
}

/// A request waiting for its answer: what it was, and for which buffer.
#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    pub method: String,
    pub buffer: Option<BufferId>,
    /// Whatever the code that asked needs to use the answer.
    pub data: Value,
}

#[derive(Debug, Clone)]
pub struct Client {
    pub id: ClientId,
    pub name: String,
    pub root: Option<PathBuf>,
    pub config: ServerConfig,
    pub state: ClientState,
    /// The server's capabilities (`initialize` result).
    pub capabilities: Value,
    pub encoding: Encoding,
    next_request: i64,
    pub pending: HashMap<i64, Pending>,
    pub docs: HashMap<BufferId, Doc>,
    /// Messages held until the server is initialized.
    queued: Vec<Value>,
}

impl Client {
    /// The server's capability at `path` (`["hoverProvider"]`), if it has it.
    pub fn capability(&self, path: &[&str]) -> Option<&Value> {
        let mut v = &self.capabilities;
        for key in path {
            v = v.get(key)?;
        }
        match v {
            Value::Null | Value::Bool(false) => None,
            v => Some(v),
        }
    }

    /// How the server wants document changes: 0 none, 1 full text, 2 incremental.
    fn sync_kind(&self) -> i64 {
        match self.capabilities.get("textDocumentSync") {
            Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
            Some(v) => v["change"].as_i64().unwrap_or(0),
            None => 0,
        }
    }

    fn wants_open_close(&self) -> bool {
        match self.capabilities.get("textDocumentSync") {
            Some(Value::Number(n)) => n.as_i64().unwrap_or(0) > 0,
            Some(v) => v["openClose"].as_bool().unwrap_or(false),
            None => false,
        }
    }

    fn wants_save(&self) -> bool {
        match self.capabilities.get("textDocumentSync") {
            Some(Value::Object(o)) => o
                .get("save")
                .is_some_and(|s| !matches!(s, Value::Bool(false))),
            _ => false,
        }
    }
}

/// A diagnostic as a server reported it (LSP positions, in the client's encoding).
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub client: ClientId,
    /// LSP `(line, character)`.
    pub start: (usize, usize),
    pub end: (usize, usize),
    /// 1 error, 2 warning, 3 information, 4 hint.
    pub severity: u8,
    pub message: String,
    pub source: Option<String>,
    pub code: Option<String>,
    /// Where it was in the buffer's text when it came, as (line, byte column) points, and the
    /// text's revision then: like Neovim's extmarks, diagnostics move with edits until the
    /// server sends new ones.
    pub placed: Option<(Revision, (usize, usize), (usize, usize))>,
}

/// Move point `p` (line, byte column) through `edit`, like an extmark: a point in deleted text
/// goes to where the deletion was; text inserted at the point pushes it along with
/// `right_gravity`.
fn follow_edit(p: (usize, usize), e: &flux_core::ByteEdit, right_gravity: bool) -> (usize, usize) {
    let start = (e.start.row, e.start.col);
    let old_end = (e.old_end.row, e.old_end.col);
    let new_end = (e.new_end.row, e.new_end.col);
    if p < start || (p == start && start == old_end && !right_gravity) {
        return p;
    }
    if p < old_end {
        return start;
    }
    if p.0 == old_end.0 {
        (new_end.0, new_end.1 + p.1 - old_end.1)
    } else {
        (p.0 + new_end.0 - old_end.0, p.1)
    }
}

/// Language servers and what they've told the editor.
#[derive(Debug, Default)]
pub struct LspState {
    /// The configs that start servers for matching buffers (`:lsp enable`).
    pub enabled: Vec<ServerConfig>,
    pub clients: Vec<Client>,
    next_client: usize,
    pub outbox: Vec<Outgoing>,
    /// Diagnostics by document URI, from each client.
    pub diagnostics: HashMap<String, Vec<Diagnostic>>,
    /// Diagnostics that arrived in Insert mode, shown when it ends ('update_in_insert' off).
    pub held_diagnostics: HashMap<String, Vec<Diagnostic>>,
}

/// The `file://` URI of `path`, percent-encoded like Neovim's `vim.uri_from_fname`.
pub fn path_to_uri(path: &Path) -> String {
    let mut out = String::from("file://");
    for b in path.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The path of a `file://` URI.
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let mut bytes = Vec::new();
    let b = rest.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = rest
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            bytes.push(v);
            i += 3;
            continue;
        }
        bytes.push(b[i]);
        i += 1;
    }
    Some(PathBuf::from(String::from_utf8_lossy(&bytes).into_owned()))
}

/// The LSP character offset of char column `col` of `line`.
pub fn to_lsp_col(line: &str, col: usize, enc: Encoding) -> usize {
    let prefix = line.chars().take(col);
    match enc {
        Encoding::Utf8 => prefix.map(char::len_utf8).sum(),
        Encoding::Utf16 => prefix.map(char::len_utf16).sum(),
        Encoding::Utf32 => prefix.count(),
    }
}

/// The char column of LSP character offset `character` in `line` (clamped to the line).
pub fn from_lsp_col(line: &str, character: usize, enc: Encoding) -> usize {
    let mut units = 0;
    for (i, c) in line.chars().enumerate() {
        if units >= character {
            return i;
        }
        units += match enc {
            Encoding::Utf8 => c.len_utf8(),
            Encoding::Utf16 => c.len_utf16(),
            Encoding::Utf32 => 1,
        };
    }
    line.chars().count()
}

/// An LSP position in `text` as a cursor (clamped to the text).
pub fn from_lsp(text: &Text, line: usize, character: usize, enc: Encoding) -> Cursor {
    let line = line.min(text.last_line());
    Cursor {
        line,
        col: from_lsp_col(&text.line_str(line), character, enc),
    }
}

/// A cursor as an LSP position.
pub fn to_lsp(text: &Text, at: Cursor, enc: Encoding) -> Value {
    let line = at.line.min(text.last_line());
    json!({ "line": line, "character": to_lsp_col(&text.line_str(line), at.col, enc) })
}

/// What flux tells servers it can do (a subset of what Neovim sends, for the features flux has).
fn client_capabilities() -> Value {
    json!({
        "general": { "positionEncodings": ["utf-8", "utf-16", "utf-32"] },
        "textDocument": {
            "synchronization": {
                "dynamicRegistration": false,
                "didSave": true,
                "willSave": false,
                "willSaveWaitUntil": false
            },
            "publishDiagnostics": {
                "relatedInformation": true,
                "tagSupport": { "valueSet": [1, 2] },
                "dataSupport": true
            },
            "hover": { "dynamicRegistration": false, "contentFormat": ["markdown", "plaintext"] },
            "definition": { "linkSupport": true },
            "declaration": { "linkSupport": true },
            "typeDefinition": { "linkSupport": true },
            "implementation": { "linkSupport": true },
            "references": { "dynamicRegistration": false },
            "documentSymbol": {
                "dynamicRegistration": false,
                "hierarchicalDocumentSymbolSupport": true,
                "symbolKind": { "valueSet": (1..=26).collect::<Vec<_>>() }
            }
        },
        "window": {
            "workDoneProgress": true,
            "showMessage": { "messageActionItem": { "additionalPropertiesSupport": true } },
            "showDocument": { "support": false }
        },
        "workspace": {
            "applyEdit": true,
            "configuration": true,
            "workspaceFolders": true,
            "workspaceEdit": { "resourceOperations": ["rename", "create", "delete"] }
        }
    })
}

impl LspState {
    pub fn client(&self, id: ClientId) -> Option<&Client> {
        self.clients.iter().find(|c| c.id == id)
    }

    pub fn client_mut(&mut self, id: ClientId) -> Option<&mut Client> {
        self.clients.iter_mut().find(|c| c.id == id)
    }

    /// Send `message` to `client` now, or once it's initialized.
    fn send(&mut self, client: ClientId, message: Value) {
        let Some(c) = self.client_mut(client) else {
            return;
        };
        match c.state {
            ClientState::Initializing => c.queued.push(message),
            ClientState::Running | ClientState::Stopping => {
                self.outbox.push(Outgoing::Send { client, message })
            }
            ClientState::Exited => {}
        }
    }

    pub fn notify(&mut self, client: ClientId, method: &str, params: Value) {
        self.send(
            client,
            json!({ "jsonrpc": "2.0", "method": method, "params": params }),
        );
    }

    /// Send a request; its answer comes back to whoever handles `pending.method`.
    pub fn request(&mut self, client: ClientId, params: Value, pending: Pending) -> Option<i64> {
        let c = self.client_mut(client)?;
        let id = c.next_request;
        c.next_request += 1;
        let message = json!({
            "jsonrpc": "2.0", "id": id, "method": pending.method, "params": params
        });
        c.pending.insert(id, pending);
        self.send(client, message);
        Some(id)
    }

    /// Answer a request from a server.
    pub fn respond(&mut self, client: ClientId, id: Value, result: Result<Value, (i64, String)>) {
        let message = match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, msg)) => {
                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": msg } })
            }
        };
        // Answers go out even while initializing (the server may ask during `initialize`).
        self.outbox.push(Outgoing::Send { client, message });
    }

    /// The clients attached to `buffer`.
    pub fn clients_for(&self, buffer: BufferId) -> Vec<ClientId> {
        self.clients
            .iter()
            .filter(|c| c.docs.contains_key(&buffer) && c.state != ClientState::Exited)
            .map(|c| c.id)
            .collect()
    }

    /// Whether `buffer` has signs to show (diagnostics, for now). `cwd` is where its name is
    /// relative to.
    pub fn has_signs(&self, buffer: &crate::Buffer, cwd: &Path) -> bool {
        !self.diagnostics.is_empty()
            && buffer.path.as_ref().is_some_and(|p| {
                let uri = path_to_uri(&crate::explorer::absolute(cwd, p));
                !self.diagnostics_for(&uri).is_empty()
            })
    }

    /// The diagnostics shown for document `uri`.
    pub fn diagnostics_for(&self, uri: &str) -> &[Diagnostic] {
        self.diagnostics.get(uri).map_or(&[], Vec::as_slice)
    }
}

impl Editor {
    /// Start (or reuse) the servers for buffer `id` and open its document in them, as
    /// Neovim does on FileType for enabled configs (`vim.lsp.enable`).
    pub fn lsp_attach(&mut self, id: BufferId) {
        let Some(buffer) = self.buffer(id) else {
            return;
        };
        if buffer.directory {
            return;
        }
        let Some(path) = buffer.path.clone() else {
            return;
        };
        let ft = buffer.opts.filetype.clone();
        let full = crate::explorer::absolute(&self.cwd, &path);
        let configs: Vec<ServerConfig> = self
            .lsp
            .enabled
            .iter()
            .filter(|c| c.filetypes.contains(&ft))
            .cloned()
            .collect();
        for config in configs {
            let root = flux_lsp::find_root(&config, &full);
            let existing = self
                .lsp
                .clients
                .iter()
                .find(|c| c.name == config.name && c.root == root && c.state != ClientState::Exited)
                .map(|c| c.id);
            let client = match existing {
                Some(c) => c,
                None => self.lsp_start(config, root.clone(), &full),
            };
            self.lsp_open(client, id, &full);
        }
    }

    /// Start a server for `config` in `root` (or, with no root, the file's directory).
    fn lsp_start(&mut self, config: ServerConfig, root: Option<PathBuf>, file: &Path) -> ClientId {
        let id = ClientId(self.lsp.next_client + 1);
        self.lsp.next_client += 1;
        let cwd = root
            .clone()
            .or_else(|| file.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| self.cwd.clone());
        self.lsp.outbox.push(Outgoing::Start {
            client: id,
            cmd: config.cmd.clone(),
            cwd,
        });
        let root_uri = root.as_deref().map(path_to_uri);
        let folders = root
            .as_deref()
            .map(|r| json!([{ "uri": path_to_uri(r), "name": r.to_string_lossy() }]));
        let params = json!({
            "processId": std::process::id(),
            "clientInfo": { "name": "flux", "version": env!("CARGO_PKG_VERSION") },
            "rootPath": root.as_deref().map(|r| r.to_string_lossy().into_owned()),
            "rootUri": root_uri,
            "workspaceFolders": folders,
            "initializationOptions": config.init_options,
            "capabilities": client_capabilities(),
            "trace": "off",
        });
        let client = Client {
            id,
            name: config.name.clone(),
            root,
            config,
            state: ClientState::Initializing,
            capabilities: Value::Null,
            encoding: Encoding::Utf16,
            next_request: 1,
            pending: HashMap::new(),
            docs: HashMap::new(),
            queued: Vec::new(),
        };
        self.lsp.clients.push(client);
        // `initialize` goes out first, ahead of the queue.
        let message =
            json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize", "params": params });
        self.lsp.outbox.push(Outgoing::Send {
            client: id,
            message,
        });
        if let Some(c) = self.lsp.client_mut(id) {
            c.pending.insert(
                0,
                Pending {
                    method: "initialize".into(),
                    buffer: None,
                    data: Value::Null,
                },
            );
        }
        id
    }

    /// Open buffer `id` (the file `path`) in `client`.
    fn lsp_open(&mut self, client: ClientId, id: BufferId, path: &Path) {
        let Some(buffer) = self.buffer(id) else {
            return;
        };
        let text = buffer.text.to_lsp_text();
        let revision = buffer.text.revision();
        let language_id = buffer.opts.filetype.clone();
        let uri = path_to_uri(path);
        let Some(c) = self.lsp.client_mut(client) else {
            return;
        };
        if c.docs.contains_key(&id) {
            return;
        }
        c.docs.insert(
            id,
            Doc {
                uri: uri.clone(),
                version: 0,
                synced: revision,
            },
        );
        let wants = c.state == ClientState::Initializing || c.wants_open_close();
        if wants {
            self.lsp.notify(
                client,
                "textDocument/didOpen",
                json!({ "textDocument": {
                    "uri": uri, "languageId": language_id, "version": 0, "text": text
                }}),
            );
        }
    }

    /// The server answered `initialize`: it's running.
    pub fn lsp_initialized(&mut self, client: ClientId, result: &Value) {
        let Some(c) = self.lsp.client_mut(client) else {
            return;
        };
        c.capabilities = result["capabilities"].clone();
        c.encoding = match c.capabilities["positionEncoding"].as_str() {
            Some("utf-8") => Encoding::Utf8,
            Some("utf-32") => Encoding::Utf32,
            _ => Encoding::Utf16,
        };
        c.state = ClientState::Running;
        let mut queued = std::mem::take(&mut c.queued);
        // A server that doesn't want documents opened doesn't get didOpen.
        if !c.wants_open_close() {
            queued.retain(|m| m["method"] != "textDocument/didOpen");
        }
        self.lsp.outbox.push(Outgoing::Send {
            client,
            message: json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
        });
        for message in queued {
            self.lsp.outbox.push(Outgoing::Send { client, message });
        }
        self.lsp_sync();
    }

    /// Bring the servers' copies of the documents up to date (`textDocument/didChange`):
    /// incremental changes from the text's edit log when the server takes them in UTF-8,
    /// otherwise the whole text.
    pub fn lsp_sync(&mut self) {
        let mut sends = Vec::new();
        for c in &mut self.lsp.clients {
            if c.state != ClientState::Running {
                continue;
            }
            let kind = c.sync_kind();
            for (id, doc) in &mut c.docs {
                let Some(buffer) = self.buffers.iter().find(|b| b.id == *id) else {
                    continue;
                };
                let text = &buffer.text;
                if text.revision() == doc.synced || kind == 0 {
                    continue;
                }
                let changes: Vec<Value> = match text.edits_since(doc.synced) {
                    Some(edits) if kind == 2 && c.encoding == Encoding::Utf8 => edits
                        .map(|e| {
                            let b = e.bytes;
                            json!({
                                "range": {
                                    "start": { "line": b.start.row, "character": b.start.col },
                                    "end": { "line": b.old_end.row, "character": b.old_end.col }
                                },
                                "rangeLength": b.old_end_byte - b.start_byte,
                                "text": e.text,
                            })
                        })
                        .collect(),
                    _ => vec![json!({ "text": text.to_lsp_text() })],
                };
                doc.version += 1;
                doc.synced = text.revision();
                sends.push((
                    c.id,
                    json!({
                        "jsonrpc": "2.0",
                        "method": "textDocument/didChange",
                        "params": {
                            "textDocument": { "uri": doc.uri, "version": doc.version },
                            "contentChanges": changes
                        }
                    }),
                ));
            }
        }
        for (client, message) in sends {
            self.lsp.outbox.push(Outgoing::Send { client, message });
        }
    }

    /// Buffer `id` was written: `textDocument/didSave`.
    pub fn lsp_did_save(&mut self, id: BufferId) {
        self.lsp_sync();
        let targets: Vec<(ClientId, String)> = self
            .lsp
            .clients
            .iter()
            .filter(|c| c.state == ClientState::Running && c.wants_save())
            .filter_map(|c| c.docs.get(&id).map(|d| (c.id, d.uri.clone())))
            .collect();
        for (client, uri) in targets {
            self.lsp.notify(
                client,
                "textDocument/didSave",
                json!({ "textDocument": { "uri": uri } }),
            );
        }
    }

    /// Buffer `id` is gone (`:bdelete`, `:bwipeout`): `textDocument/didClose`.
    pub fn lsp_detach(&mut self, id: BufferId) {
        let mut closes = Vec::new();
        for c in &mut self.lsp.clients {
            if let Some(doc) = c.docs.remove(&id)
                && c.wants_open_close()
            {
                closes.push((c.id, doc.uri));
            }
        }
        for (client, uri) in closes {
            self.lsp.notify(
                client,
                "textDocument/didClose",
                json!({ "textDocument": { "uri": uri } }),
            );
        }
    }

    /// Ask every server to shut down (`:qa`, `:lsp stop`).
    pub fn lsp_stop(&mut self, which: Option<ClientId>) {
        let ids: Vec<ClientId> = self
            .lsp
            .clients
            .iter()
            .filter(|c| which.is_none_or(|w| w == c.id))
            .filter(|c| matches!(c.state, ClientState::Running | ClientState::Initializing))
            .map(|c| c.id)
            .collect();
        for id in ids {
            self.lsp.request(
                id,
                Value::Null,
                Pending {
                    method: "shutdown".into(),
                    buffer: None,
                    data: Value::Null,
                },
            );
            if let Some(c) = self.lsp.client_mut(id) {
                c.state = ClientState::Stopping;
            }
        }
    }

    /// The server process ended.
    pub fn lsp_exited(&mut self, client: ClientId) {
        if let Some(c) = self.lsp.client_mut(client) {
            c.state = ClientState::Exited;
            c.pending.clear();
            c.docs.clear();
        }
        for list in self.lsp.diagnostics.values_mut() {
            list.retain(|d| d.client != client);
        }
    }

    /// `textDocument/publishDiagnostics` from `client`.
    pub fn lsp_publish_diagnostics(&mut self, client: ClientId, params: &Value) {
        let Some(uri) = params["uri"].as_str() else {
            return;
        };
        // Placed in the buffer's text as it is now, if the file is open.
        let enc = self
            .lsp
            .client(client)
            .map_or(Encoding::Utf16, |c| c.encoding);
        let buffer = uri_to_path(uri)
            .and_then(|p| self.find_buffer(&p))
            .and_then(|id| self.buffer(id))
            .filter(|b| b.loaded);
        let place = |line: usize, character: usize| -> Option<(Revision, (usize, usize))> {
            let text = &buffer?.text;
            let c = from_lsp(text, line, character, enc);
            let s = text.line_str(c.line);
            let byte = s.char_indices().nth(c.col).map_or(s.len(), |(i, _)| i);
            Some((text.revision(), (c.line, byte)))
        };
        let fresh: Vec<Diagnostic> = params["diagnostics"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|d| {
                let pos = |p: &Value| {
                    (
                        p["line"].as_u64().unwrap_or(0) as usize,
                        p["character"].as_u64().unwrap_or(0) as usize,
                    )
                };
                let (start, end) = (pos(&d["range"]["start"]), pos(&d["range"]["end"]));
                let placed = place(start.0, start.1)
                    .zip(place(end.0, end.1))
                    .map(|((rev, s), (_, e))| (rev, s, e));
                Diagnostic {
                    client,
                    start,
                    end,
                    placed,
                    severity: d["severity"].as_u64().unwrap_or(1).clamp(1, 4) as u8,
                    message: d["message"].as_str().unwrap_or("").to_string(),
                    source: d["source"].as_str().map(str::to_owned),
                    code: match &d["code"] {
                        Value::String(s) => Some(s.clone()),
                        Value::Number(n) => Some(n.to_string()),
                        _ => None,
                    },
                }
            })
            .collect();
        // Not shown while typing ('update_in_insert' is off): held until Insert mode ends.
        let target = if self.mode == crate::Mode::Insert {
            self.lsp
                .held_diagnostics
                .entry(uri.to_string())
                .or_insert_with(|| self.lsp.diagnostics.get(uri).cloned().unwrap_or_default())
        } else {
            self.lsp.diagnostics.entry(uri.to_string()).or_default()
        };
        target.retain(|d| d.client != client);
        target.extend(fresh);
    }

    /// Insert mode ended: show the diagnostics that arrived meanwhile.
    pub fn lsp_show_held_diagnostics(&mut self) {
        for (uri, list) in std::mem::take(&mut self.lsp.held_diagnostics) {
            self.lsp.diagnostics.insert(uri, list);
        }
    }

    /// The URI of buffer `id`'s file, if it has one.
    pub fn buffer_uri(&self, id: BufferId) -> Option<String> {
        let path = self.buffer(id)?.path.as_ref()?;
        Some(path_to_uri(&crate::explorer::absolute(&self.cwd, path)))
    }

    /// The diagnostics of buffer `id` as cursors in its text: `(start, end, severity, message)`
    /// for each, in the order they came.
    pub fn buffer_diagnostics(&self, id: BufferId) -> Vec<(Cursor, Cursor, &Diagnostic)> {
        let Some(uri) = self.buffer_uri(id) else {
            return Vec::new();
        };
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
        self.lsp
            .diagnostics_for(&uri)
            .iter()
            .map(|d| {
                // Follow the edits made since the diagnostic came.
                if let Some((rev, s, e)) = d.placed
                    && let Some(edits) = text.edits_since(rev)
                {
                    let (mut s, mut e) = (s, e);
                    for edit in edits {
                        s = follow_edit(s, &edit.bytes, true);
                        e = follow_edit(e, &edit.bytes, false);
                    }
                    return (cursor(s), cursor(e.max(s)), d);
                }
                let enc = self
                    .lsp
                    .client(d.client)
                    .map_or(Encoding::Utf16, |c| c.encoding);
                (
                    from_lsp(text, d.start.0, d.start.1, enc),
                    from_lsp(text, d.end.0, d.end.1, enc),
                    d,
                )
            })
            .collect()
    }
}

impl Editor {
    /// The buffer for file `path` (absolute): an open one, or a new one with the file read and
    /// its filetype detected, but not shown (Neovim's `bufload`, as for an edit to a file that
    /// isn't open).
    pub fn buffer_for_path(&mut self, path: &Path) -> Result<BufferId, String> {
        if let Some(id) = self.find_buffer(path) {
            if let Some(b) = self.buffer_mut(id) {
                b.listed = true;
                b.load().map_err(|e| e.to_string())?;
            }
            return Ok(id);
        }
        let name = path
            .strip_prefix(&self.cwd)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| path.to_path_buf());
        let mut buffer = crate::Buffer::open(BufferId(0), path).map_err(|e| e.to_string())?;
        buffer.path = Some(name);
        let id = self.add_buffer_hidden(buffer);
        self.detect_filetype(id);
        Ok(id)
    }

    /// Apply `edits` to buffer `id` as one undo step. Their positions are in the text as it is,
    /// and they don't overlap (LSP text edits); edits at the same position go in their order.
    /// Marks and cursors move with the text.
    pub fn apply_buffer_edits(&mut self, id: BufferId, edits: Vec<flux_core::Edit>) {
        let mut order: Vec<(usize, flux_core::Edit)> = edits.into_iter().enumerate().collect();
        order.sort_by(|(i, a), (j, b)| b.at.cmp(&a.at).then(j.cmp(i)));
        let cursor_before = if self.window.buffer == id {
            (self.window.cursor.line, self.window.cursor.col)
        } else {
            (0, 0)
        };
        let Some(buffer) = self.buffer(id) else {
            return;
        };
        let before = buffer.text.rope().clone();
        let no_lines_before = buffer.text.has_no_lines();
        let mut applied = Vec::new();
        let mut inverse = Vec::new();
        for (_, edit) in order {
            let Some(buffer) = self.buffer(id) else {
                return;
            };
            let shift = crate::LineShift::of(&buffer.text, &edit);
            for w in std::iter::once(&mut self.window).chain(self.windows.iter_mut()) {
                if w.buffer == id {
                    if let Some(p) = shift.adjust(w.cursor) {
                        w.cursor = p;
                    }
                    let top = Cursor {
                        line: w.top,
                        col: 0,
                    };
                    w.top = shift.adjust(top).map_or(w.top, |p| p.line);
                }
                w.jumps.adjust(id, &shift);
            }
            for (b, p) in self.global_marks.values_mut() {
                if *b == id
                    && let Some(q) = shift.adjust(*p)
                {
                    *p = q;
                }
            }
            let buffer = self.buffer_mut(id).expect("checked");
            buffer.marks.adjust(&shift);
            inverse.push(buffer.text.apply(&edit));
            applied.push(edit);
        }
        if applied.is_empty() {
            return;
        }
        let buffer = self.buffer_mut(id).expect("checked");
        inverse.reverse();
        let (b, a) = flux_core::Change::changed_lines(&before, buffer.text.rope(), &applied);
        buffer.history.record(flux_core::Change {
            edits: applied,
            inverse,
            cursor_before,
            before: b,
            after: a,
            no_lines_before,
            no_lines_after: buffer.text.has_no_lines(),
            saved_lines: None,
        });
        // Cursors stay on text.
        let text = &self
            .buffers
            .iter()
            .find(|b| b.id == id)
            .expect("checked")
            .text;
        for w in std::iter::once(&mut self.window).chain(self.windows.iter_mut()) {
            if w.buffer == id {
                w.cursor.line = w.cursor.line.min(text.last_line());
                w.cursor.col = w
                    .cursor
                    .col
                    .min(text.line_len(w.cursor.line).saturating_sub(1));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris() {
        let p = Path::new("/tmp/a b/é.rs");
        let uri = path_to_uri(p);
        assert_eq!(uri, "file:///tmp/a%20b/%C3%A9.rs");
        assert_eq!(uri_to_path(&uri).unwrap(), p);
    }

    #[test]
    fn columns_in_each_encoding() {
        let line = "aé😀b";
        assert_eq!(to_lsp_col(line, 3, Encoding::Utf8), 7);
        assert_eq!(to_lsp_col(line, 3, Encoding::Utf16), 4);
        assert_eq!(to_lsp_col(line, 3, Encoding::Utf32), 3);
        assert_eq!(from_lsp_col(line, 7, Encoding::Utf8), 3);
        assert_eq!(from_lsp_col(line, 4, Encoding::Utf16), 3);
        assert_eq!(from_lsp_col(line, 99, Encoding::Utf16), 4);
    }
}
