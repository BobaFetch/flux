//! `K`: Neovim's `vim.lsp.buf.hover()`, Markdown in a float.

use flux_view::Editor;
use flux_view::lsp::{Group, from_lsp, position_params};
use serde_json::Value;

/// `K`: ask the servers for hover information at the cursor.
pub(crate) fn request(editor: &mut Editor) {
    if editor
        .floats
        .iter()
        .any(|f| f.focus_id == "textDocument/hover")
    {
        return;
    }
    editor.lsp_request_all(
        "textDocument/hover",
        "hoverProvider",
        position_params,
        Value::Null,
    );
}

/// Neovim's `split_lines`: lines of a hover text. With `no_blank`, blank lines go once the
/// description is over (after the first `@param`/`@return`), except before an indented code
/// block, and each such annotation gets a blank line before it.
fn split_lines(s: &str, no_blank: bool) -> Vec<String> {
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    let mut raw: Vec<&str> = s.split('\n').collect();
    while raw.first() == Some(&"") {
        raw.remove(0);
    }
    while raw.last() == Some(&"") {
        raw.pop();
    }
    let codeblock = |l: &str| l.starts_with("    ") || l.starts_with('\t');
    let blank = |l: &str| l.trim().is_empty();
    let annotation = |l: &str| {
        let l = l.strip_prefix(' ').unwrap_or(l);
        let Some(rest) = l.strip_prefix('@') else {
            return false;
        };
        let mut chars = rest.chars();
        match chars.next() {
            Some('p' | 'r') => true,
            Some(_) => matches!(chars.next(), Some('p' | 'r')),
            None => false,
        }
    };
    let mut lines: Vec<String> = Vec::new();
    let mut in_desc = true;
    for (i, line) in raw.iter().enumerate() {
        let start_annotation = annotation(line);
        in_desc = !start_annotation && in_desc;
        if start_annotation && no_blank && !lines.last().is_none_or(|l| blank(l)) {
            lines.push(String::new());
        }
        let is_blank = blank(line);
        let keep_blank = is_blank && codeblock(raw.get(i + 1).copied().unwrap_or(""));
        if in_desc || !no_blank || !is_blank || keep_blank {
            lines.push(line.to_string());
        }
    }
    lines
}

/// Neovim's `convert_input_to_markdown_lines`: hover contents (MarkupContent, MarkedString,
/// or a list of MarkedStrings) as Markdown lines.
pub(crate) fn markdown_lines(input: &Value, out: &mut Vec<String>) {
    match input {
        Value::String(s) => out.extend(split_lines(s, true)),
        Value::Object(o) if o.contains_key("kind") => {
            out.extend(split_lines(o["value"].as_str().unwrap_or(""), true))
        }
        Value::Object(o) if o.contains_key("language") => {
            out.push(format!("```{}", o["language"].as_str().unwrap_or("")));
            out.extend(split_lines(o["value"].as_str().unwrap_or(""), false));
            out.push("```".into());
        }
        Value::Array(items) => {
            for item in items {
                markdown_lines(item, out);
            }
        }
        _ => {}
    }
}

fn has_content(contents: &Value) -> bool {
    match contents {
        Value::String(s) => !s.is_empty(),
        Value::Object(o) => o
            .get("value")
            .and_then(Value::as_str)
            .is_some_and(|v| !v.is_empty()),
        Value::Array(items) => match items.first() {
            Some(Value::String(s)) => !s.is_empty(),
            Some(v) => v["value"].as_str().is_some_and(|v| !v.is_empty()),
            None => false,
        },
        _ => false,
    }
}

/// Neovim's `_normalize_markdown`: no carriage returns or leading and trailing blank lines,
/// runs of blank lines as one, and thematic breaks (`---`) as a line of `─` `width` long.
pub(crate) fn normalize(lines: Vec<String>, width: usize) -> Vec<String> {
    let joined = lines.join("\n").replace('\r', "");
    let mut lines: Vec<String> = joined.split('\n').map(str::to_owned).collect();
    while lines.first().is_some_and(String::is_empty) {
        lines.remove(0);
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    let blank = |l: &str| l.trim().is_empty();
    let mut collapsed: Vec<String> = Vec::new();
    for l in lines {
        if blank(&l) && collapsed.last().is_some_and(|p| blank(p)) {
            continue;
        }
        collapsed.push(l);
    }
    let separator = |l: &str| {
        let t = l.trim_end();
        t.len() >= 3
            && (t.chars().all(|c| c == '-')
                || t.chars().all(|c| c == '_')
                || t.chars().all(|c| c == '*'))
    };
    let divider = "─".repeat(width);
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < collapsed.len() {
        let l = &collapsed[i];
        if separator(l) {
            if i > 0 && blank(&collapsed[i - 1]) {
                out.pop();
            }
            out.push(divider.clone());
            if collapsed.get(i + 1).is_some_and(|n| blank(n)) {
                i += 1;
            }
        } else {
            out.push(l.clone());
        }
        i += 1;
    }
    out
}

/// The servers answered: show the hover float, or say there's nothing.
pub(crate) fn show(editor: &mut Editor, group: Group) {
    if !editor.lsp_group_valid(&group) {
        return;
    }
    let mut empty_response = false;
    let mut results: Vec<(String, flux_view::lsp::Encoding, Value)> = Vec::new();
    for (client, answer) in &group.results {
        let Ok(result) = answer else {
            continue;
        };
        if result.is_null() || result["contents"].is_null() {
            continue;
        }
        if has_content(&result["contents"]) {
            let Some(c) = editor.lsp.client(*client) else {
                continue;
            };
            results.push((c.name.clone(), c.encoding, result.clone()));
        } else {
            empty_response = true;
        }
    }
    if results.is_empty() {
        editor.info(if empty_response {
            "Empty hover response"
        } else {
            "No information available"
        });
        return;
    }
    let n = results.len();
    let mut lines: Vec<String> = Vec::new();
    let mut markdown = true;
    let mut target = None;
    for (name, enc, result) in &results {
        if n > 1 {
            lines.push(format!("# {name}"));
        }
        let contents = &result["contents"];
        if contents["kind"] == "plaintext" {
            let value = contents["value"].as_str().unwrap_or("");
            let plain: Vec<String> = value
                .split('\n')
                .map(str::to_owned)
                .collect::<Vec<_>>()
                .into_iter()
                .skip_while(String::is_empty)
                .collect();
            if n == 1 {
                markdown = false;
                lines = plain;
                while lines.last().is_some_and(String::is_empty) {
                    lines.pop();
                }
            } else {
                lines.push("```".into());
                lines.extend(plain);
                lines.push("```".into());
            }
        } else {
            markdown_lines(contents, &mut lines);
        }
        if let Some(range) = result.get("range")
            && let Some(b) = editor.buffer(group.buffer)
        {
            let pos = |p: &Value| {
                from_lsp(
                    &b.text,
                    p["line"].as_u64().unwrap_or(0) as usize,
                    p["character"].as_u64().unwrap_or(0) as usize,
                    *enc,
                )
            };
            target = Some((pos(&range["start"]), pos(&range["end"])));
        }
        lines.push("---".into());
    }
    lines.pop();
    if markdown {
        // The width the float would have before normalizing, for the dividers.
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
        lines = normalize(lines, width);
    } else {
        // Plain text loses its empty lines at both ends.
        while lines.first().is_some_and(String::is_empty) {
            lines.remove(0);
        }
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
    }
    editor.open_float(
        lines,
        markdown.then_some("markdown"),
        Vec::new(),
        "textDocument/hover",
    );
    if let Some(f) = editor.floats.last_mut() {
        f.target = target;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hover_lines() {
        assert_eq!(
            split_lines("desc\n\nmore\n@param x a\n\n@return y\n", true),
            ["desc", "", "more", "", "@param x a", "", "@return y"]
        );
        assert_eq!(
            normalize(
                vec![
                    "a".into(),
                    "".into(),
                    "".into(),
                    "---".into(),
                    "".into(),
                    "b".into()
                ],
                3
            ),
            ["a", "───", "b"]
        );
    }
}
