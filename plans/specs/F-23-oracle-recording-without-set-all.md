# F-23 — Oracle recording works on every Neovim build: replace `:set all&`

| | |
|---|---|
| Backlog | F-23 (M9 release hardening; tech-debt HIGH). Unblocks F-31 (CONTRIBUTING: "add a parity case") and any new oracle cases on the owner's Mac (F-15, F-17, F-18) |
| Size | S (≈ half a day, plus one run on the owner's Mac) |
| Spec status | **Ready to implement.** No owner decision needed. The macOS fix is unverified until someone runs A4 on the Mac (no Mac was available to the PM) |
| Repo state | `BobaFetch/flux` @ `d8ab89b` (main), read Oct 6, 2026 |
| Proposed branch / PR title | `oracle-option-reset` / **Reset options between oracle cases without :set all&** |
| Sector 5 routing | Lightning (`xtask/oracle.lua`, `xtask/src/main.rs`; Lightning owns `xtask/**`) → Fina (verify on Linux and on the owner's Mac; record baseline). Docs via Y'shtola (`.sectorfive/tech-debt.md`, `baseline.md`). No `gen` |

Self-contained: everything needed is in this file and the repo.

## 1. Problem

`cargo xtask oracle gen|check` can't run on the owner's Mac, so new Neovim parity cases can't be recorded and the recorded ones can't be re-verified there.

**Evidence:**
- `.sectorfive/tech-debt.md`, first entry, **[HIGH]**: "`cargo xtask oracle gen|check` broken with local nvim: `vim.cmd("set all&")` (xtask/oracle.lua:77) silently ends `-l` scripts (exit 0, no further output) under Homebrew nvim 0.12.5 / LuaJIT 2.1.1788856981 — reproduced unsandboxed; `:set number` fine, `+cmd` mode fine. … Options: different nvim build, or replace `set all&` with explicit per-option resets."
- `.sectorfive/baseline.md`: "`cargo xtask oracle check` → BLOCKED (environment) …".
- `xtask/oracle.lua:77` runs `vim.cmd("set all&")` before every case (inside the loop at `:65–125`), followed by `only!`, `%bwipeout!`, `cd`, `clearjumps`, `edit!` and register/history/mark resets (`:78–94`).
- `xtask/src/main.rs:47–73` (`run_nvim`): runs `nvim --headless --clean -l xtask/oracle.lua cases.json <tmp>`; on exit 0 it reads `<tmp>`. When the script ends early, nothing was written, so the user sees only a bare I/O error from `fs::read_to_string(&out)?` (`:67`), with no hint of the cause.
- `oracle_gen` (`main.rs:92–98`) writes whatever came back with no count check; `oracle_check` (`:101–121`) compares lengths but not ids or order.
- CI is unaffected: `.github/workflows/oracle.yml` uses `rhysd/action-setup-vim@v1` with `version: v0.12.5` on `ubuntu-latest`.

**Verified by the PM on a Linux x86_64 test machine (Oct 6, 2026; official `nvim-linux-x86_64.tar.gz` v0.12.5, `LuaJIT 2.1.1774638290`), in a throwaway worktree:**
- Baseline, unchanged script: `cargo xtask oracle check` → `all 1163 expectations match NVIM v0.12.5` (≈ 45 s including the xtask build).
- **Prototype of R1** (snapshot every option's global and local value at startup, restore the ones that differ before each case, in place of `set all&`) → `all 1163 expectations match NVIM v0.12.5`.
- Control, no reset at all → `252 of 1163 cases differ` (first ids: `o-ts-shift, o-et-shift, o-et-tab, o-et-ts, o-sts, …`). So the reset matters, and the prototype reproduces `set all&` for every case in the corpus. 91 cases type `:set`, `:setlocal` or `:setglobal`.
- **Not verified:** that the prototype runs to completion under Homebrew nvim 0.12.5 on macOS. The root cause of the silent exit was not found; R1 avoids the command instead of explaining it.

## 2. Goal

The oracle recorder resets editor state between cases without `:set all&`, gives the same results as today on Linux, runs on the owner's Mac, and fails loudly (never silently or with a bare I/O error) if Neovim stops early.

## 3. Non-goals

- Re-recording `expected.json` (no `gen` in this PR; it must stay byte-identical).
- Changing `cases.json`, the Rust replay test (`crates/flux-vim/tests/oracle.rs`), or the other harnesses (`xtask/indent.lua`, `xtask/colors.lua`, `xtask/screens/`; none uses `set all&`).
- Finding the Homebrew/LuaJIT root cause or filing a Neovim bug (optional follow-up; see Owner notes).
- Changing the CI Neovim pin.

## 4. Current vs required behavior

| Situation | Current | Required |
|---|---|---|
| Linux, official nvim 0.12.5, `oracle check` | all 1163 match | all 1163 match (unchanged) |
| macOS, Homebrew nvim 0.12.5, `oracle check` | script ends silently at the first case; xtask fails with a bare "No such file or directory" | runs to completion; all match, or a list of differing ids (no silent end) |
| A case changes an option (`:set ts=4`, `:setlocal nu`) | `set all&` resets it before the next case | the startup value is restored before the next case, global and local |
| nvim exits 0 without writing results | bare I/O error | `nvim exited without writing results (xtask/oracle.lua ended early); see .sectorfive/tech-debt.md`, non-zero exit |
| nvim returns fewer results, or ids out of order | `gen` writes them; `check` reports a length mismatch | both `gen` and `check` refuse, naming the first missing or mismatched id |
| An option can't be restored | n/a | the run stops with `oracle.lua: cannot restore '<option>' before case <id>: <error>`, non-zero exit |

## 5. Detailed requirements

**R1. Snapshot and restore instead of `set all&`.** In `xtask/oracle.lua`:
- Before the case loop, record for every option in `vim.api.nvim_get_all_options_info()` its global value (`nvim_get_option_value(name, { scope = "global" })`), its scope, and (for non-global options) its local value (`{ scope = "local" }`).
- Replace `vim.cmd("set all&")` (line 77) with a call that, for each option, compares the current global value (and local value for non-global options) with the snapshot using `vim.deep_equal`, and only for differing ones calls `nvim_set_option_value(name, saved, { scope = "global" | "local" })`.
- Keep the call at the same point in the sequence (before `only!`/`%bwipeout!`/`edit!`), so the new buffer copies restored global values as before.
- Add a comment saying why (`set all&` ends `-l` scripts in some builds; see tech-debt) and that the reference state is "Neovim as `--headless --clean` starts it".

**R2. Fail loudly on restore errors.** Wrap each `nvim_set_option_value` in `pcall`; on failure, call `error(("oracle.lua: cannot restore '%s' before case %s: %s"):format(name, case.id, err))`. In `-l` mode an uncaught error makes nvim exit non-zero with the message on stderr, and `run_nvim` already reports stderr on non-zero exit (`main.rs:60–66`).

**R3. Detect an early end.** In `run_nvim`, if nvim exits successfully but the output file doesn't exist, `bail!` with: `{nvim} exited without writing results (xtask/oracle.lua ended early); see .sectorfive/tech-debt.md` followed by nvim's stderr and stdout (they may be empty).

**R4. Validate results in `gen` and `check`.** A shared function, e.g. `fn validate_results(cases: &[Value], results: &[Value]) -> Result<()>`, checks that the counts are equal and `results[i]["id"] == cases[i]["id"]` for every `i`, and on failure bails with the first offending index and id: `oracle results don't match cases.json at #<i>: expected '<case id>', got '<result id or "nothing">'`. `oracle_gen` must call it before writing `expected.json`; `oracle_check` calls it before comparing.

**R5. Results unchanged.** On Linux with nvim v0.12.5, `cargo xtask oracle check` must still report all cases matching, without regenerating anything.

**R6. No other behavior change.** Case order, temp-dir layout, per-case resets (`only!`, `%bwipeout!`, `clearjumps`, registers, history, marks) and the output format stay as they are.

## 6. Edge cases

1. **Options that only exist on some builds or platforms:** the snapshot comes from the running nvim, so it's self-consistent.
2. **Read-only or special options** (e.g. `'columns'`, `'lines'` in headless mode): restored only if a case changed them; if one ever can't be set, R2 stops the run with its name rather than recording wrong results.
3. **Window-local values on the surviving window:** `only!` keeps the current window, so its local values must be restored (R1 covers local scope). The control run (no reset) shows `o-nu-sp-local` and similar cases depend on this.
4. **Global-local options** (local value "unset"): compare and restore both scopes exactly as read; `deep_equal` handles the empty-string/`-1` sentinels.
5. **Buffer-local values:** the old buffer is wiped and the new one copies global values, so restoring the global value is what matters; restoring local values on the about-to-be-wiped buffer is harmless.
6. **Cases that change `'filetype'`/`'syntax'`:** handled by the same rule; the next case's `edit!` re-detects.
7. **F-03 interaction:** F-03 retags `m` fields in `cases.json` and doesn't change ids or order, so R4 is unaffected. The two PRs can merge in either order.

## 7. Acceptance criteria

`NVIM_BIN` selects the Neovim to use (`xtask/src/main.rs:43–45`).
- **A1 gates:** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo deny check` pass.
- **A2 Linux parity:** with official nvim v0.12.5 (CI's `rhysd/action-setup-vim`, or the release tarball), `cargo xtask oracle check` → `all 1163 expectations match NVIM v0.12.5`. `git status` shows no change to `crates/flux-vim/tests/oracle/`.
- **A3 the reset is exercised (temporary edit, revert before commit):** comment out the restore call → `cargo xtask oracle check` fails with at least 200 differing cases (the PM's run: 252). Restore → passes again.
- **A4 macOS (owner's Mac, Homebrew nvim 0.12.5):** `cargo xtask oracle check` runs to completion (≈1 minute), then either reports all match, or lists differing ids. **Do not run `gen`.** Paste the output in the PR. If it still ends early, R3's message must appear instead of a bare I/O error; then record the result in tech-debt and leave the entry open.
- **A5 early-end message (temporary edit):** add `os.exit(0)` before the `writefile` at the end of `oracle.lua` → `cargo xtask oracle check` exits non-zero with the R3 message. Revert.
- **A6 validation (unit tests, § 8):** pass.
- **A7 CI:** the `Neovim oracle` workflow runs on the PR (its `xtask/**` path filter matches) and is green.

## 8. Tests to add

`xtask` has no tests today; add `#[cfg(test)] mod tests` in `xtask/src/main.rs` (it runs under `cargo test --workspace`, since `xtask` is a workspace member):
- `validate_accepts_matching_ids` — 3 cases, 3 results with the same ids → `Ok`.
- `validate_rejects_short_results` — 3 cases, 2 results → error naming `#2` and the third id, `got 'nothing'`.
- `validate_rejects_reordered_ids` — error at the first mismatch.
- `missing_output_message` — if `run_nvim`'s file check is factored into a small function (`fn read_results(path, stderr, stdout) -> Result<Vec<Value>>`), assert the R3 text for a nonexistent path.
No test runs Neovim (CI's test job doesn't have it; the oracle workflow covers A2).

## 9. Files likely touched (guidance)

- `xtask/oracle.lua` (R1, R2).
- `xtask/src/main.rs` (R3, R4, tests).

## 10. Docs to update (via Y'shtola)

- `.sectorfive/tech-debt.md`: after A4 passes on the Mac, mark the HIGH entry resolved (or remove it) with the date and PR; if A4 fails, update it with the new evidence.
- `.sectorfive/baseline.md`: Fina replaces the "oracle check → BLOCKED" line with the A2/A4 results.
- `AGENTS.md` Neovim Fixtures section: no change needed (commands unchanged).

## 11. Risks

- **Mac still fails for another reason.** R1 removes the known trigger but the root cause is unknown. Mitigation: A4 is mandatory before closing the tech-debt entry; R3 makes any failure visible.
- **Hidden state differences** between `set all&` (compiled defaults) and the startup snapshot (defaults plus what `--clean` startup sets). The full corpus gives identical results on Linux (§ 1), so there's no difference for today's cases. A future case that depends on such an option would show up as a gen diff, which is reviewed anyway.
- **Platform differences on the Mac** unrelated to this change (e.g. a different `'shell'`): A4 may list differing ids. Record them; don't `gen` from the Mac until they're understood.

## 12. Definition of done

- [ ] A1–A3, A5–A7 pass; A4 run on the owner's Mac and pasted in the PR (or recorded as still failing, with the R3 message).
- [ ] `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; `cargo deny check`: pass, in `AGENTS.md` order.
- [ ] No `gen`: `expected.json`, indent expectations and `colors.json` byte-identical.
- [ ] PR titled **Reset options between oracle cases without :set all&** (sentence-case imperative, squash-merged with `(#N)`).
- [ ] PR description: A2/A3/A4 outputs and a **Fina verify** block.
- [ ] After approval, Y'shtola updates tech-debt and decisions (`YYYY-MM-DD: F-23 approved by Fina (...)`), and Fina updates `baseline.md`.

## Owner notes (non-blocking)
- If A4 passes, the Mac becomes a valid recording machine again. Recording from Linux CI stays the reference, because `expected.json`'s `NVIM_VERSION` line doesn't capture the build.
- Optional follow-up: a minimal repro (`nvim --headless --clean -l` of a file containing only `vim.cmd("set all&") print("after")`) on the Mac, filed with Homebrew or Neovim if it reproduces.
