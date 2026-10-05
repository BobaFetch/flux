# flux — verification baseline
Recorded 2026-10-03. Run with `CARGO_TARGET_DIR=/tmp/flux-baseline-target CARGO_NET_OFFLINE=true` (sandbox cannot write the repo's target/; same rustc 1.98.0, same flags otherwise). Tree clean before and after.

- `cargo fmt --all --check` → PASS
- `cargo clippy --workspace --all-targets -- -D warnings` → PASS (exit 0)
- `cargo test --workspace` → PASS, 162 passed, 0 failed (core 29, lsp 2, syntax 8, tui 14, visual_draw 1, view 52, vim 52, completion 2, indent 1, oracle 1)
- `cargo xtask indent check` → PASS ("indent expectations match NVIM v0.12.5")
- `cargo xtask colors check` → PASS ("colors.json matches NVIM v0.12.5")
- `cargo xtask oracle check` → BLOCKED (environment): nvim dies silently in `-l` scripts at `vim.cmd("set all&")` (xtask/oracle.lua:77). Reproduced unsandboxed; `:set number` works and `+cmd` mode survives — specific to `set all&` under `-l` with this build (Homebrew 0.12.5, LuaJIT 2.1.1788856981). Recorded-expectation replay (`--test oracle`) PASSES, so flux-vs-recorded is green; only live re-verification against installed nvim is blocked. See tech-debt.md.
- `cargo deny check` → NOT RUN (sandbox blocks the ~/.cargo/advisory-dbs lock; rerun outside sandbox).

rustc 1.98.0 (88d9e12ae 2026-08-18). nvim NVIM v0.12.5. cargo-deny 0.20.2 present.
