# flux — contracts (pointers, not copies)
- Neovim parity (primary behavioral contract): crates/flux-vim/tests/oracle/{cases.json,expected.json,NVIM_VERSION}; indent corpus crates/flux-vim/tests/indent/; colors crates/flux-view/src/colors.json. Pinned to NVIM v0.12.5 — oracle.yml CI + NVIM_VERSION must agree.
- Key notation + Ex grammar: crates/flux-vim/src (keys in, state changes out; engine has no IO).
- Highlight/filetype surfaces: crates/flux-syntax/{src,queries} (vendored nvim queries; queries/NOTICE respected).
- LSP JSON-RPC over stdio: crates/flux-lsp/src/{lib,transport,config}.rs.
- Rendering contract (grid cells → terminal): crates/flux-tui/src; visual expectations crates/flux-tui/tests/visual_draw.rs.
- Safety/lint policy: Cargo.toml [workspace.lints] (unsafe forbid), rustfmt.toml, CI -D warnings.
- License policy: deny.toml (permissive-only; copyleft out), dual MIT/Apache.
