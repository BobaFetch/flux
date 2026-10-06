# F-07 — Lua runtime, `init.lua` loading and `flux.opt`

| | |
|---|---|
| Backlog | F-07 (M7 Stage C, first of five separately mergeable PRs: F-07, then F-08, F-10, F-11 and F-09). Unblocks F-08–F-11, F-14, F-16, F-20, F-29 |
| Size | M (≈ 2–3 days agent time) |
| Spec status | **Ready to implement.** All owner decisions settled Oct 6, 2026 (§ Decisions) |
| Repo state | `BobaFetch/flux` @ `d8ab89b` (main), read Oct 6, 2026 |
| Proposed branch / PR title | `lua-config` / **Load init.lua at startup with a Lua runtime and flux.opt** |
| Sector 5 routing | Lightning (new `flux-lua` crate, mlua dependency, `flux.opt`/`flux.cmd`/`flux.version`, option setter refactor in `flux-vim`) → Aerith (`main.rs`: flags, load order, error surfacing) → Tifa (error and notice copy) → Fina (fixture tests, `cargo deny check`, manual pass). Docs, ownership row, decisions and contracts entries via Y'shtola |

This spec is self-contained: everything an implementer needs is in this file and the repo. The public Lua surface it starts is a compatibility promise for all of 1.x, so names, types and error texts below are normative.

## 1. Problem

flux has no user configuration. Every option resets to its default on each launch, and the M7 plan's Stage C (Lua config) has not started.

**Evidence:**
- `crates/flux/src/main.rs:22` `const USAGE: &str = "usage: flux [file ...]";`. Lines 29–42 accept only `-h/--help`, `-v/--version` and file names; any other `-x` is `bail!("unknown option …")`. There is no config path, no `--clean`, no `-u`.
- `main.rs:44–69` is the whole startup: `Editor::new`, `'termguicolors'` from `$COLORTERM` (line 47), LSP configs from `$FLUX_LSP_CONFIG` or the built-ins (53–68), then `editor.open_args(&files)` (69). Nothing reads a file from `~/.config`.
- No Lua anywhere: `Cargo.lock` has no `mlua`; no crate depends on a Lua library.
- `.sectorfive/plans/m7.md:33–37` (Stage C): "`init.lua` loaded at startup; minimal `flux` API: `flux.opt` (options), `flux.map` (non-recursive per-mode mappings), `flux.lsp` (server configs …)"; `:38–40` `:colorscheme`/`:highlight`; `:41` "Lua runtime: mlua 0.12, features `lua54` + `vendored` (see D1)". `m7.md:67–69` (D5): config at `$XDG_CONFIG_HOME/flux/init.lua`, else `~/.config/flux/init.lua`, "missing file means stock behavior (no error)". `m7.md:91–93` (C2): "config before `open_args` … error surfacing for a failing `init.lua` (message + continue stock, never a crash)".
- `m7.md:138–141`: "`flux.map` engine semantics is the largest design surface in C; if review shows it ballooning, Y'shtola may split C and defer maps to M8 by plan amendment (not silently)." This spec is that split (see § 3 and `plans/feature-backlog.md`).
- `m7.md:103–104` calls the Lua API "experimental, versioned by milestone". The owner decided on Oct 6, 2026 that Stable `flux.*` items are frozen under semver for all of 1.x (D2), so whatever is merged here is what users will copy into their configs and rely on.
- `docs/milestones.md:452–453` still says "Add TOML configuration for static editor preferences and the planned Lua API". The owner decided Lua only, no TOML.
- `:set` validation lives in a private module: `crates/flux-vim/src/lib.rs:23` `mod set;`. The checks (`set.rs:141–146` E518/E474, `:190` E521, `:199–208` E487 and the `'numberwidth'` limit, `:281–311` string option values) are not reachable from another crate.
- Only one message is shown at a time: `Editor::show` (`crates/flux-view/src/editor.rs:960–979`) replaces `self.message` and recomputes `hit_enter`. A config error shown before `open_args` would be replaced by any message `open_args` emits (for example `"x" [Permission Denied]` from `Editor::open`, `editor.rs:386`).
- `crates/flux/src/main.rs:81` creates the `Engine` inside `run()`, after the terminal session starts (`:78`).

**Verified by the PM on a Linux x86_64 test machine (Oct 6, 2026):**
- mlua dry-run in a throwaway worktree: a new crate depending on `mlua = { version = "0.12", features = ["lua54", "vendored"] }` resolved to **mlua 0.12.2** (MIT, rust-version 1.88), mlua-sys 0.13.0, lua-src 551.0.2. It built and ran on the pinned 1.98.0 toolchain (`_VERSION` = `Lua 5.4`), and **`cargo deny check` passed: `advisories ok, bans ok, licenses ok, sources ok`** (`deny.toml` has `all-features = true`, so optional LuaJIT sources were checked too). New normal dependencies: bstr, either, lock_api, mlua, mlua-sys, num-traits, parking_lot, parking_lot_core, rustc-hash, scopeguard, smallvec (plus build-only cc, lua-src, luajit-src, pkg-config, which, shlex, find-msvc-tools, autocfg). Clean build of the test crate: ≈ 9 s.
- `mlua::Lua::new()` (safe mode) took ≈ 0.11 ms. In safe mode `debug` is `nil`, `package.loadlib` raises "package.loadlib is disabled in safe mode", and only 3 package searchers remain (no C modules). `io` and `os` are present. The default `package.path` includes `./?.lua;./?/init.lua` (the current directory).
- A metatable `__newindex` written in Lua that calls a Rust function and raises with `error(msg, 2)` produced `/cfg/init.lua:2: Unknown option 'nope'` (location of the user's line). An error raised directly from a Rust callback had no file:line prefix.
- Neovim 0.12.5 with `XDG_CONFIG_HOME` pointing at a test `init.lua`:
  - Runtime error → `Error in /tmp/xdgcfg/nvim/init.lua:` / `E5113: Lua chunk: /tmp/xdgcfg/nvim/init.lua:3: attempt to index local 'x' (a nil value)` / `stack traceback:` / `        /tmp/xdgcfg/nvim/init.lua:3: in main chunk` / hit-enter prompt.
  - Syntax error → `Error in …init.lua:` / `E5112: Lua chunk: …init.lua:2: unexpected symbol near '<eof>'` (no traceback).
  - `vim.o.number = 1` → `Invalid value for option 'number': expected boolean, got number 1`. `vim.o.nosuch = 1` → `Unknown option 'nosuch'`.
  - Lines before the error **stay applied**, lines after it don't: `vim.o.number = true` (line 1) was on, `vim.o.tabstop = 3` (line 4, after the error) was not (`tabstop=8`).
  - `nvim -u /tmp/does-not-exist.lua` → `E282: Cannot read from "/tmp/does-not-exist.lua"`, then normal startup.

## 2. Goal

At startup flux runs the user's `init.lua` in an embedded Lua 5.4 runtime, before files open. The config can read and set every implemented option through `flux.opt`. A missing config is silent, a broken one shows a Neovim-style error and flux starts anyway, and `flux --clean` / `flux -u FILE` control which config runs.

## 3. Non-goals

- `flux.map` (F-09), `flux.lsp` (F-10), `:colorscheme`/`:highlight` and highlight API (F-11), and new options such as `'scrolloff'` (F-08). Their **names and shapes are reserved in Appendix A** so that this PR doesn't paint them into a corner; they are not implemented here.
- Autocmds, user commands, plugin loading, `:lua`, `:source`, `:luafile`, reloading the config without restarting (post-1.0, `plans/flux-1.0-prd.md`). `flux.cmd("source …")` simply gets `E492` like any unknown command.
- Neovim API compatibility (`vim.*`). README line 6: flux has its own Lua API.
- TOML or any other config format.
- Per-filetype option overrides from Lua (needs autocmds or ftplugin dirs; post-1.0). Filetype settings still override `init.lua` values in the buffers they apply to, as in Neovim.
- `init.vim`, `exrc`/project-local configs, `$VIMINIT`-style variables.
- Sandboxing the user's config. It runs with the user's permissions, like Neovim's (`plans/flux-1.0-prd.md` NFR "Security"); this is documented, not prevented.
- Changing `$FLUX_LSP_CONFIG` (it stays as the test override; F-10 adds the Lua route).
- Changing `servers.rs`'s state-dir logic.

## 4. Current vs required behavior

| Situation | Current | Required |
|---|---|---|
| `~/.config/flux/init.lua` with `flux.opt.number = true` | ignored; no line numbers | numbers on in the first window and every later window |
| No config file | stock | stock, no message, no visible delay |
| `init.lua` has a syntax error | n/a | flux starts; message `Error in <path>:` / `E5112: Lua chunk: <path>:<line>: <reason>`; hit-enter prompt |
| `init.lua` errors at line N | n/a | lines before N stay applied, the rest doesn't run; message `Error in <path>:` / `E5113: Lua chunk: <path>:N: <reason>` / traceback; flux starts |
| `flux.opt.tabstop = "4"` | n/a | Lua error `Invalid value for option 'tabstop': expected number, got string "4"` at the user's line |
| `flux --clean a.txt` | `unknown option --clean` | opens `a.txt` with no config loaded |
| `flux -u ~/min.lua` | `unknown option -u` | runs `~/min.lua` instead of the default config |
| `flux -u NONE` | `unknown option -u` | no config loaded |
| `flux -u missing.lua` | `unknown option -u` | starts; message `E282: Cannot read from "missing.lua"` |
| `print("hi")` in `init.lua` | n/a | `hi` shown as a message after startup; nothing written to stdout |
| `$XDG_CONFIG_HOME=/x` | n/a | reads `/x/flux/init.lua` |
| `--help` | `usage: flux [file ...]` | usage shows `--clean` and `-u` |

## 5. Detailed requirements

### Runtime and crate

**R1. New crate `crates/flux-lua`** (package `flux-lua`, `publish = false` until F-26 settles publish names; workspace `version`/`edition`/`rust-version`/`license`/`authors`; `[lints] workspace = true`). It depends on `flux-view`, `flux-vim` and `mlua`. Engine crates (`flux-core`, `flux-view`, `flux-vim`, `flux-syntax`, `flux-lsp`, `flux-tui`) must **not** depend on `mlua` or `flux-lua`. Add `flux-lua = { path = "crates/flux-lua", version = "0.0.1" }` to `[workspace.dependencies]` beside the others.

**R2. Dependency.** Add `mlua = { version = "0.12.2", features = ["lua54", "vendored"] }` to `[workspace.dependencies]` (explicit version, per `AGENTS.md` "New dependencies must use explicit versions") and use it with `mlua.workspace = true`. No other features. No other new dependency. `cargo deny check` must pass with no new warnings other than the existing `license-not-encountered` ones.

**R3. Safe mode, no `unsafe`.** Create the state with `mlua::Lua::new()` (safe standard libraries). The workspace forbids `unsafe` (`Cargo.toml` `[workspace.lints.rust] unsafe_code = "forbid"`); do not use `Lua::unsafe_new` or C module loading.

**R4. `package.path`.** Replace the default `package.path` with exactly `<config dir>/lua/?.lua;<config dir>/lua/?/init.lua`, where `<config dir>` is the directory holding the config file being loaded (for `-u FILE`, `FILE`'s directory). Set `package.cpath` to `""`. The current directory must never be on the search path (a repo could otherwise plant `foo.lua` for `require("foo")`).

**R5. One runtime per session.** The Lua state is created once at startup and **kept alive for the whole session** (owned by the binary and passed into `run()`), even though F-07 only uses it at startup. F-09 (Lua-function mappings) and F-11 (colorschemes) call back into it. It must not be stored in `Editor` (`flux-view` stays Lua-free).

**R6. Borrowing the editor.** Lua functions that touch the editor get it for the duration of a call only (for example with `mlua::Lua::scope` around the chunk, over a `RefCell<&mut Editor>`). No `'static` editor pointer, no global mutable state. (Guidance, not mandated: the prototype in § 1 used `lua.scope(|scope| { … scope.create_function(…) … })` successfully.)

### Config discovery and flags

**R7. Default path.** A pure function, testable without touching the process environment (edition 2024 makes `std::env::set_var` `unsafe`, which the workspace forbids, so tests cannot set env vars):
`fn config_path(xdg_config_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf>`
- If `xdg_config_home` is set, non-empty and absolute → `<it>/flux/init.lua`.
- Else if `home` is set and non-empty → `<home>/.config/flux/init.lua`.
- Else `None` (no config, no message).
Same rule on macOS and Linux (not `~/Library`). The command name, config path and Lua namespace stay `flux` (owner decision, Oct 5, 2026; the package name `flux-editor` does not appear in any path).

**R8. Missing default config is silent.** If the default path doesn't exist, flux starts with stock behavior and no message. If it exists but is a directory or unreadable, show `E282: Cannot read from "<path>"` (Neovim's text).

**R9. `--clean`.** Skips loading any config. Also skips nothing else today (flux has no shada/plugins); document it as "start without user config".

**R10. `-u <file>`.** Loads `<file>` instead of the default path (relative paths are relative to the current directory). `-u NONE` loads nothing. A missing or unreadable `<file>` → `E282: Cannot read from "<file>"` (as given on the command line) and flux starts. `-u` with no following argument → exit with an error before the terminal starts, like other bad flags: `bail!("argument missing after -u\n{USAGE}")`. If both are given, `--clean` wins.

**R11. Usage text.** `USAGE` becomes:
```
usage: flux [options] [file ...]
  --clean      start without loading init.lua
  -u <file>    load <file> instead of init.lua (-u NONE: load nothing)
  -h, --help   show this help
  -v, --version  show the version
```
(Exact alignment is Tifa's call; the four options and their meaning are required.)

### Load order and errors

**R12. Startup order** in `main.rs` (Aerith):
1. Parse flags.
2. `Editor::new(width, height)`.
3. `'termguicolors'` from `$COLORTERM` (as today), so the config can override it.
4. LSP configs and `enabled` (as today), so F-10 can amend them from Lua.
5. Create the Lua runtime and load the config (unless `--clean`/`-u NONE`/no default file).
6. `editor.open_args(&files)`.
7. Surface config messages (R14).
8. Enter the terminal and run the loop with the same runtime.

**R13. Partial application (decision D1).** The chunk runs top to bottom. On an uncaught error, statements already executed stay in effect and the rest of the file does not run (Neovim's behavior, verified in § 1). flux never panics or exits because of the config.

**R14. Error message format.** For a load (syntax) error:
```
Error in <path>:
E5112: Lua chunk: <lua message>
```
For a runtime error:
```
Error in <path>:
E5113: Lua chunk: <lua message>
stack traceback:
        <frame>
        …
```
- `<path>` is the path as resolved (absolute for the default path, as typed for `-u`).
- `<lua message>` is Lua's message with mlua's own prefixes removed (no `runtime error: `, no `syntax error: `), starting with `<path>:<line>:`.
- Traceback lines come from mlua's traceback when present, each indented by 8 spaces; lines that only name flux's internal wrapper chunk (named e.g. `[flux]`) may be dropped. Tests assert the first two lines exactly and only that the traceback mentions `init.lua:<line>` (Lua 5.4's wording differs from Neovim's LuaJIT and is not a parity target).
- Shown with `editor.error(...)` so it counts as an error and, being multi-line, gets the hit-enter prompt.

**R15. Messages survive `open_args`.** Because only one message is shown at a time (§ 1), config output is collected during step 5 and shown in step 7: if `open_args` left a message, the final message is the config text, a newline, then that message. `print` output (R20) comes before errors. Only one hit-enter prompt results.

**R16. Errors from flux functions point at the user's line.** Every error raised by `flux.opt`, `flux.cmd` or any later `flux.*` function must carry the caller's `<file>:<line>:` prefix (for example, by raising from a Lua-side wrapper with `error(msg, 2)`, as the prototype in § 1 did). `pcall` must catch them like any Lua error.

### `flux.*` surface implemented here

**R17. `flux` global.** A global table `flux`, also returned by `require("flux")` (`package.loaded.flux = flux`). Only the members in R18–R21 exist after this PR. Reading a missing member returns `nil` (plain table semantics); keys starting with `_` are internal and undocumented.

**R18. `flux.version`.** A table `{ major = <int>, minor = <int>, patch = <int> }` from `CARGO_PKG_VERSION` of the `flux` package (today `0, 0, 1`). Read-only is not enforced.

**R19. `flux.opt`.** A proxy table with `:set` semantics (like Neovim's `vim.o`):
- **Names:** full or short names from `crates/flux-view/src/options.rs` `OPTIONS` (36 today), via `options::find`. Error messages use the full name.
- **Read** `flux.opt.<name>` → the effective value: the current buffer's for buffer options, the current window's for window options, the global one otherwise. Types: boolean, integer, string.
- **Write** `flux.opt.<name> = v` → same effect as `:set name=v` (`Which::Both` in `set.rs`): sets the global value and the current buffer's/window's local value, applies `'filetype'` side effects, then `editor.sync_window_sizes()`. At startup this means every buffer and window created later inherits it.
- **Type rules:** boolean options take `true`/`false` only; number options take Lua integers, or floats with an integral value (`4.0` → 4); string options take strings only. `nil` is rejected for every option.
- **Errors (exact text, Lua errors with location per R16):**
  - unknown name (read or write): `Unknown option '<name as written>'`
  - wrong type: `Invalid value for option '<full name>': expected <boolean|number|string>, got <lua type> <value>` where `<value>` is `tostring(v)` for numbers/booleans/nil and the string in double quotes for strings. (`got number 1` matches Neovim's verified text.)
  - non-integral number: `Invalid value for option '<full name>': expected integer, got number 2.5`
  - value rejected by `:set` validation: the same message `:set` gives for `name=value`, e.g. `E487: Argument must be positive: tabstop=0`, `E474: Invalid argument: background=blue`, `E474: Invalid argument: numberwidth=21`.
- **No list helpers** (`:append`, `:remove`, Neovim's `vim.opt` objects). For list edits use `flux.cmd("set completeopt+=noselect")`.
- `pairs(flux.opt)` is not supported (iterating yields nothing; documented).

**R20. `print`.** The global `print` is replaced: arguments go through `tostring`, joined by a tab; each call adds one line to the startup message (R15). Nothing is written to stdout or stderr (the workspace warns on stdout/stderr printing, `Cargo.toml` `[workspace.lints.clippy]`).

**R21. `flux.cmd(command)` (decision D3).** Runs one Ex command line, as if typed after `:` (through `flux_vim::ex::run` with the session's engine, or `ex::execute` at startup). If the command reports an error (`editor.error_count` increased), raise a Lua error with that message text (e.g. `E518: Unknown option: foo`) and don't leave it as the displayed message. Non-string argument → `flux.cmd: expected string, got <type>`. Multiple commands separated by `|` are not supported (flux has no `|`; `plans/flux-1.0-prd.md` post-1.0 list).

### Shared option setter (Lightning, `flux-vim`)

**R22.** Expose from `flux-vim` a public, documented pair used by both `:set` and `flux.opt`, so validation exists once:
- `pub fn get_option(editor: &Editor, name: &str) -> Result<Value, String>` (effective value; `Err("Unknown option '<name>'")` style is the caller's to format).
- `pub fn set_option(editor: &mut Editor, name: &str, value: Value) -> Result<(), String>` applying `:set name=value` semantics and returning the same E-message `:set` would show (without `editor.error`, which stays `:set`'s job).
`do_set`/`set_one` must go through the same checks, and all existing `:set` tests and oracle cases (91 cases use `:set`/`:setl`) must stay green. The exact module path is the implementer's choice (e.g. `pub mod set` with these two functions public).

### Docs and process

**R23.** Record the dependency approval and this split of Stage C in `.sectorfive/decisions.md` (Y'shtola), e.g. `2026-10-DD: Stage C split into F-07 (runtime, init.lua, flux.opt/cmd/version, --clean/-u), F-08, F-09 (maps; may slip to M8), F-10, F-11. mlua 0.12.2 (lua54+vendored) added per m7 D1; cargo deny green.`

**R24.** Add `crates/flux-lua/**` → Lightning to `.sectorfive/ownership.md` (tests under it stay Fina's by the existing `crates/*/tests/**` row), and point `.sectorfive/contracts.md` at `docs/config.md` as the Lua API and config-path contract.

## 6. Edge cases

1. **Empty `init.lua`:** loads, no message.
2. **UTF-8 BOM or CRLF line endings** in `init.lua`: must load (strip a leading BOM before loading; Lua accepts CRLF). Test both.
3. **Shebang line** (`#!/usr/bin/env lua`): Lua skips a first line starting with `#` only for files loaded by `luaL_loadfile`; if loading from a string, strip it too, so both forms behave the same.
4. **Error inside `require("mod")`** from `<config>/lua/mod.lua`: same E5113 format; the message names `mod.lua`'s path and line; `Error in` still names `init.lua`.
5. **`flux.opt.filetype = "rust"` at startup:** applies to the initial empty buffer; files opened by `open_args` get their own detected filetype (as `:set ft=rust` before `:e` would). Document.
6. **Filetype settings vs config:** `flux.opt.shiftwidth = 2` then `flux main.rs`: Rust's filetype settings may override `shiftwidth` in that buffer, as in Neovim. Not a bug; document in `docs/config.md`.
7. **`flux.opt.termguicolors = false`** with `COLORTERM=truecolor`: config wins (step 3 runs before step 5).
8. **Window options and splits:** `flux.opt.number = true`, then `:vsplit` → both windows numbered.
9. **Very long error** (deep traceback): uses the existing hit-enter/`more` paging (`editor.rs:975–978`); no truncation needed.
10. **Infinite loop in `init.lua`:** flux hangs at startup, as Neovim does. Not handled; documented (`flux --clean` is the escape hatch).
11. **`os.exit()` in `init.lua`:** exits before the terminal starts, as Neovim does. Not handled; documented.
12. **`io.write`** writes to the shell's stdout before the TUI starts. Not intercepted; documented as unsupported (use `print`).
13. **Non-UTF-8 path** for `-u` or `$HOME`: use `OsString`/`PathBuf` throughout; display with `Path::display()`.
14. **Config sets an option to its current value:** no error, no message.
15. **`flux.opt.report = -1`:** `E487: Argument must be positive: report=-1` (same as `:set report=-1`, `set.rs:201–203`).
16. **`-u` given twice:** the last one wins.
17. **A file literally named `--clean`:** `flux -- --clean` is not supported today (no `--` handling); out of scope, unchanged.

## 7. Acceptance criteria

Run from the repo root on the pinned toolchain (`rust-toolchain.toml`: 1.98.0). Use a temp `XDG_CONFIG_HOME` so the real config is untouched.

- **A1 gates:** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo deny check` all pass. `cargo deny check` output ends with `advisories ok, bans ok, licenses ok, sources ok`.
- **A2 no Lua in engine crates:** `cargo tree -p flux-view -e normal | rg mlua` and the same for `flux-vim`, `flux-core`, `flux-syntax`, `flux-lsp`, `flux-tui` print nothing; `cargo tree -p flux -e normal | rg mlua` prints `mlua v0.12.x`.
- **A3 options apply:**
  ```sh
  d=$(mktemp -d); mkdir -p $d/flux
  printf 'flux.opt.number = true\nflux.opt.ts = 4\nflux.opt.expandtab = true\n' > $d/flux/init.lua
  XDG_CONFIG_HOME=$d cargo run -- README.md
  ```
  Line numbers shown; `:set ts? et? nu?` shows `tabstop=4`, `expandtab`, `number`; `:vsplit` shows numbers in both windows.
- **A4 partial apply + message:** `printf 'flux.opt.number = true\nlocal x = nil\nprint(x.y)\nflux.opt.tabstop = 3\n' > $d/flux/init.lua`, start flux: message's first two lines are `Error in <d>/flux/init.lua:` and `E5113: Lua chunk: <d>/flux/init.lua:3: attempt to index a nil value (local 'x')`, then `stack traceback:`, then a hit-enter prompt. After Enter: numbers on, `:set ts?` → `tabstop=8`.
- **A5 syntax error:** `printf 'flux.opt.number =\n' > …` → `Error in …:` / `E5112: Lua chunk: …init.lua:2: unexpected symbol near <eof>`; flux usable.
- **A6 type and name errors:** `flux.opt.number = 1` → `…init.lua:1: Invalid value for option 'number': expected boolean, got number 1`; `flux.opt.nosuch = 1` → `…init.lua:1: Unknown option 'nosuch'`; `flux.opt.tabstop = 0` → `…init.lua:1: E487: Argument must be positive: tabstop=0`.
- **A7 flags:** with a config that sets `number`: `flux --clean README.md` → no numbers; `flux -u NONE README.md` → no numbers; `printf 'flux.opt.relativenumber = true\n' > /tmp/alt.lua; flux -u /tmp/alt.lua README.md` → relative numbers, no absolute numbers; `flux -u /tmp/nope.lua` → message `E282: Cannot read from "/tmp/nope.lua"`, editor usable; `flux -u` → exits non-zero before the TUI with `argument missing after -u` and the usage.
- **A8 no config:** `XDG_CONFIG_HOME=$(mktemp -d) flux README.md` → no message, same screen as before this change.
- **A9 print and require:** `<d>/flux/lua/mine.lua` containing `return { tw = 72 }`, `init.lua` containing `flux.opt.textwidth = require("mine").tw; print("loaded", flux.version.major)` → message `loaded	0` (tab), `:set tw?` → `textwidth=72`. With a `mine.lua` in the **current directory** and none under `<d>/flux/lua/`, `require("mine")` fails (E5113, "module 'mine' not found").
- **A10 flux.cmd:** `flux.cmd("set completeopt+=noselect")` → `:set cot?` → `completeopt=menu,popup,noselect`; `local ok, err = pcall(function() flux.cmd("set nosuch") end)` gives `ok == false` and `err == "<path>:1: E518: Unknown option: nosuch"`, and no message is left on screen. (Called as `pcall(flux.cmd, …)` directly, the error has no location, because the caller is `pcall` itself; that is standard Lua.)
- **A11 file message ordering:** as a non-root user, `touch /tmp/locked && chmod 000 /tmp/locked`, keep the erroring config from A4, run `flux /tmp/locked` → one hit-enter message: the config error first, then the `"/tmp/locked" …` open error.
- **A12 existing behavior:** `cargo test -p flux-vim --test oracle` unchanged result; `cargo xtask indent check` and `cargo xtask colors check` pass where Neovim is installed (unchanged files).

## 8. Tests to add

Follow repo conventions: unit tests in `#[cfg(test)] mod tests` beside the code, integration tests under `crates/<crate>/tests/` (Fina-owned), fixtures as files under `tests/`, no env-var mutation.

`crates/flux-lua` (unit or `tests/config.rs`, fixtures in `crates/flux-lua/tests/fixtures/`):
- `config_path_prefers_absolute_xdg` / `config_path_ignores_empty_or_relative_xdg` / `config_path_falls_back_to_home` / `config_path_none_without_home` (R7).
- `opt_sets_global_and_local` — load `number = true`, `ts = 4`; assert `editor.options.window.number`, `editor.window.opts.number`, current buffer `tabstop`, global `tabstop`.
- `opt_reads_effective_values` — `return flux.opt.tabstop` after `:setlocal ts=3` → 3.
- `opt_short_and_full_names`.
- `opt_type_errors` — boolean/number/string mismatch, `nil`, `2.5`, `4.0` accepted; assert exact texts from R19.
- `opt_validation_reuses_set_messages` — `tabstop=0`, `background="blue"`, `numberwidth=21`, `report=-1`.
- `error_has_user_location` — error text starts with `<fixture path>:<line>:` (R16).
- `partial_apply_on_error` — fixture from A4; options before the error applied, after not.
- `message_format_runtime_and_syntax` — first two lines exact (R14); traceback mentions `init.lua:3`.
- `print_goes_to_messages`.
- `require_uses_config_lua_dir_only` — `package.path` equals R4; a module in a temp cwd is not found.
- `bom_crlf_and_shebang_load` (edge cases 2–3).
- `cmd_runs_ex_and_raises_errors` (R21).
- `version_matches_cargo` — `flux.version` equals `CARGO_PKG_VERSION` split.
- `missing_and_directory_config` — `E282` text for `-u` paths; default-path miss returns no message.

`crates/flux-vim`:
- `set_option_matches_set_command` — for every option in `OPTIONS`, setting its default via `set_option` and via `:set name&` gives the same state; invalid values give the same error text both ways.

`crates/flux/src/main.rs` (`#[cfg(test)] mod tests`, the binary crate):
- `parses_clean_and_u` — `--clean`, `-u FILE`, `-u NONE`, `-u` missing argument, last `-u` wins, `--clean` beats `-u`. Factor flag parsing into a function over `Vec<OsString>` so it can be tested.
- `config_messages_survive_open_args` — R15 ordering, using an `Editor` and a path that fails to open.

## 9. Files likely touched (guidance)

- `Cargo.toml` (workspace deps), `Cargo.lock`.
- New `crates/flux-lua/{Cargo.toml,src/lib.rs,…}` and `crates/flux-lua/tests/`.
- `crates/flux-vim/src/set.rs`, `crates/flux-vim/src/lib.rs` (R22).
- `crates/flux/Cargo.toml`, `crates/flux/src/main.rs` (flags, order, messages, pass runtime into `run`).
- No changes to `flux-view` are expected; if one is needed (e.g. a message-append helper), keep it Lua-free.

## 10. Docs to update (via Y'shtola)

- New `docs/config.md`: config path rules (R7), `--clean`/`-u`, error behavior (partial apply, message format), `package.path`, security note (runs with your permissions), `print` vs `io.write`, filetype-override note, and the reference for `flux.version`, `flux.opt` (with the option list generated from `OPTIONS` or kept in sync by a test), `flux.cmd`. Mark each item Stable or Experimental per D2, with a short section stating the D2 promise (Stable items frozen under semver for 1.x; Experimental items may change in a minor release).
- `docs/milestones.md:452–453`: replace the TOML sentence with "Lua configuration (`~/.config/flux/init.lua`) …"; add the M7 Stage C entry when F-07–F-11 land (F-12 does the final checkbox).
- `README.md`: one line under usage: "Configure flux in `~/.config/flux/init.lua` (see `docs/config.md`)."
- `.sectorfive/decisions.md`, `.sectorfive/ownership.md`, `.sectorfive/contracts.md` per R23–R24.
- `AGENTS.md` Architecture: one bullet: "`flux-lua` owns the Lua runtime and the `flux.*` API; engine crates never depend on it."

## 11. Risks

- **API lock-in.** Anything merged becomes a 1.x promise. Mitigation: small surface, exact error texts, Appendix A reserved shapes, D2 policy.
- **Native build weight.** Vendored Lua adds a C compile (≈ 9 s on the PM's Linux test machine for the probe crate) and needs a C compiler wherever flux is built from source (`cargo install flux-editor`). Mitigation: document in README build requirements (F-28); release binaries (F-25) avoid it for most users.
- **Borrowing the editor from Lua** (R6) is the main implementation risk; a wrong design here makes F-09's Lua-function mappings hard. Mitigation: R5/R6 require a session-long runtime and per-call borrowing now.
- **Message model** holds one message; ordering bugs would hide config errors (R15, A11).
- **Divergence from Neovim error texts** for Lua 5.4 vs LuaJIT wording (e.g. "attempt to index a nil value (local 'x')" vs "attempt to index local 'x' (a nil value)"). Accepted; only the `Error in` / `E511x: Lua chunk:` frame mirrors Neovim.
- **Ownership name clash:** "Aerith" in this spec is the Sector 5 binary/TUI role, not the PM agent.

## 12. Definition of done

- [ ] A1–A12 pass; A2 output pasted in the PR.
- [ ] `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; `cargo deny check`: pass, in `AGENTS.md` order.
- [ ] No `gen`; `expected.json`, indent expectations and `colors.json` byte-identical.
- [ ] PR titled **Load init.lua at startup with a Lua runtime and flux.opt** (sentence-case imperative, squash-merged with `(#N)`, matching history such as "Report a server's exit like Neovim: exit code, signal and the log (#8)").
- [ ] PR description lists the public surface added (R17–R21) verbatim and links Appendix A; includes the `cargo deny check` tail and a **Fina verify** block (approve or veto with evidence: fixture tests, deny, manual A3–A11).
- [ ] After approval, Y'shtola records the decisions/ownership/contracts entries (R23–R24) and Fina updates `.sectorfive/baseline.md` (test count, deny result).

## Appendix A — `flux.*` surface for 1.x (normative names and shapes; implemented by the spec named)

Rules for the whole namespace:
- `flux` is a global and `require("flux")`. Keys not listed here are reserved for flux; keys starting with `_` are internal.
- Every function validates its arguments and raises a Lua error with the caller's location (R16). Unknown keys in option tables raise `flux.<fn>: unknown key '<k>'` so new keys can be added later without silently changing meaning.
- Stability per D2: items below are **Stable** at 1.0 unless marked Experimental.

| Member | Spec | Shape (confirmed Oct 6, 2026, D4; behavior details are settled in each spec) |
|---|---|---|
| `flux.version` | F-07 | `{ major, minor, patch }` integers |
| `flux.opt.<name>` | F-07 (+F-08 adds options) | get/set with `:set` semantics; see R19 |
| `flux.cmd(cmd)` | F-07 | run one Ex command; errors raise (R21) |
| `flux.map(mode, lhs, rhs, opts?)` | F-09 | `mode`: `"n"`, `"v"` (Visual; `"x"` accepted as an alias, since flux has no Select mode), `"o"`, `"i"`, `"c"`, or a list of them. `lhs`: Vim key notation (`"<C-s>"`, `"<leader>f"`). `rhs`: a key string (always non-recursive) or a Lua function. `opts`: `{ desc = string }` only at first. Defining the same `mode`+`lhs` again replaces it |
| `flux.unmap(mode, lhs)` | F-09 | removes a mapping; error if none |
| `flux.g.mapleader`, `flux.g.maplocalleader` | F-09 | strings used by `<leader>`/`<localleader>` at definition time, as in Vim. `flux.g` holds only these at 1.0 |
| `flux.lsp.config(name, cfg)` | F-10 | Like Neovim 0.11+ `vim.lsp.config`: `cfg` fields `cmd` (list of strings), `filetypes`, `root_markers` (list of strings, or list of lists for equal priority), `settings` (table → JSON), `init_options`. Merged into the existing config of that name (built-in or earlier call); a new name creates one |
| `flux.lsp.enable(name \| names, enable?)` | F-10 | Like `vim.lsp.enable`. `enable = false` disables, including auto-enabled built-ins. An explicit enable counts as user-requested for F-13 |
| `flux.highlight(group, attrs)` | F-11 | Like `nvim_set_hl(0, group, attrs)`: `fg`, `bg`, `sp` (`"#rrggbb"` or a color name), `ctermfg`, `ctermbg` (0–255), `bold`, `italic`, `underline`, `undercurl`, `strikethrough`, `reverse`, `cterm = { bold=…, … }`, `link = "Group"`. Replaces the group's definition (not a merge) |
| `:colorscheme {name}` | F-11 | Built-ins `default` and one light-friendly alternate; user schemes at `<config dir>/colors/<name>.lua`, run in the same runtime (they use `flux.highlight` and `flux.opt.background`) |

`flux.map` may slip to M8 by plan amendment (`m7.md:138–141`); the names above stay reserved either way.

## Decisions (all settled, none open)

Settled by the owner on Oct 6, 2026. The requirements above implement them.

- **D1. A failing `init.lua` keeps what already ran.** Statements before the error stay applied, the rest of the file doesn't run, and flux starts (Neovim's behavior, verified on 0.12.5). Implemented by R13, A4 and `partial_apply_on_error`.
- **D2. Stable `flux.*` items are frozen under semver for all of 1.x.** Additions are allowed in minor releases; no removals or meaning changes before 2.0. Anything not ready is documented as Experimental and may change in a minor release. Until 1.0, the M7 plan's "experimental, versioned by milestone" applies. Implemented by Appendix A's rules and `docs/config.md` (§ 10).
- **D3. `flux.cmd` is part of the API.** Implemented by R21, A10 and `cmd_runs_ex_and_raises_errors`.
- **D4. The Appendix A names and shapes are confirmed:** `"x"` as an alias of `"v"`, `flux.g.mapleader`/`maplocalleader`, `flux.lsp.config`/`enable` mirroring Neovim 0.11+, and `flux.highlight` mirroring `nvim_set_hl`. F-08–F-11 specify behavior within these shapes.
- **D5. `--clean` and `-u FILE|NONE` ship in this PR** (moved from F-22). Implemented by R9–R11, A7 and `parses_clean_and_u`.
