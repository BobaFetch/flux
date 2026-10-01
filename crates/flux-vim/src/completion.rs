//! Insert-mode completion, as Vim's `insexpand.c` does it: `CTRL-N`/`CTRL-P` keyword
//! completion, `CTRL-X CTRL-O` omni completion with the language servers (see
//! [`crate::lsp::completion`]), and the matches they give shown in the popup menu
//! ([`flux_view::pum`]), selected with `CTRL-N`/`CTRL-P` and the cursor keys, accepted with
//! `CTRL-Y` and dropped with `CTRL-E`.
//!
//! The matches are a ring, like Vim's: the original text first, then the matches, each linked
//! to the next and previous one.

use flux_core::Edit;
use flux_view::Editor;
use flux_view::lsp::ClientId;
use flux_view::pum::PumItem;
use serde_json::Value;

use crate::engine::Engine;
use crate::key::{Key, KeyCode, Modifiers};
use crate::util::{self, pos};

/// Vim's `ctrl_x_mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum XMode {
    /// `CTRL-N`/`CTRL-P` completion, or none.
    #[default]
    Normal,
    /// `CTRL-X` was typed: the next key says which completion.
    NotDefinedYet,
    Omni,
    /// Matches given all at once (Vim's `complete()`), as omni completion does.
    Eval,
    /// A key that ends the completion was typed.
    Finished,
}

const MSG_CTRL_X: &str = " ^X mode (^]^D^E^F^I^K^L^N^O^P^Rs^U^V^Y)";
const MSG_KEYWORD: &str = " Keyword completion (^N^P)";
const MSG_LOCAL: &str = " Keyword Local completion (^N^P)";

/// What a language server's item was, for using it when it's accepted.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LspData {
    pub client: ClientId,
    pub item: Value,
}

/// A completion match (Vim's `compl_T` with its complete-items fields).
#[derive(Debug, Clone, Default)]
pub(crate) struct Match {
    pub word: String,
    pub abbr: Option<String>,
    pub kind: Option<String>,
    pub menu: Option<String>,
    pub info: Option<String>,
    /// Ignore case when comparing with the typed text.
    pub icase: bool,
    pub abbr_hl: Option<String>,
    pub kind_hl: Option<String>,
    pub lsp: Option<LspData>,
    /// The file it was found in (another buffer), shown as the menu text.
    pub fname: Option<String>,
    pub(crate) original: bool,
    pub(crate) number: Option<usize>,
    pub(crate) next: Option<usize>,
    pub(crate) prev: Option<usize>,
    pub(crate) in_array: bool,
}

impl Match {
    pub fn new(word: impl Into<String>) -> Self {
        Self {
            word: word.into(),
            ..Self::default()
        }
    }

    /// Whether the match starts with `leader` (Vim's `ins_compl_equal`).
    fn equal(&self, leader: &str) -> bool {
        if self.icase {
            let w: String = self.word.chars().take(leader.chars().count()).collect();
            w.to_lowercase() == leader.to_lowercase()
        } else {
            self.word.starts_with(leader)
        }
    }
}

/// The completion going on in Insert mode.
#[derive(Debug, Default)]
pub(crate) struct Completion {
    pub mode: XMode,
    pub started: bool,
    matches: Vec<Match>,
    first: Option<usize>,
    /// The match shown (Vim's `compl_shown_match`) and the current one (`compl_curr_match`).
    shown: Option<usize>,
    curr: Option<usize>,
    forward: bool,
    shows_forward: bool,
    /// Where the completed text starts: line and char column.
    line: usize,
    col: usize,
    /// The length of the text completed when the matches were looked for.
    length: usize,
    orig: String,
    leader: Option<String>,
    used_match: bool,
    enter_selects: bool,
    restarting: bool,
    /// The matches in the menu (Vim's `compl_match_array`).
    array: Option<Vec<usize>>,
    selected_item: Option<usize>,
    /// How many matches were found.
    count: usize,
    /// `CTRL-X CTRL-N`: only the current buffer.
    local: bool,
    /// The match whose text was inserted last (`v:completed_item`).
    pub completed: Option<usize>,
    /// The text typed before completing, as recorded for `.`.
    redo_base: Option<String>,
    /// Where the language server's matches replace text from (Neovim's `Context.cursor`).
    pub lsp_start: Option<flux_view::Cursor>,
    /// A `completionItem/resolve` for the info window is waiting for this match.
    pub resolving: Option<usize>,
}

impl Completion {
    fn next_of(&self, i: usize) -> Option<usize> {
        self.matches[i].next
    }

    fn prev_of(&self, i: usize) -> Option<usize> {
        self.matches[i].prev
    }

    fn is_first(&self, i: usize) -> bool {
        self.first == Some(i)
    }

    /// Add a match after (forward) or before the current one (Vim's `ins_compl_add`).
    /// Without `dup`, a match with the same text as one already there isn't added.
    fn add(&mut self, m: Match, dup: bool, forward: bool) -> bool {
        if !dup
            && self
                .iter()
                .into_iter()
                .any(|i| !self.matches[i].original && self.matches[i].word == m.word)
        {
            return false;
        }
        self.array = None;
        let i = self.matches.len();
        let mut m = m;
        m.number = if m.original { Some(0) } else { None };
        self.matches.push(m);
        match self.curr {
            None => {
                self.matches[i].next = None;
                self.matches[i].prev = None;
            }
            Some(c) if forward => {
                self.matches[i].next = self.matches[c].next;
                self.matches[i].prev = Some(c);
            }
            Some(c) => {
                self.matches[i].next = Some(c);
                self.matches[i].prev = self.matches[c].prev;
            }
        }
        if let Some(n) = self.matches[i].next {
            self.matches[n].prev = Some(i);
        }
        match self.matches[i].prev {
            Some(p) => self.matches[p].next = Some(i),
            None => self.first = Some(i),
        }
        self.curr = Some(i);
        true
    }

    /// Link the last match to the first; returns how many matches there are, not counting the
    /// original text.
    fn make_cyclic(&mut self) -> usize {
        let Some(first) = self.first else {
            return 0;
        };
        let mut m = first;
        let mut count = 0;
        while let Some(n) = self.matches[m].next {
            if n == first {
                break;
            }
            m = n;
            count += 1;
        }
        self.matches[m].next = Some(first);
        self.matches[first].prev = Some(m);
        count
    }

    /// The matches from the first, in order.
    fn iter(&self) -> Vec<usize> {
        let mut out = Vec::new();
        let Some(first) = self.first else {
            return out;
        };
        let mut m = Some(first);
        while let Some(i) = m {
            out.push(i);
            m = self.matches[i].next.filter(|&n| n != first);
        }
        out
    }

    fn clear(&mut self) {
        self.matches.clear();
        self.first = None;
        self.shown = None;
        self.curr = None;
        self.array = None;
        self.leader = None;
        self.completed = None;
        self.resolving = None;
    }

    /// Only the original text: nothing was found.
    fn no_matches(&self) -> bool {
        self.first
            .is_none_or(|f| self.matches[f].next.is_none_or(|n| n == f))
    }

    fn is_ctrl_x_key(&self, key: Key, pum_visible: bool) -> bool {
        let ctrl = |c| key == Key::ctrl(c);
        if pum_key(key, pum_visible) {
            return true;
        }
        match self.mode {
            XMode::Normal => ctrl('n') || ctrl('p') || ctrl('x'),
            XMode::NotDefinedYet => ctrl('x') || ctrl('n') || ctrl('p') || ctrl('o') || ctrl('z'),
            XMode::Omni => ctrl('o') || ctrl('n') || ctrl('p'),
            XMode::Eval => ctrl('n') || ctrl('p'),
            XMode::Finished => false,
        }
    }

    /// The text matches start with: what was typed, or the original text.
    fn leader_or_orig(&self) -> &str {
        self.leader.as_deref().unwrap_or(&self.orig)
    }

    /// The match `i` as a menu line.
    fn pum_item(&self, i: usize) -> PumItem {
        let m = &self.matches[i];
        PumItem {
            abbr: m.abbr.clone().unwrap_or_else(|| m.word.clone()),
            kind: m.kind.clone(),
            menu: m.menu.clone().or_else(|| m.fname.clone()),
            info: m.info.clone(),
            abbr_hl: m.abbr_hl.clone(),
            kind_hl: m.kind_hl.clone(),
        }
    }

    /// The match selected in the menu, if it's one of a language server's.
    pub fn selected_lsp(&self) -> Option<(usize, &LspData)> {
        let c = self.curr?;
        self.matches[c].lsp.as_ref().map(|l| (c, l))
    }

    pub fn match_info(&self, i: usize) -> Option<&Match> {
        self.matches.get(i)
    }
}

/// Keys that move in the menu while it's shown.
fn pum_key(key: Key, pum_visible: bool) -> bool {
    pum_visible
        && key.mods == Modifiers::NONE
        && matches!(
            key.code,
            KeyCode::PageUp | KeyCode::PageDown | KeyCode::Up | KeyCode::Down
        )
}

/// Whether a key goes backwards (Vim's `ins_compl_key2dir`).
fn key_forward(key: Key) -> bool {
    !(key == Key::ctrl('p')
        || (key.mods == Modifiers::NONE && matches!(key.code, KeyCode::PageUp | KeyCode::Up)))
}

/// Whether selecting with `key` inserts the match (the cursor keys only select).
fn use_match(key: Key) -> bool {
    !(key.mods == Modifiers::NONE
        && matches!(
            key.code,
            KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown
        ))
}

impl Engine {
    /// The menu is wanted: 'completeopt' has `menu` or `menuone`.
    fn pum_wanted(editor: &Editor) -> bool {
        editor.completeopt("menu") || editor.completeopt("menuone")
    }

    /// Handle `key` for completion before Insert mode does (Vim's handling of keys in
    /// `insert_check` and `ins_compl_prep`, and the completion keys of `insert_handle_key`).
    /// Returns whether the key was used up.
    pub(crate) fn completion_key(&mut self, editor: &mut Editor, key: Key) -> bool {
        if editor.completion.show_error {
            editor.completion.show_error = false;
            editor.message = None;
        }
        let ctrl = |c| key == Key::ctrl(c);
        let pum_visible = editor.completion.pum.is_some();
        // While the menu is wanted and the cursor is in the completed text.
        let cur = editor.cursor();
        let c = &self.compl;
        let in_word = c.started && cur.line == c.line && cur.col >= c.col && c.shown.is_some();
        let after_start = cur.col > c.col;
        if in_word && Self::pum_wanted(editor) {
            if (key == Key::plain(KeyCode::Backspace) || ctrl('h'))
                && after_start
                && self.compl_bs(editor)
            {
                return true;
            }
            if !self.compl.used_match {
                if let Some(ch) = key.typed_char()
                    && self.accept_char(ch)
                {
                    editor.close_floats();
                    self.compl_addleader(editor, ch);
                    return true;
                }
                let enter = key == Key::plain(KeyCode::Enter) || ctrl('m') || ctrl('j');
                if ctrl('y') || (self.compl.enter_selects && enter) {
                    self.compl_delete(editor, false);
                    self.compl_insert(editor);
                }
            }
        }
        if self.compl_prep(editor, key) {
            return true;
        }
        let pum_visible = pum_visible && editor.completion.pum.is_some();
        if ctrl('x') {
            self.compl.mode = XMode::NotDefinedYet;
            editor.completion.submode.text = Some(MSG_CTRL_X.into());
            return true;
        }
        if ctrl('o') && self.compl.mode == XMode::Omni
            || ctrl('n')
            || ctrl('p')
            || pum_key(key, pum_visible)
        {
            self.compl_do(editor, key);
            return true;
        }
        false
    }

    /// Whether `c` typed while the menu is shown narrows the matches (Vim's
    /// `ins_compl_accept_char`).
    fn accept_char(&self, c: char) -> bool {
        match self.compl.mode {
            XMode::Omni => !c.is_control() && c != ' ' && c != '\t',
            _ => flux_core::chars::is_keyword(c),
        }
    }

    /// Get ready for completion or end it, for a typed key (Vim's `ins_compl_prep`). Returns
    /// whether the key is used up.
    fn compl_prep(&mut self, editor: &mut Editor, key: Key) -> bool {
        let mut retval = false;
        let prev_mode = self.compl.mode;
        let pum_visible = editor.completion.pum.is_some();
        if self.compl.is_ctrl_x_key(key, pum_visible) {
            editor.completion.submode.extra = None;
        }
        if self.compl.mode == XMode::NotDefinedYet
            || (self.compl.mode == XMode::Normal && !self.compl.started)
        {
            self.compl.used_match = true;
        }
        if self.compl.mode == XMode::NotDefinedYet {
            retval = self.set_ctrl_x_mode(editor, key);
        } else if self.compl.mode != XMode::Normal && !self.compl.is_ctrl_x_key(key, pum_visible) {
            self.compl.mode = XMode::Finished;
            editor.completion.submode.text = None;
        }
        if self.compl.started || self.compl.mode == XMode::Finished {
            let normal_stop = self.compl.mode == XMode::Normal
                && key != Key::ctrl('n')
                && key != Key::ctrl('p')
                && !pum_key(key, pum_visible);
            if normal_stop || self.compl.mode == XMode::Finished {
                retval = self.compl_stop(editor, key, prev_mode, retval);
            }
        }
        if !self.compl.is_ctrl_x_key(key, pum_visible) {
            self.compl.local = false;
        }
        retval
    }

    /// The key after `CTRL-X` (Vim's `set_ctrl_x_mode`).
    fn set_ctrl_x_mode(&mut self, editor: &mut Editor, key: Key) -> bool {
        if key == Key::ctrl('o') {
            self.compl.mode = XMode::Omni;
            return false;
        }
        if key == Key::ctrl('n') || key == Key::ctrl('p') {
            self.compl.local = true;
        }
        self.compl.mode = XMode::Normal;
        editor.completion.submode.text = None;
        key == Key::ctrl('z')
    }

    /// End completion (Vim's `ins_compl_stop`).
    fn compl_stop(
        &mut self,
        editor: &mut Editor,
        key: Key,
        prev_mode: XMode,
        retval: bool,
    ) -> bool {
        let mut retval = retval;
        let enter = key == Key::plain(KeyCode::Enter) || key == Key::ctrl('m');
        let pum_visible = editor.completion.pum.is_some();
        let mut word = None;
        let mut accepted = None;
        if (key == Key::ctrl('y') || (self.compl.enter_selects && enter))
            && pum_visible
            && let Some(s) = self.compl.shown
        {
            word = Some(self.compl.matches[s].word.clone());
            accepted = self.compl.completed;
            retval = true;
        }
        // A match inserted without the menu shown is taken.
        if word.is_none()
            && key != Key::ctrl('e')
            && self.compl.used_match
            && self.compl.array.is_none()
            && let Some(c) = self.compl.curr
        {
            word = Some(self.compl.matches[c].word.clone());
            accepted = self.compl.completed;
        }
        if key == Key::ctrl('e') {
            self.compl_delete(editor, false);
            let p = self
                .compl
                .leader
                .clone()
                .unwrap_or_else(|| self.compl.orig.clone());
            let len = self.compl_len(editor);
            let rest: String = p.chars().skip(len).collect();
            if !rest.is_empty() {
                self.compl_insert_text(editor, &rest);
            }
            retval = true;
        }
        self.compl_fix_redo(editor);
        let accepted_match = accepted.and_then(|i| self.compl.matches.get(i).cloned());
        let start = self.compl.lsp_start;
        editor.pum_undisplay();
        self.compl.clear();
        self.compl.started = false;
        self.compl.count = 0;
        self.compl.mode = XMode::Normal;
        self.compl.enter_selects = false;
        editor.completion.submode = Default::default();
        let _ = prev_mode;
        // CompleteDone: an accepted language server item may have more to do.
        if word.is_some()
            && let Some(m) = accepted_match
            && let Some(lsp) = m.lsp
        {
            crate::lsp::completion::complete_done(self, editor, &lsp, start);
        }
        retval
    }

    /// Complete with `key`: start, or go to another match (Vim's `ins_complete`).
    fn compl_do(&mut self, editor: &mut Editor, key: Key) {
        let forward = key_forward(key);
        self.compl.forward = forward;
        let insert_match = use_match(key);
        if !self.compl.started && !self.compl_start(editor) {
            self.compl.local = false;
            return;
        }
        self.compl.shown = self.compl.curr;
        self.compl.shows_forward = forward;
        let count = match key.code {
            KeyCode::PageUp | KeyCode::PageDown => {
                let h = editor.completion.pum.as_ref().map_or(0, |p| p.height);
                if h > 3 { h - 2 } else { h }
            }
            _ => 1,
        };
        let n = self.compl_next(editor, true, count.max(1), insert_match);
        if n > 1 {
            self.compl.count = n as usize;
        }
        self.compl.curr = self.compl.shown;
        self.compl.forward = self.compl.shows_forward;
        self.compl_statusmsg(editor);
        self.compl_show_pum(editor);
    }

    /// Start completing: where the text to complete starts, and its matches (Vim's
    /// `ins_compl_start`). Omni completion asks the language servers, whose answer comes
    /// later.
    fn compl_start(&mut self, editor: &mut Editor) -> bool {
        let cur = editor.cursor();
        let line = util::line(editor, cur.line);
        let chars: Vec<char> = line.chars().collect();
        match self.compl.mode {
            XMode::Omni => {
                crate::lsp::completion::omnifunc(self, editor);
                false
            }
            XMode::Normal => {
                // The keyword before the cursor (Vim's `get_normal_compl_info`).
                let mut start = cur.col.min(chars.len());
                if start > 0 && flux_core::chars::is_keyword(chars[start - 1]) {
                    while start > 0 && flux_core::chars::is_keyword(chars[start - 1]) {
                        start -= 1;
                    }
                }
                self.compl.clear();
                self.compl.line = cur.line;
                self.compl.col = start;
                self.compl.length = cur.col - start;
                self.compl.orig = chars[start..cur.col].iter().collect();
                self.compl.redo_base = Some(self.compl.orig.clone());
                editor.completion.submode.text = Some(
                    if self.compl.local {
                        MSG_LOCAL
                    } else {
                        MSG_KEYWORD
                    }
                    .into(),
                );
                let orig = Match {
                    original: true,
                    icase: editor.options.ignorecase,
                    ..Match::new(self.compl.orig.clone())
                };
                self.compl.add(orig, false, true);
                self.compl.started = true;
                true
            }
            _ => false,
        }
    }

    /// Show the matches `items` for the text from column `start` to the cursor (Vim's
    /// `complete()`), the first one inserted.
    pub(crate) fn set_completion(&mut self, editor: &mut Editor, start: usize, items: Vec<Match>) {
        if self.compl.mode != XMode::Normal {
            self.compl_prep(editor, Key::char(' '));
        }
        editor.pum_undisplay();
        self.compl.clear();
        let cur = editor.cursor();
        let start = start.min(cur.col);
        self.compl.forward = true;
        self.compl.col = start;
        self.compl.line = cur.line;
        self.compl.length = cur.col - start;
        let line = util::line(editor, cur.line);
        self.compl.orig = line.chars().skip(start).take(cur.col - start).collect();
        self.compl.redo_base = Some(self.compl.orig.clone());
        let orig = Match {
            original: true,
            icase: editor.options.ignorecase,
            ..Match::new(self.compl.orig.clone())
        };
        self.compl.add(orig, false, true);
        self.compl.mode = XMode::Eval;
        for m in items {
            if m.word.is_empty() && m.lsp.is_none() {
                continue;
            }
            self.compl.add(m, true, true);
        }
        self.compl.count = self.compl.make_cyclic();
        self.compl.started = true;
        self.compl.used_match = true;
        self.compl.curr = self.compl.first;
        let no_insert = editor.completeopt("noinsert");
        let no_select = editor.completeopt("noselect") || editor.completeopt("longest");
        if no_insert || no_select {
            self.compl_set_selection(editor, Key::plain(KeyCode::Down));
            if no_select {
                self.compl_set_selection(editor, Key::plain(KeyCode::Up));
            }
        } else {
            self.compl_set_selection(editor, Key::ctrl('n'));
        }
        self.compl.enter_selects = no_insert;
        self.compl_show_pum(editor);
    }

    /// `ins_complete` without showing the menu.
    fn compl_set_selection(&mut self, editor: &mut Editor, key: Key) {
        let forward = key_forward(key);
        self.compl.forward = forward;
        self.compl.shown = self.compl.curr;
        self.compl.shows_forward = forward;
        let n = self.compl_next(editor, true, 1, use_match(key));
        if n > 1 {
            self.compl.count = n as usize;
        }
        self.compl.curr = self.compl.shown;
        self.compl.forward = self.compl.shows_forward;
        self.compl_statusmsg(editor);
    }

    /// Go `count` matches on and insert the match (Vim's `ins_compl_next`). Returns how many
    /// matches were found when they were looked for, or -1.
    fn compl_next(
        &mut self,
        editor: &mut Editor,
        allow_get_expansion: bool,
        count: usize,
        insert_match: bool,
    ) -> isize {
        let Some(shown) = self.compl.shown else {
            return -1;
        };
        if self.compl.leader.is_some() && !self.compl.matches[shown].original {
            self.compl_update_shown_match();
        }
        if allow_get_expansion && insert_match {
            self.compl_delete(editor, false);
        }
        let mut advance = true;
        if self.compl.restarting {
            advance = false;
            self.compl.restarting = false;
        }
        let num = self.compl_find_next(editor, allow_get_expansion, count, advance);
        if num == -2 {
            return -1;
        }
        if insert_match {
            self.compl_insert(editor);
        } else {
            self.compl.used_match = false;
        }
        self.compl.enter_selects = !insert_match && self.compl.array.is_some();
        num
    }

    /// Make the shown match one that fits the typed text (Vim's
    /// `ins_compl_update_shown_match`).
    fn compl_update_shown_match(&mut self) {
        let c = &mut self.compl;
        let Some(mut s) = c.shown else {
            return;
        };
        let leader = c.leader.clone().unwrap_or_default();
        while !c.matches[s].equal(&leader)
            && let Some(n) = c.matches[s].next
            && !c.is_first(n)
        {
            s = n;
        }
        if !c.shows_forward
            && !c.matches[s].equal(&leader)
            && c.matches[s].next.is_none_or(|n| c.is_first(n))
        {
            while !c.matches[s].equal(&leader)
                && let Some(p) = c.matches[s].prev
                && !c.is_first(p)
            {
                s = p;
            }
        }
        c.shown = Some(s);
    }

    /// The next match in the menu: skips those the typed text hides.
    fn next_in_menu(&self) -> usize {
        let c = &self.compl;
        let mut m = c.shown.expect("a shown match");
        loop {
            m = if c.shows_forward {
                c.next_of(m)
            } else {
                c.prev_of(m)
            }
            .unwrap_or(m);
            let ma = &c.matches[m];
            if !(ma.next.is_some() && !ma.in_array && !ma.original) {
                return m;
            }
        }
    }

    /// Move the shown match `todo` times (Vim's `find_next_completion_match`). Returns the
    /// number of matches found if they were looked for, -1 otherwise, -2 when there's
    /// nowhere to go.
    fn compl_find_next(
        &mut self,
        editor: &mut Editor,
        allow_get_expansion: bool,
        todo: usize,
        advance: bool,
    ) -> isize {
        let mut num = -1;
        let mut found_end;
        let mut found = None;
        let mut todo = todo as isize;
        while todo > 0 {
            todo -= 1;
            let shown = self.compl.shown.expect("a shown match");
            let forward = self.compl.shows_forward;
            if forward && self.compl.next_of(shown).is_some() {
                let s = if self.compl.array.is_some() {
                    self.next_in_menu()
                } else {
                    self.compl.next_of(shown).expect("checked")
                };
                self.compl.shown = Some(s);
                found_end = self.compl.first.is_some()
                    && (self
                        .compl
                        .next_of(s)
                        .is_some_and(|n| self.compl.is_first(n))
                        || self.compl.is_first(s));
            } else if !forward && self.compl.prev_of(shown).is_some() {
                found_end = self.compl.is_first(shown);
                let s = if self.compl.array.is_some() {
                    self.next_in_menu()
                } else {
                    self.compl.prev_of(shown).expect("checked")
                };
                self.compl.shown = Some(s);
                found_end |= self.compl.is_first(s);
            } else {
                if !allow_get_expansion {
                    return -2;
                }
                num = self.compl_get_exp(editor) as isize;
                // Go to the first match found in the direction of completion.
                if advance {
                    let s = self.compl.shown.expect("a shown match");
                    let next = if self.compl.shows_forward {
                        self.compl.next_of(s)
                    } else {
                        self.compl.prev_of(s)
                    };
                    if let Some(n) = next {
                        self.compl.shown = Some(n);
                    }
                }
                found_end = false;
            }
            let s = self.compl.shown.expect("a shown match");
            let m = &self.compl.matches[s];
            match &self.compl.leader {
                Some(leader) if !m.original && !m.equal(leader) => todo += 1,
                _ => found = Some(s),
            }
            if found_end {
                if let Some(f) = found {
                    self.compl.shown = Some(f);
                    break;
                }
                todo = 1;
            }
        }
        num
    }

    /// Look for keyword matches (Vim's `ins_compl_get_exp` for 'complete'): in the current
    /// buffer from the cursor on in the direction of completion, then in the other buffers.
    fn compl_get_exp(&mut self, editor: &mut Editor) -> usize {
        let prefix = self.compl.orig.clone();
        let forward = self.compl.forward;
        let icase = editor.options.ignorecase;
        let start = pos(self.compl.line, self.compl.col);
        let cur = editor.cursor();
        let mut words: Vec<(String, Option<String>)> = Vec::new();
        let current = editor.current_buffer().id;
        let text = editor.text();
        for w in keyword_matches(text, &prefix, cur, start, forward, icase, true) {
            words.push((w, None));
        }
        if !self.compl.local {
            let mut others: Vec<flux_view::BufferId> = editor
                .windows
                .iter()
                .map(|w| w.buffer)
                .filter(|&b| b != current)
                .collect();
            for b in &editor.buffers {
                if b.id != current && !others.contains(&b.id) && b.listed {
                    others.push(b.id);
                }
            }
            let cwd = editor.cwd.clone();
            for id in others {
                let Some(b) = editor.buffer(id) else {
                    continue;
                };
                if b.directory {
                    continue;
                }
                let name = b
                    .path
                    .as_ref()
                    .map(|p| p.strip_prefix(&cwd).unwrap_or(p).display().to_string());
                let begin = if forward {
                    pos(0, 0)
                } else {
                    pos(b.text.last_line(), b.text.line_len(b.text.last_line()))
                };
                for w in keyword_matches(&b.text, &prefix, begin, begin, forward, icase, false) {
                    words.push((w, name.clone()));
                }
            }
        }
        for (w, fname) in words {
            let m = Match {
                icase,
                fname,
                ..Match::new(w)
            };
            self.compl.add(m, false, forward);
        }
        self.compl.make_cyclic()
    }

    /// Take away the text of the match shown (Vim's `ins_compl_delete`). With `new_leader`,
    /// what the leader shares with the original text stays.
    fn compl_delete(&mut self, editor: &mut Editor, new_leader: bool) {
        let mut orig_col = 0;
        if new_leader {
            let leader = self.compl.leader_or_orig().to_string();
            orig_col = self
                .compl
                .orig
                .chars()
                .zip(leader.chars())
                .take_while(|(a, b)| a == b)
                .count();
        }
        let col = self.compl.col + orig_col;
        let cur = editor.cursor();
        if cur.line == self.compl.line && cur.col > col {
            let start = editor.text().line_start(cur.line);
            self.edit(editor, Edit::delete(start + col..start + cur.col));
            editor.window.cursor.col = col;
        }
        self.compl.completed = None;
    }

    /// Insert the rest of the match shown (Vim's `ins_compl_insert`).
    fn compl_insert(&mut self, editor: &mut Editor) {
        let Some(s) = self.compl.shown else {
            return;
        };
        let len = self.compl_len(editor);
        let word = self.compl.matches[s].word.clone();
        // Only the first line of a match that has more.
        let rest: String = word.chars().skip(len).collect();
        if !rest.is_empty() {
            self.compl_insert_text(editor, &rest);
        }
        self.compl.used_match = !self.compl.matches[s].original;
        self.compl.completed = Some(s);
    }

    /// Insert `text` at the cursor, which goes after it.
    fn compl_insert_text(&mut self, editor: &mut Editor, text: &str) {
        let cur = editor.cursor();
        let at = editor.text().pos_to_char(cur.line, cur.col);
        self.edit(editor, Edit::insert(at, text));
        let end = at + text.chars().count();
        let (line, col) = editor.text().char_to_pos(end);
        editor.window.cursor = pos(line, col);
    }

    /// The length of the completed text so far.
    fn compl_len(&self, editor: &Editor) -> usize {
        editor.cursor().col.saturating_sub(self.compl.col)
    }

    /// `<BS>` while completing: shorten the typed text and show the matches for it (Vim's
    /// `ins_compl_bs`). Returns false when it's an ordinary `<BS>` (which ends completion).
    fn compl_bs(&mut self, editor: &mut Editor) -> bool {
        let cur = editor.cursor();
        let line = util::line(editor, cur.line);
        let p = flux_core::chars::prev_grapheme(&line, cur.col);
        let c = &self.compl;
        if p < c.col || (p == c.col && c.mode != XMode::Omni) || c.mode == XMode::Eval {
            return false;
        }
        if cur.col <= c.col + c.length {
            self.compl_restart(editor);
        }
        let leader: String = line
            .chars()
            .skip(self.compl.col)
            .take(p - self.compl.col)
            .collect();
        self.compl.leader = Some(leader);
        self.compl_new_leader(editor);
        if self.compl.shown.is_some() {
            self.compl.curr = self.compl.shown;
        }
        true
    }

    /// Look for matches again from scratch (Vim's `ins_compl_restart`).
    fn compl_restart(&mut self, editor: &mut Editor) {
        editor.pum_undisplay();
        self.compl.clear();
        self.compl.started = false;
        self.compl.count = 0;
    }

    /// A character typed while the menu is shown: it goes in, and the menu shows the matches
    /// that start with what's typed (Vim's `ins_compl_addleader`).
    fn compl_addleader(&mut self, editor: &mut Editor, c: char) {
        self.compl_insert_text(editor, &c.to_string());
        let cur = editor.cursor();
        let line = util::line(editor, cur.line);
        let leader: String = line
            .chars()
            .skip(self.compl.col)
            .take(cur.col - self.compl.col)
            .collect();
        self.compl.leader = Some(leader);
        self.compl_new_leader(editor);
    }

    /// The typed text changed: show the matches for it (Vim's `ins_compl_new_leader`).
    fn compl_new_leader(&mut self, editor: &mut Editor) {
        self.compl.array = None;
        editor.completion.pum = None;
        self.compl_delete(editor, true);
        let len = self.compl_len(editor);
        let rest: String = self.compl.leader_or_orig().chars().skip(len).collect();
        if !rest.is_empty() {
            self.compl_insert_text(editor, &rest);
        }
        self.compl.used_match = false;
        if self.compl.started {
            let leader = self.compl.leader.clone().unwrap_or_default();
            if let Some(f) = self.compl.first {
                self.compl.matches[f].word = leader;
            }
        } else {
            // Look for matches again, without inserting the first.
            self.compl.restarting = true;
            if self.compl.mode == XMode::Normal {
                self.compl_do(editor, Key::ctrl('n'));
            }
            self.compl.restarting = false;
        }
        self.compl.enter_selects = !self.compl.used_match && self.compl.selected_item.is_some();
        self.compl_show_pum(editor);
        if self.compl.array.is_none() {
            self.compl.enter_selects = false;
        }
    }

    /// The mode message's second part: which match this is (Vim's
    /// `ins_compl_show_statusmsg`).
    fn compl_statusmsg(&mut self, editor: &mut Editor) {
        let c = &mut self.compl;
        let mut extra: Option<(String, Option<&'static str>)> = None;
        if c.no_matches() {
            extra = Some(("Pattern not found".into(), Some("ErrorMsg")));
        }
        if extra.is_none()
            && let Some(cm) = c.curr
        {
            if c.matches[cm].original {
                extra = Some(("Back at original".into(), Some("WarningMsg")));
            } else if c.matches[cm].next == c.matches[cm].prev {
                extra = Some(("The only match".into(), None));
                c.matches[cm].number = Some(1);
            } else {
                if c.matches[cm].number.is_none() {
                    update_numbers(c, cm);
                }
                if let Some(n) = c.matches[cm].number {
                    let text = if c.count > 0 {
                        format!("match {n} of {}", c.count)
                    } else {
                        format!("match {n}")
                    };
                    extra = Some((text, Some("Question")));
                }
            }
        }
        editor.completion.submode.extra = extra;
    }

    /// Show the menu for the matches (Vim's `ins_compl_show_pum`).
    fn compl_show_pum(&mut self, editor: &mut Editor) {
        if !Self::pum_wanted(editor) || !self.enough_matches(editor) {
            return;
        }
        let mut cur = None;
        let changed = self.compl.array.is_none();
        if changed {
            cur = self.build_pum();
        } else if let (Some(array), Some(s)) = (&self.compl.array, self.compl.shown) {
            cur = array.iter().position(|&i| i == s);
        }
        let Some(array) = self.compl.array.clone() else {
            return;
        };
        self.compl.selected_item = cur;
        let items = changed.then(|| array.iter().map(|&i| self.compl.pum_item(i)).collect());
        let leader = self.compl.leader_or_orig().to_string();
        editor.pum_display(items, cur, &leader, self.compl.col);
        if self.compl.started && self.compl.curr != self.compl.shown {
            self.compl.curr = self.compl.shown;
        }
        // CompleteChanged: a language server's match may have documentation to show.
        crate::lsp::completion::complete_changed(self, editor, cur);
    }

    /// At least two matches, or one with 'completeopt' `menuone`.
    fn enough_matches(&self, editor: &Editor) -> bool {
        let n = self
            .compl
            .iter()
            .iter()
            .filter(|&&i| !self.compl.matches[i].original)
            .count();
        if editor.completeopt("menuone") {
            n >= 1
        } else {
            n >= 2
        }
    }

    /// The matches the typed text leaves, for the menu; returns the shown one's place (Vim's
    /// `ins_compl_build_pum`).
    fn build_pum(&mut self) -> Option<usize> {
        let c = &mut self.compl;
        if c.leader.as_deref() == Some(c.orig.as_str())
            && c.shown.is_some_and(|s| !c.matches[s].original)
        {
            c.shown = c.first.and_then(|f| c.matches[f].next);
        }
        let mut shown_match_ok = c.shown.is_some_and(|s| c.matches[s].original);
        let mut did_find_shown = false;
        let mut shown_compl = None;
        let mut cur = None;
        let mut array = Vec::new();
        for i in c.iter() {
            c.matches[i].in_array = false;
            let fits = match &c.leader {
                None => true,
                Some(l) => c.matches[i].equal(l),
            };
            if !c.matches[i].original && fits {
                c.matches[i].in_array = true;
                if !shown_match_ok {
                    if Some(i) == c.shown || did_find_shown {
                        c.shown = Some(i);
                        did_find_shown = true;
                        shown_match_ok = true;
                    } else {
                        shown_compl = Some(i);
                    }
                    cur = Some(array.len());
                }
                array.push(i);
            }
            if Some(i) == c.shown {
                did_find_shown = true;
                if c.matches[i].original {
                    shown_match_ok = true;
                }
                if !shown_match_ok && shown_compl.is_some() {
                    c.shown = shown_compl;
                    shown_match_ok = true;
                }
            }
        }
        if array.is_empty() {
            return None;
        }
        c.array = Some(array);
        if !shown_match_ok {
            cur = None;
        }
        cur
    }

    /// Record for `.` what completing changed: back over the typed text that differs, then
    /// type the rest (Vim's `ins_compl_fixRedoBufForLeader`).
    fn compl_fix_redo(&mut self, editor: &mut Editor) {
        let Some(base) = self.compl.redo_base.take() else {
            return;
        };
        let cur = editor.cursor();
        if cur.line != self.compl.line || cur.col < self.compl.col {
            return;
        }
        let now: String = util::line(editor, cur.line)
            .chars()
            .skip(self.compl.col)
            .take(cur.col - self.compl.col)
            .collect();
        self.redo_retype(&base, &now);
    }
}

/// Number the matches from the last numbered one (Vim's `ins_compl_update_sequence_numbers`).
fn update_numbers(c: &mut Completion, cm: usize) {
    let mut number = 0;
    if c.forward {
        let mut m = c.matches[cm].prev;
        while let Some(i) = m {
            if c.is_first(i) {
                break;
            }
            if let Some(n) = c.matches[i].number {
                number = n;
                break;
            }
            m = c.matches[i].prev;
        }
        let mut m = m.and_then(|i| c.matches[i].next);
        while let Some(i) = m {
            if c.matches[i].number.is_some() {
                break;
            }
            number += 1;
            c.matches[i].number = Some(number);
            m = c.matches[i].next;
        }
    } else {
        let mut m = c.matches[cm].next;
        while let Some(i) = m {
            if c.is_first(i) {
                break;
            }
            if let Some(n) = c.matches[i].number {
                number = n;
                break;
            }
            m = c.matches[i].next;
        }
        let mut m = m.and_then(|i| c.matches[i].prev);
        while let Some(i) = m {
            if c.matches[i].number.is_some() {
                break;
            }
            number += 1;
            c.matches[i].number = Some(number);
            m = c.matches[i].prev;
        }
    }
}

/// The words in `text` that start with `prefix` at the start of a keyword, in the order a
/// search from `from` in the direction finds them, wrapping around when `wrap`. Without a
/// prefix, words of at least two characters. `skip` (the text being completed) isn't one.
fn keyword_matches(
    text: &flux_core::Text,
    prefix: &str,
    from: flux_view::Cursor,
    skip: flux_view::Cursor,
    forward: bool,
    icase: bool,
    wrap: bool,
) -> Vec<String> {
    let lower = |s: &str| {
        if icase {
            s.to_lowercase()
        } else {
            s.to_string()
        }
    };
    let want = lower(prefix);
    let min = if prefix.is_empty() {
        2
    } else {
        prefix.chars().count() + 1
    };
    let mut found: Vec<(flux_view::Cursor, String)> = Vec::new();
    for l in 0..text.line_count() {
        let chars: Vec<char> = text.line_str(l).chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if flux_core::chars::is_keyword(chars[i])
                && (i == 0 || !flux_core::chars::is_keyword(chars[i - 1]))
            {
                let mut e = i;
                while e < chars.len() && flux_core::chars::is_keyword(chars[e]) {
                    e += 1;
                }
                let word: String = chars[i..e].iter().collect();
                let at = pos(l, i);
                if e - i >= min && lower(&word).starts_with(&want) && at != skip {
                    found.push((at, word));
                }
                i = e;
            } else {
                i += 1;
            }
        }
    }
    let key = |c: &flux_view::Cursor| (c.line, c.col);
    let (mut after, mut before): (Vec<_>, Vec<_>) = if forward {
        found.into_iter().partition(|(c, _)| key(c) > key(&from))
    } else {
        let (a, b): (Vec<_>, Vec<_>) = found.into_iter().partition(|(c, _)| key(c) < key(&from));
        (a.into_iter().rev().collect(), b.into_iter().rev().collect())
    };
    if wrap {
        after.append(&mut before);
    }
    after.into_iter().map(|(_, w)| w).collect()
}
