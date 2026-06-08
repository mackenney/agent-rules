# Step 04 — Verify

## Objective

Full verification that the refactor is correct: build, all tests, clippy, and structural assertions.

## Prerequisites

Step 03 must be complete.

## Actions

### 1. Build

```bash
cd rust && cargo build
```

### 2. Run all tests

```bash
cd rust && cargo nextest run
```

This runs all unit tests (inline in source) and integration tests (in `tests/`). Expected: all pass, zero failures.

### 3. Clippy

```bash
cd rust && cargo clippy -- -D warnings
```

Expected: zero warnings.

### 4. Structural assertions

Verify old files are gone and new files exist:

```bash
cd rust/src

# Old flat files must NOT exist
test ! -f evaluator.rs && echo "evaluator.rs gone: OK"
test ! -f llm.rs && echo "llm.rs gone: OK"
test ! -f openrouter.rs && echo "openrouter.rs gone: OK"
test ! -f agentic.rs && echo "agentic.rs gone: OK"

# New evaluator/ directory module
test -f evaluator/mod.rs && echo "evaluator/mod.rs: OK"
test -f evaluator/anthropic.rs && echo "evaluator/anthropic.rs: OK"
test -f evaluator/openrouter.rs && echo "evaluator/openrouter.rs: OK"
test -f evaluator/agentic.rs && echo "evaluator/agentic.rs: OK"

# New commands/ directory module
test -f commands/mod.rs && echo "commands/mod.rs: OK"
test -f commands/check.rs && echo "commands/check.rs: OK"
test -f commands/cache.rs && echo "commands/cache.rs: OK"
test -f commands/rules.rs && echo "commands/rules.rs: OK"
```

### 5. Verify no stale references

```bash
cd rust
rg 'crate::(llm|agentic)::' src/ && echo "STALE REFS FOUND" || echo "No stale crate refs: OK"
rg 'agent_rules::(llm|agentic|openrouter)::' src/ tests/ && echo "STALE REFS FOUND" || echo "No stale external refs: OK"
# Note: crate::openrouter is now a child of evaluator, so `crate::evaluator::openrouter` is fine
# but `crate::openrouter::` as a top-level module ref should not exist
rg '^pub mod (llm|agentic|openrouter)' src/lib.rs && echo "STALE MOD DECLS" || echo "No stale mod decls: OK"
```

### 6. Verify main.rs is lean

```bash
cd rust
wc -l src/main.rs
# Expected: ~230-260 lines (CLI definitions + main dispatch, no command handler bodies)
```

## Acceptance Criteria

All commands exit 0:

```bash
cd rust && cargo build
cd rust && cargo nextest run
cd rust && cargo clippy -- -D warnings
cd rust/src && test ! -f llm.rs && test ! -f agentic.rs && test ! -f openrouter.rs && test ! -f evaluator.rs
cd rust/src && test -f evaluator/mod.rs && test -f evaluator/anthropic.rs && test -f evaluator/openrouter.rs && test -f evaluator/agentic.rs
cd rust/src && test -f commands/mod.rs && test -f commands/check.rs && test -f commands/cache.rs && test -f commands/rules.rs
```
