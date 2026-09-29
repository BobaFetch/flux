# flux

A terminal text editor with Vim's editing grammar at its core, written in Rust.

flux aims to behave exactly like Vim (checked against Neovim) for the editing it supports, with
its own Lua API rather than Neovim plugin compatibility. It is early: see
[docs/milestones.md](docs/milestones.md) for what works today and what's next.

## Build and run

```sh
cargo run -p flux-term -- path/to/file
```

## Layout

| Crate | What it holds |
|---|---|
| `flux-core` | Text storage (rope) and how a line is laid out on screen. No IO. |
| `flux-view` | Editor state: buffers, the window, and Vim's scrolling rules. |
| `flux-vim` | The modal engine: keys in, state changes out. Key notation, Ex commands. |
| `flux-tui` | Drawing the editor into a cell grid and writing changed cells to the terminal. |
| `flux-term` | The `flux` binary: terminal setup and the event loop. |
| `xtask` | Project automation (`cargo xtask …`). |

## Checking behavior against Neovim

`crates/flux-vim/tests/oracle/cases.json` lists key sequences, each tagged with the milestone
that implements it. `cargo xtask oracle gen` runs them through `nvim --headless --clean` and
records the results in `expected.json`; the `oracle` test replays every case up to the current
milestone through flux and fails on any difference in text, cursor or scroll position.

```sh
cargo test -p flux-vim --test oracle   # flux vs the recorded Neovim results
cargo xtask oracle check               # recorded results vs your installed Neovim
cargo xtask oracle gen                 # re-record after adding cases
```

The expectations are recorded with the Neovim version in `oracle/NVIM_VERSION`; CI pins the
same version.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your
option. `cargo deny check` enforces a permissive-only dependency policy (`deny.toml`).
