# Step 06: Integration & E2E Tests

## Context

### Overall Objective

Add OpenRouter as a second LLM provider. This step adds integration tests for the CLI provider flag behavior and e2e test stubs gated on `OPENROUTER_API_KEY`.

### Phase Context

Wave 5 — depends on step-04 (full wiring complete). These tests exercise the binary end-to-end via `assert_cmd`.

### This Step

Add integration tests that verify:
1. `--provider openrouter` without `OPENROUTER_API_KEY` exits 3 with the correct error message
2. `--provider anthropic --model some/model` triggers the model-slash guard
3. `--provider openrouter` defaults to the OpenRouter model (tested via help output)

Add e2e test stubs gated on `OPENROUTER_API_KEY` that skip gracefully when the key is not set.

## Prerequisites

- step-04 complete (full wiring, `--provider` flag functional)
- `cargo nextest run --test integration` passes (existing integration tests)

## Files to Read Before Starting

- `rust/tests/integration/check.rs` — existing `missing_api_key_exits_3` test
- `rust/tests/integration/mod.rs` — module structure
- `rust/tests/common/mod.rs` — `cmd()`, `test_repo()`, `require_env()` helpers
- `rust/tests/e2e/check.rs` — existing e2e test patterns
- `rust/tests/e2e/mod.rs` — module structure

## Implementation

### Task 1: Add OpenRouter API key integration test

In `rust/tests/integration/check.rs`, add:

```rust
/// `check --provider openrouter` without OPENROUTER_API_KEY must exit 3.
#[test]
fn missing_openrouter_api_key_exits_3() {
    cmd()
        .args([
            "check",
            "--provider",
            "openrouter",
            "--files",
            "src/api/bad_controller.py",
            "--repo",
        ])
        .arg(test_repo())
        .env_remove("OPENROUTER_API_KEY")
        .assert()
        .code(3)
        .stderr(predicates::str::contains("OPENROUTER_API_KEY"));
}
```

Add the predicates import at the top of the file if not already present:

```rust
use predicates::prelude::*;
```

Check if `predicates` is already a dev-dependency in `Cargo.toml`. The `assert_cmd` crate re-exports `predicates` — check existing test files for the import pattern. If `predicates` is available via `assert_cmd::prelude::*` or similar, use that. Otherwise add `use predicates::prelude::*;`.

### Task 2: Add model-slash guard integration test

In `rust/tests/integration/check.rs`, add:

```rust
/// Model with '/' and provider=anthropic should show a helpful error.
#[test]
fn model_slash_guard_exits_3() {
    cmd()
        .args([
            "check",
            "--model",
            "anthropic/claude-haiku-4-5",
            "--files",
            "src/api/bad_controller.py",
            "--repo",
        ])
        .arg(test_repo())
        .env("ANTHROPIC_API_KEY", "test-key")
        .assert()
        .code(3)
        .stderr(predicates::str::contains("looks like an OpenRouter model"));
}
```

### Task 3: Add provider help text integration test

In `rust/tests/integration/check.rs`, add:

```rust
/// `check --help` should document the --provider flag.
#[test]
fn help_shows_provider_flag() {
    cmd()
        .args(["check", "--help"])
        .assert()
        .success()
        .stdout(predicates::str::contains("--provider"));
}
```

### Task 4: Add e2e test stubs (gated)

In `rust/tests/e2e/check.rs`, add OpenRouter e2e tests. These use `require_env("OPENROUTER_API_KEY")` to skip gracefully when the key is not set.

First, check if the e2e tests already use `require_env`. Read `rust/tests/e2e/check.rs` to understand the existing pattern.

Add:

```rust
/// OpenRouter: bad controller should fail (exit 1) — requires OPENROUTER_API_KEY.
#[test]
fn openrouter_bad_controller_exits_1() {
    let Some(_key) = require_env("OPENROUTER_API_KEY") else {
        return;
    };

    cmd()
        .args([
            "check",
            "--provider",
            "openrouter",
            "--files",
            "src/api/bad_controller.py",
            "--repo",
        ])
        .arg(test_repo())
        .assert()
        .code(predicates::ord::in_iter([1, 2]));
}

/// OpenRouter: clean controller should pass (exit 0) — requires OPENROUTER_API_KEY.
#[test]
fn openrouter_clean_controller_exits_0() {
    let Some(_key) = require_env("OPENROUTER_API_KEY") else {
        return;
    };

    cmd()
        .args([
            "check",
            "--provider",
            "openrouter",
            "--files",
            "src/api/clean_controller.py",
            "--repo",
        ])
        .arg(test_repo())
        .assert()
        .code(0);
}
```

Check whether these e2e tests need to be gated behind a feature flag (existing e2e tests use `--features test-e2e`). Read `rust/Cargo.toml` for the `test-e2e` feature and `rust/tests/e2e/mod.rs` for conditional compilation. If existing e2e tests are behind `#[cfg(feature = "test-e2e")]`, apply the same gating to these tests.

### Task 5: Verify predicates dependency

Check `rust/Cargo.toml` for `predicates` in dev-dependencies. If it's not there but `assert_cmd` is, `predicates` might be available transitively. Test compilation. If needed, add `predicates = "3"` to `[dev-dependencies]` in `Cargo.toml`.

## Acceptance Criteria

- [ ] `cd rust && cargo build 2>&1` exits 0
- [ ] `cd rust && cargo clippy -- -D warnings 2>&1` exits 0
- [ ] `cd rust && cargo nextest run 2>&1` exits 0
- [ ] `cd rust && cargo nextest run --test integration 2>&1` exits 0
- [ ] `cd rust && cargo nextest run -E 'test(missing_openrouter_api_key_exits_3)' 2>&1` exits 0
- [ ] `cd rust && cargo nextest run -E 'test(model_slash_guard_exits_3)' 2>&1` exits 0
- [ ] `cd rust && cargo nextest run -E 'test(help_shows_provider_flag)' 2>&1` exits 0
- [ ] `grep -q 'openrouter_bad_controller' rust/tests/e2e/check.rs` exits 0

## Reviewer Instructions

1. Run all acceptance criteria commands
2. Verify `missing_openrouter_api_key_exits_3` asserts exit code 3 AND stderr contains "OPENROUTER_API_KEY"
3. Verify `model_slash_guard_exits_3` sets `ANTHROPIC_API_KEY` to avoid the missing-key error path
4. Verify e2e tests use `require_env("OPENROUTER_API_KEY")` pattern to skip gracefully
5. Verify e2e tests follow the same feature-gating pattern as existing e2e tests
6. Run `cargo nextest run --test integration` and confirm all integration tests pass (including pre-existing ones)

## Rollback

```bash
cd rust
git checkout -- tests/integration/check.rs tests/e2e/check.rs
# If Cargo.toml was modified:
git checkout -- Cargo.toml
```
