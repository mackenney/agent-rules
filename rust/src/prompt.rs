//! Prompt building for LLM evaluation calls
//!
//! Builds system and user prompts following the TypeScript implementation.
//! The LLM evaluates code changes against rules and returns verdicts via submit_verdict.

use crate::parser::add_line_numbers;
use crate::schema::{ExampleVerdict, Rule};

/// System prompt (exact TypeScript implementation text)
pub const SYSTEM_PROMPT: &str = "You are a code review agent. Your job is to evaluate a source file against one or more rules\nand call submit_verdict with your evaluation for EACH rule.\n\nVerdict meanings:\n- \"pass\": the code satisfies this rule. No violation found.\n- \"fail\": the code violates this rule.\n- \"needs-more-context\": you cannot determine compliance without reading other files\n  that are not in the diff. Use sparingly \u{2014} only when the answer genuinely depends\n  on external state. Do not use to express uncertainty about borderline cases; use\n  \"fail\" when in doubt.\n  When emitting needs-more-context you MUST populate context_hint.\n\nField guidance:\n- \"confidence\": certainty 0.0\u{2013}1.0. Use < 0.7 when genuinely ambiguous.\n- \"line_refs\": absolute line numbers in the final file (from the numbered FULL FILE\n  CONTENT block). These must match the \" N | \" prefix shown on each line. Empty for pass.\n- \"context_hint\": required only for needs-more-context.\n- If the rule doesn't apply to this file type, return \"pass\" with confidence 1.0.\n- Prefer concrete verdicts over needs-more-context when you have reasonable evidence.\n- Call submit_verdict once for EACH rule listed in the prompt.";

/// Build the formatted section for a single rule (matches TS buildRuleSection / serializeRule)
pub fn build_rule_section(rule: &Rule) -> String {
    let mut s = String::from("\nRULE TO EVALUATE:\n");
    s.push_str(&format!("  name: {}\n", rule.name));
    s.push_str(&format!("  severity: {}\n", rule.severity));
    s.push_str(&format!("  instruction: {}\n", rule.prompt.trim()));

    if !rule.needs_more_context_when.is_empty() {
        s.push_str(&format!(
            "  escalation guidance: {}\n",
            rule.needs_more_context_when.trim()
        ));
    }

    if !rule.examples.is_empty() {
        s.push_str("  examples:\n");
        for example in &rule.examples {
            let tag = if example.verdict == ExampleVerdict::Pass {
                "[PASS]"
            } else {
                "[FAIL]"
            };
            s.push_str(&format!("    {} {}\n", tag, example.description));
            s.push_str(&format!("      {}\n", example.code));
        }
    }

    s
}

/// Build the user prompt for a single-rule file check
pub fn build_user_prompt(
    file_path: &str,
    diff: &str,
    content: Option<&str>,
    rule: &Rule,
    is_new_file: bool,
) -> String {
    let mut prompt = String::new();

    prompt.push_str(&format!("FILE: {}\n\n", file_path));

    if is_new_file {
        prompt.push_str("This is a newly added file.\n\n");
    }

    if !diff.is_empty() {
        prompt.push_str("CHANGED LINES (unified diff with absolute new-file line numbers):\n\n");
        prompt.push_str("```diff\n");
        prompt.push_str(diff);
        if !diff.ends_with('\n') {
            prompt.push('\n');
        }
        prompt.push_str("```\n");
    }

    if let Some(content) = content {
        prompt.push_str(
            "\nFULL FILE CONTENT (each line prefixed \"N | \"; use N verbatim in line_refs):\n\n",
        );
        prompt.push_str("```\n");
        prompt.push_str(&add_line_numbers(content));
        prompt.push_str("\n```");
    }

    prompt.push_str(&build_rule_section(rule));

    prompt
}

/// Build the tool schema for submit_verdict
/// Build the tool schema for submit_verdict (no rule_id — one call per rule)
pub fn build_tool_schema() -> serde_json::Value {
    serde_json::json!({
        "name": "submit_verdict",
        "description": "Submit the rule evaluation verdict as structured data.",
        "input_schema": {
            "type": "object",
            "properties": {
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
                    "description": "1-3 sentences explaining the verdict, referencing specific code."
                },
                "line_refs": {
                    "type": "array",
                    "items": {"type": "integer"},
                    "description": "Absolute line numbers of violations. Empty array for pass."
                },
                "context_hint": {
                    "type": "object",
                    "description": "Required only when verdict is needs-more-context.",
                    "properties": {
                        "read_files": { "type": "array", "items": { "type": "string" } },
                        "question": { "type": "string" }
                    },
                    "required": ["read_files", "question"]
                }
            },
            "required": ["verdict", "confidence", "reasoning", "line_refs"]
        }
    })
}

/// Truncate content to max chars, preserving line boundaries
#[allow(dead_code)]
pub fn truncate_to_chars(content: &str, max_chars: usize) -> String {
    if content.chars().count() <= max_chars {
        return content.to_string();
    }

    let truncated: String = content.chars().take(max_chars).collect();
    if let Some(last_newline) = truncated.rfind('\n') {
        format!("{}\n... (truncated)", &truncated[..last_newline])
    } else {
        format!("{}... (truncated)", truncated)
    }
}

/// Build the task prompt for an agentic evaluation session
///
/// The pi subprocess reads this as the task to perform, including
/// context hints from the stateless pass and instructions to emit
/// a final JSON verdict.
pub fn build_agentic_task(
    file_path: &str,
    diff: &str,
    content: Option<&str>,
    rule: &Rule,
    hints: &[crate::schema::ContextHint],
) -> String {
    let mut parts: Vec<String> = Vec::new();

    parts.push(format!("FILE: {}", file_path));

    if !diff.is_empty() {
        parts.push(
            "\nCHANGED LINES (unified diff with absolute new-file line numbers):".to_string(),
        );
        parts.push(format!("```diff\n{}\n```", diff));
    }

    if let Some(c) = content {
        parts.push(
            "\nFULL FILE CONTENT (each line prefixed \"N | \"; use N verbatim in line_refs):\n"
                .to_string(),
        );
        parts.push(format!("```\n{}\n```", add_line_numbers(c)));
    }

    parts.push("\nRULE TO EVALUATE:".to_string());
    parts.push(build_rule_section(rule));

    if !hints.is_empty() {
        let mut hint_lines: Vec<String> = Vec::new();
        for h in hints {
            if !h.read_files.is_empty() {
                hint_lines.push(format!(
                    "Suggested files to read: {}",
                    h.read_files.join(", ")
                ));
            }
            if !h.question.is_empty() {
                hint_lines.push(format!("Question to answer: {}", h.question));
            }
        }
        if !hint_lines.is_empty() {
            parts.push(format!(
                "\nContext hints from stateless pass:\n{}",
                hint_lines.join("\n")
            ));
        }
    }

    parts.push(
        "\nIMPORTANT: Use your file-reading tools to gather whatever context you need, then\n"
            .to_string(),
    );
    parts.push(
        "emit your verdict. Your FINAL message must be EXACTLY the following JSON object\n"
            .to_string(),
    );
    parts.push(
        "and nothing else \u{2014} no preamble, no explanation, no markdown fences:\n\n"
            .to_string(),
    );
    parts.push(
        r#"{"reasoning":"<1-3 sentences>","line_refs":[],"confidence":0.0,"verdict":"pass|fail"}"#
            .to_string(),
    );
    parts.push(
        "\n\nDo NOT emit needs-more-context. You must reach a terminal verdict (pass/fail)."
            .to_string(),
    );

    parts.join("\n")
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
        let rule = make_test_rule();
        let prompt = build_user_prompt(
            "src/main.rs",
            "+new line",
            Some("fn main() {}"),
            &rule,
            false,
        );

        assert!(prompt.contains("FILE: src/main.rs"));
        assert!(prompt.contains("CHANGED LINES"));
        assert!(prompt.contains("+new line"));
        assert!(prompt.contains("FULL FILE CONTENT"));
        assert!(prompt.contains("RULE TO EVALUATE"));
        assert!(
            prompt.contains("Test Rule"),
            "rule name should be in prompt"
        );
        assert!(
            prompt.contains("Check for test issues"),
            "rule instruction should be in prompt"
        );
    }

    #[test]
    fn test_build_user_prompt_new_file() {
        let rule = make_test_rule();
        let prompt = build_user_prompt("new.rs", "+content", None, &rule, true);
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
        let prompt = SYSTEM_PROMPT;
        assert!(prompt.contains("submit_verdict"));
        assert!(prompt.contains("needs-more-context"));
    }

    #[test]
    fn test_build_rule_section() {
        let rule = make_test_rule();
        let section = build_rule_section(&rule);
        assert!(section.contains("RULE TO EVALUATE"));
        assert!(section.contains("Test Rule"));
        assert!(section.contains("Check for test issues"));
        assert!(
            !section.contains("test-rule"),
            "rule id should not appear in section"
        );
    }
    #[test]
    fn test_truncate_multibyte_chars() {
        // "日本語" is 3 characters but 9 bytes
        let content = "日本語";

        // Should NOT truncate at max_chars=3 (it's exactly 3 chars)
        let result = truncate_to_chars(content, 3);
        assert_eq!(
            result, content,
            "3-char content at max_chars=3 should not truncate"
        );

        // Should truncate at max_chars=2
        let result = truncate_to_chars(content, 2);
        assert!(
            result.contains("truncated"),
            "should truncate when chars > max"
        );
        assert_eq!(result.chars().take(2).collect::<String>(), "日本");
    }
    #[test]
    fn test_build_agentic_task_no_diff() {
        let rule = make_test_rule();
        let task = build_agentic_task("foo.rs", "", None, &rule, &[]);
        assert!(
            !task.contains("CHANGED LINES"),
            "empty diff should not produce CHANGED LINES section"
        );
    }

    #[test]
    fn test_build_agentic_task_basic() {
        let rule = make_test_rule();
        let task = build_agentic_task("src/main.rs", "+new line", Some("fn main() {}"), &rule, &[]);
        assert!(task.contains("FILE: src/main.rs"));
        assert!(task.contains("CHANGED LINES"));
        assert!(task.contains("+new line"));
        assert!(task.contains("FULL FILE CONTENT"));
        assert!(task.contains("RULE TO EVALUATE"));
        assert!(task.contains("Test Rule"));
        assert!(task.contains("needs-more-context"));
        assert!(task.contains("pass|fail"));
    }

    #[test]
    fn test_build_agentic_task_with_hints() {
        use crate::schema::ContextHint;
        let rule = make_test_rule();
        let hints = vec![ContextHint {
            read_files: vec!["src/utils.rs".to_string()],
            question: "What does get_config return?".to_string(),
        }];
        let task = build_agentic_task("src/main.rs", "", Some("code"), &rule, &hints);
        assert!(task.contains("Context hints from stateless pass"));
        assert!(task.contains("What does get_config return?"));
        assert!(task.contains("src/utils.rs"));
    }
}
