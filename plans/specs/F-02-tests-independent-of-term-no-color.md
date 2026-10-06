# F-02 — Tests pass regardless of `TERM` / `NO_COLOR`

| | |
|---|---|
| Backlog | F-02 (M7 finish / quality gates) |
| Size | S (≈ half a day) |
| Spec status | **Ready to implement.** Decided by the owner on Oct 5, 2026: flux ignores `NO_COLOR` (like Neovim) |
| Repo state | `BobaFetch/flux` @ `27f9a2a` (main), read Oct 5, 2026 |
| Proposed branch / PR title | `tests-no-color` / **Keep colors and tests independent of NO_COLOR and TERM** |
| Sector 5 routing | Aerith (`crates/flux-tui/**`, `crates/flux/src/main.rs`) → Lightning (`.github/workflows/ci.yml`) → Fina (verify). Docs via Y'shtola. Tifa not needed (no copy) |

## 1. Problem

`cargo test --workspace` fails on a machine whose environment has `NO_COLOR` set.

**Evidence (run Oct 5, 2026 at `27f9a2a`, Rust 1.98.0, with `--no-fail-fast` so every crate ran):**

| Environment | Result |
|---|---|
| `NO_COLOR=1 TERM=dumb` | 1 failure: `flux-tui` `renderer::tests::default_colors_fill_in_and_force_a_redraw`, panic at `crates/flux-tui/src/renderer.rs:205` (`out.contains("48;2;1;2;3")`). Every other suite passes |
| `NO_COLOR=1 TERM=xterm-256color` | Same single failure |
| `TERM=dumb`, `TERM=` (empty), `TERM` unset, `TERM=xterm-256color COLORTERM=truecolor` (all without `NO_COLOR`) | All pass |

So **`NO_COLOR` is the only variable that matters today; `TERM` has no effect on any test.**

**Root cause:**
- The renderer emits colors through crossterm's `SetForegroundColor` / `SetBackgroundColor` (`renderer.rs:111-117`).
- crossterm 0.29.0 checks `NO_COLOR` once per process (`Colored::ansi_color_disabled_memoized`, `crossterm-0.29.0/src/style/types/colored.rs:75-101`). When it is non-empty, every color command is written as an empty SGR (`ESC[m`).
- flux never mentions `NO_COLOR` itself: `rg NO_COLOR crates xtask` finds nothing.
- crossterm offers `crossterm::style::force_color_output(bool)` to override this (`crossterm-0.29.0/src/style.rs:183-193`).

**User-visible side effect (verified in tmux, Oct 5):**
- With `NO_COLOR=1`, flux draws no colors at all.
- After `ve` in Visual mode the selected text is **not highlighted**, the statusline is indistinguishable from text, and `~` filler lines and mode messages are unstyled.
- Without `NO_COLOR`, the same session shows the selection (`48;2;79;82;88`) and the statusline colors.
- Neovim does not appear to honor `NO_COLOR`: a GitHub code search of `neovim/neovim` finds it only in a compiler plugin and `xxd`, not in the TUI. This was not confirmed by running Neovim.

## 2. Goal

The test suite gives the same result in any terminal environment, and flux's color output is an explicit product decision rather than a side effect of a dependency.

## 3. Non-goals

- Designing a real monochrome or `NO_COLOR` mode (attributes-only highlighting). That is post-1.0 if wanted.
- macOS CI (F-04), oracle CI on every PR (F-04), and Mac oracle re-recording (F-23).
- Changing how `'termguicolors'` is detected from `COLORTERM` (`crates/flux/src/main.rs:46-48`).
- Changing `cargo xtask screens` (but see Risks).

## 4. Current vs required behavior

| | Current | Required (decided: flux ignores `NO_COLOR`) |
|---|---|---|
| `cargo test --workspace` with `NO_COLOR=1` | 1 failure | Pass |
| Any `TERM` value | Pass | Pass, guarded in CI |
| flux run with `NO_COLOR=1` | No colors; Visual selection invisible | Full colors, like Neovim |

## 5. Detailed requirements
- **R1. Renderer output does not depend on `NO_COLOR`.** For a given grid and renderer state, `Renderer::draw` (`crates/flux-tui/src/renderer.rs`) writes the same bytes whatever the process environment. No test may need to set or unset environment variables to pass. Rust 2024 makes `std::env::set_var` `unsafe`, and the workspace forbids `unsafe` (`Cargo.toml` `[workspace.lints.rust]`).
- **R2. One explicit decision point.**
  - The crossterm color override is set in exactly one place, and that place is reached both by the binary and by every flux-tui unit test that inspects output bytes, without each test having to remember.
  - Recommended: the `Renderer` constructor (`Renderer::default`/`new`) calls `crossterm::style::force_color_output(true)`, with a comment citing this spec and crossterm's NO_COLOR handling. It is an idempotent atomic store.
  - Acceptable alternative: a single `force_color_output(true)` call in `crates/flux/src/main.rs` before the first frame, plus a shared `#[cfg(test)]` helper in flux-tui, as long as every byte-inspecting test uses it.
- **R3.** `flux` renders colors with `NO_COLOR` set, exactly as without it. Byte-for-byte, the same output for the same screen.
- **R4.** flux code does not read `NO_COLOR` anywhere; the only reference is the comment at the R2 override explaining why it is ignored.
- **R5. CI guard.**
  - `.github/workflows/ci.yml` gets a second test step after `Test`: `name: Test (NO_COLOR, dumb terminal)`, `env: { NO_COLOR: "1", TERM: dumb }`, `run: cargo test --workspace`.
  - It reuses the build: `RUSTFLAGS` is unchanged and env vars don't affect compilation, so the expected added time is test runtime only (about 10–15 s locally).
  - Keep the existing `Test` step unchanged.
- **R6.** No other behavior changes. `COLORTERM` handling, 16-color vs truecolor output, and every other renderer test stay the same.
- **R7.** Scan for other environment reads that tests could hit (current list: `crates/flux/src/main.rs:48,53`, `crates/flux/src/servers.rs:23-25`, `crates/flux/src/clipboard.rs:58`, `crates/flux-lsp/src/config.rs:200`, `crates/flux-view/src/buffer.rs:443`, `crates/flux-vim/src/complete.rs:58`, `crates/flux-vim/tests/indent.rs:80`). Either confirm in the PR that none makes a test environment-dependent, or fix it under the same rule as R1. (All passed under the combinations in §1; confirm, don't assume.)

## 6. Edge cases
1. `NO_COLOR=""` (empty) or any other value: irrelevant, flux always draws colors.
2. Tests run in parallel threads within one process. The override is process-global, so any test that sets it must only ever set `true`. No test may set `false`.
3. `TERM=dumb`: flux has no dumb-terminal mode today, and this spec does not add one. Tests just have to pass.
4. `CI` or other common CI variables: no effect.
5. Under A, a user who exports `NO_COLOR` for CLI tools gets a colored flux. That is intended, matches Neovim, and is documented (§10).

## 7. Acceptance criteria
| # | Command | Expected |
|---|---|---|
| A1 (repro) | `env NO_COLOR=1 TERM=dumb cargo test --workspace --no-fail-fast` | Before: 1 failure (`default_colors_fill_in_and_force_a_redraw`). After: 0 failures |
| A2 | `env -u NO_COLOR -u COLORTERM TERM=dumb cargo test --workspace`; also with `TERM=` and with `TERM` unset | 0 failures each |
| A3 | `env NO_COLOR=1 TERM=xterm-256color COLORTERM=truecolor cargo test -p flux-tui` | 0 failures |
| A4 (manual) | `printf 'hello world\nsecond line\n' >/tmp/nc.txt`, then in tmux: `env NO_COLOR=1 COLORTERM=truecolor FLUX_LSP_CONFIG=<file containing []> cargo run -- /tmp/nc.txt`, type `ve`, and run `tmux capture-pane -p -e` | The selection shows a background SGR (`48;2;79;82;88` with the default dark scheme) and the statusline is colored, the same as without `NO_COLOR` |
| A5 | CI on the PR | Both `Test` and `Test (NO_COLOR, dumb terminal)` are green |

## 8. Tests to add or change
- `crates/flux-tui/src/renderer.rs` `mod tests`:
  - Keep `default_colors_fill_in_and_force_a_redraw` as is. It is the regression test, and must now pass under `NO_COLOR=1` without any test-local setup.
  - Add `colors_are_written_whatever_the_environment`: draw a grid with an `Rgb` fg/bg and an `Ansi(1)` fg, and assert that the `38;2;…`, `48;2;…` and `38;5;1` SGR fragments are present. crossterm writes ANSI colors as `38;5;n` (`colored.rs:131-149`); the mapping is in `renderer.rs:135-164`. It documents R1, and the CI guard (R5) runs it under `NO_COLOR`.
- There is no in-process way to set `NO_COLOR` (R1), so the CI step in R5 is the environment test. Do not add `unsafe` or `#[allow(unsafe_code)]`.

## 9. Files likely touched (guidance)
- `crates/flux-tui/src/renderer.rs` (constructor override + test)
- `crates/flux/src/main.rs` (only if the R2 alternative is used)
- `.github/workflows/ci.yml` (R5 step; update the header comment if it still talks about billed minutes. That billing rationale for private repos no longer applies now that the repo is public, but rewording it belongs to F-04)

## 10. Docs to update (via Y'shtola)
- `AGENTS.md` "Commands": add "Tests must not depend on `TERM`, `NO_COLOR` or `COLORTERM`; CI also runs `cargo test --workspace` with `NO_COLOR=1 TERM=dumb`." (AGENTS.md has no owner in `.sectorfive/ownership.md`; route it via Y'shtola.)
- `docs/milestones.md` (M7 section or a "Terminal" note) and README:
  - "flux ignores `NO_COLOR`, like Neovim. Its screen relies on color for information such as the Visual selection and the statusline, and it has no monochrome mode."
- `.sectorfive/baseline.md`: Fina records the new green result for both environments.

## 11. Risks
- **Global override.** `force_color_output` is process-wide. Calling it from the renderer constructor is a side effect in a constructor; it is acceptable because it is idempotent and documents the product rule. The alternative is a single startup call plus a test helper (R2).
- **Product change.** `NO_COLOR` users get colors. This is a deliberate move to Neovim parity, approved by the owner on Oct 5, 2026.
- **`cargo xtask screens` under `NO_COLOR`** probably mismatches today, because flux would draw no colors while Neovim does. This was not run (the box has no Neovim). This change fixes it as a side effect.
- **Rollback:** revert the PR. No data or format changes.

## 12. Definition of done
- [ ] `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; `cargo deny check`: pass, in AGENTS.md order, plus A1–A3.
- [ ] No new dependencies. No `unsafe`. No `gen` re-records.
- [ ] PR title per the header (sentence-case imperative, squash-merged with `(#N)`).
- [ ] PR description: the decided `NO_COLOR` behavior, the R7 environment-read audit, A1 before/after output, A4 capture, and a **Fina verify** block (approve or veto with evidence). The implementer does not self-approve.
- [ ] After approval, Y'shtola records `YYYY-MM-DD: F-02 approved by Fina (...)` in `.sectorfive/decisions.md`, and Fina updates `.sectorfive/baseline.md`.

## Decisions (all settled, none open)
- **`NO_COLOR`: decided by the owner on Oct 5, 2026.** flux ignores it, like Neovim. This fixes the invisible Visual selection and makes output deterministic.
