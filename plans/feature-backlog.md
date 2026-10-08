# Flux 1.0 — feature backlog (dependency-ordered)

Source: `plans/flux-1.0-prd.md`. Sizes: S ≈ ≤1 day, M ≈ 2–4 days, L ≈ 1–2 weeks (agent-paced; rough). ★ = blocks 1.0. "Ready": ✅ = can be specced now; 📄 = spec written (`specs/F-NN-*.md`); 🚢 = merged (PR number); "after F-NN" = waits for that item.

| ID | Title | Why (one line) | Size | Depends on | ★ | Ready |
|---|---|---|---|---|---|---|
| F-01 | Picker respects `.gitignore` and caps fairly | `:Files` misses real files after a build (`target/` fills the 5000 cap; reproduced) | S–M | — | ★ | 🚢 #13 |
| F-02 | Tests independent of `TERM`/`NO_COLOR` | A `flux-tui` test fails under `NO_COLOR=1`; contributors' and CI environments vary | S | — | ★ | 🚢 #11 |
| F-03 | Oracle honesty: retag and report deferred cases | 12 `m: 9` cases are silently skipped; 6 already pass; "all 1163 match" is overstated | S | — | ★ | 🚢 #10 |
| F-04 | Oracle/indent/colors CI on every PR + macOS CI job | Parity is checked only when fixtures change; macOS is untested in CI (now free: public repo) | S | — | ★ | ✅ |
| F-05 | Name and packaging decision: **✅ decided Oct 5, 2026.** Command, config path and Lua namespace stay `flux`; package name `flux-editor` | crates.io/Homebrew `flux` names are taken; had to settle before the API namespace and config path freeze | S (decision) | — | ★ | ✅ done |
| F-06 | Stage B verify and sign-off | Pickers and Tab completion shipped without a recorded gate | S | F-01 | ★ | after F-01 |
| F-07 | Lua runtime + `init.lua` loading + error handling + `flux.opt` (existing options), `flux.cmd`, `flux.version`, `--clean`/`-u`; reserves the whole `flux.*` surface (spec Appendix A) | Foundation of user config; bad config must never crash | M | F-05 (done) | ★ | 📄 |
| F-08 | Common config options (`scrolloff`, `wrap`, `list`/`listchars`, `cursorline`, `colorcolumn`, `mouse`, `clipboard`, `swapfile`); `flux.opt` itself moved to F-07 | Users' first config lines currently hit E518 | M | F-07 | ★ | after F-07 |
| F-09 | `flux.map`/`flux.unmap` keymaps (per mode, non-recursive; `flux.g.mapleader`). **May slip to M8** by plan amendment (`.sectorfive/plans/m7.md:138–141`); names reserved in F-07 Appendix A either way | Keymaps are table stakes. Largest design risk in M7 | L | F-07 | ★ | after F-07 |
| F-10 | `flux.lsp.config`/`flux.lsp.enable` (replaces `$FLUX_LSP_CONFIG` for users; explicit enable feeds F-13's flag) | Users must add or adjust servers without env vars | S–M | F-07 | ★ | after F-07 |
| F-11 | `:colorscheme` + `:highlight` + `flux.highlight` + one alternate (light-friendly) scheme | Theming is expected; light terminals need a good option | M | F-07 | ★ | after F-07 |
| F-12 | M6/M7 docs closeout (milestones ✅ + manual checks, README crate table, remove `some_file.js`) | Docs must match shipped behavior | S | F-06, F-07–F-11 | ★ | |
| F-13 | Graceful LSP startup: an auto-started built-in that fails to start → one non-blocking line, no retry for the session; explicit servers keep Neovim's message | A broken server on PATH blocks first launch today (reproduced; Neovim shows the same prompt, but only for servers the user enabled) | S | — | ★ | 🚢 #12 |
| F-14 | Swap files + crash recovery (`-r`, recover prompt) | No-data-loss promise for a public editor | L | F-08 (`'swapfile'`) | ★ | |
| F-15 | Visual-block mode (`CTRL-V` with `I`/`A`/`c`/`d`/`$`) | Daily Vim feature; 4 failing oracle cases exist | M | F-03 | ★ | |
| F-16 | `'clipboard'` `unnamed`/`unnamedplus` | The most common vimrc line; `+`/`*` sync already exists | S | F-08 | ★ | |
| F-17 | `CTRL-A` / `CTRL-X` | Frequently used; 1 failing oracle case | S | F-03 | ★ | |
| F-18 | Sentence motions/objects `(` `)` `is` `as` | Prose/comment editing; 1 failing oracle case | S | F-03 | ★ | |
| F-19 | `R` Replace mode | Expected and cheap | S | — | ★ | |
| F-20 | Basic mouse (click, focus, wheel, drag-select) + `'mouse'` | Neovim has mouse on by default; new users click | M | F-08 | ★ | |
| F-21 | `:!cmd`, `:{range}!`, `!{motion}`, `:r [file\|!cmd]` | Shell and filter workflows are core Vim usage | M | — | ★ | |
| F-22 | `:help` with bundled docs (`--clean` moved to F-07) | New users need answers inside the editor | M | F-28, F-29, F-30 | ★ | |
| F-23 | Fix oracle re-recording on macOS: replace `set all&` (`xtask/oracle.lua:77`) with snapshot/restore; loud failures | Can't record new parity cases on the owner's Mac (tech-debt HIGH). Prototype matches all 1163 cases on Linux | S | — | ★ | 📄 (Mac run needed) |
| F-24 | Performance budgets in CI (open, first frame, search, `:s`/`:g`) | Protects the speed promise (S3) | M | F-04 | ★ | |
| F-25 | Release pipeline: tagged builds for macOS arm64/x86_64, Linux x86_64/arm64, checksums, smoke install | One-command install (S1) | M | F-05, F-04 | ★ | |
| F-26 | Homebrew tap formula `flux-editor` + `cargo install flux-editor` + crate metadata (publishable names for `flux-core`/`flux-tui`, which are taken on crates.io) | Install channels strangers expect | S–M | F-05, F-25 | ★ | |
| F-27 | Versioning policy (`docs/versioning.md`) + `CHANGELOG.md` + PR template + version/changelog consistency tests | Users need to know what changed | S | — | ★ | 📄 |
| F-28 | README for strangers | First impression and install path | S | F-25, F-26 | ★ | |
| F-29 | Lua/config API reference | 100% coverage of `flux.*` (S6) | M | F-07–F-11 | ★ | |
| F-30 | Supported Vim features matrix (linked to oracle evidence) | Lets users judge fit honestly | M | F-03, M8 items | ★ | |
| F-31 | CONTRIBUTING guide | Outside contributors after a public launch | S | F-04, F-23 | ★ | |
| F-32 | 0.9 dry-run release on clean runners | Prove S1/S4 before 1.0 | S | F-25, F-26, F-13 | ★ | |
| F-33 | 1.0 RC with outside testers → 1.0.0 | Real-user validation | M | all ★ | ★ | |
| F-34 | Quickfix unit tests get their own temp folders (CI flake) | `quickfix::tests::going_through_the_list` races `the_quickfix_window` over one `flux-qf-<pid>` folder; failed CI on main at `97e6ba5` and `1a4ad7d` | XS | — | ★ | 📄 |
| P-01 | Opt-in LSP auto-completion while typing | Requested in milestones.md M7; not Vim default | M | F-07 | — | post-1.0 |
| P-02 | Tab pages; `\|` between commands; persistent undo; `q:` | Common but not day-one | M each | — | — | post-1.0 |
| P-03 | Autocmds, user commands, plugin loading | Ecosystem; out of 1.0 scope | L | F-07 | — | post-1.0 |
| P-04 | M6 leftovers: focusable floats, `:clist`/`:colder`, inlay hints, code lenses | Nice-to-haves | M | — | — | post-1.0 |
| P-05 | Windows support | No evidence of demand; `cfg(unix)` paths | L | — | — | post-1.0 |

**Specs written:** F-01, F-02, F-03 (Oct 5, 2026); F-07, F-13, F-23, F-27 (Oct 6, 2026); F-34 (Oct 7, 2026). F-05 is decided (package `flux-editor`, command `flux`).

**Next to spec:** F-04 (ready now; it was marked ready but left out of this note before, and it matters most for reviewing outside PRs), then F-08, F-10, F-11 and F-09 (F-07's API decisions, including the Appendix A shapes, were settled Oct 6, 2026).

**Stage C split** (per `.sectorfive/plans/m7.md:138–141`): F-07 (runtime, `init.lua`, `flux.opt`/`cmd`/`version`, flags) → F-08 (new options) ∥ F-10 (`flux.lsp`) ∥ F-11 (colors) ∥ F-09 (maps). Each is a separately mergeable PR. F-09 is the one the plan allows to slip to M8; if it slips, record the amendment in `.sectorfive/decisions.md`.
