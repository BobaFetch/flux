//! Options (`:set`): the ones flux implements, with Vim's names, scopes and defaults.

/// Options local to a buffer ('tabstop', …). New buffers copy the global values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferOptions {
    pub tabstop: usize,
    pub shiftwidth: usize,
    pub softtabstop: isize,
    pub expandtab: bool,
    pub autoindent: bool,
    /// 'filetype': set when a file is read (see [`crate::filetype`]), or by `:set ft=`.
    pub filetype: String,
    /// 'matchpairs': the brackets MatchParen (and `%`) pair up, as `(:),{:},[:]`.
    pub matchpairs: String,
    /// 'indentexpr': which of flux's indenters (named as Neovim's ftplugins name their
    /// functions, see `flux_vim::indent`) indents the buffer. Empty for none.
    pub indentexpr: String,
    /// 'indentkeys' (with 'indentexpr') and 'cinkeys' (without): keys that reindent the line.
    pub indentkeys: String,
    pub cinkeys: String,
    /// 'cindent' and its 'cinoptions' and 'cinwords'.
    pub cindent: bool,
    pub cinoptions: String,
    pub cinwords: String,
    /// 'comments', 'formatoptions' and 'textwidth': comment leaders, and how text is
    /// formatted while typing.
    pub comments: String,
    pub formatoptions: String,
    pub textwidth: usize,
}

/// The default 'cinkeys' and 'indentkeys'.
pub const DEFAULT_CINKEYS: &str = "0{,0},0),0],:,0#,!^F,o,O,e";

impl Default for BufferOptions {
    fn default() -> Self {
        Self {
            tabstop: 8,
            shiftwidth: 8,
            softtabstop: 0,
            expandtab: false,
            autoindent: true,
            filetype: String::new(),
            matchpairs: "(:),{:},[:]".into(),
            indentexpr: String::new(),
            indentkeys: DEFAULT_CINKEYS.into(),
            cinkeys: DEFAULT_CINKEYS.into(),
            cindent: false,
            cinoptions: String::new(),
            cinwords: "if,else,while,do,for,switch".into(),
            comments: "s1:/*,mb:*,ex:*/,://,b:#,:%,:XCOMM,n:>,fb:-".into(),
            formatoptions: "tcqj".into(),
            textwidth: 0,
        }
    }
}

impl BufferOptions {
    /// 'shiftwidth', or 'tabstop' when it's 0 (Vim's `get_sw_value`).
    pub fn sw(&self) -> usize {
        if self.shiftwidth == 0 {
            self.tabstop
        } else {
            self.shiftwidth
        }
    }

    /// 'softtabstop', with a negative value meaning 'shiftwidth' (Vim's `get_sts_value`).
    pub fn sts(&self) -> usize {
        if self.softtabstop < 0 {
            self.sw()
        } else {
            self.softtabstop as usize
        }
    }
}

/// Options local to a window ('number', …). A split copies them from the window split.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowOptions {
    pub number: bool,
    pub relativenumber: bool,
    pub numberwidth: usize,
    /// 'signcolumn': `auto` (when there are signs), `yes`, or `no`.
    pub signcolumn: String,
}

impl Default for WindowOptions {
    fn default() -> Self {
        Self {
            number: false,
            relativenumber: false,
            numberwidth: 4,
            signcolumn: "auto".into(),
        }
    }
}

/// Global options, and the global values of local ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub smarttab: bool,
    pub ignorecase: bool,
    pub smartcase: bool,
    pub wrapscan: bool,
    pub hlsearch: bool,
    pub incsearch: bool,
    pub hidden: bool,
    pub splitbelow: bool,
    pub splitright: bool,
    pub gdefault: bool,
    pub report: usize,
    /// 'background': `dark` or `light`, which colors the default colorscheme uses.
    pub background: String,
    /// 'termguicolors': 24-bit colors. flux turns it on when `$COLORTERM` says the terminal
    /// has them, as Neovim does.
    pub termguicolors: bool,
    /// 'completeopt', 'pumheight' and 'pumwidth': how Insert-mode completion shows its
    /// matches (see `flux_vim::completion`).
    pub completeopt: String,
    pub pumheight: usize,
    pub pumwidth: usize,
    pub buffer: BufferOptions,
    pub window: WindowOptions,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            smarttab: true,
            ignorecase: false,
            smartcase: false,
            wrapscan: true,
            hlsearch: true,
            incsearch: true,
            hidden: true,
            splitbelow: false,
            splitright: false,
            gdefault: false,
            report: 2,
            background: "dark".into(),
            termguicolors: false,
            completeopt: "menu,popup".into(),
            pumheight: 0,
            pumwidth: 15,
            buffer: BufferOptions::default(),
            window: WindowOptions::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Global,
    Buffer,
    Window,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Number,
    String,
}

/// An option's description.
#[derive(Debug, Clone, Copy)]
pub struct OptionDef {
    pub name: &'static str,
    pub short: &'static str,
    pub kind: Kind,
    pub scope: Scope,
}

const fn def(name: &'static str, short: &'static str, kind: Kind, scope: Scope) -> OptionDef {
    OptionDef {
        name,
        short,
        kind,
        scope,
    }
}

/// Every option flux implements, in alphabetical order (as `:set all` lists them).
pub const OPTIONS: &[OptionDef] = &[
    def("autoindent", "ai", Kind::Bool, Scope::Buffer),
    def("background", "bg", Kind::String, Scope::Global),
    def("cindent", "cin", Kind::Bool, Scope::Buffer),
    def("cinkeys", "cink", Kind::String, Scope::Buffer),
    def("cinoptions", "cino", Kind::String, Scope::Buffer),
    def("cinwords", "cinw", Kind::String, Scope::Buffer),
    def("comments", "com", Kind::String, Scope::Buffer),
    def("completeopt", "cot", Kind::String, Scope::Global),
    def("expandtab", "et", Kind::Bool, Scope::Buffer),
    def("filetype", "ft", Kind::String, Scope::Buffer),
    def("formatoptions", "fo", Kind::String, Scope::Buffer),
    def("gdefault", "gd", Kind::Bool, Scope::Global),
    def("hidden", "hid", Kind::Bool, Scope::Global),
    def("hlsearch", "hls", Kind::Bool, Scope::Global),
    def("ignorecase", "ic", Kind::Bool, Scope::Global),
    def("incsearch", "is", Kind::Bool, Scope::Global),
    def("indentexpr", "inde", Kind::String, Scope::Buffer),
    def("indentkeys", "indk", Kind::String, Scope::Buffer),
    def("matchpairs", "mps", Kind::String, Scope::Buffer),
    def("number", "nu", Kind::Bool, Scope::Window),
    def("pumheight", "ph", Kind::Number, Scope::Global),
    def("pumwidth", "pw", Kind::Number, Scope::Global),
    def("numberwidth", "nuw", Kind::Number, Scope::Window),
    def("relativenumber", "rnu", Kind::Bool, Scope::Window),
    def("report", "", Kind::Number, Scope::Global),
    def("signcolumn", "scl", Kind::String, Scope::Window),
    def("shiftwidth", "sw", Kind::Number, Scope::Buffer),
    def("smartcase", "scs", Kind::Bool, Scope::Global),
    def("smarttab", "sta", Kind::Bool, Scope::Global),
    def("softtabstop", "sts", Kind::Number, Scope::Buffer),
    def("splitbelow", "sb", Kind::Bool, Scope::Global),
    def("splitright", "spr", Kind::Bool, Scope::Global),
    def("tabstop", "ts", Kind::Number, Scope::Buffer),
    def("termguicolors", "tgc", Kind::Bool, Scope::Global),
    def("textwidth", "tw", Kind::Number, Scope::Buffer),
    def("wrapscan", "ws", Kind::Bool, Scope::Global),
];

/// The option called `name` (full or short name).
pub fn find(name: &str) -> Option<&'static OptionDef> {
    OPTIONS
        .iter()
        .find(|o| o.name == name || (!o.short.is_empty() && o.short == name))
}

/// A value, as `:set` handles it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Bool(bool),
    Number(i64),
    String(String),
}

impl Options {
    pub fn get_global(&self, name: &str) -> Option<Value> {
        let b = &self.buffer;
        let w = &self.window;
        Some(match name {
            "smarttab" => Value::Bool(self.smarttab),
            "ignorecase" => Value::Bool(self.ignorecase),
            "smartcase" => Value::Bool(self.smartcase),
            "wrapscan" => Value::Bool(self.wrapscan),
            "hlsearch" => Value::Bool(self.hlsearch),
            "incsearch" => Value::Bool(self.incsearch),
            "hidden" => Value::Bool(self.hidden),
            "splitbelow" => Value::Bool(self.splitbelow),
            "splitright" => Value::Bool(self.splitright),
            "gdefault" => Value::Bool(self.gdefault),
            "report" => Value::Number(self.report as i64),
            "background" => Value::String(self.background.clone()),
            "termguicolors" => Value::Bool(self.termguicolors),
            "completeopt" => Value::String(self.completeopt.clone()),
            "pumheight" => Value::Number(self.pumheight as i64),
            "pumwidth" => Value::Number(self.pumwidth as i64),
            _ => return b.get(name).or_else(|| w.get(name)),
        })
    }

    pub fn set_global(&mut self, name: &str, v: Value) {
        match (name, v) {
            ("smarttab", Value::Bool(x)) => self.smarttab = x,
            ("ignorecase", Value::Bool(x)) => self.ignorecase = x,
            ("smartcase", Value::Bool(x)) => self.smartcase = x,
            ("wrapscan", Value::Bool(x)) => self.wrapscan = x,
            ("hlsearch", Value::Bool(x)) => self.hlsearch = x,
            ("incsearch", Value::Bool(x)) => self.incsearch = x,
            ("hidden", Value::Bool(x)) => self.hidden = x,
            ("splitbelow", Value::Bool(x)) => self.splitbelow = x,
            ("splitright", Value::Bool(x)) => self.splitright = x,
            ("gdefault", Value::Bool(x)) => self.gdefault = x,
            ("report", Value::Number(x)) => self.report = x.max(0) as usize,
            ("background", Value::String(x)) => self.background = x,
            ("termguicolors", Value::Bool(x)) => self.termguicolors = x,
            ("completeopt", Value::String(x)) => self.completeopt = x,
            ("pumheight", Value::Number(x)) => self.pumheight = x.max(0) as usize,
            ("pumwidth", Value::Number(x)) => self.pumwidth = x.max(0) as usize,
            (_, v) => {
                self.buffer.set(name, v.clone());
                self.window.set(name, v);
            }
        }
    }
}

impl BufferOptions {
    pub fn get(&self, name: &str) -> Option<Value> {
        Some(match name {
            "tabstop" => Value::Number(self.tabstop as i64),
            "shiftwidth" => Value::Number(self.shiftwidth as i64),
            "softtabstop" => Value::Number(self.softtabstop as i64),
            "expandtab" => Value::Bool(self.expandtab),
            "autoindent" => Value::Bool(self.autoindent),
            "filetype" => Value::String(self.filetype.clone()),
            "matchpairs" => Value::String(self.matchpairs.clone()),
            "indentexpr" => Value::String(self.indentexpr.clone()),
            "indentkeys" => Value::String(self.indentkeys.clone()),
            "cinkeys" => Value::String(self.cinkeys.clone()),
            "cindent" => Value::Bool(self.cindent),
            "cinoptions" => Value::String(self.cinoptions.clone()),
            "cinwords" => Value::String(self.cinwords.clone()),
            "comments" => Value::String(self.comments.clone()),
            "formatoptions" => Value::String(self.formatoptions.clone()),
            "textwidth" => Value::Number(self.textwidth as i64),
            _ => return None,
        })
    }

    pub fn set(&mut self, name: &str, v: Value) {
        match (name, v) {
            ("tabstop", Value::Number(x)) => self.tabstop = x.max(1) as usize,
            ("shiftwidth", Value::Number(x)) => self.shiftwidth = x.max(0) as usize,
            ("softtabstop", Value::Number(x)) => self.softtabstop = x as isize,
            ("expandtab", Value::Bool(x)) => self.expandtab = x,
            ("autoindent", Value::Bool(x)) => self.autoindent = x,
            ("filetype", Value::String(x)) => self.filetype = x,
            ("matchpairs", Value::String(x)) => self.matchpairs = x,
            ("indentexpr", Value::String(x)) => self.indentexpr = x,
            ("indentkeys", Value::String(x)) => self.indentkeys = x,
            ("cinkeys", Value::String(x)) => self.cinkeys = x,
            ("cindent", Value::Bool(x)) => self.cindent = x,
            ("cinoptions", Value::String(x)) => self.cinoptions = x,
            ("cinwords", Value::String(x)) => self.cinwords = x,
            ("comments", Value::String(x)) => self.comments = x,
            ("formatoptions", Value::String(x)) => self.formatoptions = x,
            ("textwidth", Value::Number(x)) => self.textwidth = x.max(0) as usize,
            _ => {}
        }
    }
}

impl WindowOptions {
    pub fn get(&self, name: &str) -> Option<Value> {
        Some(match name {
            "number" => Value::Bool(self.number),
            "relativenumber" => Value::Bool(self.relativenumber),
            "numberwidth" => Value::Number(self.numberwidth as i64),
            "signcolumn" => Value::String(self.signcolumn.clone()),
            _ => return None,
        })
    }

    pub fn set(&mut self, name: &str, v: Value) {
        match (name, v) {
            ("number", Value::Bool(x)) => self.number = x,
            ("relativenumber", Value::Bool(x)) => self.relativenumber = x,
            ("numberwidth", Value::Number(x)) => self.numberwidth = x.clamp(1, 20) as usize,
            ("signcolumn", Value::String(x)) => self.signcolumn = x,
            _ => {}
        }
    }

    /// Width of the number column (Vim's `number_width` plus the space after it), or 0.
    pub fn number_width(&self, line_count: usize, height: usize) -> usize {
        if !self.number && !self.relativenumber {
            return 0;
        }
        let n = if self.relativenumber && !self.number {
            height
        } else {
            line_count
        };
        let digits = n.max(1).to_string().len();
        digits.max(self.numberwidth.saturating_sub(1)) + 1
    }
}
