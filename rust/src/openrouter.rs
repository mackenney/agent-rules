//! OpenRouter API client (OpenAI-compatible chat completions)
//!
//! Implements StatelessEvaluator using the same retry/verdict pattern as AnthropicClient,
//! adapted for the OpenAI-compatible message format and function-call tool schema.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::time::Duration;

use crate::evaluator::{StatelessEvalOpts, StatelessEvaluator};
use crate::llm::{LlmError, MAX_RETRIES, RETRY_BASE_DELAY_MS};
use crate::prompt::{SYSTEM_PROMPT, build_tool_schema, build_user_prompt};
use crate::schema::{ContextHint, Rule, RuleContext, RuleVerdict, Verdict};

const API_BASE_URL: &str = "https://openrouter.ai/api/v1";
static OPENAI_TOOL_SCHEMA: OnceLock<serde_json::Value> = OnceLock::new();

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

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct Usage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
    prompt_tokens_details: Option<PromptTokensDetails>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct PromptTokensDetails {
    cached_tokens: Option<u32>,
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

    fn openai_tool_schema() -> serde_json::Value {
        OPENAI_TOOL_SCHEMA
            .get_or_init(|| Self::transform_tool_schema(build_tool_schema()))
            .clone()
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

        if tool_call.function.name != "submit_verdict" {
            return Ok(RuleVerdict {
                rule_id: rule.id.clone(),
                rule_name: rule.name.clone(),
                verdict: Verdict::Fail,
                confidence: 0.0,
                reasoning: format!(
                    "Unexpected tool call '{}'; expected 'submit_verdict'",
                    tool_call.function.name
                ),
                severity: rule.severity,
                line_refs: vec![],
                line: None,
                cached: false,
                from_agentic: false,
                context_hint: None,
            });
        }
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
            .unwrap_or(0.5)
            .clamp(0.0, 1.0);

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
        let line_refs_u32: Vec<u32> = line_refs
            .iter()
            .filter_map(|&l| u32::try_from(l).ok())
            .collect();
        let line = line_refs_u32.first().copied();

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
        let tool = Self::openai_tool_schema();

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{RuleContext, Severity};

    fn make_test_rule() -> Rule {
        Rule {
            id: "test-rule".to_string(),
            name: "Test Rule".to_string(),
            prompt: "test instruction".to_string(),
            severity: Severity::Error,
            enabled: true,
            context: Default::default(),
            glob_include: vec![],
            glob_exclude: vec![],
            examples: vec![],
            needs_more_context_when: String::new(),
        }
    }

    fn make_response_with_arguments(arguments: FunctionArguments) -> ChatCompletionResponse {
        ChatCompletionResponse {
            choices: vec![Choice {
                message: ResponseMessage {
                    tool_calls: Some(vec![ToolCall {
                        function: FunctionCall {
                            name: "submit_verdict".to_string(),
                            arguments,
                        },
                    }]),
                },
                finish_reason: Some("stop".to_string()),
            }],
            usage: None,
        }
    }

    #[test]
    fn test_parse_verdict_string_arguments() {
        let client = OpenRouterClient::new("test-key".to_string()).unwrap();
        let rule = make_test_rule();

        let args_json =
            r#"{"verdict":"pass","confidence":0.9,"reasoning":"looks good","line_refs":[]}"#;
        let response =
            make_response_with_arguments(FunctionArguments::String(args_json.to_string()));

        let verdict = client.parse_verdict(&response, &rule).unwrap();
        assert_eq!(verdict.verdict, Verdict::Pass);
        assert_eq!(verdict.confidence, 0.9);
        assert_eq!(verdict.rule_id, "test-rule");
        assert!(verdict.line_refs.is_empty());
        assert_eq!(verdict.line, None);
    }

    #[test]
    fn test_parse_verdict_object_arguments() {
        let client = OpenRouterClient::new("test-key".to_string()).unwrap();
        let rule = make_test_rule();

        let args_value = serde_json::json!({
            "verdict": "pass",
            "confidence": 0.85,
            "reasoning": "code is clean",
            "line_refs": []
        });
        let response = make_response_with_arguments(FunctionArguments::Object(args_value));

        let verdict = client.parse_verdict(&response, &rule).unwrap();
        assert_eq!(verdict.verdict, Verdict::Pass);
        assert_eq!(verdict.confidence, 0.85);
    }

    #[test]
    fn test_parse_verdict_malformed_arguments() {
        let client = OpenRouterClient::new("test-key".to_string()).unwrap();
        let rule = make_test_rule();

        let response =
            make_response_with_arguments(FunctionArguments::String("not valid json {".to_string()));

        let result = client.parse_verdict(&response, &rule);
        assert!(result.is_err());
        match result.unwrap_err() {
            LlmError::Parse(_) => {}
            other => panic!("expected LlmError::Parse, got: {:?}", other),
        }
    }

    #[test]
    fn test_parse_verdict_empty_arguments() {
        let client = OpenRouterClient::new("test-key".to_string()).unwrap();
        let rule = make_test_rule();

        let response = make_response_with_arguments(FunctionArguments::String(String::new()));

        let result = client.parse_verdict(&response, &rule);
        assert!(result.is_err());
        match result.unwrap_err() {
            LlmError::Parse(_) => {}
            other => panic!("expected LlmError::Parse, got: {:?}", other),
        }
    }

    #[test]
    fn test_parse_verdict_missing_tool_calls() {
        let client = OpenRouterClient::new("test-key".to_string()).unwrap();
        let rule = make_test_rule();

        let response = ChatCompletionResponse {
            choices: vec![Choice {
                message: ResponseMessage { tool_calls: None },
                finish_reason: Some("stop".to_string()),
            }],
            usage: None,
        };

        let verdict = client.parse_verdict(&response, &rule).unwrap();
        assert_eq!(verdict.verdict, Verdict::Fail);
        assert!(verdict.reasoning.contains("No verdict"));
        assert_eq!(verdict.rule_id, "test-rule");
    }

    #[test]
    fn test_parse_verdict_empty_choices() {
        let client = OpenRouterClient::new("test-key".to_string()).unwrap();
        let rule = make_test_rule();

        let response = ChatCompletionResponse {
            choices: vec![],
            usage: None,
        };

        let verdict = client.parse_verdict(&response, &rule).unwrap();
        assert_eq!(verdict.verdict, Verdict::Fail);
        assert!(verdict.reasoning.contains("No verdict"));
    }

    #[test]
    fn test_parse_verdict_fail_with_line_refs() {
        let client = OpenRouterClient::new("test-key".to_string()).unwrap();
        let rule = make_test_rule();

        let args_json = r#"{"verdict":"fail","confidence":0.95,"reasoning":"violation found","line_refs":[10,20,30]}"#;
        let response =
            make_response_with_arguments(FunctionArguments::String(args_json.to_string()));

        let verdict = client.parse_verdict(&response, &rule).unwrap();
        assert_eq!(verdict.verdict, Verdict::Fail);
        assert_eq!(verdict.confidence, 0.95);
        assert_eq!(verdict.line_refs, vec![10, 20, 30]);
        assert_eq!(verdict.line, Some(10));
    }

    #[test]
    fn test_nmc_collapse_stateless_rule() {
        let client = OpenRouterClient::new("test-key".to_string()).unwrap();
        let mut rule = make_test_rule();
        rule.context = RuleContext::Stateless;

        let args_json = r#"{"verdict":"needs-more-context","confidence":0.5,"reasoning":"Need to check imports","line_refs":[],"context_hint":{"read_files":["src/utils.rs"],"question":"What does utils export?"}}"#;
        let response =
            make_response_with_arguments(FunctionArguments::String(args_json.to_string()));

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
        let hint = verdict.context_hint.as_ref().unwrap();
        assert_eq!(hint.read_files, vec!["src/utils.rs"]);
        assert_eq!(hint.question, "What does utils export?");
    }

    #[test]
    fn test_nmc_not_collapsed_agentic_rule() {
        let client = OpenRouterClient::new("test-key".to_string()).unwrap();
        let mut rule = make_test_rule();
        rule.context = RuleContext::Agentic;

        let args_json = r#"{"verdict":"needs-more-context","confidence":0.5,"reasoning":"Need to check imports","line_refs":[]}"#;
        let response =
            make_response_with_arguments(FunctionArguments::String(args_json.to_string()));

        let verdict = client.parse_verdict(&response, &rule).unwrap();
        assert_eq!(verdict.verdict, Verdict::NeedsMoreContext);
        assert!(
            !verdict.reasoning.contains("collapsed"),
            "reasoning should not have collapse annotation for agentic rule"
        );
    }

    #[test]
    fn test_request_system_prompt_in_messages() {
        let client = OpenRouterClient::new("test-key".to_string()).unwrap();
        let messages = client.build_messages("openai/gpt-4o", "test user prompt");

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[1].role, "user");

        match &messages[1].content {
            ChatMessageContent::Text(t) => assert_eq!(t, "test user prompt"),
            _ => panic!("expected Text content for user message"),
        }

        let request = ChatCompletionRequest {
            model: "openai/gpt-4o",
            max_tokens: 2048,
            messages,
            tools: vec![],
            tool_choice: serde_json::json!({"type": "function", "function": {"name": "submit_verdict"}}),
        };

        let serialized = serde_json::to_value(&request).unwrap();
        assert!(
            serialized.get("system").is_none(),
            "no top-level 'system' field"
        );
        assert_eq!(serialized["messages"][0]["role"], "system");
        assert_eq!(serialized["messages"][1]["role"], "user");
    }

    #[test]
    fn test_request_tool_choice_function_format() {
        let tool_choice = serde_json::json!({
            "type": "function",
            "function": { "name": "submit_verdict" }
        });

        assert_eq!(tool_choice["type"], "function");
        assert_eq!(tool_choice["function"]["name"], "submit_verdict");
        assert!(
            tool_choice.get("type").unwrap().as_str().unwrap() != "tool",
            "OpenRouter uses 'function' not 'tool'"
        );
    }

    #[test]
    fn test_transform_tool_schema() {
        let anthropic_schema = build_tool_schema();
        let openai_schema = OpenRouterClient::transform_tool_schema(anthropic_schema);

        assert_eq!(openai_schema["type"], "function");
        assert_eq!(openai_schema["function"]["name"], "submit_verdict");
        assert!(openai_schema["function"]["description"].is_string());
        assert!(openai_schema["function"]["parameters"].is_object());

        assert!(
            openai_schema.get("input_schema").is_none(),
            "OpenAI format uses 'parameters', not 'input_schema'"
        );
        assert!(openai_schema["function"].get("input_schema").is_none());
        assert!(openai_schema["function"]["parameters"]["properties"]["verdict"].is_object());
    }

    #[test]
    fn test_cache_control_anthropic_model() {
        let client = OpenRouterClient::new("test-key".to_string()).unwrap();
        let messages = client.build_messages("anthropic/claude-haiku-4-5", "test");

        match &messages[0].content {
            ChatMessageContent::Blocks(blocks) => {
                assert_eq!(blocks.len(), 1);
                assert_eq!(blocks[0].type_, "text");
                assert!(blocks[0].cache_control.is_some());
                assert_eq!(blocks[0].cache_control.as_ref().unwrap().type_, "ephemeral");
            }
            _ => panic!("expected Blocks content for anthropic model system message"),
        }

        let serialized = serde_json::to_value(&messages[0]).unwrap();
        let content = &serialized["content"];
        assert!(
            content.is_array(),
            "anthropic model system content should be array"
        );
        assert_eq!(content[0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn test_cache_control_non_anthropic_model() {
        let client = OpenRouterClient::new("test-key".to_string()).unwrap();
        let messages = client.build_messages("deepseek/deepseek-chat-v3", "test");

        match &messages[0].content {
            ChatMessageContent::Text(t) => {
                assert!(!t.is_empty());
            }
            _ => panic!("expected Text content for non-anthropic model system message"),
        }

        let serialized = serde_json::to_value(&messages[0]).unwrap();
        let serialized_str = serde_json::to_string(&serialized).unwrap();
        assert!(
            !serialized_str.contains("cache_control"),
            "non-anthropic model should have no cache_control"
        );
    }

    #[test]
    fn test_cache_control_openai_model() {
        let client = OpenRouterClient::new("test-key".to_string()).unwrap();
        let messages = client.build_messages("openai/gpt-4o", "test");

        match &messages[0].content {
            ChatMessageContent::Text(_) => {}
            _ => panic!("expected Text content for openai model"),
        }

        let serialized_str = serde_json::to_string(&messages[0]).unwrap();
        assert!(!serialized_str.contains("cache_control"));
    }

    #[test]
    fn test_function_arguments_deserialize_string() {
        let json = r#"{"name":"submit_verdict","arguments":"{\"verdict\":\"pass\"}"}"#;
        let fc: FunctionCall = serde_json::from_str(json).unwrap();
        assert!(matches!(fc.arguments, FunctionArguments::String(_)));

        let value = fc.arguments.to_value().unwrap();
        assert_eq!(value["verdict"], "pass");
    }

    #[test]
    fn test_function_arguments_deserialize_object() {
        let json = r#"{"name":"submit_verdict","arguments":{"verdict":"pass"}}"#;
        let fc: FunctionCall = serde_json::from_str(json).unwrap();
        assert!(matches!(fc.arguments, FunctionArguments::Object(_)));

        let value = fc.arguments.to_value().unwrap();
        assert_eq!(value["verdict"], "pass");
    }

    #[test]
    fn test_function_arguments_deserialize_invalid_type() {
        let json_array = r#"{"name":"submit_verdict","arguments":[1,2,3]}"#;
        let result: Result<FunctionCall, _> = serde_json::from_str(json_array);
        assert!(result.is_err(), "array arguments must be rejected");

        let json_null = r#"{"name":"submit_verdict","arguments":null}"#;
        let result: Result<FunctionCall, _> = serde_json::from_str(json_null);
        assert!(result.is_err(), "null arguments must be rejected");

        let json_number = r#"{"name":"submit_verdict","arguments":42}"#;
        let result: Result<FunctionCall, _> = serde_json::from_str(json_number);
        assert!(result.is_err(), "numeric arguments must be rejected");
    }
    #[test]
    fn test_parse_verdict_wrong_function_name() {
        let client = OpenRouterClient::new("test-key".to_string()).unwrap();
        let rule = make_test_rule();

        let response = ChatCompletionResponse {
            choices: vec![Choice {
                message: ResponseMessage {
                    tool_calls: Some(vec![ToolCall {
                        function: FunctionCall {
                            name: "some_other_function".to_string(),
                            arguments: FunctionArguments::String("{}".to_string()),
                        },
                    }]),
                },
                finish_reason: Some("stop".to_string()),
            }],
            usage: None,
        };

        let verdict = client.parse_verdict(&response, &rule).unwrap();
        assert_eq!(verdict.verdict, Verdict::Fail);
        assert!(
            verdict.reasoning.contains("Unexpected tool call"),
            "reasoning should identify unexpected name: {}",
            verdict.reasoning
        );
        assert!(
            verdict.reasoning.contains("some_other_function"),
            "reasoning should include the actual function name: {}",
            verdict.reasoning
        );
    }

    #[test]
    fn test_parse_verdict_empty_tool_calls_array() {
        let client = OpenRouterClient::new("test-key".to_string()).unwrap();
        let rule = make_test_rule();

        let response = ChatCompletionResponse {
            choices: vec![Choice {
                message: ResponseMessage {
                    tool_calls: Some(vec![]),
                },
                finish_reason: Some("stop".to_string()),
            }],
            usage: None,
        };

        let verdict = client.parse_verdict(&response, &rule).unwrap();
        assert_eq!(verdict.verdict, Verdict::Fail);
        assert!(
            verdict.reasoning.contains("No verdict"),
            "empty tool_calls array should fall back to 'No verdict': {}",
            verdict.reasoning
        );
    }

    #[test]
    fn test_parse_verdict_confidence_clamped() {
        let client = OpenRouterClient::new("test-key".to_string()).unwrap();
        let rule = make_test_rule();

        let r = make_response_with_arguments(FunctionArguments::String(
            r#"{"verdict":"pass","confidence":1.5,"reasoning":"ok","line_refs":[]}"#.to_string(),
        ));
        let v = client.parse_verdict(&r, &rule).unwrap();
        assert_eq!(v.confidence, 1.0, "confidence > 1.0 must be clamped to 1.0");

        let r2 = make_response_with_arguments(FunctionArguments::String(
            r#"{"verdict":"fail","confidence":-0.5,"reasoning":"bad","line_refs":[]}"#.to_string(),
        ));
        let v2 = client.parse_verdict(&r2, &rule).unwrap();
        assert_eq!(
            v2.confidence, 0.0,
            "confidence < 0.0 must be clamped to 0.0"
        );
    }
}
