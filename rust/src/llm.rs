//! Anthropic API client with retry logic
//!
//! Uses reqwest for HTTP, thiserror for typed errors that support retry classification.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

use crate::prompt::{SYSTEM_PROMPT, build_tool_schema, build_user_prompt, truncate_to_chars};
use crate::schema::{Rule, RuleVerdict, Severity, Verdict};

const API_BASE_URL: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2023-06-01";
const MAX_RETRIES: u32 = 3;
const RETRY_BASE_DELAY_MS: u64 = 1000;

/// LLM-specific errors with retry classification
#[derive(Debug, Error)]
pub enum LlmError {
    #[error("rate limited")]
    RateLimit,

    #[error("server error: {0}")]
    ServerError(u16),

    #[error("timeout")]
    Timeout,

    #[error("auth error: {0}")]
    Auth(String),

    #[error("request error: {0}")]
    Request(String),

    #[error("failed to parse response: {0}")]
    Parse(String),

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

/// Anthropic API client
pub struct AnthropicClient {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl AnthropicClient {
    /// Create a new client with the Anthropic production endpoint
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

    /// Create client with a custom base URL (for testing)
    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        let mut c = Self::new(api_key);
        c.base_url = base_url;
        c
    }

    /// Evaluate a file against rules, returning one verdict per rule
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
        let diff = truncate_to_chars(diff, max_diff_chars);
        let content = content.map(|c| truncate_to_chars(c, max_content_chars));

        let user_prompt =
            build_user_prompt(file_path, &diff, content.as_deref(), rules, is_new_file);

        let request = MessagesRequest {
            model,
            max_tokens: 2048,
            system: SYSTEM_PROMPT,
            messages: vec![Message {
                role: "user",
                content: &user_prompt,
            }],
            tools: vec![build_tool_schema()],
            tool_choice: ToolChoice {
                type_: "tool",
                name: Some("submit_verdict"),
                disable_parallel_tool_use: None,
            },
        };

        let response = match self.call_with_retry(&request, timeout).await {
            Ok(r) => r,
            Err(LlmError::Auth(msg)) => return Err(LlmError::Auth(msg)),
            Err(_) => {
                return Ok(rules
                    .iter()
                    .map(|rule| RuleVerdict {
                        rule_id: rule.id.clone(),
                        rule_name: rule.name.clone(),
                        verdict: Verdict::Fail,
                        confidence: 0.0,
                        reasoning: "LLM call failed".to_string(),
                        severity: rule.severity,
                        line_refs: vec![],
                        line: None,
                        cached: false,
                    })
                    .collect());
            }
        };

        self.parse_verdicts(&response, rules)
    }

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
            401 | 403 => {
                let text = response.text().await.unwrap_or_default();
                Err(LlmError::Auth(text))
            }
            429 => Err(LlmError::RateLimit),
            500..=599 => Err(LlmError::ServerError(status)),
            _ => {
                let text = response.text().await.unwrap_or_default();
                Err(LlmError::Request(format!("HTTP {}: {}", status, text)))
            }
        }
    }

    fn parse_verdicts(
        &self,
        response: &MessagesResponse,
        rules: &[Rule],
    ) -> Result<Vec<RuleVerdict>, LlmError> {
        let mut verdicts: Vec<RuleVerdict> = Vec::new();

        let rule_map: std::collections::HashMap<&str, &Rule> =
            rules.iter().map(|r| (r.id.as_str(), r)).collect();

        for block in &response.content {
            if block.type_ == "tool_use" && block.name.as_deref() == Some("submit_verdict") {
                if let Some(input) = &block.input {
                    let rule_id = input.get("rule_id").and_then(|v| v.as_str()).unwrap_or("");

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

                    let line_refs: Vec<u64> = input
                        .get("line_refs")
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.iter().filter_map(|v| v.as_u64()).collect())
                        .unwrap_or_default();
                    let line = line_refs.first().copied().map(|l| l as u32);
                    let line_refs_u32: Vec<u32> = line_refs
                        .iter()
                        .filter_map(|&l| u32::try_from(l).ok())
                        .collect();

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
                        line_refs: line_refs_u32,
                        line,
                        cached: false,
                    });
                }
            }
        }

        for rule in rules {
            if !verdicts.iter().any(|v| v.rule_id == rule.id) {
                verdicts.push(RuleVerdict {
                    rule_id: rule.id.clone(),
                    rule_name: rule.name.clone(),
                    verdict: Verdict::Fail,
                    confidence: 0.0,
                    reasoning: "No verdict received from LLM".to_string(),
                    severity: rule.severity,
                    line_refs: vec![],
                    line: None,
                    cached: false,
                });
            }
        }

        Ok(verdicts)
    }
}

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
    name: Option<&'a str>,
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
        assert!(!LlmError::Auth("unauthorized".into()).is_retryable());
        assert!(!LlmError::Parse("bad json".into()).is_retryable());
        assert!(!LlmError::Request("connection refused".into()).is_retryable());
        assert!(!LlmError::Exhausted.is_retryable());
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

    #[test]
    fn test_parse_verdicts_tool_use() {
        let client = AnthropicClient::new("test-key".to_string());

        let input = serde_json::json!({
            "rule_id": "rule-1",
            "verdict": "pass",
            "confidence": 0.9,
            "reasoning": "looks good",
            "line_refs": []
        });

        let response = MessagesResponse {
            content: vec![ContentBlock {
                type_: "tool_use".to_string(),
                name: Some("submit_verdict".to_string()),
                input: Some(input),
                text: None,
            }],
            stop_reason: Some("tool_use".to_string()),
        };

        let rules = vec![Rule {
            id: "rule-1".to_string(),
            name: "Rule 1".to_string(),
            prompt: "test".to_string(),
            severity: Severity::Warn,
            enabled: true,
            context: Default::default(),
            glob_include: vec![],
            glob_exclude: vec![],
            examples: vec![],
            needs_more_context_when: String::new(),
        }];

        let verdicts = client.parse_verdicts(&response, &rules).unwrap();
        assert_eq!(verdicts.len(), 1);
        assert_eq!(verdicts[0].verdict, Verdict::Pass);
        assert_eq!(verdicts[0].confidence, 0.9);
    }
}
