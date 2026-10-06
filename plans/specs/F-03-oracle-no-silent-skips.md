# F-03 — No silently skipped oracle cases

| | |
|---|---|
| Backlog | F-03 (M7 finish / quality gates); unblocks honest numbers for F-15, F-17, F-18 |
| Size | S (≈ half a day) |
| Spec status | **Shipped** in #10 (`b7f3341`), Oct 6, 2026 |
| Repo state | `BobaFetch/flux` @ `27f9a2a` (main), read Oct 5, 2026 |
| Proposed branch / PR title | `oracle-deferred` / **Run deferred oracle cases and report them instead of skipping silently** |
| Sector 5 routing | Fina (`crates/flux-vim/tests/oracle.rs`, `cases.json` tags; Fina owns `crates/*/tests/**` per `.sectorfive/ownership.md`) → docs via Y'shtola. No Lightning, Aerith or Tifa work. No `gen`: `expected.json` is untouched |

## 1. Problem

The oracle test skips cases tagged above the current milestone without running or reporting them, and the docs overstate what is verified.

**Evidence:**
- `crates/flux-vim/tests/oracle.rs:11` has `const MILESTONE: u64 = 6;`. `oracle.rs:240-242` does `if case["m"].as_u64().unwrap() > MILESTONE { continue; }`: no run, no count, no output.
- `AGENTS.md:25` documents this: "Oracle cases above the `MILESTONE` constant … are silently skipped."
- `crates/flux-vim/tests/oracle/cases.json` has 1,163 cases. By tag: m0 38, m1 303, m2 207, m3 175, m4 347, m5 39, m6 42, **m9 12**. So 1,151 run and 12 are skipped.
- **6 of the 12 already pass.** Probe: a temporary clone with `MILESTONE = 9`, then `cargo test -p flux-vim --test oracle` reported "6 of 1163 oracle cases differ".
  - Pass: `g-delete`, `v-delete`, `g-normal`, `range-normal`, `range-normal-long`, `g-normal-long` (`cases.json:1048-1051`, `1080-1081`).
  - Still differ: `ctrl-a`, `vblock-insert`, `vblock-delete`, `vblock-append-eol`, `paren-sentence`, `ctrl-v-dollar-c`.
- The 6 passing cases test `:g`, `:v` and `:norm`, which **shipped in M4**: `docs/milestones.md:211-212`, commit `39e9f84` "M4: search and Ex (#3)". The cases themselves date from M0 (`f868c80`) with the placeholder tag 9.
- `docs/milestones.md:402` (M6) says "All 1163 oracle cases match Neovim 0.12.5 (54 new, mostly `gq`/`gw`)." Both numbers are wrong:
  - Cases with m ≤ 6 numbered 1,151.
  - M6 added 42 cases (1,121 total at `10882cf` M5 → 1,163 at `4eb26c3` M6), all 42 tagged m6 and all `gq`/`gw`.
  - "54" is 1,163 − 1,109, mixing the M5 in-scope count with the M6 total.
  - Earlier milestone claims are correct: M4 "1070" and M5 "1109" equal the m ≤ 4 and m ≤ 5 counts.
- `README.md:31-34` says the test "replays every case up to the current milestone"; true, but it doesn't say the rest are skipped.
- No other suite skips anything: there are no `#[ignore]` attributes in `crates/`, and the indent and completion suites don't filter except through the explicit `FLUX_INDENT` (`crates/flux-vim/tests/indent.rs:80`).

## 2. Goal

Every recorded Neovim case runs on every test run. Deferred cases are visible and must still differ, so mis-tagged cases surface on their own, and the docs state the verified numbers exactly.

## 3. Non-goals
- Implementing visual-block, `CTRL-A`/`CTRL-X` or sentence objects (F-15, F-17, F-18).
- Retagging the 6 still-failing cases to a specific future milestone (they stay `m: 9`; see Owner notes).
- Re-recording expectations (`cargo xtask oracle gen`) or touching `expected.json`. `xtask` never reads `"m"` (`rg '"m"|\.m\b' xtask/` finds nothing), so tag edits need no re-record.
- Changing the indent or colors suites, or `cargo xtask oracle check`.
- Fixing `xtask/oracle.lua:77` on macOS (F-23).

## 4. Current vs required behavior
| | Current | Required |
|---|---|---|
| Cases with `m > MILESTONE` | Not run, not counted, not mentioned | Run every time. Each must **differ** from Neovim (strict expected-difference) |
| A deferred case starts matching | Nobody notices | A test fails and names it, asking for a retag |
| Test output | One test, `matches_neovim`, with no counts on success | Two tests, `matches_neovim` and `deferred_cases_still_differ`, each printing a one-line summary (visible with `--show-output`) |
| The six M4 cases | Tagged 9, skipped | Tagged 4, run as in-scope |
| Docs | "silently skipped"; "All 1163 … (54 new)" | Accurate counts and wording (§10) |

## 5. Detailed requirements
- **R1.** Every case in `cases.json` is executed on every `cargo test -p flux-vim --test oracle`. The in-scope count plus the deferred count must equal `cases.len()`; assert it.
- **R2. In-scope (`m ≤ MILESTONE`).** Behavior as today: every case must match. Keep today's failure message format (`oracle.rs:253-258`): `"{n} of {ran} oracle cases differ from Neovim:\n  {id} (keys …): …"`.
- **R3. Deferred (`m > MILESTONE`).** Every case must produce at least one difference from `run_case` (`oracle.rs:128-233`).
  - If any deferred case matches, fail with: `"{n} deferred oracle cases now match Neovim; set their \"m\" in cases.json to the milestone that implemented them:\n  {id} (keys …)"`, one line per case.
  - A panic while running a deferred case fails the test. A panic is a bug, never an expected difference.
- **R4. Two test functions** sharing the case loading and `run_case`:
  - `matches_neovim` (in-scope; keep the name so history and CI logs stay comparable) and `deferred_cases_still_differ`.
  - Each loads cases and expected output through the existing staleness checks (`oracle.rs:225-238`), either in both tests or in a shared helper.
- **R5. Summary line on stderr** from each test, with a narrow `#[allow(clippy::print_stderr)]` on the function. Workspace lints warn on `print_stderr` (`Cargo.toml [workspace.lints.clippy]`), and CI denies warnings. Precedent: `#[allow(clippy::print_stdout)]` at `crates/flux/src/main.rs:26`, `xtask/src/main.rs:91`. Exact formats:
  - `oracle: 1157 cases up to M6 match Neovim`
  - `oracle: 6 deferred cases (above M6) still differ: ctrl-a, vblock-insert, vblock-delete, vblock-append-eol, paren-sentence, ctrl-v-dollar-c`

  Order the ids as they appear in `cases.json`.
- **R6. Retag** exactly these six from `"m": 9` to `"m": 4`: `g-delete`, `v-delete`, `g-normal`, `range-normal`, `range-normal-long`, `g-normal-long`. Change nothing else on those lines; keep the one-case-per-line formatting.
- **R7.** Update the module doc comment (`oracle.rs:1-5`): cases above `MILESTONE` are deferred. They still run and must differ, and they get retagged when they start matching.
- **R8.** `MILESTONE` stays `6`. Bumping it is a milestone-landing decision (`AGENTS.md:25`).

## 6. Edge cases
1. A deferred case matches only partly: e.g. the text matches but the cursor differs. It still counts as differing (any diff), which is correct.
2. A future milestone bumps `MILESTONE` past a deferred tag: those cases become in-scope and must match. This is the intended flow.
3. All deferred cases removed or retagged: `deferred_cases_still_differ` passes and prints exactly `oracle: 0 deferred cases (above M6)`, with no trailing `still differ:` list.
4. `expected.json` stale or the ids misaligned: the existing assertions fire first, with the same messages.
5. Temp directories: `run_case` writes under `flux-oracle-<pid>/<id>` (`oracle.rs:42-48`). Two tests in one process share the pid, but case ids are unique per directory and each test runs a disjoint set of cases, so there are no collisions. Assert in a test that ids are unique.

## 7. Acceptance criteria
| # | Command | Expected |
|---|---|---|
| A1 | `cargo test -p flux-vim --test oracle -- --show-output` | `test matches_neovim ... ok` and `test deferred_cases_still_differ ... ok`; stdout/stderr shows exactly the two R5 lines |
| A2 (strictness) | Temporarily set `g-delete` back to `"m": 9`, then run A1 | `deferred_cases_still_differ` FAILS, naming `g-delete`. Revert |
| A3 (in-scope) | Temporarily set `ctrl-a` to `"m": 4`, then run A1 | `matches_neovim` FAILS with `1 of 1158 oracle cases differ`, naming `ctrl-a`. Revert |
| A4 | `python3 -c "import json,collections;c=json.load(open('crates/flux-vim/tests/oracle/cases.json'));print(len(c),sorted(collections.Counter(x['m'] for x in c).items()))"` | `1163 [(0, 38), (1, 303), (2, 207), (3, 175), (4, 353), (5, 39), (6, 42), (9, 6)]` |
| A5 | `git diff --stat origin/main...HEAD` | `expected.json` and `NVIM_VERSION` untouched; `cases.json` has exactly 6 changed lines |
| A6 | Oracle workflow (`.github/workflows/oracle.yml`, triggered by the `cases.json` change) | Green: recorded results still match Neovim 0.12.5 |
| A7 | `rg -n "silently skipped|All 1163" AGENTS.md README.md docs/` | No matches |

## 8. Tests to add or change
All in `crates/flux-vim/tests/oracle.rs`, following its style (`serde_json::Value`, `include_str!` fixtures, aggregated failure messages):
- `matches_neovim`: restricted to in-scope cases; adds the R5 summary; asserts R1's count equality.
- `deferred_cases_still_differ` (new): R3 and R5.
- `case_ids_are_unique` (new, cheap): no duplicate `id` in `cases.json`.

## 9. Files likely touched (guidance)
- `crates/flux-vim/tests/oracle.rs`
- `crates/flux-vim/tests/oracle/cases.json` (6 tag edits only)
- Docs listed in §10

## 10. Docs to update (via Y'shtola)
- `AGENTS.md:25`: "Oracle cases above the `MILESTONE` constant in `crates/flux-vim/tests/oracle.rs` are deferred: they still run and must differ from Neovim (`deferred_cases_still_differ`), so a case that starts matching fails the suite until its `m` tag is lowered. Update `MILESTONE` deliberately when landing a milestone."
- `README.md:31-34`: add "Cases tagged for later milestones still run and must differ; `-- --show-output` prints the counts."
- `docs/milestones.md:402`: correct the historical claim to "All 1151 M0–M6 oracle cases match Neovim 0.12.5 (42 new, all `gq`/`gw`); 12 cases recorded for later milestones were skipped." Then add one line in the M7 section, or wherever Y'shtola tracks current verification:
  > Deferred oracle cases now run: six `:g`/`:v`/`:norm` cases were retagged M4 (that milestone implemented them), so 1157 cases match. Six are deferred and checked to still differ: visual-block ×4, `CTRL-A`, `das`.
- `.sectorfive/tech-debt.md`: nothing to remove. Optionally note that the tag-9 placeholder now means "deferred, unscheduled".

## 11. Risks
- **Brittleness by design.** Partial work on visual-block or `CTRL-A` may make a deferred case pass early and fail the suite. That is the intent: whoever lands the change retags the case in the same PR. Say so in AGENTS.md (§10).
- **Runtime.** Deferred cases add 6 cases (about 0.5% of the total). The suite ran in about 0.75 s at opt-level 1; there is no meaningful cost.
- **Summary visibility.** libtest hides stderr for passing tests by default, so the summary shows with `--show-output`/`--nocapture` and on failure. The test names alone also make deferral visible in normal output. A custom harness (for example a libtest-mimic dependency) is out of scope.
- **Rollback:** revert the PR. Tags and the harness are self-contained.

## 12. Definition of done
- [ ] `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; `cargo deny check`: pass, in AGENTS.md order. A1–A7 pass, including the temporary-edit checks A2/A3, which must be reverted before commit.
- [ ] No `cargo xtask oracle gen`; `expected.json` byte-identical.
- [ ] PR titled **Run deferred oracle cases and report them instead of skipping silently** (sentence-case imperative, squash-merged with `(#N)`).
- [ ] PR description: the counts before and after (1151 → 1157 in scope; 12 skipped → 6 deferred and run), the A2/A3 outputs, and a **Fina verify** block (approve or veto with evidence).
- [ ] After approval, Y'shtola records `YYYY-MM-DD: F-03 approved by Fina (...)` in `.sectorfive/decisions.md`, and Fina updates the oracle line in `.sectorfive/baseline.md`.

## Owner notes (non-blocking)
- The 6 still-failing cases stay `m: 9`, read as "deferred, unscheduled". If you want them tied to the 1.0 plan's M8 ("1.0 gaps"), retag them to 8 when M8 is planned, not in this PR.
