# PROGRESS.md — Rust agent-rules CLI

## Status
Not Started

## Objective
Rewrite the agent-rules CLI in idiomatic Rust. The tool checks PR diffs against LLM-powered rules defined in `.agent-rules.toml` files. Commands: `check`, `cache stats`, `cache clear`, `rules list`, `rules validate`.

## Wave Map

| Wave | Steps | Can Parallelize | Depends On |
|------|-------|-----------------|------------|
| 0 | step-01 | No (sequential) | — |
| 1 | step-02, step-03 | Yes | step-01 |
| 2 | step-04 | No | step-02, step-03 |
| 3 | step-05, step-07 | Yes | step-04 (05), step-02 (07) |
| 4 | step-06 | No | step-04, step-05 |
| 5 | step-08 | No | all previous |

## Dependency Table

| Step | File(s) | Depends On | Depended By |
|------|---------|------------|-------------|
| 01 | Cargo.toml, main.rs, all module stubs | — | 02, 03 |
| 02 | schema.rs | 01 | 04, 05, 06, 07, 08 |
| 03 | git.rs, parser.rs, config.rs | 01 | 04 |
| 04 | resolver.rs, cache.rs | 02, 03 | 05, 06 |
| 05 | prompt.rs, llm.rs | 02, 04 | 06 |
| 06 | runner.rs | 04, 05 | 08 |
| 07 | reporter.rs, progress.rs | 02 | 08 |
| 08 | cli (main.rs wiring) | all | — |

## Orchestrator Protocol
1. Read this file to identify current wave
2. Dispatch all steps in current wave in parallel
3. After each step: dispatch reviewer agent (see step file)
4. Mark step complete only after reviewer passes
5. Advance to next wave only when all steps complete
6. Blockers: stop and report to user

## Subagent Contract
- Workers: Read step file fully before acting. Implement only what the step specifies.
- Workers: Commit changes with message "step-NN: <name>"
- Workers: Report: "Step NN complete ✅ (commit <hash>)" or "Step NN FAILED: <reason>"
- Reviewers: Run acceptance criteria commands verbatim.

## Steps

- [ ] [step-01-scaffold](./step-01-scaffold.md) — Create Cargo project, Cargo.toml, empty module files
- [ ] [step-02-schema-types](./step-02-schema-types.md) — All serde types: Rule, Verdict, FileVerdict, PRReport, FileDiff
- [ ] [step-03-git-parser](./step-03-git-parser.md) — Git shelling, changed files, TOML parser, diff annotation
- [ ] [step-04-resolver-cache](./step-04-resolver-cache.md) — Rule resolver (walk+glob+merge), file cache, NullCache
- [ ] [step-05-prompt-llm](./step-05-prompt-llm.md) — Prompt builder, Anthropic HTTP client, retry logic
- [ ] [step-06-runner](./step-06-runner.md) — check_file, check_pr, semaphore concurrency
- [ ] [step-07-reporter](./step-07-reporter.md) — Text (ruff-style), JSON, GitHub reporters; progress bar
- [ ] [step-08-cli](./step-08-cli.md) — Clap CLI wiring: check, cache, rules subcommands; exit codes

## Architecture Notes

**Module layout** (flat, no lib.rs):
```
rust/
├── Cargo.toml
└── src/
    ├── main.rs          # entrypoint, clap dispatch, module declarations
    ├── schema.rs        # Rule, Verdict, FileVerdict, PRReport, FileDiff
    ├── config.rs        # CheckConfig, defaults, env loading
    ├── git.rs           # run_git(), get_changed_files()
    ├── parser.rs        # parse_rule_file(), annotate_diff()
    ├── resolver.rs      # resolve_rules(), glob matching, merging
    ├── cache.rs         # CacheManager, NullCache, key derivation
    ├── prompt.rs        # build_system_prompt(), build_user_prompt()
    ├── llm.rs           # AnthropicClient, LlmError, retry logic
    ├── runner.rs        # check_file(), check_pr(), concurrency
    ├── reporter.rs      # TextReporter, JsonReporter, GithubReporter
    └── progress.rs      # ProgressReporter trait, TtyProgress, CiProgress
```

**Error handling**: `anyhow::Result` everywhere except `llm.rs` which uses `thiserror` for `LlmError` (needs retryable classification).

**Concurrency**: `tokio::sync::Semaphore` limits parallel LLM calls. `JoinSet` collects results with cancellation support.

**Git calls**: `std::process::Command` (blocking). Wrap in `spawn_blocking` only if profiling shows contention.

**No agentic evaluator**: `needs-more-context` verdict collapses to `fail`.
