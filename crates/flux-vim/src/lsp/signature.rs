//! Insert-mode `CTRL-S`: Neovim's `vim.lsp.buf.signature_help()`, the signature of the call at
//! the cursor in a float, with the active parameter highlighted (LspSignatureActiveParameter).

use flux_view::Editor;
use flux_view::float::FloatHighlight;
use flux_view::lsp::{Group, position_params};
use serde_json::Value;

const METHOD: &str = "textDocument/signatureHelp";

/// Where the active parameter is in the lines: (line, start byte, line, end byte).
type ActiveRange = (usize, usize, usize, usize);

/// `CTRL-S`: ask the servers.
pub(crate) fn request(editor: &mut Editor) {
    if editor.floats.iter().any(|f| f.focus_id == METHOD) {
        return;
    }
    super::edits::request_all(
        editor,
        METHOD,
        "signatureHelpProvider",
        position_params,
        Value::Null,
    );
}

/// Neovim's `convert_signature_help_to_markdown_lines` for one signature: its label in a code
/// block of filetype `ft`, its documentation, and the active parameter's; with where the active
/// parameter is, as (line, start byte, line, end byte).
fn markdown(
    signature: &Value,
    ft: &str,
    triggers: &[String],
) -> (Vec<String>, Option<ActiveRange>) {
    let mut contents: Vec<String> = Vec::new();
    let label = signature["label"].as_str().unwrap_or("");
    let fenced = if ft.is_empty() {
        label.to_string()
    } else {
        format!("```{ft}\n{label}\n```")
    };
    contents.extend(
        fenced
            .split('\n')
            .filter(|l| !l.is_empty())
            .map(str::to_owned),
    );
    let doc = &signature["documentation"];
    if !doc.is_null() {
        let doc = match doc {
            Value::String(s) => serde_json::json!({ "kind": "plaintext", "value": s }),
            d => d.clone(),
        };
        if doc["value"].as_str() != Some("") {
            contents.push("---".into());
        }
        super::hover::markdown_lines(&doc, &mut contents);
    }
    let mut active_offset: Option<(usize, usize)> = None;
    let params = signature["parameters"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if !params.is_empty() {
        let active = signature["activeParameter"].as_i64();
        let Some(active) = active.filter(|&a| a >= 0 && (a as usize) < params.len()) else {
            return (contents, None);
        };
        let active = active as usize;
        let parameter = &params[active];
        match &parameter["label"] {
            Value::Array(range) => {
                let n = |i: usize| range.get(i).and_then(Value::as_u64).unwrap_or(0) as usize;
                active_offset = Some((n(0), n(1)));
            }
            Value::String(plabel) => {
                // Lua's `string.find` with plain text, from 1-based `init`.
                let find = |needle: &str, init: usize| -> Option<usize> {
                    let from = init.saturating_sub(1);
                    label.get(from..)?.find(needle).map(|i| from + i + 1)
                };
                let mut offset = 1;
                for t in triggers {
                    if let Some(o) = find(t, 1)
                        && (offset == 1 || o < offset)
                    {
                        offset = o;
                    }
                }
                for (p, param) in params.iter().enumerate() {
                    let pl = param["label"].as_str().unwrap_or("");
                    let Some(o) = find(pl, offset) else {
                        break;
                    };
                    offset = o;
                    if p == active {
                        active_offset = Some((offset - 1, offset + plabel.len() - 1));
                        break;
                    }
                    offset += pl.len() + 1;
                }
            }
            _ => {}
        }
        if !parameter["documentation"].is_null() {
            super::hover::markdown_lines(&parameter["documentation"], &mut contents);
        }
    }
    let hl = active_offset.and_then(|(mut a, mut b)| {
        if !ft.is_empty() {
            let first = contents.first().map_or(0, String::len);
            a += first;
            b += first;
        }
        let start = pos_from_offset(a, &contents)?;
        let end = pos_from_offset(b, &contents)?;
        Some((start.0, start.1, end.0, end.1))
    });
    (contents, hl)
}

/// Neovim's `get_pos_from_offset` (whose column comes out one past the offset).
fn pos_from_offset(offset: usize, contents: &[String]) -> Option<(usize, usize)> {
    let mut i = 0;
    for (l, line) in contents.iter().enumerate() {
        if offset >= i && offset < i + line.len() {
            return Some((l, offset - i + 1));
        }
        i += line.len() + 1;
    }
    None
}

/// The servers answered: show the active signature.
pub(crate) fn show(editor: &mut Editor, group: Group) {
    if !editor.lsp_group_valid(&group) {
        return;
    }
    let mut results = group.results.clone();
    results.sort_by_key(|(c, _)| c.0);
    let mut signatures: Vec<(flux_view::lsp::ClientId, Value)> = Vec::new();
    let mut active_signature = 1;
    for (client, answer) in &results {
        let name = editor
            .lsp
            .client(*client)
            .map_or(String::new(), |c| c.name.clone());
        match answer {
            Err(err) => {
                let code = match &err["code"] {
                    Value::Number(n) => n.to_string(),
                    v => v.to_string(),
                };
                editor.error(format!(
                    "{name}: {code}: {}",
                    err["message"].as_str().unwrap_or("")
                ));
            }
            Ok(result) => {
                let Some(list) = result["signatures"].as_array() else {
                    continue;
                };
                for (i, sig) in list.iter().enumerate() {
                    let mut sig = sig.clone();
                    if sig["activeParameter"].is_null() && !result["activeParameter"].is_null() {
                        sig["activeParameter"] = result["activeParameter"].clone();
                    }
                    if result["activeSignature"].as_u64().unwrap_or(0) as usize == i {
                        active_signature = signatures.len() + 1;
                    }
                    signatures.push((*client, sig));
                }
            }
        }
    }
    if signatures.is_empty() {
        editor.info("No signature help available");
        return;
    }
    let ft = editor
        .buffer(group.buffer)
        .map_or(String::new(), |b| b.opts.filetype.clone());
    let total = signatures.len();
    let (client, sig) = &signatures[active_signature - 1];
    let Some(c) = editor.lsp.client(*client) else {
        return;
    };
    let triggers: Vec<String> = c
        .capability(&["signatureHelpProvider", "triggerCharacters"])
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let name = c.name.clone();
    let (mut lines, mut hl) = markdown(sig, &ft, &triggers);
    // With more than one, a title says which (with no border, as a heading in the float).
    if total > 1 {
        lines.insert(
            0,
            format!("# Signature Help: {name} ({active_signature}/{total}) (<C-s> to cycle)"),
        );
        if let Some(h) = hl.as_mut() {
            h.0 += 1;
            h.2 += 1;
        }
    }
    // Normalized as `open_floating_preview` does Markdown.
    let win_width = editor
        .window_rects()
        .into_iter()
        .find(|(id, _)| *id == editor.window.id)
        .map_or(80, |(_, r)| r.width);
    let width = lines
        .iter()
        .map(|l| unicode_width::UnicodeWidthStr::width(l.as_str()))
        .max()
        .unwrap_or(0)
        .min(win_width);
    let lines = super::hover::normalize(lines, width);
    // The highlight is in bytes; floats take chars.
    let highlights = hl
        .filter(|h| h.0 == h.2)
        .and_then(|(line, a, _, b)| {
            let text = lines.get(line)?;
            let chars = |byte: usize| text[..byte.min(text.len())].chars().count();
            Some(FloatHighlight {
                line,
                start: chars(a),
                end: chars(b),
                group: "LspSignatureActiveParameter".into(),
            })
        })
        .into_iter()
        .collect();
    editor.open_float(lines, Some("markdown"), highlights, METHOD);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn active_parameter_offsets() {
        let sig = json!({
            "label": "fn compute(a: i32, b: i32) -> i32",
            "parameters": [{ "label": "a: i32" }, { "label": "b: i32" }],
            "activeParameter": 1,
        });
        let (lines, hl) = markdown(&sig, "rust", &["(".into(), ",".into()]);
        assert_eq!(
            lines,
            ["```rust", "fn compute(a: i32, b: i32) -> i32", "```"]
        );
        // "b: i32" (Neovim's offsets are one off twice over, which cancels out).
        assert_eq!(hl, Some((1, 19, 1, 25)));
    }
}
