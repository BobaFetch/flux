# flux — ownership map
Status: approved 2026-10-03 by user.
Rule: every path has exactly one owner. Shared drawing files serialize via Y'shtola (Yuna builds, Tifa polishes, never parallel). The drawing role was named Aerith until 2026-10-07.

| Path | Owner | Notes |
| --- | --- | --- |
| crates/flux-core/** | Lightning | Rope storage, layout. No IO by design. |
| crates/flux-vim/** (except tests/) | Lightning | Modal engine, keys→state, Ex commands. |
| crates/flux-syntax/** | Lightning | Tree-sitter, queries, filetype detection. |
| crates/flux-lsp/** | Lightning | JSON-RPC stdio client, server configs, fake. |
| crates/flux-view/** | Lightning | Editor state. Tifa may polish highlight-group/copy surface in a later phase via plan. |
| crates/flux-view/src/colors.json | Lightning | Machine-recorded from nvim (gen); Tifa proposes, never hand-edits. |
| crates/flux/src/servers.rs | Lightning | LSP server process management. |
| crates/flux/src/main.rs, terminal.rs | Yuna | Event loop, terminal setup, user-facing runtime behavior. |
| crates/flux-tui/** (except tests/) | Yuna | Cell-grid rendering, terminal output. Tifa polish phase for visuals/copy. |
| crates/*/tests/** (cases.json, expected outputs, indent corpus, visual_draw) | Fina | Test files + recorded expectations. Harness scripts (xtask/*.lua) stay Lightning; expected outputs change only via explicit gen plan. |
| xtask/** | Lightning | Automation + nvim harnesses. |
| .github/**, deny.toml, rust-toolchain.toml, rustfmt.toml, Cargo.toml/lock, .cargo/** | Lightning | Toolchain, CI, policy. |
| docs/**, README.md, LICENSE-* | Y'shtola | Planning docs + front page (agents contribute via Y'shtola). |
| some_file.js | — | Unowned scratch; see tech-debt.md (remove). |
| .sectorfive/** | Y'shtola | Team meta (Fina records baseline.md results). |
| .sectorfive/contract.md, .sectorfive/roles/**, .sectorfive/bin/** | Y'shtola | Dev-only operating contract, role briefs with pinned models, `role` launcher. Model changes follow the contract's swap rule. |
| .grok/** | Y'shtola | Dev-only Grok CLI agent files that point at `.sectorfive/roles/`. |
| AGENTS.md `## Sector Five (build roles)` section | Y'shtola | The rest of AGENTS.md is unchanged by this row. |
| plans/** | — (PM) | Specs and product planning by the PM agent outside the team; read-only to Sector Five. |

Cross-cutting: oracle/indent/colors `gen` (rewrites expectations) is always a Y'shtola-planned, Fina-verified task — never incidental.
