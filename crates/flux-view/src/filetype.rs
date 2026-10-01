//! Filetypes: detected when a file is read, they bring the settings Neovim's ftplugins make
//! and the tree-sitter parser used for highlighting.

use crate::options::BufferOptions;
use crate::{BufferId, Editor};

/// `:filetype` settings: detection, ftplugin settings, and indenting. All on, as in Neovim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FiletypeSettings {
    pub detection: bool,
    pub plugin: bool,
    pub indent: bool,
}

impl Default for FiletypeSettings {
    fn default() -> Self {
        Self {
            detection: true,
            plugin: true,
            indent: true,
        }
    }
}

/// The options Neovim's ftplugins set (`nvim --clean`, each filetype's recommended style).
fn ftplugin(filetype: &str, o: &mut BufferOptions) {
    let c_comments = "sO:* -,mO:*  ,exO:*/,s1:/*,mb:*,ex:*/,:///,://";
    let js_comments = "sO:* -,mO:*  ,exO:*/,s1:/*,mb:*,ex:*/,://";
    let mut style = |ts: usize, sw: usize, sts: isize| {
        o.tabstop = ts;
        o.shiftwidth = sw;
        o.softtabstop = sts;
        o.expandtab = true;
    };
    match filetype {
        "rust" => {
            style(8, 4, 4);
            o.textwidth = 100;
            o.matchpairs = "(:),{:},[:],<:>".into();
        }
        "python" => style(4, 4, 4),
        "markdown" => {
            style(4, 4, 4);
            o.matchpairs = "(:),{:},[:],<:>".into();
        }
        _ => {}
    }
    let (comments, formatoptions) = match filetype {
        "text" => ("fb:-,fb:*,n:>", "tcqj"),
        "rust" => ("s0:/*!,ex:*/,s1:/*,mb:*,ex:*/,:///,://!,://", "croqnlj"),
        "python" => ("b:#,fb:-", "tcqj"),
        "lua" => (":---,:--", "jcroql"),
        "toml" => (":#", "tcqj"),
        "json" => ("", "cqj"),
        "markdown" => ("fb:*,fb:-,fb:+,n:>", "jtcqln"),
        "sh" => ("b:#", "jcroql"),
        "javascript" | "javascriptreact" | "typescript" | "typescriptreact" => {
            (js_comments, "jcroql")
        }
        "c" | "cpp" => (c_comments, "jcroql"),
        _ => return,
    };
    o.comments = comments.into();
    o.formatoptions = formatoptions.into();
}

/// The options Neovim's indent scripts set: which indenter, and the keys that trigger it.
fn indent_plugin(filetype: &str, o: &mut BufferOptions) {
    let (expr, keys): (&str, Option<&str>) = match filetype {
        "rust" => {
            o.cindent = true;
            o.cinoptions = "L0,(s,Ws,J1,j1,m1".into();
            o.cinkeys = "0{,0},!^F,o,O,0[,0],0(,0)".into();
            o.cinwords =
                "for,if,else,while,loop,impl,mod,unsafe,trait,struct,enum,fn,extern,macro".into();
            ("GetRustIndent(v:lnum)", Some("0{,0},!^F,o,O,0[,0],0(,0)"))
        }
        "python" => {
            o.cinkeys = "0{,0},0),0],:,!^F,o,O,e".into();
            (
                "python#GetIndent(v:lnum)",
                Some("0{,0},0),0],:,!^F,o,O,e,<:>,=elif,=except"),
            )
        }
        "lua" => (
            "GetLuaIndent()",
            Some("0{,0},0),0],:,0#,!^F,o,O,e,0=end,0=until"),
        ),
        "json" => ("GetJSONIndent(v:lnum)", Some("0{,0},0),0[,0],!^F,o,O,e")),
        "sh" => (
            "GetShIndent()",
            Some(
                "0{,0},0),0],!^F,o,O,e,0=then,0=do,0=else,0=elif,0=fi,0=esac,0=done,0=end,),\
                 0=;;,0=;&,0=fin,0=fil,0=fip,0=fir,0=fix",
            ),
        ),
        "javascript" | "javascriptreact" => (
            "GetJavascriptIndent()",
            Some("0{,0},0),0],:,0#,!^F,o,O,e,0],0)"),
        ),
        "typescript" | "typescriptreact" => {
            ("GetTypescriptIndent()", Some("0{,0},0),0],0,,!^F,o,O,e"))
        }
        "c" | "cpp" => {
            o.cindent = true;
            ("", None)
        }
        _ => return,
    };
    o.indentexpr = expr.into();
    if let Some(k) = keys {
        o.indentkeys = k.into();
    }
}

impl Editor {
    /// Vim's filetype detection when a file is read into buffer `id`.
    pub fn detect_filetype(&mut self, id: BufferId) {
        if !self.filetype.detection {
            return;
        }
        let Some(buffer) = self.buffer(id) else {
            return;
        };
        if buffer.directory {
            return;
        }
        let detected = buffer.path.as_deref().and_then(|p| {
            let first = buffer.text.line_str(0);
            flux_syntax::detect(p, &first)
        });
        if let Some(ft) = detected {
            self.buffer_mut(id).expect("checked").opts.filetype = ft.to_string();
            self.apply_filetype(id);
        }
    }

    /// 'filetype' of buffer `id` was set (Vim's FileType event): apply its ftplugin settings
    /// and switch to its parser.
    pub fn apply_filetype(&mut self, id: BufferId) {
        let FiletypeSettings { plugin, indent, .. } = self.filetype;
        let Some(buffer) = self.buffer_mut(id) else {
            return;
        };
        let ft = buffer.opts.filetype.clone();
        if plugin {
            ftplugin(&ft, &mut buffer.opts);
        }
        if indent {
            indent_plugin(&ft, &mut buffer.opts);
        }
        let lang = flux_syntax::lang_for_filetype(&ft);
        if buffer.syntax.as_ref().map(|s| s.lang()) != lang {
            buffer.syntax = lang.and_then(flux_syntax::Syntax::new);
        }
    }

    /// Bring the syntax trees of the buffers on screen up to date with their text.
    pub fn update_syntax(&mut self) {
        self.update_syntax_within(Some(std::time::Duration::from_secs(10)));
    }

    /// Like [`Editor::update_syntax`], parsing each buffer for at most `budget` (`None`: only
    /// move the trees along with the edits). Returns whether a parse is still unfinished, to
    /// be continued by another call.
    pub fn update_syntax_within(&mut self, budget: Option<std::time::Duration>) -> bool {
        if !self.syntax_on {
            return false;
        }
        let shown: Vec<BufferId> = std::iter::once(self.window.buffer)
            .chain(self.windows.iter().map(|w| w.buffer))
            .collect();
        let mut pending = false;
        for buffer in self.buffers.iter_mut().filter(|b| shown.contains(&b.id)) {
            if let Some(syntax) = buffer.syntax.as_mut() {
                pending |= !syntax.update(&mut buffer.text, budget);
            }
        }
        pending
    }
}
