//! Quickfix and location lists, as in Vim's `quickfix.c`: lists of places in files, shown in a
//! window of their own (`:copen`, `:lopen`) and gone through with `:cnext` and friends. Each
//! window can have a location list; the quickfix list is global. Also the tag stack
//! (`CTRL-]`, `CTRL-T`, `:tags`), which Neovim's LSP 'tagfunc' feeds.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::{Buffer, BufferId, Cursor, Editor, Jump, WindowId};

/// The height of a new quickfix window (Vim's `QF_WINHEIGHT`).
pub const QF_WINHEIGHT: usize = 10;
/// How many lists a stack keeps (Vim's `LISTCOUNT`).
const LISTCOUNT: usize = 10;
/// How many tags a window's tag stack keeps (Vim's `TAGSTACKSIZE`).
const TAGSTACKSIZE: usize = 20;

pub const NO_ERRORS: &str = "E42: No Errors";
pub const NO_LOCATION_LIST: &str = "E776: No location list";
const NO_MORE_ITEMS: &str = "E553: No more items";

/// Which kind of list a quickfix buffer shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    Quickfix,
    Location,
}

/// An entry of a list (Vim's `qfline_T`). Line and column numbers count from 1, 0 meaning
/// none; columns are byte columns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entry {
    /// The file, absolute.
    pub path: Option<PathBuf>,
    pub lnum: usize,
    pub end_lnum: usize,
    pub col: usize,
    pub end_col: usize,
    pub text: String,
    /// `E`, `W`, `I`, `N`, … (Vim's `type`).
    pub kind: Option<char>,
    pub nr: i64,
}

/// A quickfix or location list.
#[derive(Debug, Clone, Default)]
pub struct List {
    /// Shown in the window's statusline (`w:quickfix_title`).
    pub title: String,
    pub entries: Vec<Entry>,
    /// The current entry, from 0.
    pub idx: usize,
}

/// A stack of lists (`:colder`, `:cnewer`), and the buffer showing the current one.
#[derive(Debug, Clone, Default)]
pub struct Stack {
    pub lists: Vec<List>,
    pub cur: usize,
    pub buffer: Option<BufferId>,
}

impl Stack {
    pub fn current(&self) -> Option<&List> {
        self.lists.get(self.cur)
    }

    fn current_mut(&mut self) -> Option<&mut List> {
        self.lists.get_mut(self.cur)
    }

    /// Vim's `qf_new_list`: the list goes after the current one, replacing newer ones; the
    /// oldest goes when there are too many.
    fn push(&mut self, list: List) {
        if !self.lists.is_empty() {
            self.lists.truncate(self.cur + 1);
        }
        if self.lists.len() == LISTCOUNT {
            self.lists.remove(0);
        }
        self.lists.push(list);
        self.cur = self.lists.len() - 1;
    }
}

/// A list: the quickfix list or a location list (by its stack).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    Quickfix,
    Location(usize),
}

/// Where `:cnext` and friends go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JumpTo {
    /// `:cc [nr]`, `:cfirst`, `:clast`: entry `nr` (from 1; past the end is the last one), or
    /// the current one for 0.
    Nr(usize),
    /// `:cnext`, `:cprevious`: `n` entries on.
    Next(usize),
    Prev(usize),
    /// `:cnfile`, `:cpfile`: the first entry `n` files on, the last one `n` files back.
    NextFile(usize),
    PrevFile(usize),
}

/// A tag jumped to: where from, and the places it matched.
#[derive(Debug, Clone)]
pub struct Tag {
    pub name: String,
    pub from: (BufferId, Cursor),
    /// The match jumped to, from 0.
    pub cur_match: usize,
}

/// A window's tag stack (Vim's `w_tagstack`): `idx` is where the next tag goes.
#[derive(Debug, Clone, Default)]
pub struct TagStack {
    pub tags: Vec<Tag>,
    pub idx: usize,
}

#[derive(Debug, Clone, Default)]
pub struct QuickfixState {
    pub quickfix: Stack,
    loc: HashMap<usize, Stack>,
    next_loc: usize,
    /// The location list of each window that has one (Vim's `w_llist`). Windows may share one.
    owner: HashMap<WindowId, usize>,
    /// Location list windows and the list each shows (`w_llist_ref`).
    shown_in: HashMap<WindowId, usize>,
    pub tags: HashMap<WindowId, TagStack>,
}

impl QuickfixState {
    pub fn stack(&self, which: Which) -> Option<&Stack> {
        match which {
            Which::Quickfix => Some(&self.quickfix),
            Which::Location(id) => self.loc.get(&id),
        }
    }

    fn stack_mut(&mut self, which: Which) -> Option<&mut Stack> {
        match which {
            Which::Quickfix => Some(&mut self.quickfix),
            Which::Location(id) => self.loc.get_mut(&id),
        }
    }

    /// The location list window `win` uses: the one it shows if it's a location list window,
    /// else its own.
    fn loc_of(&self, win: WindowId) -> Option<usize> {
        self.shown_in
            .get(&win)
            .or_else(|| self.owner.get(&win))
            .copied()
    }

    /// A window was split from `old`: the new one gets a copy of its location list and its
    /// tag stack, as in Vim's `win_init`.
    pub fn window_split(&mut self, old: WindowId, new: WindowId) {
        if let Some(id) = self.loc_of(old)
            && let Some(stack) = self.loc.get(&id)
        {
            let copy = Stack {
                buffer: None,
                ..stack.clone()
            };
            let n = self.next_loc;
            self.next_loc += 1;
            self.loc.insert(n, copy);
            self.owner.insert(new, n);
        }
        if let Some(tags) = self.tags.get(&old).cloned() {
            self.tags.insert(new, tags);
        }
    }
}

/// Vim's `qf_types`: ` error`, ` warning  12`, ….
fn types(kind: Option<char>, nr: i64) -> String {
    let p = match kind {
        Some('W' | 'w') => " warning".to_string(),
        Some('I' | 'i') => " info".to_string(),
        Some('N' | 'n') => " note".to_string(),
        Some('E' | 'e') => " error".to_string(),
        None if nr > 0 => " error".to_string(),
        None => String::new(),
        Some(c) => format!(" {c}"),
    };
    if nr <= 0 { p } else { format!("{p} {nr:3}") }
}

/// Vim's `qf_fmt_text`: newlines as a space, dropping the white space after them.
fn fmt_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\n' {
            out.push(' ');
            while chars
                .next_if(|&c| c == ' ' || c == '\t' || c == '\n')
                .is_some()
            {}
        } else {
            out.push(c);
        }
    }
    out
}

fn skipwhite(s: &str) -> &str {
    s.trim_start_matches([' ', '\t'])
}

/// How a file is named in a list: relative to `cwd` when it's inside it (Vim's
/// `shorten_buf_fname`).
pub fn short_name(cwd: &Path, path: &Path) -> PathBuf {
    match path.strip_prefix(cwd) {
        Ok(rel) if !rel.as_os_str().is_empty() => rel.to_path_buf(),
        _ => path.to_path_buf(),
    }
}

/// An entry's line in the quickfix window (Vim's `qf_buf_add_line`):
/// `file|12 col 5-9 error| text`.
pub fn entry_line(cwd: &Path, e: &Entry) -> String {
    let mut s = String::new();
    if let Some(p) = &e.path {
        s.push_str(&short_name(cwd, p).display().to_string());
    }
    s.push('|');
    if e.lnum > 0 {
        s.push_str(&e.lnum.to_string());
        if e.end_lnum > 0 && e.end_lnum != e.lnum {
            s.push_str(&format!("-{}", e.end_lnum));
        }
        if e.col > 0 {
            s.push_str(&format!(" col {}", e.col));
            if e.end_col > 0 && e.end_col != e.col {
                s.push_str(&format!("-{}", e.end_col));
            }
        }
        s.push_str(&types(e.kind, e.nr));
    }
    s.push_str("| ");
    let text = if e.path.is_some() || e.lnum != 0 {
        skipwhite(&e.text)
    } else {
        &e.text
    };
    s.push_str(&fmt_text(text));
    s
}

/// The highlights of a quickfix window line, from Vim's `syntax/qf.vim`: char ranges and their
/// groups. `qfText` is Normal and isn't listed; neither are `warning`, `note` and `info`, whose
/// groups have no highlight.
pub fn line_highlights(line: &str) -> Vec<(usize, usize, &'static str)> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let Some(sep1) = chars.iter().position(|&c| c == '|') else {
        out.push((0, chars.len(), "Directory"));
        return out;
    };
    out.push((0, sep1, "Directory"));
    out.push((sep1, sep1 + 1, "Delimiter"));
    let start = sep1 + 1;
    let end = chars[start..]
        .iter()
        .position(|&c| c == '|')
        .map_or(chars.len(), |i| start + i);
    // `qfLineNr` contains `@qfType`: the first of these words found at each place.
    let words = ["error", "warning", "note", "info"];
    let mut at = start;
    let mut from = start;
    while at < end {
        let word = words.iter().find(|w| {
            let n = w.len();
            at + n <= end && chars[at..at + n].iter().copied().eq(w.chars())
        });
        match word {
            Some(w) => {
                if from < at {
                    out.push((from, at, "LineNr"));
                }
                if *w == "error" {
                    out.push((at, at + w.len(), "Error"));
                }
                at += w.len();
                from = at;
            }
            None => at += 1,
        }
    }
    if from < end {
        out.push((from, end, "LineNr"));
    }
    if end < chars.len() {
        out.push((end, end + 1, "Delimiter"));
    }
    out
}

/// A byte column of `line` as a char column (the char it's in).
fn byte_to_col(line: &str, byte: usize) -> usize {
    line.char_indices()
        .take_while(|&(i, _)| i <= byte)
        .count()
        .saturating_sub(1)
}

impl Editor {
    /// The list `:l…` commands (with `loc`) or `:c…` commands use from the current window.
    pub fn qf_which(&self, loc: bool) -> Result<Which, String> {
        if !loc {
            return Ok(Which::Quickfix);
        }
        self.quickfix
            .loc_of(self.window.id)
            .map(Which::Location)
            .ok_or_else(|| NO_LOCATION_LIST.to_string())
    }

    /// `setqflist([], ' ', {title, items})` (or `setloclist(0, …)` with `loc`): a new list.
    /// A window showing the list shows the new one.
    pub fn qf_set_list(&mut self, loc: bool, title: &str, entries: Vec<Entry>) {
        let which = if loc {
            let win = self.window.id;
            let id = match self.quickfix.loc_of(win) {
                Some(id) => id,
                None => {
                    let id = self.quickfix.next_loc;
                    self.quickfix.next_loc += 1;
                    self.quickfix.loc.insert(id, Stack::default());
                    self.quickfix.owner.insert(win, id);
                    id
                }
            };
            Which::Location(id)
        } else {
            Which::Quickfix
        };
        if let Some(stack) = self.quickfix.stack_mut(which) {
            stack.push(List {
                title: title.to_string(),
                entries,
                idx: 0,
            });
        }
        self.qf_fill_buffer(which);
    }

    /// The windows showing list `which`.
    fn qf_windows(&self, which: Which) -> Vec<WindowId> {
        let buffer = self.quickfix.stack(which).and_then(|s| s.buffer);
        self.window_ids()
            .into_iter()
            .filter(|&w| match which {
                Which::Quickfix => Some(self.window_ref(w).buffer) == buffer,
                Which::Location(id) => self.quickfix.shown_in.get(&w) == Some(&id),
            })
            .collect()
    }

    /// Put the current list's lines in the buffer showing `which`, if there is one.
    fn qf_fill_buffer(&mut self, which: Which) {
        let Some(stack) = self.quickfix.stack(which) else {
            return;
        };
        let Some(buffer) = stack.buffer else {
            return;
        };
        let entries = stack
            .current()
            .map(|l| l.entries.clone())
            .unwrap_or_default();
        // Buffers of the files listed are named relative to the working directory.
        for e in &entries {
            let Some(p) = &e.path else { continue };
            let short = short_name(&self.cwd, p);
            if short.is_relative()
                && let Some(id) = self.find_buffer(p)
                && let Some(b) = self.buffer_mut(id)
                && b.path.as_ref().is_some_and(|bp| bp.is_absolute())
            {
                b.path = Some(short);
            }
        }
        let mut text = String::new();
        for e in &entries {
            text.push_str(&entry_line(&self.cwd, e));
            text.push('\n');
        }
        if let Some(b) = self.buffer_mut(buffer) {
            b.text = flux_core::Text::new(&text);
            b.history = Default::default();
        }
        // Cursors stay on their line, if it's still there.
        let last = entries.len().saturating_sub(1);
        for w in self.qf_windows(which) {
            let win = self.window_mut(w);
            win.cursor.line = win.cursor.line.min(last);
            win.cursor.col = 0;
        }
    }

    /// The quickfix buffer for `which`, made if needed.
    fn qf_buffer(&mut self, which: Which) -> BufferId {
        if let Some(b) = self.quickfix.stack(which).and_then(|s| s.buffer)
            && self.buffer(b).is_some()
        {
            return b;
        }
        let mut buffer = Buffer::scratch(BufferId(0));
        buffer.listed = false;
        buffer.quickfix = Some(match which {
            Which::Quickfix => ListKind::Quickfix,
            Which::Location(_) => ListKind::Location,
        });
        let id = self.add_buffer_hidden(buffer);
        if let Some(b) = self.buffer_mut(id) {
            b.opts.filetype = "qf".into();
        }
        if let Some(s) = self.quickfix.stack_mut(which) {
            s.buffer = Some(id);
        }
        id
    }

    /// `:copen [height]` (`:lopen` with `loc`): go to the window showing the list, or open
    /// one, `height` rows high (10 by default): at the bottom of the screen with `botright`
    /// (what `vim.lsp.buf.references()` does), else below the last window, or for a location
    /// list below the current window. The cursor goes to the current entry.
    pub fn qf_open(
        &mut self,
        loc: bool,
        height: Option<usize>,
        botright: bool,
    ) -> Result<(), String> {
        let which = self.qf_which(loc)?;
        let from = self.window.id;
        match self.qf_windows(which).first() {
            Some(&w) => {
                self.goto_window(w);
                if let Some(h) = height {
                    self.layout.set_height(w, h);
                    self.sync_window_sizes();
                }
            }
            None => {
                let size = height.unwrap_or(QF_WINHEIGHT);
                let placed = if botright {
                    self.split_placed(false, Some(size), None)
                } else {
                    if !loc && let Some(&last) = self.window_ids().last() {
                        self.goto_window(last);
                    }
                    self.split_placed(false, Some(size), Some(true))
                };
                if !placed {
                    return Ok(());
                }
                let win = self.window.id;
                // The new window doesn't get a copy of the location list: it shows it.
                if let Some(id) = self.quickfix.owner.remove(&win) {
                    let shared = self.quickfix.owner.values().any(|&o| o == id)
                        || self.quickfix.shown_in.values().any(|&o| o == id);
                    if !shared {
                        self.quickfix.loc.remove(&id);
                    }
                }
                if let Which::Location(id) = which {
                    self.quickfix.shown_in.insert(win, id);
                }
                let buffer = self.qf_buffer(which);
                self.window.alt_buffer = Some(self.window.buffer);
                self.window.buffer = buffer;
                self.window.top = 0;
                self.window.cursor = Cursor::default();
                if let Some(b) = self.buffer_mut(buffer) {
                    b.created_in(win);
                }
                // Full width: the height is set again, in case making room changed it.
                let full = self.layout.rect(win).map(|r| r.width) == Some(self.screen_size().0);
                if full {
                    self.layout.set_height(win, size);
                    self.sync_window_sizes();
                }
                self.prev_window = Some(from);
            }
        }
        self.qf_fill_buffer(which);
        let idx = self
            .quickfix
            .stack(which)
            .and_then(|s| s.current())
            .map_or(0, |l| l.idx);
        let last = self.text().last_line();
        self.window.cursor = Cursor {
            line: idx.min(last),
            col: 0,
        };
        self.window.set_curswant = true;
        self.with_window(|w, m| w.scroll_to_cursor(m));
        Ok(())
    }

    /// `:cclose` (`:lclose`): close the window showing the list.
    pub fn qf_close(&mut self, loc: bool) -> Result<(), String> {
        let which = match self.qf_which(loc) {
            Ok(w) => w,
            // `:lclose` in a window without a location list does nothing.
            Err(_) => return Ok(()),
        };
        if let Some(&w) = self.qf_windows(which).first()
            && self.close_window(w)
        {
            self.quickfix.shown_in.remove(&w);
        }
        Ok(())
    }

    /// `:cwindow` (`:lwindow`): open the window if the list has entries, close it if not.
    pub fn qf_window(&mut self, loc: bool, height: Option<usize>) -> Result<(), String> {
        // `:lwindow` in a window without a location list does nothing.
        let Ok(which) = self.qf_which(loc) else {
            return Ok(());
        };
        let empty = self
            .quickfix
            .stack(which)
            .and_then(|s| s.current())
            .is_none_or(|l| l.entries.is_empty());
        let open = !self.qf_windows(which).is_empty();
        if empty {
            if open {
                self.qf_close(loc)?;
            }
            Ok(())
        } else if !open {
            self.qf_open(loc, height, false)
        } else {
            Ok(())
        }
    }

    /// The list the current window shows, if it's a quickfix window.
    pub fn qf_shown_here(&self) -> Option<Which> {
        let kind = self.current_buffer().quickfix?;
        Some(match kind {
            ListKind::Quickfix => Which::Quickfix,
            ListKind::Location => Which::Location(*self.quickfix.shown_in.get(&self.window.id)?),
        })
    }

    /// The current list of a quickfix buffer: its title, and its current entry's line.
    pub fn qf_list_of(&self, buffer: BufferId) -> Option<&List> {
        let b = self.buffer(buffer)?;
        let stack = match b.quickfix? {
            ListKind::Quickfix => &self.quickfix.quickfix,
            ListKind::Location => self
                .quickfix
                .loc
                .values()
                .find(|s| s.buffer == Some(buffer))?,
        };
        stack.current()
    }

    /// `:cc`, `:cnext`, … (`:ll`, `:lnext`, … with `loc`): go to an entry. From a quickfix
    /// window that's in another window: Vim's `qf_jump_to_usable_window`.
    pub fn qf_jump(&mut self, loc: bool, to: JumpTo) -> Result<(), String> {
        let which = self.qf_which(loc)?;
        let Some(list) = self.quickfix.stack(which).and_then(|s| s.current()) else {
            return Err(NO_ERRORS.into());
        };
        if list.entries.is_empty() {
            return Err(NO_ERRORS.into());
        }
        let n = list.entries.len();
        let old = list.idx;
        let path_of = |i: usize| list.entries[i].path.clone();
        let idx = match to {
            JumpTo::Nr(0) => old,
            JumpTo::Nr(nr) => nr.min(n) - 1,
            JumpTo::Next(count) | JumpTo::Prev(count) => {
                let forward = matches!(to, JumpTo::Next(_));
                let mut i = old;
                for step in 0..count.max(1) {
                    let next = if forward {
                        (i + 1 < n).then_some(i + 1)
                    } else {
                        i.checked_sub(1)
                    };
                    match next {
                        Some(j) => i = j,
                        None if step == 0 => return Err(NO_MORE_ITEMS.into()),
                        None => break,
                    }
                }
                i
            }
            JumpTo::NextFile(count) => {
                let mut i = old;
                for step in 0..count.max(1) {
                    let file = path_of(i);
                    match (i + 1..n).find(|&j| path_of(j) != file) {
                        Some(j) => i = j,
                        None if step == 0 => return Err(NO_MORE_ITEMS.into()),
                        None => break,
                    }
                }
                i
            }
            JumpTo::PrevFile(count) => {
                let mut i = old;
                for step in 0..count.max(1) {
                    let file = path_of(i);
                    match (0..i).rev().find(|&j| path_of(j) != file) {
                        Some(j) => i = j,
                        None if step == 0 => return Err(NO_MORE_ITEMS.into()),
                        None => break,
                    }
                }
                i
            }
        };
        let entry = list.entries[idx].clone();
        if let Some(l) = self.quickfix.stack_mut(which).and_then(Stack::current_mut) {
            l.idx = idx;
        }
        // A window showing the list follows it, and then there's no message.
        let shown = self.qf_windows(which);
        for &w in &shown {
            let win = self.window_mut(w);
            win.cursor = Cursor { line: idx, col: 0 };
            win.set_curswant = true;
        }
        if self.current_buffer().quickfix.is_some() {
            self.qf_usable_window(entry.path.as_deref());
        }
        if let Some(path) = &entry.path {
            self.qf_edit(path)?;
        } else {
            self.set_pcmark();
        }
        self.qf_goto_line(&entry);
        if shown.is_empty() {
            self.info(format!(
                "({} of {n}){}: {}",
                idx + 1,
                types(entry.kind, entry.nr),
                fmt_text(skipwhite(&entry.text))
            ));
        }
        for w in shown {
            if w == self.window.id {
                self.with_window(|win, m| win.scroll_to_cursor(m));
            } else {
                let cur = self.window.id;
                self.goto_window_quietly(w);
                self.with_window(|win, m| win.scroll_to_cursor(m));
                self.goto_window_quietly(cur);
            }
        }
        Ok(())
    }

    /// `<CR>` in a quickfix window: go to the entry under the cursor (`:.cc`, `:.ll`).
    pub fn qf_enter(&mut self) -> Result<(), String> {
        let Some(which) = self.qf_shown_here() else {
            return Ok(());
        };
        let empty = self
            .quickfix
            .stack(which)
            .and_then(|s| s.current())
            .is_none_or(|l| l.entries.is_empty());
        if empty {
            return Err(NO_ERRORS.into());
        }
        let line = self.cursor().line + 1;
        self.qf_jump(matches!(which, Which::Location(_)), JumpTo::Nr(line))
    }

    /// Switch to `id` without the side effects of entering a window (sizes, previous window).
    fn goto_window_quietly(&mut self, id: WindowId) {
        if self.window.id == id {
            return;
        }
        let Some(i) = self.windows.iter().position(|w| w.id == id) else {
            return;
        };
        let next = self.windows.remove(i);
        let prev = std::mem::replace(&mut self.window, next);
        self.windows.push(prev);
    }

    /// Vim's `setpcmark`: remember the cursor in the jumplist.
    fn set_pcmark(&mut self) {
        let pos = self.cursor();
        let buffer = self.window.buffer;
        self.window.pcmark = Some(pos);
        self.window.jumps.push(Jump { buffer, pos });
    }

    /// Show the file `path` in the current window (as `:edit` would, remembering the jump), or
    /// remember the jump if it's already shown.
    fn qf_edit(&mut self, path: &Path) -> Result<(), String> {
        let same = self
            .current_buffer()
            .path
            .as_ref()
            .is_some_and(|p| self.resolve(p) == self.resolve(path));
        if same {
            self.set_pcmark();
            return Ok(());
        }
        let name = short_name(&self.cwd, path);
        self.edit_file(&name)?;
        if let Some(b) = self.buffer_mut(self.window.buffer) {
            b.listed = true;
        }
        Ok(())
    }

    /// Vim's `qf_jump_goto_line`: the entry's line and column, or the line's first non-blank.
    fn qf_goto_line(&mut self, e: &Entry) {
        let text = self.text();
        let line = if e.lnum > 0 {
            (e.lnum - 1).min(text.last_line())
        } else {
            self.cursor().line
        };
        let s = text.line_str(line);
        let len = text.line_len(line);
        let col = if e.col > 0 {
            byte_to_col(&s, e.col - 1).min(len.saturating_sub(1))
        } else {
            s.chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .count()
                .min(len.saturating_sub(1))
        };
        self.window.cursor = Cursor { line, col };
        self.window.set_curswant = true;
        self.with_window(|w, m| w.scroll_to_cursor(m));
    }

    /// Whether window `w` shows a normal buffer (not a list or a directory listing).
    fn normal_window(&self, w: WindowId) -> bool {
        self.buffer(self.window_ref(w).buffer)
            .is_some_and(|b| b.quickfix.is_none() && !b.directory)
    }

    fn shows_file(&self, w: WindowId, path: Option<&Path>) -> bool {
        let Some(path) = path else {
            return false;
        };
        self.buffer(self.window_ref(w).buffer)
            .and_then(|b| b.path.as_ref())
            .is_some_and(|p| self.resolve(p) == self.resolve(path))
    }

    /// Vim's `qf_jump_to_usable_window`: from a quickfix window, go to the window to show
    /// an entry (in file `path`) in.
    fn qf_usable_window(&mut self, path: Option<&Path>) {
        let ids = self.window_ids();
        let cur = self.window.id;
        // In a location list window, the list it shows (whichever list is jumped in).
        let ll_ref = self.quickfix.shown_in.get(&cur).copied();
        // A window with this location list, or any window with a normal buffer.
        let usable_wp = ll_ref.and_then(|id| {
            ids.iter().copied().find(|w| {
                self.quickfix.owner.get(w) == Some(&id) && !self.quickfix.shown_in.contains_key(w)
            })
        });
        let usable = usable_wp.is_some() || ids.iter().any(|&w| self.normal_window(w));
        if ids.len() == 1 || !usable {
            // A new window above the quickfix window.
            if self.split_placed(false, None, Some(false)) {
                let new = self.window.id;
                self.quickfix.shown_in.remove(&new);
                match ll_ref {
                    Some(id) => {
                        self.quickfix.owner.insert(new, id);
                    }
                    None => {
                        self.quickfix.owner.remove(&new);
                    }
                }
            }
            return;
        }
        let pos = |w: WindowId| ids.iter().position(|&x| x == w).unwrap_or(0);
        let prev = |i: usize| if i == 0 { ids.len() - 1 } else { i - 1 };
        let target = if let Some(id) = ll_ref {
            // Vim's `qf_goto_win_with_ll_file`.
            let w = usable_wp
                .or_else(|| ids.iter().copied().find(|&w| self.shows_file(w, path)))
                .unwrap_or_else(|| {
                    let mut i = pos(cur);
                    loop {
                        if self.normal_window(ids[i]) {
                            break ids[i];
                        }
                        i = prev(i);
                        if ids[i] == cur {
                            break cur;
                        }
                    }
                });
            self.quickfix.owner.entry(w).or_insert(id);
            w
        } else {
            // Vim's `qf_goto_win_with_qfl_file`: back from the quickfix window to one showing
            // the file; failing that, the last window used ('switchbuf' `uselast`).
            let mut i = pos(cur);
            let mut altwin = None;
            loop {
                if self.shows_file(ids[i], path) {
                    break ids[i];
                }
                i = prev(i);
                let w = ids[i];
                let is_qf_window = self
                    .buffer(self.window_ref(w).buffer)
                    .is_some_and(|b| b.quickfix == Some(ListKind::Quickfix));
                if is_qf_window || w == cur {
                    let c = pos(cur);
                    break match self.prev_window.filter(|p| self.layout.contains(*p)) {
                        Some(p) => p,
                        None => altwin.unwrap_or_else(|| {
                            if c > 0 {
                                ids[c - 1]
                            } else {
                                ids[(c + 1) % ids.len()]
                            }
                        }),
                    };
                }
                if altwin.is_none() && self.normal_window(w) {
                    altwin = Some(w);
                }
            }
        };
        self.goto_window(target);
    }

    // ----- the tag stack

    /// A tag jump (`CTRL-]` with Neovim's LSP 'tagfunc'): push `name` and where the cursor is
    /// on the tag stack, then go to `path` at `line`, byte column `byte` (from 0).
    pub fn tag_jump(
        &mut self,
        name: &str,
        from: (BufferId, Cursor),
        path: &Path,
        line: usize,
        byte: usize,
    ) -> Result<(), String> {
        let win = self.window.id;
        let stack = self.quickfix.tags.entry(win).or_default();
        stack.tags.truncate(stack.idx);
        stack.tags.push(Tag {
            name: name.to_string(),
            from,
            cur_match: 0,
        });
        if stack.tags.len() > TAGSTACKSIZE {
            stack.tags.remove(0);
        }
        stack.idx = stack.tags.len();
        self.goto_file_pos(path, line, byte)
    }

    /// Show `path` (named as given unless already open) in the current window with the cursor
    /// at `line`, byte column `byte`, remembering the jump (Vim's `getfile` with `setpm`).
    pub fn goto_file_pos(&mut self, path: &Path, line: usize, byte: usize) -> Result<(), String> {
        let same = self
            .current_buffer()
            .path
            .as_ref()
            .is_some_and(|p| self.resolve(p) == self.resolve(path));
        if same {
            self.set_pcmark();
        } else {
            let name = match self.find_buffer(path) {
                Some(id) => self
                    .buffer(id)
                    .and_then(|b| b.path.clone())
                    .unwrap_or_else(|| path.to_path_buf()),
                None => path.to_path_buf(),
            };
            self.edit_file(&name)?;
            if let Some(b) = self.buffer_mut(self.window.buffer) {
                b.listed = true;
            }
        }
        let text = self.text();
        let line = line.min(text.last_line());
        let len = text.line_len(line);
        let col = byte_to_col(&text.line_str(line), byte).min(len.saturating_sub(1));
        self.window.cursor = Cursor { line, col };
        self.window.set_curswant = true;
        self.with_window(|w, m| w.scroll_to_cursor(m));
        Ok(())
    }

    /// `CTRL-T`, `:pop`: back `count` tags on the tag stack.
    pub fn tag_pop(&mut self, count: usize) -> Result<(), String> {
        let win = self.window.id;
        let stack = self.quickfix.tags.entry(win).or_default();
        if stack.tags.is_empty() {
            return Err("E73: Tag stack empty".into());
        }
        let mut error = None;
        let idx = match stack.idx.checked_sub(count.max(1)) {
            Some(i) => i,
            None if stack.idx == 0 => return Err("E555: At bottom of tag stack".into()),
            None => {
                error = Some("E555: At bottom of tag stack".to_string());
                0
            }
        };
        stack.idx = idx;
        let (buffer, pos) = stack.tags[idx].from;
        if buffer != self.window.buffer {
            if self.buffer(buffer).is_none() {
                return Err(error.unwrap_or_default());
            }
            self.show_buffer(buffer);
        } else {
            self.set_pcmark();
        }
        let text = self.text();
        let line = pos.line.min(text.last_line());
        let col = pos.col.min(text.line_len(line).saturating_sub(1));
        self.window.cursor = Cursor { line, col };
        self.window.set_curswant = true;
        self.with_window(|w, m| w.scroll_to_cursor(m));
        match error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// `:tags`: the lines listing the tag stack, and for those showing a line of the current
    /// file (in Directory), the char range of that text.
    pub fn tag_lines(&self) -> Vec<(String, Option<(usize, usize)>)> {
        let mut lines = vec![(
            "  # TO tag         FROM line  in file/text".to_string(),
            None,
        )];
        let Some(stack) = self.quickfix.tags.get(&self.window.id) else {
            lines.push((">".into(), None));
            return lines;
        };
        let width = self.screen_size().0;
        for (i, t) in stack.tags.iter().enumerate() {
            let (buffer, pos) = t.from;
            let prefix = format!(
                "{}{:2} {:2} {:<15} {:5}  ",
                if i == stack.idx { '>' } else { ' ' },
                i + 1,
                t.cur_match + 1,
                t.name,
                pos.line + 1,
            );
            let start = prefix.chars().count();
            if buffer == self.window.buffer {
                // The line's text, cut to fit (Vim's `mark_line`).
                let text = self.text();
                let shown = if pos.line > text.last_line() {
                    "-invalid-".to_string()
                } else {
                    let line = text.line_str(pos.line);
                    let mut used = 0;
                    let mut out = String::new();
                    for c in skipwhite(&line).chars() {
                        used += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
                        if used >= width.saturating_sub(30) {
                            break;
                        }
                        out.push(c);
                    }
                    out
                };
                let end = start + shown.chars().count();
                lines.push((format!("{prefix}{shown}"), Some((start, end))));
            } else {
                let name = self.buffer(buffer).map(|b| b.name()).unwrap_or_default();
                lines.push((format!("{prefix}{name}"), None));
            }
        }
        if stack.idx == stack.tags.len() {
            lines.push((">".into(), None));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, lnum: usize, col: usize, text: &str) -> Entry {
        Entry {
            path: Some(PathBuf::from(path)),
            lnum,
            col,
            text: text.into(),
            ..Default::default()
        }
    }

    #[test]
    fn entry_lines_look_like_vims() {
        let cwd = Path::new("/w");
        let mut e = entry("/w/a.rs", 3, 5, "  let x\n   = 1;");
        e.end_col = 9;
        assert_eq!(entry_line(cwd, &e), "a.rs|3 col 5-9| let x = 1;");
        let mut e = entry("/x/b.py", 7, 0, "three");
        e.end_lnum = 9;
        e.kind = Some('E');
        e.nr = 12;
        assert_eq!(entry_line(cwd, &e), "/x/b.py|7-9 error  12| three");
        let e = Entry {
            text: "  no file".into(),
            ..Default::default()
        };
        assert_eq!(entry_line(cwd, &e), "||   no file");
    }

    #[test]
    fn highlights_follow_qf_syntax() {
        assert_eq!(
            line_highlights("a.rs|7-9 error  12| x"),
            vec![
                (0, 4, "Directory"),
                (4, 5, "Delimiter"),
                (5, 9, "LineNr"),
                (9, 14, "Error"),
                (14, 18, "LineNr"),
                (18, 19, "Delimiter"),
            ]
        );
        assert_eq!(
            line_highlights("a|1 warning 2| t"),
            vec![
                (0, 1, "Directory"),
                (1, 2, "Delimiter"),
                (2, 4, "LineNr"),
                (11, 13, "LineNr"),
                (13, 14, "Delimiter"),
            ]
        );
        assert_eq!(line_highlights("plain"), vec![(0, 5, "Directory")]);
    }

    #[test]
    fn stacks_keep_ten_lists() {
        let mut s = Stack::default();
        for i in 0..12 {
            s.push(List {
                title: i.to_string(),
                ..Default::default()
            });
        }
        assert_eq!(s.lists.len(), LISTCOUNT);
        assert_eq!(s.current().map(|l| l.title.as_str()), Some("11"));
    }
}
