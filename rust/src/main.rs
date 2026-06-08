//! agent-rules CLI entrypoint
//!
//! Check PR diffs against LLM-powered rules defined in .agent-rules.toml files.

use clap::{Parser, Subcommand, ValueEnum};
use owo_colors::OwoColorize;
use std::io::IsTerminal;
use std::path::PathBuf;

use agent_rules::config::{OutputFormat, Provider};
use agent_rules::reporter::Stylesheet;

mod commands;

use commands::{run_cache, run_check, run_rules};

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
    Check(Box<CheckArgs>),
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
pub(crate) struct CheckArgs {
    /// Base git ref
    #[arg(long, default_value = "main")]
    pub(crate) base: String,

    /// Head git ref
    #[arg(long, default_value = "HEAD")]
    pub(crate) head: String,

    /// GitHub PR URL (for comment posting)
    #[arg(long)]
    pub(crate) pr: Option<String>,

    /// Explicit files to check (overrides git diff)
    #[arg(long)]
    pub(crate) files: Vec<PathBuf>,

    /// Repository root path
    #[arg(long)]
    pub(crate) repo: Option<PathBuf>,

    /// Directory filters (only check files in these dirs)
    #[arg(long = "dir-filter")]
    pub(crate) dir_filter: Vec<String>,

    /// Output format: text, json, github
    #[arg(long, short, default_value = "text")]
    pub(crate) output: OutputFormatArg,

    /// Treat warnings as errors (exit 1)
    #[arg(long)]
    pub(crate) warn_as_error: bool,

    /// Disable cache
    #[arg(long)]
    pub(crate) no_cache: bool,

    /// Model for stateless evaluation
    #[arg(long)]
    pub(crate) model: Option<String>,

    /// LLM provider: anthropic, openrouter
    #[arg(long, default_value = "anthropic")]
    pub(crate) provider: ProviderArg,

    /// Max concurrent stateless LLM calls
    #[arg(long, default_value = "10")]
    pub(crate) max_concurrent: usize,

    /// Max concurrent agentic escalations (independent of stateless slots)
    #[arg(long, default_value = "2")]
    pub(crate) agentic_concurrency: usize,

    /// Model for agentic escalation
    #[arg(long, default_value = agent_rules::config::DEFAULT_AGENTIC_MODEL)]
    pub(crate) agentic_model: String,

    /// Timeout for agentic sessions (ms)
    #[arg(long, default_value = "180000")]
    pub(crate) agentic_timeout: u64,

    /// Max file size in bytes
    #[arg(long, default_value = "100000")]
    pub(crate) max_file_bytes: u64,

    /// Max diff chars to send to LLM
    #[arg(long, default_value = "8000")]
    pub(crate) max_diff_chars: usize,

    /// Max content chars to send to LLM
    #[arg(long, default_value = "20000")]
    pub(crate) max_content_chars: usize,

    /// Timeout for stateless calls (ms)
    #[arg(long, default_value = "60000")]
    pub(crate) timeout: u64,

    /// Verbose output (full diagnostics)
    #[arg(long, short)]
    pub(crate) verbose: bool,

    /// Print prompts and responses (implies --verbose)
    #[arg(long)]
    pub(crate) trace: bool,

    /// Post comment to PR
    #[arg(long)]
    pub(crate) post_comment: bool,

    /// Strict rule file matching
    #[arg(long)]
    pub(crate) strict_rules: bool,

    /// Allow bash tool in agentic sessions
    #[arg(long)]
    pub(crate) allow_bash: bool,
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum OutputFormatArg {
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

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum ProviderArg {
    Anthropic,
    Openrouter,
}

impl From<ProviderArg> for Provider {
    fn from(arg: ProviderArg) -> Self {
        match arg {
            ProviderArg::Anthropic => Provider::Anthropic,
            ProviderArg::Openrouter => Provider::OpenRouter,
        }
    }
}

#[derive(Subcommand)]
pub(crate) enum CacheCommands {
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
pub(crate) enum RulesCommands {
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
        Commands::Check(args) => run_check(*args, &colors).await,
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
