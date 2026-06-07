# Step 05: Unit Tests

## Context

### Overall Objective

Add OpenRouter as a second LLM provider. This step adds comprehensive inline unit tests to `openrouter.rs` covering response parsing, request serialization, conditional cache_control, NMC collapse, and edge cases.

### Phase Context

Wave 4 — depends on step-02 (`openrouter.rs` exists with all types and methods). These tests validate the OpenRouter-specific behavioral differences from AnthropicClient.

### This Step

Add a `#[cfg(test)] mod tests` block to `openrouter.rs` with tests for:
1. Verdict parsing from JSON string arguments (the #1 behavioral difference)
2. Verdict parsing from pre-parsed object arguments (defensive handling)
3. Malformed and empty arguments error handling
4. Missing tool_calls fallback
5. NMC collapse for stateless rules (behavioral parity)
6. NMC preservation for agentic rules (behavioral parity)
7. Request serialization: system prompt in messages (not top-level)
8. Request serialization: tool_choice format (`type: "function"`, not `type: "tool"`)
9. Tool schema transformation (Anthropic → OpenAI format)
10. Conditional cache_control for `anthropic/` models vs non-anthropic models
11. Fail verdict with line_refs

## Prerequisites

- step-02 complete (`openrouter.rs` has all types, methods, and structs)

## Files to Read Before Starting

- `rust/src/openrouter.rs` — full file; understand all types and method signatures
- `rust/src/llm.rs` — lines 413-590 for existing Anthropic unit tests (use as pattern reference)
- `rust/src/schema.rs` — `Rule`, `RuleContext`, `Severity`, `Verdict` for constructing test inputs

## Implementation

### Task 1: Add test module and helper

At the bottom of `rust/src/openrouter.rs`, add:

```rust
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
```

### Task 2: Add verdict parsing tests

```rust
    #[test]
    fn test_parse_verdict_string_arguments() {
        let client = OpenRouterClient::new("test-key".to_string()).unwrap();
        let rule = make_test_rule();

        let args_json = r#"{"verdict":"pass","confidence":0.9,"reasoning":"looks good","line_refs":[]}"#;
        let response = make_response_with_arguments(FunctionArguments::String(args_json.to_string()));

        let verdict = client.parse_verdict(&response, &rule).unwrap();
        assert_eq!(verdict.verdict, Verdict::Pass);
        assert_eq!(verdict.confidence, 0.9);
        assert_eq!(verdict.rule_id, "test-rule");
        assert!(verdict.line_refs.is_empty());
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

        let response = make_response_with_arguments(
            FunctionArguments::String("not valid json {".to_string()),
        );

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

        let response = make_response_with_arguments(
            FunctionArguments::String(String::new()),
        );

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
        let response = make_response_with_arguments(FunctionArguments::String(args_json.to_string()));

        let verdict = client.parse_verdict(&response, &rule).unwrap();
        assert_eq!(verdict.verdict, Verdict::Fail);
        assert_eq!(verdict.confidence, 0.95);
        assert_eq!(verdict.line_refs, vec![10, 20, 30]);
        assert_eq!(verdict.line, Some(10));
    }
```

### Task 3: Add NMC collapse tests

```rust
    #[test]
    fn test_nmc_collapse_stateless_rule() {
        let client = OpenRouterClient::new("test-key".to_string()).unwrap();
        let mut rule = make_test_rule();
        rule.context = RuleContext::Stateless;

        let args_json = r#"{"verdict":"needs-more-context","confidence":0.5,"reasoning":"Need to check imports","line_refs":[],"context_hint":{"read_files":["src/utils.rs"],"question":"What does utils export?"}}"#;
        let response = make_response_with_arguments(FunctionArguments::String(args_json.to_string()));

        let verdict = client.parse_verdict(&response, &rule).unwrap();
        assert_eq!(verdict.verdict, Verdict::Fail);
        assert!(
            verdict.reasoning.contains("[collapsed from needs-more-context: stateless rule]"),
            "reasoning should contain collapse annotation: {}",
            verdict.reasoning
        );
        assert!(verdict.context_hint.is_some());
    }

    #[test]
    fn test_nmc_not_collapsed_agentic_rule() {
        let client = OpenRouterClient::new("test-key".to_string()).unwrap();
        let mut rule = make_test_rule();
        rule.context = RuleContext::Agentic;

        let args_json = r#"{"verdict":"needs-more-context","confidence":0.5,"reasoning":"Need to check imports","line_refs":[]}"#;
        let response = make_response_with_arguments(FunctionArguments::String(args_json.to_string()));

        let verdict = client.parse_verdict(&response, &rule).unwrap();
        assert_eq!(verdict.verdict, Verdict::NeedsMoreContext);
        assert!(
            !verdict.reasoning.contains("collapsed"),
            "reasoning should not have collapse annotation for agentic rule"
        );
    }
```

### Task 4: Add request serialization tests

```rust
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
        assert!(serialized.get("system").is_none(), "no top-level 'system' field");
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
        assert!(tool_choice.get("type").unwrap().as_str().unwrap() != "tool",
            "OpenRouter uses 'function' not 'tool'");
    }
```

### Task 5: Add tool schema transformation test

```rust
    #[test]
    fn test_transform_tool_schema() {
        let anthropic_schema = build_tool_schema();
        let openai_schema = OpenRouterClient::transform_tool_schema(anthropic_schema);

        assert_eq!(openai_schema["type"], "function");
        assert_eq!(openai_schema["function"]["name"], "submit_verdict");
        assert!(openai_schema["function"]["description"].is_string());
        assert!(openai_schema["function"]["parameters"].is_object());

        assert!(openai_schema.get("input_schema").is_none(),
            "OpenAI format uses 'parameters', not 'input_schema'");
        assert!(openai_schema["function"].get("input_schema").is_none());
        assert!(openai_schema["function"]["parameters"]["properties"]["verdict"].is_object());
    }
```

### Task 6: Add conditional cache_control tests

```rust
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
        assert!(content.is_array(), "anthropic model system content should be array");
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
        assert!(!serialized_str.contains("cache_control"),
            "non-anthropic model should have no cache_control");
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
```

### Task 7: Add FunctionArguments deserialization test

```rust
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
```

Close the test module:

```rust
}
```

## Acceptance Criteria

- [ ] `cd rust && cargo build 2>&1` exits 0
- [ ] `cd rust && cargo clippy -- -D warnings 2>&1` exits 0
- [ ] `cd rust && cargo nextest run 2>&1` exits 0 (all existing + new tests pass)
- [ ] `cd rust && cargo nextest run -E 'test(openrouter)' 2>&1` shows at least 14 tests passing
- [ ] `cd rust && cargo nextest run -E 'test(test_parse_verdict_string_arguments)' 2>&1` exits 0
- [ ] `cd rust && cargo nextest run -E 'test(test_parse_verdict_malformed_arguments)' 2>&1` exits 0
- [ ] `cd rust && cargo nextest run -E 'test(test_nmc_collapse_stateless_rule)' -- --test-threads=1 2>&1 | grep -q 'openrouter'` — verifies test is in openrouter module
- [ ] `cd rust && cargo nextest run -E 'test(test_cache_control_anthropic_model)' 2>&1` exits 0
- [ ] `cd rust && cargo nextest run -E 'test(test_transform_tool_schema)' 2>&1` exits 0
- [ ] `cd rust && cargo nextest run -E 'test(test_function_arguments_deserialize)' 2>&1` exits 0

## Reviewer Instructions

1. Run all acceptance criteria commands
2. Verify test count: `cargo nextest run -E 'test(openrouter)' 2>&1 | grep -c 'PASS'` should be ≥ 14
3. Verify each test is self-contained (no external dependencies, no network calls)
4. Verify NMC collapse tests mirror the exact behavior tested in `llm.rs` tests
5. Verify `FunctionArguments` deserialization handles both string and object forms
6. Verify `build_messages` and `transform_tool_schema` are tested via public-ish methods (not just private internals — they are private methods called within `#[cfg(test)]` which has access)

## Rollback

```bash
cd rust
git checkout -- src/openrouter.rs
```
