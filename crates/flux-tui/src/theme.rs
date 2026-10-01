//! The highlight groups a frame uses, resolved once for the editor's 'background' and
//! 'termguicolors'.

use std::cell::RefCell;
use std::collections::HashMap;

use flux_view::Editor;
use flux_view::highlight::{self, HlAttr, HlColor};

use crate::grid::{Color, Style};

fn color(c: Option<HlColor>) -> Color {
    match c {
        None => Color::Reset,
        Some(HlColor::Rgb(r, g, b)) => Color::Rgb(r, g, b),
        Some(HlColor::Index(n)) => Color::Ansi(n),
    }
}

fn style(a: HlAttr) -> Style {
    Style {
        fg: color(a.fg),
        bg: color(a.bg),
        sp: color(a.sp),
        bold: a.bold,
        italic: a.italic,
        underline: a.underline,
        undercurl: a.undercurl,
        strikethrough: a.strikethrough,
        reverse: a.reverse,
    }
}

pub struct Theme {
    light: bool,
    gui: bool,
    /// Normal: what `Reset` colors stand for.
    pub normal: Style,
    pub non_text: Style,
    pub end_of_buffer: Style,
    pub special_key: Style,
    pub directory: Style,
    pub error_msg: Style,
    pub warning_msg: Style,
    pub visual: Style,
    pub search: Style,
    pub cur_search: Style,
    pub inc_search: Style,
    pub substitute: Style,
    pub match_paren: Style,
    pub status_line: Style,
    pub status_line_nc: Style,
    pub line_nr: Style,
    pub cursor_line_nr: Style,
    pub win_separator: Style,
    pub more_msg: Style,
    pub mode_msg: Style,
    pub question: Style,
    pub sign_column: Style,
    /// DiagnosticSign{Error,Warn,Info,Hint} and DiagnosticUnderline…, by severity (1–4).
    pub diagnostic_sign: [Style; 4],
    pub diagnostic_underline: [Style; 4],
    /// Tree-sitter captures looked up so far (`keyword.function` → `@keyword.function`).
    captures: RefCell<HashMap<&'static str, Style>>,
    /// Other groups looked up so far.
    groups: RefCell<HashMap<String, Style>>,
    pub normal_float: Style,
}

impl Theme {
    pub fn new(editor: &Editor) -> Self {
        let light = editor.options.background == "light";
        let gui = editor.options.termguicolors;
        let g = |name: &str| style(highlight::resolve(name, light, gui));
        // Groups that link to Normal are drawn in the default colors.
        let normal = g("Normal");
        let relative = |s: Style| if s == normal { Style::default() } else { s };
        Self {
            light,
            gui,
            normal,
            non_text: relative(g("NonText")),
            end_of_buffer: relative(g("EndOfBuffer")),
            special_key: relative(g("SpecialKey")),
            directory: relative(g("Directory")),
            error_msg: relative(g("ErrorMsg")),
            warning_msg: relative(g("WarningMsg")),
            visual: relative(g("Visual")),
            search: relative(g("Search")),
            cur_search: relative(g("CurSearch")),
            inc_search: relative(g("IncSearch")),
            substitute: relative(g("Substitute")),
            match_paren: relative(g("MatchParen")),
            status_line: relative(g("StatusLine")),
            status_line_nc: relative(g("StatusLineNC")),
            line_nr: relative(g("LineNr")),
            cursor_line_nr: relative(g("CursorLineNr")),
            win_separator: relative(g("WinSeparator")),
            more_msg: relative(g("MoreMsg")),
            mode_msg: relative(g("ModeMsg")),
            question: relative(g("Question")),
            sign_column: relative(g("SignColumn")),
            diagnostic_sign: ["Error", "Warn", "Info", "Hint"]
                .map(|s| relative(g(&format!("DiagnosticSign{s}")))),
            diagnostic_underline: ["Error", "Warn", "Info", "Hint"]
                .map(|s| relative(g(&format!("DiagnosticUnderline{s}")))),
            captures: RefCell::default(),
            groups: RefCell::default(),
            normal_float: g("NormalFloat"),
        }
    }

    /// The style of highlight group `name`.
    pub fn group(&self, name: &str) -> Style {
        *self
            .groups
            .borrow_mut()
            .entry(name.to_string())
            .or_insert_with(|| style(highlight::resolve(name, self.light, self.gui)))
    }

    /// The style of tree-sitter capture `name`.
    pub fn capture(&self, name: &'static str) -> Style {
        *self.captures.borrow_mut().entry(name).or_insert_with(|| {
            style(highlight::resolve(
                &format!("@{name}"),
                self.light,
                self.gui,
            ))
        })
    }
}
