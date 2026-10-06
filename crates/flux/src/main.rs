//! The `flux` binary: terminal setup and the event loop.

mod clipboard;
mod servers;
mod terminal;

use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Result, bail};
use clipboard::Clipboard;
use crossterm::cursor::SetCursorStyle;
use crossterm::event::{Event, EventStream, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::queue;
use crossterm::style::Print;
use flux_tui::{Grid, Renderer};
use flux_view::{Editor, Mode};
use flux_vim::{Engine, Key, KeyCode, Modifiers};
use futures::StreamExt;

const USAGE: &str = "usage: flux [file ...]";
/// How long parsing may hold up a frame (see [`Editor::update_syntax_within`]).
const PARSE_SLICE: Duration = Duration::from_millis(20);

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
    // Neovim turns on 'termguicolors' when the terminal says it has 24-bit color.
    editor.options.termguicolors =
        std::env::var("COLORTERM").is_ok_and(|v| matches!(v.as_str(), "truecolor" | "24bit"));
    // The language servers flux knows that are installed start for their filetypes (in Neovim,
    // the configs given to `vim.lsp.enable`).
    // `$FLUX_LSP_CONFIG` names a JSON list of configs to use instead (for testing, until
    // configs can be set in Lua).
    let user_configs = std::env::var_os("FLUX_LSP_CONFIG");
    editor.lsp.configs = match user_configs.as_ref() {
        Some(path) => flux_lsp::config::from_json_file(std::path::Path::new(&path))
            .map_err(anyhow::Error::msg)?,
        None => flux_lsp::builtin_configs(),
    };
    editor.lsp.enabled = editor
        .lsp
        .configs
        .iter()
        .filter(|c| {
            c.cmd
                .first()
                .is_some_and(|p| flux_lsp::config::executable(p))
        })
        .cloned()
        .collect();
    if user_configs.is_none() {
        editor.lsp.auto_enabled = editor.lsp.enabled.iter().map(|c| c.name.clone()).collect();
    }
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
    let (mut servers, mut server_events) = servers::Servers::new();
    // The system clipboard behind `+`/`*`, primed once so a put works first thing.
    let clipboard = Clipboard::probe();
    if let Some(text) = clipboard.read() {
        editor.registers.set_external(text);
    }
    // The last key was a bare `"` naming a register in Normal/Visual mode.
    let mut clip_quote = false;
    let mut cursor_mode = None;
    // The last frame drawn, and the hit-enter screen while it's up: like Vim, a message that
    // needs a prompt is drawn over the screen as it was, without redrawing the text first.
    let mut last_grid: Option<Grid> = None;
    let mut prompt_base: Option<Grid> = None;
    // Keys typed while a request is waited for (Neovim's `request_sync`, as 'formatexpr' makes
    // for `gq`): they're handled once it's answered.
    let mut held: std::collections::VecDeque<Event> = std::collections::VecDeque::new();

    loop {
        if editor.lsp.waiting.is_none() && !held.is_empty() {
            while let Some(event) = held.pop_front() {
                handle_event(
                    event,
                    &mut editor,
                    &mut engine,
                    &mut renderer,
                    &clipboard,
                    &mut clip_quote,
                );
                if editor.quit || editor.lsp.waiting.is_some() {
                    break;
                }
            }
            // Drained before the quit check: a yank-then-quit must still reach
            // the clipboard.
            drain_clipboard(&mut out, &mut editor, &clipboard)?;
            if editor.quit {
                break;
            }
        }
        // Parsing gets a slice of each frame; a long one goes on between keys.
        let parsing = editor.update_syntax_within(Some(PARSE_SLICE));
        editor.fit_floats();
        editor.semantic_tokens_update(std::time::Instant::now());
        editor.update_matchparen();
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

        if servers.flush(&mut editor) {
            // A spawn error has no server event to wake the loop; redraw its message now.
            continue;
        }
        // A debounced semantic tokens request to send.
        let wake = editor
            .lsp
            .semantic_tokens
            .deadline()
            .map(tokio::time::Instant::from_std);
        let deadline = editor.lsp.waiting.map(|(_, _, at)| at);
        let batch = tokio::select! {
            biased;
            batch = events.next() => batch,
            // The server took too long: stop waiting (its answer will be ignored).
            () = async {
                match deadline {
                    Some(at) => tokio::time::sleep_until(at.into()).await,
                    None => std::future::pending().await,
                }
            }, if deadline.is_some() => {
                editor.lsp.waiting = None;
                continue;
            }
            Some(event) = server_events.recv() => {
                servers.handle(event, &mut editor, &mut engine);
                // Take whatever else arrived before redrawing.
                while let Ok(event) = server_events.try_recv() {
                    servers.handle(event, &mut editor, &mut engine);
                }
                continue;
            }
            // No key yet: parse some more.
            () = std::future::ready(()), if parsing => continue,
            () = async { tokio::time::sleep_until(wake.expect("checked")).await }, if wake.is_some() => continue,
        };
        let Some(batch) = batch else {
            // stdin closed: still deliver a pending clipboard write.
            drain_clipboard(&mut out, &mut editor, &clipboard)?;
            break;
        };
        for event in batch {
            let event = event?;
            if editor.lsp.waiting.is_some() {
                held.push_back(event);
                continue;
            }
            handle_event(
                event,
                &mut editor,
                &mut engine,
                &mut renderer,
                &clipboard,
                &mut clip_quote,
            );
            if editor.quit {
                break;
            }
        }
        // Drained before the quit check: a yank-then-quit must still reach
        // the clipboard.
        drain_clipboard(&mut out, &mut editor, &clipboard)?;
        if editor.quit {
            break;
        }
    }
    servers
        .shutdown(&mut editor, &mut engine, &mut server_events)
        .await;
    out.flush()?;
    Ok(())
}

/// Copy a yanked `+`/`*` write to the system clipboard (or record it for the
/// fake override).
fn drain_clipboard(
    out: &mut impl Write,
    editor: &mut Editor,
    clipboard: &Clipboard,
) -> io::Result<()> {
    if let Some(reg) = editor.registers.take_outbound() {
        let text = reg.clipboard_text();
        // Under the fake override the write is recorded for tests; otherwise
        // it goes out as OSC 52 for the terminal to deliver.
        if !clipboard.fake_write(&text) {
            queue!(out, Print(Clipboard::osc52(&text)))?;
        }
    }
    Ok(())
}

fn handle_event(
    event: Event,
    editor: &mut Editor,
    engine: &mut Engine,
    renderer: &mut Renderer,
    clipboard: &Clipboard,
    clip_quote: &mut bool,
) {
    match event {
        Event::Key(key) => {
            if let Some(key) = convert_key(key) {
                if key == Key::ctrl('l') {
                    renderer.invalidate();
                }
                // A `"` naming `+`/`*`: sync the mirrors from the system
                // clipboard before the engine reads them (this also covers
                // terminals without focus events).
                let typed = key.typed_char();
                let naming = matches!(editor.mode, Mode::Normal | Mode::Visual);
                if *clip_quote
                    && naming
                    && matches!(typed, Some('+' | '*'))
                    && let Some(text) = clipboard.read()
                {
                    editor.registers.set_external(text);
                }
                *clip_quote = naming && typed == Some('"');
                engine.handle_key(editor, key);
            }
        }
        Event::Resize(width, height) => {
            editor.resize(width.into(), height.into());
            renderer.invalidate();
        }
        Event::FocusGained => {
            if let Some(text) = clipboard.read() {
                editor.registers.set_external(text);
            }
            // Like Neovim's 'autoread': notice files changed by other programs.
            editor.check_time();
        }
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
