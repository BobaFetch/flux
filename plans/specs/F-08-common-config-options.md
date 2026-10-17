# F-08 — Common config options (`scrolloff`, `wrap`, `list`/`listchars`, `cursorline`, `colorcolumn`, `mouse`, `clipboard`, `swapfile`)

| | |
|---|---|
| Backlog | F-08 (M7 Stage C, second of the five Stage C PRs after F-07; PRD §2.2 "Options people put in configs", `plans/flux-1.0-prd.md:67`). Unblocks F-14 (`'swapfile'`), F-16 (`'clipboard'`), F-20 (`'mouse'`) |
| Size | M (most of it is `nowrap` horizontal scrolling; see §6) |
| Spec status | **Draft**, Oct 16, 2026. Becomes Ready after the Neovim 0.12.5 check in R14 is recorded on yonaka. Mouse default decided by the user Oct 16, 2026 (D1) |
| Repo state | `BobaFetch/flux` @ `d050d6f` (main), read Oct 16, 2026 |
| Kind | Feature: new options in the option table, `:set`/`flux.opt` validation, and drawing/scrolling behavior for five of them |
| Owner | Lightning (option table, `:set` parsing, window/scroll rules in `flux-view`), Yuna (`flux-tui` drawing: `list`, `cursorline`, `colorcolumn`, `nowrap` rendering), Tifa (error and help copy), Fina (verification gate) |
| Sector 5 routing | Y'shtola schedules → Lightning (options + scrolling) → Yuna (drawing) → Tifa (copy) → Fina verifies on Muse Spark 1.3 high → PM merges after Fina's approval and green CI. `nowrap` may ship as its own PR (§6) |

## 1. Problem

The first lines of most users' configs fail today. `set scrolloff=8`, `set nowrap`, `set list`, `set cursorline`, `set colorcolumn=80`, `set mouse=a`, `set clipboard=unnamedplus` and `set noswapfile` all give `E518: Unknown option` (`crates/flux-vim/src/set.rs:141,144`), because the option table (`crates/flux-view/src/options.rs:186–221`) has none of them. With F-07, the same lines in `init.lua` (`flux.opt.scrolloff = 8`) fail the same way and abort the rest of the file at that point (F-07 D1).

Current behavior these options touch:
- **Scrolling** follows Neovim's `update_topline` with `'scrolloff'` hard-wired to 0 and `'scrolljump'` 1 (`window.rs:336`).
- **Wrapping** is always on. Distances are counted in screen rows and wrapped lines use `w_skipcol` (`window.rs:1–5,33`). There is no left column (`leftcol`), so `nowrap` has nothing to build on.
- **`cursorline`**: `draw.rs:391` says "CursorLineNr is only for 'cursorline', which flux doesn't have."
- **`list`/`colorcolumn`**: no drawing support and no `NonText`/`Whitespace`/`ColorColumn` highlight use in the window body (`theme.rs`).
- **Mouse**: no mouse capture in `flux-tui`; the terminal's own selection works.

## 2. Goal

Every option in the F-08 row is accepted by `:set`, `:set {opt}?`, `:set {opt}&`, `:set all` and `flux.opt` with Neovim's names, short names, scope, defaults (except D1) and error messages. Five of them behave like Neovim. Three are accepted and validated now and get their behavior in later items.

## 3. Non-goals

- Mouse behavior (click, focus, wheel, drag-select): F-20. Here `'mouse'` is only stored and validated.
- Clipboard sync for `unnamed`/`unnamedplus`: F-16. `+`/`*` registers already exist and are unchanged.
- Swap files and recovery (`-r`): F-14. Here `'swapfile'` is only stored; no file is written.
- `'sidescroll'`/`'sidescrolloff'`, `'linebreak'`/`'breakindent'`/`'showbreak'` in normal windows, `'smoothscroll'`, `'cursorcolumn'`, `'cursorlineopt'`. Possible follow-ups; not in the PRD list. (`sidescroll` behavior is fixed at Neovim's default 0, see R6.)
- `:highlight` / colorschemes (F-11). F-08 uses the default theme's groups.

## 4. Options

| Option | Short | Kind | Scope (Neovim) | Default | F-08 behavior |
|---|---|---|---|---|---|
| `scrolloff` | `so` | number | global-local to window | `0` | **Real** (R4) |
| `wrap` | — | bool | window | on | **Real** (R5–R6) |
| `list` | — | bool | window | off | **Real** (R7) |
| `listchars` | `lcs` | string | global-local to window | `tab:> ,trail:-,nbsp:+` | **Real** (R7) |
| `cursorline` | `cul` | bool | window | off | **Real** (R8) |
| `colorcolumn` | `cc` | string | window | `""` | **Real** (R9) |
| `mouse` | — | string | global | **`""` (flux, D1)**; Neovim `nvi` | Validated only → F-20 |
| `clipboard` | `cb` | string | global | `""` | Validated only → F-16 |
| `swapfile` | `swf` | bool | buffer | on | Validated only → F-14 |

Defaults and scopes above are from Neovim's `:help` and must be confirmed against Neovim 0.12.5 (R14). Global-local options may be implemented as plain window options for 1.0 if `:setlocal`/`:setglobal` distinctions are not otherwise supported; record that as a deviation if so.

## 5. Requirements

### Option table and `:set`
- **R1.** Add the nine options to `OPTIONS` in `options.rs` with the names, short names, kinds and scopes in §4, and their state in the global/window/buffer option structs. `:set all` lists them in its usual order.
- **R2.** `:set`, `:set {opt}?`, `:set {opt}&`, `:set no{opt}`/`inv{opt}`/`{opt}!` (bool), `+=`/`-=`/`^=` (string and number) and `flux.opt.{name}` work for all nine, with the same semantics F-07 R19 gives existing options.
- **R3. Validation**, with Neovim's error texts (confirmed in R14):
  - `scrolloff`: negative → `E487: Argument must be positive`; a non-number → `E521: Number required after =`.
  - `listchars`: only the keys `eol`, `tab`, `space`, `multispace`, `lead`, `leadmultispace`, `trail`, `extends`, `precedes`, `conceal`, `nbsp`; `tab:` takes two or three characters; every other key exactly one display-width-1 character; duplicate keys/bad shape → `E474: Invalid argument` (Neovim may report `E1511`/`E1512` for some shapes in 0.12; use whatever R14 records).
  - `colorcolumn`: a comma list of numbers and `+N`/`-N` (relative to `'textwidth'`) → otherwise `E474`.
  - `mouse`: flags from `a n v i c h r` only → otherwise `E474`.
  - `clipboard`: comma list of `unnamed`, `unnamedplus` (others Neovim accepts, e.g. `autoselect` variants, are rejected or ignored per R14) → otherwise `E474`.
  - A rejected value leaves the option unchanged.

### Behavior
- **R4. `scrolloff`.** Generalize `update_topline`, `scroll_cursor_top`/`_bot`, `CTRL-D`/`CTRL-U`/`CTRL-F`/`CTRL-B`, `zt`/`zb`/`zz`, `H`/`L`, search jumps and `:N` so that at least `scrolloff` screen lines stay above and below the cursor, as Neovim does (including the halfway rule when the window is shorter than `2*so+1`, and `so=999` keeping the cursor centered). Distances stay in screen rows with wrap on. Window-local value overrides global (`-1` means "use global" if the local/global split is implemented).
- **R5. `wrap` (on).** Today's behavior, unchanged. All existing scroll tests must pass with `wrap` and `so=0`.
- **R6. `nowrap`.** Lines are cut at the window's right edge. A per-window `leftcol` scrolls horizontally so the cursor stays visible; with Neovim's default `'sidescroll'=0`, a cursor that leaves the view recenters it horizontally (half a screen). `zh`/`zl`/`zH`/`zL`/`zs`/`ze` are **not** required here unless cheap (follow-up). Wide characters split by the left edge or right edge show as Neovim does (filler). With `list` on, `extends`/`precedes` characters mark cut lines. Vertical scrolling counts one row per line. Changing `wrap` re-scrolls the cursor into view. Visual selections, search highlights, diagnostics underlines, the sign/number columns, inlay/virtual text currently drawn by flux and the popup menu anchor all respect `leftcol`.
- **R7. `list`/`listchars`.** With `list` on, draw tabs with `tab:` (first char once, second repeated, optional third at the end), trailing whitespace with `trail`, `nbsp` for U+00A0/U+202F, `space`/`multispace`/`lead`/`leadmultispace` when set, and `eol` after the last character. With `list` on and no `tab:` key, tabs show as `^I` like Neovim. `eol`/`extends`/`precedes` use `NonText`; the rest use `Whitespace`. Cursor column and `curswant` must match Neovim with `list` on (tabs drawn as `^I` change widths).
- **R8. `cursorline`.** Highlight the cursor's screen line(s) in the current window with `CursorLine` (for a wrapped line, all its rows, per Neovim's default `cursorlineopt=both`) and the number column with `CursorLineNr`. Other highlights (Visual, search, diagnostics) draw on top as in Neovim. Non-current windows keep their own `cursorline` value but only the cursor line of each window is highlighted.
- **R9. `colorcolumn`.** Paint each listed screen column in `ColorColumn` on every text row, past end of line too, but not past the window's last text column. `+N`/`-N` are relative to `'textwidth'` and draw nothing when `textwidth=0`. With `nowrap`, columns are buffer-display columns (shift with `leftcol`).
- **R10. `mouse`, `clipboard`, `swapfile`.** Stored and validated only. `:set mouse=a` succeeds silently and has no effect yet. `:help` / docs mark them "accepted; behavior in F-20 / F-16 / F-14". No warning is printed (configs must load clean).
- **R11. Highlight groups.** Add `CursorLine`, `CursorLineNr`, `ColorColumn`, `NonText`, `Whitespace` to the default theme (`theme.rs`) for dark and light backgrounds, readable under `TERM=dumb`/`NO_COLOR` (F-02 rules: `cursorline` may become underline or nothing; `colorcolumn` may draw nothing when there's no color).

### Tests and verification
- **R12. Unit/integration tests:** option table round-trips (`:set x?` text for each option), every validation error, `scrolloff` scroll cases (cursor near top/bottom, `so=999`, short windows, wrapped lines), `nowrap` horizontal scrolling (long lines, wide chars, tabs, `$`, `0`, search jump off-screen), `list` rendering of each `listchars` key, `cursorline`/`colorcolumn` screen tests (`xtask screens`).
- **R13. Oracle cases** (Neovim 0.12.5 recorded): `scrolloff` motions, `nowrap` cursor/`leftcol` positions, cursor columns with `list` on. Tagged in-scope.
- **R14. Recorded Neovim check (blocks Ready).** On yonaka with Neovim 0.12.5 (`nvim --clean`): record `:set {opt}?` and `:verbose set {opt}?` for all nine, and the exact error text for each R3 invalid case, in `.sectorfive/evidence/F-08-nvim-check.md`. Update §4 and R3 from it, then mark this spec Ready.
- **R15.** Fina's gate: R12–R13 green, CI green on all four checks, a manual pass at 80×24 and 40×12 with a sample config using all nine options in `init.lua` and as `:set` lines (no errors, no prompts).

## 6. Split: `nowrap` may be its own PR

`nowrap` is most of the work: `leftcol` touches the window, scrolling, every drawing layer and the cursor/column model, while the other options are a day or two. Lightning may split F-08 into **F-08a** (option table and validation for all nine, `scrolloff`, `list`/`listchars`, `cursorline`, `colorcolumn`; `:set nowrap` accepted but drawn as wrap with no error) and **F-08b** (`nowrap` behavior, R6). If split, F-08a's docs say `nowrap` lands in F-08b, and F-08 is shipped only when both are merged. Record the split in `.sectorfive/decisions.md`.

## 7. Decisions

- **D1. `mouse` defaults to off (`mouse=`), like Vim.** Decided by the user Oct 16, 2026. This is a **deliberate deviation from Neovim** (`mouse=nvi`). It is documented in the feature matrix's "differences from Neovim" list and is part of the 1.x promise: the default won't change within 1.x. Users who want Neovim's behavior set `mouse=nvi` (or `a`) once F-20 ships. This resolves PRD open question 2 (mouse part). F-20 implements mouse behavior behind this default.
- **D2.** `mouse`, `clipboard` and `swapfile` are accepted now with no behavior, so configs written today keep working when F-14/F-16/F-20 land (no E518 in between).

## 8. Open questions

1. Implement the global-local split (`:setlocal`/`:setglobal`) for `scrolloff` and `listchars`, or treat them as window options for 1.0 and document it? Default: window options plus a documented deviation, unless `:setlocal` already distinguishes them.
2. `zh`/`zl`/`zs`/`ze` with `nowrap`: in F-08b or a follow-up? Default: follow-up.
3. Does `'swapfile'` default on (Neovim) given no swap files exist until F-14? Default: on, to match Neovim; it's inert until F-14.
