# Step 05: Prompt & LLM

## Context

### Overall Objective
Build a Rust CLI that checks PR diffs against LLM-powered rules defined in `.agent-rules.toml` files. Commands: `check`, `cache stats`, `cache clear`, `rules list`, `rules validate`.

### Phase Context
Wave 3 — This step can run in parallel with step-07 (reporter). It depends on step-04 (resolver/cache) being complete. Implements the LLM integration: prompt building and Anthropic API client with retry logic.

### This Step
Implement prompt building (system prompt, user prompt with file context) and the Anthropic HTTP client using reqwest. Includes retry logic with exponential backoff for rate limits and server errors. Uses tool_choice to force the LLM to use the submit_verdict tool.

## Prerequisites
- Step 02 complete (schema types)
- Step 04 complete (resolver/cache for FileCheckRequest structure)

## Files to Read Before Starting
- `rust/src/prompt.rs` — Replace the placeholder stub
- `rust/src/llm.rs` — Replace the placeholder stub
- `rust/src/schema.rs` — Understand Rule, Verdict, RuleVerdict types

## Implementation

### Task 1: Implement prompt.rs

Replace `rust/src/prompt.rs` with:

```rust
//! Prompt building for LLM evaluation calls
//!
//! Builds system and user prompts following the TypeScript implementation.
//! The LLM evaluates code changes against rules and returns verdicts.

use crate::parser::add_line_numbers;
use crate::schema::Rule;

/// System prompt for the code review assistant
pub const SYSTEM_PROMPT: &str = r#"You are a precise code reviewer that evaluates code changes against specific rules.

For each rule, you must:
1. Analyze the provided diff and file content
2. Determine if the code violates the rule
3. Provide your verdict using the submit_verdict tool

Guidelines:
- Focus ONLY on the changed lines (lines starting with + in the diff)
- Consider the full file context when evaluating
- Be precise: only flag actual violations, not potential ones
- Provide clear, actionable reasoning for failures
- Include the specific line number where the violation occurs (if applicable)

Verdicts:
- "pass": The code follows the rule
- "fail": The code violates the rule
- "needs-more-context": Cannot determine (will be treated as fail)"#;

/// Build the user prompt for a file check
pub fn build_user_prompt(
    file_path: &str,
    diff: &str,
    content: Option<&str>,
    rules: &[Rule],
    is_new_file: bool,
) -> String {
    let mut prompt = String::new();

    // File header
    prompt.push_str(&format!("## File: {}\n\n", file_path));

    if is_new_file {
        prompt.push_str("This is a newly added file.\n\n");
    }

    // Diff section
    prompt.push_str("### Changes (diff)\n\n");
    prompt.push_str("```diff\n");
    prompt.push_str(diff);
    if !diff.ends_with('\n') {
        prompt.push('\n');
    }
    prompt.push_str("```\n\n");

    // Full file content (if available)
    if let Some(content) = content {
        prompt.push_str("### Full file content (with line numbers)\n\n");
        prompt.push_str("```\n");
        prompt.push_str(&add_line_numbers(content));
        prompt.push_str("\n```\n\n");
    }

    // Rules section
    prompt.push_str("### Rules to evaluate\n\n");
    for (i, rule) in rules.iter().enumerate() {
        prompt.push_str(&format!(
            "{}. **{}** (`{}`)\n",
            i + 1,
            rule.name,
            rule.id
        ));
        prompt.push_str(&format!("   Severity: {}\n", rule.severity));
        prompt.push_str(&format!("   {}\n\n", rule.prompt));

        // Include examples if present
        if !rule.examples.is_empty() {
            prompt.push_str("   Examples:\n");
            for example in &rule.examples {
                let pass_fail = if example.should_pass { "✓ pass" } else { "✗ fail" };
                prompt.push_str(&format!("   - {} `{}`", pass_fail, example.code));
                if !example.explanation.is_empty() {
                    prompt.push_str(&format!(" — {}", example.explanation));
                }
                prompt.push('\n');
            }
            prompt.push('\n');
        }
    }

    prompt.push_str("\nEvaluate each rule and submit your verdict using the submit_verdict tool.\n");

    prompt
}

/// Build the tool schema for submit_verdict
pub fn build_tool_schema() -> serde_json::Value {
    serde_json::json!({
        "name": "submit_verdict",
        "description": "Submit your verdict for a rule evaluation",
        "input_schema": {
            "type": "object",
            "properties": {
                "rule_id": {
                    "type": "string",
                    "description": "The ID of the rule being evaluated"
                },
                "verdict": {
                    "type": "string",
                    "enum": ["pass", "fail", "needs-more-context"],
                    "description": "Your verdict: pass if the code follows the rule, fail if it violates the rule"
                },
                "confidence": {
                    "type": "number",
                    "minimum": 0,
                    "maximum": 1,
                    "description": "Confidence in your verdict (0-1)"
                },
                "reasoning": {
                    "type": "string",
                    "description": "Brief explanation for your verdict"
                },
                "line": {
                    "type": "integer",
                    "description": "Line number where the violation occurs (for fail verdicts)"
                }
            },
            "required": ["rule_id", "verdict", "confidence", "reasoning"]
        }
    })
}

/// Truncate content to max chars, preserving line boundaries
pub fn truncate_to_chars(content: &str, max_chars: usize) -> String {
    if content.len() <= max_chars {
        return content.to_string();
    }

    // Find a good break point (end of line) before max_chars
    let truncated: String = content.chars().take(max_chars).collect();
    if let Some(last_newline) = truncated.rfind('\n') {
        format!("{}\n... (truncated)", &truncated[..last_newline])
    } else {
        format!("{}... (truncated)", truncated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Severity;

    fn make_test_rule() -> Rule {
        Rule {
            id: "test-rule".to_string(),
            name: "Test Rule".to_string(),
            prompt: "Check for test issues".to_string(),
            severity: Severity::Warn,
            enabled: true,
            context: Default::default(),
            glob_include: vec!["**/*".to_string()],
            glob_exclude: vec![],
            examples: vec![],
            needs_more_context_when: String::new(),
        }
    }

    #[test]
    fn test_build_user_prompt_basic() {
        let rules = vec![make_test_rule()];
        let prompt = build_user_prompt(
            "src/main.rs",
            "+new line",
            Some("fn main() {}"),
            &rules,
            false,
        );

        assert!(prompt.contains("## File: src/main.rs"));
        assert!(prompt.contains("### Changes (diff)"));
        assert!(prompt.contains("+new line"));
        assert!(prompt.contains("### Full file content"));
        assert!(prompt.contains("### Rules to evaluate"));
        assert!(prompt.contains("test-rule"));
    }

    #[test]
    fn test_build_user_prompt_new_file() {
        let prompt = build_user_prompt("new.rs", "+content", None, &[], true);
        assert!(prompt.contains("newly added file"));
    }

    #[test]
    fn test_build_tool_schema() {
        let schema = build_tool_schema();
        assert_eq!(schema["name"], "submit_verdict");
        assert!(schema["input_schema"]["properties"]["verdict"].is_object());
    }

    #[test]
    fn test_truncate_to_chars() {
        let content = "line1\nline2\nline3\nline4";
        let truncated = truncate_to_chars(content, 12);
        assert!(truncated.contains("truncated"));
        assert!(truncated.len() < content.len() + 20);
    }

    #[test]
    fn test_truncate_no_op_short() {
        let content = "short";
        let result = truncate_to_chars(content, 100);
        assert_eq!(result, content);
    }
}
```

### Task 2: Implement llm.rs

Replace `rust/src/llm.rs` with:

```rust
//! Anthropic API client with retry logic
//!
//! Uses reqwest for HTTP, thiserror for typed errors that support retry classification.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

use crate::prompt::{build_tool_schema, build_user_prompt, truncate_to_chars, SYSTEM_PROMPT};
use crate::schema::{Rule, RuleVerdict, Severity, Verdict};

/// Default Anthropic API base URL
const API_BASE_URL: &str = "https://api.anthropic.com";

/// API version header
const API_VERSION: &str = "2023-06-01";

/// Max retries for transient errors
const MAX_RETRIES: u32 = 3;

/// Base delay for exponential backoff (ms)
const RETRY_BASE_DELAY_MS: u64 = 1000;

/// LLM-specific errors with retry classification
#[derive(Debug, Error)]
pub enum LlmError {
    #[error("rate limited")]
    RateLimit,

    #[error("server error: {0}")]
    ServerError(u16),

    #[error("authentication failed")]
    Auth,

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("request failed: {0}")]
    Request(String),

    #[error("timeout")]
    Timeout,

    #[error("failed to parse response: {0}")]
    Parse(String),

    #[error("retries exhausted")]
    Exhausted,
}

impl LlmError {
    /// Check if this error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(self, LlmError::RateLimit | LlmError::ServerError(_) | LlmError::Timeout)
    }
}

/// Anthropic API client
pub struct AnthropicClient {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl AnthropicClient {
    /// Create a new client
    pub fn new(api_key: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .pool_max_idle_per_host(20)
            .pool_idle_timeout(Duration::from_secs(90))
            .build()
            .expect("failed to build HTTP client");

        Self {
            client,
            api_key,
            base_url: API_BASE_URL.to_string(),
        }
    }

    /// Create client with custom base URL (for testing)
    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        let mut client = Self::new(api_key);
        client.base_url = base_url;
        client
    }

    /// Evaluate a file against rules
    pub async fn evaluate(
        &self,
        file_path: &str,
        diff: &str,
        content: Option<&str>,
        rules: &[Rule],
        is_new_file: bool,
        model: &str,
        max_diff_chars: usize,
        max_content_chars: usize,
        timeout: Duration,
    ) -> Result<Vec<RuleVerdict>, LlmError> {
        // Truncate if needed
        let diff = truncate_to_chars(diff, max_diff_chars);
        let content = content.map(|c| truncate_to_chars(c, max_content_chars));

        let user_prompt = build_user_prompt(
            file_path,
            &diff,
            content.as_deref(),
            rules,
            is_new_file,
        );

        // Build request
        let request = MessagesRequest {
            model,
            max_tokens: 4096,
            system: SYSTEM_PROMPT,
            messages: vec![Message {
                role: "user",
                content: &user_prompt,
            }],
            tools: vec![build_tool_schema()],
            tool_choice: ToolChoice {
                type_: "any",
                disable_parallel_tool_use: Some(false),
            },
        };

        // Call with retry
        let response = self.call_with_retry(&request, timeout).await?;

        // Parse verdicts from tool calls
        self.parse_verdicts(&response, rules)
    }

    /// Make API call with retry logic
    async fn call_with_retry(
        &self,
        request: &MessagesRequest<'_>,
        timeout: Duration,
    ) -> Result<MessagesResponse, LlmError> {
        let mut last_error = LlmError::Exhausted;

        for attempt in 0..MAX_RETRIES {
            match self.call_once(request, timeout).await {
                Ok(response) => return Ok(response),
                Err(e) if e.is_retryable() && attempt < MAX_RETRIES - 1 => {
                    let delay = RETRY_BASE_DELAY_MS * 2u64.pow(attempt);
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    last_error = e;
                }
                Err(e) => return Err(e),
            }
        }

        Err(last_error)
    }

    /// Single API call
    async fn call_once(
        &self,
        request: &MessagesRequest<'_>,
        timeout: Duration,
    ) -> Result<MessagesResponse, LlmError> {
        let url = format!("{}/v1/messages", self.base_url);

        let response = tokio::time::timeout(
            timeout,
            self.client
                .post(&url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", API_VERSION)
                .header("content-type", "application/json")
                .json(request)
                .send(),
        )
        .await
        .map_err(|_| LlmError::Timeout)?
        .map_err(|e| LlmError::Request(e.to_string()))?;

        let status = response.status().as_u16();

        match status {
            200 => {
                let body = response
                    .json::<MessagesResponse>()
                    .await
                    .map_err(|e| LlmError::Parse(e.to_string()))?;
                Ok(body)
            }
            401 | 403 => Err(LlmError::Auth),
            429 => Err(LlmError::RateLimit),
            400 => {
                let text = response.text().await.unwrap_or_default();
                Err(LlmError::BadRequest(text))
            }
            500..=599 => Err(LlmError::ServerError(status)),
            _ => {
                let text = response.text().await.unwrap_or_default();
                Err(LlmError::Request(format!("HTTP {}: {}", status, text)))
            }
        }
    }

    /// Parse verdicts from API response
    fn parse_verdicts(
        &self,
        response: &MessagesResponse,
        rules: &[Rule],
    ) -> Result<Vec<RuleVerdict>, LlmError> {
        let mut verdicts = Vec::new();

        // Build a map of rule_id -> Rule for lookups
        let rule_map: std::collections::HashMap<&str, &Rule> =
            rules.iter().map(|r| (r.id.as_str(), r)).collect();

        for content in &response.content {
            if content.type_ == "tool_use" && content.name.as_deref() == Some("submit_verdict") {
                if let Some(input) = &content.input {
                    let rule_id = input
                        .get("rule_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    let verdict_str = input
                        .get("verdict")
                        .and_then(|v| v.as_str())
                        .unwrap_or("fail");

                    let verdict = match verdict_str {
                        "pass" => Verdict::Pass,
                        "needs-more-context" => Verdict::NeedsMoreContext,
                        _ => Verdict::Fail,
                    };

                    let confidence = input
                        .get("confidence")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.5);

                    let reasoning = input
                        .get("reasoning")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    let line = input
                        .get("line")
                        .and_then(|v| v.as_u64())
                        .map(|l| l as u32);

                    // Get rule name and severity from rule map
                    let (rule_name, severity) = rule_map
                        .get(rule_id)
                        .map(|r| (r.name.clone(), r.severity))
                        .unwrap_or_else(|| (rule_id.to_string(), Severity::Warn));

                    verdicts.push(RuleVerdict {
                        rule_id: rule_id.to_string(),
                        rule_name,
                        verdict,
                        confidence,
                        reasoning,
                        severity,
                        line,
                        cached: false,
                    });
                }
            }
        }

        // Ensure we have verdicts for all rules (mark missing as fail)
        for rule in rules {
            if !verdicts.iter().any(|v| v.rule_id == rule.id) {
                verdicts.push(RuleVerdict {
                    rule_id: rule.id.clone(),
                    rule_name: rule.name.clone(),
                    verdict: Verdict::Fail,
                    confidence: 0.0,
                    reasoning: "No verdict received from LLM".to_string(),
                    severity: rule.severity,
                    line: None,
                    cached: false,
                });
            }
        }

        Ok(verdicts)
    }
}

// ============================================================================
// API request/response types
// ============================================================================

#[derive(Debug, Serialize)]
struct MessagesRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    system: &'a str,
    messages: Vec<Message<'a>>,
    tools: Vec<serde_json::Value>,
    tool_choice: ToolChoice<'a>,
}

#[derive(Debug, Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Serialize)]
struct ToolChoice<'a> {
    #[serde(rename = "type")]
    type_: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    disable_parallel_tool_use: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct MessagesResponse {
    content: Vec<ContentBlock>,
    #[allow(dead_code)]
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    type_: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    input: Option<serde_json::Value>,
    #[allow(dead_code)]
    #[serde(default)]
    text: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_llm_error_retryable() {
        assert!(LlmError::RateLimit.is_retryable());
        assert!(LlmError::ServerError(500).is_retryable());
        assert!(LlmError::Timeout.is_retryable());
        assert!(!LlmError::Auth.is_retryable());
        assert!(!LlmError::BadRequest("x".into()).is_retryable());
    }

    #[test]
    fn test_parse_verdicts_missing_rules() {
        let client = AnthropicClient::new("test-key".to_string());

        let response = MessagesResponse {
            content: vec![],
            stop_reason: Some("end_turn".to_string()),
        };

        let rules = vec![Rule {
            id: "rule-1".to_string(),
            name: "Rule 1".to_string(),
            prompt: "test".to_string(),
            severity: Severity::Error,
            enabled: true,
            context: Default::default(),
            glob_include: vec![],
            glob_exclude: vec![],
            examples: vec![],
            needs_more_context_when: String::new(),
        }];

        let verdicts = client.parse_verdicts(&response, &rules).unwrap();
        assert_eq!(verdicts.len(), 1);
        assert_eq!(verdicts[0].verdict, Verdict::Fail);
        assert!(verdicts[0].reasoning.contains("No verdict"));
    }
}
```

## Acceptance Criteria

These must ALL pass before reporting complete:

- [ ] `cd rust && cargo build 2>&1 | grep -E "^error" | wc -l` — outputs `0`
- [ ] `cd rust && cargo test prompt:: 2>&1 | grep -E "^test result"` — shows `ok` with 0 failed
- [ ] `cd rust && cargo test llm:: 2>&1 | grep -E "^test result"` — shows `ok` with 0 failed
- [ ] `grep -c "pub fn build_user_prompt" rust/src/prompt.rs` — outputs `1`
- [ ] `grep -c "pub fn build_tool_schema" rust/src/prompt.rs` — outputs `1`
- [ ] `grep -c "pub struct AnthropicClient" rust/src/llm.rs` — outputs `1`
- [ ] `grep -c "pub enum LlmError" rust/src/llm.rs` — outputs `1`
- [ ] `grep -c "is_retryable" rust/src/llm.rs` — outputs at least `2`
- [ ] No regressions: `cd rust && cargo test 2>&1 | grep -E "^test result"` — shows 0 failed

## Reviewer Instructions

You are reviewing Step 05. Verify:

1. Run `cd rust && cargo test prompt::` — all tests pass
2. Run `cd rust && cargo test llm::` — all tests pass
3. Check `rust/src/prompt.rs` contains:
   - `SYSTEM_PROMPT` constant
   - `build_user_prompt()` building file context with rules
   - `build_tool_schema()` returning submit_verdict JSON schema
   - `truncate_to_chars()` for content limiting
4. Check `rust/src/llm.rs` contains:
   - `LlmError` enum with RateLimit, ServerError, Timeout, Auth, etc.
   - `is_retryable()` method on LlmError
   - `AnthropicClient` with `evaluate()` method
   - Exponential backoff retry logic
   - Response parsing extracting verdicts from tool_use blocks
   - Fallback to Fail for missing rule verdicts
5. Verify tool_choice uses `"type": "any"` to force tool use
6. Run `cd rust && cargo clippy 2>&1 | grep "^error"` — no errors

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong>"

## Rollback
```bash
git checkout HEAD -- rust/src/prompt.rs rust/src/llm.rs
```
