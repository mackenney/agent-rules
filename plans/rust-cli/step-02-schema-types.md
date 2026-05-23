# Step 02: Schema Types

## Context

### Overall Objective
Build a Rust CLI that checks PR diffs against LLM-powered rules defined in `.agent-rules.toml` files. Commands: `check`, `cache stats`, `cache clear`, `rules list`, `rules validate`.

### Phase Context
Wave 1 — This step runs in parallel with step-03 (git/parser). Schema types are foundational; most other modules depend on these types but not on each other during definition.

### This Step
Define all core data types with serde derives for TOML/JSON serialization. These types mirror the TypeScript implementation exactly to ensure behavioral compatibility. Key types: Rule, RuleFile, Severity, Verdict, RuleVerdict, FileVerdict, PRReport, FileDiff.

## Prerequisites
- Step 01 complete (Cargo project exists and compiles)

## Files to Read Before Starting
- `rust/src/schema.rs` — Replace the placeholder stub
- TypeScript reference (for field semantics): The TS types define rules with `id`, `name`, `severity`, `enabled`, `context`, `prompt`, `glob_include`, `glob_exclude`, `examples`, `needs_more_context_when`

## Implementation

### Task 1: Define Rule and related enums

Replace `rust/src/schema.rs` with:

```rust
//! Core data types: Rule, Verdict, FileVerdict, PRReport, FileDiff
//!
//! These types mirror the TypeScript implementation for behavioral compatibility.
//! Serde derives enable TOML parsing (rules) and JSON output (reports).

use serde::{Deserialize, Serialize};

// ============================================================================
// Rule types (parsed from .agent-rules.toml)
// ============================================================================

/// A single rule definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub severity: Severity,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub context: RuleContext,
    pub prompt: String,
    #[serde(default = "default_glob_include", alias = "glob-include")]
    pub glob_include: Vec<String>,
    #[serde(default, alias = "glob-exclude")]
    pub glob_exclude: Vec<String>,
    #[serde(default)]
    pub examples: Vec<RuleExample>,
    #[serde(default, alias = "needs-more-context-when")]
    pub needs_more_context_when: String,
}

fn default_true() -> bool {
    true
}

fn default_glob_include() -> Vec<String> {
    vec!["**/*".to_string()]
}

/// Rule severity level
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    #[default]
    Warn,
    Error,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Warn => write!(f, "warn"),
            Severity::Error => write!(f, "error"),
        }
    }
}

/// Rule evaluation context (agentic treated as stateless in Rust impl)
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RuleContext {
    #[default]
    Stateless,
    /// Parsed but treated as stateless (no agentic evaluator in Rust impl)
    Agentic,
}

/// Example for a rule (pass/fail demonstration)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleExample {
    pub code: String,
    #[serde(default)]
    pub should_pass: bool,
    #[serde(default)]
    pub explanation: String,
}

/// A rule file (.agent-rules.toml) containing rules and config
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleFile {
    /// How to handle parent rules
    #[serde(default, alias = "inherit-mode")]
    pub inherit_mode: InheritMode,
    /// Rules defined in this file
    #[serde(default)]
    pub rules: Vec<Rule>,
    /// Rule IDs to disable (inherited rules)
    #[serde(default, alias = "disable-rules")]
    pub disable_rules: Vec<String>,
}

/// How a rule file inherits from parent directories
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InheritMode {
    /// Merge with parent rules (default)
    #[default]
    Merge,
    /// Replace parent rules entirely
    Replace,
}

// ============================================================================
// Verdict types (from LLM evaluation)
// ============================================================================

/// Raw verdict from LLM (before collapse)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    Pass,
    Fail,
    /// Collapsed to Fail in Rust impl (no agentic evaluator)
    NeedsMoreContext,
}

impl Verdict {
    /// Collapse needs-more-context to fail (no agentic in Rust impl)
    pub fn resolve(self) -> ResolvedVerdict {
        match self {
            Verdict::Pass => ResolvedVerdict::Pass,
            Verdict::Fail | Verdict::NeedsMoreContext => ResolvedVerdict::Fail,
        }
    }
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Verdict::Pass => write!(f, "pass"),
            Verdict::Fail => write!(f, "fail"),
            Verdict::NeedsMoreContext => write!(f, "needs-more-context"),
        }
    }
}

/// Resolved verdict (after collapsing needs-more-context)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResolvedVerdict {
    Pass,
    Fail,
}

impl std::fmt::Display for ResolvedVerdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolvedVerdict::Pass => write!(f, "pass"),
            ResolvedVerdict::Fail => write!(f, "fail"),
        }
    }
}

/// Verdict for a single rule on a single file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleVerdict {
    pub rule_id: String,
    pub rule_name: String,
    pub verdict: Verdict,
    pub confidence: f64,
    #[serde(default)]
    pub reasoning: String,
    #[serde(default)]
    pub severity: Severity,
    /// Line number in the file (1-indexed, if applicable)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// Was this result from cache?
    #[serde(default)]
    pub cached: bool,
}

/// All verdicts for a single file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileVerdict {
    pub file_path: String,
    pub verdicts: Vec<RuleVerdict>,
    /// True if all rules passed
    pub passed: bool,
    /// Highest severity among failures (None if passed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_severity: Option<Severity>,
    /// Was this file skipped? (binary, oversized, etc.)
    #[serde(default)]
    pub skipped: bool,
    #[serde(default)]
    pub skip_reason: Option<String>,
    /// From cache?
    #[serde(default)]
    pub cached: bool,
}

impl FileVerdict {
    /// Create a new FileVerdict with computed fields
    pub fn new(file_path: String, verdicts: Vec<RuleVerdict>) -> Self {
        let passed = verdicts
            .iter()
            .all(|v| v.verdict.resolve() == ResolvedVerdict::Pass);
        let max_severity = verdicts
            .iter()
            .filter(|v| v.verdict.resolve() == ResolvedVerdict::Fail)
            .map(|v| v.severity)
            .max_by_key(|s| match s {
                Severity::Error => 1,
                Severity::Warn => 0,
            });
        let cached = verdicts.iter().any(|v| v.cached);

        Self {
            file_path,
            verdicts,
            passed,
            max_severity,
            skipped: false,
            skip_reason: None,
            cached,
        }
    }

    /// Create a skipped file verdict
    pub fn skipped(file_path: String, reason: &str) -> Self {
        Self {
            file_path,
            verdicts: vec![],
            passed: true,
            max_severity: None,
            skipped: true,
            skip_reason: Some(reason.to_string()),
            cached: false,
        }
    }
}

// ============================================================================
// Report types (aggregated results)
// ============================================================================

/// Overall verdict for a PR
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OverallVerdict {
    Pass,
    Warn,
    Fail,
}

impl std::fmt::Display for OverallVerdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OverallVerdict::Pass => write!(f, "pass"),
            OverallVerdict::Warn => write!(f, "warn"),
            OverallVerdict::Fail => write!(f, "fail"),
        }
    }
}

/// Complete PR check report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PRReport {
    pub base_ref: String,
    pub head_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
    pub files: Vec<FileVerdict>,
    pub overall_verdict: OverallVerdict,
    /// Total files checked (excluding skipped)
    pub files_checked: usize,
    /// Files that passed all rules
    pub files_passed: usize,
    /// Files with at least one failure
    pub files_failed: usize,
    /// Files skipped (binary, oversized, etc.)
    pub files_skipped: usize,
    /// Total rules evaluated
    pub rules_evaluated: usize,
    /// Rules that passed
    pub rules_passed: usize,
    /// Rules that failed
    pub rules_failed: usize,
    /// Cache hit count
    pub cache_hits: usize,
}

impl PRReport {
    /// Create a report from file verdicts
    pub fn new(
        base_ref: String,
        head_ref: String,
        pr_url: Option<String>,
        files: Vec<FileVerdict>,
    ) -> Self {
        let files_skipped = files.iter().filter(|f| f.skipped).count();
        let files_checked = files.len() - files_skipped;
        let files_passed = files
            .iter()
            .filter(|f| !f.skipped && f.passed)
            .count();
        let files_failed = files
            .iter()
            .filter(|f| !f.skipped && !f.passed)
            .count();

        let rules_evaluated: usize = files.iter().map(|f| f.verdicts.len()).sum();
        let rules_passed: usize = files
            .iter()
            .flat_map(|f| &f.verdicts)
            .filter(|v| v.verdict.resolve() == ResolvedVerdict::Pass)
            .count();
        let rules_failed = rules_evaluated - rules_passed;

        let cache_hits: usize = files
            .iter()
            .flat_map(|f| &f.verdicts)
            .filter(|v| v.cached)
            .count();

        // Determine overall verdict
        let has_error = files.iter().any(|f| f.max_severity == Some(Severity::Error));
        let has_warn = files.iter().any(|f| f.max_severity == Some(Severity::Warn));
        let overall_verdict = if has_error {
            OverallVerdict::Fail
        } else if has_warn {
            OverallVerdict::Warn
        } else {
            OverallVerdict::Pass
        };

        Self {
            base_ref,
            head_ref,
            pr_url,
            files,
            overall_verdict,
            files_checked,
            files_passed,
            files_failed,
            files_skipped,
            rules_evaluated,
            rules_passed,
            rules_failed,
            cache_hits,
        }
    }
}

// ============================================================================
// File diff types (from git)
// ============================================================================

/// A file changed in the PR
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    pub diff: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default)]
    pub is_binary: bool,
    #[serde(default)]
    pub is_deleted: bool,
    #[serde(default)]
    pub is_new: bool,
    #[serde(default)]
    pub is_oversized: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oversized_bytes: Option<u64>,
}

// ============================================================================
// Request types (for LLM calls)
// ============================================================================

/// Request to check a single file
#[derive(Debug, Clone)]
pub struct FileCheckRequest {
    pub file_path: String,
    pub diff: String,
    pub content: Option<String>,
    pub rules: Vec<Rule>,
    pub is_new: bool,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_severity_default() {
        let s: Severity = Default::default();
        assert_eq!(s, Severity::Warn);
    }

    #[test]
    fn test_verdict_resolve() {
        assert_eq!(Verdict::Pass.resolve(), ResolvedVerdict::Pass);
        assert_eq!(Verdict::Fail.resolve(), ResolvedVerdict::Fail);
        assert_eq!(Verdict::NeedsMoreContext.resolve(), ResolvedVerdict::Fail);
    }

    #[test]
    fn test_rule_deserialize() {
        let toml = r#"
            id = "test-rule"
            name = "Test Rule"
            prompt = "Check something"
        "#;
        let rule: Rule = toml::from_str(toml).unwrap();
        assert_eq!(rule.id, "test-rule");
        assert_eq!(rule.severity, Severity::Warn);
        assert!(rule.enabled);
        assert_eq!(rule.glob_include, vec!["**/*"]);
    }

    #[test]
    fn test_rule_file_deserialize() {
        let toml = r#"
            inherit-mode = "replace"
            disable-rules = ["old-rule"]

            [[rules]]
            id = "new-rule"
            name = "New Rule"
            prompt = "Check"
        "#;
        let rf: RuleFile = toml::from_str(toml).unwrap();
        assert_eq!(rf.inherit_mode, InheritMode::Replace);
        assert_eq!(rf.disable_rules, vec!["old-rule"]);
        assert_eq!(rf.rules.len(), 1);
    }

    #[test]
    fn test_file_verdict_new() {
        let verdicts = vec![
            RuleVerdict {
                rule_id: "r1".into(),
                rule_name: "Rule 1".into(),
                verdict: Verdict::Pass,
                confidence: 0.95,
                reasoning: String::new(),
                severity: Severity::Warn,
                line: None,
                cached: false,
            },
            RuleVerdict {
                rule_id: "r2".into(),
                rule_name: "Rule 2".into(),
                verdict: Verdict::Fail,
                confidence: 0.90,
                reasoning: "Problem found".into(),
                severity: Severity::Error,
                line: Some(42),
                cached: false,
            },
        ];
        let fv = FileVerdict::new("test.rs".into(), verdicts);
        assert!(!fv.passed);
        assert_eq!(fv.max_severity, Some(Severity::Error));
    }

    #[test]
    fn test_pr_report_stats() {
        let files = vec![
            FileVerdict::new(
                "a.rs".into(),
                vec![RuleVerdict {
                    rule_id: "r1".into(),
                    rule_name: "R1".into(),
                    verdict: Verdict::Pass,
                    confidence: 0.9,
                    reasoning: String::new(),
                    severity: Severity::Warn,
                    line: None,
                    cached: false,
                }],
            ),
            FileVerdict::skipped("b.bin".into(), "binary file"),
        ];
        let report = PRReport::new("main".into(), "HEAD".into(), None, files);
        assert_eq!(report.files_checked, 1);
        assert_eq!(report.files_skipped, 1);
        assert_eq!(report.overall_verdict, OverallVerdict::Pass);
    }

    #[test]
    fn test_verdict_json_serialization() {
        let v = Verdict::NeedsMoreContext;
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, "\"needs-more-context\"");

        let parsed: Verdict = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, v);
    }
}
```

## Acceptance Criteria

These must ALL pass before reporting complete:

- [ ] `cd rust && cargo build 2>&1 | grep -E "^error" | wc -l` — outputs `0`
- [ ] `cd rust && cargo test schema:: 2>&1 | grep -E "^test result"` — shows `ok` with 0 failed
- [ ] `grep -c "pub struct Rule" rust/src/schema.rs` — outputs `1`
- [ ] `grep -c "pub enum Verdict" rust/src/schema.rs` — outputs `1`
- [ ] `grep -c "pub struct PRReport" rust/src/schema.rs` — outputs `1`
- [ ] `grep -c "pub struct FileDiff" rust/src/schema.rs` — outputs `1`
- [ ] No regressions: `cd rust && cargo test 2>&1 | grep -E "^test result"` — shows 0 failed

## Reviewer Instructions

You are reviewing Step 02. Verify:

1. Run `cd rust && cargo test schema::` — all tests must pass
2. Check `rust/src/schema.rs` contains:
   - `Rule` with fields: id, name, severity, enabled, context, prompt, glob_include, glob_exclude, examples, needs_more_context_when
   - `RuleFile` with fields: inherit_mode, rules, disable_rules
   - `Verdict` enum with Pass, Fail, NeedsMoreContext
   - `FileVerdict` with computed `passed` and `max_severity`
   - `PRReport` with aggregated statistics
   - `FileDiff` with path, diff, content, flags
3. Verify serde derives support both TOML (rules) and JSON (reports)
4. Verify `Verdict::NeedsMoreContext.resolve()` returns `ResolvedVerdict::Fail`
5. Run `cd rust && cargo clippy 2>&1 | grep "^error"` — must show no errors

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong>"

## Rollback
```bash
git checkout HEAD -- rust/src/schema.rs
```
