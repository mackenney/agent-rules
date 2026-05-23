//! Prompt building for LLM evaluation calls
//!
//! Builds system and user prompts following the TypeScript implementation.
//! The LLM evaluates code changes against rules and returns verdicts via submit_verdict.

use crate::parser::add_line_numbers;
use crate::schema::Rule;

/// System prompt (exact TypeScript implementation text)
pub const SYSTEM_PROMPT: &str = "You are a code review agent. Your job is to evaluate a source file against a single rule\nand call submit_verdict with your evaluation.\n\nVerdict meanings:\n- \"pass\": the code satisfies this rule. No violation found.\n- \"fail\": the code violates this rule.\n- \"needs-more-context\": you cannot determine compliance without reading other files\n  that are not in the diff. Use sparingly \u{2014} only when the answer genuinely depends\n  on external state. Do not use to express uncertainty about borderline cases; use\n  \"fail\" when in doubt.\n  When emitting needs-more-context you MUST populate context_hint.\n\nField guidance:\n- \"confidence\": certainty 0.0\u{2013}1.0. Use < 0.7 when genuinely ambiguous.\n- \"line_refs\": absolute line numbers in the final file (from the numbered FULL FILE\n  CONTENT block). These must match the \" N | \" prefix shown on each line. Empty for pass.\n- \"context_hint\": required only for needs-more-context.\n- If the rule doesn't apply to this file type, return \"pass\" with confidence 1.0.\n- Prefer concrete verdicts over needs-more-context when you have reasonable evidence.";

/// Returns the system prompt string
pub fn build_system_prompt() -> &'static str {
    SYSTEM_PROMPT
}

/// Build the formatted section for a single rule
pub fn build_rule_section(rule: &Rule) -> String {
    let mut s = format!("**{}** (`{}`)\n", rule.name, rule.id);
    s.push_str(&format!("Severity: {}\n", rule.severity));
    s.push_str(&format!("{}\n", rule.prompt));

    if !rule.needs_more_context_when.is_empty() {
        s.push_str(&format!(
            "Use needs-more-context when: {}\n",
            rule.needs_more_context_when
        ));
    }

    if !rule.examples.is_empty() {
        s.push_str("Examples:\n");
        for example in &rule.examples {
            let pass_fail = if example.should_pass {
                "✓ pass"
            } else {
                "✗ fail"
            };
            s.push_str(&format!("- {} `{}`", pass_fail, example.code));
            if !example.explanation.is_empty() {
                s.push_str(&format!(" — {}", example.explanation));
            }
            s.push('\n');
        }
    }

    s
}

/// Build the user prompt for a file check
pub fn build_user_prompt(
    file_path: &str,
    diff: &str,
    content: Option<&str>,
    rules: &[Rule],
    is_new_file: bool,
) -> String {
    let mut prompt = String::new();

    prompt.push_str(&format!("## File: {}\n\n", file_path));

    if is_new_file {
        prompt.push_str("This is a newly added file.\n\n");
    }

    prompt.push_str("### Changes (diff)\n\n");
    prompt.push_str("```diff\n");
    prompt.push_str(diff);
    if !diff.ends_with('\n') {
        prompt.push('\n');
    }
    prompt.push_str("```\n\n");

    if let Some(content) = content {
        prompt.push_str("### Full file content (with line numbers)\n\n");
        prompt.push_str("```\n");
        prompt.push_str(&add_line_numbers(content));
        prompt.push_str("\n```\n\n");
    }

    prompt.push_str("### Rules to evaluate\n\n");
    for (i, rule) in rules.iter().enumerate() {
        prompt.push_str(&format!("{}. ", i + 1));
        prompt.push_str(&build_rule_section(rule));
        prompt.push('\n');
    }

    prompt
        .push_str("\nEvaluate each rule and submit your verdict using the submit_verdict tool.\n");

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
                "line_refs": {
                    "type": "array",
                    "items": {"type": "integer"},
                    "description": "Absolute line numbers in the final file where the violation occurs (empty for pass)"
                },
                "context_hint": {
                    "type": "string",
                    "description": "Required when verdict is needs-more-context"
                }
            },
            "required": ["rule_id", "verdict", "confidence", "reasoning", "line_refs"]
        }
    })
}

/// Truncate content to max chars, preserving line boundaries
pub fn truncate_to_chars(content: &str, max_chars: usize) -> String {
    if content.len() <= max_chars {
        return content.to_string();
    }

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

    #[test]
    fn test_build_system_prompt() {
        let prompt = build_system_prompt();
        assert!(prompt.contains("submit_verdict"));
        assert!(prompt.contains("needs-more-context"));
    }

    #[test]
    fn test_build_rule_section() {
        let rule = make_test_rule();
        let section = build_rule_section(&rule);
        assert!(section.contains("test-rule"));
        assert!(section.contains("Test Rule"));
    }
}
