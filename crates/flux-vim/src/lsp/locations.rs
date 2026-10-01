//! The LSP commands that go to places: `grr` (`vim.lsp.buf.references()`), `gri`
//! (`implementation()`), `grt` (`type_definition()`), `gO` (`document_symbol()`), and
//! `CTRL-]`, which goes to the definition through Neovim's LSP 'tagfunc'. One place is jumped
//! to; more go in the quickfix list (the location list for `gO`), shown in its window.

use std::path::{Path, PathBuf};

use flux_view::Editor;
use flux_view::lsp::{Encoding, Group, position_params, uri_to_path};
use flux_view::quickfix::Entry;
use serde_json::{Value, json};

use crate::parse::LspCmd;

/// Neovim's `lsp._unsupported_method`.
fn unsupported(method: &str) -> String {
    format!("vim.lsp: method \"{method}\" is not supported by any server activated for this buffer")
}

/// Whether any server is attached to the current buffer.
fn has_clients(editor: &Editor) -> bool {
    let buffer = editor.window.buffer;
    editor
        .lsp
        .clients
        .iter()
        .any(|c| c.docs.contains_key(&buffer))
}

/// Vim's `find_ident_under_cursor` with `FIND_IDENT`: the keyword under or after the cursor.
fn cword(editor: &Editor) -> Option<String> {
    let line = editor.text().line_str(editor.cursor().line);
    let chars: Vec<char> = line.chars().collect();
    let word = |i: usize| {
        chars
            .get(i)
            .is_some_and(|&c| flux_core::chars::class(c, false) >= 2)
    };
    let mut start = editor.cursor().col;
    while start < chars.len() && !word(start) {
        start += 1;
    }
    if start >= chars.len() {
        return None;
    }
    let class = flux_core::chars::class(chars[start], false);
    while start > 0 && flux_core::chars::class(chars[start - 1], false) == class {
        start -= 1;
    }
    let end = (start..chars.len())
        .find(|&i| flux_core::chars::class(chars[i], false) != class)
        .unwrap_or(chars.len());
    Some(chars[start..end].iter().collect())
}

/// Ask the servers for the places of `cmd`.
pub(crate) fn request(editor: &mut Editor, cmd: LspCmd) {
    let cursor = editor.cursor();
    let from = json!([editor.window.buffer.0, cursor.line, cursor.col]);
    match cmd {
        LspCmd::References => {
            let asked = editor.lsp_request_all(
                "textDocument/references",
                "referencesProvider",
                |c, t, b, cur| {
                    let mut p = position_params(c, t, b, cur);
                    p["context"] = json!({ "includeDeclaration": true });
                    p
                },
                Value::Null,
            );
            if asked == 0 && has_clients(editor) {
                editor.error(unsupported("textDocument/references"));
            }
        }
        LspCmd::DocumentSymbol => {
            let asked = editor.lsp_request_all(
                "textDocument/documentSymbol",
                "documentSymbolProvider",
                |c, _, b, _| json!({ "textDocument": { "uri": c.docs.get(&b).map(|d| d.uri.clone()) } }),
                Value::Null,
            );
            if asked == 0 && has_clients(editor) {
                editor.error(unsupported("textDocument/documentSymbol"));
            }
        }
        LspCmd::Implementation | LspCmd::TypeDefinition => {
            let (method, capability) = if cmd == LspCmd::Implementation {
                ("textDocument/implementation", "implementationProvider")
            } else {
                ("textDocument/typeDefinition", "typeDefinitionProvider")
            };
            let data = json!({ "from": from, "tagname": cword(editor).unwrap_or_default() });
            if editor.lsp_request_all(method, capability, position_params, data) == 0 {
                editor.warning(unsupported(method));
            }
        }
        LspCmd::Definition => {
            // `CTRL-]`: `:tag {ident}`, with the 'tagfunc' Neovim sets for servers that can
            // find definitions. Without one, there are only tags files, and flux has none.
            let Some(name) = cword(editor) else {
                editor.error("E349: No identifier under cursor");
                return;
            };
            let data = json!({ "from": from, "tagname": name });
            let asked = editor.lsp_request_all(
                "textDocument/definition",
                "definitionProvider",
                position_params,
                data,
            );
            if asked == 0 {
                no_tag(editor, &name);
            }
        }
        _ => {}
    }
}

/// No tag found: the errors of a `:tag` that found no tags file.
fn no_tag(editor: &mut Editor, name: &str) {
    editor.error(format!("E433: No tags file\nE426: Tag not found: {name}"));
}

/// Vim's `str_byteindex` (not strict): the byte column of LSP `character` in `line`.
fn byte_index(line: &str, character: usize, enc: Encoding) -> usize {
    let mut units = 0;
    for (i, c) in line.char_indices() {
        if units >= character {
            return i;
        }
        units += match enc {
            Encoding::Utf8 => c.len_utf8(),
            Encoding::Utf16 => c.len_utf16(),
            Encoding::Utf32 => 1,
        };
    }
    line.len()
}

/// Lines of `path`: from its buffer if it's loaded, else from the file.
fn file_lines(editor: &Editor, path: &Path) -> Vec<String> {
    if let Some(b) = editor.find_buffer(path).and_then(|id| editor.buffer(id))
        && b.loaded
    {
        return (0..b.text.line_count())
            .map(|l| b.text.line_str(l).into_owned())
            .collect();
    }
    std::fs::read_to_string(path)
        .map(|s| s.lines().map(str::to_owned).collect())
        .unwrap_or_default()
}

fn position(v: &Value) -> (usize, usize) {
    (
        v["line"].as_u64().unwrap_or(0) as usize,
        v["character"].as_u64().unwrap_or(0) as usize,
    )
}

/// Neovim's `locations_to_items`: `Location`s and `LocationLink`s as list entries, by file
/// (in URI order), then by position.
fn locations_to_items(editor: &Editor, locations: &[Value], enc: Encoding) -> Vec<Entry> {
    let mut grouped: Vec<(String, Vec<&Value>)> = Vec::new();
    for d in locations {
        let uri = d["uri"].as_str().or(d["targetUri"].as_str()).unwrap_or("");
        match grouped.iter_mut().find(|(u, _)| u == uri) {
            Some((_, rows)) => rows.push(d),
            None => grouped.push((uri.to_string(), vec![d])),
        }
    }
    grouped.sort_by(|a, b| a.0.cmp(&b.0));
    let mut items = Vec::new();
    for (uri, mut rows) in grouped {
        let range = |d: &Value| -> Value {
            if d.get("range").is_some() {
                d["range"].clone()
            } else {
                d["targetSelectionRange"].clone()
            }
        };
        rows.sort_by_key(|d| position(&range(d)["start"]));
        let Some(path) = uri_to_path(&uri) else {
            continue;
        };
        let lines = file_lines(editor, &path);
        for d in rows {
            let r = range(d);
            let (row, character) = position(&r["start"]);
            let (end_row, end_character) = position(&r["end"]);
            let line = lines.get(row).map_or("", String::as_str);
            let end_line = lines.get(end_row).map_or("", String::as_str);
            items.push(Entry {
                path: Some(path.clone()),
                lnum: row + 1,
                end_lnum: end_row + 1,
                col: byte_index(line, character, enc) + 1,
                end_col: byte_index(end_line, end_character, enc) + 1,
                text: line.to_string(),
                ..Default::default()
            });
        }
    }
    items
}

/// LSP `SymbolKind` names.
const SYMBOL_KINDS: [&str; 26] = [
    "File",
    "Module",
    "Namespace",
    "Package",
    "Class",
    "Method",
    "Property",
    "Field",
    "Constructor",
    "Enum",
    "Interface",
    "Function",
    "Variable",
    "Constant",
    "String",
    "Number",
    "Boolean",
    "Array",
    "Object",
    "Key",
    "Null",
    "EnumMember",
    "Struct",
    "Event",
    "Operator",
    "TypeParameter",
];

/// Neovim's `symbols_to_items`: `DocumentSymbol`s (with their children after them) or
/// `SymbolInformation`s as list entries, columns counted in buffer `lines`' text.
fn symbols_to_items(
    symbols: &[Value],
    path: &Path,
    lines: &[String],
    enc: Encoding,
    items: &mut Vec<Entry>,
) {
    let byte = |p: &Value| {
        let (row, character) = position(p);
        if character == 0 {
            return 0;
        }
        byte_index(lines.get(row).map_or("", String::as_str), character, enc)
    };
    for symbol in symbols {
        let (file, range) = if symbol.get("location").is_some() {
            let uri = symbol["location"]["uri"].as_str().unwrap_or("");
            (uri_to_path(uri), symbol["location"]["range"].clone())
        } else if symbol.get("selectionRange").is_some() {
            (Some(path.to_path_buf()), symbol["selectionRange"].clone())
        } else {
            (None, Value::Null)
        };
        if let Some(file) = file {
            let kind = symbol["kind"]
                .as_u64()
                .and_then(|k| SYMBOL_KINDS.get((k as usize).wrapping_sub(1)))
                .unwrap_or(&"Unknown");
            let container = match symbol["containerName"].as_str() {
                Some(c) => format!(" in {c}"),
                None => String::new(),
            };
            let deprecated = symbol["deprecated"].as_bool().is_some_and(|d| d)
                || symbol["tags"]
                    .as_array()
                    .is_some_and(|t| t.iter().any(|t| t == 1));
            items.push(Entry {
                path: Some(file),
                lnum: position(&range["start"]).0 + 1,
                col: byte(&range["start"]) + 1,
                end_lnum: position(&range["end"]).0 + 1,
                end_col: byte(&range["end"]) + 1,
                text: format!(
                    "[{kind}] {}{container}{}",
                    symbol["name"].as_str().unwrap_or(""),
                    if deprecated { " (deprecated)" } else { "" }
                ),
                ..Default::default()
            });
        }
        if let Some(children) = symbol["children"].as_array() {
            symbols_to_items(children, path, lines, enc, items);
        }
    }
}

/// The servers answered one of the requests above.
pub(crate) fn done(editor: &mut Editor, group: Group) {
    let answers: Vec<(Encoding, Value)> = group
        .results
        .iter()
        .filter_map(|(client, answer)| {
            let enc = editor.lsp.client(*client)?.encoding;
            Some((enc, answer.clone().unwrap_or(Value::Null)))
        })
        .collect();
    match group.method.as_str() {
        "textDocument/references" => {
            let mut items = Vec::new();
            for (enc, result) in &answers {
                let locations = result.as_array().cloned().unwrap_or_default();
                items.extend(locations_to_items(editor, &locations, *enc));
            }
            if items.is_empty() {
                editor.info("No references found");
            } else {
                editor.qf_set_list(false, "References", items);
                let _ = editor.qf_open(false, None, true);
            }
        }
        "textDocument/documentSymbol" => {
            let Some(path) = editor
                .buffer(group.buffer)
                .and_then(|b| b.path.clone())
                .map(|p| flux_view::explorer::absolute(&editor.cwd, &p))
            else {
                return;
            };
            let lines = file_lines(editor, &path);
            for (enc, result) in answers {
                let symbols = result.as_array().cloned().unwrap_or_default();
                if symbols.is_empty() {
                    editor.info("No document symbols found");
                    continue;
                }
                let mut items = Vec::new();
                symbols_to_items(&symbols, &path, &lines, enc, &mut items);
                let name = flux_view::quickfix::short_name(&editor.cwd, &path);
                editor.qf_set_list(true, &format!("Symbols in {}", name.display()), items);
                let _ = editor.qf_open(true, None, false);
            }
        }
        "textDocument/definition" => {
            let name = group.data["tagname"].as_str().unwrap_or("").to_string();
            let mut matches: Vec<(PathBuf, usize, usize)> = Vec::new();
            for (enc, result) in &answers {
                let list = match result {
                    Value::Array(a) => a.clone(),
                    Value::Null => Vec::new(),
                    v => vec![v.clone()],
                };
                for l in list {
                    let (uri, range) = if l.get("range").is_some() {
                        (l["uri"].as_str(), &l["range"])
                    } else {
                        (l["targetUri"].as_str(), &l["targetSelectionRange"])
                    };
                    let Some(path) = uri.and_then(uri_to_path) else {
                        continue;
                    };
                    let (row, character) = position(&range["start"]);
                    let lines = file_lines(editor, &path);
                    let byte = if character == 0 {
                        0
                    } else {
                        byte_index(lines.get(row).map_or("", String::as_str), character, *enc)
                    };
                    matches.push((path, row, byte));
                }
            }
            let Some((path, row, byte)) = matches.into_iter().next() else {
                no_tag(editor, &name);
                return;
            };
            if let Err(e) = editor.tag_jump(&name, from(&group), &path, row, byte) {
                editor.error(e);
            }
        }
        _ => {
            // `implementation`, `typeDefinition`: Neovim's `get_locations`.
            let mut items = Vec::new();
            for (enc, result) in &answers {
                let locations = match result {
                    Value::Array(a) => a.clone(),
                    Value::Null => Vec::new(),
                    v => vec![v.clone()],
                };
                items.extend(locations_to_items(editor, &locations, *enc));
            }
            match items.len() {
                0 => editor.info("No locations found"),
                1 => {
                    let item = &items[0];
                    if editor.window.id != group.window && editor.layout.contains(group.window) {
                        editor.goto_window(group.window);
                    }
                    let name = group.data["tagname"].as_str().unwrap_or("").to_string();
                    let path = item.path.clone().unwrap_or_default();
                    let result =
                        editor.tag_jump(&name, from(&group), &path, item.lnum - 1, item.col - 1);
                    if let Err(e) = result {
                        editor.error(e);
                    }
                }
                _ => {
                    editor.qf_set_list(false, "LSP locations", items);
                    let _ = editor.qf_open(false, None, true);
                }
            }
        }
    }
}

/// Where a request was made from, for the tag stack.
fn from(group: &Group) -> (flux_view::BufferId, flux_view::Cursor) {
    let f = &group.data["from"];
    (
        flux_view::BufferId(f[0].as_u64().unwrap_or(0) as usize),
        flux_view::Cursor {
            line: f[1].as_u64().unwrap_or(0) as usize,
            col: f[2].as_u64().unwrap_or(0) as usize,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_indexes_count_in_the_encoding() {
        assert_eq!(byte_index("aé😀b", 3, Encoding::Utf8), 3);
        assert_eq!(byte_index("aé😀b", 4, Encoding::Utf16), 7);
        assert_eq!(byte_index("aé😀b", 3, Encoding::Utf32), 7);
        assert_eq!(byte_index("ab", 9, Encoding::Utf8), 2);
    }

    #[test]
    fn symbols_with_children_come_after_them() {
        let symbols = serde_json::json!([{
            "name": "Point", "kind": 23,
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 3, "character": 1 } },
            "selectionRange": { "start": { "line": 0, "character": 11 }, "end": { "line": 0, "character": 16 } },
            "children": [{
                "name": "x", "kind": 8, "deprecated": true,
                "range": { "start": { "line": 1, "character": 4 }, "end": { "line": 1, "character": 10 } },
                "selectionRange": { "start": { "line": 1, "character": 4 }, "end": { "line": 1, "character": 5 } }
            }]
        }]);
        let lines = vec!["pub struct Point {".to_string(), "    x: i32,".to_string()];
        let mut items = Vec::new();
        symbols_to_items(
            symbols.as_array().unwrap(),
            Path::new("/a.rs"),
            &lines,
            Encoding::Utf16,
            &mut items,
        );
        let texts: Vec<&str> = items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(texts, ["[Struct] Point", "[Field] x (deprecated)"]);
        assert_eq!((items[0].lnum, items[0].col, items[0].end_col), (1, 12, 17));
    }
}
