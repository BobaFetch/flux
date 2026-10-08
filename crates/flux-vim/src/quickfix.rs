//! The quickfix and location list commands (`:copen`, `:cnext`, `:lopen`, …) and the tag stack
//! ones (`:pop`, `:tags`). The lists themselves are in [`flux_view::quickfix`].

use flux_view::Editor;
use flux_view::quickfix::JumpTo;

use crate::ex::Args;

/// The count given before the command (`:3cn`) or after it (`:cn 3`).
fn count(editor: &mut Editor, a: &Args) -> Result<Option<usize>, ()> {
    if a.args.is_empty() {
        return Ok(a.count);
    }
    match a.args.parse::<usize>() {
        Ok(n) => Ok(Some(n)),
        Err(_) => {
            editor.error(format!("E488: Trailing characters: {}", a.args));
            Err(())
        }
    }
}

fn report(editor: &mut Editor, result: Result<(), String>) {
    if let Err(e) = result
        && !e.is_empty()
    {
        editor.error(e);
    }
}

fn open(editor: &mut Editor, a: &Args, loc: bool) {
    let Ok(height) = count(editor, a) else { return };
    let result = editor.qf_open(loc, height, a.botright);
    report(editor, result);
}

pub(crate) fn copen(editor: &mut Editor, a: &Args) {
    open(editor, a, false);
}

pub(crate) fn lopen(editor: &mut Editor, a: &Args) {
    open(editor, a, true);
}

pub(crate) fn cclose(editor: &mut Editor, _a: &Args) {
    let result = editor.qf_close(false);
    report(editor, result);
}

pub(crate) fn lclose(editor: &mut Editor, _a: &Args) {
    let result = editor.qf_close(true);
    report(editor, result);
}

fn window(editor: &mut Editor, a: &Args, loc: bool) {
    let Ok(height) = count(editor, a) else { return };
    let result = editor.qf_window(loc, height);
    report(editor, result);
}

pub(crate) fn cwindow(editor: &mut Editor, a: &Args) {
    window(editor, a, false);
}

pub(crate) fn lwindow(editor: &mut Editor, a: &Args) {
    window(editor, a, true);
}

/// The commands that go to an entry, by what they're called without their `c` or `l`.
fn jump(editor: &mut Editor, a: &Args) {
    let loc = a.name.starts_with('l');
    let Ok(n) = count(editor, a) else { return };
    let name = &a.name[1..];
    let starts = |full: &str, min: usize| name.len() >= min && full.starts_with(name);
    let to = if name == "c" || name == "l" {
        JumpTo::Nr(n.unwrap_or(0))
    } else if starts("first", 3) || starts("rewind", 1) {
        JumpTo::Nr(n.unwrap_or(1))
    } else if starts("last", 2) {
        JumpTo::Nr(n.unwrap_or(usize::MAX))
    } else if starts("nfile", 2) {
        JumpTo::NextFile(n.unwrap_or(1))
    } else if starts("Nfile", 2) || starts("pfile", 2) {
        JumpTo::PrevFile(n.unwrap_or(1))
    } else if starts("next", 1) {
        JumpTo::Next(n.unwrap_or(1))
    } else {
        JumpTo::Prev(n.unwrap_or(1))
    };
    let result = editor.qf_jump(loc, to);
    report(editor, result);
}

pub(crate) fn cc(editor: &mut Editor, a: &Args) {
    jump(editor, a);
}

/// `:pop`, `CTRL-T`: back on the tag stack.
pub(crate) fn pop(editor: &mut Editor, a: &Args) {
    let Ok(n) = count(editor, a) else { return };
    let result = editor.tag_pop(n.unwrap_or(1));
    report(editor, result);
}

/// `:tags`: the tag stack.
pub(crate) fn tags(editor: &mut Editor, _a: &Args) {
    let lines = editor.tag_lines();
    let text: Vec<&str> = lines.iter().map(|(l, _)| l.as_str()).collect();
    editor.full_message(format!(":tags\n{}", text.join("\n")));
    editor.hit_enter = true;
    // The header in Title, the text of lines of the current file in Directory.
    editor
        .message_highlights
        .push((1, 0, text[0].chars().count(), "Title"));
    for (i, (_, range)) in lines.iter().enumerate() {
        if let Some((from, to)) = *range {
            editor
                .message_highlights
                .push((i + 1, from, to, "Directory"));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use flux_view::quickfix::Entry;

    use super::*;
    use crate::ex::execute;

    fn editor_with_list() -> Editor {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "flux-qf-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "one\n  two\nthree\n").unwrap();
        std::fs::write(dir.join("b.txt"), "x\ny\n").unwrap();
        let mut editor = Editor::new(80, 24);
        editor.cwd = dir.clone();
        editor.open(&PathBuf::from("a.txt"));
        let entry = |f: &str, lnum, col, text: &str| Entry {
            path: Some(dir.join(f)),
            lnum,
            col,
            text: text.into(),
            ..Default::default()
        };
        editor.qf_set_list(
            false,
            "T",
            vec![
                entry("a.txt", 2, 0, "first"),
                entry("a.txt", 3, 2, "second"),
                entry("b.txt", 2, 1, "third"),
            ],
        );
        editor
    }

    fn message(editor: &Editor) -> String {
        editor
            .message
            .as_ref()
            .map(|m| m.text.clone())
            .unwrap_or_default()
    }

    #[test]
    fn going_through_the_list() {
        let mut editor = editor_with_list();
        execute(&mut editor, "cc");
        assert_eq!(message(&editor), "(1 of 3): first");
        assert_eq!((editor.cursor().line, editor.cursor().col), (1, 2));
        execute(&mut editor, "cn");
        assert_eq!(message(&editor), "(2 of 3): second");
        assert_eq!((editor.cursor().line, editor.cursor().col), (2, 1));
        execute(&mut editor, "cnf");
        assert_eq!(editor.current_buffer().name(), "b.txt");
        execute(&mut editor, "cn");
        assert_eq!(message(&editor), "E553: No more items");
        execute(&mut editor, "cfirst");
        assert_eq!(editor.current_buffer().name(), "a.txt");
        execute(&mut editor, "clast");
        assert_eq!(message(&editor), "(3 of 3): third");
        execute(&mut editor, "lopen");
        assert_eq!(message(&editor), "E776: No location list");
    }

    #[test]
    fn the_quickfix_window() {
        let mut editor = editor_with_list();
        execute(&mut editor, "copen");
        assert_eq!(editor.current_buffer().name(), "[Quickfix List]");
        assert_eq!(editor.text().line_str(1), "a.txt|3 col 2| second");
        assert_eq!(editor.window.height, 10);
        editor.window.cursor.line = 2;
        editor.qf_enter().unwrap();
        assert_eq!(editor.current_buffer().name(), "b.txt");
        // The window shows the list, so no message.
        assert_eq!(message(&editor), "");
        execute(&mut editor, "cclose");
        assert_eq!(editor.window_ids().len(), 1);
    }
}
