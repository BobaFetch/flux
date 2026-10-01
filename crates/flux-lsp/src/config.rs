//! Which server to run for which filetypes, and the project root it works in. The built-in
//! configs follow nvim-lspconfig's (`lsp/*.lua`) for the commands, filetypes and root markers.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq)]
pub struct ServerConfig {
    /// nvim-lspconfig's name for it (`rust_analyzer`).
    pub name: String,
    pub cmd: Vec<String>,
    pub filetypes: Vec<String>,
    /// Files or directories that mark a project root; the nearest directory holding one wins.
    /// Markers in the same inner list have the same priority.
    pub root_markers: Vec<Vec<String>>,
    /// `workspace/configuration` answers and `initializationOptions`.
    pub settings: Value,
    pub init_options: Value,
}

fn config(name: &str, cmd: &[&str], filetypes: &[&str], root_markers: &[&[&str]]) -> ServerConfig {
    ServerConfig {
        name: name.into(),
        cmd: cmd.iter().map(|s| s.to_string()).collect(),
        filetypes: filetypes.iter().map(|s| s.to_string()).collect(),
        root_markers: root_markers
            .iter()
            .map(|group| group.iter().map(|s| s.to_string()).collect())
            .collect(),
        settings: json!({}),
        init_options: Value::Null,
    }
}

/// The servers flux knows how to run.
pub fn builtin_configs() -> Vec<ServerConfig> {
    vec![
        config(
            "rust_analyzer",
            &["rust-analyzer"],
            &["rust"],
            &[&["Cargo.toml", "rust-project.json"], &[".git"]],
        ),
        config(
            "clangd",
            &["clangd"],
            &["c", "cpp", "objc", "objcpp", "cuda"],
            &[
                &[
                    ".clangd",
                    ".clang-tidy",
                    ".clang-format",
                    "compile_commands.json",
                    "compile_flags.txt",
                    "configure.ac",
                ],
                &[".git"],
            ],
        ),
        config(
            "lua_ls",
            &["lua-language-server"],
            &["lua"],
            &[
                &[
                    ".emmyrc.json",
                    ".luarc.json",
                    ".luarc.jsonc",
                    ".luacheckrc",
                    ".stylua.toml",
                    "stylua.toml",
                    "selene.toml",
                    "selene.yml",
                ],
                &[".git"],
            ],
        ),
        config(
            "basedpyright",
            &["basedpyright-langserver", "--stdio"],
            &["python"],
            &[
                &[
                    "pyproject.toml",
                    "setup.py",
                    "setup.cfg",
                    "requirements.txt",
                    "Pipfile",
                    "pyrightconfig.json",
                ],
                &[".git"],
            ],
        ),
        config(
            "pyright",
            &["pyright-langserver", "--stdio"],
            &["python"],
            &[
                &[
                    "pyproject.toml",
                    "setup.py",
                    "setup.cfg",
                    "requirements.txt",
                    "Pipfile",
                    "pyrightconfig.json",
                ],
                &[".git"],
            ],
        ),
        config(
            "ts_ls",
            &["typescript-language-server", "--stdio"],
            &[
                "javascript",
                "javascriptreact",
                "typescript",
                "typescriptreact",
            ],
            &[
                &[
                    "package-lock.json",
                    "yarn.lock",
                    "pnpm-lock.yaml",
                    "bun.lockb",
                    "bun.lock",
                ],
                &[".git"],
            ],
        ),
        config(
            "bashls",
            &["bash-language-server", "start"],
            &["bash", "sh"],
            &[&[".git"]],
        ),
        config(
            "taplo",
            &["taplo", "lsp", "stdio"],
            &["toml"],
            &[&[".taplo.toml", "taplo.toml"], &[".git"]],
        ),
        config(
            "jsonls",
            &["vscode-json-language-server", "--stdio"],
            &["json", "jsonc"],
            &[&[".git"]],
        ),
        config(
            "marksman",
            &["marksman", "server"],
            &["markdown"],
            &[&[".marksman.toml"], &[".git"]],
        ),
    ]
}

/// Configs from a JSON file: a list of `{ "name", "cmd", "filetypes", "root_markers",
/// "settings", "init_options" }` (`root_markers` entries may be strings or lists of them).
pub fn from_json_file(path: &Path) -> Result<Vec<ServerConfig>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let list: Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let strings = |v: &Value| -> Vec<String> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s.as_str().map(str::to_owned))
            .collect()
    };
    Ok(list
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| ServerConfig {
            name: c["name"].as_str().unwrap_or("server").to_string(),
            cmd: strings(&c["cmd"]),
            filetypes: strings(&c["filetypes"]),
            root_markers: c["root_markers"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|m| match m {
                    Value::String(s) => vec![s.clone()],
                    other => strings(other),
                })
                .collect(),
            settings: c.get("settings").cloned().unwrap_or_else(|| json!({})),
            init_options: c.get("init_options").cloned().unwrap_or(Value::Null),
        })
        .collect())
}

/// Whether `program` can be run (it's on `$PATH`, or a path to a file).
pub fn executable(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

/// The project root for `file` (Neovim's `root_markers`): the nearest directory, going up,
/// that holds one of the first group of markers; failing that, the second group; and so on.
/// For rust-analyzer, Cargo's workspace root, as nvim-lspconfig asks `cargo metadata`.
pub fn find_root(config: &ServerConfig, file: &Path) -> Option<PathBuf> {
    let start = file.parent()?;
    for group in &config.root_markers {
        let found = start
            .ancestors()
            .find(|dir| group.iter().any(|m| dir.join(m).exists()));
        if let Some(dir) = found {
            if config.name == "rust_analyzer" && dir.join("Cargo.toml").is_file() {
                return Some(cargo_workspace_root(&dir.join("Cargo.toml")).unwrap_or(dir.into()));
            }
            return Some(dir.to_path_buf());
        }
    }
    None
}

fn cargo_workspace_root(manifest: &Path) -> Option<PathBuf> {
    let out = Command::new("cargo")
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(manifest)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let meta: Value = serde_json::from_slice(&out.stdout).ok()?;
    meta["workspace_root"].as_str().map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_from_markers() {
        let dir = std::env::temp_dir().join(format!("flux-lsp-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("proj/src/deep")).unwrap();
        std::fs::create_dir_all(dir.join("proj/.git")).unwrap();
        std::fs::write(dir.join("proj/src/.clangd"), "").unwrap();
        let clangd = builtin_configs()
            .into_iter()
            .find(|c| c.name == "clangd")
            .unwrap();
        let file = dir.join("proj/src/deep/a.c");
        assert_eq!(find_root(&clangd, &file), Some(dir.join("proj/src")));
        let bash = builtin_configs()
            .into_iter()
            .find(|c| c.name == "bashls")
            .unwrap();
        assert_eq!(
            find_root(&bash, &dir.join("proj/src/x.sh")),
            Some(dir.join("proj"))
        );
        assert_eq!(find_root(&bash, &dir.join("x.sh")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
