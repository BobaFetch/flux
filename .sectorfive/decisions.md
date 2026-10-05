# flux — decisions
- 2026-10-03: Bootstrap baseline recorded (fmt/clippy/tests/indent/colors green; oracle-check blocked, see tech debt). Toolchain rustc 1.98.0; nvim pin v0.12.5.
- 2026-10-03: Ownership map drafted per Sector 5 roles (pending user approval). Principles: event acquisition (Aerith) vs key semantics (Lightning); recorded expectations owned by Fina, harnesses by Lightning; `gen` always planned + verified.
- 2026-10-03: Ownership map approved by user. Status is now binding for dispatch and review gates.
- 2026-10-03: M7 plan approved (`go`). Status active; executing strict A→B→C with per-stage Lightning→Aerith→Tifa→Fina gates.
- 2026-10-03: Stage A approved by Fina (1 veto issued + fixed in-turn: clipboard drain skipped on quit/EOF paths).
- 2026-10-03 (B scope): pickers trigger via Ex commands `:Files [query]` / `:Buffers [query]` (fzf.vim-standard names); dedicated keys deferred to Stage C maps (no key squatting). Tab completion renders as a Neovim wildmenu row (parity); pickers render as a centered flux-own window. Picker key logic stays Lightning-owned (engine); B2 is rendering only.
- 2026-10-03 (B design): no new Mode variant — picker rides Mode::CmdLine with `editor.picker` as discriminator (avoids cross-boundary edits; revisit with Stage C mode-maps). Ex-command candidates sort alphabetically (documented; revisit with screens evidence). Picker Tab/S-Tab move selection; Up/Down wrap. `:` cmdline only — `/`/`?`/`input()` keep literal Tab.
