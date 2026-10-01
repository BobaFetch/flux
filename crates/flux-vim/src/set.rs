//! `:set`, `:setlocal`, `:setglobal` (Vim's `do_set`).

use flux_view::Editor;
use flux_view::options::{self, Kind, OptionDef, Options, Scope, Value};

use crate::ex::Args;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Which {
    /// `:set`: the local value and the global one.
    Both,
    Local,
    Global,
}

pub(crate) fn set(editor: &mut Editor, a: &Args) {
    do_set(editor, a.args, Which::Both);
}

pub(crate) fn setlocal(editor: &mut Editor, a: &Args) {
    do_set(editor, a.args, Which::Local);
}

pub(crate) fn setglobal(editor: &mut Editor, a: &Args) {
    do_set(editor, a.args, Which::Global);
}

fn get(editor: &Editor, def: &OptionDef, which: Which) -> Value {
    let name = def.name;
    match (def.scope, which) {
        (Scope::Global, _) | (_, Which::Global) => editor.options.get_global(name),
        (Scope::Buffer, _) => editor.buf_opts().get(name),
        (Scope::Window, _) => editor.window.opts.get(name),
    }
    .expect("known option")
}

fn put(editor: &mut Editor, def: &OptionDef, which: Which, v: Value) {
    let name = def.name;
    match def.scope {
        Scope::Global => editor.options.set_global(name, v),
        Scope::Buffer => {
            if which != Which::Global {
                editor.current_buffer_mut().opts.set(name, v.clone());
                // Like Vim's FileType event: the filetype's settings and parser follow.
                if name == "filetype" {
                    let id = editor.window.buffer;
                    editor.apply_filetype(id);
                }
            }
            if which != Which::Local {
                editor.options.buffer.set(name, v);
            }
        }
        Scope::Window => {
            if which != Which::Global {
                editor.window.opts.set(name, v.clone());
            }
            if which != Which::Local {
                editor.options.window.set(name, v);
            }
        }
    }
}

fn default_value(def: &OptionDef) -> Value {
    Options::default()
        .get_global(def.name)
        .expect("known option")
}

/// How `:set` shows one option: `  tabstop=8`, `  number`, `nonumber`.
fn show(editor: &Editor, def: &OptionDef, which: Which) -> String {
    match get(editor, def, which) {
        Value::Bool(true) => format!("  {}", def.name),
        Value::Bool(false) => format!("no{}", def.name),
        Value::Number(n) => format!("  {}={n}", def.name),
        Value::String(s) => format!("  {}={s}", def.name),
    }
}

fn do_set(editor: &mut Editor, args: &str, which: Which) {
    let args = args.trim();
    if args.is_empty() {
        show_options(editor, false, which);
        return;
    }
    let mut shown: Vec<String> = Vec::new();
    for arg in args.split_whitespace() {
        if arg == "all" {
            show_options(editor, true, which);
            return;
        }
        if arg == "all&" {
            for def in options::OPTIONS {
                put(editor, def, which, default_value(def));
            }
            continue;
        }
        match set_one(editor, arg, which) {
            Ok(Some(s)) => shown.push(s),
            Ok(None) => {}
            Err(e) => {
                editor.error(e);
                break;
            }
        }
    }
    editor.sync_window_sizes();
    if !shown.is_empty() {
        if shown.len() == 1 {
            editor.info(shown.remove(0));
        } else {
            editor.full_message(shown.join("\n"));
        }
    }
}

/// One `:set` argument. Returns what to show, if anything.
fn set_one(editor: &mut Editor, arg: &str, which: Which) -> Result<Option<String>, String> {
    let (prefix, rest) = if let Some(r) = arg.strip_prefix("no") {
        (Some(false), r)
    } else if let Some(r) = arg.strip_prefix("inv") {
        (Some(true), r)
    } else {
        (None, arg)
    };
    let name_len = rest
        .find(|c: char| !c.is_ascii_alphanumeric())
        .unwrap_or(rest.len());
    let (name, op) = rest.split_at(name_len);
    // "nonumber" names "number"; but an option may itself start with "no" or "inv".
    let (def, prefix, op) = match (options::find(name), prefix) {
        (Some(d), p) => (d, p, op),
        (None, Some(_)) => {
            let full_len = arg
                .find(|c: char| !c.is_ascii_alphanumeric())
                .unwrap_or(arg.len());
            match options::find(&arg[..full_len]) {
                Some(d) => (d, None, &arg[full_len..]),
                None => return Err(format!("E518: Unknown option: {arg}")),
            }
        }
        (None, None) => return Err(format!("E518: Unknown option: {arg}")),
    };
    let invalid = || format!("E474: Invalid argument: {arg}");
    if def.kind == Kind::Bool {
        let current = matches!(get(editor, def, which), Value::Bool(true));
        let value = match (prefix, op) {
            (Some(false), "") => false,
            (Some(true), "") | (None, "!") => !current,
            (None, "") => true,
            (None, "?") => return Ok(Some(show(editor, def, which))),
            (None, "&" | "&vim" | "&vi") => matches!(default_value(def), Value::Bool(true)),
            _ => return Err(invalid()),
        };
        put(editor, def, which, Value::Bool(value));
        return Ok(None);
    }
    if prefix.is_some() {
        return Err(invalid());
    }
    if def.kind == Kind::String {
        return set_string(editor, def, which, arg, op);
    }
    let current = match get(editor, def, which) {
        Value::Number(n) => n,
        Value::Bool(_) | Value::String(_) => 0,
    };
    let value = match op {
        "" | "?" => return Ok(Some(show(editor, def, which))),
        "&" | "&vim" | "&vi" => {
            put(editor, def, which, default_value(def));
            return Ok(None);
        }
        _ => {
            let (kind, num) = if let Some(v) = op.strip_prefix("+=") {
                ('+', v)
            } else if let Some(v) = op.strip_prefix("-=") {
                ('-', v)
            } else if let Some(v) = op.strip_prefix("^=") {
                ('^', v)
            } else if let Some(v) = op.strip_prefix(['=', ':']) {
                ('=', v)
            } else {
                return Err(invalid());
            };
            let n: i64 = num
                .parse()
                .map_err(|_| format!("E521: Number required after =: {arg}"))?;
            match kind {
                '+' => current + n,
                '-' => current - n,
                '^' => current * n,
                _ => n,
            }
        }
    };
    match def.name {
        "tabstop" if value <= 0 => return Err(format!("E487: Argument must be positive: {arg}")),
        "shiftwidth" | "report" if value < 0 => {
            return Err(format!("E487: Argument must be positive: {arg}"));
        }
        "numberwidth" if value < 1 => {
            return Err(format!("E487: Argument must be positive: {arg}"));
        }
        "numberwidth" if value > 20 => return Err(format!("E474: Invalid argument: {arg}")),
        _ => {}
    }
    put(editor, def, which, Value::Number(value));
    Ok(None)
}

/// A string option: `=`, `+=` (append), `^=` (prepend), `-=` (remove), `?`, `&`.
fn set_string(
    editor: &mut Editor,
    def: &OptionDef,
    which: Which,
    arg: &str,
    op: &str,
) -> Result<Option<String>, String> {
    let current = match get(editor, def, which) {
        Value::String(s) => s,
        _ => String::new(),
    };
    let value = match op {
        "" | "?" => return Ok(Some(show(editor, def, which))),
        "&" | "&vim" | "&vi" => match default_value(def) {
            Value::String(s) => s,
            _ => String::new(),
        },
        _ => {
            if let Some(v) = op.strip_prefix("+=") {
                format!("{current}{v}")
            } else if let Some(v) = op.strip_prefix("^=") {
                format!("{v}{current}")
            } else if let Some(v) = op.strip_prefix("-=") {
                current.replacen(v, "", 1)
            } else if let Some(v) = op.strip_prefix(['=', ':']) {
                v.to_string()
            } else {
                return Err(format!("E474: Invalid argument: {arg}"));
            }
        }
    };
    let valid = match def.name {
        "background" => matches!(value.as_str(), "dark" | "light"),
        "signcolumn" => matches!(value.as_str(), "auto" | "yes" | "no"),
        "completeopt" => value.split(',').filter(|v| !v.is_empty()).all(|v| {
            matches!(
                v,
                "menu"
                    | "menuone"
                    | "longest"
                    | "preview"
                    | "popup"
                    | "noinsert"
                    | "noselect"
                    | "fuzzy"
                    | "nosort"
                    | "preinsert"
                    | "nearest"
            )
        }),
        "filetype" => value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')),
        "matchpairs" => {
            value.is_empty()
                || flux_view::matchparen::pairs(&value).len() == value.split(',').count()
        }
        _ => true,
    };
    if !valid {
        return Err(format!("E474: Invalid argument: {arg}"));
    }
    put(editor, def, which, Value::String(value));
    Ok(None)
}

/// `:set` (the options that differ from their default) and `:set all`, laid out like Vim's
/// `showoptions`: short items in columns of 20, then long ones one per line.
fn show_options(editor: &mut Editor, all: bool, which: Which) {
    let title = match which {
        Which::Both => "--- Options ---",
        Which::Local => "--- Local option values ---",
        Which::Global => "--- Global option values ---",
    };
    let mut lines = vec![title.to_string()];
    let width = editor.screen_size().0.max(1);
    let items: Vec<(String, usize)> = options::OPTIONS
        .iter()
        .filter(|d| all || get(editor, d, which) != default_value(d))
        .map(|d| {
            let s = show(editor, d, which);
            let len = match d.kind {
                Kind::Bool => 1,
                Kind::Number | Kind::String => s.len() - 2 + 1,
            };
            (s, len)
        })
        .collect();
    let short: Vec<&String> = items.iter().filter(|i| i.1 <= 17).map(|i| &i.0).collect();
    let long: Vec<&String> = items.iter().filter(|i| i.1 > 17).map(|i| &i.0).collect();
    let cols = ((width + 3 - 3) / 20).max(1);
    let rows = short.len().div_ceil(cols);
    for row in 0..rows {
        let mut line = String::new();
        for (c, item) in short.iter().skip(row).step_by(rows.max(1)).enumerate() {
            let target = c * 20;
            if line.len() < target {
                line.push_str(&" ".repeat(target - line.len()));
            }
            line.push_str(item);
        }
        lines.push(line);
    }
    for item in long {
        lines.push(item.clone());
    }
    editor.full_message(lines.join("\n"));
}
