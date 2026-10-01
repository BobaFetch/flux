use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use flux_core::{Change, Edit, History, LineEnding, Text};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BufferId(pub usize);

/// What the file looked like on disk when we last read or wrote it, to notice changes made by
/// other programs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DiskState {
    modified: Option<SystemTime>,
    len: u64,
}

impl DiskState {
    fn of(path: &Path) -> Option<Self> {
        let meta = fs::metadata(path).ok()?;
        Some(Self {
            modified: meta.modified().ok(),
            len: meta.len(),
        })
    }
}

#[derive(Debug)]
pub struct Buffer {
    pub id: BufferId,
    pub text: Text,
    pub history: History,
    /// The path as the user gave it, which is also how Vim names the buffer.
    pub path: Option<PathBuf>,
    /// The file didn't exist when opened.
    pub new_file: bool,
    /// Edits have been made that aren't part of the undo history yet (an Insert session in
    /// progress). They count as modifications.
    pub uncommitted: bool,
    /// The file wasn't valid UTF-8 and was loaded with replacement characters. Writing it back
    /// would corrupt it.
    pub invalid_utf8: bool,
    pub marks: crate::Marks,
    /// Shown by `:ls` (`:bdelete` unlists a buffer; it keeps its number).
    pub listed: bool,
    /// The file has been read. Buffers named on the command line are read when first shown.
    pub loaded: bool,
    /// Where the cursor was in each window that showed this buffer, most recent first, to go
    /// back there when the buffer is shown again (Vim's `wininfo`). `None` is Vim's line 0: the
    /// buffer was created in that window and never left there.
    pub positions: Vec<(crate::WindowId, Option<crate::Cursor>)>,
    /// The last Visual selection, for `gv`: anchor, cursor, kind and whether it went to the end
    /// of lines (`$`).
    pub last_visual: Option<(crate::Cursor, crate::Cursor, crate::VisualKind, bool)>,
    /// Buffer-local options ('tabstop', …).
    pub opts: crate::options::BufferOptions,
    /// The buffer lists a directory (see [`crate::explorer`]). It can't be changed or written,
    /// and isn't listed by `:ls`.
    pub directory: bool,
    /// The parser for the buffer's filetype, if flux has one.
    pub syntax: Option<flux_syntax::Syntax>,
    disk: Option<DiskState>,
}

impl Buffer {
    pub fn scratch(id: BufferId) -> Self {
        Self {
            id,
            text: Text::default(),
            history: History::default(),
            path: None,
            new_file: false,
            uncommitted: false,
            invalid_utf8: false,
            marks: Default::default(),
            listed: true,
            loaded: true,
            positions: Vec::new(),
            last_visual: None,
            opts: Default::default(),
            directory: false,
            syntax: None,
            disk: None,
        }
    }

    /// Load `path`. A missing file gives an empty buffer marked as new, as in Vim; a directory
    /// gives its listing, named by its absolute path.
    pub fn open(id: BufferId, path: &Path) -> io::Result<Self> {
        let mut buffer = Self::scratch(id);
        buffer.path = Some(path.to_path_buf());
        if path.is_dir() {
            let base = std::env::current_dir().unwrap_or_default();
            let path = crate::explorer::absolute(&base, path);
            buffer.text = Text::new(&crate::explorer::list_dir(&path)?);
            buffer.disk = DiskState::of(&path);
            buffer.path = Some(path);
            buffer.directory = true;
            buffer.listed = false;
            return Ok(buffer);
        }
        match read_file(path) {
            Ok((text, invalid_utf8)) => {
                buffer.text = text;
                buffer.invalid_utf8 = invalid_utf8;
                buffer.disk = DiskState::of(path);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => buffer.new_file = true,
            Err(e) => return Err(e),
        }
        Ok(buffer)
    }

    /// A buffer for `path` that will be read when first shown.
    pub fn unloaded(id: BufferId, path: &Path) -> Self {
        let mut buffer = Self::scratch(id);
        buffer.path = Some(path.to_path_buf());
        buffer.loaded = false;
        buffer
    }

    /// Read the file of an unloaded buffer.
    pub fn load(&mut self) -> io::Result<()> {
        if self.loaded {
            return Ok(());
        }
        let path = self.path.clone().ok_or(io::ErrorKind::NotFound)?;
        let fresh = Self::open(self.id, &path)?;
        *self = Self {
            listed: self.listed && !fresh.directory,
            positions: std::mem::take(&mut self.positions),
            opts: self.opts.clone(),
            ..fresh
        };
        Ok(())
    }

    /// Forget the text (for `:bdelete`); the buffer keeps its number, name and marks.
    pub fn unload(&mut self) {
        self.text = Text::default();
        self.history = History::default();
        self.uncommitted = false;
        self.loaded = false;
    }

    /// Where to put the cursor when the buffer is shown in `win`.
    pub fn last_position(&self, win: crate::WindowId) -> Option<crate::Cursor> {
        self.position_entry(win).and_then(|&(_, p)| p)
    }

    /// Vim's `find_wininfo`: the entry for `win`, or else the most recent one.
    fn position_entry(
        &self,
        win: crate::WindowId,
    ) -> Option<&(crate::WindowId, Option<crate::Cursor>)> {
        self.positions
            .iter()
            .find(|(w, _)| *w == win)
            .or(self.positions.first())
    }

    /// The line `:ls` shows for this buffer when it isn't current (Vim's `buflist_findlnum`):
    /// 0 for a buffer never left in a window, 1 when it has no position at all.
    pub fn listed_line(&self, win: crate::WindowId) -> usize {
        match self.position_entry(win) {
            Some((_, Some(p))) => p.line + 1,
            Some((_, None)) => 0,
            None => 1,
        }
    }

    /// The buffer was just created in `win` (Vim's `buflist_new`).
    pub fn created_in(&mut self, win: crate::WindowId) {
        if !self.positions.iter().any(|(w, _)| *w == win) {
            self.positions.insert(0, (win, None));
        }
    }

    /// The buffer is being left in `win` with the cursor at `pos` (Vim's `buflist_altfpos`).
    pub fn remember_position(&mut self, win: crate::WindowId, pos: crate::Cursor) {
        self.positions.retain(|(w, _)| *w != win);
        self.positions.insert(0, (win, Some(pos)));
    }

    /// `win`, showing this buffer, is closing (Vim's `close_buffer`): the first line doesn't
    /// replace a position already known for the window.
    pub fn window_closed(&mut self, win: crate::WindowId, pos: crate::Cursor) {
        let old = self
            .positions
            .iter()
            .position(|(w, _)| *w == win)
            .map(|i| self.positions.remove(i));
        let entry = match old {
            Some((_, p)) if pos.line == 0 => p,
            _ => Some(pos),
        };
        self.positions.insert(0, (win, entry));
    }

    /// The name Vim shows for the buffer.
    pub fn name(&self) -> String {
        match &self.path {
            Some(path) => path.display().to_string(),
            None => "[No Name]".into(),
        }
    }

    pub fn modified(&self) -> bool {
        self.uncommitted || self.history.is_modified()
    }

    fn line_and_byte_counts(&self) -> (usize, usize) {
        let lines = if self.text.has_no_lines() {
            0
        } else {
            self.text.line_count()
        };
        (lines, self.text.to_file_contents().len())
    }

    /// The file-info message Vim shows after loading, e.g. `"main.rs" 42L, 1337B`.
    pub fn file_info(&self) -> String {
        let mut msg = format!("\"{}\"", self.name());
        if self.directory {
            msg.push_str(" is a directory");
            return msg;
        }
        if self.new_file {
            msg.push_str(" [New]");
            return msg;
        }
        if self.invalid_utf8 {
            msg.push_str(" [invalid UTF-8]");
        }
        if !self.text.has_final_eol() {
            msg.push_str(" [noeol]");
        }
        if self.text.line_ending() == LineEnding::Crlf {
            msg.push_str(" [dos]");
        }
        let (lines, bytes) = self.line_and_byte_counts();
        msg.push_str(&format!(" {lines}L, {bytes}B"));
        msg
    }

    /// The file changed on disk since we last read or wrote it.
    pub fn changed_on_disk(&self) -> bool {
        let Some(path) = &self.path else {
            return false;
        };
        DiskState::of(path) != self.disk
    }

    /// Accept the file's current state on disk as known, so the same change isn't reported
    /// twice.
    pub fn acknowledge_disk_state(&mut self) {
        if let Some(path) = &self.path {
            self.disk = DiskState::of(path);
        }
    }

    /// Write the buffer to `path` (its own file when `None`), replacing the file atomically.
    /// `force` is `:w!`: write even if the file changed on disk or would be overwritten.
    /// Returns Vim's `"name" 3L, 42B written` message, or an error message.
    pub fn write(&mut self, path: Option<&Path>, force: bool) -> Result<String, String> {
        if self.directory {
            return Err(format!("E502: \"{}\" is a directory", self.name()));
        }
        let own = path.is_none() || path == self.path.as_deref();
        let target = match path.or(self.path.as_deref()) {
            Some(p) => p.to_path_buf(),
            None => return Err("E32: No file name".into()),
        };
        if !force {
            if own && self.changed_on_disk() && self.disk.is_some() {
                return Err(
                    "WARNING: The file has been changed since reading it!!! (add ! to override)"
                        .into(),
                );
            }
            if !own && target.exists() {
                return Err("E13: File exists (add ! to override)".into());
            }
            if own && self.invalid_utf8 {
                return Err(
                    "E513: Write error, conversion failed: the file was not valid UTF-8 (add ! to override)"
                        .into(),
                );
            }
        }
        let existed = target.exists();
        let contents = self.text.to_file_contents();
        write_atomically(&target, contents.as_bytes())
            .map_err(|e| format!("E212: Can't open file for writing: {e}"))?;

        let (lines, bytes) = self.line_and_byte_counts();
        let name = target.display().to_string();
        if own || self.path.is_none() {
            // `:w file` on an unnamed buffer names it, as in Vim.
            self.path = Some(target.clone());
            self.disk = DiskState::of(&target);
            self.history.mark_saved();
            self.new_file = false;
            self.invalid_utf8 = false;
        }
        let new = if existed { "" } else { " [New]" };
        Ok(format!("\"{name}\"{new} {lines}L, {bytes}B written"))
    }

    /// `:[range]w[!] [file]`: write lines `first..=last` (0-based). Writing part of the buffer
    /// over its own file needs `!` (E140), as does overwriting another existing file (E13).
    /// The buffer stays modified.
    /// `own` says whether `target` is the buffer's own file.
    pub fn write_lines(
        &self,
        target: &Path,
        first: usize,
        last: usize,
        force: bool,
        own: bool,
    ) -> Result<String, String> {
        if !force {
            if own {
                return Err("E140: Use ! to write partial buffer".into());
            }
            if target.exists() {
                return Err("E13: File exists (add ! to override)".into());
            }
        }
        let eol = match self.text.line_ending() {
            flux_core::LineEnding::Lf => "\n",
            flux_core::LineEnding::Crlf => "\r\n",
        };
        let mut contents = String::new();
        for line in first..=last.min(self.text.last_line()) {
            contents.push_str(&self.text.line_str(line));
            contents.push_str(eol);
        }
        let existed = target.exists();
        write_atomically(target, contents.as_bytes())
            .map_err(|e| format!("E212: Can't open file for writing: {e}"))?;
        let new = if existed { "" } else { " [New]" };
        let n = last.min(self.text.last_line()) + 1 - first;
        Ok(format!(
            "\"{}\"{new} {n}L, {}B written",
            target.display(),
            contents.len()
        ))
    }

    /// Reload the file from disk. Like Vim's 'undoreload', the reload is itself an undoable
    /// change.
    pub fn reload(&mut self, cursor: (usize, usize)) -> io::Result<()> {
        let path = self.path.clone().ok_or(io::ErrorKind::NotFound)?;
        if self.directory {
            // A listing is just read again; there is nothing to undo.
            self.text = Text::new(&crate::explorer::list_dir(&path)?);
            self.disk = DiskState::of(&path);
            return Ok(());
        }
        let (new_text, invalid_utf8) = read_file(&path)?;
        let before = self.text.rope().clone();
        let replace = Edit::replace(0..self.text.len_chars(), new_text.rope().to_string());
        let inverse = Edit::replace(0..new_text.len_chars(), before.to_string());
        let (b, a) =
            Change::changed_lines(&before, new_text.rope(), std::slice::from_ref(&replace));
        let change = Change {
            edits: vec![replace],
            inverse: vec![inverse],
            cursor_before: cursor,
            before: b,
            after: a,
            no_lines_before: self.text.has_no_lines(),
            no_lines_after: new_text.has_no_lines(),
            saved_lines: None,
        };
        self.text = new_text;
        self.invalid_utf8 = invalid_utf8;
        self.history.record(change);
        self.history.mark_saved();
        self.disk = DiskState::of(&path);
        self.new_file = false;
        Ok(())
    }
}

fn read_file(path: &Path) -> io::Result<(Text, bool)> {
    let bytes = fs::read(path)?;
    Ok(match String::from_utf8(bytes) {
        Ok(s) => (Text::new(&s), false),
        Err(e) => (Text::new(&String::from_utf8_lossy(e.as_bytes())), true),
    })
}

/// Write `contents` to a temporary file next to `path` and rename it into place, so a crash or
/// full disk never leaves a half-written file. Writes through symlinks and keeps the original
/// file's permissions.
fn write_atomically(path: &Path, contents: &[u8]) -> io::Result<()> {
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let dir = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let file_name = target
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a file name"))?;
    let tmp = dir.join(format!(
        ".{}.flux-{}.tmp",
        file_name.to_string_lossy(),
        std::process::id()
    ));
    let result = (|| {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(contents)?;
        file.sync_all()?;
        if let Ok(meta) = fs::metadata(&target) {
            fs::set_permissions(&tmp, meta.permissions())?;
        }
        fs::rename(&tmp, &target)
    })();
    if result.is_err() {
        fs::remove_file(&tmp).ok();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp_path(name: &str) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "flux-view-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    fn buffer_with(contents: &[u8]) -> Buffer {
        let path = temp_path("f.txt");
        fs::write(&path, contents).unwrap();
        let mut buf = Buffer::open(BufferId(1), &path).unwrap();
        buf.path = Some("f.txt".into());
        buf
    }

    #[test]
    fn file_info_messages() {
        assert_eq!(buffer_with(b"a\nb\n").file_info(), "\"f.txt\" 2L, 4B");
        assert_eq!(buffer_with(b"").file_info(), "\"f.txt\" 0L, 0B");
        assert_eq!(buffer_with(b"a\nb").file_info(), "\"f.txt\" [noeol] 2L, 4B");
        assert_eq!(buffer_with(b"a\r\n").file_info(), "\"f.txt\" [dos] 1L, 3B");
    }

    #[test]
    fn missing_file_is_new() {
        let buf = Buffer::open(BufferId(1), Path::new("/nonexistent/flux/x.txt")).unwrap();
        assert!(buf.new_file);
        assert_eq!(buf.file_info(), "\"/nonexistent/flux/x.txt\" [New]");
    }

    #[test]
    fn invalid_utf8_is_flagged_and_not_written_back() {
        let path = temp_path("bad.txt");
        fs::write(&path, b"ok\xff\n").unwrap();
        let mut buf = Buffer::open(BufferId(1), &path).unwrap();
        assert!(buf.invalid_utf8);
        assert!(buf.write(None, false).unwrap_err().starts_with("E513"));
        assert_eq!(fs::read(&path).unwrap(), b"ok\xff\n");
    }

    #[test]
    fn write_round_trips_and_marks_saved() {
        let path = temp_path("w.txt");
        fs::write(&path, "one\r\ntwo\r\n").unwrap();
        let mut buf = Buffer::open(BufferId(1), &path).unwrap();
        let inverse = buf.text.apply(&Edit::insert(0, "zero\n"));
        buf.history.record(Change {
            edits: vec![Edit::insert(0, "zero\n")],
            inverse: vec![inverse],
            cursor_before: (0, 0),
            before: 0..0,
            after: 0..1,
            no_lines_before: false,
            no_lines_after: false,
            saved_lines: None,
        });
        assert!(buf.modified());
        let msg = buf.write(None, false).unwrap();
        assert!(msg.ends_with("3L, 16B written"), "{msg}");
        assert!(!buf.modified());
        assert_eq!(fs::read_to_string(&path).unwrap(), "zero\r\none\r\ntwo\r\n");
    }

    #[test]
    fn write_refuses_after_external_change() {
        let path = temp_path("ext.txt");
        fs::write(&path, "a\n").unwrap();
        let mut buf = Buffer::open(BufferId(1), &path).unwrap();
        fs::write(&path, "changed elsewhere\n").unwrap();
        assert!(buf.changed_on_disk());
        assert!(buf.write(None, false).unwrap_err().starts_with("WARNING"));
        assert!(buf.write(None, true).is_ok());
        assert_eq!(fs::read_to_string(&path).unwrap(), "a\n");
        assert!(!buf.changed_on_disk());
    }

    #[test]
    fn write_new_file_and_to_other_path() {
        let path = temp_path("new.txt");
        let mut buf = Buffer::open(BufferId(1), &path).unwrap();
        assert!(
            buf.write(None, false)
                .unwrap()
                .contains("[New] 0L, 0B written")
        );
        let other = temp_path("other.txt");
        fs::write(&other, "x").unwrap();
        assert!(
            buf.write(Some(&other), false)
                .unwrap_err()
                .starts_with("E13")
        );
        assert!(buf.write(Some(&other), true).is_ok());
        assert_eq!(buf.path.as_deref(), Some(path.as_path()));
    }

    #[cfg(unix)]
    #[test]
    fn write_keeps_permissions_and_symlinks() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let real = temp_path("real.txt");
        fs::write(&real, "a\n").unwrap();
        fs::set_permissions(&real, fs::Permissions::from_mode(0o640)).unwrap();
        let link = real.with_file_name("link.txt");
        symlink(&real, &link).unwrap();
        let mut buf = Buffer::open(BufferId(1), &link).unwrap();
        buf.write(None, true).unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::metadata(&real).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    #[test]
    fn reload_is_undoable() {
        let path = temp_path("r.txt");
        fs::write(&path, "old\n").unwrap();
        let mut buf = Buffer::open(BufferId(1), &path).unwrap();
        fs::write(&path, "new\n").unwrap();
        buf.reload((0, 0)).unwrap();
        assert_eq!(buf.text.line_str(0), "new");
        assert!(!buf.modified());
        let change = buf.history.undo().unwrap().change;
        for edit in &change.inverse {
            buf.text.apply(edit);
        }
        assert_eq!(buf.text.line_str(0), "old");
    }
}
