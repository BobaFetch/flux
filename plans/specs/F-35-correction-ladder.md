# F-35 — Correction ladder: encode each correction in the most durable layer

| | |
|---|---|
| Backlog | F-35 (process; source: poteto, "whenever you correct your agent", Oct 2026) |
| Size | M overall, split into owned items below (most are XS–S) |
| Spec status | 📝 Draft, Oct 16, 2026 |
| Repo state | `BobaFetch/flux` @ `1726eb7` (main), read Oct 16, 2026 |
| Kind | **Plan, no code.** Lists what to change, by owner, for Sector Five to build later. Dev-only; nothing here ships in the editor |
| Owner | Y'shtola sequences; each row's owner builds; Fina verifies |
| Sector 5 routing | Y'shtola turns rows into plan steps in `.sectorfive/plans/` → owner builds → Fina verifies → PM merges after Fina approves and CI is green |

## 1. Problem

Most Flux rules live in prose (`AGENTS.md`, `.sectorfive/contract.md`, role briefs) or depend on Fina noticing. Prose is read unevenly by agents, and review catches things late and not every time. Fina's F-06 veto (B1: buffer text bleeds around the `:Files` picker, e.g. `liFiles>` at 80×24) was a bug no automated check covered, found only by a careful reviewer. Agents copy what the code already does, so a correction that only lives in a chat or a review comment comes back.

## 2. Goal

Adopt the correction ladder as policy, and move existing prose/review-only rules down the ladder where a cheap check exists:

1. **Codebase**: architecture, types, invariants, crate boundaries (the wrong thing will not compile or cannot be expressed).
2. **Static analysis**: lints, `cargo xtask` checks, tests, CI jobs (the wrong thing fails CI).
3. **Prose**: `AGENTS.md`, `.sectorfive/` contract and briefs (the wrong thing is documented as wrong).
4. **Human review**: Fina, the user (last resort).

Each correction goes in the highest layer (1 is highest) that can hold it.

## 3. Non-goals

- Writing any of the checks in this spec. Each row becomes its own plan step.
- Changing behavior of the editor.
- Removing Fina's gate. Review stays for judgment calls (scope, design, parity taste); it stops being the only guard for mechanical rules.
- New dependencies for lint tooling unless a row says so and the user approves (dependency adds are a contract stop).

## 4. Requirements

### R1. The ladder is policy

Y'shtola adds a short "Correction ladder" section to `.sectorfive/contract.md` (and one line in `AGENTS.md` pointing to it) stating the order above.

### R2. Every correction gets a proposed encoding

Every future Fina finding (veto or note) and every user correction is recorded in `.sectorfive/decisions.md` with a **proposed lowest-layer encoding**: the layer (1–4), the concrete check, and the owner. Format:

`- YYYY-MM-DD correction: <what went wrong> → layer <n>: <proposed check> (owner <role>, item <F-xx or plan step>)`

"Layer 4 only" is allowed but needs a one-line reason (e.g. "taste call, no mechanical signal"). Fina's review checklist gains an item: "each finding has a proposed encoding."

### R3. Trim prose once enforced

When a check lands that enforces a rule, the matching prose in `AGENTS.md` / contract / briefs is cut to one line that names the check (or removed). Y'shtola does this in the same plan as the check, so prose and checks don't drift.

### R4. Move current rules down (inventory)

| # | Rule today (where) | Today's layer | Target layer | Proposed check (described, not coded) | Owner | Priority |
|---|---|---|---|---|---|---|
| C1 | Pickers paint only their own box; no buffer text bleeds through (Fina F-06 veto B1) | 4 (Fina) | 2 | Screen/snapshot test of `:Files` over a buffer full of text at **80×24 and 40×12**: every cell outside the picker's box equals the underlying window's cells, every cell inside belongs to the picker; also a `cargo xtask screens` case `picker-files.txt` vs Neovim/fzf layout. Fails on any row like `liFiles>` | Yuna (Fina writes expected cells) | P0 (blocks F-06) |
| C2 | Same for `:Buffers` and any other float (hover, diag float, completion menu) | 4 | 2 | Generalize C1 into a reusable "float over filled buffer" assertion in `flux-tui` tests, run for every float kind at both sizes | Yuna | P1 |
| C3 | `flux-core` has no IO (AGENTS.md Architecture) | 3 | 1/2 | Dependency check in xtask (`cargo xtask arch`): `flux-core` may not depend on `std::fs`/`std::process`/`std::net` users or on IO crates; grep-based deny of `std::fs`, `std::process`, `std::net` in `crates/flux-core/src` | Lightning | P1 |
| C4 | Crate layering (`flux-tui` doesn't reach into `flux-vim` key semantics; LSP split across `flux-lsp` / `flux-view::lsp` / `flux-vim::lsp` / `servers.rs`) | 3 | 1/2 | `cargo xtask arch` reads `cargo metadata` and fails on any edge not in an allowed crate-dependency table checked into xtask | Lightning | P1 |
| C5 | Tests don't depend on `TERM`/`NO_COLOR`/`COLORTERM` | 2 (CI job with `NO_COLOR=1 TERM=dumb`) + 3 | 2 | Keep the CI job; trim AGENTS.md line to a pointer (R3). Optionally add a third run with `COLORTERM=truecolor TERM=xterm-256color` | Lightning / Y'shtola | P2 |
| C6 | Run `cargo test --workspace`, not plain `cargo test` | 3 | 2 | Already enforced by CI; trim prose to one line | Y'shtola | P2 |
| C7 | Generated snapshots (`expected.json`, indent `expected/`, `colors.json`) are never hand-edited; `gen` only by plan | 3 + 4 | 2 | CI step: run `cargo xtask <oracle\|indent\|colors> gen` and fail if the tree differs (already partly via `check` in `oracle.yml`); add a header/hash in each generated file that the check verifies | Lightning | P1 |
| C8 | Deferred oracle cases must still differ (`MILESTONE`) | 2 (`deferred_cases_still_differ`) | 2 | Done; keep. Trim prose | Y'shtola | P3 |
| C9 | Every path has exactly one owner (`ownership.md`) | 4 (checked by hand with `git ls-files`) | 2 | `cargo xtask owners`: parse `ownership.md` globs, fail if any tracked file has 0 or 2+ owners; run in CI | Lightning | P1 |
| C10 | Roles stay inside their paths (contract veto: out-of-scope files touched) | 4 | 2 | `cargo xtask owners --diff <base>`: given a PR's role (from branch prefix or PR label), list changed files outside that role's paths; CI warns, Fina reads the output instead of eyeballing | Lightning | P1 |
| C11 | Tifa never changes behavior (polish only) | 4 | 2 (partial) | CI job on Tifa PRs: full test + oracle + screens must pass with **no** expected-output changes except screens/string snapshots listed in the plan; anything else fails | Lightning (job), Fina | P2 |
| C12 | No dependency adds without approval; explicit versions; permissive licenses | 2 (`cargo deny`) + 3 | 2 | Add CI diff check: if `Cargo.lock` gains a crate, require a matching `decisions.md` line dated in the PR | Lightning | P2 |
| C13 | No `dbg!`, stdout/stderr printing, no unsafe | 2 (workspace lints) | 2 | Done; trim prose | Y'shtola | P3 |
| C14 | Build roles never merge; dangerous actions need approval | 3 + 4 | 1 (platform) | GitHub branch protection on `main`: require CI + one approving review; agents' tokens lack merge rights. User decision (Open question 2) | User | P2 |
| C15 | Fina model rule: high for any non-Markdown change, medium only for Markdown under `.sectorfive/`, `.grok/`, `docs/`, `AGENTS.md` | 3 | 2 | `.sectorfive/bin/role fina` prints the required thinking level for a given PR by reading changed paths (`--pr <n>`) and refuses medium when any non-Markdown file changed | Y'shtola (script, shellcheck in CI) | P2 |
| C16 | Error messages short and actionable; Neovim wording wins (Tifa brief) | 3 + 4 | 2 (partial) | Screen cases for known failure paths (`edit-lsp-err*`, missing `tsserver` as seen in the F-06 hands-on pass) assert the message fits one line at 80 cols and names a fix | Tifa (copy), Fina (cases) | P2 |
| C17 | Legibility under `NO_COLOR`/`TERM=dumb`; never meaning by color alone | 3 + 4 | 2 | `cargo xtask screens` gains a no-color mode for picker, wildmenu, diagnostics; assertion that selected/active items differ in text or attributes, not only color | Yuna / Tifa | P2 |
| C18 | Every Fina finding / user correction gets an encoding (R2) | — | 3 + 2 | Prose in contract (R1/R2); later a CI check that a PR touching `decisions.md` with a `veto` line also has a `correction:` line | Y'shtola | P1 |
| C19 | Prose trimmed when enforced (R3) | — | 3 | Contract rule; Fina checklist item | Y'shtola | P1 |

## 5. Acceptance checks

- A1. Contract and `AGENTS.md` state the ladder (R1); Fina's checklist has the "proposed encoding" item (R2).
- A2. C1 lands before F-06 is re-verified: the test fails on `main` @ `1726eb7` (reproduces B1) and passes after Yuna's fix, at 80×24 and 40×12.
- A3. Each P0/P1 row is either a merged check or a plan step with an owner in `.sectorfive/plans/`.
- A4. For each check merged, the matching prose is trimmed in the same PR or the next one (R3), and `AGENTS.md` stays under its current 50 lines.
- A5. The next three Fina findings after this lands each have a `correction:` line in `decisions.md`.

## 6. Risks

- Checks that are flaky or slow erode trust faster than prose does. Keep each check fast and deterministic (F-02, F-34 lessons); screens stay in their own job.
- Ownership-diff checks (C10) can block legitimate cross-role plans. Warn, don't fail, until the plan format names allowed paths.
- Over-trimming prose: agents still need the "why". Keep one line per rule pointing at its check.

## 7. Open questions

1. Should C1 be its own backlog item (F-06 fix) or ride F-35? Proposal: its own fix PR by Yuna, referenced here.
2. Branch protection on `main` (C14): does the user want required reviews, given plans are pushed directly to `main` today?
3. Should `cargo xtask arch` / `owners` be one xtask or two? (Lightning's call.)
