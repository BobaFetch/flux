# F-06 Stage B evidence — pickers + command-line Tab completion

- Verdict: **Stage B approved by Fina** on re-verify.
- Model: `meta/muse-spark-1.3` high.
- Heads: first pass 2026-10-15 VETO on `1726eb7`; re-verify 2026-10-16 APPROVE on `main @ d0340e9` (PR #20 squash-merge).
- Spec: `plans/specs/F-06-stage-b-sign-off.md` (R7 record). No product code, tests, or recorded expectations change in this PR.
- Raw captures: `target/fina-f06-stage-b/` (79 files, gitignored) starting at `00-verdict.md`. This file summarizes; it does not copy raw captures.
- Open-question defaults applied: Tifa B3 skipped (copy issues become follow-ups); unspecified picker behavior recorded as-is (non-blocking); user hands-on taken as input, not as verification.

## R1 gates (A1) — all PASS

On both `1726eb7` (first pass) and `d0340e9` (re-verify): `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (223 passed first pass, 224 on re-verify, 0 failed), `env NO_COLOR=1 TERM=dumb cargo test --workspace` (19 suites ok), `cargo deny check`, `cargo xtask indent check` and `cargo xtask colors check` (match Neovim v0.12.5), oracle recorded replay PASS. Live `oracle check` remains blocked per baseline (pre-existing, not Stage B). CI green with all four checks on both heads (lint, test ubuntu/macos, neovim).

## R2 coverage audit (A2) — map + gaps

| Stage B scope item | Covering tests |
|---|---|
| Fuzzy matcher | `flux-view/src/matcher.rs`: subsequence/case, ranking, rank/filter (3 tests) |
| Picker state (filter, select, wrap, Tab/S-Tab, Up/Down) | `flux-view/src/picker.rs`: filter_select_wrap, editing_refilters, truncation flag, non-UTF8 path, empty-match (5 tests) |
| Picker keys + `:Files`/`:Buffers` open/prefill/Esc/empty-confirm | `flux-vim/src/engine.rs`: pickers_open_files_and_buffers, files_picker_with_nothing_eligible_fails_confirm |
| F-01 gitignore spot area | engine `files_picker_skips_gitignored_files`, `files_picker_omits_an_open_gitignored_buffer` + explorer fair-cap tests (~10) |
| Completion target detection | 3 tests (commands-after-ranges, register args, attached bang) |
| Candidate lists | `option_forms_and_value_positions`, `register_and_lsp_lists` |
| Cmdline Tab cycling + wrap + typing-accepts + search-literal-Tab + no-match-fails | engine `cmdline_tab_cycles_command_names`, `cmdline_tab_completes_command_after_attached_range`, `cmdline_tab_completes_each_argument_kind` |
| Wildmenu + picker drawing, highlight groups | `flux-tui/src/draw/picker.rs`: wildmenu selection style, prompt+matches, truncation marker, long query (4 tests) |
| `crates/flux-vim/tests/completion.rs` | Insert-mode keyword completion only; cmdline Tab is covered in engine.rs instead |

Gaps, all manual-only and covered by R3/R4 captures: engine test for `:lsp <Tab>` contexts; engine test for `<C-n>`/`<C-p>` picker movement. Non-blocking follow-up test items.

## R3 picker checklist — all PASS

Setup: tmux 80x24 and 40x12, `cargo run -- .` in a clean worktree of `main`, key sequences with `capture-pane` excerpts in raw captures.

- R3.1 `:Files` opens (`Files> ` + match list); typing narrows fuzzily; `<Down>`/`<Up>` and `CTRL-N`/`CTRL-P` move selection (ANSI-verified); `<CR>` opens the selected file; `<Esc>` closes with buffer, cursor, and mode intact.
- R3.2 `:Files` with pre-filled query selects the expected file; empty query in a git repo lists no ignored entries (F-01 spot check still holds).
- R3.3 `:Buffers` with 2–3 open buffers lists them and `<CR>` switches; single buffer opens and lists the one buffer (recorded as-is, non-blocking).
- R3.4 Query matching nothing: list empty, `<CR>` closes picker, buffer unchanged, no crash.
- R3.5 Names with spaces, non-ASCII, and wide characters (created in `<tmp>`) display and open correctly.
- R3.6 Resizing the tmux window with the picker open redraws without crash or garbage. 40x12 render captured (see B1).

## R4 Tab-completion checklist — flux vs Neovim 0.12.5 (`nvim --clean`, same dir/size)

- R4.1 Ex commands: `:Fi<Tab>` completes to the flux-only `:Files` (Neovim has no match; both case-sensitive, no deviation). `:b<Tab>` completes both; flux lists alphabetically (recorded 2026-10-03 decision, not a finding).
- R4.2 File paths: `:e <Tab>` lists files and dirs (dirs end in `/`); `:e <subdir>/<Tab>` descends; `:e ~/<Tab>` completes home entries (first-pick order differs, see N2).
- R4.3 Buffer names: `:b`/`:bd <Tab>` complete buffer names both (flux excludes directory buffers, see N3).
- R4.4 Options: `:set <Tab>`, `:set nu<Tab>` complete both (first pick differs, see N2); option-value positions complete nothing both.
- R4.5 Registers/LSP: `:reg`/`:put <Tab>` complete register names in flux where Neovim inserts a literal Tab (intended scope, see N4). `:lsp <Tab>` lists subcommands; `:lsp re<Tab>` completes to restart.
- R4.6 Cycling: `<Tab>` repeats forward and wraps through the original text; back-tab reverses to the original; `<Esc>` cancels and restores; typing after a candidate accepts it (identical both editors).
- R4.7 No regressions: Insert-mode `<Tab>` inserts a tab both; `<Tab>` with no candidates leaves text alone both (fail/bell, no crash).

## R5 screen and color sanity — as observed

- Picker selection uses `Pmenu`+`PmenuSel` (reverse); wildmenu selection uses `StatusLine`+`WildMenu` (reverse). ANSI-verified, matching draw tests.
- Under `TERM=dumb`, flux still emits color (consistent with the recorded NO_COLOR decision); legible in tmux. Recorded as observed (see N5).

## User hands-on pass (Oct 14, 2026) — all passed

The user ran the hands-on checklist on `main @ 1726eb7`: all steps passed. Taken as input to the sign-off, not as verification (per spec, Fina's gate is the sign-off).

## Findings

### B1 (was BLOCKING, owner Yuna) — CLOSED by PR #20

First pass found the centered picker painting only its inset columns, leaving margin columns showing buffer cells (illegible rows such as prompt text glued to filenames) whenever picker rows overlapped buffer text at 80x24 and 40x12. Function was unaffected (filter/move/open/Esc all worked). Re-verify on `d0340e9` re-ran R1 gates plus R3 pickers over a repo root and over a full text buffer at both sizes: every picker row fully picker-owned, zero bleed; filter, `<CR>`-open, `:Buffers`, `<Esc>`, and empty-match all behave as before. **B1 closed.** F-35 encodes this as layer-2 check C1 (screen test in `flux-tui`, owner Yuna, shipped in PR #20).

### Non-blocking follow-ups N1–N6 (for backlog/docs, not blockers)

- N1 Tifa B3 skipped per default; no copy/spacing issues found (prompts `Files> `/`Buffers> ` and the overflow marker all reasonable). No Tifa items from Fina.
- N2 Candidate order differs from Neovim though both sides complete validly (first picks differ on options and home paths). Suggest recording flux's order as a documented deviation alongside the 2026-10-03 alphabetical decision.
- N3 Flux excludes directory buffers from `:b`/`:bd` completion and `:Buffers`; Neovim offers its file-browser buffer. Suggest recording as intended (consistent with each other).
- N4 Flux completes register names after `:reg`/`:put` where Neovim inserts a literal Tab — planned scope that intentionally exceeds Neovim. Record as intended scope, not a parity gap.
- N5 `TERM=dumb` still gets color; legible. Recorded as observed (extends the NO_COLOR decision thematically).
- N6 (user-reported, Tifa): no notification when an LSP server starts. Non-blocking per user instruction.

Unspecified behaviors recorded as-is (non-blocking): single-buffer `:Buffers` lists the buffer; Enter on an empty match closes the picker unchanged (bell/fail); `:Buffers` numbering `N: name`.
