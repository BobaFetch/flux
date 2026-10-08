//! Interactive selection state: the `:Files`/`:Buffers` pickers and the
//! command-line wildmenu. Rendering lives in flux-tui, key handling in
//! flux-vim; this is the data both work from.

use std::path::PathBuf;

use crate::BufferId;
use crate::matcher;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKind {
    Files,
    Buffers,
}

/// What picking an entry opens.
#[derive(Debug, Clone)]
pub enum PickerValue {
    /// A path relative to the editor's working directory.
    File(PathBuf),
    Buffer(BufferId),
}

#[derive(Debug, Clone)]
pub struct PickerEntry {
    /// The line as shown and matched.
    pub text: String,
    pub value: PickerValue,
}

/// A picker prompt: a query line over fuzzy-filtered entries. The selection
/// wraps; scrolling is computed by the renderer from `selected`.
#[derive(Debug, Clone)]
pub struct Picker {
    pub kind: PickerKind,
    entries: Vec<PickerEntry>,
    pub input: String,
    /// The cursor in `input`, as a char index.
    pub pos: usize,
    /// Entry indices matching `input`, best first.
    shown: Vec<usize>,
    /// The selection, as an index into `shown`.
    pub selected: usize,
    /// `:Files` cut the list (the cap or the walk ceiling). `:Buffers` stays false.
    pub truncated: bool,
}

impl Picker {
    pub fn new(kind: PickerKind, entries: Vec<PickerEntry>) -> Self {
        let shown = (0..entries.len()).collect();
        Self {
            kind,
            entries,
            input: String::new(),
            pos: 0,
            shown,
            selected: 0,
            truncated: false,
        }
    }

    pub fn files(paths: Vec<PathBuf>) -> Self {
        Self::files_truncated(paths, false)
    }

    /// `:Files` entries. `truncated` is set when the walk hit its cap or ceiling.
    /// A non-UTF-8 name is shown lossy; `PickerValue::File` keeps the real path.
    pub fn files_truncated(paths: Vec<PathBuf>, truncated: bool) -> Self {
        let mut picker = Self::new(
            PickerKind::Files,
            paths
                .into_iter()
                .map(|p| PickerEntry {
                    text: p.to_string_lossy().into_owned(),
                    value: PickerValue::File(p),
                })
                .collect(),
        );
        picker.truncated = truncated;
        picker
    }

    /// An initial query (from `:Files query`): filters at once.
    pub fn set_query(&mut self, query: &str) {
        self.input = query.to_string();
        self.pos = self.input.chars().count();
        self.refilter();
    }

    pub fn insert(&mut self, s: &str) {
        let pos = self.pos.min(self.input.chars().count());
        let at = self
            .input
            .char_indices()
            .nth(pos)
            .map_or(self.input.len(), |(i, _)| i);
        self.input.insert_str(at, s);
        self.pos = pos + s.chars().count();
        self.refilter();
    }

    pub fn backspace(&mut self) {
        let pos = self.pos.min(self.input.chars().count());
        if pos == 0 {
            return;
        }
        self.input = self
            .input
            .chars()
            .enumerate()
            .filter(|&(i, _)| i != pos - 1)
            .map(|(_, c)| c)
            .collect();
        self.pos = pos - 1;
        self.refilter();
    }

    pub fn delete_forwards(&mut self) {
        let pos = self.pos.min(self.input.chars().count());
        if pos >= self.input.chars().count() {
            return;
        }
        self.input = self
            .input
            .chars()
            .enumerate()
            .filter(|&(i, _)| i != pos)
            .map(|(_, c)| c)
            .collect();
        self.refilter();
    }

    /// `CTRL-W`: delete the word before the cursor (command-line rules: blanks,
    /// then a run of word or non-word characters).
    pub fn delete_word_before(&mut self) {
        let pos = self.pos.min(self.input.chars().count());
        let head: String = self.input.chars().take(pos).collect();
        let mut cut = head.clone();
        let trimmed = cut.trim_end_matches(' ').len();
        cut.truncate(trimmed);
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        match cut.chars().last() {
            None => {}
            Some(last) => {
                let word = is_word(last);
                while cut.chars().last().is_some_and(|c| is_word(c) == word) {
                    cut.pop();
                }
            }
        }
        let from = cut.chars().count();
        let tail: String = self.input.chars().skip(pos).collect();
        cut.push_str(&tail);
        self.input = cut;
        self.pos = from;
        self.refilter();
    }

    /// `CTRL-U`: delete everything before the cursor.
    pub fn clear_before(&mut self) {
        let pos = self.pos.min(self.input.chars().count());
        self.input = self.input.chars().skip(pos).collect();
        self.pos = 0;
        self.refilter();
    }

    pub fn move_to(&mut self, pos: usize) {
        self.pos = pos.min(self.input.chars().count());
    }

    /// Move the selection, wrapping around an empty list safely.
    pub fn move_sel(&mut self, delta: isize) {
        if self.shown.is_empty() {
            self.selected = 0;
            return;
        }
        let n = self.shown.len() as isize;
        self.selected = (self.selected as isize + delta).rem_euclid(n) as usize;
    }

    pub fn shown_count(&self) -> usize {
        self.shown.len()
    }

    /// Entries held before filtering. The truncation marker uses this count.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// The entries matching `input`, best first.
    pub fn shown_entries(&self) -> Vec<&PickerEntry> {
        self.shown.iter().map(|&i| &self.entries[i]).collect()
    }

    pub fn selected_entry(&self) -> Option<&PickerEntry> {
        self.shown.get(self.selected).map(|&i| &self.entries[i])
    }

    fn refilter(&mut self) {
        if self.input.is_empty() {
            self.shown = (0..self.entries.len()).collect();
        } else {
            let texts: Vec<String> = self.entries.iter().map(|e| e.text.clone()).collect();
            self.shown = matcher::rank(&self.input, &texts);
        }
        self.selected = 0;
    }
}

/// The command-line wildmenu: `<Tab>` cycling state. `selected` is `None` once
/// the cycle has come back around to the originally typed text.
#[derive(Debug, Clone)]
pub struct Wildmenu {
    pub items: Vec<String>,
    pub selected: Option<usize>,
    pub original: String,
    /// Char index in the command line where the completed word starts.
    pub start: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files_picker() -> Picker {
        Picker::files(
            ["src/main.rs", "src/lib.rs", "README.md", "docs/guide.md"]
                .iter()
                .map(PathBuf::from)
                .collect(),
        )
    }

    #[test]
    fn filter_select_wrap() {
        let mut p = files_picker();
        assert_eq!(p.shown_count(), 4);
        p.set_query("main");
        assert_eq!(p.shown_count(), 1);
        assert_eq!(p.selected_entry().unwrap().text, "src/main.rs");
        p.set_query("rs");
        assert!(p.shown_count() >= 2);
        p.move_sel(1);
        assert_eq!(p.selected, 1);
        p.move_sel(-1);
        assert_eq!(p.selected, 0);
        // Wraps both ways.
        p.move_sel(-1);
        assert_eq!(p.selected, p.shown_count() - 1);
        p.move_sel(1);
        assert_eq!(p.selected, 0);
    }

    #[test]
    fn editing_refilters() {
        let mut p = files_picker();
        p.insert("gui");
        assert_eq!(p.selected_entry().unwrap().text, "docs/guide.md");
        p.backspace();
        p.backspace();
        p.backspace();
        assert_eq!(p.shown_count(), 4);
        p.insert("src/");
        p.delete_word_before();
        assert_eq!(p.input, "src");
        p.delete_word_before();
        assert!(p.input.is_empty());
        p.insert("read");
        p.clear_before();
        assert!(p.input.is_empty());
        assert_eq!(p.shown_count(), 4);
    }

    #[test]
    fn files_records_whether_the_list_was_truncated() {
        let full = Picker::files(vec![PathBuf::from("a.txt")]);
        assert!(!full.truncated);
        assert_eq!(full.entry_count(), 1);
        let cut = Picker::files_truncated(vec![PathBuf::from("a.txt")], true);
        assert!(cut.truncated);
        assert_eq!(cut.entry_count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn file_entry_keeps_a_non_utf8_path() {
        use std::os::unix::ffi::OsStringExt;

        let path = PathBuf::from(std::ffi::OsString::from_vec(vec![0xff, b'.', b't']));
        let picker = Picker::files(vec![path.clone()]);
        let entry = picker.selected_entry().unwrap();
        assert_eq!(entry.text, path.to_string_lossy());
        match &entry.value {
            PickerValue::File(kept) => assert_eq!(kept, &path),
            other => panic!("expected a file path, got {other:?}"),
        }
    }

    #[test]
    fn empty_matches_nothing_selected() {
        let mut p = files_picker();
        p.set_query("zzz-no-such-file");
        assert_eq!(p.shown_count(), 0);
        assert!(p.selected_entry().is_none());
        p.move_sel(1);
        assert_eq!(p.selected, 0);
    }
}
