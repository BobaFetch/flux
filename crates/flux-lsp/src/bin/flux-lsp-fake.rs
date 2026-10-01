//! A scripted language server for tests and for comparing flux with Neovim: it answers from a
//! JSON script instead of understanding code, so both editors get exactly the same answers.
//!
//! Usage: `flux-lsp-fake SCRIPT.json`. The script:
//!
//! ```json
//! {
//!   "capabilities": { ... },          // the `initialize` result's capabilities
//!   "log": "/tmp/messages.jsonl",     // optional: every message received, one per line
//!   "on": {
//!     "textDocument/hover": { "result": { ... } },   // a request's answer
//!     "textDocument/didOpen": { "send": [ { "method": "...", "params": { ... } } ] }
//!   }
//! }
//! ```
//!
//! An entry may have `result` or `error` (for a request) and `send`: messages to send after it
//! (notifications, or requests with an `id`). A list of entries is used in turn, the last one
//! from then on. In everything sent (values and keys), `"$URI"` becomes the triggering message's
//! document URI (or the last one seen) and `"$ROOT"` the root URI.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};

use serde_json::{Value, json};

fn read_message(r: &mut impl BufRead) -> Option<Value> {
    let mut len = None;
    loop {
        let mut line = String::new();
        if r.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(v) = line.strip_prefix("Content-Length:") {
            len = v.trim().parse::<usize>().ok();
        }
    }
    let mut body = vec![0; len?];
    r.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

fn write_message(w: &mut impl Write, v: &Value) {
    let body = v.to_string();
    let _ = write!(w, "Content-Length: {}\r\n\r\n{body}", body.len());
    let _ = w.flush();
}

/// Replace the placeholders in `v`.
fn fill(v: &Value, uri: &str, root: &str) -> Value {
    match v {
        Value::String(s) if s == "$URI" => Value::String(uri.into()),
        Value::String(s) if s == "$ROOT" => Value::String(root.into()),
        Value::String(s) => Value::String(s.replace("$URI", uri).replace("$ROOT", root)),
        Value::Array(a) => Value::Array(a.iter().map(|x| fill(x, uri, root)).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, x)| {
                    let k = k.replace("$URI", uri).replace("$ROOT", root);
                    (k, fill(x, uri, root))
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

fn main() {
    let script: Value = std::env::args()
        .nth(1)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));
    let mut log = script["log"].as_str().and_then(|p| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .ok()
    });
    let mut uses: HashMap<String, usize> = HashMap::new();
    let mut root = String::new();
    let mut last_uri = String::new();
    let stdin = std::io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let mut out = std::io::stdout().lock();
    while let Some(msg) = read_message(&mut input) {
        if let Some(f) = log.as_mut() {
            let _ = writeln!(f, "{msg}");
        }
        let method = msg["method"].as_str().unwrap_or("").to_string();
        let id = msg.get("id").cloned();
        if method.is_empty() {
            continue; // A response to one of our requests.
        }
        if method == "initialize" {
            root = msg["params"]["rootUri"].as_str().unwrap_or("").to_string();
        }
        // A message without a document (`codeAction/resolve`) is about the last one seen.
        if let Some(u) = msg["params"]["textDocument"]["uri"].as_str() {
            last_uri = u.to_string();
        }
        let uri = last_uri.clone();
        let entry = match &script["on"][&method] {
            Value::Array(list) if !list.is_empty() => {
                let n = uses.entry(method.clone()).or_insert(0);
                let e = list[(*n).min(list.len() - 1)].clone();
                *n += 1;
                e
            }
            Value::Null => Value::Null,
            e => e.clone(),
        };
        if let Some(id) = id {
            let response = match method.as_str() {
                "initialize" => json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": {
                        "capabilities": script["capabilities"],
                        "serverInfo": { "name": "flux-lsp-fake" }
                    }
                }),
                "shutdown" => json!({ "jsonrpc": "2.0", "id": id, "result": null }),
                _ if entry.get("error").is_some() => {
                    json!({ "jsonrpc": "2.0", "id": id, "error": fill(&entry["error"], &uri, &root) })
                }
                _ => json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": fill(entry.get("result").unwrap_or(&Value::Null), &uri, &root)
                }),
            };
            write_message(&mut out, &response);
        }
        for m in entry["send"].as_array().into_iter().flatten() {
            let mut m = fill(m, &uri, &root);
            m["jsonrpc"] = json!("2.0");
            write_message(&mut out, &m);
        }
        if method == "exit" {
            break;
        }
    }
}
