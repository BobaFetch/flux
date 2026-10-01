//! Window commands (`CTRL-W …`) and switching buffers (`CTRL-^`).

use flux_view::Editor;

use crate::engine::Engine;
use crate::normal::normalize_cursor;
use crate::parse::WinCmd;

impl Engine {
    pub(crate) fn window_command(
        &mut self,
        editor: &mut Editor,
        cmd: WinCmd,
        count: Option<usize>,
    ) {
        let ids = editor.window_ids();
        let pos = ids.iter().position(|&w| w == editor.window.id).unwrap_or(0);
        let goto = |editor: &mut Editor, i: usize| {
            let id = editor.window_ids()[i];
            editor.goto_window(id);
        };
        match cmd {
            WinCmd::Split => {
                editor.split(false, count);
            }
            WinCmd::VSplit => {
                editor.split(true, count);
            }
            WinCmd::New => {
                if editor.split(false, count) {
                    let id = editor.new_buffer();
                    editor.show_buffer(id);
                }
            }
            WinCmd::SplitAlternate => {
                let target = match count {
                    Some(n) => Some(flux_view::BufferId(n)),
                    None => editor.window.alt_buffer,
                };
                match target.filter(|&b| editor.buffer(b).is_some()) {
                    Some(b) => {
                        if editor.split(false, None) {
                            editor.show_buffer(b);
                        }
                    }
                    None => editor.error("E23: No alternate file"),
                }
            }
            WinCmd::Close => {
                let id = editor.window.id;
                if !editor.close_window(id) {
                    editor.error("E444: Cannot close last window");
                }
            }
            WinCmd::Quit => crate::ex::run(self, editor, "quit"),
            WinCmd::Only => editor.only_window(),
            WinCmd::Next => match count {
                Some(n) => goto(editor, (n.max(1) - 1).min(ids.len() - 1)),
                None => goto(editor, (pos + 1) % ids.len()),
            },
            WinCmd::Prev => match count {
                Some(n) => goto(editor, (n.max(1) - 1).min(ids.len() - 1)),
                None => goto(editor, (pos + ids.len() - 1) % ids.len()),
            },
            WinCmd::Previous => match editor.prev_window {
                Some(id) if editor.layout.contains(id) => editor.goto_window(id),
                _ => self.failed = true,
            },
            WinCmd::Top => goto(
                editor,
                count.map_or(0, |n| (n.max(1) - 1).min(ids.len() - 1)),
            ),
            WinCmd::Bottom => goto(editor, ids.len() - 1),
            WinCmd::Go(dir) => {
                let at = cursor_on_screen(editor);
                match editor
                    .layout
                    .neighbor(editor.window.id, dir, count.unwrap_or(1), at)
                {
                    Some(id) => editor.goto_window(id),
                    None => self.failed = true,
                }
            }
            WinCmd::Taller | WinCmd::Shorter | WinCmd::SetHeight => {
                let h = editor.window.height;
                let n = count.unwrap_or(1);
                let target = match cmd {
                    WinCmd::Taller => h + n,
                    WinCmd::Shorter => h.saturating_sub(n).max(1),
                    _ => count.unwrap_or(usize::MAX / 2),
                };
                let id = editor.window.id;
                editor.layout.set_height(id, target);
                editor.sync_window_sizes();
            }
            WinCmd::Wider | WinCmd::Narrower | WinCmd::SetWidth => {
                let w = editor.window.width;
                let n = count.unwrap_or(1);
                let target = match cmd {
                    WinCmd::Wider => w + n,
                    WinCmd::Narrower => w.saturating_sub(n).max(1),
                    _ => count.unwrap_or(usize::MAX / 2),
                };
                let id = editor.window.id;
                editor.layout.set_width(id, target);
                editor.sync_window_sizes();
            }
            WinCmd::Equalize => editor.equalize_windows(),
            WinCmd::Exchange => {
                let id = editor.window.id;
                match editor.layout.exchange(id) {
                    // The cursor stays where it was on screen: in the other window.
                    Some(other) => {
                        editor.goto_window(other);
                        editor.sync_window_sizes();
                    }
                    None => self.failed = true,
                }
            }
            WinCmd::Rotate { down } => {
                let id = editor.window.id;
                if editor.layout.rotate(id, down) {
                    editor.sync_window_sizes();
                } else {
                    editor.error("E443: Cannot rotate when another window is split");
                }
            }
            WinCmd::Move(edge) => {
                let id = editor.window.id;
                editor.layout.move_to_edge(id, edge);
                editor.sync_window_sizes();
            }
        }
    }

    /// `CTRL-^`: the alternate buffer, or buffer `count`.
    pub(crate) fn alternate_buffer(&mut self, editor: &mut Editor, count: Option<usize>) {
        let target = match count {
            Some(n) => Some(flux_view::BufferId(n)),
            None => editor.window.alt_buffer,
        };
        match target.filter(|&b| editor.buffer(b).is_some()) {
            Some(b) => {
                editor.show_buffer(b);
                normalize_cursor(editor);
            }
            None if count.is_some() => {
                editor.error(format!("E86: Buffer {} does not exist", count.unwrap_or(0)))
            }
            None => editor.error("E23: No alternate file"),
        }
    }
}

/// The cursor's screen position, which picks the window `CTRL-W h/j/k/l` goes to.
fn cursor_on_screen(editor: &Editor) -> (usize, usize) {
    let rect = editor
        .layout
        .rect(editor.window.id)
        .unwrap_or(flux_view::Rect {
            row: 0,
            col: 0,
            width: editor.window.width,
            height: editor.window.height,
            vsep: false,
        });
    let m = editor.metrics();
    let win = &editor.window;
    let above: usize = (win.top..win.cursor.line).map(|l| m.rows(l)).sum();
    let layout =
        flux_core::layout_line(&m.text.line_str(win.cursor.line), m.tabstop, Some(m.width));
    let (r, x) = layout.cursor_position(win.cursor.col, false);
    (rect.row + above + r, rect.col + x)
}
