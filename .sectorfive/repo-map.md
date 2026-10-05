# flux — repo map
Seeded 2026-10-03 by Sector 5 bootstrap.

## What it is
Terminal text editor with Vim's editing grammar, written in Rust. Behavior is checked against Neovim (recorded-expectation oracle). Milestone specs + manual checks: docs/milestones.md (M6 LSP latest per git log).

## Toolchain
- Rust 1.98.0 pinned (rust-toolchain.toml; components: rustfmt, clippy). Edition 2024, rust-version 1.98.
- Neovim v0.12.5 required for oracle/indent/colors checks (must match crates/flux-vim/tests/oracle/NVIM_VERSION; CI pins the same).
- tmux needed only for `cargo xtask screens`.

## Commands
- `cargo fmt --all --check` — format gate (CI)
- `cargo clippy --workspace --all-targets -- -D warnings` — lint gate (CI also sets RUSTFLAGS=-D warnings)
- `cargo test --workspace` — full suite (plain `cargo test` covers only the default binary)
- `cargo deny check` — licenses/advisories (CI; needs advisory DB)
- `cargo xtask oracle|indent|colors check` — recorded-vs-installed-Neovim verification (oracle.yml CI; see baseline.md for a local caveat on oracle)
- `cargo xtask <...> gen` — RE-RECORDS expectations (writes expected outputs; only via explicit plan)
- `cargo run -- <file>` — run the editor
- `cargo xtask screens` — side-by-side tmux comparison (needs tmux)

## Layout
| Path | Contents |
| --- | --- |
| crates/flux | `flux` binary: main.rs event loop, terminal setup, servers.rs (LSP spawning) |
| crates/flux-core | Text storage (rope), screen layout of lines. No IO by design. |
| crates/flux-view | Editor state: buffers, windows, options, filetypes, highlight groups, colors.json (embedded nvim colors) |
| crates/flux-vim | Modal engine: keys in, state changes out. Key notation, Ex commands. tests/: oracle, indent, completion |
| crates/flux-syntax | Tree-sitter parsing/highlighting with nvim queries; filetype detection. queries/ holds vendored .scm |
| crates/flux-tui | Cell-grid renderer + terminal writer. tests/: visual_draw |
| crates/flux-lsp | LSP client: JSON-RPC over stdio, server configs. src/bin/flux-lsp-fake.rs is a test fake |
| xtask | Project automation: oracle/indent/colors gen+check, screens. *.lua run under `nvim --headless` |
| docs/milestones.md | Milestone specs + manual checks |
| .github/workflows | ci.yml (fmt/clippy/test/deny), oracle.yml (nvim checks on oracle-related paths) |

## Entry points
- Binary: crates/flux/src/main.rs (`cargo run -- <file>`)
- Automation: xtask/src/main.rs via the `cargo xtask` alias (.cargo/config.toml)
- Key suites: `cargo test -p flux-vim --test oracle|indent`, `cargo test -p flux-tui --test visual_draw`

## Notes
- profile.test opt-level=1 (corpora replay speed).
- Workspace lints: unsafe_code forbid; clippy dbg_macro/print_* warn (denied in CI).
- some_file.js at root is unreferenced scratch (see tech-debt.md).
