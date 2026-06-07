# Agent Guidelines — agent-rules

## Project

`agent-rules` is a multi-implementation CLI tool for directory-scoped AI rule enforcement during PR reviews. Given a set of changed files (from a git diff or explicit list), it evaluates each file against the rules that apply to it, using an LLM as the evaluator. The result is a structured verdict report that can block merges, post GitHub PR comments, or be consumed by downstream tooling.

Rules are defined in `.agent-rules.toml` files and cascade from the repo root to subdirectories; child rules override parent rules by ID.

The behavioral contract lives in `SPEC.md` and is shared across all implementations.

## Workspace layout

```
typescript/           TypeScript implementation
  src/                Source modules
  tests/              Tests — unit (inline) + integration (tests/) + e2e (tests/integration/)
  package.json
  tsconfig.json
  vitest.config.ts
rust/                 Rust implementation
  src/                Source modules (unit tests inline via #[cfg(test)])
  tests/
    common/           Shared test helpers
    integration/      Integration tests (no API key required)
    e2e/              E2E tests (gated by --features test-e2e)
  Cargo.toml
  .config/nextest.toml
test-repo/            Shared fixture repository used by both implementations
docs/                 Supplementary documentation
examples/             Example .agent-rules.toml configurations
plans/                Active development plans — committed, deleted when complete
artifacts/            Transient working files — git-ignored, never committed
.worktrees/           Git worktrees (convention for this repo)
SPEC.md               Behavioral contract shared across all implementations
MASTER_PROGRESS.md    Single source of truth for project-wide work status
```

## SPEC-driven development

- Every implementation conforms to `SPEC.md`. When behavior is ambiguous, the spec wins.
- Spec documents use MUST / SHOULD / MAY language (RFC 2119).
- The pipeline is: **investigate → spec → plan → orchestrate**.
- Both implementations must pass equivalent test suites for the same behavioral invariants.

## Master Progress

`MASTER_PROGRESS.md` is the single source of truth for project-wide work status.
Every human and agent working on agent-rules reads it first to understand current state.

**What it contains:**
- **In Progress** — active plans with a link to their plan directory
- **Queued** — not-started plans with a link to their plan directory
- **Completed** — one-liner per finished feature/plan, with commit hash
- **Known Gaps** — identified issues with no active plan owner

**Rules:**
- When a plan completes and is deleted → add a one-liner to Completed with the merge commit
- When a new plan is created → add it to Queued
- When work starts on a plan → move it from Queued to In Progress
- Keep entries as one-liners; all detail lives in plan files and git history
- Never let `MASTER_PROGRESS.md` drift: update it in the same commit as the plan change

## Plans

- `plans/` is committed; each plan lives under `plans/<feature>/` with `PROGRESS.md` and `step-NN-*.md` files.
- Plans are created by the planner skill; they hold all in-progress work context for the duration of execution.
- The entire plan directory is deleted once the plan is complete. No plan files survive after completion; their outcomes live in git history and in `MASTER_PROGRESS.md`.
- Code comments and docs MUST NOT reference plan files.

## Artifacts

- All transient working files (investigation scans, fact-check reports, progress trackers, brainstorm notes) live in `artifacts/` and are git-ignored.
- Never commit files from `artifacts/`.
- Never commit scratch files or session recordings to the repo root.

## Testing

### TypeScript

```sh
cd typescript
npm test                          # all unit + integration tests (157 total)
npm run test:integration          # integration only (live API, needs ANTHROPIC_API_KEY)
```

### Rust

```sh
cd rust
cargo nextest run                             # 77 unit tests (inline)
cargo nextest run --test integration          # 9 integration tests (no API key)
ANTHROPIC_API_KEY=<key> cargo nextest run --test e2e --features test-e2e  # 7 e2e tests
cargo clippy -- -D warnings                  # must be clean
```

### Principles

- **Inline unit tests** (`#[cfg(test)]` in Rust, co-located in TS) are implementation details — change freely during refactors.
- **External tests** (`tests/`) are behavioral contracts. A failing external test is a bug or a deliberate spec change, never a refactor casualty.
- Never weaken an external test to make a refactor pass.

## Conventions

- Commit messages: imperative mood, 72 chars, no period
- Branch names: `ignacio@<module>/<kebab-description>`
- TypeScript: `npm run lint`, `npm run build` before committing
- Rust: `cargo fmt`, `cargo clippy -- -D warnings` before committing
- No section-separator comments (`// ---`, `// ===`, etc.)
- Comments explain WHY, not WHAT

## Tooling

- **Shell:** bash; `jq`, `rg`, `fdfind` available
- **TypeScript:** Node.js + npm; `tsx` for execution; `vitest` for tests
- **Rust:** `cargo`, `cargo nextest run` (required for tests)
- **Worktrees:** `.worktrees/<name>/` convention
