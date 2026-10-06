//! Replays the Neovim oracle cases through flux and compares the results.
//!
//! `oracle/cases.json` holds the inputs and `oracle/expected.json` what Neovim did with them
//! (regenerate with `cargo xtask oracle gen`). Each case is tagged with the milestone that
//! implements its keys. Cases above `MILESTONE` are deferred: they still run and must
//! differ from Neovim, then get retagged when they start matching.

use flux_view::Editor;
use flux_vim::{Engine, parse_keys};
use serde_json::Value;

const MILESTONE: u64 = 6;
/// Neovim's headless screen; the text area is 80x22 once the statusline and command line are
/// taken.
const SCREEN: (usize, usize) = (80, 24);

fn input_text(case: &Value) -> String {
    let Some(generator) = case.get("gen") else {
        return case["text"].as_str().unwrap().to_owned();
    };
    let long_every = generator["long_every"].as_u64();
    let long_width = generator["long_width"].as_u64().unwrap_or(0) as usize;
    (1..=generator["lines"].as_u64().unwrap())
        .map(|i| match long_every {
            Some(k) if i % k == 0 => format!("{i}{}", "x".repeat(long_width)),
            _ => i.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A case's keys: a string, or a list of chunks the oracle types with pauses in between.
fn keys_of(case: &Value) -> String {
    match &case["keys"] {
        Value::Array(chunks) => chunks.iter().filter_map(Value::as_str).collect(),
        keys => keys.as_str().unwrap().to_owned(),
    }
}

fn tempdir(id: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("flux-oracle-{}", std::process::id()))
        .join(id);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The layout in the oracle's shape: `["leaf", buffer name, height, width, [line, byte col],
/// top line, current]`, or `["row" | "col", [children]]`.
fn layout_json(editor: &Editor) -> Value {
    fn go(editor: &Editor, t: &flux_view::LayoutTree) -> Value {
        match t {
            flux_view::LayoutTree::Leaf(id) => {
                let w = editor.window_ref(*id);
                let b = editor.buffer(w.buffer).unwrap();
                let name = b
                    .path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let line = b.text.line_str(w.cursor.line.min(b.text.last_line()));
                let byte_col: usize = line.chars().take(w.cursor.col).map(char::len_utf8).sum();
                // Neovim's window width includes the number column.
                let width = editor
                    .window_rects()
                    .into_iter()
                    .find(|(wid, _)| wid == id)
                    .map_or(w.width, |(_, r)| r.width);
                serde_json::json!([
                    "leaf",
                    name,
                    w.height,
                    width,
                    [w.cursor.line, byte_col],
                    w.top,
                    *id == editor.window.id,
                    w.skipcol()
                ])
            }
            flux_view::LayoutTree::Row(c) => {
                serde_json::json!(["row", c.iter().map(|t| go(editor, t)).collect::<Vec<_>>()])
            }
            flux_view::LayoutTree::Col(c) => {
                serde_json::json!(["col", c.iter().map(|t| go(editor, t)).collect::<Vec<_>>()])
            }
        }
    }
    go(editor, &editor.layout.tree())
}

/// A compact form of a layout for failure messages.
fn show_layout(v: &Value) -> String {
    match v[0].as_str() {
        Some("leaf") => format!(
            "{}{}[{}x{} c{},{} t{}{}]",
            if v[6].as_bool() == Some(true) {
                "*"
            } else {
                ""
            },
            v[1].as_str().unwrap_or(""),
            v[2],
            v[3],
            v[4][0],
            v[4][1],
            v[5],
            match v[7].as_u64() {
                Some(0) | None => String::new(),
                Some(n) => format!(" s{n}"),
            }
        ),
        Some(kind) => format!(
            "{kind}({})",
            v[1].as_array()
                .map(|c| c.iter().map(show_layout).collect::<Vec<_>>().join(" "))
                .unwrap_or_default()
        ),
        None => v.to_string(),
    }
}

fn pair(v: &Value) -> (usize, usize) {
    (
        v[0].as_u64().unwrap() as usize,
        v[1].as_u64().unwrap() as usize,
    )
}

/// Run one case, returning a description of every way flux differs from Neovim.
fn run_case(case: &Value, expected: &Value) -> Vec<String> {
    let text = input_text(case);
    let mut editor = Editor::new(SCREEN.0, SCREEN.1);
    let _dir = if case.get("layout").is_some() || case.get("file").is_some() {
        // Like the oracle: a directory holding `main.txt` (or the file the case names, for its
        // filetype) and the case's other files, opened from that directory.
        let dir = tempdir(case["id"].as_str().unwrap());
        let main = case
            .get("file")
            .and_then(Value::as_str)
            .unwrap_or("main.txt");
        std::fs::write(dir.join(main), format!("{text}\n")).unwrap();
        if let Some(files) = case.get("files").and_then(Value::as_object) {
            for (name, contents) in files {
                std::fs::write(dir.join(name), format!("{}\n", contents.as_str().unwrap()))
                    .unwrap();
            }
        }
        editor.cwd = dir.clone();
        editor.open(std::path::Path::new(main));
        Some(dir)
    } else {
        // Loading from a file: the text is newline-terminated, like the oracle's temp file.
        editor.set_text(&format!("{text}\n"));
        None
    };
    // Neovim's cursor columns are byte offsets; a column inside a character means that
    // character.
    let (line, byte_col) = pair(&case["cur"]);
    let line_text = text.split('\n').nth(line).unwrap_or("");
    let col = line_text
        .char_indices()
        .filter(|&(i, _)| i <= byte_col)
        .count()
        .saturating_sub(1);
    editor.with_window(|win, m| win.set_cursor(line, col, m));

    let mut engine = Engine::new();
    for key in parse_keys(&keys_of(case))
        .into_iter()
        .chain(parse_keys("<Esc>"))
    {
        engine.handle_key(&mut editor, key);
    }

    let buffer = editor.current_buffer();
    let got_text: Vec<String> = (0..buffer.text.line_count())
        .map(|i| buffer.text.line_str(i).into_owned())
        .collect();
    let got_text = got_text.join("\n");
    let want_text = expected["text"]
        .as_str()
        .map_or(text.clone(), str::to_owned);

    let cursor = editor.window.cursor;
    let byte_col = buffer
        .text
        .line_str(cursor.line)
        .char_indices()
        .nth(cursor.col)
        .map_or(buffer.text.line(cursor.line).len_bytes(), |(i, _)| i);
    let got_cur = (cursor.line, byte_col);
    let want_cur = pair(&expected["cur"]);
    let got_top = editor.window.top;
    let want_top = expected["top"].as_u64().unwrap() as usize;

    let mut diffs = Vec::new();
    if let Some(want) = expected.get("layout") {
        let got = layout_json(&editor);
        if &got != want {
            diffs.push(format!(
                "layout {}, want {}",
                show_layout(&got),
                show_layout(want)
            ));
        }
    }
    // Registers the case asks about, as Vim's `getreg()` and `getregtype()` report them.
    if let Some(regs) = expected.get("regs").and_then(Value::as_object) {
        for (name, want) in regs {
            let c = name.chars().next().unwrap();
            let got = match editor.register(Some(c)) {
                Some(r) => match r.kind {
                    flux_view::RegisterKind::Char => (r.text.clone(), "v".to_string()),
                    flux_view::RegisterKind::Line => (format!("{}\n", r.text), "V".to_string()),
                    flux_view::RegisterKind::Block => (r.text.clone(), "\u{16}".to_string()),
                },
                None => (String::new(), "v".to_string()),
            };
            let want = (
                want[0].as_str().unwrap().to_string(),
                want[1].as_str().unwrap().to_string(),
            );
            if got != want {
                diffs.push(format!("register {name} {got:?}, want {want:?}"));
            }
        }
    }
    if got_text != want_text {
        diffs.push(format!("text {got_text:?}, want {want_text:?}"));
    }
    if got_cur != want_cur {
        diffs.push(format!("cursor {got_cur:?}, want {want_cur:?}"));
    }
    if got_top != want_top {
        diffs.push(format!("top line {got_top}, want {want_top}"));
    }
    diffs
}

fn cases_and_expected() -> (Vec<Value>, Vec<Value>) {
    let cases: Vec<Value> = serde_json::from_str(include_str!("oracle/cases.json")).unwrap();
    let expected: Vec<Value> = serde_json::from_str(include_str!("oracle/expected.json")).unwrap();
    assert_eq!(
        cases.len(),
        expected.len(),
        "expected.json is stale; run `cargo xtask oracle gen`"
    );
    for (case, want) in cases.iter().zip(&expected) {
        assert_eq!(
            case["id"], want["id"],
            "expected.json is stale; run `cargo xtask oracle gen`"
        );
    }
    (cases, expected)
}

#[test]
#[allow(clippy::print_stderr)]
fn matches_neovim() {
    let (cases, expected) = cases_and_expected();

    let mut ran = 0;
    let mut deferred = 0;
    let mut failures = Vec::new();
    for (case, want) in cases.iter().zip(&expected) {
        if case["m"].as_u64().unwrap() > MILESTONE {
            deferred += 1;
            continue;
        }
        ran += 1;
        let diffs = run_case(case, want);
        if !diffs.is_empty() {
            failures.push(format!(
                "{} (keys {}): {}",
                case["id"].as_str().unwrap(),
                keys_of(case),
                diffs.join("; ")
            ));
        }
    }
    assert_eq!(ran + deferred, cases.len());
    assert!(
        failures.is_empty(),
        "{} of {ran} oracle cases differ from Neovim:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
    eprintln!("oracle: {ran} cases up to M{MILESTONE} match Neovim");
}

#[test]
#[allow(clippy::print_stderr)]
fn deferred_cases_still_differ() {
    let (cases, expected) = cases_and_expected();

    let mut ran = 0;
    let mut in_scope = 0;
    let mut still_differ = Vec::new();
    let mut failures = Vec::new();
    for (case, want) in cases.iter().zip(&expected) {
        if case["m"].as_u64().unwrap() <= MILESTONE {
            in_scope += 1;
            continue;
        }
        ran += 1;
        let diffs = run_case(case, want);
        if diffs.is_empty() {
            failures.push(format!(
                "{} (keys {})",
                case["id"].as_str().unwrap(),
                keys_of(case)
            ));
        } else {
            still_differ.push(case["id"].as_str().unwrap());
        }
    }
    assert_eq!(in_scope + ran, cases.len());
    assert!(
        failures.is_empty(),
        "{} deferred oracle cases now match Neovim; set their \"m\" in cases.json to the milestone that implemented them:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
    if still_differ.is_empty() {
        eprintln!("oracle: 0 deferred cases (above M{MILESTONE})");
    } else {
        eprintln!(
            "oracle: {ran} deferred cases (above M{MILESTONE}) still differ: {}",
            still_differ.join(", ")
        );
    }
}

#[test]
fn case_ids_are_unique() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("oracle/cases.json")).unwrap();
    let mut ids = std::collections::HashSet::new();
    let mut duplicates = Vec::new();
    for case in &cases {
        let id = case["id"].as_str().unwrap();
        if !ids.insert(id) {
            duplicates.push(id);
        }
    }
    assert!(
        duplicates.is_empty(),
        "duplicate oracle case ids: {}",
        duplicates.join(", ")
    );
}
