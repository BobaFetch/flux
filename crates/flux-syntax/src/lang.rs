//! The languages flux can parse: their grammars and queries, loaded on first use.

use std::sync::OnceLock;

use tree_sitter::{Language, Query};

use crate::predicate::Patterns;

macro_rules! queries {
    ($kind:literal: $($lang:literal),*) => {
        &[$(include_str!(concat!("../queries/", $lang, "/", $kind, ".scm"))),*]
    };
}

/// A language as compiled in: its grammar and the query files that make up its queries, in
/// order (Neovim's `; inherits:` already resolved).
struct Def {
    name: &'static str,
    grammar: fn() -> Language,
    highlights: &'static [&'static str],
    injections: &'static [&'static str],
}

const DEFS: &[Def] = &[
    Def {
        name: "bash",
        grammar: || tree_sitter_bash::LANGUAGE.into(),
        highlights: queries!("highlights": "bash"),
        injections: queries!("injections": "bash"),
    },
    Def {
        name: "c",
        grammar: || tree_sitter_c::LANGUAGE.into(),
        highlights: queries!("highlights": "c"),
        injections: queries!("injections": "c"),
    },
    Def {
        name: "javascript",
        grammar: || tree_sitter_javascript::LANGUAGE.into(),
        highlights: queries!("highlights": "ecma", "jsx", "javascript"),
        injections: queries!("injections": "ecma", "jsx", "javascript"),
    },
    Def {
        name: "json",
        grammar: || tree_sitter_json::LANGUAGE.into(),
        highlights: queries!("highlights": "json"),
        injections: queries!("injections": "json"),
    },
    Def {
        name: "lua",
        grammar: || tree_sitter_lua::LANGUAGE.into(),
        highlights: queries!("highlights": "lua"),
        injections: queries!("injections": "lua"),
    },
    Def {
        name: "markdown",
        grammar: || tree_sitter_md::LANGUAGE.into(),
        highlights: queries!("highlights": "markdown"),
        injections: queries!("injections": "markdown"),
    },
    Def {
        name: "markdown_inline",
        grammar: || tree_sitter_md::INLINE_LANGUAGE.into(),
        highlights: queries!("highlights": "markdown_inline"),
        injections: queries!("injections": "markdown_inline"),
    },
    Def {
        name: "python",
        grammar: || tree_sitter_python::LANGUAGE.into(),
        highlights: queries!("highlights": "python"),
        injections: queries!("injections": "python"),
    },
    Def {
        name: "rust",
        grammar: || tree_sitter_rust::LANGUAGE.into(),
        highlights: queries!("highlights": "rust"),
        injections: queries!("injections": "rust"),
    },
    Def {
        name: "toml",
        grammar: || tree_sitter_toml_ng::LANGUAGE.into(),
        highlights: queries!("highlights": "toml"),
        injections: queries!("injections": "toml"),
    },
    Def {
        name: "tsx",
        grammar: || tree_sitter_typescript::LANGUAGE_TSX.into(),
        highlights: queries!("highlights": "ecma", "typescript", "jsx", "tsx"),
        injections: queries!("injections": "ecma", "typescript", "jsx", "tsx"),
    },
    Def {
        name: "typescript",
        grammar: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        highlights: queries!("highlights": "ecma", "typescript"),
        injections: queries!("injections": "ecma", "typescript"),
    },
];

/// The names of the languages flux can parse.
pub fn names() -> impl Iterator<Item = &'static str> {
    DEFS.iter().map(|d| d.name)
}

/// A query with what flux needs to evaluate it: the predicates and settings of each pattern.
pub(crate) struct LangQuery {
    pub query: Query,
    pub patterns: Patterns,
}

/// A loaded language.
pub(crate) struct Lang {
    pub name: &'static str,
    pub language: Language,
    pub highlights: LangQuery,
    pub injections: Option<LangQuery>,
}

/// Neovim extends `#set!` with capture arguments that tree-sitter's own parser rejects, so the
/// queries go through as a general predicate flux evaluates itself.
fn query_source(parts: &[&str]) -> String {
    parts.concat().replace("(#set! ", "(#nvim-set! ")
}

fn compile(language: &Language, parts: &[&str]) -> Result<LangQuery, String> {
    let query = Query::new(language, &query_source(parts)).map_err(|e| e.to_string())?;
    let patterns = Patterns::new(&query)?;
    Ok(LangQuery { query, patterns })
}

fn load(def: &Def) -> Result<Lang, String> {
    let language = (def.grammar)();
    let highlights =
        compile(&language, def.highlights).map_err(|e| format!("{} highlights: {e}", def.name))?;
    let injections = if def.injections.iter().all(|s| s.trim().is_empty()) {
        None
    } else {
        Some(
            compile(&language, def.injections)
                .map_err(|e| format!("{} injections: {e}", def.name))?,
        )
    };
    Ok(Lang {
        name: def.name,
        language,
        highlights,
        injections,
    })
}

/// The language called `name` (a parser name such as `rust` or `markdown_inline`), loaded on
/// first use. `None` if flux doesn't have it, or its queries don't load (which tests rule out).
pub(crate) fn get(name: &str) -> Option<&'static Lang> {
    static LOADED: OnceLock<Vec<OnceLock<Option<Lang>>>> = OnceLock::new();
    let loaded = LOADED.get_or_init(|| DEFS.iter().map(|_| OnceLock::new()).collect());
    let i = DEFS.iter().position(|d| d.name == name)?;
    loaded[i].get_or_init(|| load(&DEFS[i]).ok()).as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_query_loads() {
        for def in DEFS {
            if let Err(e) = load(def) {
                panic!("{e}");
            }
        }
    }
}
