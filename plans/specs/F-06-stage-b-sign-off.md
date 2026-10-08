# F-06 — Stage B verify and sign-off (pickers + command-line Tab completion)

| | |
|---|---|
| Backlog | F-06 (M7 finish; PRD "Stage B sign-off", `plans/flux-1.0-prd.md:45`) |
| Size | S (about half a day of verification; fixes, if any, are separate items) |
| Spec status | 📄 Written Oct 7, 2026 |
| Repo state | `BobaFetch/flux` @ `55b2313` (main), read Oct 7, 2026 |
| Kind | **Verification and record-keeping, not a feature.** No product code is expected to change |
| Owner | Fina (verification gate, plan step B4 in `.sectorfive/plans/m7.md`). Y'shtola records the verdict and the evidence (`.sectorfive/**`) |
| Sector 5 routing | Y'shtola schedules B4 → Fina verifies on Muse Spark 1.3 high (this is a gate over code, even if the deliverable is Markdown) → Y'shtola opens one Markdown-only PR with the evidence and the `decisions.md` entry → the PM merges after Fina's approval is recorded and CI is green. Any defect found goes to its owner as a new item: Lightning (engine, `flux-vim`/`flux-view`), Yuna (`flux-tui`, drawing), Tifa (copy) |

## 1. Problem

Stage B of M7 (`:Files`/`:Buffers` pickers and command-line `<Tab>`/`<S-Tab>` completion) shipped without its gate.

- `.sectorfive/plans/m7.md` requires each stage to go Lightning → Yuna → Tifa → Fina, with "Fina's approve gat[ing] the next stage" (step **B4**: "corpus/matcher tests, oracle additions if planned, manual picker + completion checks").
- `.sectorfive/decisions.md` records "Stage A approved by Fina" (2026-10-03) and the two Stage B scope/design decisions (2026-10-03). It has **no Stage B approval** and no Tifa B3 entry.
- `plans/repo-review.md:65` marks Stage B **Partial**: the code is in `flux-view/src/{picker,matcher}.rs`, `flux-vim/src/complete.rs`, `flux-vim/src/ex.rs` (`:Files`/`:Buffers`, completion targets around `ex.rs:591–626`) and `flux-tui/src/draw/picker.rs`. "No Fina approval recorded."
- The one reproduced Stage B bug (`:Files` ignored `.gitignore`) was F-01, shipped in #13. Fina's F-01 approval (2026-10-06) "unblocks F-06".
- The PRD makes the sign-off the **team's verify gate**: "Pickers (`:Files`, `:Buffers`) and command-line Tab completion pass the team's verify gate, recorded in `.sectorfive/decisions.md`." F-12 (M6/M7 docs closeout) depends on F-06.

So F-06 is mostly a checklist run plus a written record. Agents can prepare and run all of it. The user signs nothing unless they choose to (Open question 1).

## 2. Goal

A recorded, evidence-backed verdict on Stage B as it exists on `main`: either **Stage B approved by Fina**, with the evidence linked, or a veto with a short list of owned fix items that are re-verified before approval.

## 3. Non-goals

- New picker or completion features (preview, multi-select, dedicated keys or mappings, `:colorscheme` completion). Those belong to Stage C / F-07+ (`decisions.md` 2026-10-03 B scope defers keys to Stage C maps).
- Fixing defects inside F-06. Each defect becomes its own item, routed by `ownership.md`.
- The M7 section of `docs/milestones.md` (description, manual check, ✅). That is F-12. F-06's evidence feeds it.
- Re-verifying F-01. Its gitignore and fair-cap behavior was approved on 2026-10-06; F-06 only spot-checks that it still holds.
- Performance budgets (F-24) and Linux manual runs. Linux is covered by CI tests; the manual checks run on macOS (yonaka).
- Recording new oracle cases. Stage B planned none ("oracle additions if planned"), and Mac recording is F-23.

## 4. What agents prepare vs. what only the user can do

| Agents (Fina, Y'shtola) | User |
|---|---|
| Run the automated gates and the Stage B tests; audit test coverage against the Stage B scope | Nothing required. Optionally, a 10-minute hands-on pass with the same checklist (Open question 1) |
| Run the manual checklist (§5 R3–R5) in tmux on macOS and save `capture-pane` output for each step | Decide Open questions 1–3 if the defaults below aren't right |
| Compare the wildmenu with Neovim 0.12.5 side by side in tmux | |
| Write the verdict; record it in `decisions.md` with the evidence (Markdown PR) | |

## 5. Requirements

- **R1. Baseline gates on `main`** (current head at verification time, named in the report): `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `env NO_COLOR=1 TERM=dumb cargo test --workspace`, `cargo deny check`, and `cargo xtask indent check` / `colors check` where Neovim 0.12.5 is available. Also link the green CI run for that head (all four checks: `test (ubuntu-latest)`, `test (macos-latest)`, `lint`, `neovim`).
- **R2. Test coverage audit.** Map each Stage B scope item (m7.md Stage B bullets, plus the two 2026-10-03 B decisions) to the unit or integration tests that cover it: matcher, picker state, `:Files`/`:Buffers` open, completion target detection (`ex.rs`), candidate lists (`complete.rs`), wildmenu stepping, picker drawing (`flux-tui/src/draw/picker.rs`), and `crates/flux-vim/tests/completion.rs`. List gaps. A gap that the manual checks cover is non-blocking and goes to a follow-up test item. A gap in behavior that the manual checks can't observe is blocking.
- **R3. Picker checklist** (tmux, 80×24 and 40×12, `cargo run -- .` in a clean worktree of `main`). For each step, record the key sequence and a `capture-pane` excerpt:
  1. `:Files` opens the picker (prompt `Files> `, match list). Typing narrows the list fuzzily. The selection moves with `<Down>`/`<Up>` and `CTRL-N`/`CTRL-P` (both are bound in `flux-vim/src/engine.rs:876–877`). `<CR>` opens the selected file. `<Esc>` closes the picker and leaves buffer, cursor and mode as they were.
  2. `:Files oracle.lua` pre-fills the query and selects `xtask/oracle.lua` (F-01 spot check). With an empty query the list has no `target/` entries.
  3. `:Buffers` with 3 open buffers lists them, and `<CR>` switches buffer. With a single buffer it still opens and behaves sensibly (record what it does).
  4. A query that matches nothing: record what `<CR>` does (expected: nothing opens, no crash, picker stays or closes cleanly).
  5. File names with spaces, non-ASCII and wide characters (create them in a temp dir) display and open correctly.
  6. Resizing the tmux window while the picker is open redraws without a crash or garbage.
- **R4. Tab-completion checklist** (same setup; Neovim parity is the goal for the wildmenu row, per the 2026-10-03 B scope decision). For each context, capture flux and `nvim --clean` 0.12.5 side by side, in the same directory and at the same size:
  1. Ex commands: `:Fi<Tab>`, `:b<Tab>` (alphabetical candidate order is a documented deviation, 2026-10-03 B design, so it's not a finding).
  2. File paths: `:e <Tab>`, `:e crates/<Tab>` (directories end in `/`), `:e ~/<Tab>`.
  3. Buffer names: `:b <Tab>`, `:bd <Tab>`.
  4. Options: `:set <Tab>`, `:set nu<Tab>`, and the option-value positions covered by `option_forms_and_value_positions`.
  5. Registers: `:reg <Tab>`, `:put <Tab>`. LSP: `:lsp <Tab>`.
  6. Cycling: `<Tab>` repeatedly, then `<S-Tab>` back, wrapping through the original text. `<Esc>` cancels and restores. Typing after a candidate accepts it.
  7. No regressions: `<Tab>` in Insert mode still inserts a tab, and `<Tab>` with no candidates matches Neovim (record both).
- **R5. Screen and color sanity.** Picker and wildmenu use the expected highlight groups (`Pmenu`/`PmenuSel` for the picker, `StatusLine`/`WildMenu` for the wildmenu row, per `draw/picker.rs`), are legible in the default scheme, and render without color under `TERM=dumb` (flux ignores `NO_COLOR` by decision; record as observed).
- **R6. Findings policy.**
  - A behavior that breaks a Stage B scope item, crashes, corrupts the screen, or regresses existing behavior is **blocking** (veto).
  - A deviation from Neovim's wildmenu that isn't a recorded decision is a finding. Fina marks it blocking only if it breaks a scope item; otherwise it's non-blocking with a follow-up.
  - Unspecified picker behavior (R3.3, R3.4) is recorded as observed and is non-blocking unless it crashes or loses state.
  - Copy or spacing issues are non-blocking follow-ups for Tifa (see Open question 2).
  - No finding is fixed inside F-06. Each blocking finding becomes an owned item, and F-06 is re-verified after it merges.
- **R7. Record.** On approval, Y'shtola opens one Markdown-only PR that:
  - adds `YYYY-MM-DD: Stage B approved by Fina (<model>, main @ <sha>; evidence: .sectorfive/evidence/stage-b.md)` to `.sectorfive/decisions.md`;
  - adds `.sectorfive/evidence/stage-b.md` with the checklist, captures (trimmed to the relevant rows), the coverage map and any follow-ups.
  
  On veto, the same file records the findings and the follow-up items instead, and no approval entry is written. No secrets or machine-specific paths go in the evidence (replace them with `<repo>`, `<tmp>`).

## 6. Acceptance checks

- **A1.** R1 gates all pass on the named `main` head, and the CI run link shows the four checks green.
- **A2.** The coverage map (R2) lists every Stage B scope item with its tests or "manual only", and each gap is classified.
- **A3.** Every R3 and R4 step has a key sequence and a capture. The R4 steps each have a matching Neovim capture.
- **A4.** Findings are classified per R6, and every blocking finding names its owner and a proposed item.
- **A5.** On approval: the `decisions.md` entry and `.sectorfive/evidence/stage-b.md` are merged, and the F-06 backlog row becomes 🚢. On veto: the follow-up items exist in the backlog, and F-06 stays 📄 until a re-verify approves.
- **A6.** No product code, tests or recorded expectations changed in the F-06 PR (`git diff --stat` shows only `.sectorfive/**`).

## 7. Risks

- **Unspecified behavior turns into argument.** R6 makes unspecified picker behavior non-blocking, and parity applies only to the wildmenu.
- **Evidence bloat.** Trim captures to the relevant rows and keep one capture per step.
- **Stage C pressure.** F-07 doesn't technically depend on F-06, but m7.md says Fina's B approval gates Stage C. If F-07 work starts first, Y'shtola records a plan amendment.

## Open questions (user decision)

1. **Who signs?** The PRD says Fina's recorded gate is the sign-off. Is that enough, or do you also want to do the hands-on checklist yourself before F-06 is marked shipped? Default: Fina's gate is the sign-off, and your pass is optional.
2. **Tifa B3:** m7.md plans a Tifa polish step for Stage B, and none is recorded. Should a Tifa pass run before sign-off, or should it be skipped with that rationale recorded? Default: skip, and Fina lists any copy or spacing issues as non-blocking Tifa follow-ups.
3. **Picker edge behavior:** for cases nothing specifies (Enter on an empty match list, `:Buffers` with one buffer), should current behavior be accepted if it's sensible and recorded, or do you want a specific behavior (e.g. fzf.vim's)? Default: record as-is, non-blocking.
