# Milestones

Each milestone ends with a runnable editor and a manual check before the next one starts.

## M0: Skeleton ✅ (awaiting manual check)

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

## M1: Core editing

Normal and Insert modes, the operator/motion state machine, counts, undo/redo, dot-repeat,
`:w`/`:e`. 42 oracle cases are waiting on this milestone.

## M2: Visual mode, text objects, registers

## M3: Windows and buffers

## M4: Search and Ex

## M5: Syntax highlighting

## M6: LSP

## M7: Picker, Lua config, clipboard
