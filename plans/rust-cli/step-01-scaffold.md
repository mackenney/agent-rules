# Step 01: Scaffold

## Context

### Overall Objective
Build a Rust CLI that checks PR diffs against LLM-powered rules defined in `.agent-rules.toml` files. Commands: `check`, `cache stats`, `cache clear`, `rules list`, `rules validate`.

### Phase Context
Wave 0 — This is the foundation step that must complete before any other work begins. All subsequent steps depend on the Cargo project existing with proper dependencies and module stubs.

### This Step
Create the Rust project structure in `rust/` subdirectory. Set up `Cargo.toml` with all dependencies, create `main.rs` with module declarations, and create empty stub files for all modules. The goal is a project that compiles (with warnings about unused code) so parallel steps can begin implementation.

## Prerequisites
- Rust toolchain installed (rustc 1.75+)
- Working directory is the worktree root (contains `package.json`, `src/`, etc.)

## Files to Read Before Starting
- This file only — no other files needed for scaffolding

## Implementation

### Task 1: Create rust/ directory structure

Create the following directory structure:
```
rust/
├── Cargo.toml
└── src/
    ├── main.rs
    ├── schema.rs
    ├── config.rs
    ├── git.rs
    ├── parser.rs
    ├── resolver.rs
    ├── cache.rs
    ├── prompt.rs
    ├── llm.rs
    ├── runner.rs
    ├── reporter.rs
    └── progress.rs
```

### Task 2: Create Cargo.toml

Create `rust/Cargo.toml` with these exact contents:

```toml
[package]
name = "agent-rules"
version = "0.1.0"
edition = "2021"

[dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "time", "sync", "process", "macros"] }
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls", "charset"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
sha2 = "0.10"
hex = "0.4"
globset = "0.4"
clap = { version = "4", features = ["derive", "env"] }
indicatif = "0.17"
owo-colors = "4"
anyhow = "1"
thiserror = "2"
regex = "1"
# once_cell not needed — use std::sync::LazyLock (stable since Rust 1.80)

[dev-dependencies]
tempfile = "3"
# Note: no atty crate — use std::io::IsTerminal (stable since Rust 1.70)

### Task 3: Create main.rs with module declarations

Create `rust/src/main.rs`:

```rust
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
```

### Task 4: Create module stub files

Create each module file with a minimal stub that compiles. Each file should have a comment indicating its purpose.

**rust/src/schema.rs:**
```rust
//! Core data types: Rule, Verdict, FileVerdict, PRReport, FileDiff

use serde::{Deserialize, Serialize};

/// Placeholder - will be implemented in step-02
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
}
```

**rust/src/config.rs:**
```rust
//! Configuration loading and defaults

/// Placeholder - will be implemented in step-03
pub struct CheckConfig;
```

**rust/src/git.rs:**
```rust
//! Git operations: diff, show, changed files

use anyhow::Result;
use std::path::Path;

/// Placeholder - will be implemented in step-03
pub fn run_git(_args: &[&str], _cwd: &Path) -> Result<String> {
    todo!()
}
```

**rust/src/parser.rs:**
```rust
//! TOML parsing and diff annotation

/// Placeholder - will be implemented in step-03
pub fn annotate_diff(_diff: &str, _total_lines: usize) -> String {
    todo!()
}
```

**rust/src/resolver.rs:**
```rust
//! Rule resolution: directory walking, glob matching, merging

/// Placeholder - will be implemented in step-04
pub fn resolve_rules() {
    todo!()
}
```

**rust/src/cache.rs:**
```rust
//! Caching: FileCache, NullCache, key derivation

/// Placeholder - will be implemented in step-04
pub struct CacheManager;
```

**rust/src/prompt.rs:**
```rust
//! Prompt building for LLM calls

/// Placeholder - will be implemented in step-05
pub fn build_system_prompt() -> String {
    todo!()
}
```

**rust/src/llm.rs:**
```rust
//! Anthropic API client and retry logic

use thiserror::Error;

/// LLM-specific errors (needs retry classification)
#[derive(Debug, Error)]
pub enum LlmError {
    #[error("rate limited")]
    RateLimit,
    #[error("server error: {0}")]
    ServerError(u16),
    #[error("request failed: {0}")]
    Request(String),
    #[error("timeout")]
    Timeout,
    #[error("retries exhausted")]
    Exhausted,
}

/// Placeholder - will be implemented in step-05
pub struct AnthropicClient;
```

**rust/src/runner.rs:**
```rust
//! Check orchestration: check_file, check_pr, concurrency

/// Placeholder - will be implemented in step-06
pub async fn check_pr() {
    todo!()
}
```

**rust/src/reporter.rs:**
```rust
//! Output formatting: Text, JSON, GitHub reporters

/// Placeholder - will be implemented in step-07
pub fn print_report() {
    todo!()
}
```

**rust/src/progress.rs:**
```rust
//! Progress reporting: TTY progress bar, CI output

/// Placeholder - will be implemented in step-07
pub trait ProgressReporter: Send + Sync {
    fn set_total(&self, n: usize);
    fn on_file_start(&self, path: &str);
    fn on_file_done(&self, path: &str);
    fn finish(&self);
}
```

## Acceptance Criteria

These must ALL pass before reporting complete:

- [ ] `cd rust && cargo build 2>&1 | tail -5` — exits 0 (warnings OK, no errors)
- [ ] `ls rust/src/*.rs | wc -l` — outputs `12` (main.rs + 11 modules)
- [ ] `grep -c "^mod " rust/src/main.rs` — outputs `11`
- [ ] `cd rust && cargo run -- --help 2>&1 | head -3` — shows "Check PR diffs against LLM-powered rules"
- [ ] `cd rust && cargo run -- check 2>&1 | grep -q "not yet implemented"` — exits 0 (todo panics are expected)

## Reviewer Instructions

You are reviewing Step 01. Verify:

1. Run `cd rust && cargo build` — must exit 0 (warnings allowed)
2. Run `ls rust/src/` — must show: main.rs, schema.rs, config.rs, git.rs, parser.rs, resolver.rs, cache.rs, prompt.rs, llm.rs, runner.rs, reporter.rs, progress.rs
3. Run `cd rust && cargo run -- --help` — must show subcommands: check, cache, rules
4. Verify `rust/Cargo.toml` has all required dependencies (tokio, reqwest, serde, clap, etc.)
5. Run `cd rust && cargo clippy 2>&1 | grep -c "^error"` — must output `0`

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong>"

## Rollback
```bash
rm -rf rust/
```
