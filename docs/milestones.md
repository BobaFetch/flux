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

## M3: Windows and buffers ✅

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

## M4: Search and Ex ✅

- Vim patterns, translated to the `regex` crate: all four magic levels (`\v \m \M \V`),
  `\c`/`\C`, 'ignorecase' and 'smartcase', `\< \>`, `\zs \ze`, `\{n,m}` and `\{-}`, groups
  and alternation, collections (with `[:alpha:]` classes and `\_[...]`), `\s \d \w \a \l \u
  \x \k \f …`, line breaks (`\n`, `\_s`, `\_.`) and `~`.
- `/` and `?` with offsets (`e`, `s`, `b`, `+N`), `//` and `??`, `n` `N` `*` `#` `g*` `g#` with
  counts, as motions after operators and in Visual mode, 'wrapscan', and Neovim's messages:
  `/pattern  [3/17]` (with `W` after wrapping), E486, E384/E385, E35. Searches are jumps, fill
  `"/`, and are in the history.
- 'hlsearch' (with CurSearch for the match under the cursor), `:noh`, 'incsearch' while
  typing a search or the pattern of `:s`, and Neovim's default 'inccommand': `:s` is previewed
  in the buffer as you type it.
- The command line edits like Vim's: cursor keys, `<S-Left>`/`<S-Right>`, `CTRL-B`/`CTRL-E`,
  `<Del>`, `CTRL-V`, `CTRL-R`, history on `<Up>`/`<Down>` (matching what's typed) and
  `CTRL-P`/`CTRL-N`; a long command line wraps.
- Ex ranges: `.` `$` `%` `*` numbers, marks, `/pat/` `?pat?` `\/` `\?` `\&`, `+N`/`-N`, `,` and
  `;`, a count before `:` (`3:` → `:.,.+2`), E16/E481, and the "Backwards range given, OK to
  swap (y/n)?" question.
- `:s` with flags `& c e g i I n p # l r` and a count, `&` `\0`–`\9` `\u \U \l \L \e \E` `\r`
  `\n` `\t` `~` in the replacement, patterns that join lines, confirmation (`y n a q l`,
  `CTRL-E`/`CTRL-Y`), Vim's cursor, `'[`/`']`, messages and undo; `:&`, `:&&`, `:~`, and `&`/`g&`
  in Normal mode.
- `:g` and `:v` (any command, default `:p`), `:d`, `:y`, `:>`, `:<`, `:m`, `:t`/`:co`, `:j`,
  `:pu`, `:norm`, `:p`, `:nu`/`:#`, `:=`, `:k`/`:mark`, `:u`, `:red`, and `:[range]w` for part
  of the buffer.
- `:set`, `:setlocal`, `:setglobal` (`opt`, `noopt`, `invopt`, `opt!`, `opt&`, `opt?`, `=` `+=`
  `-=` `^=`) for the options flux implements: 'autoindent' 'expandtab' 'gdefault' 'hidden'
  'hlsearch' 'ignorecase' 'incsearch' 'number' 'numberwidth' 'relativenumber' 'report'
  'shiftwidth' 'smartcase' 'smarttab' 'softtabstop' 'splitbelow' 'splitright' 'tabstop'
  'wrapscan', each with Vim's scope (buffer and window options are local). Others give E518.
- Long output is paged with `-- More --` (`<Space> <CR> d b u k g G q`), quitting early stops
  a `:g` where Vim would.

Verified: all 1070 M0–M4 oracle cases match Neovim 0.12.5 (347 new: searches, ranges, every
line command, `:s` and `:g` including confirmation and undo, options), 19 new key sequences
produce screens identical to `nvim --clean` (plus the 144 earlier ones), and highlight
positions (hlsearch, incsearch, the preview, confirmation) match cell for cell. On a
200,000-line file, a failing search takes 0.04 s, `:%s/alpha/A/g` 0.4 s, `:g/beta/d` 1.3 s
and `:g/^/m0` 1.2 s (Neovim: 50 s).

### Manual check

This is the dogfooding point: try editing flux's own source with flux.

1. Search: `/fn `, `n`/`N`, `*` on a name, `/foo/e`, `?`, a pattern with `\<` and `\v`,
   `d/pat<CR>`, and `v` + `/pat<CR>`. Watch hlsearch and incsearch while typing; `:noh`.
2. `:s`: `:%s/old/new/g`, with `c` to confirm, with `\(\)`/`\1`, `\u&`, `\r`; `u` afterwards;
   `&`, `g&`, `:&&`. Watch the preview as you type.
3. Ranges and line commands: `:.,+3d`, `:'<,'>>`, `:/start/,/end/y`, `:m0`, `:t.`, `:g/TODO/`,
   `:g/^$/d`, `:v/./d`, `:5,10norm A;`.
4. `:set nu rnu`, `:set ts=4 sw=4 et`, `:set ic scs`, `:setlocal sw=2` in one of two windows;
   `:set sb spr` and split.
5. Command line: `<Up>` after typing a prefix, cursor keys, `CTRL-R {register}`, and a long
   `:g/pat/` listing paged with `-- More --`.

Known gaps, for later: `\@` lookaround and back-references in patterns, `\%V`/`\%23l` and other
position items, `\=` expressions in `:s`, `/pat/;/pat2/`, `|` between commands, Tab completion
on the command line (it arrives with the pickers in M7), the command-line window (`q:`), and
options not in the list above ('wrap', 'scrolloff', 'list', …). `:set` alone lists only the
options above, so it shows less than Neovim's. A long command line typed at a prompt over an
earlier multi-line message scrolls a little differently from Neovim.

## Directory browsing (between M4 and M5) ✅

A small netrw: `flux .`, `flux some/dir`, `:e dir`, `:sp dir` and `:vs dir` show a listing of
the directory: `../`, then directories (marked `/`, in the Directory color), then files, each
sorted by name.

- In a listing: `<CR>` opens the file or directory under the cursor (`../` goes up), `-` goes
  up with the cursor on the directory just left, `o` / `v` open the entry in a new window.
  Every other Normal-mode key (`j`, `/pat`, `G`, `CTRL-^`, …) works as usual.
- `:Explore [dir]` (`:Ex`, `:E`) lists the current file's directory with the cursor on the file;
  `:Sexplore` / `:Vexplore` do it in a new window.
- Listings are read-only (E21 for any change, E502 for `:w`), read again each time they are
  shown and when the terminal regains focus, and not listed by `:ls` / `:bn`.

Not netrw's: no banner, no file operations (`%`, `d`, `D`, `R`), no sorting or hiding options,
no tree view. Fuzzy file finding is still M7.

### Manual check

1. `flux .` in a project: the listing shows; `j`/`k`/`/name` move; `<CR>` on a directory and on
   a file; `-` back up lands on the directory you left.
2. From a file, `:Ex` shows its directory with the cursor on the file; `CTRL-^` returns to it.
3. `:Vex`, then `o` on a file; `v` on another.
4. `dd`, `x`, `i`, `p` in a listing give E21; `:w` gives E502.

## M5: Syntax highlighting, indenting, filetypes ✅

- Tree-sitter highlighting (new `flux-syntax` crate) for Rust, C, Lua, Python, JavaScript (with
  JSX), TypeScript, TSX, JSON, TOML, Bash and Markdown, with the queries Neovim uses (its own
  for C, Lua and Markdown; nvim-treesitter's for the rest, vendored with a NOTICE). Neovim's
  query extensions work: `#lua-match?`, `#contains?`, `#has-ancestor?`, `#has-parent?`,
  `#kind-eq?`, `#set!` (priority, capture values), `#offset!`. Injections are highlighted too:
  Markdown's inline text and fenced code, Rust macro arguments, regexes and so on.
- Parsing is incremental: `Text` logs its edits in bytes for tree-sitter. A parse gets 20 ms
  per frame and resumes between keys, so a big file opens at once and an unclosed bracket
  doesn't freeze typing; meanwhile the old tree, moved along with the edits, keeps the colors
  roughly in place.
- Neovim's default colorscheme, generated from Neovim (`cargo xtask colors gen`), for
  'background' dark and light, in 24-bit color ('termguicolors', on when `$COLORTERM` says so,
  as in Neovim) or 16 colors. Every screen element now uses its Neovim highlight group
  (Normal's background, StatusLine, LineNr, NonText, SpecialKey, Visual, Search, CurSearch,
  IncSearch, Substitute, ErrorMsg, ModeMsg, MoreMsg, Question, Directory, …), and Visual mode
  is drawn like Neovim's (the cell under the cursor and line breaks aren't highlighted). Links
  in Markdown are OSC 8 hyperlinks, as Neovim makes them.
- MatchParen, like Neovim's matchparen plugin: the bracket under (or, in Insert mode, before)
  the cursor and its match within the window, skipping brackets in strings and comments.
- Filetype detection (extensions, file names, `#!` lines), `'filetype'`, `:set ft=`, and what
  Neovim's ftplugins and indent scripts set per filetype: 'tabstop' 'shiftwidth'
  'softtabstop' 'expandtab' 'textwidth' 'matchpairs' 'comments' 'formatoptions'
  'indentexpr' 'indentkeys' 'cindent' 'cinoptions' 'cinkeys' 'cinwords'. `:filetype [plugin]
  [indent] on|off|detect`, `:syntax on|off`.
- Indenting like Neovim: Vim's C indenter (cindent, with 'cinoptions') and ports of Neovim's
  indent scripts for Rust, Python, Lua, sh, JavaScript, TypeScript and JSON. Lines are
  reindented on `<CR>`, `o`, `O`, `CTRL-F` and the keys in 'indentkeys' (`}`, `else`, `end`,
  `:` …), and with the `=` operator (`==`, `=ip`, `gg=G`, Visual `=`).
- Comments: `<CR>`, `o` and `O` continue the comment leader ('formatoptions' `r` and `o`, Vim's
  `open_line`), including three-part comments (`/*`, ` * `, ` */`, typing `/` to end one), a `//`
  comment after code, and the offsets and flags in 'comments'. `J` removes leaders
  ('formatoptions' `j`). Typing past 'textwidth' wraps comments (`c`) and text (`t`) at the last
  blank, continuing the leader; `l` keeps long lines. An automatic indent or leader that
  nothing was typed after loses its trailing blanks on `<Esc>`.

Verified:
- All 1109 M0–M5 oracle cases match Neovim 0.12.5 (39 new: comment leaders, `fo-j`, auto-wrap,
  `x`-ended comments).
- The new indent corpus (62 files, from small topic files to a 969-line flux source file) gives
  Neovim's results both reindented with `gg=G` and typed line by line: 124 of 124
  (`cargo xtask indent gen|check`).
- `cargo xtask screens` (new) opens 26 files in Neovim and flux side by side in tmux and compares
  every cell's character, colors, attributes and hyperlink, and the cursor: all identical, in
  24-bit and in 16 colors. Neovim gets flux's parsers (built from the same grammar crates) and
  queries, so every language is compared, not just the ones Neovim ships.
- In a 200,000-line Rust file (release build): the first parse takes 0.56 s (in the
  background), typing code takes 8 ms a key on average (worst 23 ms), `=` 0.26 ms a line.

### Manual check

1. Open a few files of different languages (`flux crates/flux-view/src/editor.rs`, a Python
   script, a README): colors and background look like `nvim --clean` on the same file (Neovim
   only uses tree-sitter for Lua and Markdown by default; for the others compare the general
   look). A Markdown file shows code blocks highlighted and links you can click.
2. Type a Rust function from scratch: `fn main() {<CR>`, a `vec![` over several lines, `}`;
   indents follow, and `}` / `]` snap back. Do the same in Python (`if x:`, `else:`), Lua
   (`function … end`) and a shell script (`if …; then … fi`).
3. `gg=G` on a badly indented file; `=ip`; `==`.
4. In Rust: `// comment<CR>` continues the comment; `o` on a comment line does too; type a
   long comment past column 100 and it wraps. `/*<CR>` in C gives ` * `; typing `/` on the
   next line ends it. `J` on two comment lines drops the second `//`.
5. Put the cursor on a bracket: it and its match light up; move into Insert mode after a `)`.
6. `:set bg=light`, `:set notgc`, `:syntax off` / `on`, `:set ft=python` on a `.txt` buffer,
   `:filetype`.
7. Open a very large file: it shows at once, colors follow shortly.

Known gaps:
- Highlighting is tree-sitter for every filetype, while `nvim --clean` uses Vim's regex
  syntax files except for Lua and Markdown, so colors of other languages match Neovim with
  tree-sitter (nvim-treesitter's queries), not `nvim --clean` exactly. Indenting, which in
  Neovim asks the regex syntax about strings and comments, asks the tree-sitter tree instead;
  the corpus shows no difference, but unusual code may.
- No `'background'` detection (Neovim asks the terminal): set `:set bg=light` for a light
  terminal. `:syntax off` turns off all highlighting (in Neovim it leaves tree-sitter
  highlighting alone). Only `default` colors; `:colorscheme` and `:highlight` come with the
  Lua config (M7). `.h` files are C++ (as in Neovim 0.12), for which flux has no parser.
- Not ported: 'smartindent', 'lisp', `gq` and 'formatexpr', numbered lists ('formatoptions'
  `n`), Vim's `b:js_cache`, and tree-sitter's indent queries. `gc` (Neovim's commenting) isn't
  there yet.
- Found by the new screen comparison, from earlier milestones: in Visual mode Neovim lets the
  cursor go past the last character (`v$`, `l` at the end of a line); flux keeps it on the
  last character, so the cursor and ruler differ there (what operators do is the same).
  `:resize` in a window that has only vertical splits makes Neovim give the room to the
  command line; flux doesn't.

## M6: LSP

- Language servers (new `flux-lsp` crate), started per filetype and project root like Neovim's
  `vim.lsp.enable` configs: rust-analyzer, clangd, lua-language-server, basedpyright or
  pyright, typescript-language-server, bash-language-server, taplo,
  vscode-json-language-server and marksman, each only when it's installed. `$FLUX_LSP_CONFIG`
  names a JSON file with a list of configs to use instead. Documents are synced
  incrementally (in UTF-8, UTF-16 or UTF-32 positions, as the server prefers); saves and
  buffer deletes are reported; servers are shut down on quit. The server log is
  `~/.local/state/flux/lsp.log`.
- Diagnostics as Neovim shows them: signs in the sign column (`'signcolumn'`), underlines
  colored by severity, `E:1 W:2` in the statusline, held back while in Insert mode, and kept
  in place as the text is edited. `]d` `[d` `]D` `[D` jump, `CTRL-W d` opens a float with
  the diagnostics under the cursor.
- `K`: hover in a Markdown float, with code highlighted and Markdown markup hidden
  (`'conceallevel'` 2, as in Neovim's floats), the hovered symbol highlighted.
- Locations: the quickfix list and location lists, with Neovim's list window (`:copen`,
  `:cclose`, `:cwindow`, `:cc`, `:cnext`, `:cprevious`, `:cfirst`, `:clast`, `:cnfile`,
  `:cpfile`, the `:l…` versions, `:botright`), `]q` `[q` `]Q` `[Q` `]l` `[l` …, `<CR>` in the
  list. `grr` references, `gri` implementation, `grt` type definition (one result jumps,
  several fill the quickfix list), `gO` document symbols (location list), `CTRL-]` definition
  through the tag stack, `CTRL-T`, `:pop`, `:tags`.
- Completion: Vim's popup menu (`'completeopt'`, `'pumheight'`, `'pumwidth'`) with the info
  window for documentation. `CTRL-X CTRL-O` asks the servers (Neovim's LSP omnifunc:
  filtering, resolve for documentation, additional edits on accept, snippets with tabstops,
  placeholders in Select mode, `<Tab>`/`<S-Tab>`, mirrors and choices). `CTRL-N`/`CTRL-P`
  complete keywords from the buffers. `CTRL-Y`, `CTRL-E`, `<CR>`, `<BS>`, narrowing as you
  type, and the mode messages are Vim's.
- `grn` rename (with prepareRename and an `input()` prompt), `gra` code actions (Normal and
  Visual, Vim's numbered list, resolve, commands), Insert-mode `CTRL-S` signature help with
  the active parameter highlighted.
- `gq` and `gw`: Vim's formatting (comment leaders, 'formatoptions' `2 n w p 1`, numbered
  lists, nroff paragraphs), or the server's range formatting when it has one, as Neovim's
  'formatexpr' does.
- Semantic tokens: `@lsp.type.*`, `@lsp.mod.*` and `@lsp.typemod.*` highlights over
  tree-sitter's, full, delta and range requests, following edits until the next answer.
- `:lsp enable|disable|restart|stop [name …]`.

Verified:
- All 1151 M0–M6 oracle cases match Neovim 0.12.5 (42 new, all `gq`/`gw`); 12 cases recorded for later milestones were skipped.
- `cargo xtask screens` runs a scripted fake language server (`flux-lsp-fake`) under both
  Neovim and flux for the LSP samples and compares the screens cell by cell: 120 samples (94 new),
  all identical in 24-bit and 16 colors (diagnostics, floats, hover, quickfix and the LSP
  location commands, the popup menu and snippets, rename, code actions, formatting,
  signature help, `:lsp`, semantic tokens).
- With the fake server logging, flux sends the same messages in the same order as Neovim
  (initialize, didOpen, didChange, didSave, the requests, shutdown, exit).

### Manual check

1. In `~/Projects/flux`, `flux crates/flux-view/src/editor.rs`: rust-analyzer starts (after
   it finishes loading, diagnostics appear). Add an error (`let x: u32 = "a";`): an `E` sign,
   a red underline and `E:1` in the statusline after leaving Insert mode. `]d`, `[d`,
   `CTRL-W d`.
2. `K` on a function or type: a float with its signature highlighted and docs; moving closes
   it.
3. `CTRL-]` on a call goes to its definition, `CTRL-T` comes back. `grr` on a function opens
   the quickfix list; `<CR>` on an entry, `:cnext`, `:cclose`. `gO` lists the file's symbols.
4. In Insert mode, type `editor.` and `CTRL-X CTRL-O`: the menu with documentation beside it;
   `CTRL-N`, `CTRL-Y`. Accept a function with arguments: `<Tab>` moves through them.
   `CTRL-N` alone completes words from the buffer.
5. `grn` on a local variable, type a new name, `<CR>`: every use changes. `gra` on the error
   from step 1: pick an action.
6. Inside a call's parentheses in Insert mode, `CTRL-S`: the signature with the current
   argument highlighted.
7. `gqip` on a long comment paragraph (in a `.txt` or Markdown file it formats with
   'textwidth'; in Rust rust-analyzer formats the range).
8. Semantic colors: after rust-analyzer loads, names change color slightly (e.g. mutable
   variables underlined, as in Neovim).
9. `:lsp restart`, `:lsp stop`, `:lsp enable`. Also try a Python, TypeScript or Lua file if
   those servers are installed.

Known gaps:
- Diagnostic signs currently expose only a red `E`/`W` and status counts by default; improve the
  default diagnostic detail and make the severity, message, code and source easier to discover.
- Floats can't be focused (`KK` doesn't enter the hover window; `CTRL-S` twice doesn't cycle
  signatures in it).
- Progress (`$/progress`) isn't shown, as in Neovim 0.12 by default.
- Not done: `:clist`, `:colder`/`:cnewer`, `:tnext`/`:tselect` (several definitions go to the
  first), 'winfixheight' for the list window; `'completeopt'` `fuzzy`, `longest` and
  `preview`, other `CTRL-X` modes, 'infercase', running a completion item's command; change
  annotations that need confirmation; 'formatoptions' `m`/`M`, 'formatprg', a user
  'formatexpr'; `input()` history; `:lsp` completion; inlay hints, code lenses, document
  highlight, workspace symbols, call hierarchy (not default keys in Neovim either).
- When several keys arrive at once, Neovim skips redraws in between; flux redraws after
  each, so a fast typist can briefly see states Neovim never draws.

## M7: Picker, Lua config, clipboard

- Deferred oracle cases now run: six `:g`/`:v`/`:norm` cases were retagged M4 (that milestone implemented them), so 1157 cases match. Six are deferred and checked to still differ: visual-block ×4, `CTRL-A`, `das`.
- Add TOML configuration for static editor preferences and the planned Lua API for programmable
  configuration such as behavior, commands and keymaps.
- Make LSP completion optionally automatic while typing, like VS Code or Zed, while preserving
  explicit Vim-style `CTRL-X CTRL-O` completion.
- `:Files` skips gitignored files inside git repos and `.ignore` matches, and marks a
  truncated list with `N+`.

### Manual check

1. After `cargo test --workspace` (so `target/` exists), `cargo run -- .`, then
   `:Files oracle.lua` and Enter. `xtask/oracle.lua` is the selected entry. Clear the
   query with `CTRL-U`: no `target/` path, and no truncation marker (this tree has 657
   tracked non-hidden files, equal to `git ls-files | grep -v '^\.' | wc -l`; the spec's
   "about 646" was an older checkout).
2. A non-git directory with `big/` (6000 files) and `small/{a,b,c}`, cap 5000. `:Files`
   shows the marker `5000+`. `:Files small/` matches `small/a`, `small/b`, and `small/c`.
