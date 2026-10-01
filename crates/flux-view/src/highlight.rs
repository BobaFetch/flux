//! Highlight groups: Neovim's default colorscheme (`colors.json`, made by `cargo xtask colors
//! gen`), looked up the way Neovim does.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

/// A color as a highlight group gives it: 24-bit for 'termguicolors', otherwise a terminal
/// color number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HlColor {
    Rgb(u8, u8, u8),
    Index(u8),
}

/// What a highlight group sets. Unset colors leave the color underneath (Neovim's
/// `hl_combine_attr`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HlAttr {
    pub fg: Option<HlColor>,
    pub bg: Option<HlColor>,
    /// The underline color (24-bit; Neovim sends it with or without 'termguicolors').
    pub sp: Option<HlColor>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub undercurl: bool,
    pub strikethrough: bool,
    pub reverse: bool,
}

/// One group in both color modes, or a link to another group.
#[derive(Debug, Clone, Default)]
struct GroupDef {
    link: Option<String>,
    gui: HlAttr,
    cterm: HlAttr,
}

type Groups = HashMap<String, GroupDef>;

fn rgb(v: &Value) -> Option<HlColor> {
    let n = v.as_u64()?;
    Some(HlColor::Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

fn attrs(base: HlAttr, list: Option<&Value>) -> HlAttr {
    let mut a = base;
    for name in list.and_then(Value::as_array).into_iter().flatten() {
        match name.as_str() {
            Some("bold") => a.bold = true,
            Some("italic") => a.italic = true,
            Some("underline") => a.underline = true,
            Some("undercurl") => a.undercurl = true,
            Some("strikethrough") => a.strikethrough = true,
            Some("reverse") => a.reverse = true,
            _ => {}
        }
    }
    a
}

fn parse_groups(v: &Value) -> Groups {
    let mut groups = HashMap::new();
    for (name, d) in v.as_object().into_iter().flatten() {
        let index = |k: &str| {
            d.get(k)
                .and_then(Value::as_u64)
                .map(|n| HlColor::Index(n as u8))
        };
        let sp = d.get("sp").and_then(rgb);
        let gui = HlAttr {
            fg: d.get("fg").and_then(rgb),
            bg: d.get("bg").and_then(rgb),
            sp,
            ..HlAttr::default()
        };
        let cterm = HlAttr {
            fg: index("ctermfg"),
            bg: index("ctermbg"),
            sp,
            ..HlAttr::default()
        };
        groups.insert(
            name.clone(),
            GroupDef {
                link: d.get("link").and_then(Value::as_str).map(str::to_owned),
                gui: attrs(gui, d.get("gui")),
                cterm: attrs(cterm, d.get("cterm")),
            },
        );
    }
    groups
}

/// The default colorscheme for 'background' dark and light.
fn defaults() -> &'static (Groups, Groups) {
    static DEFAULTS: OnceLock<(Groups, Groups)> = OnceLock::new();
    DEFAULTS.get_or_init(|| {
        let v: Value = serde_json::from_str(include_str!("colors.json")).expect("colors.json");
        (parse_groups(&v["dark"]), parse_groups(&v["light"]))
    })
}

/// Look up group `name` for 'background' `light` or dark, in 24-bit colors (`gui`) or terminal
/// colors. A tree-sitter capture group (`@keyword.function`) falls back to shorter names
/// (`@keyword`), and links are followed. An unknown group sets nothing.
pub fn resolve(name: &str, light: bool, gui: bool) -> HlAttr {
    let (dark_groups, light_groups) = defaults();
    let groups = if light { light_groups } else { dark_groups };
    let mut name = name.to_string();
    for _ in 0..20 {
        let found = groups.get(&name).or_else(|| {
            let mut n = name.as_str();
            while n.starts_with('@') {
                n = &n[..n.rfind('.')?];
                if let Some(d) = groups.get(n) {
                    return Some(d);
                }
            }
            None
        });
        let Some(def) = found else {
            return HlAttr::default();
        };
        match &def.link {
            Some(link) => name = link.clone(),
            None => return if gui { def.gui } else { def.cterm },
        }
    }
    HlAttr::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_resolve_like_neovim() {
        let normal = resolve("Normal", false, true);
        assert_eq!(normal.fg, Some(HlColor::Rgb(0xe0, 0xe2, 0xea)));
        assert_eq!(normal.bg, Some(HlColor::Rgb(0x14, 0x16, 0x1b)));
        assert_eq!(resolve("Normal", false, false), HlAttr::default());
        // @keyword → Keyword → Statement: bold.
        assert!(resolve("@keyword", false, true).bold);
        // @function.call isn't defined: @function → Function.
        assert_eq!(
            resolve("@function.call", false, true),
            resolve("Function", false, true)
        );
        assert_eq!(resolve("String", false, false).fg, Some(HlColor::Index(10)));
        assert!(resolve("@markup.strong", true, true).bold);
        assert_eq!(resolve("NoSuchGroup", false, true), HlAttr::default());
        assert_eq!(resolve("@no.such.capture", false, true), HlAttr::default());
    }
}
