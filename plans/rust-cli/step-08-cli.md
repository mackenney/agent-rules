# Step 08: CLI Wiring

## Context

### Overall Objective
Build a Rust CLI that checks PR diffs against LLM-powered rules defined in `.agent-rules.toml` files. Commands: `check`, `cache stats`, `cache clear`, `rules list`, `rules validate`.

### Phase Context
Wave 5 — This is the final step that wires everything together. It depends on all previous steps being complete.

### This Step
Wire up the complete CLI using clap derive macros. Implement all subcommands: `check`, `cache stats`, `cache clear`, `rules list`, `rules validate`. Handle argument validation, error reporting, and exit codes.

## Prerequisites
- All previous steps complete (01-07)

## Files to Read Before Starting
- `rust/src/main.rs` — Replace the scaffold with full implementation
- All other modules for imports and function signatures

## Implementation

### Task 1: Replace main.rs with full CLI implementation

Replace `rust/src/main.rs` with:

```rust
//! agent-rules CLI entrypoint
//!
//! Check PR diffs against LLM-powered rules defined in .agent-rules.toml files.

mod cache;
mod config;
mod git;
mod llm;
mod parser;
mod progress;
mod prompt;
mod reporter;
mod resolver;
mod runner;
mod schema;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use std::sync::Arc;

use crate::cache::{Cache, CacheManager};
use crate::config::{get_api_key, CheckConfig, OutputFormat};
use crate::git::get_repo_root;
use crate::parser::{parse_rule_file, validate_rule, RULE_FILE_NAME};
use crate::progress::{create_progress, NullProgress};
use crate::reporter::{exit_code_for_report, print_report, Stylesheet};
use crate::resolver::{find_all_rule_files, resolve_rules_for_file};
use crate::runner::{check_pr, CheckInfra};

/// Check PR diffs against LLM-powered rules
#[derive(Parser)]
#[command(name = "agent-rules")]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Color output: auto, always, never
    #[arg(long, global = true, default_value = "auto")]
    color: ColorChoice,
}

#[derive(Clone, Copy, ValueEnum)]
enum ColorChoice {
    Auto,
    Always,
    Never,
}

#[derive(Subcommand)]
enum Commands {
    /// Check files against rules
    Check(CheckArgs),
    /// Cache management
    Cache {
        #[command(subcommand)]
        command: CacheCommands,
    },
    /// Rule management
    Rules {
        #[command(subcommand)]
        command: RulesCommands,
    },
}

#[derive(clap::Args)]
struct CheckArgs {
    /// Base git ref
    #[arg(long, default_value = "main")]
    base: String,

    /// Head git ref
    #[arg(long, default_value = "HEAD")]
    head: String,

    /// GitHub PR URL (for comment posting)
    #[arg(long)]
    pr: Option<String>,

    /// Explicit files to check (overrides git diff)
    #[arg(long)]
    files: Vec<PathBuf>,

    /// Repository root path
    #[arg(long)]
    repo: Option<PathBuf>,

    /// Directory filters (only check files in these dirs)
    #[arg(long = "dir-filter")]
    dir_filter: Vec<String>,

    /// Output format: text, json, github
    #[arg(long, short, default_value = "text")]
    output: OutputFormatArg,

    /// Treat warnings as errors (exit 1)
    #[arg(long)]
    warn_as_error: bool,

    /// Disable cache
    #[arg(long)]
    no_cache: bool,

    /// Model for stateless evaluation
    #[arg(long, default_value = "claude-haiku-4-5")]
    model: String,

    /// Max concurrent stateless LLM calls
    #[arg(long, default_value = "10")]
    max_concurrent: usize,

    /// Max file size in bytes
    #[arg(long, default_value = "100000")]
    max_file_bytes: u64,

    /// Max diff chars to send to LLM
    #[arg(long, default_value = "8000")]
    max_diff_chars: usize,

    /// Max content chars to send to LLM
    #[arg(long, default_value = "20000")]
    max_content_chars: usize,

    /// Timeout for stateless calls (ms)
    #[arg(long, default_value = "60000")]
    timeout: u64,

    /// Verbose output (full diagnostics)
    #[arg(long, short)]
    verbose: bool,

    /// Print prompts and responses (implies --verbose)
    #[arg(long)]
    trace: bool,

    /// Post comment to PR
    #[arg(long)]
    post_comment: bool,

    /// Strict rule file matching
    #[arg(long)]
    strict_rules: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum OutputFormatArg {
    Text,
    Json,
    Github,
}

impl From<OutputFormatArg> for OutputFormat {
    fn from(arg: OutputFormatArg) -> Self {
        match arg {
            OutputFormatArg::Text => OutputFormat::Text,
            OutputFormatArg::Json => OutputFormat::Json,
            OutputFormatArg::Github => OutputFormat::Github,
        }
    }
}

#[derive(Subcommand)]
enum CacheCommands {
    /// Show cache statistics
    Stats {
        /// Repository root path
        #[arg(long)]
        repo: Option<PathBuf>,
    },
    /// Clear the cache
    Clear {
        /// Repository root path
        #[arg(long)]
        repo: Option<PathBuf>,

        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum RulesCommands {
    /// List rules that apply to a file
    List {
        /// File path to check rules for
        #[arg(long)]
        path: PathBuf,

        /// Repository root path
        #[arg(long)]
        repo: Option<PathBuf>,
    },
    /// Validate rule files in the repository
    Validate {
        /// Repository root path
        #[arg(long)]
        repo: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let color_enabled = match cli.color {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => atty::is(atty::Stream::Stdout),
    };
    let colors = Stylesheet::new(color_enabled);

    let result = match cli.command {
        Commands::Check(args) => run_check(args, &colors).await,
        Commands::Cache { command } => run_cache(command, &colors),
        Commands::Rules { command } => run_rules(command, &colors),
    };

    match result {
        Ok(exit_code) => std::process::exit(exit_code),
        Err(e) => {
            eprintln!("{}: {}", "error".red().bold(), e);
            std::process::exit(3);
        }
    }
}

/// Run the check command
async fn run_check(args: CheckArgs, colors: &Stylesheet) -> Result<i32> {
    // Get API key
    let api_key = get_api_key().context(
        "ANTHROPIC_API_KEY not set. Set the environment variable:\n  export ANTHROPIC_API_KEY=sk-ant-...",
    )?;

    // Determine repo root
    let repo_root = match args.repo {
        Some(r) => r,
        None => get_repo_root(&std::env::current_dir()?)?,
    };

    // Build config
    let config = CheckConfig {
        base_ref: args.base,
        head_ref: args.head,
        pr_url: args.pr,
        repo_root,
        files: args.files,
        dir_filters: args.dir_filter,
        output_format: args.output.into(),
        warn_as_error: args.warn_as_error,
        no_cache: args.no_cache,
        model: args.model,
        max_concurrent: args.max_concurrent,
        max_file_bytes: args.max_file_bytes,
        max_diff_chars: args.max_diff_chars,
        max_content_chars: args.max_content_chars,
        timeout_ms: args.timeout,
        verbose: args.verbose || args.trace,
        trace: args.trace,
        post_comment: args.post_comment,
        strict_rules: args.strict_rules,
    };

    // Create infrastructure
    let infra = CheckInfra::new(api_key, config.no_cache)?;

    // Add progress reporter (skip for JSON output)
    let infra = if config.output_format == OutputFormat::Json {
        infra.with_progress(Arc::new(NullProgress))
    } else {
        infra.with_progress(Arc::from(create_progress(false)))
    };

    // Run checks
    let report = check_pr(&infra, &config).await?;

    // Print report
    let mut stdout = std::io::stdout();
    print_report(&report, config.output_format, config.verbose, &mut stdout, colors)?;

    // Return exit code
    Ok(exit_code_for_report(&report, config.warn_as_error))
}

/// Run cache subcommands
fn run_cache(command: CacheCommands, colors: &Stylesheet) -> Result<i32> {
    match command {
        CacheCommands::Stats { repo: _ } => {
            let cache = CacheManager::new()?;
            let stats = cache.stats()?;

            use owo_colors::OwoColorize;

            println!("{}", "Cache Statistics".bold());
            println!("  Entries: {}", stats.total_entries);
            println!(
                "  Size: {} KB",
                stats.total_size_bytes / 1024
            );
            println!("  Total hits: {}", stats.total_hits);

            if let Some(oldest) = stats.oldest_entry {
                let age_secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs_f64()
                    - oldest;
                let age_hours = (age_secs / 3600.0) as u64;
                println!("  Oldest entry: {} hours ago", age_hours);
            }

            Ok(0)
        }
        CacheCommands::Clear { repo: _, yes } => {
            if !yes {
                eprint!("Clear all cache entries? [y/N] ");
                std::io::Write::flush(&mut std::io::stderr())?;

                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;

                if !input.trim().eq_ignore_ascii_case("y") {
                    println!("Cancelled.");
                    return Ok(0);
                }
            }

            let cache = CacheManager::new()?;
            let count = cache.clear()?;

            use owo_colors::OwoColorize;
            println!(
                "{} Cleared {} cache entries",
                "✓".style(colors.success),
                count
            );

            Ok(0)
        }
    }
}

/// Run rules subcommands
fn run_rules(command: RulesCommands, colors: &Stylesheet) -> Result<i32> {
    match command {
        RulesCommands::List { path, repo } => {
            let repo_root = match repo {
                Some(r) => r,
                None => get_repo_root(&std::env::current_dir()?)?,
            };

            let rules = resolve_rules_for_file(&path, &repo_root)?;

            use owo_colors::OwoColorize;

            if rules.is_empty() {
                println!("No rules apply to {}", path.display());
                return Ok(0);
            }

            println!(
                "{} rules apply to {}:",
                rules.len().bold(),
                path.display()
            );
            println!();

            for rule in &rules {
                let severity_str = match rule.severity {
                    schema::Severity::Error => "error".style(colors.error),
                    schema::Severity::Warn => "warn".style(colors.warning),
                };

                println!(
                    "  {} {} [{}]",
                    "•".style(colors.note),
                    rule.name.bold(),
                    rule.id.style(colors.dim),
                );
                println!("    Severity: {}", severity_str);
                if !rule.glob_include.is_empty() && rule.glob_include != vec!["**/*"] {
                    println!("    Include: {}", rule.glob_include.join(", "));
                }
                if !rule.glob_exclude.is_empty() {
                    println!("    Exclude: {}", rule.glob_exclude.join(", "));
                }
                println!();
            }

            Ok(0)
        }
        RulesCommands::Validate { repo } => {
            let repo_root = match repo {
                Some(r) => r,
                None => get_repo_root(&std::env::current_dir()?)?,
            };

            let rule_files = find_all_rule_files(&repo_root)?;

            use owo_colors::OwoColorize;

            if rule_files.is_empty() {
                println!("No {} files found in {}", RULE_FILE_NAME, repo_root.display());
                return Ok(0);
            }

            println!(
                "Validating {} rule files in {}",
                rule_files.len(),
                repo_root.display()
            );
            println!();

            let mut all_valid = true;
            let mut total_rules = 0;
            let mut all_rule_ids: std::collections::HashMap<String, Vec<String>> =
                std::collections::HashMap::new();

            for path in &rule_files {
                let relative = path
                    .strip_prefix(&repo_root)
                    .unwrap_or(path)
                    .display()
                    .to_string();

                match parse_rule_file(path) {
                    Ok(rf) => {
                        let mut file_errors = Vec::new();

                        for rule in &rf.rules {
                            // Validate individual rule
                            let errors = validate_rule(rule);
                            file_errors.extend(errors);

                            // Track rule IDs for cross-file conflict detection
                            all_rule_ids
                                .entry(rule.id.clone())
                                .or_default()
                                .push(relative.clone());

                            total_rules += 1;
                        }

                        if file_errors.is_empty() {
                            println!(
                                "  {} {} ({} rules)",
                                "✓".style(colors.success),
                                relative,
                                rf.rules.len()
                            );
                        } else {
                            all_valid = false;
                            println!("  {} {}", "✗".style(colors.error), relative);
                            for err in file_errors {
                                println!("    - {}", err.style(colors.error));
                            }
                        }
                    }
                    Err(e) => {
                        all_valid = false;
                        println!("  {} {}", "✗".style(colors.error), relative);
                        println!("    - {}", e.to_string().style(colors.error));
                    }
                }
            }

            // Check for ID conflicts
            let conflicts: Vec<_> = all_rule_ids
                .iter()
                .filter(|(_, files)| files.len() > 1)
                .collect();

            if !conflicts.is_empty() {
                println!();
                println!(
                    "{} Rule ID conflicts detected:",
                    "Warning:".style(colors.warning)
                );
                for (id, files) in conflicts {
                    println!("  {} defined in:", id.style(colors.note));
                    for f in files {
                        println!("    - {}", f);
                    }
                }
                // Conflicts are warnings, not errors (closer files override)
            }

            println!();
            if all_valid {
                println!(
                    "{} All {} rules in {} files are valid",
                    "✓".style(colors.success),
                    total_rules,
                    rule_files.len()
                );
                Ok(0)
            } else {
                println!(
                    "{} Validation found errors",
                    "✗".style(colors.error)
                );
                Ok(3)
            }
        }
    }
}

// Re-export for use by other modules
use owo_colors::OwoColorize;
```

## Acceptance Criteria

These must ALL pass before reporting complete:

- [ ] `cd rust && cargo build 2>&1 | grep -E "^error" | wc -l` — outputs `0`
- [ ] `cd rust && cargo run -- --help 2>&1 | head -3` — shows "Check PR diffs against LLM-powered rules"
- [ ] `cd rust && cargo run -- check --help 2>&1 | grep -c "\-\-base"` — outputs `1`
- [ ] `cd rust && cargo run -- cache stats --help 2>&1 | grep -c "cache statistics"` — outputs `1` (case insensitive)
- [ ] `cd rust && cargo run -- rules validate --help 2>&1 | grep -c "\-\-repo"` — outputs `1`
- [ ] `cd rust && cargo run -- rules list --path nonexistent.rs 2>&1` — runs without panic (may error on no repo)
- [ ] No regressions: `cd rust && cargo test 2>&1 | grep -E "^test result"` — shows 0 failed

## Reviewer Instructions

You are reviewing Step 08. Verify:

1. Run `cd rust && cargo build` — must exit 0
2. Run `cd rust && cargo run -- --help` — shows check, cache, rules subcommands
3. Check `rust/src/main.rs` contains:
   - Clap derive structs for Cli, Commands, CheckArgs, etc.
   - All check flags: --base, --head, --pr, --files, --repo, --output, --verbose, --trace, etc.
   - Global --color flag with auto/always/never
   - `run_check()` async function calling check_pr
   - `run_cache()` with stats and clear subcommands
   - `run_rules()` with list and validate subcommands
   - Exit codes: 0 (pass), 1 (warn + --warn-as-error), 2 (fail), 3 (error)
4. Verify cache clear has confirmation prompt (unless -y)
5. Verify rules validate checks for cross-file ID conflicts
6. Run `cd rust && cargo clippy 2>&1 | grep "^error"` — no errors

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong>"

## Rollback
```bash
git checkout HEAD -- rust/src/main.rs
```
