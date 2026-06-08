# Step 01 — Create `evaluator/` Submodule

## Objective

Replace the flat files `evaluator.rs`, `llm.rs`, `openrouter.rs`, `agentic.rs` with a directory module `evaluator/` containing:
- `evaluator/mod.rs` — traits, opts, `LlmError`, retry constants
- `evaluator/anthropic.rs` — `AnthropicClient`
- `evaluator/openrouter.rs` — `OpenRouterClient`
- `evaluator/agentic.rs` — `PiAgenticEvaluator`

## Parallelizable

Yes — this step touches none of the files modified in Step 02. Can run in parallel with Step 02. Both must complete before Step 03.

## Actions

### 1. Create `rust/src/evaluator/` directory

### 2. Create `rust/src/evaluator/mod.rs`

Merge contents from current `evaluator.rs` and the shared types from `llm.rs`. The key changes:
- The `#![allow(...)]` attribute from old `evaluator.rs` line 1 becomes a module-level `#[allow(...)]` or is dropped (it was `#![allow(dead_code, clippy::too_many_arguments)]`; the `dead_code` suppression is unnecessary if all items are used, and `too_many_arguments` only applies in implementation files). Keep it as a module-level inner attribute.
- `LlmError` (definition + `is_retryable()` impl) moves here from `llm.rs`
- `MAX_RETRIES` and `RETRY_BASE_DELAY_MS` move here from `llm.rs`
- Remove `use crate::llm::LlmError` — it's now defined locally
- Re-export submodule public items for external access

Content for `evaluator/mod.rs`:

```rust
#![allow(dead_code, clippy::too_many_arguments)]
//! Evaluator protocols and implementations
//!
//! Traits (`StatelessEvaluator`, `AgenticEvaluator`), shared types (`LlmError`),
//! and all provider implementations (Anthropic, OpenRouter, agentic).

mod agentic;
mod anthropic;
mod openrouter;

pub use agentic::PiAgenticEvaluator;
pub use anthropic::AnthropicClient;
pub use openrouter::OpenRouterClient;

use async_trait::async_trait;
use std::time::Duration;
use thiserror::Error;

use crate::schema::{ContextHint, Rule, RuleVerdict};

pub(crate) const MAX_RETRIES: u32 = 3;
pub(crate) const RETRY_BASE_DELAY_MS: u64 = 1000;

/// LLM-specific errors with retry classification
#[derive(Debug, Error)]
pub enum LlmError {
    /// API rate limit exceeded (HTTP 429)
    #[error("rate limited")]
    RateLimit,

    /// Non-retryable server error with status code
    #[error("server error: {0}")]
    ServerError(u16),

    /// Request timed out
    #[error("timeout")]
    Timeout,

    /// Authentication failure (invalid or missing API key)
    #[error("auth error: {0}")]
    Auth(String),

    /// Network or HTTP request failure
    #[error("request error: {0}")]
    Request(String),

    /// Failed to parse the API response body
    #[error("failed to parse response: {0}")]
    Parse(String),

    /// All retry attempts exhausted without success
    #[error("retries exhausted")]
    Exhausted,
}

impl LlmError {
    /// Returns true if this error is worth retrying
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            LlmError::RateLimit | LlmError::ServerError(_) | LlmError::Timeout
        )
    }
}

/// Options for stateless evaluation
#[derive(Debug, Clone)]
pub struct StatelessEvalOpts {
    /// Model identifier (e.g., "claude-haiku-4-5")
    pub model: String,
    /// Per-call timeout
    pub timeout: Duration,
    /// Max diff characters (for validation, not truncation)
    pub max_diff_chars: usize,
    /// Max content characters (for validation, not truncation)
    pub max_content_chars: usize,
    /// Enable trace logging of prompts/responses
    pub trace: bool,
    /// Hint: system prompt will be reused across multiple calls
    pub cache_system_prompt: bool,
    /// Hint: file context will be reused across multiple rule calls
    pub cache_file_context: bool,
}

/// Options for agentic evaluation
#[derive(Debug, Clone)]
pub struct AgenticEvalOpts {
    /// Model identifier for agentic session (e.g., "claude-sonnet-4-6")
    pub model: String,
    /// Session timeout
    pub timeout: Duration,
    /// Allow bash tool in agentic session
    pub allow_bash: bool,
    /// Enable trace logging
    pub trace: bool,
}

/// Stateless evaluator trait — evaluates (file, rule) without filesystem access
///
/// Each call evaluates exactly ONE rule. The returned RuleVerdict's rule_id
/// MUST equal the provided rule.id — implementations set it from the argument,
/// never from model output.
#[async_trait]
pub trait StatelessEvaluator: Send + Sync {
    /// Evaluate a single rule against a file
    ///
    /// # Arguments
    /// * `file_path` - Path to the file being evaluated (relative to repo root)
    /// * `diff` - Annotated unified diff (may be empty for --files mode)
    /// * `content` - Full file content (may be None for deleted files)
    /// * `rule` - The single rule to evaluate
    /// * `is_new_file` - True if this is a newly added file
    /// * `opts` - Evaluation options
    ///
    /// # Returns
    /// A RuleVerdict with rule_id == rule.id. May have verdict = needs-more-context.
    ///
    /// # Errors
    /// Returns LlmError on non-retryable failures (auth errors).
    /// Retryable failures (timeout, rate limit) are handled internally.
    async fn evaluate(
        &self,
        file_path: &str,
        diff: &str,
        content: Option<&str>,
        rule: &Rule,
        is_new_file: bool,
        opts: &StatelessEvalOpts,
    ) -> Result<RuleVerdict, LlmError>;
}

/// Agentic evaluator trait — evaluates (file, rule) with filesystem read access
///
/// Invoked only on agentic escalation (stateless returned needs-more-context
/// on an agentic-typed rule).
#[async_trait]
pub trait AgenticEvaluator: Send + Sync {
    /// Evaluate a single rule with agentic capabilities
    ///
    /// # Arguments
    /// * `file_path` - Path to the file being evaluated
    /// * `diff` - Annotated unified diff
    /// * `content` - Full file content
    /// * `rule` - The rule to evaluate (always has context = agentic)
    /// * `hints` - Context hints from the stateless pass (files to read, question)
    /// * `repo_root` - Repository root for file access
    /// * `opts` - Evaluation options
    ///
    /// # Returns
    /// A RuleVerdict with from_agentic = true and verdict in {pass, fail}.
    /// NEVER returns needs-more-context (collapsed to fail internally).
    ///
    /// # Errors
    /// Returns LlmError on non-retryable failures.
    /// Timeout is handled internally (returns fallback fail verdict).
    async fn evaluate(
        &self,
        file_path: &str,
        diff: &str,
        content: Option<&str>,
        rule: &Rule,
        hints: &[ContextHint],
        repo_root: &std::path::Path,
        opts: &AgenticEvalOpts,
    ) -> Result<RuleVerdict, LlmError>;
}
```

### 3. Create `rust/src/evaluator/anthropic.rs`

Copy the entire content of current `llm.rs` with these changes:
- Remove: `LlmError` definition + `impl LlmError` (moved to `mod.rs`)
- Remove: `MAX_RETRIES`, `RETRY_BASE_DELAY_MS` constants (moved to `mod.rs`)
- Remove: `use thiserror::Error;` (no longer needed here)
- Change: `use crate::evaluator::{StatelessEvalOpts, StatelessEvaluator};` → `use super::{StatelessEvalOpts, StatelessEvaluator, LlmError, MAX_RETRIES, RETRY_BASE_DELAY_MS};`
- Remove: `use crate::prompt::{...}` stays but becomes `use crate::prompt::{...}`  (unchanged since it's still in the crate)
- Keep the `#[cfg(test)] mod tests` block with tests; update the `use super::*;` will automatically pick up the right items. The test that references `LlmError` directly (e.g., `test_llm_error_retryable`) will work because `use super::*` brings in the re-export via `use super::{..., LlmError, ...}`.

Key import block:

```rust
use serde::{Deserialize, Serialize};
use std::time::Duration;

use async_trait::async_trait;

use super::{LlmError, StatelessEvalOpts, StatelessEvaluator, MAX_RETRIES, RETRY_BASE_DELAY_MS};
use crate::prompt::{build_tool_schema, build_user_prompt, SYSTEM_PROMPT};
use crate::schema::{ContextHint, Rule, RuleContext, RuleVerdict, Verdict};
```

Everything else in the file stays identical. The `#[cfg(test)]` block's `use super::*` will pull in `LlmError` through the `use super::...` import at the top of the file. However, the test `test_llm_error_retryable` references `LlmError` variants directly — `super::*` will include the `LlmError` import. Add `use crate::schema::{RuleContext, Severity};` stays in the test module.

### 4. Create `rust/src/evaluator/openrouter.rs`

Copy current `openrouter.rs` (at `rust/src/openrouter.rs`) with these changes:
- Change: `use crate::evaluator::{StatelessEvalOpts, StatelessEvaluator};` → `use super::{StatelessEvalOpts, StatelessEvaluator};`
- Change: `use crate::llm::{LlmError, MAX_RETRIES, RETRY_BASE_DELAY_MS};` → `use super::{LlmError, MAX_RETRIES, RETRY_BASE_DELAY_MS};`
- The rest is identical.

Key import block:

```rust
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::time::Duration;

use super::{LlmError, StatelessEvalOpts, StatelessEvaluator, MAX_RETRIES, RETRY_BASE_DELAY_MS};
use crate::prompt::{build_tool_schema, build_user_prompt, SYSTEM_PROMPT};
use crate::schema::{ContextHint, Rule, RuleContext, RuleVerdict, Verdict};
```

### 5. Create `rust/src/evaluator/agentic.rs`

Copy current `agentic.rs` with these changes:
- Change: `use crate::evaluator::{AgenticEvalOpts, AgenticEvaluator};` → `use super::{AgenticEvalOpts, AgenticEvaluator, LlmError};`
- Remove: `use crate::llm::LlmError;` (now comes from `super`)
- The rest is identical.

Key import block:

```rust
use std::path::{Path, PathBuf};
use std::process::Stdio;

use async_trait::async_trait;
use tokio::process::Command;
use tokio::time::timeout;

use super::{AgenticEvalOpts, AgenticEvaluator, LlmError};
use crate::prompt::build_agentic_task;
use crate::schema::{ContextHint, Rule, RuleVerdict, Verdict};
```

## DO NOT do in this step

- Do not delete old files yet (`evaluator.rs`, `llm.rs`, `openrouter.rs`, `agentic.rs`)
- Do not modify `lib.rs` or `main.rs`
- Those happen in Step 03

## Acceptance Criteria

Files exist and have valid Rust syntax:

```bash
cd rust
test -f src/evaluator/mod.rs
test -f src/evaluator/anthropic.rs
test -f src/evaluator/openrouter.rs
test -f src/evaluator/agentic.rs
```

Note: Build won't succeed until Step 03 wires everything together.
