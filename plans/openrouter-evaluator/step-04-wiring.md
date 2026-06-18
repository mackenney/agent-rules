# Step 04: Wiring

## Context

### Overall Objective

Add OpenRouter as a second LLM provider. This step wires everything together: provider-aware API key resolution, conditional evaluator instantiation, model default resolution, and cache key correctness.

### Phase Context

Wave 3 — depends on step-02 (`OpenRouterClient` exists in `openrouter.rs`) and step-03 (`--provider` flag and `config.provider` populated). This step makes the feature functional end-to-end.

### This Step

Rewrite `run_check()` to branch on `config.provider`:
- Read the correct API key env var
- Instantiate the correct client
- Apply provider-specific model defaults
- Handle agentic evaluator availability when using OpenRouter
- Fix cache key to include provider (prevents cross-provider collisions)

## Prerequisites

- step-02 complete (`openrouter.rs` exists with `OpenRouterClient` implementing `StatelessEvaluator`, `mod openrouter;` in `main.rs`)
- step-03 complete (`--provider` flag, `config.provider` populated, model-slash guard)

## Files to Read Before Starting

- `rust/src/main.rs` — `run_check()` function (lines 254-330+), imports at top
- `rust/src/openrouter.rs` — `OpenRouterClient::new` signature
- `rust/src/config.rs` — `Provider`, `DEFAULT_OPENROUTER_MODEL`, `get_api_key`
- `rust/src/cache.rs` — `compute_cache_key()` function (lines 63-91) — this is where the cache key collision fix goes

## Implementation

### Task 1: Add `OpenRouterClient` import to `main.rs`

Add alongside the existing `AnthropicClient` import (around line 38):

```rust
use crate::openrouter::OpenRouterClient;
```

The `mod openrouter;` declaration was added in step-02.

### Task 2: Rewrite `run_check()` provider wiring

Replace the current API key and evaluator setup in `run_check()`. The current code (approximately lines 255-319):

```rust
let api_key = get_api_key(config::Provider::Anthropic).context(
    "ANTHROPIC_API_KEY not set. Set the environment variable:\n  export ANTHROPIC_API_KEY=sk-ant-...",
)?;
// ... config struct ...
let stateless: Arc<dyn StatelessEvaluator> = Arc::new(
    AnthropicClient::new(api_key.clone())
        .map_err(|e| anyhow::anyhow!("failed to create Anthropic client: {}", e))?,
);
let agentic: Option<Arc<dyn AgenticEvaluator>> = match PiAgenticEvaluator::new(api_key) {
    Ok(e) => Some(Arc::new(e)),
    Err(e) => {
        eprintln!("Warning: agentic evaluator unavailable: {}", e);
        None
    }
};
```

Replace with provider-aware logic. The key change: API key reading and client construction now branch on `config.provider`.

**Step-by-step replacement in `run_check()`:**

1. **Move the model default resolution before the config struct literal.** The config struct literal currently uses `args.model` directly. Add model resolution logic before it:

```rust
    let provider: Provider = args.provider.into();

    let model = if args.model == config::DEFAULT_MODEL && provider == Provider::OpenRouter {
        config::DEFAULT_OPENROUTER_MODEL.to_string()
    } else {
        args.model.clone()
    };
```

Then in the config struct literal, use `model` instead of `args.model`:
```rust
        model,  // was: model: args.model,
```

And set provider:
```rust
        provider,  // was: provider: args.provider.into(),
```

(Remove the duplicate `args.provider.into()` if step-03 put it inside the struct literal.)

2. **Move the model-slash guard before API key reading** (it may already be there from step-03 — just ensure correct placement).

3. **Replace the API key and client construction block:**

```rust
    let api_key = get_api_key(provider).context(match provider {
        Provider::Anthropic => {
            "ANTHROPIC_API_KEY not set. Set the environment variable:\n  \
             export ANTHROPIC_API_KEY=sk-ant-..."
        }
        Provider::OpenRouter => {
            "OPENROUTER_API_KEY not set. Set the environment variable:\n  \
             export OPENROUTER_API_KEY=sk-or-..."
        }
    })?;

    let stateless: Arc<dyn StatelessEvaluator> = match provider {
        Provider::Anthropic => Arc::new(
            AnthropicClient::new(api_key.clone())
                .map_err(|e| anyhow::anyhow!("failed to create Anthropic client: {}", e))?,
        ),
        Provider::OpenRouter => Arc::new(
            OpenRouterClient::new(api_key.clone())
                .map_err(|e| anyhow::anyhow!("failed to create OpenRouter client: {}", e))?,
        ),
    };

    let anthropic_key_for_agentic = if provider == Provider::Anthropic {
        Some(api_key)
    } else {
        std::env::var("ANTHROPIC_API_KEY").ok()
    };

    let agentic: Option<Arc<dyn AgenticEvaluator>> = match anthropic_key_for_agentic {
        Some(key) => match PiAgenticEvaluator::new(key) {
            Ok(e) => Some(Arc::new(e)),
            Err(e) => {
                eprintln!("Warning: agentic evaluator unavailable: {}", e);
                None
            }
        },
        None => {
            eprintln!(
                "Warning: agentic evaluator unavailable (ANTHROPIC_API_KEY not set)"
            );
            None
        }
    };
```

### Task 3: Fix cache key to include provider

In `rust/src/cache.rs`, the `compute_cache_key()` function (around line 63) hashes `version`, `model`, `rules`, `path`, `content`, `diff`. It does NOT include provider. Two different providers using the same model string (unlikely but possible — e.g., a user aliasing) would collide.

Add provider to the hash. The function signature needs a new parameter.

**Change the function signature** from:

```rust
fn compute_cache_key(
    file_path: &str,
    content: Option<&str>,
    diff: &str,
    rules: &[Rule],
    model: &str,
) -> String {
```

To:

```rust
fn compute_cache_key(
    file_path: &str,
    content: Option<&str>,
    diff: &str,
    rules: &[Rule],
    model: &str,
    provider: &str,
) -> String {
```

Add after the `model` line in the hash computation:

```rust
    hasher.update(format!("provider:{}\n", provider));
```

**Find all call sites** of `compute_cache_key` in `cache.rs` and add the `provider` parameter. The call sites are in `CacheManager::key_for` (around line 238) and `NullCache::key_for` (around line 277) — NOT in `get()` or `put()`. The `Cache` trait has a `key_for(...)` method; update its signature to accept `provider: &str`, then update both impls.

Trace upward: `CacheManager` methods are called from `runner.rs` or wherever the cache is used. The `CheckConfig` now has `provider: Provider`. Pass `provider.as_str()` or similar.

**Add a display method to `Provider`:**

In `config.rs`, add:

```rust
impl Provider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Provider::Anthropic => "anthropic",
            Provider::OpenRouter => "openrouter",
        }
    }
}
```

Then thread `provider.as_str()` through the cache call chain. Trace the exact call path:

1. Read `cache.rs` fully to find all `compute_cache_key` call sites
2. Read `runner.rs` to find where `CacheManager` methods are called
3. Add `provider: &str` parameter to the `Cache` trait methods if needed, or to just `compute_cache_key` and the `CacheManager` methods that call it

**Important**: The `Cache` trait in `cache.rs` has methods `get(&self, key: &str)` and `put(...)`. The key is pre-computed before being passed. So `compute_cache_key` is called before `Cache::get/put`. Find where `compute_cache_key` is called and add the provider parameter there.

Read `cache.rs` fully to understand the call chain before making changes.

### Task 4: Remove unused import warning

After step-01, the `get_api_key` import used `config::Provider::Anthropic` as a hardcoded argument. This step replaces that with the dynamic `provider` variable. Ensure no dead code warnings remain.

## Acceptance Criteria

- [ ] `cd rust && cargo build 2>&1` exits 0
- [ ] `cd rust && cargo clippy -- -D warnings 2>&1` exits 0
- [ ] `cd rust && cargo nextest run 2>&1` exits 0
- [ ] `grep -q 'use crate::openrouter::OpenRouterClient' rust/src/main.rs` exits 0
- [ ] `grep -q 'Provider::OpenRouter => Arc::new' rust/src/main.rs` exits 0
- [ ] `grep -q 'provider' rust/src/cache.rs` exits 0 (provider included in cache key)
- [ ] `cd rust && cargo build && ./target/debug/agent-rules check --provider openrouter --files src/main.rs --repo . 2>&1 | grep -qi 'OPENROUTER_API_KEY'` exits 0 (correct env var error when key not set)
- [ ] `cd rust && cargo build && ./target/debug/agent-rules check --model anthropic/claude-haiku-4-5 --files src/main.rs --repo . 2>&1 | grep -q "looks like an OpenRouter"` exits 0 (model-slash guard triggers)

## Reviewer Instructions

1. Run all acceptance criteria commands
2. Verify `run_check()` reads the correct env var per provider
3. Verify model default resolution: `--provider openrouter` without `--model` uses `DEFAULT_OPENROUTER_MODEL`
4. Verify agentic evaluator falls back to `ANTHROPIC_API_KEY` when `--provider openrouter` is used
5. Verify `compute_cache_key` includes provider in the hash
6. Verify all `compute_cache_key` call sites pass the provider parameter
7. Check for no compiler warnings

## Rollback

```bash
cd rust
git checkout -- src/main.rs src/cache.rs src/config.rs
```
