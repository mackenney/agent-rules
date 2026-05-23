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
        assert_eq!(
            "github".parse::<OutputFormat>().unwrap(),
            OutputFormat::Github
        );
        assert!("unknown".parse::<OutputFormat>().is_err());
    }
}
