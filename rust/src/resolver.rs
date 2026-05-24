//! Rule resolution: walk directory tree, glob match, merge rules
//!
//! The resolver walks from a file path up to the repository root,
//! collecting .agent-rules.toml files and merging them according
//! to inherit_mode settings.

use anyhow::Result;
use globset::{GlobSet, GlobSetBuilder};
use std::path::{Path, PathBuf};

use crate::parser::{RULE_FILE_NAME, parse_rule_file};
use crate::schema::{InheritMode, Rule, RuleFile};

/// Resolve all rules that apply to a given file
pub fn resolve_rules_for_file(file_path: &Path, repo_root: &Path) -> Result<Vec<Rule>> {
    // Collect rule files from file's directory up to repo root
    let rule_files = collect_rule_files(file_path, repo_root)?;

    // Merge rules according to inherit_mode
    let merged = merge_rule_files(rule_files);

    // Filter by glob patterns
    let relative_path = file_path
        .strip_prefix(repo_root)
        .unwrap_or(file_path)
        .to_string_lossy();

    let matching = filter_rules_by_glob(&merged, &relative_path);

    Ok(matching)
}

/// Collect all rule files from file's directory up to repo root
/// Returns in order: repo root first, file's dir last (so closer rules override)
fn collect_rule_files(file_path: &Path, repo_root: &Path) -> Result<Vec<RuleFile>> {
    let mut rule_files = Vec::new();

    // Start from the file's parent directory
    let start_dir = file_path.parent().unwrap_or(file_path);

    // Normalize paths for comparison
    let repo_root = repo_root
        .canonicalize()
        .unwrap_or_else(|_| repo_root.to_path_buf());

    let mut current = if start_dir.is_absolute() {
        start_dir.to_path_buf()
    } else {
        std::env::current_dir()?.join(start_dir)
    };
    current = current.canonicalize().unwrap_or(current);

    // Guard: if path is outside repo_root, start from repo_root instead
    if !current.starts_with(&repo_root) {
        current = repo_root.clone();
    }

    // Walk up the directory tree
    loop {
        let rule_file_path = current.join(RULE_FILE_NAME);
        if rule_file_path.exists() {
            match parse_rule_file(&rule_file_path) {
                Ok(rf) => rule_files.push(rf),
                Err(e) => {
                    // Log warning but continue (non-fatal)
                    eprintln!(
                        "Warning: failed to parse {}: {}",
                        rule_file_path.display(),
                        e
                    );
                }
            }
        }

        // Stop at repo root
        if current == repo_root {
            break;
        }

        // Move to parent
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => break,
        }
    }

    // Reverse so repo root rules come first (will be overridden by closer rules)
    rule_files.reverse();

    Ok(rule_files)
}

/// Merge rule files according to inherit_mode
fn merge_rule_files(rule_files: Vec<RuleFile>) -> Vec<Rule> {
    let mut merged_rules: Vec<Rule> = Vec::new();
    let mut disabled_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for rf in rule_files {
        match rf.inherit_mode {
            InheritMode::Replace => {
                // Discard all inherited rules
                merged_rules.clear();
                disabled_ids.clear();
            }
            InheritMode::Merge => {
                // Keep inherited rules, collect disabled IDs
                for id in &rf.disable_rules {
                    disabled_ids.insert(id.clone());
                }
            }
        }

        // Add/override rules from this file
        for rule in rf.rules {
            // Remove any existing rule with same ID
            merged_rules.retain(|r| r.id != rule.id);
            // Add the new rule
            merged_rules.push(rule);
        }
    }

    // Filter out rules in the disable-rules list
    merged_rules.retain(|r| !disabled_ids.contains(&r.id));

    // Filter out rules with enabled = false
    merged_rules.retain(|r| r.enabled);

    merged_rules
}

/// Filter rules by glob patterns
fn filter_rules_by_glob(rules: &[Rule], file_path: &str) -> Vec<Rule> {
    rules
        .iter()
        .filter(|rule| glob_matches(file_path, rule))
        .cloned()
        .collect()
}

/// Check if a file matches a rule's glob patterns
pub fn glob_matches(file_path: &str, rule: &Rule) -> bool {
    // Build include matcher
    let include_set = build_glob_set(&rule.glob_include);
    let exclude_set = build_glob_set(&rule.glob_exclude);

    // Must match at least one include pattern
    let included = match &include_set {
        Some(set) => set.is_match(file_path),
        None => true, // No includes = match all
    };

    if !included {
        return false;
    }

    // Must not match any exclude pattern
    match &exclude_set {
        Some(set) => !set.is_match(file_path),
        None => true,
    }
}

/// Build a GlobSet from patterns, with `literal_separator(true)` so `*` does NOT
/// cross directory boundaries — matching micromatch's default behavior.
fn build_glob_set(patterns: &[String]) -> Option<GlobSet> {
    if patterns.is_empty() {
        return None;
    }

    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        // IMPORTANT: literal_separator(true) ensures `*` doesn't match `/`
        // This matches micromatch semantics used by the TypeScript implementation.
        // Without this, `*.ts` would match `src/foo.ts` which micromatch would not.
        match globset::GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
        {
            Ok(glob) => {
                builder.add(glob);
            }
            Err(_) => eprintln!("Warning: invalid glob pattern: {}", pattern),
        }
    }

    builder.build().ok()
}

/// Find all rule files in a repository
pub fn find_all_rule_files(repo_root: &Path) -> Result<Vec<PathBuf>> {
    let mut rule_files = Vec::new();

    fn walk_dir(dir: &Path, rule_files: &mut Vec<PathBuf>) -> Result<()> {
        if !dir.is_dir() {
            return Ok(());
        }

        // Check for rule file in this directory
        let rule_path = dir.join(RULE_FILE_NAME);
        if rule_path.exists() {
            rule_files.push(rule_path);
        }

        // Recurse into subdirectories
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            // Skip hidden directories and common non-source dirs
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('.')
                    || matches!(
                        name,
                        "node_modules" | "target" | "dist" | "__pycache__" | ".next" | ".cache"
                    )
                {
                    continue;
                }
            }

            if path.is_dir() {
                walk_dir(&path, rule_files)?;
            }
        }

        Ok(())
    }

    walk_dir(repo_root, &mut rule_files)?;
    Ok(rule_files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Severity;

    fn make_rule(id: &str, include: Vec<&str>, exclude: Vec<&str>) -> Rule {
        Rule {
            id: id.to_string(),
            name: id.to_string(),
            prompt: "test".to_string(),
            severity: Severity::Warn,
            enabled: true,
            context: Default::default(),
            glob_include: include.into_iter().map(String::from).collect(),
            glob_exclude: exclude.into_iter().map(String::from).collect(),
            examples: vec![],
            needs_more_context_when: String::new(),
        }
    }

    #[test]
    fn test_glob_matches_basic() {
        let rule = make_rule("test", vec!["**/*.rs"], vec![]);
        assert!(glob_matches("src/main.rs", &rule));
        assert!(!glob_matches("src/main.ts", &rule));
    }

    #[test]
    fn test_glob_matches_exclude() {
        let rule = make_rule("test", vec!["**/*"], vec!["**/test_*.rs"]);
        assert!(glob_matches("src/main.rs", &rule));
        assert!(!glob_matches("src/test_utils.rs", &rule));
    }

    #[test]
    fn test_glob_matches_all() {
        let rule = make_rule("test", vec!["**/*"], vec![]);
        assert!(glob_matches("anything.txt", &rule));
        assert!(glob_matches("deep/nested/path.rs", &rule));
    }

    #[test]
    fn test_merge_replace() {
        let rf1 = RuleFile {
            inherit_mode: InheritMode::Merge,
            rules: vec![make_rule("parent", vec!["**/*"], vec![])],
            disable_rules: vec![],
        };
        let rf2 = RuleFile {
            inherit_mode: InheritMode::Replace,
            rules: vec![make_rule("child", vec!["**/*"], vec![])],
            disable_rules: vec![],
        };

        let merged = merge_rule_files(vec![rf1, rf2]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "child");
    }

    #[test]
    fn test_merge_disable() {
        let rf1 = RuleFile {
            inherit_mode: InheritMode::Merge,
            rules: vec![make_rule("parent", vec!["**/*"], vec![])],
            disable_rules: vec![],
        };
        let rf2 = RuleFile {
            inherit_mode: InheritMode::Merge,
            rules: vec![make_rule("child", vec!["**/*"], vec![])],
            disable_rules: vec!["parent".to_string()],
        };

        let merged = merge_rule_files(vec![rf1, rf2]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "child");
    }

    #[test]
    fn test_merge_override() {
        let mut parent_rule = make_rule("same-id", vec!["**/*"], vec![]);
        parent_rule.severity = Severity::Warn;

        let mut child_rule = make_rule("same-id", vec!["**/*"], vec![]);
        child_rule.severity = Severity::Error;

        let rf1 = RuleFile {
            inherit_mode: InheritMode::Merge,
            rules: vec![parent_rule],
            disable_rules: vec![],
        };
        let rf2 = RuleFile {
            inherit_mode: InheritMode::Merge,
            rules: vec![child_rule],
            disable_rules: vec![],
        };

        let merged = merge_rule_files(vec![rf1, rf2]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].severity, Severity::Error);
    }
}
