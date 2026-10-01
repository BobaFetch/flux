//! Insert-mode keyword completion, compared with what Neovim does with the same keys.

use flux_view::Editor;
use flux_vim::{Engine, parse_keys};

fn run(text: &str, keys: &str) -> (Vec<String>, Editor) {
    let mut editor = Editor::new(80, 24);
    editor.current_buffer_mut().text = flux_core::Text::new(text);
    let mut engine = Engine::new();
    for key in parse_keys(keys) {
        engine.handle_key(&mut editor, key);
    }
    let t = &editor.current_buffer().text;
    let lines = (0..t.line_count())
        .map(|l| t.line_str(l).into_owned())
        .collect();
    (lines, editor)
}

#[test]
fn completed_text_is_repeated_by_dot() {
    let (lines, _) = run("value values validate\nvar\n", "Go va<C-n><C-n><Esc>.j0.");
    assert_eq!(
        lines,
        [
            "value values validate",
            "var",
            " values",
            "  values",
            "   values"
        ]
    );
}

#[test]
fn cancel_narrow_and_accept() {
    let (lines, editor) = run(
        "value values validate\nvar\n",
        "Go val<C-p><C-e>u<C-n><C-y>x<Esc>",
    );
    assert_eq!(lines, ["value values validate", "var", " valuex"]);
    let dot = editor.register(Some('.')).map(|r| r.text);
    assert_eq!(dot.as_deref(), Some(" valuex"));
    // Nothing is left of the completion.
    assert!(editor.completion.pum.is_none());
    assert_eq!(editor.completion.submode, Default::default());
}
