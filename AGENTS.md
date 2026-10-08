# Repository Guide

## Commands

- The workspace is pinned to Rust 1.98.0. `cargo run -- path/to/file` runs the editor because `crates/flux` is the only default member.
- Do not use plain `cargo test` as the full check; it covers only the default binary. Run `cargo test --workspace`.
- Tests must not depend on `TERM`, `NO_COLOR` or `COLORTERM`; CI also runs `cargo test --workspace` with `NO_COLOR=1 TERM=dumb`.
- Reproduce CI in this order: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo deny check`.
- Focus a crate or unit test with `cargo test -p <crate>` or `cargo test -p <crate> <test-name>`.
- The Neovim behavior suite is `cargo test -p flux-vim --test oracle`. It replays all cases through flux against committed results; it does not invoke Neovim.
- The indent corpus is `cargo test -p flux-vim --test indent`; set `FLUX_INDENT=<filename-substring>` to narrow it.

## Architecture

- Keep `flux-core` free of IO: it owns rope text, positions, editing history, patterns, and screen-line layout.
- `flux-view` owns editor state (buffers, windows, options, highlights, and protocol-level LSP state); `flux-vim` turns keys and Ex commands into state changes and performs file IO only for Ex operations.
- `flux-syntax` owns filetype detection plus incremental tree-sitter parsing/highlighting. Its queries under `crates/flux-syntax/queries/` are compiled in with `include_str!`.
- `flux-tui` converts editor state into a cell grid and incrementally renders it. Terminal setup, event-loop orchestration, and the executable entrypoint belong in `crates/flux`.
- Keep LSP responsibilities split: process/config/JSON-RPC transport in `flux-lsp`, editor protocol state in `flux-view::lsp`, message semantics and commands in `flux-vim::lsp`, and process-to-editor wiring in `crates/flux/src/servers.rs`.

## Neovim Fixtures

- Vim-compatible behavior is checked against Neovim 0.12.5. The pinned version is recorded in `crates/flux-vim/tests/oracle/NVIM_VERSION` and CI; `NVIM_BIN` can select another executable for local xtasks.
- Treat `crates/flux-vim/tests/oracle/expected.json`, `crates/flux-vim/tests/indent/expected/`, and `crates/flux-view/src/colors.json` as generated snapshots. Regenerate them with `cargo xtask oracle gen`, `cargo xtask indent gen`, or `cargo xtask colors gen`, then inspect the diff.
- Validate generated snapshots against an installed Neovim with the corresponding `cargo xtask <oracle|indent|colors> check` command.
- Oracle cases above the `MILESTONE` constant in `crates/flux-vim/tests/oracle.rs` are deferred: they still run and must differ from Neovim (`deferred_cases_still_differ`), so a case that starts matching fails the suite until its `m` tag is lowered. Update `MILESTONE` deliberately when landing a milestone.
- `cargo xtask screens [filter]` compares flux and Neovim cell-by-cell in both truecolor and 16-color modes. It requires `tmux`, Neovim, and a C compiler, and builds reusable parser artifacts under `target/`.

## Constraints

- Workspace lints forbid unsafe code and CI promotes every warning to an error. `dbg!`, stdout printing, and stderr printing are workspace warnings unless narrowly allowed.
- New dependencies must use explicit versions and pass the permissive-only license/source policy in `deny.toml`; copyleft licenses, wildcard registry dependencies, unknown registries, and unknown git sources are rejected.

## Sector Five (build roles)

Dev-only: these roles, `.sectorfive/` and `.grok/` are used to build Flux and are never part of the editor or a published package.

- **Lightning**: engine and core (`flux-core`, `flux-vim`, `flux-view`, `flux-syntax`, `flux-lsp`, `servers.rs`, xtask, CI).
- **Yuna**: drawing and terminal runtime (`crates/flux/src/main.rs`, `terminal.rs`, `flux-tui`).
- **Tifa**: polish and copy, after a flow works; never changes behavior.
- **Fina**: tests and verification; can veto; always runs on Muse Spark 1.3 (GPT-6.1 Sol if Muse built the change).
- **Y'shtola**: plans, sequencing, `.sectorfive/`, docs.

Every path has one owner: `.sectorfive/ownership.md`. Rules and templates: `.sectorfive/contract.md`. Briefs and pinned models: `.sectorfive/roles/<name>.md`.

Startup ritual: read your brief, the contract, `ownership.md`, the active plan in `.sectorfive/plans/` (and its spec in `plans/specs/`), and `baseline.md`. Stay inside your paths; anything else is a request to Y'shtola. Default order: Lightning → Yuna → Tifa → Fina.

Start a role with `.sectorfive/bin/role <name>` (pi, pinned model; add `--model provider/id:thinking` to override) or `grok --agent <name>`. Other agents: read the brief and contract first.

Build roles open pull requests but never merge them. The PM (Aerith, outside the team) may merge once Fina approves and CI is green; otherwise the user merges.
