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

/// The options Neovim's ftplugins and indent scripts set (`nvim --clean`, each filetype's
/// recommended style), on top of the global values.
fn ftplugin(filetype: &str, opts: &mut BufferOptions) {
    let c_comments = "sO:* -,mO:*  ,exO:*/,s1:/*,mb:*,ex:*/,:///,://";
    let js_comments = "sO:* -,mO:*  ,exO:*/,s1:/*,mb:*,ex:*/,://";
    let set = |o: &mut BufferOptions, inde: &str, indk: Option<&str>, com: &str, fo: &str| {
        o.indentexpr = inde.into();
        if let Some(k) = indk {
            o.indentkeys = k.into();
        }
        o.comments = com.into();
        o.formatoptions = fo.into();
    };
    match filetype {
        "text" => set(opts, "", None, "fb:-,fb:*,n:>", "tcqj"),
        "rust" => {
            opts.shiftwidth = 4;
            opts.softtabstop = 4;
            opts.expandtab = true;
            opts.textwidth = 100;
            opts.cindent = true;
            opts.cinoptions = "L0,(s,Ws,J1,j1,m1".into();
            opts.cinkeys = "0{,0},!^F,o,O,0[,0],0(,0)".into();
            opts.cinwords =
                "for,if,else,while,loop,impl,mod,unsafe,trait,struct,enum,fn,extern,macro".into();
            opts.matchpairs = "(:),{:},[:],<:>".into();
            set(
                opts,
                "GetRustIndent(v:lnum)",
                Some("0{,0},!^F,o,O,0[,0],0(,0)"),
                "s0:/*!,ex:*/,s1:/*,mb:*,ex:*/,:///,://!,://",
                "croqnlj",
            );
        }
        "python" => {
            opts.tabstop = 4;
            opts.shiftwidth = 4;
            opts.softtabstop = 4;
            opts.expandtab = true;
            set(
                opts,
                "python#GetIndent(v:lnum)",
                Some("0{,0},0),0],:,!^F,o,O,e,<:>,=elif,=except"),
                "b:#,fb:-",
                "tcqj",
            );
            opts.cinkeys = "0{,0},0),0],:,!^F,o,O,e".into();
        }
        "lua" => set(
            opts,
            "GetLuaIndent()",
            Some("0{,0},0),0],:,0#,!^F,o,O,e,0=end,0=until"),
            ":---,:--",
            "jcroql",
        ),
        "toml" => set(opts, "", None, ":#", "tcqj"),
        "json" => set(
            opts,
            "GetJSONIndent(v:lnum)",
            Some("0{,0},0),0[,0],!^F,o,O,e"),
            "",
            "cqj",
        ),
        "markdown" => {
            opts.tabstop = 4;
            opts.shiftwidth = 4;
            opts.softtabstop = 4;
            opts.expandtab = true;
            opts.matchpairs = "(:),{:},[:],<:>".into();
            set(opts, "", None, "fb:*,fb:-,fb:+,n:>", "jtcqln");
        }
        "sh" => set(
            opts,
            "GetShIndent()",
            Some(
                "0{,0},0),0],!^F,o,O,e,0=then,0=do,0=else,0=elif,0=fi,0=esac,0=done,0=end,),\
                 0=;;,0=;&,0=fin,0=fil,0=fip,0=fir,0=fix",
            ),
            "b:#",
            "jcroql",
        ),
        "javascript" | "javascriptreact" => set(
            opts,
            "GetJavascriptIndent()",
            Some("0{,0},0),0],:,0#,!^F,o,O,e,0],0)"),
            js_comments,
            "jcroql",
        ),
        "typescript" | "typescriptreact" => set(
            opts,
            "GetTypescriptIndent()",
            Some("0{,0},0),0],0,,!^F,o,O,e"),
            js_comments,
            "jcroql",
        ),
        "c" | "cpp" => {
            opts.cindent = true;
            set(opts, "", None, c_comments, "jcroql");
        }
        _ => {}
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
        let plugin = self.filetype.plugin;
        let Some(buffer) = self.buffer_mut(id) else {
            return;
        };
        let ft = buffer.opts.filetype.clone();
        if plugin {
            ftplugin(&ft, &mut buffer.opts);
        }
        let lang = flux_syntax::lang_for_filetype(&ft);
        if buffer.syntax.as_ref().map(|s| s.lang()) != lang {
            buffer.syntax = lang.and_then(flux_syntax::Syntax::new);
        }
    }

    /// Bring the syntax trees of the buffers on screen up to date with their text.
    pub fn update_syntax(&mut self) {
        if !self.syntax_on {
            return;
        }
        let shown: Vec<BufferId> = std::iter::once(self.window.buffer)
            .chain(self.windows.iter().map(|w| w.buffer))
            .collect();
        for buffer in self.buffers.iter_mut().filter(|b| shown.contains(&b.id)) {
            if let Some(syntax) = buffer.syntax.as_mut() {
                syntax.update(&mut buffer.text);
            }
        }
    }
}
