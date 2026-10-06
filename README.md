# flux

A terminal text editor with Vim's editing grammar at its core, written in Rust.

flux aims to behave exactly like Vim (checked against Neovim) for the editing it supports, with
its own Lua API rather than Neovim plugin compatibility. It is early: see
[docs/milestones.md](docs/milestones.md) for what works today and what's next.

## Build and run

```sh
cargo run -- path/to/file                 # run from the repo
cargo install --path crates/flux          # or install `flux` into ~/.cargo/bin
cargo test --workspace                    # all tests (plain `cargo test` covers only the binary)
```

## Layout

| Crate | What it holds |
|---|---|
| `flux-core` | Text storage (rope) and how a line is laid out on screen. No IO. |
| `flux-syntax` | Tree-sitter parsing and highlighting with Neovim's queries; filetype detection. |
| `flux-view` | Editor state: buffers, windows, options, filetypes, highlight groups. |
| `flux-vim` | The modal engine: keys in, state changes out. Key notation, Ex commands. |
| `flux-tui` | Drawing the editor into a cell grid and writing changed cells to the terminal. |
| `flux` | The `flux` binary: terminal setup and the event loop. |
| `xtask` | Project automation (`cargo xtask …`). |

flux ignores `NO_COLOR`, like Neovim. Its screen relies on color for information such as the
Visual selection and the statusline, and it has no monochrome mode.

## Checking behavior against Neovim

`crates/flux-vim/tests/oracle/cases.json` lists key sequences, each tagged with the milestone
that implements it. `cargo xtask oracle gen` runs them through `nvim --headless --clean` and
records the results in `expected.json`; the `oracle` test replays every case up to the current
milestone through flux and fails on any difference in text, cursor or scroll position. Cases
tagged for later milestones still run and must differ; `-- --show-output` prints the counts.

```sh
cargo test -p flux-vim --test oracle   # flux vs the recorded Neovim results
cargo xtask oracle check               # recorded results vs your installed Neovim
cargo xtask oracle gen                 # re-record after adding cases
```

The expectations are recorded with the Neovim version in `oracle/NVIM_VERSION`; CI pins the
same version. Two more comparisons work the same way:

```sh
cargo test -p flux-vim --test indent   # the indent corpus (tests/indent) vs Neovim's results
cargo xtask indent gen|check           # record / verify them with your Neovim
cargo xtask colors gen|check           # Neovim's default colorscheme, which flux embeds
cargo xtask screens                    # screens side by side in tmux, cell by cell (needs tmux)
```

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your
option. `cargo deny check` enforces a permissive-only dependency policy (`deny.toml`).
