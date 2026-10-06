# F-01 — `:Files` respects .gitignore and shares its file cap fairly

| | |
|---|---|
| Backlog | F-01 (M7 finish), blocks F-06 Stage B sign-off |
| Size | M (≈ 1–2 days agent time) |
| Spec status | **Ready to implement.** Owner approved the `ignore` dependency on Oct 5, 2026 as an explicit exception to the M7 plan's no-new-deps rule |
| Repo state | `BobaFetch/flux` @ `27f9a2a` (main), read Oct 5, 2026 |
| Proposed branch / PR title | `files-picker-gitignore` / **Respect .gitignore in :Files and share its file cap fairly** |
| Sector 5 routing | Lightning (walk, picker state, deps, unit tests) → Aerith (truncation marker render) → Tifa (marker copy) → Fina (verify). Docs via Y'shtola. |

## 1. Problem

`:Files` builds its list with `flux_view::explorer::walk_files(&editor.cwd, 5000)` (`crates/flux-vim/src/ex.rs:1444-1453`). The walker (`crates/flux-view/src/explorer.rs:37-82`):

- skips only names starting with `.` (`explorer.rs:59`), so build output (`target/`), `node_modules/` and everything else gitignored is listed;
- walks depth-first, each directory's files first and then its subdirectories in sorted order (`explorer.rs:70-81`), and stops at the cap (`explorer.rs:49`, `73`).

So one big early directory fills the cap and later directories are never reached.

**Evidence (reproduced Oct 5, 2026, in a clean clone at `27f9a2a` after `cargo test --workspace`):**

- `target/` held 5,473 non-hidden files. The current order put the first `target/` file at index 333 and `xtask/oracle.lua` at index 5,809.
- `walk_files(<repo>, 5000)` returned 5,000 paths, 4,667 of them under `target/`, and **no `xtask/oracle.lua`**. Run from a scratch crate outside the repo.
- So in the editor, `:Files oracle.lua` cannot find `xtask/oracle.lua`. The same symptom was seen earlier with a 6,741-file `target/`; the exact counts depend on the build.
- For comparison, a gitignore-aware walk (the `ignore` crate with default filters) over the same tree took 2 ms (release build, on the box). It returned 646 files: 0 under `target/`, `xtask/oracle.lua` included. That equals `git ls-files | grep -v '^\.' | wc -l` (646).

The existing unit test `walk_lists_relative_skips_hidden_and_caps` (`explorer.rs:249-269`) locks in the "files first, then truncate" order: cap 2 gives `["a.rs", "b.rs"]`.

## 2. Goal

`:Files` lists the files a developer considers part of the project: ignored files are left out, every directory gets a fair share when the cap applies, and the picker says when the list was cut.

## 3. Non-goals

- Changing `:Explore` / netrw listings (`list_dir`) or command-line `:e <Tab>` path completion. They keep showing ignored files, like Vim.
- Listing hidden files (dot-files and dot-directories stay excluded, as today).
- Async or incremental walking, file preview, a configurable cap, or a `'wildignore'` option.
- Changing matcher ranking (`crates/flux-view/src/matcher.rs`) or picker keys (`crates/flux-vim/src/engine.rs:849-920`).
- A hand-written gitignore matcher, or shipping fairness without ignore rules. The owner-approved `ignore` crate is the required approach (R13).

## 4. Current vs required behavior

| Situation | Current | Required |
|---|---|---|
| Repo with a built `target/` (gitignored) | `target/` files listed and fill the cap; `xtask/oracle.lua` missing | `target/` not listed; `xtask/oracle.lua` listed |
| Non-git directory where one subdirectory has more files than the cap | That subdirectory (or the earlier one) takes the cap; later siblings missing | Every sibling gets a max-min fair share (R4). Small directories are complete |
| Eligible files ≤ cap | All listed | All listed (unchanged) |
| List was truncated | Silent | Picker shows a truncation marker (R8) |
| Order of entries with an empty query | Per directory: files (sorted), then subdirectories (sorted), recursively | Same order, restricted to the selected set (R5) |

## 5. Detailed requirements

**Eligibility**
- **R1.** A path is listed only if all of these hold:
  - (a) It is a regular file or a symlink. Symlinks are not followed, and a symlink to a directory is listed as an entry and never descended into (today's behavior, `explorer.rs:62-66`).
  - (b) No path component below the walk root starts with `.` (today's behavior).
  - (c) It is not ignored by git-style rules: `.gitignore` in the walk root and every directory below it; `.gitignore` in ancestors of the walk root up to the repository root; `.git/info/exclude`; the user's global git excludes file; and `.ignore` files. Rules use gitignore semantics: negation `!`, directory-only `dir/`, anchored `/x`, `**`.
- **R2.** Git rules (`.gitignore`, `info/exclude`, global excludes) apply only when the walk root is inside a git repository, which is when an ancestor-or-self contains `.git`. This matches ripgrep and fd defaults. `.ignore` files apply everywhere.
- **R3.** Ignored directories are never descended into.

**Selection**
- **R4. Max-min fair share.** Define a directory's *buckets* as its own eligible files (one bucket) plus one bucket per eligible subdirectory (in sorted order), each with the count of eligible files in it. A directory given budget B splits B with this exact algorithm (normative):
  ```
  alloc[i] = 0 for all buckets; remaining = B
  unsat = buckets with count > 0, in order (own files first, then subdirs sorted)
  while remaining > 0 and unsat not empty:
      share = remaining / len(unsat)            # integer division
      if share == 0:
          give 1 to each of the first `remaining` buckets in unsat; remaining = 0; break
      for b in unsat: g = min(count[b] - alloc[b], share); alloc[b] += g; remaining -= g
      unsat = [b in unsat where alloc[b] < count[b]]
  ```
  - The walk root's budget is the cap.
  - Own files take the first `alloc` files by name.
  - Each subdirectory recurses with its `alloc` as its budget.
  - Worked examples (budget, counts → alloc): `(2,[2,1])→[1,1]`, `(10,[1,50,3])→[1,6,3]`, `(5,[10,1,10])→[2,1,2]`, `(7,[10,1,10])→[3,1,3]`, `(3,[1,1,1,1])→[1,1,1,0]`, `(0,[5])→[0]`, `(100,[5,5])→[5,5]`.
- **R5. Order.**
  - The returned list keeps today's order: for each directory, its selected files sorted by name, then its subdirectories sorted by name, recursively.
  - Name order is today's `OsString` ordering (`explorer.rs:70-71`).
  - Paths are relative to the walk root, as today.
- **R6. Completeness.** If the total number of eligible files is ≤ the cap, every eligible file is listed. R4 guarantees this; it is stated separately because it gets its own test.
- **R7. Bounded work.**
  - Stop collecting after `WALK_CEILING = 50_000` eligible files and build the list from what was collected.
  - Mark the result truncated.
  - The result must be deterministic for a given tree.
  - Work is depth-first in sorted order, so at the ceiling, later siblings may be unseen; this is accepted, see Risks.

**Surface**
- **R8. Truncation marker.**
  - The walk reports whether it truncated (by cap or ceiling), and the `Picker` carries that flag.
  - When truncated, the picker's prompt row shows `<listed>+` right-aligned in the `Pmenu` style, for example `5000+`. Tifa may adjust the copy; the spec only requires that truncation is visible.
  - Nothing is shown when the list is complete.
  - The `Files> ` label, the prompt text and the cursor position are unchanged.
- **R9.** The cap stays 5,000, as a named constant (for example `FILES_CAP`) instead of the literal at `ex.rs:1446`.
- **R10.** Errors (unreadable directories, unreadable or invalid ignore files) are skipped silently, with no message and no panic, as today (`explorer.rs:54-56`).
- **R11. Testability.** The walk is callable with an explicit cap, ceiling, and a switch to disable the user's global git excludes. Tests must not depend on the developer's `~/.config/git/ignore` or `core.excludesFile`. The editor itself enables global excludes.
- **R12. Performance.** Release build, on this repo with a built `target/`: the walk for `:Files` takes ≤ 20 ms. Worst case (ceiling hit) ≤ 250 ms on box-class hardware. These are estimates from the prototype (about 4 µs per eligible file warm, 15k files in ~60–120 ms). Report the measured numbers in the PR.

**Dependency (owner-approved exception)**
- **R13.** Required approach: use the `ignore` crate (ripgrep's gitignore engine) as a direct dependency of `flux-view` with an explicit version, following the repo's style for single-crate deps (`crates/flux-syntax/Cargo.toml`), for example `ignore = "0.4.33"`.
  - Suggested settings: `WalkBuilder` with `hidden(true)`, `parents(true)`, `ignore(true)`, `git_ignore(true)`, `git_exclude(true)`, `git_global(<configurable>)`, `require_git(true)`, `follow_links(false)`, `sort_by_file_name(Ord::cmp)`.
  - Implementation shape is the implementer's choice, as long as R1–R12 hold: for example, collect eligible files into a per-directory tree up to the ceiling, then apply R4 and emit in R5 order.
  - Do not hand-write gitignore matching and do not add any other new dependency.
- **R14. Record the exception** in `.sectorfive/decisions.md`, in the same PR, using the file's convention (one dated bullet). For example:
  ```markdown
  - 2026-10-05: Owner approved `ignore` 0.4.33 (flux-view) as an explicit exception to the M7 plan's "no new deps outside mlua" rule (D4 / Success Criteria), for F-01 gitignore-aware `:Files`. Adds 9 crates, all MIT, Apache-2.0 or Unlicense; `cargo deny check` green.
  ```
  `.sectorfive/**` is Y'shtola-owned (`.sectorfive/ownership.md`): the implementer drafts the line in the PR, and Y'shtola commits it.

## 6. Edge cases (each must behave as stated)
1. Walk root is a subdirectory of a repo: `.gitignore` files above the root, up to the repo root, apply.
2. Walk root not in a git repo: `.gitignore` is ignored (R2), `.ignore` still applies, and fairness (R4) prevents starvation.
3. Nested repo or submodule inside the root: its own `.gitignore` applies inside it.
4. Negation that tries to re-include a file under an excluded directory (`build/` plus `!build/keep.txt`): stays excluded, as in git.
5. `.gitignore` containing `*`: an empty list. The picker opens with no entries and `<CR>` fails the command as today (`engine.rs:1468-1473` test).
6. Symlink loop or symlink to a directory: listed as a single entry, never followed.
7. Non-UTF-8 file names: shown via `to_string_lossy`. `PickerValue::File` keeps the real `PathBuf` (`picker.rs:59-70`, unchanged).
8. Walk root unreadable: an empty list, no panic.
9. Root is `~` or `/` (huge, non-git): the ceiling bounds the work, the result is marked truncated, and the editor stays responsive (R12).
10. A gitignored file that is open in a buffer: not listed by `:Files`; `:Buffers` still lists it.
11. `.jj` directory: the `ignore` crate also treats `.jj` as a repo marker. Accept this; note it in the docs only if it shows up in testing.

## 7. Acceptance criteria
| # | Command / action | Expected |
|---|---|---|
| A1 (repro) | In a fresh clone: `cargo test --workspace` (populates `target/`), then `cargo run -- .`, type `:Files oracle.lua` then `<CR>` | Before: `xtask/oracle.lua` is not in the list. After: `xtask/oracle.lua` is the selected (first) entry and no `target/` path appears for an empty query (`<C-u>`) |
| A2 | Same session, `<C-u>` to clear the query | No truncation marker (about 646 entries, all tracked non-hidden files) |
| A3 | `mkdir -p /tmp/fair/{big,small} && for i in $(seq 1 6000); do : > /tmp/fair/big/f$i; done && touch /tmp/fair/small/{a,b,c} && cd /tmp/fair && <flux> .` then `:Files` | Marker `5000+` shown; `:Files small/` matches `small/a`, `small/b`, `small/c` |
| A4 | `cargo test -p flux-view explorer` | New and updated tests pass (§8) |
| A5 | `cargo test -p flux-vim pickers` and `cargo test -p flux-tui picker` | Pass |
| A6 | `cargo tree -p flux-view -e normal -i ignore` | Shows `ignore v0.4.x` used only by `flux-view` |
| A7 | `cargo deny check` | `advisories ok, bans ok, licenses ok, sources ok` (verified in a scratch copy: adding `ignore 0.4.33` adds 9 crates, all MIT, Apache-2.0 or Unlicense; no new warnings) |
| A8 | CI gates (§12) | Green |

## 8. Tests to add or change
Follow the existing style: unit tests in the same file's `#[cfg(test)] mod tests`, fixtures under `std::env::temp_dir()` named with `std::process::id()`, removed at the end (see `explorer.rs:249-269`, `engine.rs:1106-1114`). Create `.git` as an empty directory to mark a repo (that is enough for the `ignore` crate). Disable global excludes in walk tests (R11).

`crates/flux-view/src/explorer.rs`:
- `fair_shares_split_budget_max_min`: table test of the R4 allocation with the seven worked examples.
- `walk_respects_gitignore_in_a_repo`:
  - Fixture: `.git/`; `.gitignore` = `target/\n*.log\n!keep.log\n`; files `README.md`, `keep.log`, `a.log`, `src/main.rs`, `target/debug/x`.
  - Asserts exactly `["README.md", "keep.log", "src/main.rs"]`, not truncated.
- `walk_ignores_gitignore_outside_a_repo`: same fixture without `.git/`. Asserts `target/debug/x` and `a.log` are listed (R2).
- `walk_applies_ancestor_gitignore_from_repo_root`: `.git/` and `.gitignore` = `build/` at the top; walk from `top/sub`. Asserts `sub/build/**` is excluded.
- `walk_respects_nested_gitignore_and_dot_ignore`: a nested `.gitignore` and a `.ignore` each exclude one file.
- `walk_shares_cap_fairly_between_directories`:
  - Non-git fixture: `top.txt`, `big/f00..f49`, `small/{a,b,c}`, cap 10.
  - Asserts exactly `["top.txt", "big/f00".."big/f05", "small/a", "small/b", "small/c"]`, truncated.
- `walk_lists_everything_under_the_cap`: 3 dirs × 3 files, cap 100. All 9, not truncated.
- `walk_keeps_files_hidden_by_a_big_ignored_dir` (scaled repro):
  - Repo fixture with `.gitignore` = `/target`, `target/` holding 60 files, and `xtask/oracle.lua`.
  - With cap 50: `xtask/oracle.lua` is listed and no `target/` path appears.
  - Without `.gitignore`, also cap 50: `xtask/oracle.lua` is still listed (fairness), truncated.
- `walk_stops_at_the_ceiling`: ceiling 5 over 20 files gives at most 5 entries, truncated, and the same result on two runs.
- **Change** `walk_lists_relative_skips_hidden_and_caps`: the cap-2 expectation becomes `["a.rs", "sub/c.rs"]` (an intended behavior change per R4; call it out in the PR).

`crates/flux-vim/src/engine.rs` (next to `pickers_open_files_and_buffers`, `engine.rs:1410`):
- `files_picker_skips_gitignored_files`: `temp_tree` plus `.git/` and `.gitignore` = `sub/`. `:Files<CR>` shows `a.txt`, `b.txt` and nothing under `sub/`.
- The existing `pickers_open_files_and_buffers` must pass unchanged (3 entries, `a.txt` first).

`crates/flux-tui/src/draw/picker.rs` (next to `picker_window_shows_prompt_and_matches`):
- `picker_window_marks_a_truncated_list`: a truncated files picker shows `N+` right-aligned on the prompt row, in `Pmenu`. The existing test still shows no marker.

## 9. Files likely touched (guidance, not a mandate)
- `crates/flux-view/Cargo.toml` (`ignore = "0.4.33"`), `Cargo.lock`
- `crates/flux-view/src/explorer.rs` (walk, allocation, tests)
- `crates/flux-view/src/picker.rs` (truncated flag on `Picker`; `Picker::files` keeps its current signature or gains a sibling constructor)
- `crates/flux-vim/src/ex.rs:1444-1453` (`files_picker`: constant, pass the flag)
- `crates/flux-vim/src/engine.rs` (test)
- `crates/flux-tui/src/draw/picker.rs` (marker + test)
- `.sectorfive/decisions.md` (R14 entry, via Y'shtola), `.sectorfive/plans/m7.md` (§10 amendment)

## 10. Docs to update (via Y'shtola)
- `docs/milestones.md` M7 section: one line saying `:Files` skips gitignored files (inside git repos) and `.ignore` matches, and marks a truncated list with `N+`. Add a manual-check step (A1, A3).
- `.sectorfive/plans/m7.md`: amend D4/Success Criteria ("no new dependencies outside mlua") to record the approved `ignore` exception (cross-reference the R14 `decisions.md` entry).
- `.sectorfive/contracts.md`: no change, since there is no new public contract.

## 11. Risks
- **New dependency.** The M7 plan says "no new deps outside mlua" (`.sectorfive/plans/m7.md`, D4 and Success Criteria). The owner approved `ignore` as an explicit exception (R14). License risk is low: 9 crates, all permissive (ignore, globset, walkdir, same-file, winapi-util: Unlicense/MIT; bstr, crossbeam-deque/epoch/utils: MIT/Apache-2.0). `cargo deny check` passed in a scratch copy, with no duplicates added beyond the existing `syn` warning.
- **Behavior change in an existing test** (R4 replaces "files first"). This is intended; reviewers should expect the diff.
- **Ceiling plus depth-first collection** can still under-represent later siblings in huge non-git trees. It is bounded and marked truncated. Breadth-first or async walking is post-1.0.
- **Global excludes** make results machine-dependent in the editor. That is intended and matches git; tests disable it (R11).

## 12. Definition of done
- [ ] `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; `cargo deny check`: all pass locally, in AGENTS.md order.
- [ ] A1–A8 pass; R12 timings reported in the PR description.
- [ ] No edits to `expected.json`, the indent corpus or `colors.json`; no `cargo xtask * gen`.
- [ ] PR titled **Respect .gitignore in :Files and share its file cap fairly** (repo style: sentence-case imperative, no type prefix; squash-merged with `(#N)`).
- [ ] `.sectorfive/decisions.md` has the R14 dependency-exception entry, and `.sectorfive/plans/m7.md` is amended (§10).
- [ ] PR description has: what changed, the approved-dependency note with the license list, the intended test-expectation change, and a **Fina verify** block (commands run, results, manual A1/A3 notes, approve or veto). The implementer does not self-approve.
- [ ] After Fina approves: Y'shtola records `YYYY-MM-DD: F-01 approved by Fina (...)` in `.sectorfive/decisions.md`. This also unblocks F-06 (Stage B sign-off).

## Decisions (all settled, none open)
- `ignore 0.4.33` as a new dependency: **approved by the owner** on Oct 5, 2026 as an explicit exception to M7 D4 (R13, R14).
- Git rules apply only inside git repos (R2), and the marker copy is `N+` (R8; Tifa may polish the copy). Both are specified behavior, not open questions.
