//! Ex commands (`:q`, `:42`). M0 knows only enough to quit and jump to a line; ranges and the
//! rest of the command table arrive in M4.

use flux_view::Editor;

/// An Ex command name, and how short Vim lets you abbreviate it (`:q`, `:qa`, `:quita`).
struct Command {
    name: &'static str,
    min_len: usize,
    run: fn(&mut Editor, bool, &str),
}

const COMMANDS: &[Command] = &[
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

fn quit(editor: &mut Editor, _bang: bool, args: &str) {
    if !args.is_empty() {
        editor.error(format!("E488: Trailing characters: {args}"));
        return;
    }
    editor.quit = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(cmd: &str) -> Editor {
        let mut editor = Editor::new(80, 24);
        execute(&mut editor, cmd);
        editor
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
        assert_eq!(e.message.unwrap().text, "E492: Not an editor command: qz");
        let e = run("q foo");
        assert!(!e.quit);
        assert_eq!(e.message.unwrap().text, "E488: Trailing characters: foo");
        assert!(!run("quitx").quit);
    }
}
