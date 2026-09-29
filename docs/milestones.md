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

## M1: Core editing ✅

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

## M2: Visual mode, text objects, registers ✅

- Visual mode `v` / `V`: every motion, `o`, `gv`, switching kinds, counts; operators `d x c s
  y > < ~ u U J gJ r`, the linewise `D X C S R Y`, and `p` / `P` replacing the selection (`p`
  sends the replaced text to the registers, `P` doesn't). `v$` takes the line break. `.` repeats
  a Visual operator on the same amount of text. `:` starts the command line with `'<,'>` (the
  range itself works in M4). The selection is highlighted, with its size in the showcmd area.
- Text objects, with counts, after operators and in Visual mode (where they grow the
  selection): `iw aw iW aW`, `i" a" i' a' i` a``, `i( a( ib ab`, `i[ a[`, `i{ a{ iB aB`,
  `i< a<`, `ip ap`, `it at`. Ported from Vim's `textobject.c`, including its white-space rules,
  multi-line blocks becoming linewise, and the between-two-strings quirk of `i"`.
- Registers: `"x` before any command; `"a`–`"z` and appending with `"A`–`"Z`; `"0`, `"1`–`"9`
  (multi-line deletes shift through them even into a named register), `"-`, `"_`; read-only
  `".` (last inserted text), `":` (last command line) and `"%` (file name). `"1p` then `.`
  puts `"2`, `"3`, …. `CTRL-R {reg}` in Insert mode and on the command line.
- Macros: `q{reg}` / `q` (with `recording @a` shown), `@{reg}`, `@@`, `@:`, counts, appending
  with `qA`. Stored as text like Vim, so `"ap` shows a macro and a yanked line can be run. A
  failing command stops the macro.
- Marks `m{a-zA-Z}`, `'x` / `` `x ``, `''` / ``` `` ```, and the automatic `'[ '] '< '> '. '^ '"`.
  Marks move with inserted and deleted lines; a mark on a deleted line is deleted (E20).
- Jumplist: `CTRL-O`, `CTRL-I` / `<Tab>` with counts; `G gg % { } H M L` and mark jumps add to
  it; `gi`.
- `:registers` / `:display`, `:marks`, `:delmarks`, `:jumps`, formatted like Neovim's.

Verified: all 548 M0–M2 oracle cases match Neovim 0.12.5 (register contents and types
included), and 126 key sequences produce screens and saved files identical to `nvim --clean`.
Fixed along the way: a crash when deleting every line from far down a long file (reachable with
`dG` in M1 too), and undo after an operator now returns to where Vim puts it.

### Manual check

1. Visual mode on real code: `vjjd`, `Vjj>`, `v$y`, `viwp` over another word, `gv`, `vip` then
   `J`, and `.` after a Visual operator.
2. Text objects: `ciw`, `daw`, `ci"`, `da(`, `di{` inside a multi-line block, `dap`, `cit` in
   HTML, `yi(` then `P`.
3. Registers: `"ayy`, `"Ayy`, `"ap`; delete a few lines then `"1p..`; `:reg`.
4. A macro: `qa` … `q`, `5@a`, `@@`; one that fails partway (runs off the end of the file).
5. Marks and jumps: `ma`, move, `'a` and `` `a ``, `G` then `CTRL-O` / `CTRL-I`, `:marks`,
   `:jumps`, `gi`.

Known gaps: visual-block mode (`CTRL-V`), `:normal` and `:g` (all later); sentence objects
(`is`/`as`) and `(`/`)`; clipboard registers `"+`/`"*` (M7); marks on other files (M3).

## M3: Windows and buffers ✅ (awaiting manual check)

- Windows: `:sp [file]`, `:vs [file]`, `:new`, `:vnew`, with a size (`:5sp`, `5:sp`) and the
  `:vert[ical]` modifier; `:close` (E444), `:only`, `:q` closing a window, `:resize [+-]N`,
  `:vertical resize N`. `CTRL-W` with `s S v n ^ c q o w W p t b h j k l` (and arrows), `+ - _`
  `< > |` and `=` with counts, `x`, `r R`, and `H J K L` to move a window to an edge.
- The layout is Vim's frame tree, ported from `window.c`: 'equalalways' sizing, the room check
  for a new window (E36), 'winheight'/'winwidth' applied to the window entered, closed windows
  giving their space to a neighbour, `CTRL-W x`/`r` keeping each window's size, and terminal
  resizes spread over the windows.
- Each window has its own statusline (the current one bold) and `│` separators, drawn like
  Neovim's default statusline, including how it is cut short in narrow windows.
- Buffers: 'hidden' behaviour, several files on the command line (the argument list, with
  `E173: N more files to edit` on the first `:q`), `:e #` and `CTRL-^` / `N CTRL-^`, `:b N` /
  `:b name` (E86, E93, E94), `:bn :bN :bp :bf :br :bl` with counts, `:bd[!]` / `:bw[!]` by
  number or name (E89), `:enew`, `:ls` / `:buffers` / `:files` with Neovim's flags and line
  numbers. Each window remembers where its cursor was in every buffer.
- Marks `A`–`Z` jump to other files; the jumplist crosses files; `:jumps` shows file names.
- `zt zz zb`, `z<CR> z. z-` with counts. `CTRL-D`/`CTRL-U`/`CTRL-F`/`CTRL-B` are now ported
  from Neovim's `pagescroll` (counts included: `N CTRL-D` sets 'scroll'), and a line taller than
  its window scrolls within itself so the cursor stays visible, marked `<<<` like Neovim.
- A count before `:` fills in a range (`3:` → `:.,.+2`); addresses `.`, `$`, `%`, numbers, marks
  and `+N`/`-N` work as counts and for `:N`. `:qa` now quits with any number of windows and
  `:q!` still refuses when a hidden buffer has changes, as in Vim. showcmd shows `^W`.

Verified: all 723 M0–M3 oracle cases match Neovim 0.12.5 (175 new, now including every
window's size, cursor, top line and `skipcol`), 18 multi-window key sequences produce screens
identical to `nvim --clean`, and the 126 earlier sequences still do. One known difference:
after `CTRL-D` in a split, Neovim sometimes leaves its terminal cursor a row above the cursor
line until the next key; flux draws it on the cursor line.

### Manual check

Run `flux file1 file2 file3` (ideally next to `nvim --clean file1 file2 file3`):

1. `:sp`, `:vs`, `CTRL-W s/v`, then move around with `CTRL-W h/j/k/l/w/p`. Resize with
   `CTRL-W +/-/</>/_/|/=` and `:resize`; resize the terminal too.
2. Edit the same buffer in two windows: both update, and each keeps its own cursor and scroll.
3. `:e file2`, `CTRL-^`, `:ls`, `:b 3`, `:bn`/`:bp`, `:bd`. Leave a buffer modified and switch
   away (allowed with 'hidden'), then try `:q` in the last window.
4. `CTRL-W x`, `r`, `H J K L`, `:only`, `:close`, `:q` until one window is left; `:q` warns
   about files not yet edited (E173) and quits on the second try.
5. `mA` in one file, `'A` from another; `CTRL-O` back across files.
6. A small window (`:resize 3`) over a file with very long lines: `$`, `j`/`k`, `CTRL-D`/`U`/`F`/`B`.

Known gaps: tab pages, `:sb`/`:sball`, `:args`/`:next`/`:prev` (the argument list only
affects `:q`), `'splitbelow'`/`'splitright'` and other options (`:set` is M4), `CTRL-W f`,
`CTRL-W ]`, and the mouse.

## M4: Search and Ex

## M5: Syntax highlighting

## M6: LSP

## M7: Picker, Lua config, clipboard
