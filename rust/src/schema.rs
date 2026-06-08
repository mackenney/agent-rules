//! Core data types: Rule, Verdict, FileVerdict, PRReport, FileDiff
//!
//! These types mirror the TypeScript implementation for behavioral compatibility.
//! Serde derives enable TOML parsing (rules) and JSON output (reports).

use serde::{Deserialize, Serialize};

/// A single rule definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    /// Unique rule identifier (used for override/disable lookups)
    pub id: String,
    /// Human-readable rule name
    pub name: String,
    #[serde(default)]
    /// Severity applied when the rule fails
    pub severity: Severity,
    #[serde(default = "default_true")]
    /// Whether the rule is enabled; disabled rules are skipped
    pub enabled: bool,
    #[serde(default)]
    /// Evaluation context (stateless or agentic)
    pub context: RuleContext,
    /// Prompt text sent to the LLM for evaluation
    pub prompt: String,
    #[serde(default = "default_glob_include", alias = "glob-include")]
    /// Glob patterns for files this rule applies to
    pub glob_include: Vec<String>,
    #[serde(default, alias = "glob-exclude")]
    /// Glob patterns for files this rule should skip
    pub glob_exclude: Vec<String>,
    #[serde(default)]
    /// Pass/fail examples for prompt context
    pub examples: Vec<RuleExample>,
    #[serde(default, alias = "needs-more-context-when")]
    /// Condition under which stateless pass may request more context
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
    /// Warning-level failure (does not block by default)
    #[default]
    Warn,
    /// Error-level failure (blocks merge)
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
    /// Evaluate using a single stateless LLM call
    #[default]
    Stateless,
    /// Parsed but treated as stateless (no agentic evaluator in Rust impl)
    Agentic,
}

/// Example for a rule (pass/fail demonstration)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuleExample {
    /// Source code snippet demonstrating the rule
    pub code: String,
    /// Serde alias supports both "verdict" (TS format) and the older field name
    #[serde(default = "default_example_verdict", alias = "verdict")]
    pub verdict: ExampleVerdict,
    #[serde(default, alias = "description")]
    /// Human-readable description of why this example passes or fails
    pub description: String,
}

/// Indicates whether a rule example represents a passing or failing case.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ExampleVerdict {
    /// The example represents code that passes the rule
    #[default]
    Pass,
    /// The example represents code that fails the rule
    Fail,
}

fn default_example_verdict() -> ExampleVerdict {
    ExampleVerdict::Pass
}

/// Serde default for confidence when deserializing old cache entries
fn confidence_default() -> f64 {
    1.0
}

/// Hint for agentic escalation when stateless pass returns needs-more-context
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContextHint {
    /// Files the LLM suggests reading to resolve the verdict
    pub read_files: Vec<String>,
    /// Question the LLM is trying to answer
    pub question: String,
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

/// Raw verdict from LLM (before collapse)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// File passes the rule
    Pass,
    /// File violates the rule
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
    /// Rule check passed
    Pass,
    /// Rule check failed (includes collapsed needs-more-context)
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

/// Overall verdict for a PR report
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OverallVerdict {
    /// All checked files passed all rules
    #[default]
    Pass,
    /// At least one warning-severity failure, no errors
    Warn,
    /// Serializes as "error" to match the TypeScript implementation and SPEC.
    #[serde(rename = "error")]
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

/// Verdict for a single rule on a single file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleVerdict {
    /// Rule identifier
    pub rule_id: String,
    /// Rule display name
    pub rule_name: String,
    /// Raw verdict from the LLM
    pub verdict: Verdict,
    /// Confidence 0.0–1.0. Runtime default (LLM omits): 0.5. Serde default (old cache): 1.0.
    #[serde(default = "confidence_default")]
    pub confidence: f64,
    #[serde(default)]
    /// Human-readable explanation from the LLM
    pub reasoning: String,
    #[serde(default)]
    /// Severity of this verdict's rule
    pub severity: Severity,
    /// All line numbers cited by the LLM (for verbose source context)
    #[serde(default)]
    pub line_refs: Vec<u32>,
    /// First line ref — kept for backward compat with cache/JSON
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default)]
    /// Whether this verdict was served from cache
    pub cached: bool,
    /// True if this verdict came from the agentic pass
    #[serde(default)]
    pub from_agentic: bool,
    /// Context hint from stateless pass when verdict was needs-more-context
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_hint: Option<ContextHint>,
}

/// All verdicts for a single file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileVerdict {
    /// Relative file path
    pub file_path: String,
    /// All rule verdicts for this file
    pub verdicts: Vec<RuleVerdict>,
    /// True if all rules passed
    pub passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Highest failure severity across all failed rules
    pub max_severity: Option<Severity>,
    #[serde(default)]
    /// True if this file was skipped (size, binary, etc.)
    pub skipped: bool,
    #[serde(default)]
    /// Human-readable reason for skipping
    pub skip_reason: Option<String>,
    #[serde(default)]
    /// True if all verdicts came from cache
    pub cached: bool,
}

impl FileVerdict {
    /// Creates a new [`FileVerdict`] for the given file with its verdicts.
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
        let cached = !verdicts.is_empty() && verdicts.iter().all(|v| v.cached);

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

    /// Creates a [`FileVerdict`] representing a skipped file with the given reason.
    pub fn skipped(file_path: String, reason: String) -> Self {
        Self {
            file_path,
            verdicts: vec![],
            passed: true,
            max_severity: None,
            skipped: true,
            skip_reason: Some(reason),
            cached: false,
        }
    }
}

/// Full PR report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PRReport {
    /// Base git ref used for the diff
    pub base_ref: String,
    /// Head git ref used for the diff
    pub head_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// GitHub PR URL if provided
    pub pr_url: Option<String>,
    /// Model used for stateless evaluation
    pub model: String,
    /// Per-file evaluation results
    pub files: Vec<FileVerdict>,
    /// Aggregate verdict for the entire PR
    pub overall_verdict: OverallVerdict,
    /// Number of files evaluated (excluding skipped)
    pub files_checked: usize,
    /// Number of files where all rules passed
    pub files_passed: usize,
    /// Number of files where at least one rule failed
    pub files_failed: usize,
    /// Number of files skipped (binary, oversized, no rules, etc.)
    pub files_skipped: usize,
    /// Total rule evaluations performed
    pub rules_evaluated: usize,
    /// Number of rule evaluations that passed
    pub rules_passed: usize,
    /// Number of rule evaluations that failed
    pub rules_failed: usize,
    /// Number of rule evaluations served from cache
    pub cache_hits: usize,
    #[serde(default)]
    /// Wall-clock duration of the check run in milliseconds
    pub duration_ms: u64,
}

/// File diff data for passing to LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    /// Relative file path
    pub path: String,
    /// Annotated unified diff text
    pub diff: String,
    /// Full file content, or `None` for deleted files
    pub content: Option<String>,
    /// True if the file is binary (no diff shown)
    pub is_binary: bool,
    /// True if the file was deleted in this diff
    pub is_deleted: bool,
    /// True if the file was newly added
    pub is_new: bool,
    /// True if the file exceeds `max_file_bytes`
    pub is_oversized: bool,
    /// File size in bytes when oversized, otherwise `None`
    pub oversized_bytes: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_verdict(cached: bool) -> RuleVerdict {
        RuleVerdict {
            rule_id: "test".to_string(),
            rule_name: "Test".to_string(),
            verdict: Verdict::Pass,
            confidence: 1.0,
            reasoning: String::new(),
            severity: Severity::Warn,
            line_refs: vec![],
            line: None,
            cached,
            from_agentic: false,
            context_hint: None,
        }
    }

    #[test]
    fn test_cached_flag_all_cached() {
        let v1 = make_verdict(true);
        let v2 = make_verdict(true);
        let fv = FileVerdict::new("test.rs".to_string(), vec![v1, v2]);
        assert!(
            fv.cached,
            "file should be cached when ALL verdicts are cached"
        );
    }

    #[test]
    fn test_cached_flag_one_uncached() {
        let v1 = make_verdict(true);
        let v2 = make_verdict(false);
        let fv = FileVerdict::new("test.rs".to_string(), vec![v1, v2]);
        assert!(
            !fv.cached,
            "file should NOT be cached when ANY verdict is uncached"
        );
    }

    #[test]
    fn test_cached_flag_empty_verdicts() {
        let fv = FileVerdict::new("test.rs".to_string(), vec![]);
        assert!(
            !fv.cached,
            "file with no verdicts should not be marked cached"
        );
    }

    #[test]
    fn test_cached_flag_all_uncached() {
        let v1 = make_verdict(false);
        let v2 = make_verdict(false);
        let fv = FileVerdict::new("test.rs".to_string(), vec![v1, v2]);
        assert!(
            !fv.cached,
            "file should not be cached when no verdicts are cached"
        );
    }
    #[test]
    fn test_confidence_serde_default() {
        let json = r#"{
            "rule_id": "test",
            "rule_name": "Test",
            "verdict": "pass",
            "reasoning": "",
            "severity": "warn",
            "line_refs": [],
            "cached": false
        }"#;
        let verdict: RuleVerdict = serde_json::from_str(json).unwrap();
        assert_eq!(
            verdict.confidence, 1.0,
            "old cache entries should default to confidence 1.0"
        );
    }

    #[test]
    fn test_from_agentic_default() {
        let json = r#"{
            "rule_id": "test",
            "rule_name": "Test",
            "verdict": "fail",
            "confidence": 0.8,
            "reasoning": "reason",
            "severity": "error",
            "line_refs": [10],
            "cached": false
        }"#;
        let verdict: RuleVerdict = serde_json::from_str(json).unwrap();
        assert!(
            !verdict.from_agentic,
            "from_agentic should default to false"
        );
        assert!(
            verdict.context_hint.is_none(),
            "context_hint should default to None"
        );
    }
}
