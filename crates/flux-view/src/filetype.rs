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

/// The options Neovim's ftplugins set (`nvim --clean` with the filetype's recommended style).
fn ftplugin(filetype: &str, opts: &mut BufferOptions) {
    match filetype {
        "rust" => {
            opts.shiftwidth = 4;
            opts.softtabstop = 4;
            opts.expandtab = true;
        }
        "python" | "markdown" => {
            opts.tabstop = 4;
            opts.shiftwidth = 4;
            opts.softtabstop = 4;
            opts.expandtab = true;
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
