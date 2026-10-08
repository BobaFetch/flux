# F-34 — Quickfix unit tests stop racing over a shared temp folder

| | |
|---|---|
| Backlog | F-34 (quality gates / CI health; test-only) |
| Size | XS (≈ an hour) |
| Spec status | **Shipped** in #15 (`2ca04e4`), Oct 7, 2026. Written Oct 7, 2026 |
| Repo state | `BobaFetch/flux` @ `1a4ad7d` (main), read Oct 7, 2026 |
| Proposed branch / PR title | `fix-quickfix-test-race` / **Give each quickfix test its own temp folder** |
| Sector 5 routing | Lightning (`crates/flux-vim/**` except `tests/`; the change is in the inline `#[cfg(test)]` module of `crates/flux-vim/src/quickfix.rs`) → Fina (verify). No drawing, copy or docs work. Y'shtola updates the backlog row when it ships |

## 1. Problem

CI on `main` is red, intermittently, because of a flaky unit test.

**Evidence (GitHub Actions, workflow CI, job `check`, step Test):**

| Run | Commit | Result |
|---|---|---|
| 37725266854 (Oct 7, 8:58 PM PT) | `97e6ba5` | `quickfix::tests::going_through_the_list` FAILED at `crates/flux-vim/src/quickfix.rs:173:9`: `left: (0, 0)`, `right: (1, 2)` |
| 37727474954 (Oct 7, 9:26 PM PT) | `1a4ad7d` | Same test, same line, same values |
| 37495762091 (Oct 6) | `51285be` | Pass (no code change to `quickfix.rs` since) |

In both failures, the sibling test `quickfix::tests::the_quickfix_window` passed in the same binary.

**Root cause:**
- Both tests build their editor with the shared helper `editor_with_list()` (`quickfix.rs:133-158`).
- The helper uses one folder per *process*: `std::env::temp_dir().join(format!("flux-qf-{}", std::process::id()))` (`quickfix.rs:134`). Rust runs unit tests on parallel threads in one process, so both tests get the **same** folder.
- Each call rewrites `a.txt` and `b.txt` with `std::fs::write`, which truncates the file before writing. If one test truncates `a.txt` while the other is reading it in `editor.open` / the `:cc` jump, that test loads an empty or partial buffer. The jump to line 2, column 2 then clamps to `(0, 0)`, which is exactly the failure above.
- Nothing ever removes the folder, so stale `flux-qf-<pid>` folders also pile up in the temp dir.

The repo already has the right pattern in several places: a per-call counter (`crates/flux-vim/src/ex.rs` `temp_file`, `crates/flux-vim/src/lsp/mod.rs` `setup`, `crates/flux-view/src/buffer.rs` `temp_path`) or a per-test name (`crates/flux-vim/src/engine.rs` `temp_tree(name)`).

## 2. Goal

The quickfix unit tests give the same result every run, however the test threads are scheduled, with no change to what they check.

## 3. Non-goals

- Any change to quickfix behavior or to product code outside the `#[cfg(test)]` module.
- New dependencies (no `tempfile` crate; std is enough and `deny.toml` stays untouched).
- Changing other tests' temp-folder helpers. The other pid-only folders (`flux-view/src/explorer.rs` `flux-walk-test`, `flux-lsp/src/config.rs` `flux-lsp-root`, `flux/src/clipboard.rs` `flux-clipboard-test`) are each used by a single test, so they cannot race within a process today. Fina may list them as non-blocking hardening notes.
- Retrying, serializing (`--test-threads=1`, mutexes) or `#[ignore]`-ing tests. Those hide the bug instead of removing the shared state.

## 4. Current vs required behavior

| | Current | Required |
|---|---|---|
| Temp folder per `editor_with_list()` call | One per process (`flux-qf-<pid>`), shared by both tests | Unique per call (and so per test) |
| Shared mutable files between tests | Yes (`a.txt`, `b.txt` rewritten concurrently) | None |
| `cargo test -p flux-vim --lib quickfix` repeated | Intermittent `(0, 0)` vs `(1, 2)` failure | Passes every time |
| Assertions in `going_through_the_list` / `the_quickfix_window` | As at `1a4ad7d` | Identical (none removed, loosened or reordered) |

## 5. Detailed requirements

- **R1. Unique folder per call.** `editor_with_list()` creates a folder that no other call in the same process (or a concurrent test process) can get. Use either a `static AtomicUsize` counter combined with `std::process::id()` (matching `ex.rs` `temp_file`), or a test-name parameter combined with the pid (matching `engine.rs` `temp_tree(name)`). A counter is preferred because it cannot collide if a future test forgets to pass a distinct name.
- **R2. Fresh folder contents.** The helper starts from an empty folder (e.g. `let _ = std::fs::remove_dir_all(&dir);` before `create_dir_all`), so a leftover folder from an earlier run with a recycled pid cannot affect the result.
- **R3. No shared state between tests.** No two tests in `quickfix.rs` read or write the same path. No `static` mutable data is shared other than the counter.
- **R4. Assertions unchanged.** Every `assert_eq!` / `assert!` in the quickfix test module at `1a4ad7d` is kept with the same expected values. Test names stay the same. No test is removed, ignored or made conditional.
- **R5. Test-only, Lightning's surface.** The diff touches only the `#[cfg(test)] mod tests` block of `crates/flux-vim/src/quickfix.rs`. No product code, no `crates/*/tests/**` (Fina's), no `Cargo.*`, no CI files.
- **R6. Optional cleanup.** Removing the folder at the end of each test is welcome but not required. If added, it must not be able to mask an assertion failure (e.g. a `Drop` guard is fine; cleanup that runs only on success is fine).

## 6. Acceptance checks

Run in a worktree at the PR head.

- **A1. Race gone.** `for i in $(seq 1 50); do cargo test -q -p flux-vim --lib quickfix || break; done` completes all 50 iterations with no failure. Report the count.
- **A2. Race reproducible before (evidence, best effort).** Running the same loop at `1a4ad7d` (or with the old helper temporarily restored in a scratch copy, never committed) shows at least one `(0, 0)` vs `(1, 2)` failure, or the report says it did not reproduce locally in N runs and cites the two CI failures above.
- **A3. Unique paths.** Code reading shows two calls to `editor_with_list()` in one process get different folders (cite the lines).
- **A4. Assertions intact.** `git diff origin/main -- crates/flux-vim/src/quickfix.rs` shows no removed or changed `assert` lines and no change outside the test module.
- **A5. Gates.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `env NO_COLOR=1 TERM=dumb cargo test --workspace`, `cargo deny check` all exit 0.
- **A6. CI.** The PR's GitHub CI `check` job is green.

## 7. Risks

- **Low.** The change is confined to a test helper. The only realistic mistake is a still-shared path (e.g. using only the test name without the pid, so two concurrent `cargo test` processes collide); R1 rules that out.
