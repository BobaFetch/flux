//! Drawing Visual selections over the kinds of lines the layout handles specially.

use flux_tui::{Grid, draw};
use flux_view::Editor;
use flux_vim::{Engine, parse_keys};

fn sample() -> String {
    let mut s = String::new();
    for i in 1..=40 {
        match i {
            3 => s.push_str("\tindented\twith tabs\n"),
            5 => s.push_str("wide: 日本語テキスト and emoji 🎉 end\n"),
            7 => s.push_str("ctrl: a\u{1}b\u{7f}c\n"),
            n if n % 10 == 0 => s.push_str(&format!("{n} {}\n", "long ".repeat(40))),
            n => s.push_str(&format!("line {n}\n")),
        }
    }
    s
}

#[test]
fn selections_draw_without_panicking() {
    for keys in [
        "vip", "vG", "VG", "v$", "5Gv$j", "vjjj", "10Gve", "5G0vee", "vipo", "Gvgg",
    ] {
        let mut editor = Editor::new(80, 24);
        editor.set_text(&sample());
        let mut engine = Engine::new();
        for key in parse_keys(keys) {
            engine.handle_key(&mut editor, key);
        }
        let mut grid = Grid::new(80, 24);
        draw(&editor, "", &mut grid);
    }
}
