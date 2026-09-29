//! Ex commands. M1 has what's needed to edit and save one file; ranges and the rest of the
//! command table arrive in M4.

use std::path::PathBuf;

use flux_view::Editor;

/// An Ex command name, and how short Vim lets you abbreviate it (`:w`, `:wq`, `:quita`).
struct Command {
    name: &'static str,
    min_len: usize,
    run: fn(&mut Editor, bool, &str),
}

const COMMANDS: &[Command] = &[
    Command {
        name: "write",
        min_len: 1,
        run: write,
    },
    Command {
        name: "wq",
        min_len: 2,
        run: write_quit,
    },
    Command {
        name: "wall",
        min_len: 2,
        run: write_all,
    },
    Command {
        name: "wqall",
        min_len: 3,
        run: write_quit_all,
    },
    Command {
        name: "xit",
        min_len: 1,
        run: exit,
    },
    Command {
        name: "xall",
        min_len: 2,
        run: exit,
    },
    Command {
        name: "exit",
        min_len: 3,
        run: exit,
    },
    Command {
        name: "update",
        min_len: 2,
        run: update,
    },
    Command {
        name: "edit",
        min_len: 1,
        run: edit,
    },
    Command {
        name: "quit",
        min_len: 1,
        run: quit,
    },
    Command {
        name: "qall",
        min_len: 2,
        run: quit,
    },
    Command {
        name: "quitall",
        min_len: 5,
        run: quit,
    },
    Command {
        name: "checktime",
        min_len: 6,
        run: checktime,
    },
];

pub fn execute(editor: &mut Editor, line: &str) {
    let line = line.trim_start_matches([' ', ':']).trim_end();
    if line.is_empty() {
        return;
    }
    if let Ok(n) = line.parse::<usize>() {
        let target = n.saturating_sub(1);
        editor.with_window(|win, m| win.set_cursor_line(target, m));
        return;
    }

    let name_len = line
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(line.len());
    let (name, rest) = line.split_at(name_len);
    let (bang, args) = match rest.strip_prefix('!') {
        Some(args) => (true, args.trim()),
        None => (false, rest.trim()),
    };
    let command = COMMANDS
        .iter()
        .find(|c| name.len() >= c.min_len && c.name.starts_with(name));
    match command {
        Some(command) => (command.run)(editor, bang, args),
        None => editor.error(format!("E492: Not an editor command: {line}")),
    }
}

fn no_args(editor: &mut Editor, args: &str) -> bool {
    if args.is_empty() {
        return true;
    }
    editor.error(format!("E488: Trailing characters: {args}"));
    false
}

/// Write the buffer, reporting the result. True on success.
fn do_write(editor: &mut Editor, bang: bool, args: &str) -> bool {
    let path = (!args.is_empty()).then(|| PathBuf::from(args));
    match editor.current_buffer_mut().write(path.as_deref(), bang) {
        Ok(msg) => {
            editor.file_message(msg);
            true
        }
        Err(msg) => {
            editor.error(msg);
            false
        }
    }
}

fn write(editor: &mut Editor, bang: bool, args: &str) {
    do_write(editor, bang, args);
}

fn write_all(editor: &mut Editor, bang: bool, args: &str) {
    if no_args(editor, args) && editor.current_buffer().modified() {
        do_write(editor, bang, "");
    }
}

fn update(editor: &mut Editor, bang: bool, args: &str) {
    if editor.current_buffer().modified() || !args.is_empty() {
        do_write(editor, bang, args);
    }
}

fn write_quit(editor: &mut Editor, bang: bool, args: &str) {
    if do_write(editor, bang, args) {
        editor.quit = true;
    }
}

fn write_quit_all(editor: &mut Editor, bang: bool, args: &str) {
    if no_args(editor, args) && do_write(editor, bang, "") {
        editor.quit = true;
    }
}

/// `:x`: write only if there are changes, then quit.
fn exit(editor: &mut Editor, bang: bool, args: &str) {
    if (editor.current_buffer().modified() || !args.is_empty()) && !do_write(editor, bang, args) {
        return;
    }
    editor.quit = true;
}

fn quit(editor: &mut Editor, bang: bool, args: &str) {
    if !no_args(editor, args) {
        return;
    }
    if !bang && editor.current_buffer().modified() {
        let name = editor.current_buffer().name();
        editor.error(format!(
            "E37: No write since last change\nE162: No write since last change for buffer \"{name}\""
        ));
        return;
    }
    editor.quit = true;
}

/// `:e[!] [file]`. Without a file, re-read the current one; `!` discards changes.
fn edit(editor: &mut Editor, bang: bool, args: &str) {
    let modified = editor.current_buffer().modified();
    if args.is_empty() {
        if editor.current_buffer().path.is_none() {
            if bang {
                editor.set_text("");
            } else {
                editor.error("E32: No file name");
            }
            return;
        }
        if modified && !bang {
            editor.error("E37: No write since last change (add ! to override)");
            return;
        }
        let cursor = editor.cursor();
        if let Err(e) = editor
            .current_buffer_mut()
            .reload((cursor.line, cursor.col))
        {
            editor.error(format!("\"{}\" {e}", editor.current_buffer().name()));
        }
        editor.with_window(|win, m| {
            let line = win.cursor.line.min(m.text.last_line());
            win.set_cursor_line(line, m);
        });
        return;
    }
    if modified && !bang {
        editor.error("E37: No write since last change (add ! to override)");
        return;
    }
    editor.open(&PathBuf::from(args));
}

fn checktime(editor: &mut Editor, _bang: bool, args: &str) {
    if no_args(editor, args) {
        editor.check_time();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn run(cmd: &str) -> Editor {
        let mut editor = Editor::new(80, 24);
        execute(&mut editor, cmd);
        editor
    }

    fn temp_file(contents: &str) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "flux-ex-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("file.txt");
        fs::write(&path, contents).unwrap();
        path
    }

    fn modify(editor: &mut Editor) {
        let mut engine = crate::Engine::new();
        for key in crate::parse_keys("x") {
            engine.handle_key(editor, key);
        }
    }

    fn message(editor: &Editor) -> &str {
        &editor.message.as_ref().unwrap().text
    }

    #[test]
    fn quit_and_its_abbreviations() {
        for cmd in [
            "q", "qu", "quit", "q!", "qa", "qall", "qa!", "quita", "quitall!", " :q ",
        ] {
            assert!(run(cmd).quit, ":{cmd} should quit");
        }
    }

    #[test]
    fn errors() {
        let e = run("qz");
        assert!(!e.quit);
        assert_eq!(message(&e), "E492: Not an editor command: qz");
        let e = run("q foo");
        assert!(!e.quit);
        assert_eq!(message(&e), "E488: Trailing characters: foo");
        assert!(!run("quitx").quit);
        assert_eq!(message(&run("w")), "E32: No file name");
    }

    #[test]
    fn quit_refuses_unsaved_changes() {
        let path = temp_file("abc\n");
        let mut editor = Editor::new(80, 24);
        editor.open(&path);
        modify(&mut editor);
        execute(&mut editor, "q");
        assert!(!editor.quit);
        assert!(editor.hit_enter);
        assert!(message(&editor).starts_with("E37: No write since last change\nE162:"));
        execute(&mut editor, "e");
        assert!(message(&editor).starts_with("E37"));
        execute(&mut editor, "q!");
        assert!(editor.quit);
    }

    #[test]
    fn write_and_wq() {
        let path = temp_file("abc\n");
        let mut editor = Editor::new(80, 24);
        editor.open(&path);
        modify(&mut editor);
        execute(&mut editor, "w");
        assert!(
            message(&editor).ends_with("1L, 3B written"),
            "{}",
            message(&editor)
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "bc\n");
        execute(&mut editor, "q");
        assert!(editor.quit);

        let mut editor = Editor::new(80, 24);
        editor.open(&path);
        modify(&mut editor);
        execute(&mut editor, "x");
        assert!(editor.quit);
        assert_eq!(fs::read_to_string(&path).unwrap(), "c\n");
    }

    #[test]
    fn edit_bang_reloads_and_is_undoable() {
        let path = temp_file("abc\n");
        let mut editor = Editor::new(80, 24);
        editor.open(&path);
        modify(&mut editor);
        execute(&mut editor, "e!");
        assert_eq!(editor.text().line_str(0), "abc");
        assert!(!editor.current_buffer().modified());
    }

    #[test]
    fn autoread_on_checktime() {
        let path = temp_file("one\n");
        let mut editor = Editor::new(80, 24);
        editor.open(&path);
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&path, "two\nlines\n").unwrap();
        execute(&mut editor, "checktime");
        assert_eq!(editor.text().line_str(0), "two");

        fs::write(&path, "three\n").unwrap();
        modify(&mut editor);
        execute(&mut editor, "checktime");
        assert!(message(&editor).starts_with("W12"));
        assert_eq!(editor.text().line_str(0), "wo");
    }
}
