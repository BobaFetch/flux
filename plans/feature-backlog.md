# Flux 1.0 — feature backlog (dependency-ordered)

Source: `plans/flux-1.0-prd.md`. Sizes: S ≈ ≤1 day, M ≈ 2–4 days, L ≈ 1–2 weeks (agent-paced; rough). ★ = blocks 1.0. "Ready" = can be specced now.

| ID | Title | Why (one line) | Size | Depends on | ★ | Ready |
|---|---|---|---|---|---|---|
| F-01 | Picker respects `.gitignore` and caps fairly | `:Files` misses real files after a build (`target/` fills the 5000 cap; reproduced) | S–M | — | ★ | ✅ |
| F-02 | Tests independent of `TERM`/`NO_COLOR` | A `flux-tui` test fails under `NO_COLOR=1`; contributors' and CI environments vary | S | — | ★ | ✅ |
| F-03 | Oracle honesty: retag and report deferred cases | 12 `m: 9` cases are silently skipped; 6 already pass; "all 1163 match" is overstated | S | — | ★ | ✅ |
| F-04 | Oracle/indent/colors CI on every PR + macOS CI job | Parity is checked only when fixtures change; macOS is untested in CI (now free: public repo) | S | — | ★ | ✅ |
| F-05 | Name and packaging decision: **✅ decided Oct 5, 2026.** Command, config path and Lua namespace stay `flux`; package name `flux-editor` | crates.io/Homebrew `flux` names are taken; had to settle before the API namespace and config path freeze | S (decision) | — | ★ | ✅ done |
| F-06 | Stage B verify and sign-off | Pickers and Tab completion shipped without a recorded gate | S | F-01 | ★ | after F-01 |
| F-07 | Lua runtime + `init.lua` loading + error handling | Foundation of user config; bad config must never crash | M | F-05 (done) | ★ | ✅ |
| F-08 | `flux.opt` + common config options (`scrolloff`, `wrap`, `list`/`listchars`, `cursorline`, `colorcolumn`, `mouse`, `clipboard`, `swapfile`) | Users' first config lines currently hit E518 | M | F-07 | ★ | |
| F-09 | `flux.map` keymaps (per mode, non-recursive) | Keymaps are table stakes. Largest design risk in M7 | L | F-07 | ★ | |
| F-10 | `flux.lsp` server configs (replaces `$FLUX_LSP_CONFIG` for users) | Users must add or adjust servers without env vars | S–M | F-07 | ★ | |
| F-11 | `:colorscheme` + `:highlight` + one alternate (light-friendly) scheme | Theming is expected; light terminals need a good option | M | F-07 | ★ | |
| F-12 | M6/M7 docs closeout (milestones ✅ + manual checks, README crate table, remove `some_file.js`) | Docs must match shipped behavior | S | F-06, F-07–F-11 | ★ | |
| F-13 | Graceful LSP startup (no blocking prompt; report via `:lsp`/statusline) | A broken server on PATH blocks first launch today | S | — | ★ | ✅ |
| F-14 | Swap files + crash recovery (`-r`, recover prompt) | No-data-loss promise for a public editor | L | F-08 (`'swapfile'`) | ★ | |
| F-15 | Visual-block mode (`CTRL-V` with `I`/`A`/`c`/`d`/`$`) | Daily Vim feature; 4 failing oracle cases exist | M | F-03 | ★ | |
| F-16 | `'clipboard'` `unnamed`/`unnamedplus` | The most common vimrc line; `+`/`*` sync already exists | S | F-08 | ★ | |
| F-17 | `CTRL-A` / `CTRL-X` | Frequently used; 1 failing oracle case | S | F-03 | ★ | |
| F-18 | Sentence motions/objects `(` `)` `is` `as` | Prose/comment editing; 1 failing oracle case | S | F-03 | ★ | |
| F-19 | `R` Replace mode | Expected and cheap | S | — | ★ | |
| F-20 | Basic mouse (click, focus, wheel, drag-select) + `'mouse'` | Neovim has mouse on by default; new users click | M | F-08 | ★ | |
| F-21 | `:!cmd`, `:{range}!`, `!{motion}`, `:r [file\|!cmd]` | Shell and filter workflows are core Vim usage | M | — | ★ | |
| F-22 | `:help` with bundled docs; `--clean` flag | New users need answers inside the editor | M | F-28, F-29, F-30 | ★ | |
| F-23 | Fix oracle re-recording on macOS (`xtask/oracle.lua:77`) | Can't record new parity cases on the owner's Mac (tech-debt HIGH) | S–M | — | ★ | ✅ |
| F-24 | Performance budgets in CI (open, first frame, search, `:s`/`:g`) | Protects the speed promise (S3) | M | F-04 | ★ | |
| F-25 | Release pipeline: tagged builds for macOS arm64/x86_64, Linux x86_64/arm64, checksums, smoke install | One-command install (S1) | M | F-05, F-04 | ★ | |
| F-26 | Homebrew tap formula `flux-editor` + `cargo install flux-editor` + crate metadata (publishable names for `flux-core`/`flux-tui`, which are taken on crates.io) | Install channels strangers expect | S–M | F-05, F-25 | ★ | |
| F-27 | Versioning + `CHANGELOG.md` (semver from 1.0.0) | Users need to know what changed | S | — | ★ | ✅ |
| F-28 | README for strangers | First impression and install path | S | F-25, F-26 | ★ | |
| F-29 | Lua/config API reference | 100% coverage of `flux.*` (S6) | M | F-07–F-11 | ★ | |
| F-30 | Supported Vim features matrix (linked to oracle evidence) | Lets users judge fit honestly | M | F-03, M8 items | ★ | |
| F-31 | CONTRIBUTING guide | Outside contributors after a public launch | S | F-04, F-23 | ★ | |
| F-32 | 0.9 dry-run release on clean runners | Prove S1/S4 before 1.0 | S | F-25, F-26, F-13 | ★ | |
| F-33 | 1.0 RC with outside testers → 1.0.0 | Real-user validation | M | all ★ | ★ | |
| P-01 | Opt-in LSP auto-completion while typing | Requested in milestones.md M7; not Vim default | M | F-07 | — | post-1.0 |
| P-02 | Tab pages; `\|` between commands; persistent undo; `q:` | Common but not day-one | M each | — | — | post-1.0 |
| P-03 | Autocmds, user commands, plugin loading | Ecosystem; out of 1.0 scope | L | F-07 | — | post-1.0 |
| P-04 | M6 leftovers: focusable floats, `:clist`/`:colder`, inlay hints, code lenses | Nice-to-haves | M | — | — | post-1.0 |
| P-05 | Windows support | No evidence of demand; `cfg(unix)` paths | L | — | — | post-1.0 |

**Spec first:** F-01 (picker), F-02 (test env independence), F-03 (oracle honesty). F-05 is decided (package `flux-editor`, command `flux`), and F-13/F-23/F-27 are also ready.
