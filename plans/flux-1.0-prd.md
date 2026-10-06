# Product Requirement Document (PRD): Flux 1.0

Status: Draft v1, Oct 5, 2026. Owner: you. PM: Aerith.
Grounded in https://github.com/BobaFetch/flux at `27f9a2a` (read-only) and `plans/repo-review.md`.
Decided: "complete" means a **public 1.0 that other people can install**. No hard date; ship when solid.
Decided (Oct 5, 2026): the binary/command, config path (`~/.config/flux/`) and Lua namespace (`flux.*`) stay **`flux`**. The package name is **`flux-editor`** (verified free on crates.io and Homebrew core on Oct 5, 2026).

---

## 1. Executive Summary & Goals

**Core objective.** Ship Flux 1.0: a terminal editor that does Vim's editing *exactly* (verified against Neovim). It comes with LSP, tree-sitter, a picker and a small Lua config, and a stranger can install it in one command and use it as their daily editor without losing work.

**Target audience (inferred; no audience is stated in the repo).** Vim and Neovim users who:
- want exact Vim behavior and muscle memory, with no surprises
- are tired of assembling and maintaining a plugin stack just to get LSP, highlighting and a file finder
- prefer a small, documented Lua config over Neovim's large API

They live in macOS or Linux terminals. Secondary audience: Rust developers who might contribute.

**Value proposition**
| vs | Flux's pitch |
|---|---|
| Neovim | Same editing behavior, proven by an oracle test suite. LSP, tree-sitter and a picker work out of the box with no plugins. A smaller, stable Lua API |
| Helix | Keeps Vim's verb-object grammar and muscle memory (Helix uses selection-first editing). Batteries-included like Helix |
| Vim | Modern defaults, built-in LSP and tree-sitter, Lua config, a single binary |

**1.0 success criteria (measurable)**
| # | Criterion | Target |
|---|---|---|
| S1 | One-command install | Homebrew tap formula `flux-editor` (macOS arm64/x86_64, Linux), prebuilt binaries for macOS arm64/x86_64 and Linux x86_64/arm64, `cargo install flux-editor`. Every channel installs the `flux` command. Each is verified on a clean runner in CI per release |
| S2 | Vim parity honesty | 100% of 1.0-scope oracle cases pass. **Zero silently skipped cases**: anything deferred is listed and reported by the test run |
| S3 | Responsiveness | First frame ≤ 100 ms for a typical source file. 200,000-line file opens ≤ 0.5 s. Tested commands respond ≤ 30 ms (milestones.md records 0.3 s and < 30 ms today) |
| S4 | Clean first run | On a machine with no config and no language servers, flux never shows a blocking prompt at startup. A broken `init.lua` shows a message and flux starts with stock settings |
| S5 | No lost work | After the process is killed mid-edit, recovery restores unsaved changes in 100% of recovery tests. Saves stay atomic (already true) |
| S6 | Documented | Every 1.0 feature appears in the supported-features matrix. The Lua API reference covers 100% of the public `flux.*` surface |

---

## 2. Core Features (1.0 must-haves)

### 2.1 Finish M7 (daily-driver baseline)
- **Picker fix.** `:Files` respects `.gitignore` (plus common build directories when no git). A file cap must never hide whole directories. Today `walk_files` (`crates/flux-view/src/explorer.rs`, cap 5000 at `crates/flux-vim/src/ex.rs:1446`) only skips dot-files; after a build, `target/` starves `xtask/` (reproduced).
- **Stage B sign-off.** Pickers (`:Files`, `:Buffers`) and command-line Tab completion pass the team's verify gate, recorded in `.sectorfive/decisions.md`.
- **Stage C: Lua config** (mlua 0.12, `lua54` + `vendored`, per `.sectorfive/plans/m7.md`):
  - `init.lua` loaded from the XDG config path. A missing file means stock behavior. Errors show a message and never crash.
  - `flux.opt` covers every implemented option. `flux.map` gives per-mode, non-recursive keymaps. `flux.lsp` replaces `$FLUX_LSP_CONFIG`, which stays as a test override.
  - `:colorscheme` (built-in `default` plus at least one alternate, including a light-friendly scheme) and `:highlight` (inspect and override).
  - The API is documented as **stable for 1.x**. Anything not ready is marked experimental.
- **Config format:** Lua only for 1.0 (recommended; matches the approved M7 plan). TOML is dropped from 1.0 scope (§7).

### 2.2 Day-one Vim gaps (ranked)
| Rank | Gap | 1.0? | Why |
|---|---|---|---|
| 1 | **Swap/crash recovery** (`'swapfile'`, recover prompt, `-r`) | **1.0** | Data safety is non-negotiable for a public editor. Today there's no swap file (milestones.md M1 gaps) |
| 2 | **Visual-block mode** (`CTRL-V`, `I`/`A`/`c`/`d`/`$`) | **1.0** | Used daily by Vim users. 4 oracle cases already exist and fail when enabled |
| 3 | **System clipboard default** (`'clipboard'` = `unnamed`/`unnamedplus`) | **1.0** | The most common line in every vimrc. `+`/`*` sync exists; only the option is missing (`.sectorfive/tech-debt.md`) |
| 4 | **`CTRL-A`/`CTRL-X`** | **1.0** | Small, frequently used. 1 failing oracle case exists |
| 5 | **Sentence motions/objects** (`(` `)` `is` `as`) | **1.0** | Small. Prose and comment editing. 1 failing oracle case |
| 6 | **`R` Replace mode** | **1.0** | Small, expected, cheap |
| 7 | **Mouse, basic** (click to place cursor and focus window, wheel scroll, drag to select) | **1.0** | Neovim enables mouse by default; strangers will click. Advanced mouse (resize by drag, menus) later |
| 8 | **LSP auto-completion while typing** | **Post-1.0** (opt-in) | Not Vim/Neovim default behavior. Explicit `CTRL-X CTRL-O` already works. Keeps 1.0 focused |

Also 1.0, because new users hit them right away (verified missing in `crates/flux-vim/src/ex.rs` and the options list):
- **`:!cmd`, `:{range}!filter`, `!{motion}`, `:r [file|!cmd]`**: running and filtering through shell commands.
- **Options people put in configs:** `'scrolloff'`, `'wrap'`, `'list'`/`'listchars'`, `'cursorline'`, `'colorcolumn'`, `'mouse'`, `'clipboard'`, `'swapfile'`. Unknown options currently give E518.

### 2.3 First-run experience
- **FR-1** Missing, broken or failing language servers never block startup. Report through the statusline or `:lsp` and the log, not a hit-enter prompt. Today a non-working `rust-analyzer` on PATH (for example a rustup proxy without the component) produces a blocking hit-enter prompt (observed on the box).
- **FR-2** Sane defaults with no config: Neovim's defaults (already the reference), truecolor detection (exists), a readable scheme on light terminals (`:set bg=light` today; at least documented).
- **FR-3** `:help [topic]` opens bundled docs (feature matrix, Lua API, keys) inside flux. `flux --help` points to the same docs.
- **FR-4** `flux --version` and `--help` exist (`crates/flux/src/main.rs`); add `--clean` (ignore config) and `-r` (recover).

### 2.4 Distribution
- **FR-D1** GitHub Releases with prebuilt binaries: macOS arm64 and x86_64, Linux x86_64 and arm64, with checksums. Built and smoke-tested in CI from a tag.
- **FR-D2** Package name `flux-editor` everywhere a package is named; the installed command is always `flux`.
  - Homebrew: formula `flux-editor` in the owner's tap (`brew install <tap>/flux-editor`), installing the `flux` binary. If another installed formula also provides a `flux` binary, the formula declares it in `conflicts_with`. Homebrew core's `flux` is InfluxData's; whether its binary is named `flux` is unverified.
  - crates.io: `cargo install flux-editor`. The binary crate is published as `flux-editor` with a `[[bin]]` named `flux`.
  - Every workspace crate the binary depends on must also be publishable. `flux-core` and `flux-tui` are taken on crates.io, so they need free names (for example with a `flux-editor-` prefix) or must be folded into the published crate. `flux-view`, `flux-vim`, `flux-lsp` and `flux-syntax` were free when checked earlier. How to resolve this is an implementation choice (F-26).
  - Until the first publish, `cargo install --git https://github.com/BobaFetch/flux flux-editor` is the documented fallback.
- **FR-D3** Semantic versioning starting at 1.0.0. `CHANGELOG.md` with every user-visible change. Today the version is `0.0.1`, with no tags and no changelog.
- **FR-D4** Crate metadata ready for publishing (package name `flux-editor`, repository, readme, keywords). Today `crates/flux/Cargo.toml` has `name = "flux"` and only `description = "The flux editor"`.

### 2.5 Quality gates
- **FR-Q1** macOS CI alongside Linux. The repo is now public, so standard macOS runners are free (the Linux-only choice in `ci.yml` was made for private-repo billing).
- **FR-Q2** The Neovim oracle, indent and colors checks run on **every PR**. Today `oracle.yml` runs only when fixture paths change.
- **FR-Q3** Tests pass regardless of `TERM`/`NO_COLOR`. Today `flux-tui renderer::tests::default_colors_fill_in_and_force_a_redraw` fails under `NO_COLOR=1`.
- **FR-Q4** Re-recording expectations works on macOS (`xtask/oracle.lua:77` `set all&`; tech-debt HIGH).
- **FR-Q5** No silent skips: the test run prints deferred cases, and each has a reason (S2).
- **FR-Q6** Release-blocking checks: fmt, clippy `-D warnings`, `cargo test --workspace` on both OSes, `cargo deny`, oracle/indent/colors, install smoke tests.

### 2.6 Documentation
- **README for strangers:** what it is, install, first 5 minutes, configuring, how it differs from Neovim, links.
- **Lua/config reference:** every `flux.*` function and option, with examples.
- **Supported Vim features matrix:** supported, partial, or not planned, linked to oracle evidence.
- **CONTRIBUTING:** build, tests, oracle workflow, dependency policy, PR process.

---

## 3. User Stories
1. **Install.** As a Neovim user on an M-series Mac, I run `brew install <tap>/flux-editor`, type `flux .`, and I'm editing in under a minute.
2. **Linux, no Homebrew.** As a Linux user, I download a release binary, or run `cargo install flux-editor`, and it runs on a clean machine with no prompts.
3. **Configure.** I create `~/.config/flux/init.lua` with `flux.opt` (numbers, tabs, `clipboard = "unnamedplus"`), a few `flux.map` keymaps and `:colorscheme`. A typo shows a clear error, and flux still opens.
4. **Muscle memory.** I use `CTRL-V` to add `;` to ten lines, `CTRL-A` to bump a version, `das` to drop a sentence, `R` to overwrite. Everything does exactly what Vim does.
5. **Find and jump.** `:Files` finds any source file in a large Rust repo instantly, never shows `target/`, and `:Buffers` switches files.
6. **LSP with no setup.** I open a TypeScript file. If the server is installed, I get diagnostics and hover. If not, flux just works without it and `:lsp` tells me why.
7. **Crash.** My terminal dies mid-edit. Reopening the file offers recovery and I get my changes back.
8. **Shell.** `:!cargo test`, and `!ip sort` over a block, work like Vim's.
9. **Evaluate fit.** Before switching, I read the feature matrix and see exactly what's supported and what isn't.
10. **Contribute.** I follow CONTRIBUTING to add an oracle case and a fix, and CI checks it against Neovim on my PR.

---

## 4. Phased release plan
| Milestone | Scope | Exit criteria |
|---|---|---|
| **Decide** ✅ | Name/packaging decision before the Lua API and config path are frozen | Done Oct 5, 2026: command, config path and Lua namespace `flux`; package `flux-editor` |
| **M7 finish** | Picker fix. Test environment independence. Oracle honesty (retag, report deferred). Stage B sign-off. Stage C Lua config + `:colorscheme`/`:highlight` | Fina approvals for B and C. `cargo deny` green with mlua. M6 and M7 checked in `docs/milestones.md` with manual checks. Owner uses it daily for a week |
| **M8: 1.0 gaps** | Swap/recovery. Visual-block. `'clipboard'`. `CTRL-A`/`X`. Sentence objects. `R`. Basic mouse. `:!`/filters/`:r`. Config options. Graceful LSP startup. `:help` | All 12 currently deferred oracle cases tagged in-scope and passing, plus new cases for each gap. Recovery test suite passes (S5). Clean-machine run has no prompts (S4) |
| **M9: release hardening** | macOS CI. Oracle on every PR. Mac re-recording fixed. Release pipeline + Homebrew + `cargo install`. Changelog/versioning. README, Lua reference, feature matrix, CONTRIBUTING. Performance budgets in CI | S1, S3, S6 met. A dry-run release (`0.9.0`) installs on clean macOS and Linux runners |
| **1.0 RC → 1.0** | Public release candidate. A few outside Vim users try it | No open data-loss or crash bugs. No P1 parity bugs open for 2 weeks. Then tag `1.0.0` |

**Post-1.0 candidates:** opt-in LSP auto-completion, tab pages, `|` between commands, persistent undo, `q:`, autocmds and user commands, plugin loading, inlay hints and code lenses, `:clist`/`:colder`, focusable floats, Windows.

---

## 5. Non-functional requirements
- **Performance:** S3 budgets measured in CI on a fixed corpus (open, first frame, search, `:s`, `:g` on 200,000 lines). Typing never blocks on parsing (the 20 ms parse slice exists) or on LSP.
- **Reliability and data safety:** atomic saves that keep permissions, symlinks and CRLF (exists). Refuse to overwrite files changed on disk (exists). Swap files and recovery (new). A panic restores the terminal (exists in `crates/flux/src/terminal.rs`) and keeps recovery data.
- **Platform:** macOS 13+ (arm64, x86_64) and Linux glibc (x86_64, arm64) at 1.0; static/musl Linux is open. Terminals tested: Terminal.app, iTerm2, Ghostty, kitty, WezTerm, Alacritty, tmux. Clipboard via OSC 52 plus `pbpaste`/`wl-paste`/`xclip`/`xsel` (exists).
- **Licensing:** permissive-only dependencies enforced by `cargo deny` (`deny.toml`). Vendored queries keep their NOTICE. Releases include license files and third-party notices.
- **Terminals, colors and accessibility:** works in 16-color and truecolor (both compared against Neovim today). Ships a light-background-friendly scheme. Defined, documented behavior for `NO_COLOR` and `TERM=dumb`. No information conveyed by color alone (diagnostic signs keep `E`/`W` letters).
- **Security:** no network access. `init.lua` runs with the user's permissions (documented). Language servers are started only from known configs or the user's config.

## 6. Constraints & Out of Scope
- **Given constraints:** Rust (pinned toolchain), crossterm, ropey, tree-sitter with Neovim queries, Lua 5.4 via mlua, `unsafe` forbidden, Neovim 0.12.5 as the behavioral oracle.
- **Out of scope for 1.0:**
  - Windows (no CI, `cfg(unix)` paths, no evidence of use)
  - Neovim plugin or API compatibility (README: flux has its own Lua API)
  - Vimscript; plugin manager or ecosystem
  - GUI front-ends; remote/SSH editing beyond running in a terminal
  - Debugger (DAP); built-in terminal emulator; LSP auto-completion (post-1.0)

## 7. Doc/code corrections to make
- `docs/milestones.md` says "All 1163 oracle cases match", but 12 are tagged `m: 9` and skipped by `MILESTONE = 6` (`crates/flux-vim/tests/oracle.rs`). Of those, 6 fail (4 visual-block, `CTRL-A`, `paren-sentence`) and 6 pass (`:g`/`:v`/`:norm`) and should be retagged.
- The M7 section in `docs/milestones.md` (TOML + Lua + auto-completion) contradicts `.sectorfive/plans/m7.md` (Lua only, no auto-completion). Align it to this PRD.
- M6 is merged but has no ✅. M7 A/B shipped with no milestones entry or manual check.
- Milestones M0 says CI covers Linux and macOS; `ci.yml` is Linux-only.
- README: the layout table omits `flux-lsp`; it promises a Lua API that doesn't exist yet; it has no install-for-users path.
- `some_file.js` at the repo root is scratch; remove it.
- `crates/flux/Cargo.toml` lacks publish metadata; the version is `0.0.1`; there are no tags or changelog.
- Leftover merged branches: `directory-browser`, `docs/roadmap-and-agent-guide`, `m5-syntax`, `milestones-checked`.

## 8. Open questions (owner)
1. **Targets:** Linux arm64 and a static/musl build at 1.0? Minimum macOS version?
2. **Defaults:** mouse on by default like Neovim (`mouse=nvi`) or off? Confirm LSP auto-completion is post-1.0.
3. **Process:** must every change go through a PR (M7 A/B went straight to `main`)? Who gives final 1.0 sign-off: you, or Fina's gate? Should the existing Sector 5 "Aerith" role be renamed to avoid confusion with your PM agent?
4. **API promise:** is `flux.*` frozen under semver for all of 1.x, with experimental items clearly marked?
