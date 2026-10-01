use std::collections::HashMap;
use std::path::{Path, PathBuf};

use flux_core::Text;

pub use crate::options::Options;
use crate::{
    Buffer, BufferId, Cursor, Dir, Jump, Layout, Metrics, Rect, Registers, Window, WindowId,
};

/// Width of the sign column ('signcolumn') in a window showing a buffer that has signs or not.
pub fn sign_width(signcolumn: &str, has_signs: bool) -> usize {
    match signcolumn {
        "yes" => 2,
        "auto" if has_signs => 2,
        _ => 0,
    }
}

/// The command line: the one row below the windows.
/// Rows below the windows for the command line and messages.
pub const CMDLINE_ROWS: usize = 1;
/// Vim's 'winheight' and 'winwidth': the least size of the current window.
const WINHEIGHT: usize = 1;
const WINWIDTH: usize = 20;

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
    /// A question waiting for an answer (`(y/n)?`), with the cursor after it.
    Question,
    /// Shown in full in the warning color (WarningMsg).
    Warning,
}

impl Message {
    pub fn is_error(&self) -> bool {
        self.kind == MessageKind::Error
    }
}

/// A live preview of a command's effect on the current buffer ('inccommand').
#[derive(Debug, Clone)]
pub struct Preview {
    pub text: Text,
    pub highlights: Vec<(Cursor, Cursor)>,
    /// The text differs from the buffer's (shown as `[+]`, as Neovim does).
    pub changed: bool,
    /// Where incsearch puts the cursor: the first match from the start of the range.
    pub first_match: Option<Cursor>,
}

#[derive(Debug)]
pub struct Editor {
    /// Every buffer, in number order (`:ls`).
    pub buffers: Vec<Buffer>,
    /// The current window.
    pub window: Window,
    /// The other windows.
    pub windows: Vec<Window>,
    pub layout: Layout,
    /// The window `CTRL-W p` goes back to.
    pub prev_window: Option<WindowId>,
    /// `'A`–`'Z`: marks that name a file as well as a position.
    pub global_marks: HashMap<char, (BufferId, Cursor)>,
    /// The directory relative file names are taken from (`:cd`).
    pub cwd: PathBuf,
    pub mode: Mode,
    /// Text typed on the command line, after its `cmdline_kind` character.
    pub cmdline: String,
    /// `:` for an Ex command, `/` or `?` for a search.
    pub cmdline_kind: char,
    /// The cursor on the command line, as a char index into `cmdline`.
    pub cmdline_pos: usize,
    /// The mode the command line returns to (Visual for a search typed in Visual mode).
    pub cmdline_return: Mode,
    /// 'incsearch' while typing a search: the match to show, and the pattern typed so far
    /// (highlighted instead of the last search pattern).
    pub incsearch: Option<(Cursor, Cursor)>,
    pub incsearch_pattern: Option<String>,
    /// 'inccommand' while typing `:s`: the buffer text as the command would leave it, and the
    /// parts to highlight (the replacements, or the matches before a replacement is typed).
    pub preview: Option<Preview>,
    pub message: Option<Message>,
    pub options: Options,
    /// `:syntax on` / `:syntax off`.
    pub syntax_on: bool,
    /// `:filetype` detection, plugin and indent.
    pub filetype: crate::filetype::FiletypeSettings,
    /// The brackets MatchParen highlights in the current window (see [`crate::matchparen`]).
    pub matchparen: Option<[Cursor; 2]>,
    /// Language servers (see [`crate::lsp`]).
    pub lsp: crate::lsp::LspState,
    /// Floating windows (hover, diagnostics), drawn over the others.
    pub floats: Vec<crate::float::Float>,
    /// Quickfix and location lists, and the tag stacks (see [`crate::quickfix`]).
    pub quickfix: crate::quickfix::QuickfixState,
    /// Insert-mode completion's menu and mode message (see [`crate::pum`]).
    pub completion: crate::pum::CompletionView,
    pub registers: Registers,
    /// A message longer than one line is on screen, waiting for a key (Vim's hit-enter prompt).
    pub hit_enter: bool,
    /// Paging through a long message (`-- More --`): the first row shown.
    pub more_top: Option<usize>,
    /// A key the pager doesn't know was typed: show its keys.
    pub more_help: bool,
    /// The message was shown by this command to stay (Vim's `keep_msg`): line-count messages
    /// don't replace it.
    pub keep_msg: bool,
    /// A kept message to show again when the command ends (after an Insert mode it started).
    pub kept_message: Option<Message>,
    /// For output made by running a command on many lines (`:g/pat/p`): the cursor after each
    /// message line was produced. Quitting the pager early stops there, as Vim does (it pages
    /// while the command runs).
    pub message_positions: Vec<Option<Cursor>>,
    /// Highlights in the message's lines (`:tags` shows its header in Title): line, char
    /// range and group.
    pub message_highlights: Vec<(usize, usize, usize, &'static str)>,
    /// The furthest message row the pager has reached (the command has run up to it).
    pub more_max_row: usize,
    /// The window's top line before the command that made the message.
    pub more_restore_top: Option<usize>,
    /// Draw the message over a freshly drawn screen, with this view (top line, cursor),
    /// rather than the screen before the command (`:g` redraws before its output).
    pub fresh_message_base: Option<(usize, Cursor)>,
    /// The screen isn't redrawn above the command line (it was typed at a prompt, over a
    /// message still showing).
    pub stale_screen: bool,
    /// A Normal-mode command is being typed with `CTRL-O` from Insert mode.
    pub insert_pending: bool,
    pub visual: Visual,
    /// The register a macro is being recorded into (`qa`).
    pub recording: Option<char>,
    /// Counts errors reported, so a running macro can stop at the first one.
    pub error_count: u64,
    pub quit: bool,
    /// The argument list: the files named on the command line.
    pub args: Vec<BufferId>,
    /// The last file in the argument list has been shown (Vim's `arg_had_last`), so quitting
    /// doesn't warn about files not yet edited.
    pub arg_had_last: bool,
    /// Set by `E173` so that `:q` right after it quits anyway (Vim's `quitmore`); counts down
    /// with every Ex command.
    pub quitmore: u8,
    /// The next buffer switch is into a window just split off (`:new`, `:split file`), which
    /// doesn't record where the old buffer was left (Vim's `do_ecmd` without `oldwin`).
    pub in_new_window: bool,
    /// The statusline doesn't show `[+]` for the current buffer yet (Neovim doesn't redraw it
    /// while `:s///c` asks).
    pub hide_modified: bool,
    /// `:g` is running: jumps aren't remembered (Vim's `setpcmark` does nothing then).
    pub global_busy: bool,
    /// The next buffer switch may abandon unsaved changes (`:e!`, `:b!` without 'hidden').
    pub force_abandon: bool,
    unload_on_switch: Option<BufferId>,
    /// The last search and substitute patterns, and command-line history.
    pub search: crate::search::SearchState,
    screen_width: usize,
    screen_height: usize,
    next_buffer: usize,
    next_window: usize,
}

impl Editor {
    /// An editor with one empty buffer in one window on a `width` x `height` screen.
    pub fn new(width: usize, height: usize) -> Self {
        let buffer = BufferId(1);
        // Vim numbers windows from 1000.
        let win = WindowId(1000);
        let mut first = Buffer::scratch(buffer);
        first.created_in(win);
        let mut editor = Self {
            buffers: vec![first],
            window: Window::new(win, buffer, 0, 0),
            windows: Vec::new(),
            layout: Layout::new(win, 1, 2),
            prev_window: None,
            global_marks: HashMap::new(),
            cwd: std::env::current_dir().unwrap_or_default(),
            mode: Mode::Normal,
            cmdline: String::new(),
            cmdline_kind: ':',
            cmdline_pos: 0,
            cmdline_return: Mode::Normal,
            incsearch: None,
            incsearch_pattern: None,
            preview: None,
            message: None,
            options: Options::default(),
            syntax_on: true,
            filetype: Default::default(),
            matchparen: None,
            lsp: Default::default(),
            floats: Vec::new(),
            quickfix: Default::default(),
            completion: Default::default(),
            registers: Registers::default(),
            hit_enter: false,
            more_top: None,
            more_help: false,
            keep_msg: false,
            kept_message: None,
            message_positions: Vec::new(),
            message_highlights: Vec::new(),
            more_max_row: 0,
            more_restore_top: None,
            fresh_message_base: None,
            stale_screen: false,
            insert_pending: false,
            visual: Visual {
                anchor: Cursor::default(),
                kind: VisualKind::Char,
            },
            recording: None,
            error_count: 0,
            quit: false,
            args: Vec::new(),
            arg_had_last: false,
            quitmore: 0,
            in_new_window: false,
            search: Default::default(),
            force_abandon: false,
            global_busy: false,
            hide_modified: false,
            unload_on_switch: None,
            screen_width: 0,
            screen_height: 0,
            next_buffer: 2,
            next_window: 1001,
        };
        editor.resize(width, height);
        editor
    }

    // ----- buffers

    pub fn buffer(&self, id: BufferId) -> Option<&Buffer> {
        self.buffers.iter().find(|b| b.id == id)
    }

    pub fn buffer_mut(&mut self, id: BufferId) -> Option<&mut Buffer> {
        self.buffers.iter_mut().find(|b| b.id == id)
    }

    pub fn current_buffer(&self) -> &Buffer {
        self.buffer(self.window.buffer)
            .expect("the current window shows a buffer")
    }

    pub fn current_buffer_mut(&mut self) -> &mut Buffer {
        let id = self.window.buffer;
        self.buffer_mut(id)
            .expect("the current window shows a buffer")
    }

    /// A path as given, taken relative to the editor's directory.
    pub fn resolve(&self, path: &Path) -> PathBuf {
        let full = self.cwd.join(path);
        std::fs::canonicalize(&full).unwrap_or(full)
    }

    /// The buffer for `path`, if one exists.
    pub fn find_buffer(&self, path: &Path) -> Option<BufferId> {
        let target = self.resolve(path);
        self.buffers
            .iter()
            .find(|b| b.path.as_ref().is_some_and(|p| self.resolve(p) == target))
            .map(|b| b.id)
    }

    fn add_buffer(&mut self, mut buffer: Buffer) -> BufferId {
        let id = BufferId(self.next_buffer);
        self.next_buffer += 1;
        buffer.id = id;
        buffer.opts = self.options.buffer.clone();
        buffer.created_in(self.window.id);
        self.buffers.push(buffer);
        id
    }

    /// Add `buffer` without showing it.
    pub(crate) fn add_buffer_hidden(&mut self, buffer: Buffer) -> BufferId {
        self.add_buffer(buffer)
    }

    /// A new empty buffer (`:enew`, `:new`).
    pub fn new_buffer(&mut self) -> BufferId {
        self.add_buffer(Buffer::scratch(BufferId(0)))
    }

    /// Add buffers for files named on the command line; the first is shown, the others are
    /// read when first shown.
    pub fn open_args(&mut self, paths: &[PathBuf]) {
        let Some((first, rest)) = paths.split_first() else {
            return;
        };
        self.open(first);
        self.args = vec![self.window.buffer];
        for path in rest {
            let id = match self.find_buffer(path) {
                Some(id) => id,
                None => self.add_buffer(Buffer::unloaded(BufferId(0), path)),
            };
            self.args.push(id);
        }
        self.check_arg_idx();
    }

    /// Vim's `check_arg_idx`: note when the current window shows the last file of the
    /// argument list.
    fn check_arg_idx(&mut self) {
        if self.args.last() == Some(&self.window.buffer) {
            self.arg_had_last = true;
        }
    }

    /// Open `path` in the current window at startup, replacing the empty first buffer. Like
    /// Neovim (`'shortmess'` has `F`), a successful load is silent; only errors are reported.
    pub fn open(&mut self, path: &Path) {
        let id = self.window.buffer;
        let full = self.cwd.join(path);
        match Buffer::open(id, &full) {
            Ok(mut buffer) => {
                if !buffer.directory {
                    buffer.path = Some(path.to_path_buf());
                }
                let current = self.current_buffer_mut();
                buffer.positions = std::mem::take(&mut current.positions);
                buffer.opts = current.opts.clone();
                *current = buffer;
                self.detect_filetype(id);
            }
            Err(e) => self.error(format!("\"{}\" {e}", path.display())),
        }
        self.reset_view();
    }

    /// `:edit {file}`: show `path` in the current window, reading it into a new buffer unless
    /// one is already open.
    pub fn edit_file(&mut self, path: &Path) -> Result<(), String> {
        let id = match self.find_buffer(path) {
            Some(id) => {
                // A directory listing is read again each time it is shown.
                let buffer = self.buffer_mut(id).expect("found");
                if buffer.directory {
                    buffer
                        .reload((0, 0))
                        .map_err(|e| format!("\"{}\" {e}", path.display()))?;
                }
                id
            }
            None => {
                let full = self.cwd.join(path);
                let mut buffer = Buffer::open(BufferId(0), &full)
                    .map_err(|e| format!("\"{}\" {e}", path.display()))?;
                if !buffer.directory {
                    buffer.path = Some(path.to_path_buf());
                }
                let id = self.add_buffer(buffer);
                self.detect_filetype(id);
                id
            }
        };
        self.show_buffer(id);
        Ok(())
    }

    /// Show buffer `id` in the current window: the old one becomes the alternate buffer, the
    /// cursor goes back to where it last was in `id`, and the jump is remembered.
    pub fn show_buffer(&mut self, id: BufferId) {
        self.switch_buffer(id, true);
    }

    /// Like [`Editor::show_buffer`]; `remember` is false when moving through the jumplist
    /// itself.
    pub fn switch_buffer(&mut self, id: BufferId, remember: bool) {
        let old = self.window.buffer;
        if old == id {
            return;
        }
        // Without 'hidden' a buffer can't be left with unsaved changes unless another window
        // shows it (or `!` was used), and one that is left is unloaded.
        let shown_elsewhere = self.windows.iter().any(|w| w.buffer == old);
        let force = std::mem::take(&mut self.force_abandon);
        if !self.options.hidden && !shown_elsewhere {
            if self.current_buffer().modified() && !force {
                self.error("E37: No write since last change (add ! to override)");
                return;
            }
            self.unload_on_switch = Some(old);
        }
        let win = self.window.id;
        let cursor = self.window.cursor;
        let new_window = std::mem::take(&mut self.in_new_window);
        if let Some(b) = self.buffer_mut(old) {
            if !new_window {
                b.remember_position(win, cursor);
            }
            b.marks.set('"', cursor);
        }
        if remember {
            self.window.jumps.push(Jump {
                buffer: old,
                pos: cursor,
            });
            self.window.pcmark = Some(cursor);
        }
        if let Some(buffer) = self.buffer_mut(id) {
            let was_loaded = buffer.loaded;
            let loaded = buffer.load();
            buffer.listed = !buffer.directory;
            match loaded {
                Err(e) => {
                    let name = buffer.name();
                    self.error(format!("\"{name}\" {e}"));
                }
                Ok(()) if !was_loaded => self.detect_filetype(id),
                Ok(()) => {}
            }
        }
        self.window.alt_buffer = Some(old);
        self.window.buffer = id;
        self.check_arg_idx();
        let pos = self.current_buffer().last_position(win).unwrap_or_default();
        self.window.top = 0;
        self.window.set_curswant = true;
        self.with_window(|w, m| {
            let line = pos.line.min(m.text.last_line());
            let col = pos.col.min(m.text.line_len(line).saturating_sub(1));
            w.cursor = Cursor { line, col };
            w.scroll_to_cursor(m);
        });
        if let Some(old) = self.unload_on_switch.take()
            && self.buffer(old).is_some_and(|b| b.path.is_some())
        {
            self.lsp_detach(old);
            if let Some(b) = self.buffer_mut(old) {
                b.unload();
            }
        }
    }

    /// Listed buffers in number order.
    pub fn listed_buffers(&self) -> Vec<BufferId> {
        self.buffers
            .iter()
            .filter(|b| b.listed)
            .map(|b| b.id)
            .collect()
    }

    /// Whether any window shows buffer `id`.
    pub fn is_shown(&self, id: BufferId) -> bool {
        self.window.buffer == id || self.windows.iter().any(|w| w.buffer == id)
    }

    /// `:bdelete` (or `:bwipeout` when `wipe`): unlist and unload a buffer. Windows showing it
    /// close, except the last, which shows another buffer instead.
    pub fn delete_buffer(&mut self, id: BufferId, wipe: bool, force: bool) -> Result<(), String> {
        let Some(buffer) = self.buffer(id) else {
            return Err(format!("E516: No buffers were deleted: bd {}", id.0));
        };
        if buffer.modified() && !force {
            return Err(format!(
                "E89: No write since last change for buffer {} (add ! to override)",
                id.0
            ));
        }
        // Close other windows showing it.
        let showing: Vec<WindowId> = self
            .layout
            .windows()
            .into_iter()
            .filter(|&w| self.window_ref(w).buffer == id)
            .collect();
        for w in showing {
            if self.layout.windows().len() > 1 {
                self.close_window(w);
            }
        }
        if self.window.buffer == id {
            let alt = self
                .window
                .alt_buffer
                .filter(|&a| a != id && self.buffer(a).is_some_and(|b| b.listed));
            let next = alt.or_else(|| {
                let listed = self.listed_buffers();
                let pos = listed.iter().position(|&b| b == id).unwrap_or(0);
                listed
                    .iter()
                    .skip(pos + 1)
                    .chain(listed.iter().take(pos))
                    .copied()
                    .find(|&b| b != id)
            });
            let next = next.unwrap_or_else(|| self.new_buffer());
            self.show_buffer(next);
        }
        for w in std::iter::once(&mut self.window).chain(self.windows.iter_mut()) {
            if w.alt_buffer == Some(id) && wipe {
                w.alt_buffer = None;
            }
            if wipe {
                w.jumps.remove_buffer(id);
            }
        }
        self.lsp_detach(id);
        if wipe {
            self.buffers.retain(|b| b.id != id);
            self.global_marks.retain(|_, (b, _)| *b != id);
        } else if let Some(b) = self.buffer_mut(id) {
            b.listed = false;
            b.unload();
        }
        Ok(())
    }

    /// Start at the top of a freshly loaded buffer. Like Vim's `:edit`, that position goes in
    /// the jumplist, and `'"` (last position in the file) starts at the top too.
    fn reset_view(&mut self) {
        self.window.cursor = Default::default();
        self.window.top = 0;
        self.window.pcmark = Some(Cursor::default());
        let buffer = self.window.buffer;
        self.window.jumps.push(Jump {
            buffer,
            pos: Cursor::default(),
        });
        self.current_buffer_mut().marks.set('"', Cursor::default());
    }

    /// Replace the current buffer's text, as if it had been loaded, and reset the view.
    pub fn set_text(&mut self, text: &str) {
        let buffer = self.current_buffer_mut();
        buffer.text = Text::new(text);
        buffer.history = Default::default();
        self.reset_view();
    }

    // ----- windows

    /// Window ids in Vim's order (top-left to bottom-right).
    pub fn window_ids(&self) -> Vec<WindowId> {
        self.layout.windows()
    }

    pub fn window_ref(&self, id: WindowId) -> &Window {
        if self.window.id == id {
            &self.window
        } else {
            self.windows
                .iter()
                .find(|w| w.id == id)
                .expect("window in the layout")
        }
    }

    pub fn window_mut(&mut self, id: WindowId) -> &mut Window {
        if self.window.id == id {
            &mut self.window
        } else {
            self.windows
                .iter_mut()
                .find(|w| w.id == id)
                .expect("window in the layout")
        }
    }

    /// Whether window `id`'s statusline joins the one to its right (see
    /// [`Layout::stl_connected`]).
    pub fn stl_connected(&self, id: WindowId) -> bool {
        self.layout.stl_connected(id)
    }

    /// Each window with its place on screen.
    pub fn window_rects(&self) -> Vec<(WindowId, Rect)> {
        self.layout.rects()
    }

    /// Make `id` the current window.
    pub fn goto_window(&mut self, id: WindowId) {
        if self.window.id == id {
            return;
        }
        let Some(i) = self.windows.iter().position(|w| w.id == id) else {
            return;
        };
        let next = self.windows.remove(i);
        let prev = std::mem::replace(&mut self.window, next);
        self.prev_window = Some(prev.id);
        self.windows.push(prev);
        self.enter_resize(WINHEIGHT, WINWIDTH);
    }

    /// Vim's `win_enter`: grow the window just entered to 'winheight' and 'winwidth'.
    fn enter_resize(&mut self, min_height: usize, min_width: usize) {
        let id = self.window.id;
        let Some(rect) = self.layout.rect(id) else {
            return;
        };
        if rect.height < min_height {
            self.layout.set_height(id, min_height);
        }
        if rect.width < min_width {
            self.layout.set_width(id, min_width);
        }
        self.sync_window_sizes();
    }

    /// `:split` / `:vsplit`: a new window above (or left of) the current one, showing the same
    /// buffer at the same place, becomes current. `size` is a count for its height or width.
    pub fn split(&mut self, vertical: bool, size: Option<usize>) -> bool {
        let after = if vertical {
            self.options.splitright
        } else {
            self.options.splitbelow
        };
        self.split_placed(vertical, size, Some(after))
    }

    /// Like [`Editor::split`], with the new window below / right of the current one when
    /// `after` is `Some(true)`, above / left of it with `Some(false)`, and with `None` at the
    /// bottom of the screen, full width (`:botright`).
    pub fn split_placed(
        &mut self,
        vertical: bool,
        size: Option<usize>,
        after: Option<bool>,
    ) -> bool {
        let id = WindowId(self.next_window);
        let fits = match after {
            Some(after) => self.layout.split(self.window.id, id, vertical, size, after),
            None => self.layout.split_bottom(id, size),
        };
        if !fits {
            self.error("E36: Not enough room");
            return false;
        }
        self.next_window += 1;
        let mut new = self.window.clone();
        new.id = id;
        let old = std::mem::replace(&mut self.window, new);
        self.prev_window = Some(old.id);
        self.quickfix.window_split(old.id, id);
        self.windows.push(old);
        // A count stands in for 'winheight' or 'winwidth' while entering the new window.
        let (min_height, min_width) = match size {
            Some(n) if vertical => (WINHEIGHT, n),
            Some(n) => (n, WINWIDTH),
            None => (WINHEIGHT, WINWIDTH),
        };
        self.enter_resize(min_height, min_width);
        true
    }

    /// Close window `id`, making the window that gets its space current if it was. False for
    /// the last window.
    pub fn close_window(&mut self, id: WindowId) -> bool {
        let Some(gets_space) = self.layout.close(id) else {
            return false;
        };
        let closed = if self.window.id == id {
            self.goto_window(gets_space);
            let i = self
                .windows
                .iter()
                .position(|w| w.id == id)
                .expect("closed window");
            self.windows.remove(i)
        } else {
            let i = self
                .windows
                .iter()
                .position(|w| w.id == id)
                .expect("closed window");
            self.windows.remove(i)
        };
        if let Some(b) = self.buffer_mut(closed.buffer) {
            b.window_closed(closed.id, closed.cursor);
        }
        if self.prev_window == Some(id) {
            self.prev_window = None;
        }
        self.sync_window_sizes();
        true
    }

    /// `:only`: close every other window.
    pub fn only_window(&mut self) {
        for w in std::mem::take(&mut self.windows) {
            if let Some(b) = self.buffer_mut(w.buffer) {
                b.window_closed(w.id, w.cursor);
            }
        }
        self.layout.only(self.window.id);
        self.prev_window = None;
        self.sync_window_sizes();
    }

    pub fn equalize_windows(&mut self) {
        self.layout.equalize(self.window.id, Dir::Both, false);
        self.sync_window_sizes();
    }

    /// Set window sizes from the layout, keeping each cursor at the same relative height.
    pub fn sync_window_sizes(&mut self) {
        for (id, rect) in self.layout.rects() {
            let win = if self.window.id == id {
                &mut self.window
            } else {
                match self.windows.iter_mut().find(|w| w.id == id) {
                    Some(w) => w,
                    None => continue,
                }
            };
            let Some(buffer) = self.buffers.iter().find(|b| b.id == win.buffer) else {
                continue;
            };
            let text = &buffer.text;
            let tabstop = buffer.opts.tabstop;
            let height = rect.height.max(1);
            // The sign and number columns take their share of the width.
            let numw = win.opts.number_width(text.line_count(), height)
                + sign_width(&win.opts.signcolumn, self.lsp.has_signs(buffer, &self.cwd));
            let width = rect.width.saturating_sub(numw).max(1);
            let old_width = win.width.max(1);
            win.set_height(&Metrics {
                text,
                tabstop,
                width: old_width,
                height,
            });
            win.set_width(&Metrics {
                text,
                tabstop,
                width,
                height,
            });
        }
    }

    /// Resize windows whose number column changed width (lines added past a power of ten,
    /// 'number' set, …).
    pub fn refresh_window_widths(&mut self) {
        let stale = self.layout.rects().into_iter().any(|(id, rect)| {
            let win = self.window_ref(id);
            let Some(buffer) = self.buffer(win.buffer) else {
                return false;
            };
            let numw = win
                .opts
                .number_width(buffer.text.line_count(), rect.height.max(1))
                + sign_width(&win.opts.signcolumn, self.lsp.has_signs(buffer, &self.cwd));
            win.width != rect.width.saturating_sub(numw).max(1)
        });
        if stale {
            self.sync_window_sizes();
        }
    }

    /// Buffer-local options of the current buffer.
    pub fn buf_opts(&self) -> &crate::options::BufferOptions {
        &self.current_buffer().opts
    }

    /// Whether a Visual selection is active (also while typing a search from Visual mode).
    pub fn visual_active(&self) -> bool {
        self.mode == Mode::Visual
            || (self.mode == Mode::CmdLine && self.cmdline_return == Mode::Visual)
    }

    pub fn screen_size(&self) -> (usize, usize) {
        (self.screen_width, self.screen_height)
    }

    /// Resize the screen: the layout gives or takes rows and columns from the bottom and
    /// rightmost windows, and each window keeps its cursor at the same relative height.
    pub fn resize(&mut self, width: usize, height: usize) {
        if (width, height) != (self.screen_width, self.screen_height) {
            // A resize repaints the whole screen, which clears the message line.
            self.message = None;
        }
        self.screen_width = width;
        self.screen_height = height;
        let rows = height.saturating_sub(CMDLINE_ROWS).max(2);
        self.layout.resize(width.max(1), rows);
        self.sync_window_sizes();
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
            tabstop: self.current_buffer().opts.tabstop,
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
        let exists = buffer
            .path
            .as_ref()
            .is_some_and(|p| self.cwd.join(p).exists());
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
        let id = self.window.buffer;
        let buffer = self
            .buffers
            .iter()
            .find(|b| b.id == id)
            .expect("buffer of the current window");
        let metrics = Metrics {
            text: &buffer.text,
            tabstop: buffer.opts.tabstop,
            width: self.window.width,
            height: self.window.height,
        };
        f(&mut self.window, &metrics)
    }

    /// Move other windows showing the current buffer for an edit, as Vim's `mark_adjust` does
    /// for their cursors and top lines.
    pub fn adjust_other_windows(&mut self, shift: &crate::LineShift) {
        let buffer = self.window.buffer;
        for w in self.windows.iter_mut().filter(|w| w.buffer == buffer) {
            if let Some(p) = shift.adjust(w.cursor) {
                w.cursor = p;
            }
            let top = Cursor {
                line: w.top,
                col: 0,
            };
            w.top = shift.adjust(top).map_or(w.top, |p| p.line);
        }
        for w in std::iter::once(&mut self.window).chain(self.windows.iter_mut()) {
            w.jumps.adjust(buffer, shift);
        }
        for (b, p) in self.global_marks.values_mut() {
            if *b == buffer
                && let Some(q) = shift.adjust(*p)
            {
                *p = q;
            }
        }
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.show(text.into(), MessageKind::Info);
    }

    /// Vim's `msgmore` (`3 fewer lines`): not shown over a message the current command
    /// asked to keep (a search's `/pattern  [1/5]`).
    pub fn more_info(&mut self, text: impl Into<String>) {
        if !self.keep_msg {
            self.info(text);
        }
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
        let wraps = matches!(
            kind,
            MessageKind::Full | MessageKind::Error | MessageKind::Warning
        ) && unicode_width::UnicodeWidthStr::width(text.as_str())
            >= self.screen_width.max(1);
        // Anything longer than one line waits for a key.
        self.hit_enter = text.contains('\n') || wraps;
        self.message = Some(Message { text, kind });
        self.kept_message = None;
        self.message_positions.clear();
        self.message_highlights.clear();
        self.more_max_row = 0;
        self.more_restore_top = None;
        self.fresh_message_base = None;
        // More than a screenful is shown a page at a time ('more').
        self.more_top =
            (self.hit_enter && self.message_lines().len() >= self.screen_height).then_some(0);
    }

    /// The message line shown on screen row `row` of the message (rows count wrapped lines).
    pub fn message_line_at(&self, row: usize) -> usize {
        let width = self.screen_width.max(1);
        let mut seen = 0;
        let text = self.message.as_ref().map_or("", |m| m.text.as_str());
        for (i, l) in text.lines().enumerate() {
            seen += wrap(l, width).len();
            if row < seen {
                return i;
            }
        }
        text.lines().count().saturating_sub(1)
    }

    /// The message split into screen rows.
    pub fn message_lines(&self) -> Vec<String> {
        let width = self.screen_width.max(1);
        self.message
            .as_ref()
            .map(|m| m.text.lines().flat_map(|l| wrap(l, width)).collect())
            .unwrap_or_default()
    }

    /// An error message. One that doesn't fit on the command line wraps, and waits for a key
    /// like any multi-line message.
    pub fn error(&mut self, text: impl Into<String>) {
        self.error_count += 1;
        self.show(text.into(), MessageKind::Error);
    }

    /// A warning: like an error, in the warning color, without counting as an error.
    pub fn warning(&mut self, text: impl Into<String>) {
        self.show(text.into(), MessageKind::Warning);
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
        if name == Some('/') {
            let pat = self.search.last_pattern()?;
            return Some(crate::Register::new(
                pat.pat.clone(),
                crate::RegisterKind::Char,
            ));
        }
        self.registers.get(name).cloned()
    }
}

/// Split `line` into pieces at most `width` cells wide.
pub fn wrap(line: &str, width: usize) -> Vec<String> {
    use unicode_segmentation::UnicodeSegmentation;
    let mut out = vec![String::new()];
    let mut used = 0;
    for g in line.graphemes(true) {
        let w = unicode_width::UnicodeWidthStr::width(g);
        if used + w > width {
            out.push(String::new());
            used = 0;
        }
        out.last_mut().unwrap().push_str(g);
        used += w;
    }
    out
}
