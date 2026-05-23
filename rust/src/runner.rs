//! Check orchestration: check_file, check_pr, concurrency control
//!
//! Coordinates rule resolution, caching, LLM calls, and result aggregation.
//! Uses semaphore-based concurrency limiting and JoinSet for task management.

use anyhow::{Context, Result};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::cache::{Cache, CacheManager, NullCache};
use crate::config::CheckConfig;
use crate::git::{get_changed_files, is_binary_extension};
use crate::llm::AnthropicClient;
use crate::parser::annotate_diff;
use crate::progress::ProgressReporter;
use crate::resolver::resolve_rules_for_file;
use crate::schema::{FileDiff, FileVerdict, OverallVerdict, PRReport, Severity, Verdict};

/// Infrastructure for running checks
pub struct CheckInfra {
    pub llm: Arc<AnthropicClient>,
    pub cache: Arc<dyn Cache>,
    pub progress: Option<Arc<dyn ProgressReporter>>,
}

impl CheckInfra {
    pub fn new(api_key: String, no_cache: bool, repo_root: &std::path::Path) -> Result<Self> {
        let llm = Arc::new(AnthropicClient::new(api_key));
        let cache: Arc<dyn Cache> = if no_cache {
            Arc::new(NullCache)
        } else {
            Arc::new(CacheManager::new(repo_root)?)
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
    let start = std::time::Instant::now();

    let file_diffs: Vec<FileDiff> = get_changed_files(
        &config.base_ref,
        &config.head_ref,
        &config.repo_root,
        config.max_file_bytes,
    )
    .context("failed to get changed files")?
    .into_iter()
    .filter(|f| !should_skip_file(&f.path, config))
    .collect();

    if let Some(progress) = &infra.progress {
        progress.set_total(file_diffs.len());
    }

    let semaphore = Arc::new(Semaphore::new(config.max_concurrent));
    let mut tasks: JoinSet<Result<FileVerdict>> = JoinSet::new();

    for file_diff in file_diffs {
        let infra = CheckInfra {
            llm: infra.llm.clone(),
            cache: infra.cache.clone(),
            progress: infra.progress.clone(),
        };
        let config = config.clone();
        let sem = semaphore.clone();

        tasks.spawn(async move {
            let _permit = sem.acquire().await?;
            check_file(&infra, &config, file_diff).await
        });
    }

    let mut file_verdicts: Vec<FileVerdict> = Vec::new();
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(verdict)) => file_verdicts.push(verdict),
            Ok(Err(e)) => eprintln!("Warning: file check failed: {e}"),
            Err(e) => eprintln!("Warning: task panicked: {e}"),
        }
    }

    if let Some(progress) = &infra.progress {
        progress.finish();
    }

    let duration_ms = start.elapsed().as_millis() as u64;

    Ok(build_pr_report(
        config.base_ref.clone(),
        config.head_ref.clone(),
        config.pr_url.clone(),
        file_verdicts,
        duration_ms,
    ))
}

/// Check a single file against its applicable rules
pub async fn check_file(
    infra: &CheckInfra,
    config: &CheckConfig,
    file_diff: FileDiff,
) -> Result<FileVerdict> {
    let file_path = file_diff.path.clone();

    if file_diff.is_binary {
        return Ok(FileVerdict::skipped(file_path, "binary file".to_string()));
    }
    if file_diff.is_deleted {
        return Ok(FileVerdict::skipped(file_path, "deleted file".to_string()));
    }
    if file_diff.is_oversized {
        let reason = format!(
            "file too large ({} bytes)",
            file_diff.oversized_bytes.unwrap_or(0)
        );
        return Ok(FileVerdict::skipped(file_path, reason));
    }

    let file_path_buf = config.repo_root.join(&file_path);
    let rules = resolve_rules_for_file(&file_path_buf, &config.repo_root)
        .context("failed to resolve rules")?;

    if rules.is_empty() {
        return Ok(FileVerdict::skipped(
            file_path,
            "no matching rules".to_string(),
        ));
    }

    let cache_key = infra.cache.key_for(
        &file_path,
        file_diff.content.as_deref(),
        &file_diff.diff,
        &rules,
        &config.model,
    );

    if let Some(cached) = infra.cache.get(&cache_key) {
        if let Some(progress) = &infra.progress {
            progress.on_call_done(&format!("{}[cached]", file_path));
        }
        return Ok(cached);
    }

    let total_lines = file_diff
        .content
        .as_ref()
        .map(|c| c.lines().count())
        .unwrap_or(100);
    let annotated_diff = annotate_diff(&file_diff.diff, total_lines);

    let label = format!("{}[{} rule(s)]", file_path, rules.len());
    if let Some(progress) = &infra.progress {
        progress.on_call_start(&label);
    }

    let timeout = Duration::from_millis(config.timeout_ms);
    let verdicts = infra
        .llm
        .evaluate(
            &file_path,
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
        .map_err(|e| anyhow::anyhow!("LLM evaluation failed: {e}"))?;

    let file_verdict = FileVerdict::new(file_path.clone(), verdicts);

    let rule_ids: Vec<String> = rules.iter().map(|r| r.id.clone()).collect();
    infra.cache.put(
        &cache_key,
        &file_verdict,
        &config.model,
        &file_path,
        &rule_ids,
    );

    if let Some(progress) = &infra.progress {
        progress.on_call_done(&label);
    }

    Ok(file_verdict)
}

/// Build a PRReport from collected file verdicts
fn build_pr_report(
    base_ref: String,
    head_ref: String,
    pr_url: Option<String>,
    files: Vec<FileVerdict>,
    duration_ms: u64,
) -> PRReport {
    let files_checked = files.iter().filter(|f| !f.skipped).count();
    let files_skipped = files.iter().filter(|f| f.skipped).count();
    let files_passed = files.iter().filter(|f| !f.skipped && f.passed).count();
    let files_failed = files.iter().filter(|f| !f.skipped && !f.passed).count();

    let rules_evaluated: usize = files.iter().map(|f| f.verdicts.len()).sum();
    let rules_passed: usize = files
        .iter()
        .flat_map(|f| &f.verdicts)
        .filter(|v| matches!(v.verdict, Verdict::Pass))
        .count();
    let rules_failed = rules_evaluated - rules_passed;
    let cache_hits = files.iter().filter(|f| f.cached).count();

    let overall_verdict = if files
        .iter()
        .any(|f| !f.skipped && !f.passed && f.max_severity == Some(Severity::Error))
    {
        OverallVerdict::Fail
    } else if files.iter().any(|f| !f.skipped && !f.passed) {
        OverallVerdict::Warn
    } else {
        OverallVerdict::Pass
    };

    PRReport {
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
        duration_ms,
    }
}

/// Check if a file should be skipped before resolving rules
fn should_skip_file(path: &str, config: &CheckConfig) -> bool {
    if is_binary_extension(path) {
        return true;
    }

    if !config.dir_filters.is_empty() {
        let matches_filter = config
            .dir_filters
            .iter()
            .any(|filter| path.starts_with(filter) || path.contains(&format!("/{filter}/")));
        if !matches_filter {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{RuleVerdict, Severity, Verdict};

    fn make_file_verdict(path: &str, passed: bool) -> FileVerdict {
        FileVerdict {
            file_path: path.to_string(),
            verdicts: vec![RuleVerdict {
                rule_id: "rule-1".to_string(),
                rule_name: "Rule 1".to_string(),
                verdict: if passed { Verdict::Pass } else { Verdict::Fail },
                confidence: 0.9,
                reasoning: String::new(),
                severity: Severity::Warn,
                line: None,
                cached: false,
            }],
            passed,
            max_severity: if passed { None } else { Some(Severity::Warn) },
            skipped: false,
            skip_reason: None,
            cached: false,
        }
    }

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
        assert!(!should_skip_file("any/path/file.rs", &config));
    }

    #[test]
    fn test_build_pr_report_empty() {
        let report = build_pr_report("main".to_string(), "HEAD".to_string(), None, vec![], 0);
        assert_eq!(report.files_checked, 0);
        assert_eq!(report.files_skipped, 0);
        assert_eq!(report.overall_verdict, OverallVerdict::Pass);
    }

    #[test]
    fn test_build_pr_report_all_pass() {
        let files = vec![
            make_file_verdict("a.rs", true),
            make_file_verdict("b.rs", true),
        ];
        let report = build_pr_report("main".to_string(), "HEAD".to_string(), None, files, 100);
        assert_eq!(report.files_checked, 2);
        assert_eq!(report.files_passed, 2);
        assert_eq!(report.files_failed, 0);
        assert_eq!(report.rules_evaluated, 2);
        assert_eq!(report.rules_passed, 2);
        assert_eq!(report.overall_verdict, OverallVerdict::Pass);
    }

    #[test]
    fn test_build_pr_report_with_warn() {
        let files = vec![make_file_verdict("a.rs", false)];
        let report = build_pr_report("main".to_string(), "HEAD".to_string(), None, files, 0);
        assert_eq!(report.files_failed, 1);
        assert_eq!(report.overall_verdict, OverallVerdict::Warn);
    }

    #[test]
    fn test_build_pr_report_with_error() {
        let mut fv = make_file_verdict("a.rs", false);
        fv.max_severity = Some(Severity::Error);
        let report = build_pr_report("main".to_string(), "HEAD".to_string(), None, vec![fv], 0);
        assert_eq!(report.overall_verdict, OverallVerdict::Fail);
    }

    #[test]
    fn test_build_pr_report_with_skipped() {
        let skipped = FileVerdict::skipped("bin.exe".to_string(), "binary file".to_string());
        let report = build_pr_report(
            "main".to_string(),
            "HEAD".to_string(),
            None,
            vec![skipped],
            0,
        );
        assert_eq!(report.files_skipped, 1);
        assert_eq!(report.files_checked, 0);
        assert_eq!(report.overall_verdict, OverallVerdict::Pass);
    }

    #[test]
    fn test_build_pr_report_cache_hits() {
        let mut fv = make_file_verdict("a.rs", true);
        fv.cached = true;
        let report = build_pr_report("main".to_string(), "HEAD".to_string(), None, vec![fv], 0);
        assert_eq!(report.cache_hits, 1);
    }
}
