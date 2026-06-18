//! Anthropic API client with retry logic
//!
//! Uses reqwest for HTTP. Retry and error classification via `LlmError` (defined in the parent module).

use serde::{Deserialize, Serialize};
use std::time::Duration;

use async_trait::async_trait;

use super::helpers::{make_fail_verdict, parse_verdict_from_input, retry_with_backoff};
use super::{LlmError, StatelessEvalOpts, StatelessEvaluator};
use crate::prompt::{SYSTEM_PROMPT, build_tool_schema, build_user_prompt};
use crate::schema::{Rule, RuleVerdict};

const API_BASE_URL: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2023-06-01";

/// Anthropic API client
pub struct AnthropicClient {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl AnthropicClient {
    /// Create a new client with the Anthropic production endpoint
    ///
    /// # Errors
    /// Returns an error if the HTTP client cannot be constructed.
    pub fn new(api_key: String) -> Result<Self, LlmError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .pool_max_idle_per_host(20)
            .pool_idle_timeout(Duration::from_secs(90))
            .build()
            .map_err(|e| LlmError::Request(format!("failed to build HTTP client: {}", e)))?;

        Ok(Self {
            client,
            api_key,
            base_url: API_BASE_URL.to_string(),
        })
    }

    /// Evaluate a file against a single rule, returning one verdict
    ///
    /// Backward-compatible wrapper; prefer using `StatelessEvaluator` for new code.
    #[allow(dead_code)]
    #[allow(clippy::too_many_arguments)]
    pub async fn evaluate(
        &self,
        file_path: &str,
        diff: &str,
        content: Option<&str>,
        rule: &Rule,
        is_new_file: bool,
        model: &str,
        max_diff_chars: usize,
        max_content_chars: usize,
        timeout: Duration,
    ) -> Result<RuleVerdict, LlmError> {
        self.evaluate_internal(
            file_path,
            diff,
            content,
            rule,
            is_new_file,
            model,
            max_diff_chars,
            max_content_chars,
            timeout,
        )
        .await
    }

    /// Evaluate a file against a single rule, returning one verdict
    #[allow(clippy::too_many_arguments)]
    async fn evaluate_internal(
        &self,
        file_path: &str,
        diff: &str,
        content: Option<&str>,
        rule: &Rule,
        is_new_file: bool,
        model: &str,
        _max_diff_chars: usize,
        _max_content_chars: usize,
        timeout: Duration,
    ) -> Result<RuleVerdict, LlmError> {
        let user_prompt = build_user_prompt(file_path, diff, content, rule, is_new_file);

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
            Err(LlmError::Exhausted) => {
                return Ok(make_fail_verdict(rule, "LLM call failed after retries"));
            }
            Err(e) => return Err(e),
        };

        self.parse_verdict(&response, rule)
    }

    async fn call_with_retry(
        &self,
        request: &MessagesRequest<'_>,
        timeout: Duration,
    ) -> Result<MessagesResponse, LlmError> {
        retry_with_backoff(|| self.call_once(request, timeout)).await
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

    fn parse_verdict(
        &self,
        response: &MessagesResponse,
        rule: &Rule,
    ) -> Result<RuleVerdict, LlmError> {
        for block in &response.content {
            if block.type_ == "tool_use" && block.name.as_deref() == Some("submit_verdict") {
                if let Some(input) = &block.input {
                    return Ok(parse_verdict_from_input(input, rule));
                }
            }
        }

        Ok(make_fail_verdict(rule, "No verdict received from LLM"))
    }
}

#[async_trait]
impl StatelessEvaluator for AnthropicClient {
    async fn evaluate(
        &self,
        file_path: &str,
        diff: &str,
        content: Option<&str>,
        rule: &Rule,
        is_new_file: bool,
        opts: &StatelessEvalOpts,
    ) -> Result<RuleVerdict, LlmError> {
        self.evaluate_internal(
            file_path,
            diff,
            content,
            rule,
            is_new_file,
            &opts.model,
            opts.max_diff_chars,
            opts.max_content_chars,
            opts.timeout,
        )
        .await
    }
    async fn normalize(
        &self,
        raw_output: &str,
        rule: &Rule,
        file_path: &str,
        model: &str,
        timeout: Duration,
        trace: bool,
    ) -> Result<RuleVerdict, LlmError> {
        let system = "Extract a structured rule evaluation verdict from the agent's analysis. \
                      The agent has already done the investigation.";

        let user_content = format!(
            "File: {}\nRule: {} \u{2014} {}\n\nAgent analysis:\n{}\n\nExtract the verdict.",
            file_path,
            rule.id,
            rule.name,
            &raw_output[..raw_output.len().min(8000)]
        );

        let tool = serde_json::json!({
            "name": "submit_verdict",
            "description": "Submit the extracted verdict",
            "input_schema": {
                "type": "object",
                "properties": {
                    "verdict": {
                        "type": "string",
                        "enum": ["pass", "fail"]
                    },
                    "confidence": {
                        "type": "number",
                        "minimum": 0,
                        "maximum": 1
                    },
                    "reasoning": {
                        "type": "string"
                    },
                    "line_refs": {
                        "type": "array",
                        "items": {"type": "integer"}
                    }
                },
                "required": ["verdict", "confidence", "reasoning", "line_refs"]
            }
        });

        let request = MessagesRequest {
            model,
            max_tokens: 1024,
            system,
            messages: vec![Message {
                role: "user",
                content: &user_content,
            }],
            tools: vec![tool],
            tool_choice: ToolChoice {
                type_: "tool",
                name: Some("submit_verdict"),
                disable_parallel_tool_use: None,
            },
        };

        if trace {
            eprintln!("[TRACE] Normalization request for {}", file_path);
        }

        let response = match self.call_with_retry(&request, timeout).await {
            Ok(r) => r,
            Err(LlmError::Exhausted) => {
                return Ok(make_fail_verdict(
                    rule,
                    "Normalization failed after retries",
                ));
            }
            Err(e) => return Err(e),
        };

        if trace {
            eprintln!("[TRACE] Normalization response: {:?}", response);
        }

        for block in &response.content {
            if block.type_ == "tool_use" && block.name.as_deref() == Some("submit_verdict") {
                if let Some(input) = &block.input {
                    let mut verdict = parse_verdict_from_input(input, rule);
                    verdict.from_agentic = true;
                    verdict.context_hint = None;
                    return Ok(verdict);
                }
            }
        }

        Ok(make_fail_verdict(
            rule,
            "No verdict received from normalization",
        ))
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
    use crate::schema::{RuleContext, Severity, Verdict};

    #[test]
    fn test_parse_verdict_missing_tool_use() {
        let client = AnthropicClient::new("test-key".to_string()).unwrap();

        let response = MessagesResponse {
            content: vec![],
            stop_reason: Some("end_turn".to_string()),
        };

        let rule = Rule {
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
        };

        let verdict = client.parse_verdict(&response, &rule).unwrap();
        assert_eq!(verdict.verdict, Verdict::Fail);
        assert!(verdict.reasoning.contains("No verdict"));
        assert_eq!(verdict.rule_id, "rule-1");
    }

    #[test]
    fn test_parse_verdict_tool_use() {
        let client = AnthropicClient::new("test-key".to_string()).unwrap();

        let input = serde_json::json!({
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

        let rule = Rule {
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
        };

        let verdict = client.parse_verdict(&response, &rule).unwrap();
        assert_eq!(verdict.verdict, Verdict::Pass);
        assert_eq!(verdict.confidence, 0.9);
        assert_eq!(verdict.rule_id, "rule-1");
    }

    #[test]
    fn test_nmc_collapse_stateless_rule() {
        let client = AnthropicClient::new("test-key".to_string()).unwrap();

        let input = serde_json::json!({
            "verdict": "needs-more-context",
            "confidence": 0.5,
            "reasoning": "Need to check imports",
            "line_refs": [],
            "context_hint": {
                "read_files": ["src/utils.rs"],
                "question": "What does utils export?"
            }
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

        let rule = Rule {
            id: "test-rule".to_string(),
            name: "Test Rule".to_string(),
            prompt: "test".to_string(),
            severity: Severity::Warn,
            enabled: true,
            context: RuleContext::Stateless,
            glob_include: vec![],
            glob_exclude: vec![],
            examples: vec![],
            needs_more_context_when: String::new(),
        };

        let verdict = client.parse_verdict(&response, &rule).unwrap();

        assert_eq!(verdict.verdict, Verdict::Fail);
        assert!(
            verdict
                .reasoning
                .contains("[collapsed from needs-more-context: stateless rule]"),
            "reasoning should contain collapse annotation: {}",
            verdict.reasoning
        );
        assert!(verdict.context_hint.is_some());
    }

    #[test]
    fn test_nmc_not_collapsed_agentic_rule() {
        let client = AnthropicClient::new("test-key".to_string()).unwrap();

        let input = serde_json::json!({
            "verdict": "needs-more-context",
            "confidence": 0.5,
            "reasoning": "Need to check imports",
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

        let rule = Rule {
            id: "test-rule".to_string(),
            name: "Test Rule".to_string(),
            prompt: "test".to_string(),
            severity: Severity::Warn,
            enabled: true,
            context: RuleContext::Agentic,
            glob_include: vec![],
            glob_exclude: vec![],
            examples: vec![],
            needs_more_context_when: String::new(),
        };

        let verdict = client.parse_verdict(&response, &rule).unwrap();

        assert_eq!(verdict.verdict, Verdict::NeedsMoreContext);
        assert!(
            !verdict.reasoning.contains("collapsed"),
            "reasoning should not have collapse annotation for agentic rule"
        );
    }
}
