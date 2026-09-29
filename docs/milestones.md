# Milestones

Each milestone ends with a runnable editor and a manual check before the next one starts.

## M0: Skeleton ✅

Open a file read-only, move around it, and quit, with screen output matching Neovim.

- Workspace, dual MIT/Apache license, CI (fmt, clippy, tests on Linux and macOS, cargo-deny,
  Neovim oracle check).
- Text loading the way Vim counts lines: final newline, `[noeol]`, CRLF (`[dos]`) files, invalid
  UTF-8 flagged.
- Screen layout: tabs, `^X` / `<hex>` for unprintable characters, wide characters and emoji,
  soft wrap with `>` fillers, `@@@` for a last line that doesn't fit, `~` past the end.
- Neovim's default statusline (`name … line,col[-vcol]  Top/Bot/All/NN%`) and command line.
- Keys: `j` `k` `<Down>` `<Up>` `CTRL-N` `CTRL-P` `CTRL-J`, `gg` `G`, `CTRL-E` `CTRL-Y`,
  `CTRL-D` `CTRL-U`, `CTRL-F` `CTRL-B` `<PageDown>` `<PageUp>`, `CTRL-L`.
- Command line: typing, `<BS>` (leaves on empty), `CTRL-W`, `CTRL-U`, `<Esc>`, `CTRL-C`.
- Ex: `:q` `:q!` `:qa` `:quitall` and abbreviations, `:N` to jump to a line, E492/E488 errors.
- Scrolling follows Neovim 0.12 exactly, including wrapped lines: when to scroll minimally vs
  recenter, where `CTRL-D`/`CTRL-U` leave the cursor inside wrapped lines, and keeping the cursor
  at the same relative height across terminal resizes.

Verified: 38 oracle cases match Neovim, and 22 key sequences plus 7 resize sequences produce
screens identical to `nvim --clean` in an 80x24 terminal.

### Manual check

Run `cargo run -- <some file>` (a long source file with tabs and long lines is best)
and, ideally, `nvim --clean <same file>` next to it:

1. The first screen matches Neovim's: text, `~` rows, statusline, cursor.
2. `j`/`k`, `gg`/`G`, `CTRL-D`/`CTRL-U`, `CTRL-F`/`CTRL-B`, `CTRL-E`/`CTRL-Y` move and scroll the
   same way. Try them across long wrapped lines.
3. `:42<CR>` jumps to line 42 and leaves `:42` on the command line; `:bogus<CR>` shows E492 in red.
4. Resize the terminal: the cursor stays at the same relative height, nothing is garbled.
5. `:q` exits and your shell is back to normal. `flux` with no file shows `[No Name]`; a missing
   file opens empty.
6. Wide characters, emoji and tabs line up; `CTRL-L` redraws cleanly.

Known gaps, by design until later milestones: no horizontal movement or editing (M1), one file
only (M3), no syntax colors (M5).

## M1: Core editing ✅ (awaiting manual check)

Normal and Insert modes with Vim's grammar, undo, `.`, registers for yank/put, and saving.

- Motions: `h j k l`, `<Space>`/`<BS>` (wrap), `0 ^ $ |`, `+ - _ <CR>`, `w W b B e E ge gE`,
  `f F t T ; ,`, `%` (and `N%`), `{ }`, `gg G`, `H M L`, with counts.
- Operators `d c y > < gu gU g~` with any motion or doubled (`dd`, `>>`, `gUU`); counts on both
  sides multiply (`2d3w`). Vim's range rules: exclusive/inclusive/linewise, `:h exclusive-linewise`,
  `cw` as `ce`, `dw` stopping at the end of the line, deletes that become linewise.
- `x X D C s S Y` (Neovim's `Y` = `y$`), `p P` with counts, `r` (including `r<CR>`), `J gJ`, `~`,
  `i a I A gI o O` with counts, `u`, `CTRL-R`, `.` with a new count, `CTRL-G`, `ZZ`, `ZQ`.
- Insert mode: typing, `<CR>` with autoindent (and dropping unused autoindent), `<BS>` over
  line breaks, the insert start and 'smarttab' indent, `<Del>`, `CTRL-W`, `CTRL-U` (stopping once
  at the insert start), `<Tab>`, `CTRL-T`/`CTRL-D`, `CTRL-V`, `CTRL-R {reg}`, `CTRL-O`, arrow keys
  (which split undo and restart `.`), `<Esc>`/`CTRL-C`. An unmapped Alt key is `<Esc>` + key.
- Registers `""`, `"0`, `"1`–`"9` (shifting), `"-`, `"a`–`"z` (and `"A`–`"Z` to append), `"_`.
- Undo tree (`u`/`CTRL-R`; new changes after undo branch), Vim's cursor placement after
  undo/redo, and its `1 change; before #3  2 seconds ago` messages.
- Ex: `:w [file]`, `:w!`, `:wq`, `:x`, `:up`, `:wa`, `:xa`, `:e [file]`, `:e!` (undoable, like
  'undoreload'), `:q` (E37/E162 with unsaved changes), `:qa`, `:checktime`, `:N`.
- Saving is atomic (write a temp file, then rename), keeps permissions, writes through symlinks,
  keeps CRLF files CRLF and adds a missing final newline ('fixeol'). It refuses (without `!`) if
  the file changed on disk since it was read. When the terminal regains focus, a changed file is
  reloaded if there are no unsaved changes ('autoread'), otherwise W12 is shown.
- Screen: `[+]`, `-- INSERT --`, `-- (insert) --`, showcmd, Vim's report messages (`3 fewer
  lines`, `4 lines yanked`, …), message truncation (`<…` for file messages, `...` otherwise), and
  the hit-enter prompt for messages longer than a line.

Verified: all 341 M0/M1 oracle cases match Neovim 0.12.5, and 62 key sequences produce screens
(and saved files) identical to `nvim --clean` in an 80x24 terminal. A 200,000-line (10 MB) file
opens in 0.3 s and every tested command responds in under 30 ms.

### Manual check

Open a real source file with `flux <file>` (ideally side by side with `nvim --clean <copy>`):

1. Edit as you normally would (text objects like `ciw` are M2): `dw`, `cw`, `d}`, `3dd`, `yyp`,
   `xp`, `ddp`, `J`, `>>`, `.` with and without counts, `u` and `CTRL-R` several times.
2. Insert mode on indented code: `o`/`O`/`<CR>` keep the indent, `<Esc>` on an empty new line
   drops it, `<BS>` removes a whole indent level, `CTRL-W`/`CTRL-U`, arrow keys then `u`.
3. `:w`, then `:q`; make a change and try `:q` (E37/E162 prompt), `:wq`, `ZZ`.
4. Change the file in another editor while flux has it open, focus flux again: it reloads. Make
   an unsaved change first and it warns (W12) instead; `:w` then refuses without `!`.
5. Save a CRLF file and a file without a final newline and check the bytes (`xxd`).

Known gaps, planned for later milestones: Visual mode, text objects, macros, marks and the
jumplist (M2); multiple buffers/windows (M3); search, `:s` and Ex ranges (M4); syntax colors (M5).
Smaller ones not yet scheduled: `R` Replace mode, `CTRL-A`/`CTRL-X`, `gj`/`gk`, `gp`/`gP`,
joining comment lines ('formatoptions' `j`), `%` skipping brackets in quotes, undo messages for
changes older than 100 seconds (Vim shows a clock time), and swap files. Saving replaces the file
with a new one, so hard links to it are not updated.

## M2: Visual mode, text objects, registers

## M3: Windows and buffers

## M4: Search and Ex

## M5: Syntax highlighting

## M6: LSP

## M7: Picker, Lua config, clipboard
