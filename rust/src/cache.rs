//! File-based caching with SHA-256 keys
//!
//! Cache format is compatible with TypeScript implementation.
//! Key derivation must match exactly for cross-implementation cache hits.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::{CACHE_VERSION, get_cache_dir};
use crate::schema::{FileVerdict, Rule};

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
    fn put(
        &self,
        key: &str,
        verdict: &FileVerdict,
        model: &str,
        file_path: &str,
        rule_ids: &[String],
    );
    fn key_for(
        &self,
        file_path: &str,
        content: Option<&str>,
        diff: &str,
        rules: &[Rule],
        model: &str,
        provider: &str,
    ) -> String;
    fn stats(&self) -> Result<CacheStats>;
    fn clear(&self) -> Result<usize>;
}

/// Compute cache key deterministically (shared by all Cache implementations).
///
/// Key derivation MUST match TypeScript exactly for cross-implementation cache hits.
fn compute_cache_key(
    file_path: &str,
    content: Option<&str>,
    diff: &str,
    rules: &[Rule],
    model: &str,
    provider: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("version:{}\n", CACHE_VERSION));
    hasher.update(format!("model:{}\n", model));
    hasher.update(format!("provider:{}\n", provider));

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

/// File-based cache manager
pub struct CacheManager {
    cache_dir: PathBuf,
}

impl CacheManager {
    /// Create a new cache manager using the project-local cache directory
    pub fn new(repo_root: &std::path::Path) -> Result<Self> {
        Self::with_dir(get_cache_dir(repo_root))
    }

    /// Create a cache manager with a specific directory
    pub fn with_dir(cache_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&cache_dir)?;
        Ok(Self { cache_dir })
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

    fn put(
        &self,
        key: &str,
        verdict: &FileVerdict,
        model: &str,
        file_path: &str,
        rule_ids: &[String],
    ) {
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

            if path.extension().map(|e| e == "json").unwrap_or(false)
                && std::fs::remove_file(&path).is_ok()
            {
                count += 1;
            }
        }

        Ok(count)
    }

    fn key_for(
        &self,
        file_path: &str,
        content: Option<&str>,
        diff: &str,
        rules: &[Rule],
        model: &str,
        provider: &str,
    ) -> String {
        compute_cache_key(file_path, content, diff, rules, model, provider)
    }
}

/// Null cache (no-op, for --no-cache mode)
pub struct NullCache;

impl Cache for NullCache {
    fn get(&self, _key: &str) -> Option<FileVerdict> {
        None
    }

    fn put(
        &self,
        _key: &str,
        _verdict: &FileVerdict,
        _model: &str,
        _file_path: &str,
        _rule_ids: &[String],
    ) {
        // No-op
    }

    fn stats(&self) -> Result<CacheStats> {
        Ok(CacheStats::default())
    }

    fn clear(&self) -> Result<usize> {
        Ok(0)
    }

    fn key_for(
        &self,
        file_path: &str,
        content: Option<&str>,
        diff: &str,
        rules: &[Rule],
        model: &str,
        provider: &str,
    ) -> String {
        compute_cache_key(file_path, content, diff, rules, model, provider)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{RuleVerdict, Severity, Verdict};
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
                line_refs: vec![],
                line: None,
                cached: false,
                from_agentic: false,
                context_hint: None,
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

        let key1 = cache.key_for(
            "test.rs",
            Some("content"),
            "diff",
            &rules,
            "claude",
            "anthropic",
        );
        let key2 = cache.key_for(
            "test.rs",
            Some("content"),
            "diff",
            &rules,
            "claude",
            "anthropic",
        );

        assert_eq!(key1, key2);
    }

    #[test]
    fn test_cache_key_rule_order_independent() {
        let temp = TempDir::new().unwrap();
        let cache = CacheManager::with_dir(temp.path().to_path_buf()).unwrap();

        let rules1 = vec![make_test_rule("a"), make_test_rule("b")];
        let rules2 = vec![make_test_rule("b"), make_test_rule("a")];

        let key1 = cache.key_for("test.rs", Some("c"), "d", &rules1, "model", "anthropic");
        let key2 = cache.key_for("test.rs", Some("c"), "d", &rules2, "model", "anthropic");

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
