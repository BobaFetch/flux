# F-13 — Graceful LSP startup: a broken auto-started server never blocks

| | |
|---|---|
| Backlog | F-13 (M8 "1.0 gaps"; PRD FR-1 and success metric S4 "clean first run"). Prerequisite for F-32 (0.9 dry-run) |
| Size | S (≈ 1 day agent time) |
| Spec status | **Ready to implement.** All owner decisions settled Oct 6, 2026 (§ Decisions) |
| Repo state | `BobaFetch/flux` @ `d8ab89b` (main), read Oct 6, 2026 |
| Proposed branch / PR title | `lsp-quiet-autostart` / **Don't block startup when an auto-started language server fails** |
| Sector 5 routing | Lightning (`flux-view::lsp` state, `flux-vim::lsp::handle_exit`, `ex_lsp.rs`, unit tests) → Aerith (`main.rs` marks auto-enabled configs) → Tifa (warning copy) → Fina (verify). Docs via Y'shtola |

Self-contained: everything needed is in this file and the repo.

## 1. Problem

flux starts every built-in language server whose executable is on `PATH`, without the user asking. When that executable exists but can't run (the common case is a rustup proxy for a component that isn't installed), the first Rust file opened shows a two-line error and a **hit-enter prompt**, and every further Rust file opened shows it again.

**Evidence:**
- `crates/flux/src/main.rs:49–68`: with no `$FLUX_LSP_CONFIG`, `editor.lsp.configs = flux_lsp::builtin_configs()` and `editor.lsp.enabled` = every config whose `cmd[0]` passes `flux_lsp::config::executable`. The comment says these are "in Neovim, the configs given to `vim.lsp.enable`", but in Neovim nothing is enabled unless the user enables it.
- `crates/flux-lsp/src/config.rs:196–202` `executable()` only checks that a file named `cmd[0]` exists in a `PATH` directory. A rustup proxy passes.
- When the process exits, `crates/flux/src/servers.rs:86–95` appends `. Check log for errors: <log>` and calls `flux_vim::lsp::handle_exit` (`crates/flux-vim/src/lsp/mod.rs:73–84`), which shows `editor.error(format!("Client {name} quit {why}"))` unless the client was `Stopping`. A spawn failure takes the same path with `with error: …` (`servers.rs:58–60`).
- `Editor::show` (`crates/flux-view/src/editor.rs:960–967`) sets `hit_enter` for any error at least as wide as the screen. The message is about 110 columns, so on an 80-column terminal it wraps and prompts.
- `Editor::lsp_attach` (`crates/flux-view/src/lsp.rs:575–608`) reuses a client only when it isn't `Exited` (`:600`), so each new buffer of that filetype starts the server again, and it fails again.
- There is no test of `handle_exit` (`rg 'handle_exit\(' crates` finds only the two calls in `servers.rs`).

**Reproduced by the PM on a Linux x86_64 test machine (Oct 6, 2026, tmux 80×24, `TERM=xterm-256color`):** `rust-analyzer` on `PATH` is the rustup proxy; `rust-analyzer --version` prints `error: Unknown binary 'rust-analyzer' in official toolchain '1.98.0-x86_64-unknown-linux-gnu'.`
- `flux crates/flux/src/servers.rs` shows:
  ```
  Client rust_analyzer quit with exit code 1 and signal 0. Check log for errors: /
  home/<user>/.local/state/flux/lsp.log
  Press ENTER or type command to continue
  ```
  (Home directory shown as `<user>`.) `~/.local/state/flux/lsp.log` holds `[rust-analyzer] error: Unknown binary 'rust-analyzer' in official toolchain …`. After Enter, `:e crates/flux/src/main.rs` shows the same prompt again.
- Neovim 0.12.5 with `vim.lsp.config('rust_analyzer', {cmd={'rust-analyzer'}, filetypes={'rust'}, root_markers={'Cargo.toml','.git'}})` + `vim.lsp.enable('rust_analyzer')` shows the **identical** message and prompt (with `nvim/lsp.log`), and repeats it on `:e` of another Rust file.

So flux's message matches Neovim for a server the user enabled. The problem is that flux shows it for a server the user never asked for, which is flux's own convenience feature. A user with no config gets a blocking prompt on first launch, breaking PRD S4.

## 2. Goal

A built-in server that flux started on its own and that fails to start costs the user one short, non-blocking line, once per session. Servers the user explicitly enabled keep Neovim's exact message.

## 3. Non-goals

- Detecting broken executables before starting them (running `--version` probes at startup costs time and isn't reliable across servers).
- Changing the message for explicitly enabled servers, or for servers that crash after initializing (both stay Neovim-identical).
- A status view (`:checkhealth`, `:LspInfo`, a new `:lsp status`); decided out for 1.0 (D1).
- Statusline indicators (screens parity; not needed).
- Changing which built-ins exist or the `executable()` check.
- `flux.lsp` (F-10). This spec defines the "explicit" flag that F-10's `flux.lsp.enable` will set.

## 4. Current vs required behavior

| Situation | Current | Required |
|---|---|---|
| Auto-enabled built-in exits before answering `initialize` | 2-line error + hit-enter prompt | one-line warning that fits the screen, no prompt; config disabled for the session |
| Same, then open another file of that filetype | prompt again | nothing (no restart, no message) |
| Auto-enabled built-in fails to spawn (`Server::start` error) | error + prompt if long | same as the row above |
| Two auto-enabled built-ins fail (e.g. `basedpyright` and `pyright`) | two prompts | one line naming both |
| Auto-enabled server initializes, later crashes | Neovim message (+ prompt if long) | unchanged |
| Explicitly enabled server (`:lsp enable x`, `$FLUX_LSP_CONFIG`) fails | Neovim message | unchanged |
| `:lsp enable rust_analyzer` after an auto failure | n/a (config still enabled) | re-enabled as explicit, starts for open buffers; a failure now shows Neovim's message |
| Failure while another message waits for Enter (e.g. an `init.lua` error from F-07) | the LSP error replaces it | the waiting message stays; the warning line is added after it |
| Server not on `PATH` | silent (not enabled) | unchanged |

## 5. Detailed requirements

**R1. Origin flag (decision D2).** flux records, per enabled config name, whether it was enabled **automatically** (a built-in whose executable was found at startup, `main.rs:59–68` when `$FLUX_LSP_CONFIG` is unset) or **explicitly** (everything else: `:lsp enable` with or without names, every config from `$FLUX_LSP_CONFIG`, and, after F-10, `flux.lsp.enable`). The flag lives in `flux_view::lsp::LspState` (for example `pub auto_enabled: HashSet<String>`); `main.rs` fills it; `ex_lsp::checked_enable` (`crates/flux-vim/src/lsp/ex_lsp.rs:77–112`) removes a name from it when enabling.

**R2. "Failed to start".** A client failed to start when either:
- spawning failed (`servers.rs:58–60`), or
- its process exited while the client was `ClientState::Initializing` (`crates/flux-view/src/lsp.rs:46–53`: "Started, `initialize` not answered yet"), and it was not `Stopping`.

**R3. Quiet path.** In `flux_vim::lsp::handle_exit`, if the client failed to start (R2) **and** its config name is auto-enabled (R1):
1. Do not call `editor.error` with the Neovim text.
2. Remove the config from `editor.lsp.enabled` for the rest of the session (decision D3), so `lsp_attach` won't start it again. Leave it in `editor.lsp.configs` so `:lsp enable <name>` still works.
3. Record it in a session list (for example `editor.lsp.failed: Vec<String>` of config names, in failure order, no duplicates).
4. Show the warning (R4).
Everything else `handle_exit` does today (`editor.lsp_exited`, `ex_lsp::exited`) still happens.

**R4. Warning text and width.** Show with `editor.warning` (warning color, not counted as an error). `<names>` is every name in the failed list (R3.3), joined with `, `. Use the first candidate whose display width is **less than** the screen width (so `Editor::show` never wraps it into a prompt):
1. `<names> failed to start; see <log path>`
2. `<names> failed to start; see lsp.log`
3. candidate 2 cut to `screen width − 1` columns, the last column being `…`.
`<log path>` is the `Servers` log path (`servers.rs:21–28`); when there is no log path (no `$HOME` and no `$XDG_STATE_HOME`), skip candidate 1. The log path has to reach `handle_exit`: pass it in, or keep passing the full `why`, and let the quiet path ignore what it doesn't need. The implementer chooses. Tifa may adjust the wording, but must keep the three-step width rule and the names.

**R5. Never dismiss a waiting message.** If `editor.hit_enter` is already true when the warning is due, keep the current message and prompt, and add the warning as one more line at the end of the message. Don't replace it, and don't clear `hit_enter`. (Startup case: F-07 shows `init.lua` errors with a prompt; a server failing a few milliseconds later must not hide them.)

**R6. Explicit and running cases unchanged.** For explicit configs (R1), and for any client that exits after reaching `Running`, `handle_exit` behaves exactly as today: same text, same `editor.error`, same prompt rules.

**R7. Re-enabling.** `:lsp enable <name>` (or `:lsp enable` in a buffer of that filetype) after a quiet failure re-adds the config to `enabled` as explicit (R1), removes the name from the failed list, and starts it for loaded buffers (existing `checked_enable` behavior). If it fails again, the Neovim message is shown (R6).

**R8. Log unchanged.** The server's stderr still goes to `lsp.log` with the `[<cmd>]` prefix, exactly as today.

**R9. No startup cost.** No new process spawns, probes or waits at startup.

## 6. Edge cases

1. **Failure before the first frame:** the warning is the first message drawn; no prompt.
2. **Very narrow terminal (e.g. 20 columns):** candidate 3 applies; width < screen width holds; test at width 20.
3. **Screen width 1 or 2:** candidate 3 may be just `…`; no panic on slicing (cut by display width with `unicode-width`, which is already a dependency of `flux-view`, never by bytes).
4. **Wide characters in the log path** (non-ASCII home dir): width measured with `unicode-width`, as `Editor::show` does.
5. **User stops a server with `:lsp stop` while it initializes:** the client is `Stopping`, so the exit is expected and there's no message (unchanged, `lsp/mod.rs:77–82`).
6. **`:lsp restart` of an auto-enabled server that then fails during initialize:** the user acted on it, so treat it as explicit: `restart` marks the name explicit before stopping. Neovim message.
7. **Two clients of the same auto config for different roots, both failing:** one name in the list, one disable.
8. **The server answers `initialize` and then exits within the same batch of events:** `Running` was reached, so R6 applies. Order of event handling decides; acceptable.
9. **`$FLUX_LSP_CONFIG` set** (tests, `xtask screens`): all its configs are explicit, so existing scripted comparisons with Neovim are unaffected.
10. **Resize after the warning:** nothing special; the message is already one line.

## 7. Acceptance criteria

A fake broken server works everywhere (no rustup needed):
```sh
mkdir -p /tmp/fakebin
printf '#!/bin/sh\necho "broken on purpose" >&2\nexit 1\n' > /tmp/fakebin/rust-analyzer
chmod +x /tmp/fakebin/rust-analyzer
```
- **A1 gates:** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo deny check` pass.
- **A2 repro fixed:** in an 80×24 terminal, `PATH=/tmp/fakebin:$PATH cargo run -- crates/flux/src/servers.rs` shows the file with one bottom line `rust_analyzer failed to start; see <home>/.local/state/flux/lsp.log` (or candidate 2 if that doesn't fit) and **no** "Press ENTER" prompt; keys work immediately.
- **A3 no retry:** then `:e crates/flux/src/main.rs` → no message, no new `broken on purpose` line appended to `lsp.log` (check `wc -l` before and after).
- **A4 explicit keeps Neovim text:** then `:lsp enable rust_analyzer` → `Client rust_analyzer quit with exit code 1 and signal 0. Check log for errors: …` with the prompt, as today.
- **A5 explicit via env unchanged:** `printf '[{"name":"rust_analyzer","cmd":["rust-analyzer"],"filetypes":["rust"],"root_markers":[["Cargo.toml"]]}]' > /tmp/lsp.json; PATH=/tmp/fakebin:$PATH FLUX_LSP_CONFIG=/tmp/lsp.json cargo run -- crates/flux/src/servers.rs` → the Neovim message and prompt (unchanged). (Keys as parsed by `crates/flux-lsp/src/config.rs:161–193`.)
- **A6 narrow:** same as A2 in a 30-column terminal → one line ending in `…` or candidate 2, no prompt.
- **A7 working server unaffected:** with a working `rust-analyzer` (where available) or the scripted fake server `flux-lsp-fake` via `$FLUX_LSP_CONFIG`, hover/diagnostics behave as before. Existing tests pass.

## 8. Tests to add

Unit tests next to the code (`#[cfg(test)] mod tests` in `crates/flux-vim/src/lsp/mod.rs`; that file has none today). Build the state without processes: an `Editor`, a temp dir with a `main.rs`, a `ServerConfig { name: "fake", cmd: vec!["fake".into()], filetypes: vec!["rust".into()], root_markers: vec![], settings: json!({}), init_options: Value::Null }` in both `lsp.configs` and `lsp.enabled`, then `editor.open(path)` (filetype detection calls `lsp_attach`, `crates/flux-view/src/filetype.rs:157`) to get a client in `Initializing`.
- `auto_failure_is_one_line_without_prompt` — mark `fake` auto; `handle_exit(…, "with exit code 1 and signal 0. Check log for errors: /tmp/x/lsp.log")` → `editor.hit_enter == false`, message kind Warning, text `fake failed to start; see /tmp/x/lsp.log`, `editor.error_count` unchanged.
- `auto_failure_disables_config_for_session` — after it, `lsp.enabled` lacks `fake`, `lsp.configs` has it; opening a second `.rs` file creates no new client and no outbox `Start`.
- `two_auto_failures_share_one_line` — configs `a` and `b` for the same filetype → text starts `a, b failed to start`.
- `warning_fits_narrow_screens` — widths 80, 30, 20, 2: displayed width < screen width, `hit_enter == false`.
- `explicit_failure_keeps_neovim_message` — not auto → `Client fake quit with exit code 1 …` via `editor.error`.
- `running_crash_keeps_neovim_message` — auto, but the client is set to `Running` first → Neovim text.
- `waiting_message_is_not_dismissed` — `editor.error("line one\nline two")` (prompt up), then an auto failure → `hit_enter` still true, text ends with the warning line, starts with `line one`.
- `lsp_enable_makes_it_explicit` — after an auto failure, `:lsp enable fake` (through `ex::execute`) → `fake` back in `enabled`, not in the auto set or failed list.
- `restart_counts_as_explicit` (edge case 6).

## 9. Files likely touched (guidance)

- `crates/flux-view/src/lsp.rs` (origin flag, failed list on `LspState`).
- `crates/flux-vim/src/lsp/mod.rs` (`handle_exit` quiet path + tests), `crates/flux-vim/src/lsp/ex_lsp.rs` (explicit on enable/restart).
- `crates/flux/src/main.rs` (mark auto-enabled names), possibly `crates/flux/src/servers.rs` (how the log path reaches `handle_exit`).
- Possibly `crates/flux-view/src/editor.rs` for an "append a line to the waiting message" helper (R5).

## 10. Docs to update (via Y'shtola)

- `docs/milestones.md` M6 LSP section (around lines 366–369, "each only when it's installed"): add "A built-in server that fails to start is reported once on one line and not retried for the session; `:lsp enable <name>` retries it."
- `docs/config.md` (created by F-07) or README LSP paragraph: the same sentence, plus where the log lives.
- `plans/flux-1.0-prd.md` FR-1 already matches this spec (updated Oct 6, 2026); nothing to change in this PR.

## 11. Risks

- **Hiding real problems:** a user who wanted rust-analyzer may miss the one-line warning. Mitigation: the line names the server and the log, and `:lsp enable` retries with the full message.
- **Message-model edge (R5):** appending to a waiting message touches shared message code; keep the change small and covered by `waiting_message_is_not_dismissed`.
- **Divergence from Neovim:** only for servers flux started on its own; Neovim never starts those. Explicit paths stay identical, so oracle/screens comparisons (which use `$FLUX_LSP_CONFIG`) are unaffected.

## 12. Definition of done

- [ ] A1–A7 pass; A2/A3 screen captures (e.g. `tmux capture-pane -p`) in the PR.
- [ ] `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; `cargo deny check`: pass, in `AGENTS.md` order.
- [ ] No `gen`; recorded expectations byte-identical.
- [ ] PR titled **Don't block startup when an auto-started language server fails** (sentence-case imperative, squash-merged with `(#N)`).
- [ ] PR description: before/after captures, the final warning copy (Tifa), and a **Fina verify** block.
- [ ] After approval, Y'shtola records `YYYY-MM-DD: F-13 approved by Fina (...)` in `.sectorfive/decisions.md`.

## Decisions (all settled, none open)

Settled by the owner on Oct 6, 2026. The requirements above implement them.

- **D1. Reporting is the warning line plus `lsp.log`; no `:lsp status`.** flux keeps Neovim's `:lsp` subcommands (`enable|disable|restart|stop`) and adds no status view or statusline item for 1.0. PRD FR-1 was updated to match. Implemented by R4, R8 and the § 3 non-goal.
- **D2. Configs from `$FLUX_LSP_CONFIG` count as user-enabled (explicit).** Implemented by R1, edge case 9 and A5.
- **D3. A failed auto-started server is disabled for the rest of the session;** `:lsp enable <name>` retries it. Implemented by R3.2, R7, A3 and `auto_failure_disables_config_for_session`.
