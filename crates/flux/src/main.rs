//! The `flux` binary: terminal setup and the event loop.

mod terminal;

use std::io::{self, BufWriter, Write};
use std::path::PathBuf;

use anyhow::{Result, bail};
use crossterm::cursor::SetCursorStyle;
use crossterm::event::{Event, EventStream, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::queue;
use flux_tui::{Grid, Renderer};
use flux_view::{Editor, Mode};
use flux_vim::{Engine, Key, KeyCode, Modifiers};
use futures::StreamExt;

const USAGE: &str = "usage: flux [file ...]";

#[allow(clippy::print_stdout)]
fn main() -> Result<()> {
    let mut files: Vec<PathBuf> = Vec::new();
    for arg in std::env::args_os().skip(1) {
        match arg.to_str() {
            Some("-h" | "--help") => {
                println!("{USAGE}");
                return Ok(());
            }
            Some("-v" | "--version") => {
                println!("flux {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            Some(s) if s.starts_with('-') && s != "-" => bail!("unknown option {s}\n{USAGE}"),
            _ => files.push(arg.into()),
        }
    }

    let (width, height) = crossterm::terminal::size()?;
    let mut editor = Editor::new(width.into(), height.into());
    editor.open_args(&files);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run(editor))
}

async fn run(mut editor: Editor) -> Result<()> {
    let _terminal = terminal::Session::enter()?;
    let mut out = BufWriter::new(io::stdout());
    let mut renderer = Renderer::default();
    let mut engine = Engine::new();
    // Batches whatever events are already waiting (a paste, key repeat) so they are all handled
    // before the next redraw.
    let mut events = EventStream::new().ready_chunks(256);
    let mut cursor_mode = None;
    // The last frame drawn, and the hit-enter screen while it's up: like Vim, a message that
    // needs a prompt is drawn over the screen as it was, without redrawing the text first.
    let mut last_grid: Option<Grid> = None;
    let mut prompt_base: Option<Grid> = None;

    loop {
        let (width, height) = editor.screen_size();
        let same_size = |g: &Grid| g.width() == width && g.height() == height;
        let (grid, cursor) = if editor.hit_enter {
            // The screen as it was when the prompt came up, with the prompt drawn over it.
            let base = match prompt_base.take() {
                Some(g) if same_size(&g) => g,
                _ => match last_grid.take() {
                    Some(g) if same_size(&g) && editor.fresh_message_base.is_none() => g,
                    _ => {
                        // Drawn with the view the message asks for, then the view is put back.
                        let saved = (editor.window.top, editor.window.cursor);
                        if let Some((top, cursor)) = editor.fresh_message_base {
                            editor.window.top = top;
                            editor.window.cursor = cursor;
                        }
                        let mut g = Grid::new(width, height);
                        flux_tui::draw(&editor, "", &mut g);
                        (editor.window.top, editor.window.cursor) = saved;
                        g
                    }
                },
            };
            let mut grid = base.clone();
            prompt_base = Some(base);
            let cursor = flux_tui::draw::draw_hit_enter(&editor, &mut grid);
            (grid, cursor)
        } else if editor.stale_screen
            && editor.mode == Mode::CmdLine
            && last_grid.as_ref().is_some_and(same_size)
        {
            // A command line typed at a prompt: the message stays until the command runs.
            prompt_base = None;
            let mut grid = last_grid.take().expect("checked");
            let cursor = flux_tui::draw::draw_cmdline_only(&editor, &mut grid);
            (grid, cursor)
        } else {
            editor.stale_screen = false;
            prompt_base = None;
            let mut grid = Grid::new(width, height);
            let pending = match editor.mode {
                Mode::Visual => engine.visual_pending_keys(),
                _ => engine.pending_keys(),
            };
            let showcmd: String = pending.iter().map(|k| k.showcmd()).collect();
            let cursor = flux_tui::draw(&editor, &showcmd, &mut grid);
            (grid, cursor)
        };
        if cursor_mode != Some(editor.mode) {
            cursor_mode = Some(editor.mode);
            let style = match editor.mode {
                Mode::Normal | Mode::Visual => SetCursorStyle::SteadyBlock,
                Mode::Insert | Mode::CmdLine => SetCursorStyle::SteadyBar,
            };
            queue!(out, style)?;
        }
        renderer.draw(&mut out, &grid, cursor)?;
        last_grid = Some(grid);

        let Some(batch) = events.next().await else {
            break;
        };
        for event in batch {
            handle_event(event?, &mut editor, &mut engine, &mut renderer);
            if editor.quit {
                break;
            }
        }
        if editor.quit {
            break;
        }
    }
    out.flush()?;
    Ok(())
}

fn handle_event(event: Event, editor: &mut Editor, engine: &mut Engine, renderer: &mut Renderer) {
    match event {
        Event::Key(key) => {
            if let Some(key) = convert_key(key) {
                if key == Key::ctrl('l') {
                    renderer.invalidate();
                }
                engine.handle_key(editor, key);
            }
        }
        Event::Resize(width, height) => {
            editor.resize(width.into(), height.into());
            renderer.invalidate();
        }
        // Like Neovim's 'autoread': notice files changed by other programs.
        Event::FocusGained => editor.check_time(),
        _ => {}
    }
}

fn convert_key(event: KeyEvent) -> Option<Key> {
    if event.kind == KeyEventKind::Release {
        return None;
    }
    use crossterm::event::KeyCode as C;
    let m = event.modifiers;
    let mut mods = Modifiers {
        ctrl: m.contains(KeyModifiers::CONTROL),
        alt: m.contains(KeyModifiers::ALT),
        shift: m.contains(KeyModifiers::SHIFT),
        meta: m.contains(KeyModifiers::SUPER),
    };
    let code = match event.code {
        C::Char(c) => KeyCode::Char(c),
        C::Esc => KeyCode::Esc,
        C::Enter => KeyCode::Enter,
        C::Backspace => KeyCode::Backspace,
        C::Tab => KeyCode::Tab,
        C::BackTab => {
            mods.shift = true;
            KeyCode::Tab
        }
        C::Delete => KeyCode::Delete,
        C::Insert => KeyCode::Insert,
        C::Up => KeyCode::Up,
        C::Down => KeyCode::Down,
        C::Left => KeyCode::Left,
        C::Right => KeyCode::Right,
        C::Home => KeyCode::Home,
        C::End => KeyCode::End,
        C::PageUp => KeyCode::PageUp,
        C::PageDown => KeyCode::PageDown,
        C::F(n) => KeyCode::F(n),
        _ => return None,
    };
    Some(Key::new(code, mods))
}
