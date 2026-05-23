# Step 04: Resolver & Cache

## Context

### Overall Objective
Build a Rust CLI that checks PR diffs against LLM-powered rules defined in `.agent-rules.toml` files. Commands: `check`, `cache stats`, `cache clear`, `rules list`, `rules validate`.

### Phase Context
Wave 2 — This step depends on both step-02 (schema types) and step-03 (git/parser). It implements rule resolution (walking directory tree, glob matching, merging) and file-based caching.

### This Step
Implement the rule resolver that walks from a file path up to repo root, collecting and merging `.agent-rules.toml` files. Also implement the file cache with SHA-256 key derivation matching the TypeScript implementation for cache compatibility.

## Prerequisites
- Step 02 complete (schema types available)
- Step 03 complete (parser, git, config available)

## Files to Read Before Starting
- `rust/src/resolver.rs` — Replace the placeholder stub
- `rust/src/cache.rs` — Replace the placeholder stub
- `rust/src/schema.rs` — Understand Rule, RuleFile, InheritMode types
- `rust/src/parser.rs` — Understand parse_rule_file function

## Implementation

### Task 1: Implement resolver.rs

Replace `rust/src/resolver.rs` with:

```rust
//! Rule resolution: walk directory tree, glob match, merge rules
//!
//! The resolver walks from a file path up to the repository root,
//! collecting .agent-rules.toml files and merging them according
//! to inherit_mode settings.

use anyhow::Result;
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::path::{Path, PathBuf};

use crate::parser::{parse_rule_file, RULE_FILE_NAME};
use crate::schema::{InheritMode, Rule, RuleFile};

/// Resolve all rules that apply to a given file
pub fn resolve_rules_for_file(
    file_path: &Path,
    repo_root: &Path,
) -> Result<Vec<Rule>> {
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
/// Returns in order: closest (file's dir) to furthest (repo root)
fn collect_rule_files(file_path: &Path, repo_root: &Path) -> Result<Vec<RuleFile>> {
    let mut rule_files = Vec::new();

    // Start from the file's parent directory
    let start_dir = file_path.parent().unwrap_or(file_path);

    // Normalize paths for comparison
    let repo_root = repo_root.canonicalize().unwrap_or_else(|_| repo_root.to_path_buf());

    let mut current = if start_dir.is_absolute() {
        start_dir.to_path_buf()
    } else {
        std::env::current_dir()?.join(start_dir)
    };
    current = current.canonicalize().unwrap_or(current);

    // Walk up the directory tree
    loop {
        let rule_file_path = current.join(RULE_FILE_NAME);
        if rule_file_path.exists() {
            match parse_rule_file(&rule_file_path) {
                Ok(rf) => rule_files.push(rf),
                Err(e) => {
                    // Log warning but continue (non-fatal)
                    eprintln!("Warning: failed to parse {}: {}", rule_file_path.display(), e);
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

    // Filter out disabled rules
    merged_rules.retain(|r| !disabled_ids.contains(&r.id));

    // Filter out disabled rules
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
            Ok(glob) => { builder.add(glob); }
            Err(_) => eprintln!("Warning: invalid glob pattern: {}", pattern),
        }
    }

    builder.build().ok()
}
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
                if name.starts_with('.') || name == "node_modules" || name == "target" {
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
```

### Task 2: Implement cache.rs

Replace `rust/src/cache.rs` with:

```rust
//! File-based caching with SHA-256 keys
//!
//! Cache format is compatible with TypeScript implementation.
//! Key derivation must match exactly for cross-implementation cache hits.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::{get_cache_dir, CACHE_VERSION};
use crate::schema::{FileVerdict, Rule, Severity};

/// Cache entry stored on disk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub cache_key: String,
    pub file_path: String,
    pub rule_ids: Vec<String>,
    pub model: String,
    pub created_at: f64,
    pub hit_count: u64,
    pub verdict: FileVerdict,
}

/// Cache statistics
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    pub total_entries: usize,
    pub total_size_bytes: u64,
    pub oldest_entry: Option<f64>,
    pub newest_entry: Option<f64>,
    pub total_hits: u64,
}

/// Trait for cache implementations
pub trait Cache: Send + Sync {
    fn get(&self, key: &str) -> Option<FileVerdict>;
    fn put(&self, key: &str, verdict: &FileVerdict, model: &str, file_path: &str, rule_ids: &[String]);
    fn stats(&self) -> Result<CacheStats>;
    fn clear(&self) -> Result<usize>;
}

/// File-based cache manager
pub struct CacheManager {
    cache_dir: PathBuf,
}

impl CacheManager {
    /// Create a new cache manager with the default cache directory
    pub fn new() -> Result<Self> {
        Self::with_dir(get_cache_dir())
    }

    /// Create a cache manager with a specific directory
    pub fn with_dir(cache_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&cache_dir)?;
        Ok(Self { cache_dir })
    }

    /// Generate a cache key for a file check request
    ///
    /// Key derivation MUST match TypeScript exactly for cache compatibility:
    /// - version:N
    /// - model:NAME
    /// - rule:ID:SEVERITY:PROMPT (sorted by ID)
    /// - path:FILEPATH
    /// - content:CONTENT (or empty)
    /// - diff:DIFF
    pub fn key_for(
        &self,
        file_path: &str,
        content: Option<&str>,
        diff: &str,
        rules: &[Rule],
        model: &str,
    ) -> String {
        let mut hasher = Sha256::new();

        // Order must match TypeScript exactly
        hasher.update(format!("version:{}\n", CACHE_VERSION));
        hasher.update(format!("model:{}\n", model));

        // Sort rules by ID for deterministic key
        let mut sorted_rules: Vec<&Rule> = rules.iter().collect();
        sorted_rules.sort_by(|a, b| a.id.cmp(&b.id));

        for rule in sorted_rules {
            hasher.update(format!(
                "rule:{}:{}:{}\n",
                rule.id,
                rule.severity,
                rule.prompt.trim()
            ));
        }

        hasher.update(format!("path:{}\n", file_path));
        hasher.update(format!("content:{}\n", content.unwrap_or("")));
        hasher.update(format!("diff:{}", diff));

        hex::encode(hasher.finalize())
    }

    fn entry_path(&self, key: &str) -> PathBuf {
        self.cache_dir.join(format!("{}.json", key))
    }
}

impl Cache for CacheManager {
    fn get(&self, key: &str) -> Option<FileVerdict> {
        let path = self.entry_path(key);
        let content = std::fs::read_to_string(&path).ok()?;
        let mut entry: CacheEntry = serde_json::from_str(&content).ok()?;

        // Increment hit count
        entry.hit_count += 1;
        if let Ok(json) = serde_json::to_string_pretty(&entry) {
            let _ = std::fs::write(&path, json);
        }

        // Mark verdict as cached
        let mut verdict = entry.verdict;
        verdict.cached = true;
        for rv in &mut verdict.verdicts {
            rv.cached = true;
        }

        Some(verdict)
    }

    fn put(&self, key: &str, verdict: &FileVerdict, model: &str, file_path: &str, rule_ids: &[String]) {
        let entry = CacheEntry {
            cache_key: key.to_string(),
            file_path: file_path.to_string(),
            rule_ids: rule_ids.to_vec(),
            model: model.to_string(),
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64(),
            hit_count: 0,
            verdict: verdict.clone(),
        };

        let path = self.entry_path(key);
        if let Ok(json) = serde_json::to_string_pretty(&entry) {
            let _ = std::fs::write(path, json);
        }
    }

    fn stats(&self) -> Result<CacheStats> {
        let mut stats = CacheStats::default();

        if !self.cache_dir.exists() {
            return Ok(stats);
        }

        for entry in std::fs::read_dir(&self.cache_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().map(|e| e == "json").unwrap_or(false) {
                stats.total_entries += 1;

                if let Ok(meta) = entry.metadata() {
                    stats.total_size_bytes += meta.len();
                }

                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(ce) = serde_json::from_str::<CacheEntry>(&content) {
                        stats.total_hits += ce.hit_count;

                        match stats.oldest_entry {
                            None => stats.oldest_entry = Some(ce.created_at),
                            Some(old) if ce.created_at < old => {
                                stats.oldest_entry = Some(ce.created_at)
                            }
                            _ => {}
                        }

                        match stats.newest_entry {
                            None => stats.newest_entry = Some(ce.created_at),
                            Some(new) if ce.created_at > new => {
                                stats.newest_entry = Some(ce.created_at)
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        Ok(stats)
    }

    fn clear(&self) -> Result<usize> {
        let mut count = 0;

        if !self.cache_dir.exists() {
            return Ok(0);
        }

        for entry in std::fs::read_dir(&self.cache_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if std::fs::remove_file(&path).is_ok() {
                    count += 1;
                }
            }
        }

        Ok(count)
    }
}

impl Default for CacheManager {
    fn default() -> Self {
        Self::new().expect("failed to create cache manager")
    }
}

/// Null cache (no-op, for --no-cache mode)
pub struct NullCache;

impl Cache for NullCache {
    fn get(&self, _key: &str) -> Option<FileVerdict> {
        None
    }

    fn put(&self, _key: &str, _verdict: &FileVerdict, _model: &str, _file_path: &str, _rule_ids: &[String]) {
        // No-op
    }

    fn stats(&self) -> Result<CacheStats> {
        Ok(CacheStats::default())
    }

    fn clear(&self) -> Result<usize> {
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{RuleVerdict, Verdict};
    use tempfile::TempDir;

    fn make_test_rule(id: &str) -> Rule {
        Rule {
            id: id.to_string(),
            name: id.to_string(),
            prompt: "test prompt".to_string(),
            severity: Severity::Warn,
            enabled: true,
            context: Default::default(),
            glob_include: vec!["**/*".to_string()],
            glob_exclude: vec![],
            examples: vec![],
            needs_more_context_when: String::new(),
        }
    }

    fn make_test_verdict() -> FileVerdict {
        FileVerdict {
            file_path: "test.rs".to_string(),
            verdicts: vec![RuleVerdict {
                rule_id: "rule-1".to_string(),
                rule_name: "Rule 1".to_string(),
                verdict: Verdict::Pass,
                confidence: 0.95,
                reasoning: String::new(),
                severity: Severity::Warn,
                line: None,
                cached: false,
            }],
            passed: true,
            max_severity: None,
            skipped: false,
            skip_reason: None,
            cached: false,
        }
    }

    #[test]
    fn test_cache_key_deterministic() {
        let temp = TempDir::new().unwrap();
        let cache = CacheManager::with_dir(temp.path().to_path_buf()).unwrap();

        let rules = vec![make_test_rule("rule-1")];

        let key1 = cache.key_for("test.rs", Some("content"), "diff", &rules, "claude");
        let key2 = cache.key_for("test.rs", Some("content"), "diff", &rules, "claude");

        assert_eq!(key1, key2);
    }

    #[test]
    fn test_cache_key_rule_order_independent() {
        let temp = TempDir::new().unwrap();
        let cache = CacheManager::with_dir(temp.path().to_path_buf()).unwrap();

        let rules1 = vec![make_test_rule("a"), make_test_rule("b")];
        let rules2 = vec![make_test_rule("b"), make_test_rule("a")];

        let key1 = cache.key_for("test.rs", Some("c"), "d", &rules1, "model");
        let key2 = cache.key_for("test.rs", Some("c"), "d", &rules2, "model");

        assert_eq!(key1, key2, "keys should be independent of rule order");
    }

    #[test]
    fn test_cache_put_get() {
        let temp = TempDir::new().unwrap();
        let cache = CacheManager::with_dir(temp.path().to_path_buf()).unwrap();

        let verdict = make_test_verdict();
        let key = "test-key-123";

        cache.put(key, &verdict, "model", "test.rs", &["rule-1".to_string()]);

        let retrieved = cache.get(key);
        assert!(retrieved.is_some());

        let v = retrieved.unwrap();
        assert!(v.cached);
        assert_eq!(v.file_path, "test.rs");
    }

    #[test]
    fn test_cache_miss() {
        let temp = TempDir::new().unwrap();
        let cache = CacheManager::with_dir(temp.path().to_path_buf()).unwrap();

        let result = cache.get("nonexistent-key");
        assert!(result.is_none());
    }

    #[test]
    fn test_cache_stats() {
        let temp = TempDir::new().unwrap();
        let cache = CacheManager::with_dir(temp.path().to_path_buf()).unwrap();

        // Empty cache
        let stats = cache.stats().unwrap();
        assert_eq!(stats.total_entries, 0);

        // Add entry
        cache.put("key1", &make_test_verdict(), "model", "a.rs", &[]);

        let stats = cache.stats().unwrap();
        assert_eq!(stats.total_entries, 1);
    }

    #[test]
    fn test_cache_clear() {
        let temp = TempDir::new().unwrap();
        let cache = CacheManager::with_dir(temp.path().to_path_buf()).unwrap();

        cache.put("key1", &make_test_verdict(), "model", "a.rs", &[]);
        cache.put("key2", &make_test_verdict(), "model", "b.rs", &[]);

        let cleared = cache.clear().unwrap();
        assert_eq!(cleared, 2);

        let stats = cache.stats().unwrap();
        assert_eq!(stats.total_entries, 0);
    }

    #[test]
    fn test_null_cache() {
        let cache = NullCache;

        cache.put("key", &make_test_verdict(), "model", "test.rs", &[]);
        assert!(cache.get("key").is_none());

        let stats = cache.stats().unwrap();
        assert_eq!(stats.total_entries, 0);
    }
}
```

## Acceptance Criteria

These must ALL pass before reporting complete:

- [ ] `cd rust && cargo build 2>&1 | grep -E "^error" | wc -l` — outputs `0`
- [ ] `cd rust && cargo test resolver:: 2>&1 | grep -E "^test result"` — shows `ok` with 0 failed
- [ ] `cd rust && cargo test cache:: 2>&1 | grep -E "^test result"` — shows `ok` with 0 failed
- [ ] `grep -c "pub fn resolve_rules_for_file" rust/src/resolver.rs` — outputs `1`
- [ ] `grep -c "pub fn glob_matches" rust/src/resolver.rs` — outputs `1`
- [ ] `grep -c "impl Cache for CacheManager" rust/src/cache.rs` — outputs `1`
- [ ] `grep -c "pub struct NullCache" rust/src/cache.rs` — outputs `1`
- [ ] No regressions: `cd rust && cargo test 2>&1 | grep -E "^test result"` — shows 0 failed

## Reviewer Instructions

You are reviewing Step 04. Verify:

1. Run `cd rust && cargo test resolver::` — all tests pass
2. Run `cd rust && cargo test cache::` — all tests pass
3. Check `rust/src/resolver.rs` contains:
   - `resolve_rules_for_file()` walking directory tree
   - `glob_matches()` using globset
   - `merge_rule_files()` handling InheritMode::Replace and disable_rules
   - `find_all_rule_files()` for repository scanning
4. Check `rust/src/cache.rs` contains:
   - `CacheManager` with `key_for()` using SHA-256
   - Cache key includes: version, model, rules (sorted), path, content, diff
   - `Cache` trait with get/put/stats/clear
   - `NullCache` implementation
   - Hit count tracking
5. Verify cache key derivation matches TypeScript format (version:N, model:, rule:ID:SEV:PROMPT, path:, content:, diff:)
6. Run `cd rust && cargo clippy 2>&1 | grep "^error"` — no errors

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong>"

## Rollback
```bash
git checkout HEAD -- rust/src/resolver.rs rust/src/cache.rs
```
