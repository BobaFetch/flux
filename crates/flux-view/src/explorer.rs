//! Directory listings, like Vim's netrw: a directory opens as a read-only buffer listing its
//! entries, and `<CR>` / `-` in it move through the file tree.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::{Cursor, Editor};

/// The error for changing a listing (Vim's `'modifiable'` is off in netrw buffers).
pub const NOT_MODIFIABLE: &str = "E21: Cannot make changes, 'modifiable' is off";

/// The text of `dir`'s listing: `../`, then the directories (marked with a trailing `/`), then
/// the files, each sorted by name. Symlinks to directories list as directories.
pub fn list_dir(dir: &Path) -> io::Result<String> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if fs::metadata(entry.path()).is_ok_and(|m| m.is_dir()) {
            dirs.push(format!("{name}/"));
        } else {
            files.push(name);
        }
    }
    dirs.sort();
    files.sort();
    let mut text = String::from("../\n");
    for name in dirs.iter().chain(&files) {
        text.push_str(name);
        text.push('\n');
    }
    Ok(text)
}

/// Every file under `dir`: each level's files first (sorted, as in `list_dir`),
/// then its subdirectories in order. Paths are relative to `dir`. Hidden
/// entries are skipped, symlinked directories are listed but never descended
/// into, and unreadable directories are skipped; at most `cap` files. The cap applies
/// after sorting, so truncation is deterministic.
pub fn walk_files(dir: &Path, cap: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_into(dir, dir, &mut out, cap);
    out
}

fn walk_into(base: &Path, dir: &Path, out: &mut Vec<PathBuf>, cap: usize) {
    if out.len() >= cap {
        return;
    }
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        // `file_type` doesn't follow symlinks: a symlink to a directory lists
        // as a file and is never descended into, so walks can't loop.
        match entry.file_type() {
            Ok(t) if t.is_dir() => dirs.push(name),
            Ok(t) if t.is_file() || t.is_symlink() => files.push(name),
            _ => {}
        }
    }
    dirs.sort();
    files.sort();
    for name in files {
        if out.len() >= cap {
            return;
        }
        let full = dir.join(&name);
        out.push(full.strip_prefix(base).unwrap_or(&full).to_path_buf());
    }
    for name in dirs {
        walk_into(base, &dir.join(&name), out, cap);
    }
}

/// `path` taken from `base` (unless already absolute), with `.` and `..` resolved by name,
/// without following symlinks.
pub fn absolute(base: &Path, path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in base.join(path).components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// How a file opened from a listing is named: relative to `cwd` when it is inside it, as Vim
/// shortens buffer names.
fn short_name(cwd: &Path, path: &Path) -> PathBuf {
    match path.strip_prefix(cwd) {
        Ok(rel) if !rel.as_os_str().is_empty() => rel.to_path_buf(),
        _ => path.to_path_buf(),
    }
}

impl Editor {
    /// The directory the current buffer lists, if it is a listing.
    fn listed_dir(&self) -> Option<PathBuf> {
        let buffer = self.current_buffer();
        buffer
            .directory
            .then(|| buffer.path.as_deref().map(|p| absolute(&self.cwd, p)))
            .flatten()
    }

    /// `:Explore [dir]`: list `dir`, or else the directory of the current file with the cursor
    /// on that file (the working directory for an unnamed buffer, the same directory again,
    /// re-read, in a listing).
    pub fn explore(&mut self, dir: &str) -> Result<(), String> {
        if !dir.is_empty() {
            let dir = absolute(&self.cwd, Path::new(dir));
            return self.open_dir(&dir, None);
        }
        if let Some(dir) = self.listed_dir() {
            return self.open_dir(&dir, None);
        }
        match self.current_buffer().path.as_deref() {
            Some(p) => {
                let file = absolute(&self.cwd, p);
                let dir = file.parent().unwrap_or(&file).to_path_buf();
                let name = file.file_name().map(|n| n.to_string_lossy().into_owned());
                self.open_dir(&dir, name.as_deref())
            }
            None => self.open_dir(&self.cwd.clone(), None),
        }
    }

    /// Show the listing of `dir` (absolute) in the current window, read afresh, with the cursor
    /// on the entry `select` if given.
    pub fn open_dir(&mut self, dir: &Path, select: Option<&str>) -> Result<(), String> {
        if !dir.is_dir() {
            return Err(format!("\"{}\" is not a directory", dir.display()));
        }
        self.edit_file(dir)?;
        if let Some(name) = select {
            let dir_name = format!("{name}/");
            let text = self.text();
            let line = (0..text.line_count()).find(|&l| {
                let s = text.line_str(l);
                s == name || s == dir_name
            });
            if let Some(line) = line {
                self.window.cursor = Cursor { line, col: 0 };
                self.window.set_curswant = true;
                self.with_window(|w, m| w.scroll_to_cursor(m));
            }
        }
        Ok(())
    }

    /// What the cursor line of a listing names, as an absolute path (`../` is the parent).
    fn entry_under_cursor(&self) -> Option<PathBuf> {
        let dir = self.listed_dir()?;
        let line = self.text().line_str(self.cursor().line);
        match line.trim_end_matches('/') {
            "" => None,
            ".." => dir.parent().map(Path::to_path_buf),
            name => Some(dir.join(name)),
        }
    }

    /// `<CR>` in a listing: open the file under the cursor, or list the directory.
    pub fn open_entry(&mut self) -> Result<(), String> {
        let Some(path) = self.entry_under_cursor() else {
            return Ok(());
        };
        if self.listed_dir().as_deref().and_then(Path::parent) == Some(path.as_path()) {
            return self.open_parent();
        }
        self.open_path(&path)
    }

    /// `o` / `v` in a listing: open the entry under the cursor in a new window above / to the
    /// left ('splitbelow' and 'splitright' apply).
    pub fn open_entry_in_split(&mut self, vertical: bool) -> Result<(), String> {
        let Some(path) = self.entry_under_cursor() else {
            return Ok(());
        };
        let old = self.window.id;
        if !self.split(vertical, None) {
            return Ok(());
        }
        self.in_new_window = true;
        let result = self.open_path(&path);
        self.in_new_window = false;
        let current = self.window.buffer;
        let from = self.window_mut(old);
        if from.buffer != current {
            from.alt_buffer = Some(current);
        }
        result
    }

    /// `-` in a listing: list the parent directory, with the cursor on the one just left.
    pub fn open_parent(&mut self) -> Result<(), String> {
        let Some(dir) = self.listed_dir() else {
            return Ok(());
        };
        let Some(parent) = dir.parent() else {
            return Ok(());
        };
        let name = dir.file_name().map(|n| n.to_string_lossy().into_owned());
        self.open_dir(parent, name.as_deref())
    }

    fn open_path(&mut self, path: &Path) -> Result<(), String> {
        if path.is_dir() {
            self.open_dir(path, None)
        } else {
            let name = short_name(&self.cwd, path);
            self.edit_file(&name)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_resolves_dots_by_name() {
        let cwd = Path::new("/a/b");
        assert_eq!(absolute(cwd, Path::new(".")), Path::new("/a/b"));
        assert_eq!(absolute(cwd, Path::new("../c/./d")), Path::new("/a/c/d"));
        assert_eq!(absolute(cwd, Path::new("/x/../y")), Path::new("/y"));
        assert_eq!(absolute(cwd, Path::new("../../..")), Path::new("/"));
    }

    #[test]
    fn short_names_are_relative_inside_cwd() {
        let cwd = Path::new("/a/b");
        assert_eq!(short_name(cwd, Path::new("/a/b/c.rs")), Path::new("c.rs"));
        assert_eq!(short_name(cwd, Path::new("/a/x.rs")), Path::new("/a/x.rs"));
    }

    #[test]
    fn walk_lists_relative_skips_hidden_and_caps() {
        let root = std::env::temp_dir().join(format!("flux-walk-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        for name in ["b.rs", "a.rs", ".hidden", "sub/c.rs", "sub/.h"] {
            std::fs::write(root.join(name), "x").unwrap();
        }
        let walked = walk_files(&root, 100);
        let texts: Vec<String> = walked
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert_eq!(texts, vec!["a.rs", "b.rs", "sub/c.rs"]);
        let capped: Vec<String> = walk_files(&root, 2)
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert_eq!(capped, vec!["a.rs", "b.rs"]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
