//! TOML parsing for rule files and diff annotation
//!
//! Handles .agent-rules.toml parsing and unified diff annotation with line numbers.

use anyhow::{Context, Result};
use std::path::Path;

use crate::schema::{Rule, RuleFile};

/// Default rule file name
pub const RULE_FILE_NAME: &str = ".agent-rules.toml";

/// Parse a rule file from disk
pub fn parse_rule_file(path: &Path) -> Result<RuleFile> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read rule file: {}", path.display()))?;
    parse_rule_file_content(&content, &path.display().to_string())
}

/// Parse rule file content from a string
pub fn parse_rule_file_content(content: &str, source_path: &str) -> Result<RuleFile> {
    let rule_file: RuleFile = toml::from_str(content)
        .with_context(|| format!("failed to parse rule file: {}", source_path))?;

    // Validate: check for duplicate rule IDs
    let mut seen_ids = std::collections::HashSet::new();
    for rule in &rule_file.rules {
        if !seen_ids.insert(&rule.id) {
            anyhow::bail!("duplicate rule ID '{}' in {}", rule.id, source_path);
        }
    }

    Ok(rule_file)
}

/// Validate a single rule
pub fn validate_rule(rule: &Rule) -> Vec<String> {
    let mut errors = Vec::new();

    if rule.id.is_empty() {
        errors.push("rule id cannot be empty".to_string());
    }
    if rule.name.is_empty() {
        errors.push(format!("rule '{}': name cannot be empty", rule.id));
    }
    if rule.prompt.is_empty() {
        errors.push(format!("rule '{}': prompt cannot be empty", rule.id));
    }

    errors
}

// Regex for parsing unified diff hunk headers: @@ -old,count +new,count @@
static HUNK_HEADER_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@").unwrap()
});

/// Annotate a unified diff with line numbers
///
/// Adds line numbers to added/context lines to help LLM reference specific lines.
/// Format: `  42 | +added line` or `  43 |  context line`
pub fn annotate_diff(diff: &str, total_lines: usize) -> String {
    let width = if total_lines == 0 {
        1
    } else {
        total_lines.to_string().len()
    };

    let mut output = Vec::new();
    let mut new_line: usize = 0;

    for raw in diff.lines() {
        // Check for hunk header
        if let Some(caps) = HUNK_HEADER_RE.captures(raw) {
            if let Some(m) = caps.get(1) {
                if let Ok(start) = m.as_str().parse::<usize>() {
                    new_line = start;
                }
            }
            output.push(raw.to_string());
            continue;
        }

        // Before first hunk (file headers)
        if new_line == 0 {
            output.push(raw.to_string());
            continue;
        }

        // Process diff lines
        let marker = raw.chars().next();
        match marker {
            Some('+') => {
                output.push(format!("{:>width$} | {}", new_line, raw, width = width));
                new_line += 1;
            }
            Some('-') => {
                // Show current newLine position; don't increment (line is removed from new file)
                output.push(format!("{:>width$} | {}", new_line, raw, width = width));
            }
            Some(' ') => {
                // Context line
                output.push(format!("{:>width$} | {}", new_line, raw, width = width));
                new_line += 1;
            }
            _ => {
                // Other (e.g., "\ No newline at end of file")
                output.push(format!("{:>width$} | {}", "", raw, width = width));
            }
        }
    }

    output.join("\n")
}

/// Add line numbers to file content
pub fn add_line_numbers(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let width = lines.len().to_string().len().max(1);

    lines
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{:>width$} | {}", i + 1, line, width = width))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_annotate_diff_basic() {
        let diff = r#"diff --git a/test.rs b/test.rs
index abc..def 100644
--- a/test.rs
+++ b/test.rs
@@ -1,3 +1,4 @@
 line one
+inserted
 line two
 line three"#;

        let annotated = annotate_diff(diff, 4);

        // Check that line numbers are added
        assert!(annotated.contains("1 |  line one"));
        assert!(annotated.contains("2 | +inserted"));
        assert!(annotated.contains("3 |  line two"));
    }

    #[test]
    fn test_annotate_diff_removal() {
        let diff = r#"@@ -1,3 +1,2 @@
 line one
-removed
 line two"#;

        let annotated = annotate_diff(diff, 2);

        // Removed line shows current new_line counter (not incremented)
        assert!(annotated.contains("2 | -removed"));
        assert!(annotated.contains("1 |  line one"));
        assert!(annotated.contains("2 |  line two"));
    }

    #[test]
    fn test_add_line_numbers() {
        let content = "first\nsecond\nthird";
        let numbered = add_line_numbers(content);

        assert!(numbered.contains("1 | first"));
        assert!(numbered.contains("2 | second"));
        assert!(numbered.contains("3 | third"));
    }

    #[test]
    fn test_validate_rule_empty_id() {
        let rule = Rule {
            id: String::new(),
            name: "Test".to_string(),
            prompt: "Check".to_string(),
            severity: Default::default(),
            enabled: true,
            context: Default::default(),
            glob_include: vec!["**/*".to_string()],
            glob_exclude: vec![],
            examples: vec![],
            needs_more_context_when: String::new(),
        };
        let errors = validate_rule(&rule);
        assert!(errors.iter().any(|e| e.contains("id cannot be empty")));
    }
}
