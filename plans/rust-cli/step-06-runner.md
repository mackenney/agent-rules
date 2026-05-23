# Step 06: Runner

## Context

### Overall Objective
Build a Rust CLI that checks PR diffs against LLM-powered rules defined in `.agent-rules.toml` files. Commands: `check`, `cache stats`, `cache clear`, `rules list`, `rules validate`.

### Phase Context
Wave 4 — This step depends on step-04 (resolver/cache) and step-05 (prompt/llm). It implements the orchestration layer that coordinates parallel LLM calls with caching and concurrency control.

### This Step
Implement `check_file` and `check_pr` functions that orchestrate the entire check flow: resolve rules, check cache, call LLM (with concurrency limits), update cache, and aggregate results. Uses `tokio::sync::Semaphore` for bounded parallelism and `JoinSet` for task collection.

## Prerequisites
- Step 04 complete (resolver, cache)
- Step 05 complete (llm, prompt)

## Files to Read Before Starting
- `rust/src/runner.rs` — Replace the placeholder stub
- `rust/src/schema.rs` — FileCheckRequest, FileVerdict, PRReport
- `rust/src/cache.rs` — Cache trait
- `rust/src/llm.rs` — AnthropicClient
- `rust/src/resolver.rs` — resolve_rules_for_file
- `rust/src/config.rs` — CheckConfig

## Implementation

### Task 1: Implement runner.rs

Replace `rust/src/runner.rs` with:

```rust
//! Check orchestration: check_file, check_pr, concurrency control
//!
//! Coordinates rule resolution, caching, LLM calls, and result aggregation.
//! Uses semaphore-based concurrency limiting and JoinSet for task management.

use anyhow::{Context, Result};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::cache::{Cache, CacheManager, NullCache};
use crate::config::CheckConfig;
use crate::git::{get_changed_files, get_file_at_ref, get_file_diff, is_binary_extension, FileStatus};
use crate::llm::AnthropicClient;
use crate::parser::annotate_diff;
use crate::progress::ProgressReporter;
use crate::resolver::resolve_rules_for_file;
use crate::schema::{FileDiff, FileVerdict, PRReport, Rule};

/// Infrastructure for running checks
pub struct CheckInfra {
    pub llm: Arc<AnthropicClient>,
    pub cache: Arc<dyn Cache>,
    pub progress: Option<Arc<dyn ProgressReporter>>,
}

impl CheckInfra {
    pub fn new(api_key: String, no_cache: bool) -> Result<Self> {
        let llm = Arc::new(AnthropicClient::new(api_key));
        let cache: Arc<dyn Cache> = if no_cache {
            Arc::new(NullCache)
        } else {
            Arc::new(CacheManager::new()?)
        };

        Ok(Self {
            llm,
            cache,
            progress: None,
        })
    }

    pub fn with_progress(mut self, progress: Arc<dyn ProgressReporter>) -> Self {
        self.progress = Some(progress);
        self
    }
}

/// Check all changed files in a PR
pub async fn check_pr(infra: &CheckInfra, config: &CheckConfig) -> Result<PRReport> {
    // Get changed files
    let changed = get_changed_files(&config.base_ref, &config.head_ref, &config.repo_root)
        .context("failed to get changed files")?;

    // Filter and prepare files
    let file_diffs: Vec<FileDiff> = changed
        .iter()
        .filter(|f| !should_skip_file(&f.path, config))
        .map(|f| prepare_file_diff(f, config))
        .collect::<Result<Vec<_>>>()?;

    // Set progress total
    if let Some(progress) = &infra.progress {
        progress.set_total(file_diffs.len());
    }

    // Check each file with concurrency control
    let semaphore = Arc::new(Semaphore::new(config.max_concurrent));
    let mut tasks: JoinSet<Result<FileVerdict>> = JoinSet::new();

    for file_diff in file_diffs {
        let infra = CheckInfra {
            llm: infra.llm.clone(),
            cache: infra.cache.clone(),
            progress: infra.progress.clone(),
        };
        let config = config.clone();
        let permit = semaphore.clone();

        tasks.spawn(async move {
            let _permit = permit.acquire().await?;
            check_file(&infra, &config, file_diff).await
        });
    }

    // Collect results
    let mut file_verdicts = Vec::new();
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(verdict)) => file_verdicts.push(verdict),
            Ok(Err(e)) => {
                // Log error but continue with other files
                eprintln!("Warning: file check failed: {}", e);
            }
            Err(e) => {
                // Task panicked
                eprintln!("Warning: task panicked: {}", e);
            }
        }
    }

    // Finish progress
    if let Some(progress) = &infra.progress {
        progress.finish();
    }

    // Build report
    let report = PRReport::new(
        config.base_ref.clone(),
        config.head_ref.clone(),
        config.pr_url.clone(),
        file_verdicts,
    );

    Ok(report)
}

/// Check a single file against its applicable rules
pub async fn check_file(
    infra: &CheckInfra,
    config: &CheckConfig,
    file_diff: FileDiff,
) -> Result<FileVerdict> {
    let file_path = &file_diff.path;

    // Report progress start
    if let Some(progress) = &infra.progress {
        progress.on_file_start(file_path);
    }

    // Handle skipped files
    if file_diff.is_binary {
        return Ok(FileVerdict::skipped(file_path.clone(), "binary file"));
    }
    if file_diff.is_deleted {
        return Ok(FileVerdict::skipped(file_path.clone(), "deleted file"));
    }
    if file_diff.is_oversized {
        let reason = format!(
            "file too large ({} bytes)",
            file_diff.oversized_bytes.unwrap_or(0)
        );
        return Ok(FileVerdict::skipped(file_path.clone(), &reason));
    }

    // Resolve rules for this file
    let file_path_buf = config.repo_root.join(file_path);
    let rules = resolve_rules_for_file(&file_path_buf, &config.repo_root)
        .context("failed to resolve rules")?;

    if rules.is_empty() {
        return Ok(FileVerdict::skipped(file_path.clone(), "no matching rules"));
    }

    // Check cache
    let cache_key = infra.cache.key_for(
        file_path,
        file_diff.content.as_deref(),
        &file_diff.diff,
        &rules,
        &config.model,
    );

    if let Some(cached) = infra.cache.get(&cache_key) {
        if let Some(progress) = &infra.progress {
            progress.on_file_done(file_path);
        }
        return Ok(cached);
    }

    // Annotate diff for LLM
    let total_lines = file_diff
        .content
        .as_ref()
        .map(|c| c.lines().count())
        .unwrap_or(100);
    let annotated_diff = annotate_diff(&file_diff.diff, total_lines);

    // Call LLM
    let timeout = Duration::from_millis(config.timeout_ms);
    let verdicts = infra
        .llm
        .evaluate(
            file_path,
            &annotated_diff,
            file_diff.content.as_deref(),
            &rules,
            file_diff.is_new,
            &config.model,
            config.max_diff_chars,
            config.max_content_chars,
            timeout,
        )
        .await
        .map_err(|e| anyhow::anyhow!("LLM evaluation failed: {}", e))?;

    // Build file verdict
    let file_verdict = FileVerdict::new(file_path.clone(), verdicts);

    // Cache result
    let rule_ids: Vec<String> = rules.iter().map(|r| r.id.clone()).collect();
    infra.cache.put(
        &cache_key,
        &file_verdict,
        &config.model,
        file_path,
        &rule_ids,
    );

    // Report progress done
    if let Some(progress) = &infra.progress {
        progress.on_file_done(file_path);
    }

    Ok(file_verdict)
}

/// Check if a file should be skipped
fn should_skip_file(path: &str, config: &CheckConfig) -> bool {
    // Skip binary files
    if is_binary_extension(path) {
        return true;
    }

    // Apply directory filters if specified
    if !config.dir_filters.is_empty() {
        let matches_filter = config.dir_filters.iter().any(|filter| {
            path.starts_with(filter) || path.contains(&format!("/{}/", filter))
        });
        if !matches_filter {
            return true;
        }
    }

    false
}

/// Prepare a file diff with content and metadata
fn prepare_file_diff(
    changed: &crate::git::ChangedFile,
    config: &CheckConfig,
) -> Result<FileDiff> {
    let path = &changed.path;
    let is_deleted = changed.status == FileStatus::Deleted;
    let is_new = changed.status == FileStatus::Added;
    let is_binary = is_binary_extension(path);

    // Get diff
    let diff = if !is_binary {
        get_file_diff(&config.base_ref, &config.head_ref, path, &config.repo_root)
            .unwrap_or_default()
    } else {
        String::new()
    };

    // Get content (unless deleted or binary)
    let (content, is_oversized, oversized_bytes) = if is_deleted || is_binary {
        (None, false, None)
    } else {
        match get_file_at_ref(&config.head_ref, path, &config.repo_root) {
            Some(c) => {
                let byte_len = c.len() as u64;
                if byte_len > config.max_file_bytes {
                    (None, true, Some(byte_len))
                } else {
                    (Some(c), false, None)
                }
            }
            None => (None, false, None),
        }
    };

    Ok(FileDiff {
        path: path.clone(),
        diff,
        content,
        is_binary,
        is_deleted,
        is_new,
        is_oversized,
        oversized_bytes,
    })
}

/// Re-export cache key generation for external use
impl dyn Cache {
    pub fn key_for(
        &self,
        file_path: &str,
        content: Option<&str>,
        diff: &str,
        rules: &[Rule],
        model: &str,
    ) -> String {
        // Delegate to CacheManager's key generation
        use sha2::{Digest, Sha256};
        use crate::config::CACHE_VERSION;

        let mut hasher = Sha256::new();
        hasher.update(format!("version:{}\n", CACHE_VERSION));
        hasher.update(format!("model:{}\n", model));

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Severity;

    #[test]
    fn test_should_skip_binary() {
        let config = CheckConfig::default();
        assert!(should_skip_file("image.png", &config));
        assert!(should_skip_file("file.exe", &config));
        assert!(!should_skip_file("code.rs", &config));
    }

    #[test]
    fn test_should_skip_dir_filter() {
        let mut config = CheckConfig::default();
        config.dir_filters = vec!["src".to_string()];

        assert!(!should_skip_file("src/main.rs", &config));
        assert!(should_skip_file("tests/test.rs", &config));
    }

    #[test]
    fn test_dir_filter_empty() {
        let config = CheckConfig::default();
        // No filters = don't skip anything (except binary)
        assert!(!should_skip_file("any/path/file.rs", &config));
    }
}
```

### Task 2: Add key_for method to Cache trait

The runner needs to generate cache keys through the Cache trait. Update `rust/src/cache.rs` to add `key_for` to the trait:

In `rust/src/cache.rs`, update the `Cache` trait to include:

```rust
pub trait Cache: Send + Sync {
    fn get(&self, key: &str) -> Option<FileVerdict>;
    fn put(&self, key: &str, verdict: &FileVerdict, model: &str, file_path: &str, rule_ids: &[String]);
    fn key_for(
        &self,
        file_path: &str,
        content: Option<&str>,
        diff: &str,
        rules: &[Rule],
        model: &str,
    ) -> String;
    fn stats(&self) -> Result<CacheStats>;
    fn clear(&self) -> Result<usize>;
}
```

And implement it for both `CacheManager` and `NullCache`. For `NullCache`:

```rust
impl Cache for NullCache {
    // ... existing methods ...

    fn key_for(
        &self,
        file_path: &str,
        content: Option<&str>,
        diff: &str,
        rules: &[Rule],
        model: &str,
    ) -> String {
        // Still compute key for consistency (even though we don't cache)
        use sha2::{Digest, Sha256};
        use crate::config::CACHE_VERSION;

        let mut hasher = Sha256::new();
        hasher.update(format!("version:{}\n", CACHE_VERSION));
        hasher.update(format!("model:{}\n", model));

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
}
```

**Note:** Remove the `impl dyn Cache` block from runner.rs after adding key_for to the trait. The implementation above in runner.rs is a workaround that should be replaced by proper trait methods.

## Acceptance Criteria

These must ALL pass before reporting complete:

- [ ] `cd rust && cargo build 2>&1 | grep -E "^error" | wc -l` — outputs `0`
- [ ] `cd rust && cargo test runner:: 2>&1 | grep -E "^test result"` — shows `ok` with 0 failed
- [ ] `grep -c "pub async fn check_pr" rust/src/runner.rs` — outputs `1`
- [ ] `grep -c "pub async fn check_file" rust/src/runner.rs` — outputs `1`
- [ ] `grep -c "Semaphore::new" rust/src/runner.rs` — outputs `1`
- [ ] `grep -c "JoinSet" rust/src/runner.rs` — outputs at least `1`
- [ ] No regressions: `cd rust && cargo test 2>&1 | grep -E "^test result"` — shows 0 failed

## Reviewer Instructions

You are reviewing Step 06. Verify:

1. Run `cd rust && cargo test runner::` — all tests pass
2. Check `rust/src/runner.rs` contains:
   - `CheckInfra` struct with llm, cache, progress fields
   - `check_pr()` that gets changed files and spawns tasks
   - `check_file()` that resolves rules, checks cache, calls LLM
   - `Semaphore` for concurrency limiting
   - `JoinSet` for task collection with error handling
   - Progress reporting hooks (on_file_start, on_file_done)
   - Cache get/put integration
3. Verify `check_file` skips: binary files, deleted files, oversized files, files with no rules
4. Verify error handling: individual file failures don't crash the whole run
5. Check that `Cache` trait now includes `key_for` method
6. Run `cd rust && cargo clippy 2>&1 | grep "^error"` — no errors

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong>"

## Rollback
```bash
git checkout HEAD -- rust/src/runner.rs rust/src/cache.rs
```
