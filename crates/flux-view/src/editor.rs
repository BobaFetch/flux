use std::path::Path;

use flux_core::Text;

use crate::{Buffer, BufferId, Cursor, Metrics, Registers, Window};

/// Rows below the text area: the statusline and the command line.
const CHROME_ROWS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Visual,
    CmdLine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualKind {
    Char,
    Line,
}

/// The Visual selection: from `anchor` to the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Visual {
    pub anchor: Cursor,
    pub kind: VisualKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub text: String,
    pub kind: MessageKind,
}

/// How a message that doesn't fit on the command line is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    /// Cut in the middle, leaving `...` ('shortmess' `T`).
    Info,
    /// A file message (`:w`): loses its start, marked `<` ('shortmess' `t`).
    File,
    /// Shown in full, wrapping and waiting for a key (`CTRL-G`).
    Full,
    /// Shown in full in the error color.
    Error,
}

impl Message {
    pub fn is_error(&self) -> bool {
        self.kind == MessageKind::Error
    }
}

/// Neovim's defaults for the options flux implements so far.
#[derive(Debug, Clone)]
pub struct Options {
    pub tabstop: usize,
    pub shiftwidth: usize,
    pub expandtab: bool,
    pub autoindent: bool,
    pub smarttab: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            tabstop: 8,
            shiftwidth: 8,
            expandtab: false,
            autoindent: true,
            smarttab: true,
        }
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
    pub registers: Registers,
    /// A message longer than one line is on screen, waiting for a key (Vim's hit-enter prompt).
    pub hit_enter: bool,
    /// A Normal-mode command is being typed with `CTRL-O` from Insert mode.
    pub insert_pending: bool,
    pub visual: Visual,
    /// The register a macro is being recorded into (`qa`).
    pub recording: Option<char>,
    /// Counts errors reported, so a running macro can stop at the first one.
    pub error_count: u64,
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
            registers: Registers::default(),
            hit_enter: false,
            insert_pending: false,
            visual: Visual {
                anchor: Cursor::default(),
                kind: VisualKind::Char,
            },
            recording: None,
            error_count: 0,
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
        self.reset_view();
    }

    /// Start at the top of a freshly loaded buffer. Like Vim's `:edit`, that position goes in
    /// the jumplist, and `'"` (last position in the file) starts at the top too.
    fn reset_view(&mut self) {
        self.window.cursor = Default::default();
        self.window.top = 0;
        self.window.pcmark = Some(Cursor::default());
        self.window.jumps.push(Cursor::default());
        self.current_buffer_mut().marks.set('"', Cursor::default());
    }

    /// Replace the current buffer's text, as if it had been loaded, and reset the view.
    pub fn set_text(&mut self, text: &str) {
        let id = self.window.buffer;
        self.buffers[id.0].text = Text::new(text);
        self.buffers[id.0].history = Default::default();
        self.reset_view();
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

    pub fn current_buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffers[self.window.buffer.0]
    }

    pub fn text(&self) -> &Text {
        &self.current_buffer().text
    }

    pub fn cursor(&self) -> Cursor {
        self.window.cursor
    }

    /// Metrics of the current window's buffer.
    pub fn metrics(&self) -> Metrics<'_> {
        Metrics {
            text: &self.current_buffer().text,
            tabstop: self.options.tabstop,
            width: self.window.width,
            height: self.window.height,
        }
    }

    /// Vim's `:checktime`, also run when the terminal regains focus: if the file changed on
    /// disk, reload it when there are no unsaved changes ('autoread'), otherwise warn.
    pub fn check_time(&mut self) {
        let buffer = self.current_buffer();
        if !buffer.changed_on_disk() {
            return;
        }
        let name = buffer.name();
        let exists = buffer.path.as_ref().is_some_and(|p| p.exists());
        if !exists {
            self.current_buffer_mut().acknowledge_disk_state();
            self.error(format!("E211: File \"{name}\" no longer available"));
        } else if buffer.modified() {
            self.current_buffer_mut().acknowledge_disk_state();
            self.error(format!(
                "W12: Warning: File \"{name}\" has changed and the buffer was changed in Vim as well"
            ));
        } else {
            let cursor = self.window.cursor;
            if let Err(e) = self.current_buffer_mut().reload((cursor.line, cursor.col)) {
                self.error(format!("\"{name}\" {e}"));
            }
            self.with_window(|win, m| {
                let line = win.cursor.line.min(m.text.last_line());
                win.cursor.line = line;
                win.cursor.col = win.cursor.col.min(m.text.line_len(line).saturating_sub(1));
                win.scroll_to_cursor(m);
            });
        }
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
        self.show(text.into(), MessageKind::Info);
    }

    /// A message about a file, like `"main.rs" 42L, 1337B written`.
    pub fn file_message(&mut self, text: impl Into<String>) {
        self.show(text.into(), MessageKind::File);
    }

    /// A message shown in full even when it doesn't fit on one line.
    pub fn full_message(&mut self, text: impl Into<String>) {
        self.show(text.into(), MessageKind::Full);
    }

    fn show(&mut self, text: String, kind: MessageKind) {
        let wraps = matches!(kind, MessageKind::Full | MessageKind::Error)
            && unicode_width::UnicodeWidthStr::width(text.as_str()) >= self.screen_width.max(1);
        // Anything longer than one line waits for a key.
        self.hit_enter = text.contains('\n') || wraps;
        self.message = Some(Message { text, kind });
    }

    /// An error message. One that doesn't fit on the command line wraps, and waits for a key
    /// like any multi-line message.
    pub fn error(&mut self, text: impl Into<String>) {
        self.error_count += 1;
        self.show(text.into(), MessageKind::Error);
    }

    /// Register contents, including the read-only `"%` (the file name).
    pub fn register(&self, name: Option<char>) -> Option<crate::Register> {
        if name == Some('%') {
            let path = self.current_buffer().path.as_ref()?;
            return Some(crate::Register::new(
                path.display().to_string(),
                crate::RegisterKind::Char,
            ));
        }
        self.registers.get(name).cloned()
    }
}
