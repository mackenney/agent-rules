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

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use owo_colors::OwoColorize;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::Arc;

use crate::cache::{Cache, CacheManager};
use crate::config::{CheckConfig, OutputFormat, get_api_key};
use crate::git::get_repo_root;
use crate::parser::{RULE_FILE_NAME, parse_rule_file, validate_rule};
use crate::progress::{NullProgress, create_progress_reporter};
use crate::reporter::{Stylesheet, exit_code_for_report, print_report};
use crate::resolver::{find_all_rule_files, resolve_rules_for_file};
use crate::runner::{CheckInfra, check_pr};
use crate::schema::Severity;

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
    #[arg(long, default_value = config::DEFAULT_MODEL)]
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
        ColorChoice::Auto => std::io::stdout().is_terminal(),
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

async fn run_check(args: CheckArgs, colors: &Stylesheet) -> Result<i32> {
    let api_key = get_api_key().context(
        "ANTHROPIC_API_KEY not set. Set the environment variable:\n  export ANTHROPIC_API_KEY=sk-ant-...",
    )?;

    let repo_root = match args.repo {
        Some(r) => r,
        None => get_repo_root(&std::env::current_dir()?)?,
    };

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

    if config.post_comment {
        if config.pr_url.is_none() {
            eprintln!("Warning: --post-comment requires --pr to be set; skipping comment");
        } else if std::env::var("GITHUB_TOKEN").is_err() {
            bail!("GITHUB_TOKEN not set (required for --post-comment)");
        } else {
            eprintln!("Note: GitHub comment posting not yet implemented");
        }
    }

    if config.strict_rules {
        eprintln!("Note: --strict-rules is not yet implemented; ignoring");
    }

    let infra = CheckInfra::new(api_key, config.no_cache, &config.repo_root)?;

    let infra = if config.output_format == OutputFormat::Json {
        infra.with_progress(Arc::new(NullProgress))
    } else {
        infra.with_progress(Arc::from(create_progress_reporter(0)))
    };

    let report = check_pr(&infra, &config).await?;

    let mut stdout = std::io::stdout();
    print_report(
        &report,
        config.output_format,
        config.verbose,
        &mut stdout,
        colors,
    )?;

    Ok(exit_code_for_report(&report, config.warn_as_error))
}

fn run_cache(command: CacheCommands, colors: &Stylesheet) -> Result<i32> {
    match command {
        CacheCommands::Stats { repo } => {
            let repo_root = match repo {
                Some(r) => r,
                None => get_repo_root(&std::env::current_dir()?)?,
            };
            let cache = CacheManager::new(&repo_root)?;
            let stats = cache.stats()?;

            println!("{}", "Cache Statistics".bold());
            println!("  Entries: {}", stats.total_entries);
            println!("  Size: {} KB", stats.total_size_bytes / 1024);
            println!("  Total hits: {}", stats.total_hits);

            if let Some(oldest) = stats.oldest_entry {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs_f64();
                let age_secs = (now - oldest).max(0.0);
                let age_str = if age_secs < 3600.0 {
                    format!("{:.0}m ago", age_secs / 60.0)
                } else if age_secs < 86400.0 {
                    format!("{:.1}h ago", age_secs / 3600.0)
                } else {
                    format!("{:.1}d ago", age_secs / 86400.0)
                };
                println!("  Oldest entry: {}", age_str);
            }

            Ok(0)
        }
        CacheCommands::Clear { repo, yes } => {
            let repo_root = match repo {
                Some(r) => r,
                None => get_repo_root(&std::env::current_dir()?)?,
            };
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

            let cache = CacheManager::new(&repo_root)?;
            let count = cache.clear()?;

            println!(
                "{} Cleared {} cache entries",
                "✓".style(colors.success),
                count
            );

            Ok(0)
        }
    }
}

fn run_rules(command: RulesCommands, colors: &Stylesheet) -> Result<i32> {
    match command {
        RulesCommands::List { path, repo } => {
            let repo_root = match repo {
                Some(r) => r,
                None => std::env::current_dir()?,
            };

            let abs_path = if path.is_absolute() {
                path.clone()
            } else {
                repo_root.join(&path)
            };
            let rules = resolve_rules_for_file(&abs_path, &repo_root)?;

            if rules.is_empty() {
                println!("No rules apply to {}", path.display());
                return Ok(0);
            }

            println!("{} rules apply to {}:", rules.len().bold(), path.display());
            println!();

            for rule in &rules {
                let severity_str = match rule.severity {
                    Severity::Error => "error".style(colors.error),
                    Severity::Warn => "warn".style(colors.warning),
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
                None => std::env::current_dir()?,
            };

            let rule_files = find_all_rule_files(&repo_root)?;

            if rule_files.is_empty() {
                println!(
                    "No {} files found in {}",
                    RULE_FILE_NAME,
                    repo_root.display()
                );
                return Ok(0);
            }

            println!(
                "Validating {} rule files in {}",
                rule_files.len(),
                repo_root.display()
            );
            println!();

            let mut all_valid = true;
            let mut total_rules = 0usize;
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
                            let errors = validate_rule(rule);
                            file_errors.extend(errors);

                            all_rule_ids
                                .entry(rule.id.clone())
                                .or_default()
                                .push(relative.clone());

                            total_rules += 1;
                        }

                        if file_errors.is_empty() {
                            println!(
                                "  {} {}  {} rule(s)",
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

            // Only report conflicts for files that are NOT in ancestor-descendant relationship.
            // Parent-child overrides (same ID in parent dir and child dir) are valid cascade.
            let conflicts: Vec<_> = all_rule_ids
                .iter()
                .filter(|(_, files)| {
                    if files.len() < 2 {
                        return false;
                    }
                    // Check if any pair of files is unrelated (not ancestor-descendant)
                    let dirs: Vec<std::path::PathBuf> = files
                        .iter()
                        .map(|f| {
                            std::path::Path::new(f)
                                .parent()
                                .map(|p| p.to_path_buf())
                                .unwrap_or_default()
                        })
                        .collect();
                    for i in 0..dirs.len() {
                        for j in (i + 1)..dirs.len() {
                            if !dirs[i].starts_with(&dirs[j]) && !dirs[j].starts_with(&dirs[i]) {
                                return true;
                            }
                        }
                    }
                    false
                })
                .collect();

            if !conflicts.is_empty() {
                all_valid = false;
                println!();
                println!(
                    "{} Cross-file rule ID conflicts detected:",
                    "Error:".style(colors.error)
                );
                for (id, files) in &conflicts {
                    println!("  {} defined in:", id.style(colors.note));
                    for f in *files {
                        println!("    - {}", f);
                    }
                }
            }

            println!();
            println!(
                "Validated {} file(s), {} rule(s) total.",
                rule_files.len(),
                total_rules
            );

            if all_valid {
                println!(
                    "{} All {} rules in {} files are valid",
                    "✓".style(colors.success),
                    total_rules,
                    rule_files.len()
                );
                Ok(0)
            } else {
                println!("{} Validation found errors", "✗".style(colors.error));
                // Exit 1 for rule validation failures (not config errors — rules validate is a linting command)
                Ok(1)
            }
        }
    }
}
