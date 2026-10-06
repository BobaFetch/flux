# plans/

Product planning for Flux 1.0, written by the PM agent (Aerith) from a read-only review of this repo (at `27f9a2a` for the PRD, backlog, review and F-01–F-03; at `d8ab89b` for F-07, F-13, F-23 and F-27).

| File | What it is |
|---|---|
| `flux-1.0-prd.md` | Product requirements for a public, installable Flux 1.0: goals, must-have features, user stories, milestones (M7 finish → M8 1.0 gaps → M9 release hardening → 1.0), non-functional requirements, scope, open questions |
| `feature-backlog.md` | Dependency-ordered backlog F-01…F-33 (plus post-1.0 P-items), each with size, dependencies and a blocker flag |
| `repo-review.md` | PM review of the repo as of Oct 5, 2026: state, gaps to 1.0, risks, build/test results |
| `specs/` | Implementation-ready specs, one per backlog item, ordered by the backlog |

## Specs

Each spec is written to be handed to a coding agent as the prompt, then reviewed against it. Each one lists requirements, acceptance commands, tests, Sector 5 routing and a definition of done.

Start in this order:

1. `specs/F-03-oracle-no-silent-skips.md`: run deferred oracle cases instead of skipping them silently; retag the six M4 cases; fix the counts in the docs.
2. `specs/F-02-tests-independent-of-term-no-color.md`: tests pass whatever `TERM`/`NO_COLOR` are set to; flux ignores `NO_COLOR`.
3. `specs/F-01-files-picker-gitignore-fair-cap.md`: `:Files` respects `.gitignore` and shares its 5,000-file cap fairly.
4. `specs/F-23-oracle-recording-without-set-all.md`: reset options between oracle cases without `:set all&`, so recording works on the owner's Mac; fail loudly if Neovim stops early.
5. `specs/F-13-graceful-lsp-startup.md`: a built-in language server that flux started on its own and that fails to start gets one non-blocking line and no retries; explicitly enabled servers keep Neovim's message.
6. `specs/F-27-versioning-and-changelog.md`: `CHANGELOG.md`, `docs/versioning.md`, a PR template, and tests that keep version numbers and changelog headings consistent.
7. `specs/F-07-lua-runtime-init-lua-flux-opt.md`: embedded Lua 5.4 (mlua), `~/.config/flux/init.lua` loaded before files open, `flux.opt`/`flux.cmd`/`flux.version`, `--clean` and `-u`, Neovim-style errors. Its Appendix A reserves the whole `flux.*` surface for F-08–F-11.

All seven specs are ready to implement, with every decision settled. F-23's last acceptance check is a run on the owner's Mac. F-07 is the start of M7 Stage C, which is split into separately mergeable PRs: F-07, then F-08 (options), F-10 (`flux.lsp`), F-11 (colors) and F-09 (keymaps, which the M7 plan allows to slip to M8).

## Owner decisions (recorded Oct 5, 2026)

- **Name:** the package name is `flux-editor`. The command, config path and Lua namespace stay `flux`.
- **F-01:** the `ignore` crate is approved as an explicit exception to the M7 plan's no-new-dependencies rule.
- **F-02:** flux ignores `NO_COLOR`, like Neovim.

## Owner decisions (recorded Oct 6, 2026)

- **F-07:** a failing `init.lua` keeps what ran before the error (like Neovim); Stable `flux.*` items are frozen under semver for 1.x, the rest is Experimental; `flux.cmd` is included; the Appendix A shapes for `flux.map`, `flux.g`, `flux.lsp` and `flux.highlight` are confirmed; `--clean` and `-u` ship with F-07.
- **F-13:** a failed auto-started server gets a warning line and `lsp.log`, with no `:lsp status`; `$FLUX_LSP_CONFIG` configs count as user-enabled; a failed auto-started server is disabled for the session.
- **F-27:** the version covers the CLI, config path, Stable `flux.*`, options, Ex commands and keys (Rust crates are internal); no CI gate for changelog entries (PR-template checkbox plus review); `[Unreleased]` is seeded with one line per milestone.

## Owner decisions pending

None.
