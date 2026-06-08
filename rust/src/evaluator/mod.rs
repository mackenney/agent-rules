//! Evaluator protocols and implementations
//!
//! This module owns shared types (`LlmError`, `StatelessEvalOpts`, `AgenticEvalOpts`)
//! and the two evaluator traits (`StatelessEvaluator`, `AgenticEvaluator`).
//! Provider implementations live in submodules:
//! - `anthropic` — Anthropic API client (direct HTTP, tool-use protocol)
//! - `openrouter` — OpenRouter client (OpenAI-compatible chat completions)
//! - `agentic` — Pi-based agentic evaluator (subprocess, needs-more-context escalation)

#![allow(clippy::too_many_arguments)]

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

#[cfg(test)]
mod tests {
    use super::LlmError;

    #[test]
    fn test_error_retryable() {
        assert!(LlmError::RateLimit.is_retryable());
        assert!(LlmError::ServerError(500).is_retryable());
        assert!(LlmError::Timeout.is_retryable());
        assert!(!LlmError::Auth("unauthorized".into()).is_retryable());
        assert!(!LlmError::Parse("bad json".into()).is_retryable());
        assert!(!LlmError::Request("connection refused".into()).is_retryable());
        assert!(!LlmError::Exhausted.is_retryable());
    }
}
