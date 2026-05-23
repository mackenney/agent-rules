# Step 03: Git & Parser

## Context

### Overall Objective
Build a Rust CLI that checks PR diffs against LLM-powered rules defined in `.agent-rules.toml` files. Commands: `check`, `cache stats`, `cache clear`, `rules list`, `rules validate`.

### Phase Context
Wave 1 — This step runs in parallel with step-02 (schema types). Git operations and TOML parsing are foundational infrastructure that other modules depend on.

### This Step
Implement git operations (diff, show, changed files) and TOML parsing for rule files. Also implement diff annotation (adding line numbers to unified diffs). These modules use `std::process::Command` for git (not async) and `toml` crate for parsing.

## Prerequisites
- Step 01 complete (Cargo project exists and compiles)

## Files to Read Before Starting
- `rust/src/git.rs` — Replace the placeholder stub
- `rust/src/parser.rs` — Replace the placeholder stub  
- `rust/src/config.rs` — Replace the placeholder stub

## Implementation

### Task 1: Implement git.rs

Replace `rust/src/git.rs` with:

```rust
//! Git operations: run commands, get changed files, show file content
//!
//! Uses std::process::Command (blocking). Wrap in spawn_blocking if needed.

use anyhow::{bail, Context, Result};
use std::path::Path;

/// List of binary file extensions to skip
const BINARY_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "ico", "webp", "svg",
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx",
    "zip", "tar", "gz", "bz2", "7z", "rar",
    "exe", "dll", "so", "dylib", "bin",
    "ttf", "otf", "woff", "woff2", "eot",
    "mp3", "mp4", "avi", "mov", "mkv", "webm",
    "lock", "lockb",
];

/// Run a git command and return stdout
pub fn run_git(args: &[&str], cwd: &Path) -> Result<String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .with_context(|| format!("failed to run: git {}", args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git {} failed: {}", args.join(" "), stderr.trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Run git command, returning None on failure instead of error
pub fn run_git_optional(args: &[&str], cwd: &Path) -> Option<String> {
    run_git(args, cwd).ok()
}

/// Get the repository root directory
pub fn get_repo_root(cwd: &Path) -> Result<std::path::PathBuf> {
    let output = run_git(&["rev-parse", "--show-toplevel"], cwd)?;
    Ok(std::path::PathBuf::from(output.trim()))
}

/// Check if a file path has a binary extension
pub fn is_binary_extension(path: &str) -> bool {
    path.rsplit('.')
        .next()
        .map(|ext| BINARY_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// A changed file from git diff
#[derive(Debug, Clone)]
pub struct ChangedFile {
    pub path: String,
    pub status: FileStatus,
}

/// Git file status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
}

/// Get list of changed files between two refs
pub fn get_changed_files(base_ref: &str, head_ref: &str, cwd: &Path) -> Result<Vec<ChangedFile>> {
    let output = run_git(&["diff", "--name-status", base_ref, head_ref], cwd)?;

    let mut files = Vec::new();
    for line in output.lines() {
        if line.trim().is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split('\t').collect();
        if parts.is_empty() {
            continue;
        }

        let status_char = parts[0].chars().next().unwrap_or('M');
        let status = match status_char {
            'A' => FileStatus::Added,
            'D' => FileStatus::Deleted,
            'R' => FileStatus::Renamed,
            _ => FileStatus::Modified,
        };

        // For renamed files, use the new name (second path)
        let path = if status == FileStatus::Renamed && parts.len() >= 3 {
            parts[2].to_string()
        } else if parts.len() >= 2 {
            parts[1].to_string()
        } else {
            continue;
        };

        files.push(ChangedFile { path, status });
    }

    Ok(files)
}

/// Get diff for a specific file
pub fn get_file_diff(base_ref: &str, head_ref: &str, file_path: &str, cwd: &Path) -> Result<String> {
    run_git(&["diff", base_ref, head_ref, "--", file_path], cwd)
}

/// Get file content at a specific ref
pub fn get_file_at_ref(ref_: &str, file_path: &str, cwd: &Path) -> Option<String> {
    let spec = format!("{}:{}", ref_, file_path);
    run_git_optional(&["show", &spec], cwd)
}

/// Count total lines in a file (for diff annotation width calculation)
pub fn count_file_lines(content: &str) -> usize {
    if content.is_empty() {
        0
    } else {
        content.lines().count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_binary_extension() {
        assert!(is_binary_extension("image.png"));
        assert!(is_binary_extension("FILE.PNG"));
        assert!(is_binary_extension("path/to/doc.pdf"));
        assert!(!is_binary_extension("code.rs"));
        assert!(!is_binary_extension("Makefile"));
    }

    #[test]
    fn test_count_file_lines() {
        assert_eq!(count_file_lines(""), 0);
        assert_eq!(count_file_lines("one"), 1);
        assert_eq!(count_file_lines("one\ntwo"), 2);
        assert_eq!(count_file_lines("one\ntwo\n"), 2);
    }
}
```

### Task 2: Implement parser.rs

Replace `rust/src/parser.rs` with:

```rust
//! TOML parsing for rule files and diff annotation
//!
//! Handles .agent-rules.toml parsing and unified diff annotation with line numbers.

use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::Path;

// Import schema types (will be available after step-02)
// For now we reference them; they'll resolve when both steps merge
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
            anyhow::bail!(
                "duplicate rule ID '{}' in {}",
                rule.id,
                source_path
            );
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
static HUNK_HEADER_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@").unwrap());

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
                // Removed lines don't exist in new file, show placeholder
                output.push(format!("{:>width$} | {}", "", raw, width = width));
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

        // Removed line has no line number
        assert!(annotated.contains("  | -removed"));
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
```

### Task 3: Implement config.rs

Replace `rust/src/config.rs` with:

```rust
//! Configuration loading and defaults
//!
//! Defines CheckConfig with all tunable parameters and their defaults.

use std::path::PathBuf;

/// Default model for stateless evaluation
pub const DEFAULT_MODEL: &str = "claude-haiku-4-5";

/// Default timeout in milliseconds
pub const DEFAULT_TIMEOUT_MS: u64 = 60_000;

/// Default max concurrent stateless calls
pub const DEFAULT_MAX_CONCURRENT: usize = 10;

/// Default max file size in bytes
pub const DEFAULT_MAX_FILE_BYTES: u64 = 100_000;

/// Default max diff chars
pub const DEFAULT_MAX_DIFF_CHARS: usize = 8_000;

/// Default max content chars
pub const DEFAULT_MAX_CONTENT_CHARS: usize = 20_000;

/// Cache version (bump to invalidate all caches)
pub const CACHE_VERSION: u32 = 2;

/// Configuration for a check run
#[derive(Debug, Clone)]
pub struct CheckConfig {
    /// Base git ref (e.g., "main")
    pub base_ref: String,
    /// Head git ref (e.g., "HEAD")
    pub head_ref: String,
    /// GitHub PR URL (for comment posting)
    pub pr_url: Option<String>,
    /// Repository root path
    pub repo_root: PathBuf,
    /// Explicit files to check (overrides git diff)
    pub files: Vec<PathBuf>,
    /// Directory filters
    pub dir_filters: Vec<String>,
    /// Output format
    pub output_format: OutputFormat,
    /// Treat warnings as errors (exit 1)
    pub warn_as_error: bool,
    /// Disable cache
    pub no_cache: bool,
    /// Model for stateless evaluation
    pub model: String,
    /// Max concurrent stateless LLM calls
    pub max_concurrent: usize,
    /// Max file size in bytes
    pub max_file_bytes: u64,
    /// Max diff chars to send to LLM
    pub max_diff_chars: usize,
    /// Max content chars to send to LLM
    pub max_content_chars: usize,
    /// Timeout for stateless calls (ms)
    pub timeout_ms: u64,
    /// Verbose output (full diagnostics)
    pub verbose: bool,
    /// Trace mode (print prompts/responses)
    pub trace: bool,
    /// Post comment to PR
    pub post_comment: bool,
    /// Strict rule file matching (fail on missing)
    pub strict_rules: bool,
}

impl Default for CheckConfig {
    fn default() -> Self {
        Self {
            base_ref: "main".to_string(),
            head_ref: "HEAD".to_string(),
            pr_url: None,
            repo_root: PathBuf::from("."),
            files: vec![],
            dir_filters: vec![],
            output_format: OutputFormat::Text,
            warn_as_error: false,
            no_cache: false,
            model: DEFAULT_MODEL.to_string(),
            max_concurrent: DEFAULT_MAX_CONCURRENT,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            max_diff_chars: DEFAULT_MAX_DIFF_CHARS,
            max_content_chars: DEFAULT_MAX_CONTENT_CHARS,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            verbose: false,
            trace: false,
            post_comment: false,
            strict_rules: false,
        }
    }
}

/// Output format options
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
    Github,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "text" => Ok(OutputFormat::Text),
            "json" => Ok(OutputFormat::Json),
            "github" => Ok(OutputFormat::Github),
            _ => Err(format!("unknown output format: {}", s)),
        }
    }
}

/// Get the cache directory path
pub fn get_cache_dir() -> PathBuf {
    // Use XDG cache dir or ~/.cache
    if let Some(cache) = dirs::cache_dir() {
        cache.join("agent-rules")
    } else {
        // Fallback to home directory
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".cache")
            .join("agent-rules")
    }
}

/// Get API key from environment
pub fn get_api_key() -> Option<String> {
    std::env::var("ANTHROPIC_API_KEY").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = CheckConfig::default();
        assert_eq!(config.base_ref, "main");
        assert_eq!(config.model, DEFAULT_MODEL);
        assert_eq!(config.max_concurrent, DEFAULT_MAX_CONCURRENT);
    }

    #[test]
    fn test_output_format_parse() {
        assert_eq!("text".parse::<OutputFormat>().unwrap(), OutputFormat::Text);
        assert_eq!("JSON".parse::<OutputFormat>().unwrap(), OutputFormat::Json);
        assert_eq!("github".parse::<OutputFormat>().unwrap(), OutputFormat::Github);
        assert!("unknown".parse::<OutputFormat>().is_err());
    }
}
```

### Task 4: Add dirs dependency

The `config.rs` uses `dirs` crate for XDG paths. Verify `Cargo.toml` does NOT already have dirs — if not, add it:

**If `dirs` is missing from Cargo.toml, add under [dependencies]:**
```toml
dirs = "6"
```

Note: Check if already present in step-01's Cargo.toml. The task spec Cargo.toml doesn't include dirs, so it needs to be added.

## Acceptance Criteria

These must ALL pass before reporting complete:

- [ ] `cd rust && cargo build 2>&1 | grep -E "^error" | wc -l` — outputs `0`
- [ ] `cd rust && cargo test git:: 2>&1 | grep -E "^test result"` — shows `ok` with 0 failed
- [ ] `cd rust && cargo test parser:: 2>&1 | grep -E "^test result"` — shows `ok` with 0 failed
- [ ] `cd rust && cargo test config:: 2>&1 | grep -E "^test result"` — shows `ok` with 0 failed
- [ ] `grep -c "pub fn run_git" rust/src/git.rs` — outputs `1`
- [ ] `grep -c "pub fn annotate_diff" rust/src/parser.rs` — outputs `1`
- [ ] `grep -c "pub struct CheckConfig" rust/src/config.rs` — outputs `1`
- [ ] No regressions: `cd rust && cargo test 2>&1 | grep -E "^test result"` — shows 0 failed

## Reviewer Instructions

You are reviewing Step 03. Verify:

1. Run `cd rust && cargo test git::` — all tests pass
2. Run `cd rust && cargo test parser::` — all tests pass
3. Run `cd rust && cargo test config::` — all tests pass
4. Check `rust/src/git.rs` contains:
   - `run_git()` using `std::process::Command`
   - `get_changed_files()` parsing `--name-status` output
   - `get_file_at_ref()` for file content
   - `is_binary_extension()` check
5. Check `rust/src/parser.rs` contains:
   - `parse_rule_file()` and `parse_rule_file_content()`
   - `annotate_diff()` with line number prefixes
   - `add_line_numbers()` for file content
   - Validation for duplicate rule IDs
6. Check `rust/src/config.rs` contains:
   - `CheckConfig` with all documented fields
   - Default values matching spec
   - `get_cache_dir()` using dirs crate
7. Run `cd rust && cargo clippy 2>&1 | grep "^error"` — no errors

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong>"

## Rollback
```bash
git checkout HEAD -- rust/src/git.rs rust/src/parser.rs rust/src/config.rs rust/Cargo.toml
```
