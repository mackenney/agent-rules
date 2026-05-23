#![allow(dead_code)]

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

use anyhow::Result;
use clap::Parser;

#[derive(Parser)]
#[command(name = "agent-rules")]
#[command(about = "Check PR diffs against LLM-powered rules")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Check files against rules
    Check,
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

#[derive(clap::Subcommand)]
enum CacheCommands {
    /// Show cache statistics
    Stats,
    /// Clear the cache
    Clear,
}

#[derive(clap::Subcommand)]
enum RulesCommands {
    /// List rules that apply to a file
    List,
    /// Validate rule files
    Validate,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Check => {
            todo!("check command")
        }
        Commands::Cache { command } => match command {
            CacheCommands::Stats => todo!("cache stats"),
            CacheCommands::Clear => todo!("cache clear"),
        },
        Commands::Rules { command } => match command {
            RulesCommands::List => todo!("rules list"),
            RulesCommands::Validate => todo!("rules validate"),
        },
    }
}
