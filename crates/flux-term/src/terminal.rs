//! Putting the terminal into editor mode and reliably restoring it, including after a panic.

use std::io::{self, Write};

use crossterm::cursor::{SetCursorStyle, Show};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

/// Raw mode on the alternate screen for as long as this lives.
pub struct Session;

impl Session {
    pub fn enter() -> io::Result<Self> {
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            default_hook(info);
        }));
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;
        Ok(Self)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        restore();
    }
}

fn restore() {
    let mut out = io::stdout();
    // Best effort: there's nowhere to report a failure while tearing down.
    let _ = execute!(
        out,
        SetCursorStyle::DefaultUserShape,
        Show,
        LeaveAlternateScreen
    );
    let _ = disable_raw_mode();
    let _ = out.flush();
}
