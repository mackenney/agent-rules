# Step 01: Foundation

## Context

### Overall Objective

Add OpenRouter as a second LLM provider for the Rust `agent-rules` CLI. This requires a `Provider` enum, provider-aware API key resolution, and shared constants that the new client will import.

### Phase Context

Wave 1 — this step has no dependencies and unblocks all subsequent steps.

### This Step

Add the `Provider` enum to `config.rs`, make retry constants `pub(crate)` in `llm.rs`, refactor `get_api_key()` to accept a `Provider`, and add the OpenRouter default model constant. This step changes no external behavior — all existing tests must continue to pass.

## Prerequisites

- `cargo nextest run` passes (77 unit tests)

## Files to Read Before Starting

- `rust/src/config.rs` — current `get_api_key()`, `CheckConfig`, `OutputFormat` enum, `DEFAULT_MODEL`
- `rust/src/llm.rs` — lines 17-18 for `MAX_RETRIES` and `RETRY_BASE_DELAY_MS` constants
- `rust/src/main.rs` — line 27 for `use crate::config::get_api_key` import and line 255-257 for the call site

## Implementation

### Task 1: Add `Provider` enum to `config.rs`

Add after the existing `DEFAULT_MAX_FILE_BYTES` constant block (around line 27), before the `CheckConfig` struct:

```rust
/// LLM provider selection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Anthropic,
    OpenRouter,
}
```

Add a new constant after `DEFAULT_MAX_FILE_BYTES`:

```rust
/// Default model for OpenRouter stateless evaluation
pub const DEFAULT_OPENROUTER_MODEL: &str = "anthropic/claude-3-5-haiku-20241022";
```

### Task 2: Add `provider` field to `CheckConfig`

Add `pub provider: Provider` field to `CheckConfig` (after `model` field, around line 60).

In `Default for CheckConfig`, set `provider: Provider::Anthropic`.

### Task 3: Refactor `get_api_key` to accept `Provider`

Change the existing function signature from:

```rust
pub fn get_api_key() -> Option<String> {
    std::env::var("ANTHROPIC_API_KEY").ok()
}
```

To:

```rust
pub fn get_api_key(provider: Provider) -> Option<String> {
    match provider {
        Provider::Anthropic => std::env::var("ANTHROPIC_API_KEY").ok(),
        Provider::OpenRouter => std::env::var("OPENROUTER_API_KEY").ok(),
    }
}
```

### Task 4: Update `get_api_key` call site in `main.rs`

In `run_check()` (around line 255), change:

```rust
let api_key = get_api_key().context(
    "ANTHROPIC_API_KEY not set. Set the environment variable:\n  export ANTHROPIC_API_KEY=sk-ant-...",
)?;
```

To:

```rust
let api_key = get_api_key(config::Provider::Anthropic).context(
    "ANTHROPIC_API_KEY not set. Set the environment variable:\n  export ANTHROPIC_API_KEY=sk-ant-...",
)?;
```

This preserves existing behavior — provider selection wiring happens in step-04. Import `Provider` via the `config` module path that's already imported.

Also update the import at the top of `main.rs` from:
```rust
use crate::config::{get_api_key, CheckConfig, OutputFormat};
```
to include `Provider`:
```rust
use crate::config::{get_api_key, CheckConfig, OutputFormat, Provider};
```

### Task 5: Make retry constants `pub(crate)` in `llm.rs`

Change lines 17-18 from:

```rust
const MAX_RETRIES: u32 = 3;
const RETRY_BASE_DELAY_MS: u64 = 1000;
```

To:

```rust
pub(crate) const MAX_RETRIES: u32 = 3;
pub(crate) const RETRY_BASE_DELAY_MS: u64 = 1000;
```

### Task 6: Add unit test for provider-aware `get_api_key`

In the existing `#[cfg(test)] mod tests` block in `config.rs`, add:

```rust
#[test]
fn test_get_api_key_reads_correct_env_var() {
    // Save and clear
    let saved_anthropic = std::env::var("ANTHROPIC_API_KEY").ok();
    let saved_openrouter = std::env::var("OPENROUTER_API_KEY").ok();

    unsafe {
        std::env::set_var("ANTHROPIC_API_KEY", "test-anthropic-key");
        std::env::set_var("OPENROUTER_API_KEY", "test-openrouter-key");
    }

    assert_eq!(
        get_api_key(Provider::Anthropic),
        Some("test-anthropic-key".to_string())
    );
    assert_eq!(
        get_api_key(Provider::OpenRouter),
        Some("test-openrouter-key".to_string())
    );

    // Restore
    unsafe {
        match saved_anthropic {
            Some(v) => std::env::set_var("ANTHROPIC_API_KEY", v),
            None => std::env::remove_var("ANTHROPIC_API_KEY"),
        }
        match saved_openrouter {
            Some(v) => std::env::set_var("OPENROUTER_API_KEY", v),
            None => std::env::remove_var("OPENROUTER_API_KEY"),
        }
    }
}

#[test]
fn test_default_config_has_anthropic_provider() {
    let config = CheckConfig::default();
    assert_eq!(config.provider, Provider::Anthropic);
}
```

## Acceptance Criteria

- [ ] `cd rust && cargo build 2>&1` exits 0
- [ ] `cd rust && cargo nextest run 2>&1` exits 0 (all existing tests pass + 2 new tests)
- [ ] `cd rust && cargo nextest run -E 'test(test_get_api_key_reads_correct_env_var)' 2>&1` exits 0
- [ ] `cd rust && cargo nextest run -E 'test(test_default_config_has_anthropic_provider)' 2>&1` exits 0
- [ ] `cd rust && cargo clippy -- -D warnings 2>&1` exits 0
- [ ] `grep -q 'pub(crate) const MAX_RETRIES' rust/src/llm.rs` exits 0
- [ ] `grep -q 'pub(crate) const RETRY_BASE_DELAY_MS' rust/src/llm.rs` exits 0
- [ ] `grep -q 'pub enum Provider' rust/src/config.rs` exits 0
- [ ] `grep -q 'DEFAULT_OPENROUTER_MODEL' rust/src/config.rs` exits 0

## Reviewer Instructions

1. Run each acceptance criteria command verbatim
2. Verify `get_api_key` signature is `pub fn get_api_key(provider: Provider) -> Option<String>`
3. Verify `CheckConfig` has `provider: Provider` field with `Provider::Anthropic` default
4. Verify no behavioral change to existing CLI (same exit codes, same error messages)

## Rollback

```bash
cd rust
git checkout -- src/config.rs src/llm.rs src/main.rs
```
