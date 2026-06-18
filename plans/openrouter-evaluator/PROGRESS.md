# PROGRESS.md

## Status

Complete

## Objective

Add OpenRouter as a second LLM provider for stateless evaluation. A new `--provider openrouter` flag selects the provider, reads `OPENROUTER_API_KEY`, and instantiates `OpenRouterClient` (a new struct in `openrouter.rs` implementing `StatelessEvaluator`). The client uses the OpenAI-compatible chat completions API, transforms tool schemas to function-call format, conditionally injects `cache_control` for `anthropic/` models, and parses `arguments` as a JSON string.

## Open Decisions

- **Default OpenRouter model**: Plan uses `anthropic/claude-3-5-haiku-20241022`. User may prefer a different default (e.g., `anthropic/claude-haiku-4-5`). Hardcoded as a constant — easy to change.

## Wave Map

| Wave | Steps | Can Parallelize | Depends On |
|------|-------|-----------------|------------|
| 1 | step-01 | No | — |
| 2 | step-02 | No | Wave 1 |
| 3 | step-03 | No | Wave 2 |
| 4 | step-04 | No | Wave 3 |
| 5 | step-05, step-06 | Yes | Wave 4 |

## Dependency Table

| Step | File(s) | Depends On | Depended By |
|------|---------|------------|-------------|
| step-01 | `config.rs`, `llm.rs` | — | step-02, step-03, step-04, step-05, step-06 |
| step-02 | `openrouter.rs`, `main.rs` | step-01 | step-03 |
| step-03 | `main.rs` | step-02 | step-04 |
| step-04 | `main.rs`, `cache.rs`, `config.rs` | step-03 | step-05, step-06 |
| step-05 | `openrouter.rs` (tests) | step-04 | — |
| step-06 | `tests/integration/check.rs`, `tests/e2e/check.rs` | step-04 | — |

## Orchestrator Protocol

1. Read this file to identify current wave
2. Dispatch all steps in current wave in parallel
3. After each step: dispatch reviewer agent
4. Mark step complete only after reviewer passes
5. Advance to next wave only when all steps in current wave are complete
6. Blockers: stop and report to user with full context

## Subagent Contract

- Workers: Read step file fully before acting. Implement only what the step specifies.
- Workers: Commit changes with message "step-NN: <name>"
- Workers: Report back: "Step NN complete ✅ (commit <hash>)" or "Step NN FAILED: <reason>"
- Reviewers: Run acceptance criteria commands verbatim. Pass or fail with specifics.

## Steps

- [x] [step-01-foundation](./step-01-foundation.md) — Add `Provider` enum, `pub(crate)` retry constants, `get_api_key(provider)`, OpenRouter model default (6c3c5d1)
- [x] [step-02-openrouter-client](./step-02-openrouter-client.md) — Create `openrouter.rs` with types, client struct, request building, response parsing, `StatelessEvaluator` impl (b1d733d)
- [x] [step-03-cli-provider-flag](./step-03-cli-provider-flag.md) — Add `--provider` flag to `CheckArgs`, `ProviderArg` enum, model-slash guard (feecbd0)
- [x] [step-04-wiring](./step-04-wiring.md) — Wire provider into `run_check()`, add `OpenRouterClient` import, fix cache key to include provider (90395ef)
- [x] [step-05-unit-tests](./step-05-unit-tests.md) — Inline unit tests for `openrouter.rs`: 17 tests covering parsing, serialization, cache_control, NMC collapse (eb71cda)
- [x] [step-06-integration-tests](./step-06-integration-tests.md) — Integration tests for provider CLI behavior; e2e test stubs gated on `OPENROUTER_API_KEY` (115d972)
