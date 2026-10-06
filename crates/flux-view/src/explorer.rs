//! Directory listings, like Vim's netrw: a directory opens as a read-only buffer listing its
//! entries, and `<CR>` / `-` in it move through the file tree.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use ignore::WalkBuilder;

use crate::{Cursor, Editor};

/// How many files `:Files` lists before it cuts the list.
pub const FILES_CAP: usize = 5_000;

/// Stop reading eligible files after this many, then select from what was read.
pub const WALK_CEILING: usize = 50_000;

/// Knobs for [`walk_files_with`]. The editor walk uses [`FILES_CAP`],
/// [`WALK_CEILING`], and the user's global git excludes.
pub struct WalkOpts {
    pub cap: usize,
    pub ceiling: usize,
    /// Read the user's global git excludes. Tests leave this off.
    pub git_global: bool,
}

/// Files under a walk root, in `:Files` order.
pub struct FileWalk {
    /// Paths relative to the walk root.
    pub paths: Vec<PathBuf>,
    /// The cap or the walk ceiling cut the list.
    pub truncated: bool,
}

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

/// Every file under `dir` for `:Files`, using the editor's ceiling and global excludes.
pub fn walk_files(dir: &Path, cap: usize) -> FileWalk {
    walk_files_with(
        dir,
        WalkOpts {
            cap,
            ceiling: WALK_CEILING,
            git_global: true,
        },
    )
}

/// Like [`walk_files`], with an explicit cap, ceiling, and global-excludes switch.
///
/// A path is listed when it is a regular file or a symlink (symlinks are never
/// followed, so a symlink to a directory is one entry), no component below
/// `dir` is hidden, and gitignore rules do not exclude it. Inside a git
/// repository those rules are `.gitignore` files from `dir` up to the
/// repository root, `.git/info/exclude`, and, when `git_global` is set, the
/// user's global excludes. `.ignore` files apply even outside a repository.
/// Ignored directories are not descended into. Unreadable directories and
/// ignore files are skipped.
///
/// Each directory's own files and each subdirectory share the directory's
/// budget max-min fairly ([`fair_shares`]). Names stay in `OsString` order:
/// selected files first, then subdirectories. Collection stops after `ceiling`
/// eligible files, depth-first in that name order; either limit sets
/// [`FileWalk::truncated`].
pub fn walk_files_with(dir: &Path, opts: WalkOpts) -> FileWalk {
    let mut root = Node::default();
    let mut seen = 0usize;
    let mut hit_ceiling = false;
    let walker = WalkBuilder::new(dir)
        .hidden(true)
        .parents(true)
        .ignore(true)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(opts.git_global)
        .require_git(true)
        .follow_links(false)
        .sort_by_file_name(|a, b| a.cmp(b))
        .build();
    for entry in walker {
        let Ok(entry) = entry else {
            continue;
        };
        if entry.depth() == 0 {
            continue;
        }
        let Some(ty) = entry.file_type() else {
            continue;
        };
        if !ty.is_file() && !ty.is_symlink() {
            continue;
        }
        if seen == opts.ceiling {
            hit_ceiling = true;
            break;
        }
        seen += 1;
        let path = entry.path();
        let rel = path.strip_prefix(dir).unwrap_or(path);
        root.insert(rel);
    }
    root.finish();
    let mut paths = Vec::new();
    root.emit(Path::new(""), opts.cap, &mut paths);
    let truncated = hit_ceiling || paths.len() < root.file_count;
    FileWalk { paths, truncated }
}

/// Max-min fair split of `budget` across `counts`, in the given order.
///
/// Small buckets are filled and dropped out of the next round; a remainder
/// smaller than the number of still-hungry buckets goes to the earliest ones.
fn fair_shares(budget: usize, counts: &[usize]) -> Vec<usize> {
    let mut alloc = vec![0usize; counts.len()];
    let mut remaining = budget;
    let mut unsat: Vec<usize> = counts
        .iter()
        .enumerate()
        .filter(|(_, count)| **count > 0)
        .map(|(index, _)| index)
        .collect();
    while remaining > 0 && !unsat.is_empty() {
        let share = remaining / unsat.len();
        if share == 0 {
            for &bucket in unsat.iter().take(remaining) {
                alloc[bucket] += 1;
            }
            break;
        }
        for &bucket in &unsat {
            let room = counts[bucket] - alloc[bucket];
            let given = room.min(share);
            alloc[bucket] += given;
            remaining -= given;
        }
        unsat.retain(|&bucket| alloc[bucket] < counts[bucket]);
    }
    alloc
}

/// One directory's eligible files and child directories, keyed in name order.
#[derive(Default)]
struct Node {
    files: Vec<OsString>,
    children: BTreeMap<OsString, Node>,
    file_count: usize,
}

impl Node {
    fn insert(&mut self, rel: &Path) {
        let parts: Vec<OsString> = rel
            .components()
            .filter_map(|component| match component {
                Component::Normal(name) => Some(name.to_os_string()),
                _ => None,
            })
            .collect();
        if !parts.is_empty() {
            self.insert_parts(&parts);
        }
    }

    fn insert_parts(&mut self, parts: &[OsString]) {
        if let [file] = parts {
            self.files.push(file.clone());
            return;
        }
        self.children
            .entry(parts[0].clone())
            .or_default()
            .insert_parts(&parts[1..]);
    }

    fn finish(&mut self) {
        self.files.sort();
        let mut count = self.files.len();
        for child in self.children.values_mut() {
            child.finish();
            count += child.file_count;
        }
        self.file_count = count;
    }

    fn emit(&self, prefix: &Path, budget: usize, out: &mut Vec<PathBuf>) {
        let mut counts = Vec::with_capacity(1 + self.children.len());
        counts.push(self.files.len());
        for child in self.children.values() {
            counts.push(child.file_count);
        }
        let alloc = fair_shares(budget, &counts);
        for name in self.files.iter().take(alloc.first().copied().unwrap_or(0)) {
            out.push(prefix.join(name));
        }
        for ((name, child), &child_budget) in self.children.iter().zip(alloc.iter().skip(1)) {
            if child_budget > 0 {
                child.emit(&prefix.join(name), child_budget, out);
            }
        }
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

    /// A temp tree named with this process id. Removed when dropped.
    struct Tmp(PathBuf);

    impl Tmp {
        fn new(label: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("flux-walk-{label}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn touch(root: &Path, rel: &str) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, "x").unwrap();
    }

    fn git_repo(root: &Path) {
        std::fs::create_dir(root.join(".git")).unwrap();
    }

    fn listed(root: &Path, cap: usize) -> (Vec<String>, bool) {
        listed_with(root, cap, WALK_CEILING)
    }

    fn listed_with(root: &Path, cap: usize, ceiling: usize) -> (Vec<String>, bool) {
        let walk = walk_files_with(
            root,
            WalkOpts {
                cap,
                ceiling,
                git_global: false,
            },
        );
        let texts = walk
            .paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        (texts, walk.truncated)
    }

    #[test]
    fn fair_shares_split_budget_max_min() {
        let cases = [
            (2, &[2, 1][..], &[1, 1][..]),
            (10, &[1, 50, 3], &[1, 6, 3]),
            (5, &[10, 1, 10], &[2, 1, 2]),
            (7, &[10, 1, 10], &[3, 1, 3]),
            (3, &[1, 1, 1, 1], &[1, 1, 1, 0]),
            (0, &[5], &[0]),
            (100, &[5, 5], &[5, 5]),
        ];
        for (budget, counts, expect) in cases {
            assert_eq!(
                fair_shares(budget, counts),
                expect,
                "budget {budget} counts {counts:?}"
            );
        }
    }

    #[test]
    fn walk_lists_relative_skips_hidden_and_caps() {
        let tmp = Tmp::new("relative");
        for name in ["b.rs", "a.rs", ".hidden", "sub/c.rs", "sub/.h"] {
            touch(tmp.path(), name);
        }
        let (texts, truncated) = listed(tmp.path(), 100);
        assert!(!truncated);
        assert_eq!(texts, ["a.rs", "b.rs", "sub/c.rs"]);
        // Cap 2 is shared: one root file, then the subdirectory, instead of both root files.
        let (capped, truncated) = listed(tmp.path(), 2);
        assert!(truncated);
        assert_eq!(capped, ["a.rs", "sub/c.rs"]);
    }

    #[test]
    fn walk_respects_gitignore_in_a_repo() {
        let tmp = Tmp::new("gitignore");
        git_repo(tmp.path());
        std::fs::write(tmp.path().join(".gitignore"), "target/\n*.log\n!keep.log\n").unwrap();
        for name in [
            "README.md",
            "keep.log",
            "a.log",
            "src/main.rs",
            "target/debug/x",
        ] {
            touch(tmp.path(), name);
        }
        let (texts, truncated) = listed(tmp.path(), 100);
        assert!(!truncated);
        assert_eq!(texts, ["README.md", "keep.log", "src/main.rs"]);
    }

    #[test]
    fn walk_ignores_gitignore_outside_a_repo() {
        let tmp = Tmp::new("norepo");
        std::fs::write(tmp.path().join(".gitignore"), "target/\n*.log\n!keep.log\n").unwrap();
        for name in [
            "README.md",
            "keep.log",
            "a.log",
            "src/main.rs",
            "target/debug/x",
        ] {
            touch(tmp.path(), name);
        }
        // `.ignore` still applies outside a repository.
        std::fs::create_dir_all(tmp.path().join("notes")).unwrap();
        std::fs::write(tmp.path().join("notes/.ignore"), "skip.txt\n").unwrap();
        touch(tmp.path(), "notes/skip.txt");
        touch(tmp.path(), "notes/keep.txt");
        let (texts, truncated) = listed(tmp.path(), 100);
        assert!(!truncated);
        assert!(texts.contains(&"target/debug/x".to_string()));
        assert!(texts.contains(&"a.log".to_string()));
        assert!(!texts.iter().any(|path| path == "notes/skip.txt"));
        assert_eq!(
            texts,
            [
                "README.md",
                "a.log",
                "keep.log",
                "notes/keep.txt",
                "src/main.rs",
                "target/debug/x"
            ]
        );
    }

    #[test]
    fn walk_applies_ancestor_gitignore_from_repo_root() {
        let tmp = Tmp::new("ancestor");
        git_repo(tmp.path());
        std::fs::write(tmp.path().join(".gitignore"), "build/\n").unwrap();
        let sub = tmp.path().join("sub");
        touch(&sub, "keep.txt");
        touch(&sub, "build/hidden.txt");
        let (texts, truncated) = listed(&sub, 100);
        assert!(!truncated);
        assert_eq!(texts, ["keep.txt"]);
    }

    #[test]
    fn walk_respects_nested_gitignore_and_dot_ignore() {
        let tmp = Tmp::new("nested");
        git_repo(tmp.path());
        std::fs::create_dir_all(tmp.path().join("pkg")).unwrap();
        std::fs::write(tmp.path().join("pkg/.gitignore"), "from_git.txt\n").unwrap();
        touch(tmp.path(), "pkg/from_git.txt");
        touch(tmp.path(), "pkg/stay.rs");
        std::fs::create_dir_all(tmp.path().join("notes")).unwrap();
        std::fs::write(tmp.path().join("notes/.ignore"), "from_dot.txt\n").unwrap();
        touch(tmp.path(), "notes/from_dot.txt");
        touch(tmp.path(), "notes/stay.txt");
        let (texts, truncated) = listed(tmp.path(), 100);
        assert!(!truncated);
        assert_eq!(texts, ["notes/stay.txt", "pkg/stay.rs"]);
    }

    #[test]
    fn walk_applies_gitignore_inside_a_nested_repo() {
        let tmp = Tmp::new("nestedrepo");
        git_repo(tmp.path());
        let nested = tmp.path().join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        git_repo(&nested);
        std::fs::write(nested.join(".gitignore"), "hide.txt\n").unwrap();
        touch(&nested, "hide.txt");
        touch(&nested, "show.txt");
        let (texts, truncated) = listed(tmp.path(), 100);
        assert!(!truncated);
        assert_eq!(texts, ["nested/show.txt"]);
    }

    #[test]
    fn walk_does_not_reinclude_a_file_under_an_excluded_directory() {
        let tmp = Tmp::new("negation");
        git_repo(tmp.path());
        std::fs::write(tmp.path().join(".gitignore"), "build/\n!build/keep.txt\n").unwrap();
        touch(tmp.path(), "build/keep.txt");
        touch(tmp.path(), "keep.txt");
        let (texts, truncated) = listed(tmp.path(), 100);
        assert!(!truncated);
        assert_eq!(texts, ["keep.txt"]);
    }

    #[test]
    fn walk_respects_git_info_exclude() {
        let tmp = Tmp::new("exclude");
        std::fs::create_dir_all(tmp.path().join(".git/info")).unwrap();
        std::fs::write(tmp.path().join(".git/info/exclude"), "secret.txt\n").unwrap();
        touch(tmp.path(), "secret.txt");
        touch(tmp.path(), "visible.txt");
        let (texts, truncated) = listed(tmp.path(), 100);
        assert!(!truncated);
        assert_eq!(texts, ["visible.txt"]);
    }

    #[test]
    fn walk_star_gitignore_lists_nothing() {
        let tmp = Tmp::new("star");
        git_repo(tmp.path());
        std::fs::write(tmp.path().join(".gitignore"), "*\n").unwrap();
        touch(tmp.path(), "file.txt");
        touch(tmp.path(), "sub/a.txt");
        let (texts, truncated) = listed(tmp.path(), 100);
        assert!(!truncated);
        assert!(texts.is_empty());
    }

    #[test]
    fn walk_shares_cap_fairly_between_directories() {
        let tmp = Tmp::new("fair");
        touch(tmp.path(), "top.txt");
        for i in 0..50 {
            touch(tmp.path(), &format!("big/f{i:02}"));
        }
        for name in ["a", "b", "c"] {
            touch(tmp.path(), &format!("small/{name}"));
        }
        let (texts, truncated) = listed(tmp.path(), 10);
        assert!(truncated);
        let mut expect = vec!["top.txt".to_string()];
        for i in 0..6 {
            expect.push(format!("big/f{i:02}"));
        }
        expect.extend(["small/a", "small/b", "small/c"].map(str::to_string));
        assert_eq!(texts, expect);
    }

    #[test]
    fn walk_lists_everything_under_the_cap() {
        let tmp = Tmp::new("complete");
        for dir in ["d0", "d1", "d2"] {
            for name in ["a", "b", "c"] {
                touch(tmp.path(), &format!("{dir}/{name}"));
            }
        }
        let (texts, truncated) = listed(tmp.path(), 100);
        assert!(!truncated);
        assert_eq!(texts.len(), 9);
        assert_eq!(
            texts,
            [
                "d0/a", "d0/b", "d0/c", "d1/a", "d1/b", "d1/c", "d2/a", "d2/b", "d2/c"
            ]
        );
    }

    #[test]
    fn walk_keeps_files_hidden_by_a_big_ignored_dir() {
        let tmp = Tmp::new("bigignored");
        git_repo(tmp.path());
        std::fs::write(tmp.path().join(".gitignore"), "/target\n").unwrap();
        for i in 0..60 {
            touch(tmp.path(), &format!("target/t{i:02}"));
        }
        touch(tmp.path(), "xtask/oracle.lua");
        let (texts, truncated) = listed(tmp.path(), 50);
        assert!(!truncated);
        assert_eq!(texts, ["xtask/oracle.lua"]);

        std::fs::remove_file(tmp.path().join(".gitignore")).unwrap();
        let (texts, truncated) = listed(tmp.path(), 50);
        assert!(truncated);
        assert_eq!(texts.len(), 50);
        assert!(texts.iter().any(|path| path == "xtask/oracle.lua"));
        assert!(texts.iter().any(|path| path.starts_with("target/")));
    }

    #[test]
    fn walk_stops_at_the_ceiling() {
        let tmp = Tmp::new("ceiling");
        for i in 0..20 {
            touch(tmp.path(), &format!("f{i:02}"));
        }
        let once = listed_with(tmp.path(), 100, 5);
        let twice = listed_with(tmp.path(), 100, 5);
        assert!(once.1);
        assert_eq!(once.0.len(), 5);
        assert_eq!(once, twice);
    }

    #[cfg(unix)]
    #[test]
    fn walk_lists_a_symlink_to_a_directory_without_following_it() {
        use std::os::unix::fs::symlink;

        let tmp = Tmp::new("symlink");
        touch(tmp.path(), "real/inside.txt");
        symlink("real", tmp.path().join("link")).unwrap();
        symlink(".", tmp.path().join("loop")).unwrap();
        let (texts, truncated) = listed(tmp.path(), 100);
        assert!(!truncated);
        assert_eq!(texts, ["link", "loop", "real/inside.txt"]);
    }

    #[cfg(unix)]
    #[test]
    fn walk_unreadable_root_is_empty() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = Tmp::new("unreadable");
        touch(tmp.path(), "a.txt");
        let mut blocked = std::fs::metadata(tmp.path()).unwrap().permissions();
        blocked.set_mode(0o000);
        std::fs::set_permissions(tmp.path(), blocked).unwrap();
        let _restore = ModeGuard(tmp.path().to_path_buf());
        if std::fs::read_dir(tmp.path()).is_ok() {
            return;
        }
        let (texts, truncated) = listed(tmp.path(), 10);
        assert!(texts.is_empty());
        assert!(!truncated);
    }

    #[cfg(unix)]
    #[test]
    fn walk_skips_an_unreadable_directory() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = Tmp::new("baddir");
        touch(tmp.path(), "ok.txt");
        touch(tmp.path(), "mid/secret.txt");
        let mid = tmp.path().join("mid");
        let mut blocked = std::fs::metadata(&mid).unwrap().permissions();
        blocked.set_mode(0o000);
        std::fs::set_permissions(&mid, blocked).unwrap();
        let _restore = ModeGuard(mid);
        let (texts, truncated) = listed(tmp.path(), 20);
        assert!(!truncated);
        assert_eq!(texts, ["ok.txt"]);
    }

    #[cfg(unix)]
    struct ModeGuard(PathBuf);

    #[cfg(unix)]
    impl Drop for ModeGuard {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;

            if let Ok(meta) = std::fs::metadata(&self.0) {
                let mut perms = meta.permissions();
                perms.set_mode(0o755);
                let _ = std::fs::set_permissions(&self.0, perms);
            }
        }
    }
}
