//! `CTRL-X CTRL-O` with the language servers: Neovim's `vim.lsp.omnifunc`
//! (`vim.lsp.completion`). The servers' items become completion matches; the selected one's
//! documentation shows in the info window (asked for with `completionItem/resolve`); an
//! accepted one's snippet is expanded and its `additionalTextEdits` applied.

use flux_core::Edit;
use flux_view::lsp::{ClientId, Encoding, Group, Pending, from_lsp_col, position_params};
use flux_view::{Cursor, Editor, Mode};
use serde_json::{Value, json};

use crate::completion::{LspData, Match};
use crate::engine::Engine;
use crate::lsp::snippet;
use crate::util::pos;

const KINDS: [&str; 25] = [
    "Text",
    "Method",
    "Function",
    "Constructor",
    "Field",
    "Variable",
    "Class",
    "Interface",
    "Module",
    "Property",
    "Unit",
    "Value",
    "Enum",
    "Keyword",
    "Snippet",
    "Color",
    "File",
    "Reference",
    "Folder",
    "EnumMember",
    "Constant",
    "Struct",
    "Event",
    "Operator",
    "TypeParameter",
];

/// The clients attached to the current buffer that complete.
fn completing_clients(editor: &Editor) -> Vec<ClientId> {
    let buffer = editor.window.buffer;
    editor
        .lsp
        .clients_for(buffer)
        .into_iter()
        .filter(|&c| {
            editor
                .lsp
                .client(c)
                .is_some_and(|c| c.capability(&["completionProvider"]).is_some())
        })
        .collect()
}

/// `CTRL-X CTRL-O` (Neovim's `_omnifunc`): ask the servers; their answer shows the matches.
/// Without a server that completes there's no 'omnifunc'.
pub(crate) fn omnifunc(engine: &mut Engine, editor: &mut Editor) {
    let _ = engine;
    if completing_clients(editor).is_empty() {
        editor.error("E764: Option 'omnifunc' is not set");
        editor.completion.show_error = true;
        return;
    }
    editor.lsp_request_all(
        "textDocument/completion",
        "completionProvider",
        |client, text, buffer, cursor| {
            let mut params = position_params(client, text, buffer, cursor);
            params["context"] = json!({ "triggerKind": 1 });
            params
        },
        Value::Null,
    );
}

/// Every server answered: show the matches, unless Insert mode ended or the cursor left the
/// line (Neovim's `trigger` callback).
pub(crate) fn show(engine: &mut Engine, editor: &mut Editor, group: Group) {
    if editor.mode != Mode::Insert
        || editor.window.buffer != group.buffer
        || editor.window.id != group.window
        || editor.cursor().line != group.cursor.line
    {
        return;
    }
    let cur = editor.cursor();
    let line = crate::util::line(editor, cur.line);
    let chars: Vec<char> = line.chars().collect();
    let cursor_col = cur.col.min(chars.len());
    let mut word_boundary = cursor_col;
    while word_boundary > 0 && flux_core::chars::is_keyword(chars[word_boundary - 1]) {
        word_boundary -= 1;
    }
    let mut matches = Vec::new();
    let mut server_start = None;
    for (client, answer) in group.results {
        let name = editor
            .lsp
            .client(client)
            .map_or("UNKNOWN".to_string(), |c| c.name.clone());
        let result = match answer {
            Ok(r) => r,
            Err(e) => {
                let code = e["code"]
                    .as_i64()
                    .map_or("NO_CODE".to_string(), |c| c.to_string());
                editor.warning(format!(
                    "{name}: {code} {}",
                    e["message"].as_str().unwrap_or("")
                ));
                continue;
            }
        };
        let items = items_of(&result);
        if items.is_empty() {
            continue;
        }
        let enc = super::encoding(editor, client);
        let (found, start) = convert_results(
            editor,
            &chars,
            cur.line,
            cursor_col,
            client,
            word_boundary,
            &result,
            items,
            enc,
        );
        server_start = start.or(server_start);
        matches.extend(found);
    }
    let start = server_start.unwrap_or(word_boundary);
    engine.compl.lsp_start = Some(pos(cur.line, start));
    engine.set_completion(editor, start, matches);
}

/// A completion result's items, with the list's `itemDefaults` filled in (Neovim's
/// `get_items` and `apply_defaults`).
fn items_of(result: &Value) -> Vec<Value> {
    let (items, defaults) = match result {
        Value::Array(items) => (items.clone(), None),
        Value::Object(o) => (
            o.get("items")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            o.get("itemDefaults").filter(|d| d.is_object()),
        ),
        _ => (Vec::new(), None),
    };
    let Some(d) = defaults else {
        return items;
    };
    items
        .into_iter()
        .map(|mut item| {
            for key in ["insertTextFormat", "insertTextMode", "data"] {
                if item.get(key).is_none_or(Value::is_null) && d.get(key).is_some() {
                    item[key] = d[key].clone();
                }
            }
            if let Some(range) = d.get("editRange") {
                let mut edit = item.get("textEdit").cloned().unwrap_or_else(|| json!({}));
                if edit.get("newText").is_none() {
                    edit["newText"] = item
                        .get("textEditText")
                        .or_else(|| item.get("insertText"))
                        .or_else(|| item.get("label"))
                        .cloned()
                        .unwrap_or(Value::Null);
                }
                if range.get("start").is_some() {
                    if edit.get("range").is_none() {
                        edit["range"] = range.clone();
                    }
                } else if range.get("insert").is_some() {
                    edit["insert"] = range["insert"].clone();
                    edit["replace"] = range["replace"].clone();
                }
                item["textEdit"] = edit;
            }
            item
        })
        .collect()
}

/// Where an item's text edit starts, when it's on line `lnum`.
fn item_start(item: &Value, lnum: usize) -> Option<usize> {
    let edit = item.get("textEdit")?;
    let start = if edit.get("range").is_some() {
        &edit["range"]["start"]
    } else {
        &edit["insert"]["start"]
    };
    (start["line"].as_u64()? as usize == lnum).then_some(start["character"].as_u64()? as usize)
}

/// Neovim's `_convert_results`: the matches of one server, and where they replace text from
/// (the leftmost start of their edits).
#[allow(clippy::too_many_arguments)]
fn convert_results(
    editor: &Editor,
    line: &[char],
    lnum: usize,
    cursor_col: usize,
    client: ClientId,
    client_start: usize,
    result: &Value,
    items: Vec<Value>,
    enc: Encoding,
) -> (Vec<Match>, Option<usize>) {
    let line_str: String = line.iter().collect();
    let server_start = items
        .iter()
        .filter_map(|i| item_start(i, lnum))
        .min()
        .map(|c| from_lsp_col(&line_str, c, enc));
    let from = server_start.unwrap_or(client_start).min(cursor_col);
    let prefix: String = line[from..cursor_col].iter().collect();
    let matches = to_complete_items(
        editor,
        items,
        &prefix,
        client,
        server_start,
        line,
        lnum,
        enc,
        result,
    );
    (matches, server_start)
}

/// Whether `value` starts with `prefix`, ignoring case as 'ignorecase' and 'smartcase' say.
fn match_value(editor: &Editor, value: &str, prefix: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    let o = &editor.options;
    if o.ignorecase && (!o.smartcase || !prefix.chars().any(char::is_uppercase)) {
        return value.to_lowercase().starts_with(&prefix.to_lowercase());
    }
    value.starts_with(prefix)
}

/// Neovim's `_lsp_to_complete_items`: the items that start with `prefix`, as matches, sorted
/// by `sortText`.
#[allow(clippy::too_many_arguments)]
fn to_complete_items(
    editor: &Editor,
    items: Vec<Value>,
    prefix: &str,
    client: ClientId,
    server_start: Option<usize>,
    line: &[char],
    lnum: usize,
    enc: Encoding,
    _result: &Value,
) -> Vec<Match> {
    let line_str: String = line.iter().collect();
    let filter = prefix.chars().any(|c| c.is_ascii_alphanumeric());
    let fits = |item: &Value| -> bool {
        if !filter {
            return true;
        }
        if let Some(f) = item["filterText"].as_str() {
            return match_value(editor, f, prefix);
        }
        if item.get("textEdit").is_some() && item["textEdit"].get("newText").is_none() {
            return true;
        }
        match_value(editor, item["label"].as_str().unwrap_or(""), prefix)
    };
    let mut out: Vec<(String, Match)> = Vec::new();
    for item in items {
        if !fits(&item) {
            continue;
        }
        let mut word = completion_word(editor, &item, prefix);
        if let Some(server_start) = server_start
            && let Some(c) = item_start(&item, lnum)
        {
            let item_start = from_lsp_col(&line_str, c, enc);
            if item_start > server_start {
                let missing: String = line[server_start..item_start.min(line.len())]
                    .iter()
                    .collect();
                word = missing + &word;
            }
        }
        let deprecated = item["deprecated"].as_bool() == Some(true)
            || item["tags"]
                .as_array()
                .is_some_and(|t| t.iter().any(|t| t.as_i64() == Some(1)));
        let label = item["label"].as_str().unwrap_or("").to_string();
        let abbr = format!(
            "{label}{}",
            item["labelDetails"]["detail"].as_str().unwrap_or("")
        );
        let menu = item["labelDetails"]["description"]
            .as_str()
            .or_else(|| item["detail"].as_str())
            .unwrap_or("")
            .to_string();
        let info = doc_of(&item);
        let m = Match {
            abbr: Some(abbr),
            kind: kind_of(&item),
            menu: (!menu.is_empty()).then_some(menu),
            info: (!info.is_empty()).then_some(info),
            icase: true,
            abbr_hl: deprecated.then(|| "DiagnosticDeprecated".to_string()),
            lsp: Some(LspData {
                client,
                item: item.clone(),
            }),
            ..Match::new(word)
        };
        let key = item["sortText"].as_str().unwrap_or(&label).to_string();
        out.push((key, m));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.into_iter().map(|(_, m)| m).collect()
}

/// An item's documentation text (Neovim's `get_doc`).
fn doc_of(item: &Value) -> String {
    match &item["documentation"] {
        Value::String(s) => s.clone(),
        Value::Object(o) => o
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    }
}

/// The kind's name (Neovim's `generate_kind`). A color shows as a square.
fn kind_of(item: &Value) -> Option<String> {
    let Some(k) = item["kind"]
        .as_u64()
        .filter(|k| (1..=25).contains(k))
        .map(|k| k as usize)
    else {
        return Some("Unknown".into());
    };
    if k != 16 {
        return Some(KINDS[k - 1].into());
    }
    let doc = doc_of(item);
    let hex: String = doc
        .trim_start_matches('#')
        .chars()
        .take_while(char::is_ascii_hexdigit)
        .collect();
    (doc.contains("rgb(") || matches!(hex.len(), 3 | 6)).then(|| "■".into())
}

/// The text inserted when an item is selected (Neovim's `get_completion_word`).
fn completion_word(editor: &Editor, item: &Value, prefix: &str) -> String {
    let label = item["label"].as_str().unwrap_or("");
    let insert_text = item["insertText"].as_str();
    let edit_text = item["textEdit"]["newText"].as_str();
    if item["insertTextFormat"].as_i64() == Some(2) {
        if edit_text.is_some() || insert_text.is_some_and(|t| !t.is_empty()) {
            let text = snippet::parse_text(insert_text.or(edit_text).unwrap_or(""));
            let word = if text.len() < label.len() {
                text.chars()
                    .take_while(|&c| flux_core::chars::is_keyword(c))
                    .collect()
            } else {
                match item["filterText"].as_str() {
                    Some(f)
                        if !label
                            .chars()
                            .next()
                            .is_some_and(flux_core::chars::is_keyword) =>
                    {
                        f.to_string()
                    }
                    _ => label.to_string(),
                }
            };
            if let Some(f) = item["filterText"].as_str()
                && !match_value(editor, &word, prefix)
            {
                return f.to_string();
            }
            return word;
        }
        return label.to_string();
    }
    if let Some(t) = edit_text {
        let t = t.replace("\r\n", "\n").replace('\r', "\n");
        return t.split('\n').next().unwrap_or("").to_string();
    }
    if let Some(t) = insert_text.filter(|t| !t.is_empty()) {
        return t.to_string();
    }
    label.to_string()
}

/// Whether `client` answers `completionItem/resolve`.
fn resolves(editor: &Editor, client: ClientId) -> bool {
    editor.lsp.client(client).is_some_and(|c| {
        c.capability(&["completionProvider", "resolveProvider"])
            .is_some_and(|v| v.as_bool() == Some(true))
    })
}

/// The selected match changed (Neovim's CompleteChanged handler): show its documentation,
/// asking the server for it when the item has none.
pub(crate) fn complete_changed(engine: &mut Engine, editor: &mut Editor, selected: Option<usize>) {
    engine.compl.resolving = None;
    if selected.is_none() {
        return;
    }
    let Some((index, lsp)) = engine.compl.selected_lsp() else {
        return;
    };
    let lsp = lsp.clone();
    let m = engine.compl.match_info(index).expect("selected");
    if m.info.is_some() {
        let kind = lsp.item["documentation"]["kind"].as_str();
        if kind.is_none_or(|k| k == "markdown") {
            editor.pum_info_markdown();
        }
        return;
    }
    if !resolves(editor, lsp.client) {
        if editor.completeopt("popup") && lsp.item["insertTextFormat"].as_i64() == Some(2) {
            let src = lsp.item["insertText"]
                .as_str()
                .or_else(|| lsp.item["textEdit"]["newText"].as_str())
                .unwrap_or("");
            let ft = editor.buf_opts().filetype.clone();
            let info = format!("```{ft}\n{}\n```", snippet::parse_text(src));
            editor.pum_set_info(&info);
            editor.pum_info_markdown();
        }
        return;
    }
    engine.compl.resolving = Some(index);
    editor.lsp.request(
        lsp.client,
        lsp.item.clone(),
        Pending {
            method: "completionItem/resolve".into(),
            buffer: Some(editor.window.buffer),
            data: json!({ "purpose": "info", "match": index }),
        },
    );
}

/// The answer to a `completionItem/resolve`.
pub(crate) fn resolved(
    engine: &mut Engine,
    editor: &mut Editor,
    client: ClientId,
    pending: &Pending,
    result: &Value,
) {
    match pending.data["purpose"].as_str() {
        Some("info") => resolved_info(engine, editor, pending, result),
        Some("done") => {
            let revision = editor.buffer(pending.buffer.unwrap_or(editor.window.buffer));
            let unchanged = revision.is_some_and(|b| {
                format!("{:?}", b.text.revision())
                    == pending.data["revision"].as_str().unwrap_or("")
            });
            if !unchanged || pending.buffer != Some(editor.window.buffer) {
                return;
            }
            let start = pending.data["start"].as_array().map(|a| {
                pos(
                    a[0].as_u64().unwrap_or(0) as usize,
                    a[1].as_u64().unwrap_or(0) as usize,
                )
            });
            let cursor = pending.data["cursor"].as_array().map(|a| {
                pos(
                    a[0].as_u64().unwrap_or(0) as usize,
                    a[1].as_u64().unwrap_or(0) as usize,
                )
            });
            let mut item = pending.data["item"].clone();
            let mut edits = Vec::new();
            if let Some(e) = result["additionalTextEdits"].as_array() {
                edits = e.clone();
            }
            if result.get("command").is_some_and(|c| !c.is_null()) {
                item["command"] = result["command"].clone();
            }
            finish(engine, editor, client, &item, start, cursor, &edits);
        }
        _ => {}
    }
}

/// The documentation of the selected item arrived (Neovim's `CompletionResolver:request`
/// callback).
fn resolved_info(engine: &mut Engine, editor: &mut Editor, pending: &Pending, result: &Value) {
    let index = pending.data["match"].as_u64().map(|i| i as usize);
    if editor.mode != Mode::Insert
        || editor.completion.pum.is_none()
        || index.is_none()
        || engine.compl.resolving != index
        || pending.buffer != Some(editor.window.buffer)
    {
        return;
    }
    if !result.is_object() || result.as_object().is_some_and(|o| o.is_empty()) {
        return;
    }
    let mut value = result["documentation"]["value"]
        .as_str()
        .map(str::to_string);
    let mut kind = result["documentation"]["kind"].as_str().map(str::to_string);
    let ft = editor.buf_opts().filetype.clone();
    if let Some(detail) = result["detail"].as_str().filter(|d| !d.is_empty()) {
        let block = format!("```{ft}\n{detail}\n```");
        match &value {
            None => {
                value = Some(block);
                kind = kind.or(Some("markdown".into()));
            }
            Some(v) if !v.contains(detail) => {
                value = Some(format!("{block}\n{v}"));
                kind = kind.or(Some("markdown".into()));
            }
            _ => {}
        }
    }
    if value.is_none() {
        if result["insertTextFormat"].as_i64() != Some(2) {
            return;
        }
        if let Some(t) = result["insertText"].as_str() {
            value = Some(format!("```{ft}\n{}\n```", snippet::parse_text(t)));
            kind = Some("markdown".into());
        }
    }
    let Some(value) = value else {
        return;
    };
    editor.pum_set_info(&value);
    if kind.as_deref() == Some("markdown") {
        editor.pum_info_markdown();
    }
}

/// A language server's match was accepted (Neovim's `on_complete_done`): its snippet is
/// expanded, its other edits made (asking the server for them first when it can resolve).
pub(crate) fn complete_done(
    engine: &mut Engine,
    editor: &mut Editor,
    lsp: &LspData,
    start: Option<Cursor>,
) {
    let cursor = editor.cursor();
    let item = &lsp.item;
    let edits = item["additionalTextEdits"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if edits.is_empty() && resolves(editor, lsp.client) {
        let revision = format!("{:?}", editor.text().revision());
        let start = start.map(|s| json!([s.line, s.col]));
        editor.lsp.request(
            lsp.client,
            item.clone(),
            Pending {
                method: "completionItem/resolve".into(),
                buffer: Some(editor.window.buffer),
                data: json!({
                    "purpose": "done",
                    "item": item,
                    "start": start,
                    "cursor": [cursor.line, cursor.col],
                    "revision": revision,
                }),
            },
        );
        return;
    }
    finish(
        engine,
        editor,
        lsp.client,
        item,
        start,
        Some(cursor),
        &edits,
    );
}

/// Remove the inserted word of a snippet item, make `edits`, and expand the snippet.
fn finish(
    engine: &mut Engine,
    editor: &mut Editor,
    client: ClientId,
    item: &Value,
    start: Option<Cursor>,
    cursor: Option<Cursor>,
    edits: &[Value],
) {
    let expand = item["insertTextFormat"].as_i64() == Some(2)
        && (item.get("textEdit").is_some_and(|e| !e.is_null())
            || item.get("insertText").is_some_and(|t| !t.is_null()));
    if expand && let (Some(start), Some(cursor)) = (start, cursor) {
        let text = editor.text();
        let from = text.pos_to_char(start.line, start.col);
        let to = text.pos_to_char(cursor.line, cursor.col);
        if to > from {
            engine.edit(editor, Edit::delete(from..to));
        }
        editor.window.cursor = start;
    }
    if !edits.is_empty() {
        let enc = super::encoding(editor, client);
        let buffer = editor.window.buffer;
        let mut edits = super::text_edits(editor, buffer, enc, edits);
        edits.sort_by_key(|e| std::cmp::Reverse(e.at));
        for e in edits {
            let cur = editor.cursor();
            let mut at = editor.text().pos_to_char(cur.line, cur.col);
            if e.at < at {
                let end = e.at + e.delete;
                let new = e.insert.chars().count();
                at = if end <= at {
                    at + new - e.delete
                } else {
                    e.at + new
                };
            }
            engine.edit(editor, e);
            let (line, col) = editor.text().char_to_pos(at.min(editor.text().len_chars()));
            editor.window.cursor = pos(line, col);
        }
    }
    if expand {
        let src = item["textEdit"]["newText"]
            .as_str()
            .or_else(|| item["insertText"].as_str())
            .unwrap_or("")
            .to_string();
        expand_snippet(engine, editor, &src);
    }
}

/// Insert snippet `src` at the cursor and put the cursor at its first tabstop (Neovim's
/// `vim.snippet.expand`).
fn expand_snippet(engine: &mut Engine, editor: &mut Editor, src: &str) {
    let cur = editor.cursor();
    let line = crate::util::line(editor, cur.line);
    let indent: String = line
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    let o = editor.buf_opts();
    let expand_tab = o.expandtab.then(|| o.sw());
    let path = editor.current_buffer().path.clone();
    let var = |name: &str| -> Option<String> {
        match name {
            "TM_CURRENT_LINE" => Some(line.clone()),
            "TM_LINE_INDEX" => Some(cur.line.to_string()),
            "TM_LINE_NUMBER" => Some((cur.line + 1).to_string()),
            "TM_FILENAME" => path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned()),
            "TM_FILENAME_BASE" => path
                .as_ref()
                .and_then(|p| p.file_stem())
                .map(|n| n.to_string_lossy().into_owned()),
            "TM_SELECTED_TEXT" => Some(String::new()),
            _ => None,
        }
    };
    let Some(expanded) = snippet::expand(src, &indent, expand_tab, &var) else {
        // Not a snippet: insert the text as it is.
        let at = editor.text().pos_to_char(cur.line, cur.col);
        engine.edit(editor, Edit::insert(at, src));
        let (line, col) = editor.text().char_to_pos(at + src.chars().count());
        editor.window.cursor = pos(line, col);
        return;
    };
    let at = editor.text().pos_to_char(cur.line, cur.col);
    engine.edit(editor, Edit::insert(at, expanded.text.clone()));
    // The first tabstop: the lowest number from 1, or the end.
    let dest = expanded
        .tabstops
        .iter()
        .filter(|t| t.0 > 0)
        .min_by_key(|t| (t.0, t.1))
        .or_else(|| expanded.tabstops.iter().find(|t| t.0 == 0))
        .copied();
    if let Some((_, start, _)) = dest {
        let (line, col) = editor.text().char_to_pos(at + start);
        editor.window.cursor = pos(line, col);
    }
}
