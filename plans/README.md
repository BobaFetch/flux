# plans/

Product planning for Flux 1.0, written by the PM agent (Aerith) from a read-only review of this repo at `27f9a2a`.

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

## Owner decisions (recorded Oct 5, 2026)

- **Name:** the package name is `flux-editor`. The command, config path and Lua namespace stay `flux`.
- **F-01:** the `ignore` crate is approved as an explicit exception to the M7 plan's no-new-dependencies rule.
- **F-02:** flux ignores `NO_COLOR`, like Neovim.
