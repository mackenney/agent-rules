//! OpenRouter API client (OpenAI-compatible chat completions)
//!
//! Implements StatelessEvaluator using the same retry/verdict pattern as AnthropicClient,
//! adapted for the OpenAI-compatible message format and function-call tool schema.

#![allow(dead_code)]
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::evaluator::{StatelessEvalOpts, StatelessEvaluator};
use crate::llm::{LlmError, MAX_RETRIES, RETRY_BASE_DELAY_MS};
use crate::prompt::{SYSTEM_PROMPT, build_tool_schema, build_user_prompt};
use crate::schema::{ContextHint, Rule, RuleContext, RuleVerdict, Verdict};

const API_BASE_URL: &str = "https://openrouter.ai/api/v1";

#[derive(Debug, Serialize)]
struct ChatCompletionRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    messages: Vec<ChatMessage>,
    tools: Vec<serde_json::Value>,
    tool_choice: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: String,
    content: ChatMessageContent,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum ChatMessageContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Debug, Serialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    type_: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
}

#[derive(Debug, Serialize)]
struct CacheControl {
    #[serde(rename = "type")]
    type_: String,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
    #[allow(dead_code)]
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ResponseMessage,
    #[allow(dead_code)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResponseMessage {
    tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Deserialize)]
struct ToolCall {
    function: FunctionCall,
}

#[derive(Debug, Deserialize)]
struct FunctionCall {
    #[allow(dead_code)]
    name: String,
    arguments: FunctionArguments,
}

#[derive(Debug)]
enum FunctionArguments {
    String(String),
    Object(serde_json::Value),
}

impl<'de> Deserialize<'de> for FunctionArguments {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::String(s) => Ok(FunctionArguments::String(s)),
            other @ serde_json::Value::Object(_) => Ok(FunctionArguments::Object(other)),
            _ => Err(serde::de::Error::custom(
                "arguments must be string or object",
            )),
        }
    }
}

impl FunctionArguments {
    fn to_value(&self) -> Result<serde_json::Value, LlmError> {
        match self {
            FunctionArguments::String(s) => serde_json::from_str(s)
                .map_err(|e| LlmError::Parse(format!("failed to parse arguments JSON: {}", e))),
            FunctionArguments::Object(v) => Ok(v.clone()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct Usage {
    #[allow(dead_code)]
    prompt_tokens: Option<u32>,
    #[allow(dead_code)]
    completion_tokens: Option<u32>,
    #[allow(dead_code)]
    prompt_tokens_details: Option<PromptTokensDetails>,
}

#[derive(Debug, Deserialize)]
struct PromptTokensDetails {
    #[allow(dead_code)]
    cached_tokens: Option<u32>,
    #[allow(dead_code)]
    cache_write_tokens: Option<u32>,
}

pub struct OpenRouterClient {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl OpenRouterClient {
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

    #[cfg(test)]
    fn with_base_url(api_key: String, base_url: String) -> Result<Self, LlmError> {
        let mut client = Self::new(api_key)?;
        client.base_url = base_url;
        Ok(client)
    }

    fn transform_tool_schema(anthropic_schema: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": anthropic_schema["name"],
                "description": anthropic_schema["description"],
                "parameters": anthropic_schema["input_schema"],
            }
        })
    }

    fn build_messages(&self, model: &str, user_prompt: &str) -> Vec<ChatMessage> {
        let system_content = if model.starts_with("anthropic/") {
            ChatMessageContent::Blocks(vec![ContentBlock {
                type_: "text".to_string(),
                text: SYSTEM_PROMPT.to_string(),
                cache_control: Some(CacheControl {
                    type_: "ephemeral".to_string(),
                }),
            }])
        } else {
            ChatMessageContent::Text(SYSTEM_PROMPT.to_string())
        };

        vec![
            ChatMessage {
                role: "system".to_string(),
                content: system_content,
            },
            ChatMessage {
                role: "user".to_string(),
                content: ChatMessageContent::Text(user_prompt.to_string()),
            },
        ]
    }

    async fn call_once(
        &self,
        request: &ChatCompletionRequest<'_>,
        timeout: Duration,
    ) -> Result<ChatCompletionResponse, LlmError> {
        let url = format!("{}/chat/completions", self.base_url);

        let response = tokio::time::timeout(
            timeout,
            self.client
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.api_key))
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
                    .json::<ChatCompletionResponse>()
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

    async fn call_with_retry(
        &self,
        request: &ChatCompletionRequest<'_>,
        timeout: Duration,
    ) -> Result<ChatCompletionResponse, LlmError> {
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

    fn parse_verdict(
        &self,
        response: &ChatCompletionResponse,
        rule: &Rule,
    ) -> Result<RuleVerdict, LlmError> {
        let tool_call = response
            .choices
            .first()
            .and_then(|c| c.message.tool_calls.as_ref())
            .and_then(|tc| tc.first());

        let Some(tool_call) = tool_call else {
            return Ok(RuleVerdict {
                rule_id: rule.id.clone(),
                rule_name: rule.name.clone(),
                verdict: Verdict::Fail,
                confidence: 0.0,
                reasoning: "No verdict received from LLM".to_string(),
                severity: rule.severity,
                line_refs: vec![],
                line: None,
                cached: false,
                from_agentic: false,
                context_hint: None,
            });
        };

        let input = tool_call.function.arguments.to_value()?;

        let verdict_str = input
            .get("verdict")
            .and_then(|v| v.as_str())
            .unwrap_or("fail");

        let mut verdict = match verdict_str {
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

        let reasoning =
            if verdict == Verdict::NeedsMoreContext && rule.context == RuleContext::Stateless {
                verdict = Verdict::Fail;
                format!(
                    "{} [collapsed from needs-more-context: stateless rule]",
                    reasoning.trim()
                )
            } else {
                reasoning
            };

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

        let context_hint = input
            .get("context_hint")
            .and_then(|v| v.as_object())
            .map(|obj| ContextHint {
                read_files: obj
                    .get("read_files")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                question: obj
                    .get("question")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            });

        Ok(RuleVerdict {
            rule_id: rule.id.clone(),
            rule_name: rule.name.clone(),
            verdict,
            confidence,
            reasoning,
            severity: rule.severity,
            line_refs: line_refs_u32,
            line,
            cached: false,
            from_agentic: false,
            context_hint,
        })
    }

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

        let messages = self.build_messages(model, &user_prompt);
        let tool = Self::transform_tool_schema(build_tool_schema());

        let request = ChatCompletionRequest {
            model,
            max_tokens: 2048,
            messages,
            tools: vec![tool],
            tool_choice: serde_json::json!({
                "type": "function",
                "function": { "name": "submit_verdict" }
            }),
        };

        let response = match self.call_with_retry(&request, timeout).await {
            Ok(r) => r,
            Err(LlmError::Auth(msg)) => return Err(LlmError::Auth(msg)),
            Err(_) => {
                return Ok(RuleVerdict {
                    rule_id: rule.id.clone(),
                    rule_name: rule.name.clone(),
                    verdict: Verdict::Fail,
                    confidence: 0.0,
                    reasoning: "LLM call failed".to_string(),
                    severity: rule.severity,
                    line_refs: vec![],
                    line: None,
                    cached: false,
                    from_agentic: false,
                    context_hint: None,
                });
            }
        };

        self.parse_verdict(&response, rule)
    }
}

#[async_trait]
impl StatelessEvaluator for OpenRouterClient {
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
}
