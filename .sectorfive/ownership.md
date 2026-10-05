# flux — ownership map
Status: approved 2026-10-03 by user.
Rule: every path has exactly one owner. Shared client files serialize via Y'shtola (Aerith builds, Tifa polishes, never parallel).

| Path | Owner | Notes |
| --- | --- | --- |
| crates/flux-core/** | Lightning | Rope storage, layout. No IO by design. |
| crates/flux-vim/** (except tests/) | Lightning | Modal engine, keys→state, Ex commands. |
| crates/flux-syntax/** | Lightning | Tree-sitter, queries, filetype detection. |
| crates/flux-lsp/** | Lightning | JSON-RPC stdio client, server configs, fake. |
| crates/flux-view/** | Lightning | Editor state. Tifa may polish highlight-group/copy surface in a later phase via plan. |
| crates/flux-view/src/colors.json | Lightning | Machine-recorded from nvim (gen); Tifa proposes, never hand-edits. |
| crates/flux/src/servers.rs | Lightning | LSP server process management. |
| crates/flux/src/main.rs, terminal.rs | Aerith | Event loop, terminal setup, user-facing runtime behavior. |
| crates/flux-tui/** (except tests/) | Aerith | Cell-grid rendering, terminal output. Tifa polish phase for visuals/copy. |
| crates/*/tests/** (cases.json, expected outputs, indent corpus, visual_draw) | Fina | Test files + recorded expectations. Harness scripts (xtask/*.lua) stay Lightning; expected outputs change only via explicit gen plan. |
| xtask/** | Lightning | Automation + nvim harnesses. |
| .github/**, deny.toml, rust-toolchain.toml, rustfmt.toml, Cargo.toml/lock, .cargo/** | Lightning | Toolchain, CI, policy. |
| docs/**, README.md, LICENSE-* | Y'shtola | Planning docs + front page (agents contribute via Y'shtola). |
| some_file.js | — | Unowned scratch; see tech-debt.md (remove). |
| .sectorfive/** | Y'shtola | Team meta (Fina records baseline.md results). |

Cross-cutting: oracle/indent/colors `gen` (rewrites expectations) is always a Y'shtola-planned, Fina-verified task — never incidental.
