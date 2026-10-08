# F-04 — CI on every PR, macOS included

| | |
|---|---|
| Backlog | F-04 (M9 release hardening; PRD FR-Q1, FR-Q2, FR-Q6) |
| Size | S (≈ half a day, plus fixing whatever macOS surfaces) |
| Spec status | 📄 Written Oct 7, 2026 |
| Repo state | `BobaFetch/flux` @ `48f5a44` (main), read Oct 7, 2026 |
| Proposed branch / PR title | `ci-every-pr-macos` / **Run CI on macOS and the Neovim checks on every PR** |
| Owner | Lightning (`.github/**`, toolchain files per `.sectorfive/ownership.md`) |
| Sector 5 routing | Lightning builds → Fina verifies (Muse Spark 1.3, high). A macOS fix outside `.github/**` goes to that path's owner (Yuna for `crates/flux/src/{main,terminal,clipboard}.rs` and `flux-tui`; Fina for `crates/*/tests/**`). Y'shtola updates `docs/milestones.md` M0 and the backlog row when it ships |

## 1. Problem

- `.github/workflows/ci.yml` runs one job, `check`, on `ubuntu-latest` only. Its header says macOS "is covered locally". That choice was made to save billed minutes on a private repo. The repo is now public, so standard GitHub-hosted runners, macOS included, cost nothing (PRD FR-Q1).
- `docs/milestones.md` M0 claims "tests on Linux and macOS". CI does not do that (`plans/repo-review.md:74`).
- `.github/workflows/oracle.yml` (oracle, indent, colors checks against Neovim 0.12.5) runs only when fixture paths change. M7 never triggered it (`plans/repo-review.md:110`). PRD FR-Q2 wants it on every PR.
- The Rust version is written in three places: `rust-toolchain.toml` (`1.98.0`), `ci.yml` and `oracle.yml` (`dtolnay/rust-toolchain@1.98.0`). They can drift.
- `.sectorfive/bin/role` (POSIX sh) has no lint in CI (Fina follow-up from PR #14 and #16).

Baseline timings (GitHub Actions, warm cache, Oct 6 to 7): `CI` takes about 2 to 3.5 min wall-clock. `Neovim oracle` takes about 1.2 min. The full workspace test suite already passes on macOS arm64 locally (yonaka, Oct 7: 223 tests, including the `NO_COLOR=1 TERM=dumb` run).

## 2. Goal

Every PR and every push to `main` gets the same release-blocking signal (FR-Q6, minus install smoke tests, which belong to F-25/F-32) on Linux and macOS. The run stays fast enough that nobody is tempted to skip it.

## 3. Non-goals

- Release builds, cross-compiled targets, macOS x86_64 / Linux arm64 runners, install smoke tests (F-25, F-26, F-32).
- Performance budgets in CI (F-24).
- Running the oracle/indent/colors checks on macOS. Recording is broken on macOS (F-23). The checks compare against Linux Neovim and stay Linux-only.
- `cargo xtask screens` in CI (needs tmux plus Neovim, compares screens; separate decision).
- Branch protection / required-status settings in GitHub (repo settings, not code; see Open questions).
- Windows (P-05).

## 4. Current vs required behavior

| | Current | Required |
|---|---|---|
| Triggers | `push` to `main`, `pull_request` | Same |
| OS | `ubuntu-latest` | `ubuntu-latest` and `macos-latest` (Apple Silicon) |
| Gates per OS | Linux: fmt, clippy, test, test with `NO_COLOR=1 TERM=dumb`, deny | Both OSes: test, test with `NO_COLOR=1 TERM=dumb`, clippy. Linux only: fmt, deny, shellcheck |
| Oracle / indent / colors | Only when fixture paths change | Every PR and push to `main` (Linux) |
| Toolchain source | Hard-coded `1.98.0` in two workflows plus `rust-toolchain.toml` | `rust-toolchain.toml` is the single source |
| Concurrency | `cancel-in-progress: true` per ref | Kept, for every workflow |
| Cache | `Swatinem/rust-cache@v2` | Kept, per OS |

## 5. Detailed requirements

- **R1. Triggers.** CI runs on `pull_request` (any branch) and on `push` to `main`. Keep `workflow_dispatch` on the Neovim workflow.
- **R2. OS matrix.** The test job runs on `[ubuntu-latest, macos-latest]` with `fail-fast: false`, so one OS failing still reports the other. `macos-latest` must resolve to an Apple Silicon (arm64) image. Print `uname -m` and `sw_vers` in a step so the log shows it.
- **R3. Gates.**
  - On both OSes: `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, and `cargo test --workspace` with `NO_COLOR=1` and `TERM=dumb`.
  - On Linux only:
    - `cargo fmt --all --check`, because formatting doesn't depend on the OS.
    - `cargo deny check`. `cargo-deny-action` is a Docker action that runs only on Linux, and `deny.toml` checks all targets (`[graph] all-features = true`, no target filter), so one run covers macOS-only dependencies too.
  - Every gate in today's `ci.yml` stays. None is removed, moved to `continue-on-error`, or made conditional except by OS as listed here.
- **R4. Shellcheck.** One Linux step runs `shellcheck -s sh .sectorfive/bin/role`. ShellCheck is preinstalled on `ubuntu-latest`; if it isn't, install it with `apt-get`. The step must fail CI on findings.
- **R5. Neovim checks on every PR.** Remove the `paths:` filters from `oracle.yml`, or fold its job into `ci.yml` as a separate Linux job. Either is fine as long as it runs on every PR and push to `main`, keeps `NVIM_VERSION` pinned to `v0.12.5` (the comment linking `crates/flux-vim/tests/oracle/NVIM_VERSION` stays), and still runs `oracle check`, `indent check` and `colors check`.
- **R6. One toolchain source.** Both workflows install the toolchain from `rust-toolchain.toml` (channel `1.98.0`, components `rustfmt`, `clippy`). Use an action or step that honors the file (e.g. `actions-rust-lang/setup-rust-toolchain@v1`, or `rustup show` after checkout). No workflow hard-codes a Rust version. A CI log line shows `rustc 1.98.0` on each OS.
- **R7. Caching.** `Swatinem/rust-cache@v2` on every job that builds, keyed per OS and job (the action's defaults do this). The cache must not hide failures: no cached test results, only build artifacts.
- **R8. Concurrency.** Every workflow keeps `concurrency: { group: <workflow>-${{ github.ref }}, cancel-in-progress: true }`.
- **R9. Time budget.** With a warm cache, a PR's slowest job finishes in **≤ 10 min**, with an expected Linux time of about 3 min and macOS about 4 to 6 min. Jobs run in parallel. If macOS goes over budget, report the per-step timings in the PR before changing anything. Don't drop a gate to fit the budget.
- **R10. Header comment.** Rewrite the `ci.yml` header. Remove the private-repo billing rationale and say what runs where. One sentence notes that standard macOS runners are free for public repos.
- **R11. Status names.** Use stable, readable job names (e.g. `test (ubuntu-latest)`, `test (macos-latest)`, `lint`, `neovim`). List the final check names in the PR body so the PM and the user can update any required-status setting.

## 6. macOS failures (policy)

If a test fails only on macOS:

1. **Fix it** when the cause is test hygiene. That fix goes in this PR or a linked PR from the owning role. Likely causes:
   - `std::env::temp_dir()` is a `/var/folders/…` path behind the `/private` symlink, so a raw path comparison against a canonicalized path fails.
   - Case-insensitive APFS.
   - Different `PATH` tools (`pbpaste`/`pbcopy` exist; `xclip`/`wl-paste` don't).
   - No TTY on the runner.
   - BSD vs GNU command flags in test helpers.
   - Timing on a slower runner (as with F-34).
2. **Otherwise, a tracked skip, which needs the user's decision.** If a failure exposes a real macOS product bug, or needs more than a small fix:
   - Open a backlog item (`F-NN`) with the failing test, the log link and the root cause.
   - Skip only that test, only on macOS (`#[cfg_attr(target_os = "macos", ignore = "F-NN: <reason>")]`).
   - Say so in the PR body under a **macOS skips** heading.
   - The user approves each skip before merge. Fina vetoes any skip that lacks a backlog item, a reason string or user approval.
3. **Never weaken silently.** Don't remove assertions, don't loosen expected values, don't use `continue-on-error`, don't `|| true`, don't make whole jobs non-blocking (`allow-failure`), and don't filter tests by name in the workflow. Rule 2 is the only allowed route. Under FR-Q5 no skip is silent; the `ignore` reason is printed by `cargo test`.

## 7. Acceptance checks

- **A1.** On the PR, GitHub shows passing checks for `test (ubuntu-latest)`, `test (macos-latest)`, the Linux lint/fmt/deny/shellcheck job(s), and the Neovim job, all on the PR head.
- **A2.** The macOS job log shows `arm64`, a macOS version, and `rustc 1.98.0`. The Linux log shows `rustc 1.98.0`.
- **A3.** `grep -rn '1\.98' .github/` finds no hard-coded Rust version (the Neovim `v0.12.5` pin is expected).
- **A4.** The diff keeps every command from today's `ci.yml` (fmt, clippy `-D warnings`, test, `NO_COLOR=1 TERM=dumb` test, cargo-deny), with no `continue-on-error`, `|| true` or job-level `if:` that could skip a gate on PRs.
- **A5.** Shellcheck fails as intended. In a scratch commit on the PR branch, deliberately break `.sectorfive/bin/role` (e.g. an unquoted `$1`), confirm the shellcheck step goes red, then revert the commit. Link the red run in the PR body.
- **A6.** The Neovim job runs on a PR that touches no fixture path (the F-04 PR itself counts, as long as the run shows `oracle check`, `indent check` and `colors check` passing).
- **A7.** Concurrency works. A second push to the PR branch cancels the in-progress runs (link the cancelled run).
- **A8.** Time. The PR body lists wall-clock times for each job on a warm-cache rerun, and the slowest is ≤ 10 min.
- **A9.** Any macOS skip follows §6.2 exactly, or there are none.

## 8. Files likely touched (guidance, not a mandate)

- `.github/workflows/ci.yml` (matrix, toolchain source, shellcheck, header).
- `.github/workflows/oracle.yml` (triggers, toolchain source), or removed if folded into `ci.yml`.
- Test-hygiene fixes surfaced by macOS, owned by whoever owns the path.

## 9. Docs to update (via Y'shtola)

- `docs/milestones.md` M0: "CI (fmt, clippy, tests on Linux and macOS…)" becomes true again; adjust wording if needed.
- Backlog row F-04 to 🚢 when merged.

## 10. Risks

- **Check names change.** `check` becomes per-OS names. Any branch-protection rule or script that waits on `check` must be updated (R11).
- **`macos-latest` moves.** GitHub can point it at a new macOS image without notice. The A2 log line makes a change visible. Pinning (e.g. `macos-15`) is an open question.
- **macOS minutes on a public repo** are free for standard runners. A larger runner is not, so don't use one.
- **Hidden macOS bugs.** The macOS suite passes locally, but runners differ: no TTY, a clean HOME, no Homebrew tools beyond the image. §6 decides what happens.

## Open questions (user decision)

1. **Required checks:** after merge, should the new checks (both OS test jobs, lint, Neovim) be required status checks on `main`? That's a repo setting the user changes, not part of this PR.
2. **macOS runner label:** `macos-latest` (floating, as asked) or a pinned `macos-15` (predictable)? The spec defaults to `macos-latest`.
3. **macOS x86_64:** the PRD targets macOS x86_64 at 1.0. Is testing on arm64 enough for CI, with x86_64 covered only by F-25 release builds? The spec assumes yes.
