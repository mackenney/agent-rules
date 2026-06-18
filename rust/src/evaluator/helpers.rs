use std::time::Duration;

use super::{LlmError, MAX_RETRIES, RETRY_BASE_DELAY_MS};
use crate::schema::{ContextHint, Rule, RuleContext, RuleVerdict, Verdict};

/// Generic retry loop with exponential backoff.
///
/// Calls `call_once` up to `MAX_RETRIES` times. Retryable errors (rate limit,
/// server error, timeout) trigger a delay; non-retryable errors propagate
/// immediately.
pub(super) async fn retry_with_backoff<F, Fut, Resp>(mut call_once: F) -> Result<Resp, LlmError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Resp, LlmError>>,
{
    let mut last_error = LlmError::Exhausted;

    for attempt in 0..MAX_RETRIES {
        match call_once().await {
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

/// Extract a `RuleVerdict` from the provider-neutral tool-call input object.
///
/// Handles verdict parsing (pass/fail/needs-more-context), NMC collapse for
/// stateless rules, confidence clamping, line_refs, and context_hint.
/// Infallible — every field has a fallback default.
pub(super) fn parse_verdict_from_input(input: &serde_json::Value, rule: &Rule) -> RuleVerdict {
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

    RuleVerdict {
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
    }
}

/// Construct a standard fail verdict with zero confidence and no context.
pub(super) fn make_fail_verdict(rule: &Rule, reasoning: &str) -> RuleVerdict {
    RuleVerdict {
        rule_id: rule.id.clone(),
        rule_name: rule.name.clone(),
        verdict: Verdict::Fail,
        confidence: 0.0,
        reasoning: reasoning.to_string(),
        severity: rule.severity,
        line_refs: vec![],
        line: None,
        cached: false,
        from_agentic: false,
        context_hint: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Severity;

    fn test_rule() -> Rule {
        Rule {
            id: "test-rule".to_string(),
            name: "Test Rule".to_string(),
            prompt: "test".to_string(),
            severity: Severity::Error,
            enabled: true,
            context: Default::default(),
            glob_include: vec![],
            glob_exclude: vec![],
            examples: vec![],
            needs_more_context_when: String::new(),
        }
    }

    #[test]
    fn test_parse_verdict_pass() {
        let input = serde_json::json!({
            "verdict": "pass",
            "confidence": 0.9,
            "reasoning": "looks good",
            "line_refs": [10, 20]
        });

        let v = parse_verdict_from_input(&input, &test_rule());
        assert_eq!(v.verdict, Verdict::Pass);
        assert_eq!(v.confidence, 0.9);
        assert_eq!(v.reasoning, "looks good");
        assert_eq!(v.line_refs, vec![10, 20]);
        assert_eq!(v.line, Some(10));
        assert!(!v.from_agentic);
    }

    #[test]
    fn test_parse_verdict_defaults() {
        let input = serde_json::json!({});
        let v = parse_verdict_from_input(&input, &test_rule());
        assert_eq!(v.verdict, Verdict::Fail);
        assert_eq!(v.confidence, 0.5);
        assert_eq!(v.reasoning, "");
        assert!(v.line_refs.is_empty());
    }

    #[test]
    fn test_parse_verdict_nmc_collapse() {
        let mut rule = test_rule();
        rule.context = RuleContext::Stateless;
        let input = serde_json::json!({
            "verdict": "needs-more-context",
            "confidence": 0.5,
            "reasoning": "Need imports",
            "line_refs": []
        });

        let v = parse_verdict_from_input(&input, &rule);
        assert_eq!(v.verdict, Verdict::Fail);
        assert!(
            v.reasoning
                .contains("[collapsed from needs-more-context: stateless rule]")
        );
    }

    #[test]
    fn test_parse_verdict_nmc_preserved_agentic() {
        let mut rule = test_rule();
        rule.context = RuleContext::Agentic;
        let input = serde_json::json!({
            "verdict": "needs-more-context",
            "confidence": 0.5,
            "reasoning": "Need imports",
            "line_refs": []
        });

        let v = parse_verdict_from_input(&input, &rule);
        assert_eq!(v.verdict, Verdict::NeedsMoreContext);
        assert!(!v.reasoning.contains("collapsed"));
    }

    #[test]
    fn test_parse_verdict_confidence_clamped() {
        let input = serde_json::json!({
            "verdict": "pass",
            "confidence": 1.5,
            "reasoning": "ok",
            "line_refs": []
        });
        let v = parse_verdict_from_input(&input, &test_rule());
        assert_eq!(v.confidence, 1.0);
    }

    #[test]
    fn test_parse_verdict_context_hint() {
        let input = serde_json::json!({
            "verdict": "pass",
            "confidence": 0.8,
            "reasoning": "ok",
            "line_refs": [],
            "context_hint": {
                "read_files": ["src/foo.rs"],
                "question": "What does foo export?"
            }
        });
        let v = parse_verdict_from_input(&input, &test_rule());
        let hint = v.context_hint.unwrap();
        assert_eq!(hint.read_files, vec!["src/foo.rs"]);
        assert_eq!(hint.question, "What does foo export?");
    }

    #[test]
    fn test_make_fail_verdict() {
        let rule = test_rule();
        let v = make_fail_verdict(&rule, "something went wrong");
        assert_eq!(v.verdict, Verdict::Fail);
        assert_eq!(v.confidence, 0.0);
        assert_eq!(v.reasoning, "something went wrong");
        assert_eq!(v.rule_id, "test-rule");
        assert!(v.line_refs.is_empty());
        assert!(!v.from_agentic);
        assert!(v.context_hint.is_none());
    }
}
