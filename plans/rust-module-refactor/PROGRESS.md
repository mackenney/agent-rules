# Rust Module Refactor — Progress

## Goal

Restructure `rust/src/` to group evaluator implementations under `evaluator/` and extract command handlers from `main.rs` into `commands/`. Zero behavior changes.

## Steps

| Step | Description | Status |
|------|-------------|--------|
| 01 | Create `evaluator/` submodule (mod.rs, anthropic.rs, openrouter.rs, agentic.rs) | Not started |
| 02 | Create `commands/` submodule (mod.rs, check.rs, cache.rs, rules.rs) | Not started |
| 03 | Wire `lib.rs` and `main.rs` to use new module paths; delete old files | Not started |
| 04 | Verify: build, test, clippy | Not started |

## Constraints

- Zero behavior changes — file moves and import fixes only
- All tests must pass: `cargo nextest run` (unit + integration)
- `cargo clippy -- -D warnings` must be clean
- No new public API surface beyond what restructuring requires
- Do NOT touch: `cache.rs`, `config.rs`, `git.rs`, `parser.rs`, `progress.rs`, `prompt.rs`, `reporter.rs`, `resolver.rs`, `runner.rs`, `schema.rs`
