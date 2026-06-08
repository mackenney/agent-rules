# MASTER_PROGRESS.md

## Status

Stable

## Objective

`agent-rules` is a directory-scoped AI rule enforcement tool for PR reviews. Full behavioral contract in `SPEC.md`. Two implementations: TypeScript (stable) and Rust (complete).

## In Progress

- `openrouter-evaluator` — Add OpenRouter as second LLM provider (Rust): `--provider openrouter`, `OpenRouterClient`, cache key fix; [plan](plans/openrouter-evaluator/PROGRESS.md)

## Queued

- `ignacio@llm/evaluator-protocol` — TypeScript evaluator protocol refactor: `StatelessEvaluator`/`AgenticEvaluator` interfaces extracted to `src/evaluator.ts`; needs rebase onto main before merge

## Completed

- `initial-typescript-implementation` — TypeScript CLI: resolver, cache, two-pass LLM evaluation, GitHub reporter; commit 8b7e9cb
- `verdict-model-simplification` — simplified verdict enum, YAML→TOML migration, impl cleanup; plans deleted, outcomes in history
- `cleanup-refactor` — architecture, code quality, testing, API ergonomics scouts; multi-phase refactor; plans deleted
- `spec-unification` — merged SPEC.md + SPEC-evaluator-protocol.md into single canonical spec with evaluator protocol as first-class section; commit 80fedf1
- `multi-impl-restructure` — TypeScript moved to `typescript/`, test-repo to root, shared layout established; commit 7fefd01
- `rust-implementation` — full Rust reimplementation: all 8 plan steps, evaluator protocol, two-pass agentic routing, 86 tests (77 unit + 9 integration); commit 873313e
- `rust-module-refactor` — restructured rust/src/: evaluator/ submodule (AnthropicClient, OpenRouterClient, PiAgenticEvaluator), commands/ submodule (check, cache, rules); commit 898ee73

## Known Gaps

- TypeScript evaluator protocol (`ignacio@llm/evaluator-protocol`) not yet merged — `StatelessEvaluator`/`AgenticEvaluator` interfaces exist in Rust but not yet extracted in TS
- No CI configuration (GitHub Actions) for either implementation
- Rust e2e tests require `ANTHROPIC_API_KEY` and are not run in any automated pipeline
- `rust/src/evaluator/anthropic.rs` and `rust/src/evaluator/openrouter.rs` duplicate `call_with_retry` logic and have near-identical `parse_verdict` implementations; should share a common retry helper and verdict-parsing function
- `evaluator/agentic.rs` `normalize_verdict` makes raw HTTP requests instead of reusing `AnthropicClient`/`OpenRouterClient`; bypasses retry logic and duplicates request/response handling from the stateless clients
