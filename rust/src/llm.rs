//! Anthropic API client and retry logic

use thiserror::Error;

/// LLM-specific errors (needs retry classification)
#[derive(Debug, Error)]
pub enum LlmError {
    #[error("rate limited")]
    RateLimit,
    #[error("server error: {0}")]
    ServerError(u16),
    #[error("request failed: {0}")]
    Request(String),
    #[error("timeout")]
    Timeout,
    #[error("retries exhausted")]
    Exhausted,
}

/// Placeholder - will be implemented in step-05
pub struct AnthropicClient;
