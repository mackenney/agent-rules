//! Core data types: Rule, Verdict, FileVerdict, PRReport, FileDiff
//!
//! These types mirror the TypeScript implementation for behavioral compatibility.
//! Serde derives enable TOML parsing (rules) and JSON output (reports).

use serde::{Deserialize, Serialize};

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default)]
    pub cached: bool,
}

/// All verdicts for a single file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileVerdict {
    pub file_path: String,
    pub verdicts: Vec<RuleVerdict>,
    pub passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_severity: Option<Severity>,
    #[serde(default)]
    pub skipped: bool,
    #[serde(default)]
    pub skip_reason: Option<String>,
    #[serde(default)]
    pub cached: bool,
}

impl FileVerdict {
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
    pub base_ref: String,
    pub head_ref: String,
    pub files: Vec<FileVerdict>,
    pub passed: bool,
    pub total_rules_run: usize,
    pub total_failures: usize,
    pub cached_count: usize,
}

/// File diff data for passing to LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    pub diff: String,
    pub content: Option<String>,
    pub is_binary: bool,
    pub is_deleted: bool,
    pub is_new: bool,
    pub is_oversized: bool,
    pub oversized_bytes: Option<u64>,
}
