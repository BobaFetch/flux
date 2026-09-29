use std::path::Path;

use crate::{Buffer, BufferId, Metrics, Window};

/// Rows below the text area: the statusline and the command line.
const CHROME_ROWS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    CmdLine,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub text: String,
    pub is_error: bool,
}

#[derive(Debug, Clone)]
pub struct Options {
    pub tabstop: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self { tabstop: 8 }
    }
}

#[derive(Debug)]
pub struct Editor {
    pub buffers: Vec<Buffer>,
    pub window: Window,
    pub mode: Mode,
    /// Text typed after `:`.
    pub cmdline: String,
    pub message: Option<Message>,
    pub options: Options,
    pub quit: bool,
    screen_width: usize,
    screen_height: usize,
}

impl Editor {
    /// An editor with one empty buffer on a `width` x `height` screen.
    pub fn new(width: usize, height: usize) -> Self {
        let id = BufferId(0);
        let mut editor = Self {
            buffers: vec![Buffer::scratch(id)],
            window: Window::new(id, 0, 0),
            mode: Mode::Normal,
            cmdline: String::new(),
            message: None,
            options: Options::default(),
            quit: false,
            screen_width: 0,
            screen_height: 0,
        };
        editor.resize(width, height);
        editor
    }

    /// Replace the current buffer with `path`. Like Neovim (`'shortmess'` has `F`), a successful
    /// load is silent; only errors are reported.
    pub fn open(&mut self, path: &Path) {
        let id = self.window.buffer;
        match Buffer::open(id, path) {
            Ok(buffer) => self.buffers[id.0] = buffer,
            Err(e) => self.error(format!("\"{}\" {e}", path.display())),
        }
        self.window.cursor = Default::default();
        self.window.top = 0;
    }

    /// Replace the current buffer's text, as if it had been loaded, and reset the view.
    pub fn set_text(&mut self, text: &str) {
        let id = self.window.buffer;
        self.buffers[id.0].text = flux_core::Text::new(text);
        self.window.cursor = Default::default();
        self.window.top = 0;
    }

    pub fn screen_size(&self) -> (usize, usize) {
        (self.screen_width, self.screen_height)
    }

    /// Resize the screen. Like Neovim, the window takes its new height first (keeping the
    /// cursor at the same relative height) and then its new width.
    pub fn resize(&mut self, width: usize, height: usize) {
        if (width, height) != (self.screen_width, self.screen_height) {
            // A resize repaints the whole screen, which clears the message line.
            self.message = None;
        }
        self.screen_width = width;
        self.screen_height = height;
        let (width, height) = (width.max(1), height.saturating_sub(CHROME_ROWS).max(1));
        let text = &self.buffers[self.window.buffer.0].text;
        let tabstop = self.options.tabstop;
        let old_width = self.window.width.max(1);
        self.window.set_height(&Metrics {
            text,
            tabstop,
            width: old_width,
            height,
        });
        self.window.set_width(&Metrics {
            text,
            tabstop,
            width,
            height,
        });
    }

    pub fn current_buffer(&self) -> &Buffer {
        &self.buffers[self.window.buffer.0]
    }

    /// Run `f` with the window and the metrics of the buffer it shows.
    pub fn with_window<R>(&mut self, f: impl FnOnce(&mut Window, &Metrics) -> R) -> R {
        let buffer = &self.buffers[self.window.buffer.0];
        let metrics = Metrics {
            text: &buffer.text,
            tabstop: self.options.tabstop,
            width: self.window.width,
            height: self.window.height,
        };
        f(&mut self.window, &metrics)
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.message = Some(Message {
            text: text.into(),
            is_error: false,
        });
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.message = Some(Message {
            text: text.into(),
            is_error: true,
        });
    }
}
