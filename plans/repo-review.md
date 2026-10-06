# flux — PM repo review

Reviewed Oct 5, 2026 (PT) by Aerith (PM agent). Read-only: a local clone at HEAD `27f9a2a`, GitHub public API, user-Github connector, user-Linear connector. Nothing was pushed, commented on or edited.
Repo: https://github.com/BobaFetch/flux (public, 2 stars, default branch `main`, created Sep 28, 2026 8:53 PM PT).

## 1. What Flux is
- **Product:** "A terminal text editor with Vim's editing grammar, written in Rust" (repo description, `README.md`). Its goal is to "behave exactly like Vim (checked against Neovim) for the editing it supports, with its own Lua API rather than Neovim plugin compatibility".
- **Who it's for:** not stated. The evidence points to the owner as primary user: the M4 manual check is "the dogfooding point: try editing flux's own source with flux" (`docs/milestones.md`), the M6 check uses `~/Projects/flux`, and the M7 goal is "flux works as a daily driver" (`.sectorfive/plans/m7.md`). Vim/Neovim users wanting a fast, hackable Rust editor are implied.
- **Problem it solves:** implied, not stated. Vim-exact editing in a self-contained Rust binary, with built-in tree-sitter highlighting, LSP and its own Lua config, without Neovim's plugin ecosystem.
- **Stack:**
  - Rust 1.98.0 pinned (`rust-toolchain.toml`), edition 2024, `unsafe_code = "forbid"` (`Cargo.toml`)
  - crossterm TUI, ropey text storage, tokio, regex, tree-sitter grammars (`crates/flux-syntax/Cargo.toml`)
  - LSP over stdio (`crates/flux-lsp`)
  - Dual MIT/Apache license; permissive-only dependency policy (`deny.toml`)
- **Crates:**
  - `flux-core`: rope, layout, no IO (~2.2k lines)
  - `flux-view`: editor state (~11.4k)
  - `flux-vim`: modal engine, Ex commands (~25.5k)
  - `flux-syntax` (~1.3k), `flux-tui` (~2.4k), `flux-lsp` (~0.65k)
  - `flux` binary (~0.66k), `xtask` (~0.8k)
- **Platform:** terminal on Linux and macOS. CI runs on Linux only (`.github/workflows/ci.yml`); macOS is "covered by running the tests locally" (commit `dac59b9`). Windows isn't mentioned; there are `cfg(unix)` paths in `flux-lsp/src/transport.rs` and `flux-view/src/buffer.rs`. No releases or tags; install is `cargo install --path crates/flux`.
- **Verification approach (core quality bar):**
  - Neovim 0.12.5 oracle: 1163 key-sequence cases (`crates/flux-vim/tests/oracle/cases.json`, `expected.json`, `NVIM_VERSION`)
  - an indent corpus
  - an embedded Neovim colorscheme
  - `cargo xtask screens`, a cell-by-cell tmux comparison with Neovim, including a fake LSP server
- **How it's built:** with an agent team called "Sector 5" (`.sectorfive/`). Roles are Y'shtola (planning/docs), Lightning (engine), **Aerith** (binary/TUI), Tifa (polish) and Fina (verification gates). Ownership is approved per path (`.sectorfive/ownership.md`). Commits are co-authored by Claude.

## 2. What "complete" means per the repo
- **There's no repo-wide definition of v1/1.0 or "complete".** `README.md` says "It is early"; the version is `0.0.1`.
- **Milestones** (`docs/milestones.md`): M0–M5 plus "Directory browsing" are marked ✅. **M6: LSP has no ✅**. **M7: Picker, Lua config, clipboard** has two bullets:
  - "Add TOML configuration for static editor preferences and the planned Lua API…"
  - "Make LSP completion optionally automatic while typing…"
- **Closest thing to a definition of done** is the active M7 plan (`.sectorfive/plans/m7.md`, status active, dated Oct 3):
  - Goal: "flux works as a daily driver"
  - Stage A: `+`/`*` registers on the system clipboard
  - Stage B: `:Files`/`:Buffers` fuzzy pickers and command-line Tab completion
  - Stage C: Lua config (`init.lua`, `flux.opt`/`flux.map`/`flux.lsp`, `:colorscheme`/`:highlight`, mlua 0.12 `lua54`+`vendored`)
  - Success criteria: Fina approves each stage, baseline gates green, `cargo deny` green after C, and a **checked M7 section with a manual check in `docs/milestones.md`**
- **Named as beyond M7** (plan non-goals, "M8+ candidates"): visual-block mode, `q:`, autocmds, user commands, plugins, mouse, swap files, inlay hints and code lenses.

## 3. Current state (verified, not trusted)

**Activity:** 14 commits from Sep 28, 7:17 PM PT (M0) to Oct 4, 11:13 PM PT (M7 A/B, pushed directly to `main` at 11:17 PM PT with no PR).
- 9 PRs (#1–#9), all merged, at https://github.com/BobaFetch/flux/pulls?q=is%3Apr
- 0 issues. CI is green on every `main` push, including `27f9a2a` (https://github.com/BobaFetch/flux/actions).
- Unmerged leftover branches: `directory-browser`, `docs/roadmap-and-agent-guide`, `m5-syntax`, `milestones-checked`.

**Build and tests on the PM's Linux test machine** (Linux, Rust 1.98.0 installed with rustup):
- `cargo fmt --all --check`: pass.
- `cargo clippy --workspace --all-targets -- -D warnings`: pass.
- `cargo test --workspace`: **184 of 185 pass**. The one failure, `flux-tui renderer::tests::default_colors_fill_in_and_force_a_redraw`, is caused by that machine's environment (`NO_COLOR=1`, `TERM=dumb`). It passes with `NO_COLOR` unset and `TERM=xterm-256color` (16/16). The baseline on Oct 3 recorded 162 tests (`.sectorfive/baseline.md`).
- Not run: `cargo deny check` (not installed here; CI runs it and it passed on the latest push), and the xtask oracle/indent/colors/screens checks (they need Neovim 0.12.5).
- Smoke test in tmux:
  - The editor opens and renders.
  - Command-line Tab completion works (`:e crates/flux/src/ma<Tab>` became `main.rs`).
  - `:Files` opens the picker.
  - With rust-analyzer not installed for the toolchain, flux shows "Client rust_analyzer quit with exit code 1… Check log" and a hit-enter prompt at startup.

| Area | Status | Evidence |
|---|---|---|
| M0–M4, directory browsing, M5 | **Done** | ✅ in `docs/milestones.md`; oracle replay passes (`crates/flux-vim/tests/oracle.rs`, `MILESTONE = 6`); indent corpus passes |
| M6 LSP | **Done in code, not marked done** | PR #7 + #8 merged; heading has no ✅. Known gaps listed: floats can't be focused; no `:clist`/`:colder`; no inlay hints. Diagnostic detail is weak (added in #9) |
| M7 Stage A: clipboard | **Done (code + approval)** | `crates/flux/src/clipboard.rs` (OSC 52 write; `pbpaste`/`wl-paste`/`xclip`/`xsel` read; `FLUX_CLIPBOARD_FAKE`), `flux-view/src/registers.rs`. `.sectorfive/decisions.md`: "Stage A approved by Fina" |
| M7 Stage B: pickers + Tab completion | **Partial** | Code in `flux-view/src/{picker,matcher}.rs`, `flux-vim/src/complete.rs`, `flux-tui/src/draw/picker.rs`, `:Files`/`:Buffers` (`flux-vim/src/ex.rs:159`). **No Fina approval recorded** in `decisions.md`. **Bug (reproduced):** the file walk (`flux-view/src/explorer.rs` `walk_files`, cap 5000 at `ex.rs:1446`) only skips dot-files. It ignores `.gitignore`, so after a build `target/` (6,741 files here) fills the cap and `xtask/oracle.lua` can't be found with `:Files oracle.lua` |
| M7 Stage C: Lua config, `:colorscheme`, `:highlight` | **Missing** | No `mlua` in any `Cargo.toml`, no `init.lua` loading in `crates/flux/src/main.rs` |
| M7 docs | **Missing** | `docs/milestones.md` M7 section has no description of A/B, no manual check, no ✅ |
| Auto LSP completion (milestones M7 bullet) | **Missing / out of plan** | Not in `.sectorfive/plans/m7.md` scope |

**Docs vs code mismatches**
- **"All 1163 oracle cases match Neovim 0.12.5"** (milestones M6) is not literally true. 12 cases are tagged `m: 9` and silently skipped (`oracle.rs` gate at `MILESTONE = 6`). With the gate raised locally to 9 (then reverted), **6 fail**: `ctrl-a`, `vblock-insert`, `vblock-delete`, `vblock-append-eol`, `ctrl-v-dollar-c`, `paren-sentence`. The other 6 (`:g`/`:v`/`:norm` cases) pass but stay mis-tagged.
- **M7 scope conflict:** `docs/milestones.md` says TOML config + Lua API + automatic completion. `.sectorfive/plans/m7.md` says Lua only (mlua), no TOML, and no automatic completion.
- **README:** the layout table omits `flux-lsp`, and README says "its own Lua API" though no Lua exists yet. Listed as LOW in `.sectorfive/tech-debt.md`.
- **CI comment vs milestones:** milestones M0 says "CI (fmt, clippy, tests on Linux and macOS…)", but CI is now Linux-only (`ci.yml`, commit `dac59b9`).
- **Leftover file:** `some_file.js` at the root is unreferenced scratch (tech-debt LOW).
- **Broken locally:** `cargo xtask oracle gen|check` fails on the owner's Mac with Homebrew nvim (`xtask/oracle.lua:77` `set all&`). Tech-debt HIGH; CI's Linux Neovim works.

## 4. Gap to done (ordered by dependency; ★ = blocker for a usable v1, assuming v1 = "M7 done, daily driver")
1. ★ **S:** Owner defines v1 (scope, audience, date) and resolves the M7 conflict: Lua-only vs TOML+Lua, and auto-completion in or out.
2. ★ **S–M:** Fix the `:Files` walk: respect `.gitignore` (or at least skip `target/`/`node_modules/`) and make the cap not starve later directories. Add a test.
3. ★ **S:** Stage B verification gate (Fina): matcher/picker tests, manual picker + Tab checks. Record the approval in `.sectorfive/decisions.md`.
4. ★ **L:** Stage C, Lua config:
   - mlua dependency + `cargo deny`
   - `init.lua` load order and error handling
   - `flux.opt`, `flux.lsp`, `flux.map`
   - `:colorscheme` (default + one alternate) and `:highlight`
   - fixture tests

   The plan flags `flux.map` as the biggest risk; it may split to M8.
5. ★ **S:** Docs closeout: M7 section with manual check + ✅; ✅ on M6 after its manual check; README (crates table, current features, Lua status); remove `some_file.js`.
6. **S:** Retag the 6 passing `m: 9` oracle cases to their real milestone, and correct the "all 1163" claim.
7. **S–M:** Fix `cargo xtask oracle gen|check` on macOS (replace `set all&`). This blocks recording new oracle cases on the owner's machine.
8. **S:** Make tests independent of `NO_COLOR`/`TERM`, or document the requirement.
9. **M:** Common Vim gaps, if the owner needs them for daily use:
   - visual-block `CTRL-V` (4 failing oracle cases): M
   - `CTRL-A`/`CTRL-X`: S
   - sentence objects `is`/`as`: S
   - `R` Replace mode: S
   - `'clipboard'` unnamedplus: S
   - `|` between commands: S–M
   - mouse: M
   - swap files: M
10. **M:** Optional automatic LSP completion while typing (milestones M7 bullet).
11. **S–M:** Packaging for others: tags/releases, prebuilt binaries or a Homebrew formula, install docs. Only a blocker if v1 is public.
12. **S–M:** Handle a missing language server gracefully at startup (currently a hit-enter prompt when rust-analyzer isn't installed for the toolchain).

## 5. Risks and unknowns
- **No stated v1.** "Exactly like Vim" is an open-ended target; without a cut line, "complete soon" can't be measured.
- **Stage C is the biggest piece left:** a new native dependency (vendored Lua C build), a new public API surface (`flux.*`), and keymap semantics.
- **Velocity vs. process:** about 2,000-line milestone commits; the M7 A/B commit skipped the PR flow used for #1–#9. The Neovim-oracle CI workflow only runs when oracle paths change, so M7 didn't trigger it.
- **Local oracle tooling broken on the owner's Mac** (tech-debt HIGH): new behavior can't be recorded against Neovim locally.
- **Platform:** no macOS CI. Linux clipboard reads need `wl-paste`/`xclip`/`xsel`. OSC 52 support varies by terminal. Windows unknown.
- **Name collision:** the repo's agent team already has an "**Aerith**" role (binary/TUI owner, `.sectorfive/ownership.md`), the same name as this PM agent. That could cause confusion when dispatching or reading decisions.
- **Unverified here:** `cargo deny`, the Neovim xtask checks, macOS behavior, LSP features with real servers.

## 6. Open questions for the owner (max 5)
1. What does "complete" mean: M7 done so flux is your daily driver, or a public 1.0 others install? By when?
2. Config: Lua-only via mlua (the M7 plan) or TOML + Lua (milestones.md)? Must `flux.map` keymaps be in v1, or can they go to M8?
3. Which Vim gaps are must-haves for you: visual-block, `CTRL-A/X`, sentence objects, `R`, mouse, swap files, `unnamedplus`, automatic completion?
4. Platforms and distribution: macOS + Linux only? Prebuilt binaries/Homebrew, or `cargo install` only?
5. How should I fit with the Sector 5 team? Should I own `docs/` and planning instead of Y'shtola, or feed Y'shtola? Should all work go through PRs? And what about the existing "Aerith" role name?

## 7. Linear
- **Workspace:** one team, **Yonaka** (commit email domain `yonaka.dev`).
- **Projects:** only "yonaka-bookshelf Phase 1" (Completed) and "yonaka-bookshelf Phase 2" (In Progress, https://linear.app/yonaka/project/yonaka-bookshelf-phase-2-2a789247a2b6). **No project or issue matches "flux"** (searched projects including archived, and issues).
- **Repo:** no references to Linear (searched for `linear`, `linear.app`, issue keys).
