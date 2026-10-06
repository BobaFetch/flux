# F-27 — Versioning policy and `CHANGELOG.md`

| | |
|---|---|
| Backlog | F-27 (M9 release hardening; PRD FR-D3). Feeds F-25 (release pipeline), F-26 (packages), F-28 (README), F-32 (0.9 dry run) |
| Size | S (≈ half a day) |
| Spec status | **Ready to implement.** All owner decisions settled Oct 6, 2026 (§ Decisions) |
| Repo state | `BobaFetch/flux` @ `d8ab89b` (main), read Oct 6, 2026 |
| Proposed branch / PR title | `changelog` / **Add a changelog and a versioning policy** |
| Sector 5 routing | Y'shtola (`CHANGELOG.md`, `docs/versioning.md`, ownership row) → Lightning (`.github/pull_request_template.md`, xtask consistency test) → Fina (verify). No Aerith/Tifa work |

Self-contained: everything needed is in this file and the repo.

## 1. Problem

flux has no record of user-visible changes and no stated rule for what a version number promises, but a public 1.0 needs both. Users need to know what changed, and packagers (F-25/F-26) need a version they can trust.

**Evidence:**
- `Cargo.toml:8` `[workspace.package] version = "0.0.1"`; every crate uses `version.workspace = true`. The six internal path dependencies repeat the number separately (`Cargo.toml:15–20`, `version = "0.0.1"` each), so a bump must touch seven places and nothing checks that they agree.
- `crates/flux/src/main.rs:35–37`: `-v/--version` prints `flux {CARGO_PKG_VERSION}`.
- No `CHANGELOG.md`, no `CONTRIBUTING.md`, no `.github/pull_request_template.md` (repo root and `.github/` listing). No git tags locally or on `origin` (`git ls-remote --tags origin` is empty).
- History lives only in milestone docs and commit titles: `docs/milestones.md` sections M0–M6 (✅ for M0–M5 and directory browsing), M7 in progress; commits are titled like `M6: LSP (#7)`, `Report a server's exit like Neovim: exit code, signal and the log (#8)`.
- `plans/flux-1.0-prd.md` FR-D3: "Semantic versioning starting at 1.0.0. `CHANGELOG.md` with every user-visible change." The owner decided on Oct 6, 2026 that Stable `flux.*` items are frozen under semver for 1.x (F-07 decision D2).
- `.sectorfive/ownership.md` has no row for a root `CHANGELOG.md` (it lists `docs/**, README.md, LICENSE-*` for Y'shtola).

## 2. Goal

The repo states what a flux version number promises and keeps a human-written changelog that every user-visible PR updates. A simple automated check keeps the version numbers and changelog headings consistent.

## 3. Non-goals

- Cutting a release, creating tags, or bumping the version (stays `0.0.1`; the first tag is the F-32 dry run).
- Release automation, binaries, Homebrew, crates.io metadata (F-25, F-26).
- Generating the changelog from commits (it's written by people; commit titles are input, not output).
- A CI gate that fails PRs without a changelog entry (decided against, D2).
- `CONTRIBUTING.md` (F-31; it will link to this policy).

## 4. Current vs required behavior

| Item | Current | Required |
|---|---|---|
| Changelog | none | `CHANGELOG.md` in Keep a Changelog 1.1.0 format with an `## [Unreleased]` section seeded from the milestones |
| Versioning policy | none | `docs/versioning.md`: SemVer 2.0.0, what's public, what counts as breaking, pre-1.0 rules, release checklist |
| Internal dependency versions | 7 copies, unchecked | a test fails if any differs from `[workspace.package] version` |
| Changelog headings | n/a | a test checks the heading format, order and the presence of `[Unreleased]` |
| PR checklist | none | `.github/pull_request_template.md` with a changelog line, the gates and the Fina verify block |
| `flux --version` | `flux 0.0.1` | unchanged format, now documented as stable (`flux X.Y.Z`) |

## 5. Detailed requirements

**R1. `CHANGELOG.md`** at the repo root:
- Header: title `# Changelog`, a line saying the format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/) and the project follows [Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html) as described in `docs/versioning.md`.
- Section `## [Unreleased]` with the standard subsections used as needed: `### Added`, `### Changed`, `### Deprecated`, `### Removed`, `### Fixed`, `### Security`.
- Seed `### Added` with one bullet per finished milestone (decision D3), each one line, user-facing wording, linking the matching `docs/milestones.md` heading: M0 viewing and scrolling; M1 core editing; M2 Visual mode, text objects, registers, macros, marks and jumps; M3 windows and buffers; M4 search and Ex; directory browsing; M5 syntax highlighting, indenting and filetypes; M6 language servers; M7 so far: system clipboard (`"+`/`"*`), `:Files`/`:Buffers` pickers, command-line Tab completion. Add a final bullet that behavior is checked against Neovim 0.12.5. Don't invent features: take each bullet from the milestone's own text.
- Version sections, when they exist later, are `## [X.Y.Z] - YYYY-MM-DD`, newest first, with compare links at the bottom (`[Unreleased]: https://github.com/BobaFetch/flux/compare/vX.Y.Z...HEAD`). None are added in this PR.

**R2. `docs/versioning.md`** (Y'shtola), with these sections:
1. **Scheme.** SemVer 2.0.0. Tags `vX.Y.Z` (annotated, on `main`). Until 1.0.0, `0.MINOR.PATCH` and a minor bump may break things; the planned path is a `0.9.0` dry run, then `1.0.0`.
2. **What the version covers (public surface)** (decision D1):
   - the `flux` command: its name, flags (`-h/--help`, `-v/--version`, and those F-07 adds: `--clean`, `-u`), and the `flux X.Y.Z` output of `--version`;
   - the config file location and loading rules (`~/.config/flux/init.lua`, `$XDG_CONFIG_HOME`), once F-07 lands;
   - the `flux.*` Lua API items documented as Stable, frozen for all of 1.x (Experimental items are excluded and may change in a minor release; F-07 decision D2);
   - option names and the values they accept, and flux's own defaults;
   - Ex command names and key bindings that flux implements.
3. **What it doesn't cover:** the Rust APIs of the workspace library crates (`flux-core`, `flux-view`, `flux-vim`, `flux-lsp`, `flux-syntax`, `flux-tui`, `flux-lua`), which are internal and move in lockstep with the binary; screen layout details beyond Vim parity; log file formats; test hooks such as `$FLUX_LSP_CONFIG`, `$FLUX_CLIPBOARD_FAKE` (`crates/flux/src/clipboard.rs:14`) and `$FLUX_INDENT`.
4. **Vim parity rule:** flux's behavior target is Neovim at the pinned version (`crates/flux-vim/tests/oracle/NVIM_VERSION`). A change that makes flux match that Neovim more closely is a **fix** (patch), even if someone relied on the old behavior; it is listed under `### Fixed`. Moving the pin to a newer Neovim is at least a **minor** release with a `### Changed` entry.
5. **What's breaking (major after 1.0):** removing or renaming anything in the public surface, changing its meaning, changing a default in a way that changes existing users' editing (except parity fixes), or moving the config path.
6. **Changelog rule:** every PR with a user-visible change adds a line under `## [Unreleased]` in the same PR, in user language, ending with the PR number once known (`(#12)`). PRs with no user-visible change say so in the PR template.
7. **Release checklist** (manual until F-25): bump `[workspace.package] version` and the six internal `version = "…"` entries in `[workspace.dependencies]`; run `cargo build` to refresh `Cargo.lock`; move `[Unreleased]` entries into `## [X.Y.Z] - YYYY-MM-DD` and add the compare link; open a PR titled `Release X.Y.Z`; after merge, tag `vX.Y.Z` on the merge commit. The checks in R3 must pass.

**R3. Consistency checks** (Lightning), as `#[cfg(test)]` tests in `xtask/src/main.rs` (or a new `xtask/src/version.rs` module), so they run in `cargo test --workspace` with no new dependency (no `toml` crate: it isn't in `Cargo.lock`; parse the few needed lines by hand from `include_str!("../../Cargo.toml")` and `include_str!("../../CHANGELOG.md")`):
- `internal_dependency_versions_match_workspace` — every `[workspace.dependencies]` entry with a `path = "crates/…"` has `version = "<the [workspace.package] version>"`. Failure message names the crate and both versions.
- `changelog_has_unreleased_section` — exactly one `## [Unreleased]` heading, and it's the first `## ` heading.
- `changelog_version_headings_are_valid` — every other `## ` heading matches `## [X.Y.Z] - YYYY-MM-DD` (X, Y, Z non-negative integers without leading zeros; a real calendar date), versions strictly decreasing top to bottom, no duplicates.
- `changelog_mentions_current_version_when_released` — if the workspace version is not `0.0.1` (that is, after the first release PR), a `## [<version>]` heading exists. With `0.0.1` the test passes trivially.

**R4. PR template** `.github/pull_request_template.md` (Lightning owns `.github/**`):
```markdown
## What and why

## Changelog
- [ ] Added a line under `## [Unreleased]` in CHANGELOG.md, or
- [ ] No user-visible change

## Checks (AGENTS.md order)
- [ ] cargo fmt --all --check
- [ ] cargo clippy --workspace --all-targets -- -D warnings
- [ ] cargo test --workspace
- [ ] cargo deny check

## Fina verify
<!-- approve or veto, with evidence -->
```
Wording may be polished; the four parts are required.

**R5. `--version` stays exactly `flux X.Y.Z`** (a space, no `v`, no build metadata), so packagers and scripts can parse it. Add a unit test in `crates/flux/src/main.rs` for the string (factor it into a function such as `fn version_line() -> String`).

**R6. Ownership:** add a row to `.sectorfive/ownership.md`: `CHANGELOG.md | Y'shtola | Agents add Unreleased lines in their PRs; Y'shtola edits at release`. Note in the row that every role may append Unreleased lines in its own PRs (otherwise R2.6 conflicts with "every path has exactly one owner").

**R7. This PR's own changelog line:** none needed beyond the seed (it adds no user-visible editor behavior). Its PR template box is "No user-visible change".

## 6. Edge cases

1. **Windows line endings in `CHANGELOG.md`:** the tests must accept `\r\n`.
2. **Headings inside fenced code blocks** in the changelog: ignore lines inside ``` fences when scanning.
3. **Pre-release versions** (`1.0.0-rc.1`): allowed by SemVer. The heading test accepts an optional `-<pre>` suffix; ordering uses SemVer precedence for the numeric part and treats a pre-release as lower than the same version without one. The version policy says whether rc tags are used (default: not planned).
4. **Workspace version changes but a crate pins `version` locally** (no `.workspace = true`): the first test only covers `[workspace.dependencies]`. Add a check that every `crates/*/Cargo.toml` and `xtask/Cargo.toml` uses `version.workspace = true` (true today).
5. **A PR merged without a changelog line:** caught in review via the template (D2), fixed in a follow-up PR.

## 7. Acceptance criteria

- **A1 gates:** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo deny check` pass.
- **A2 tests bite (temporary edits, revert before commit):**
  - change `flux-tui`'s `version = "0.0.1"` in `Cargo.toml` `[workspace.dependencies]` to `"0.0.2"` → `cargo test -p xtask` fails naming `flux-tui`, `0.0.2` and `0.0.1`;
  - rename `## [Unreleased]` to `## Unreleased` → `changelog_has_unreleased_section` fails;
  - add `## [0.1.0] - 2026-13-01` → `changelog_version_headings_are_valid` fails.
- **A3 content:** `CHANGELOG.md` renders on GitHub with the header links, `[Unreleased]` and the seeded bullets, each linking a `docs/milestones.md` heading that exists. `docs/versioning.md` has the seven sections in R2.
- **A4 template:** opening a test PR (or GitHub's preview of the file) shows the template.
- **A5 version line:** `cargo run -- --version` prints `flux 0.0.1`.

## 8. Tests to add

- `xtask`: the four tests in R3, plus `all_crates_use_workspace_version` (edge case 4) and a CRLF variant of the heading test (edge case 1).
- `crates/flux/src/main.rs`: `version_line_is_flux_space_semver` (R5).

## 9. Files likely touched (guidance)

- New `CHANGELOG.md`, `docs/versioning.md`, `.github/pull_request_template.md`.
- `xtask/src/main.rs` (or new `xtask/src/version.rs`), `crates/flux/src/main.rs` (tiny refactor for R5).
- `.sectorfive/ownership.md` (R6).

## 10. Docs to update (via Y'shtola)

- `README.md`: one line linking `CHANGELOG.md` and `docs/versioning.md` (the full README rewrite is F-28).
- `AGENTS.md` Commands: "User-visible changes add a line under `## [Unreleased]` in `CHANGELOG.md` (see `docs/versioning.md`)."
- `.sectorfive/decisions.md`: `YYYY-MM-DD: Versioning policy adopted (SemVer; public surface per docs/versioning.md; parity fixes are patches).`

## 11. Risks

- **Changelog drift** without a CI gate. Mitigation: template checkbox, PM review of every PR (the owner's workflow), and a release-PR pass.
- **Over-promising** if the public surface is defined too broadly (e.g. every default). Mitigation: D1 and the parity rule keep Vim-compatibility fixes out of "breaking".
- **Hand parsing `Cargo.toml`** is brittle if its layout changes. Mitigation: the tests fail loudly with a clear message, and the format is stable (`name = { path = …, version = "…" }` on one line).

## 12. Definition of done

- [ ] A1–A5 pass.
- [ ] `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; `cargo deny check`: pass, in `AGENTS.md` order.
- [ ] Version still `0.0.1`; no tags created.
- [ ] PR titled **Add a changelog and a versioning policy** (sentence-case imperative, squash-merged with `(#N)`), using its own new template.
- [ ] **Fina verify** block in the PR (A2 outputs).
- [ ] After approval, Y'shtola records the decision (§ 10) and the ownership row.

## Decisions (all settled, none open)

Settled by the owner on Oct 6, 2026. The requirements above implement them.

- **D1. The version covers** the CLI (command, flags, `--version` output), the config path and loading rules, Stable `flux.*` items, option names, accepted values and defaults, and Ex commands and keys. Matching the pinned Neovim more closely is a fix, not a breaking change. The workspace Rust crates are internal and not covered. Implemented by R2.2–R2.5.
- **D2. No CI gate for changelog entries:** a PR-template checkbox plus review. Implemented by R4 and the § 3 non-goal.
- **D3. `[Unreleased]` is seeded with one line per finished milestone.** Implemented by R1.
