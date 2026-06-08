# Step 03 — Wire `lib.rs` and `main.rs`; Delete Old Files

## Objective

Update module declarations so the new directory modules replace the old flat files. Delete the old files. Fix any remaining imports.

## Prerequisites

Steps 01 and 02 must both be complete.

## Actions

### 1. Update `rust/src/lib.rs`

Replace the module declarations. The file currently has:

```rust
pub mod agentic;
pub mod cache;
pub mod config;
pub mod evaluator;
pub mod git;
pub mod llm;
pub mod openrouter;
pub mod parser;
pub mod progress;
pub mod prompt;
pub mod reporter;
pub mod resolver;
pub mod runner;
pub mod schema;
```

Change to:

```rust
pub mod cache;
pub mod config;
pub mod evaluator;
pub mod git;
pub mod parser;
pub mod progress;
pub mod prompt;
pub mod reporter;
pub mod resolver;
pub mod runner;
pub mod schema;
```

Removed: `pub mod agentic`, `pub mod llm`, `pub mod openrouter` — these are now submodules of `evaluator`.

The `pub mod evaluator` declaration now resolves to `evaluator/mod.rs` (directory module) instead of `evaluator.rs` (flat file). Rust picks up the directory form automatically when `evaluator/mod.rs` exists and `evaluator.rs` does not.

### 2. Delete old flat files

```bash
rm rust/src/evaluator.rs
rm rust/src/llm.rs
rm rust/src/openrouter.rs
rm rust/src/agentic.rs
```

### 3. Update `rust/src/main.rs`

#### 3a. Add `mod commands` declaration and update imports

The import block (lines 1–25) changes from:

```rust
use agent_rules::agentic::PiAgenticEvaluator;
use agent_rules::evaluator::{AgenticEvaluator, StatelessEvaluator};
use agent_rules::llm::AnthropicClient;
use agent_rules::openrouter::OpenRouterClient;
```

to:

```rust
// These imports are no longer needed in main.rs itself — they moved to commands/check.rs
```

More precisely, `main.rs` changes as follows:

**Remove these imports** (moved to command handlers):
- `use agent_rules::agentic::PiAgenticEvaluator;`
- `use agent_rules::evaluator::{AgenticEvaluator, StatelessEvaluator};`
- `use agent_rules::llm::AnthropicClient;`
- `use agent_rules::openrouter::OpenRouterClient;`
- `use agent_rules::cache::{Cache, CacheManager};`
- `use agent_rules::config::{get_api_key, CheckConfig, OutputFormat, Provider};`
- `use agent_rules::git::get_repo_root;`
- `use agent_rules::parser::{parse_rule_file, validate_rule, RULE_FILE_NAME};`
- `use agent_rules::progress::{create_progress_reporter, NullProgress};`
- `use agent_rules::reporter::{exit_code_for_report, print_report, Stylesheet};`
- `use agent_rules::resolver::{find_all_rule_files, resolve_rules_for_file};`
- `use agent_rules::runner::{check_pr, CheckInfra};`
- `use agent_rules::schema::Severity;`
- `use std::sync::Arc;`

**Keep/change these imports** in `main.rs`:
```rust
use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use owo_colors::OwoColorize;
use std::io::IsTerminal;
use std::path::PathBuf;

use agent_rules::config::{OutputFormat, Provider};
use agent_rules::reporter::Stylesheet;
```

**Add**:
```rust
mod commands;

use commands::{run_cache, run_check, run_rules};
```

#### 3b. Make CLI structs visible to `commands/`

Change visibility of these types from private to `pub(crate)`:
- `struct CheckArgs` → `pub(crate) struct CheckArgs`
- `enum CacheCommands` → `pub(crate) enum CacheCommands`
- `enum RulesCommands` → `pub(crate) enum RulesCommands`

The fields of `CheckArgs` also need `pub(crate)` since the command handler accesses them. Apply `pub(crate)` to all fields of `CheckArgs` and the inner variants/fields of `CacheCommands` and `RulesCommands`.

Alternative (simpler): make the whole args sections `pub(crate)` — since these are in the binary crate, `pub(crate)` has no external visibility.

#### 3c. Remove the three function bodies from `main.rs`

Delete:
- `async fn run_check(...)` — lines 258–383 (entire function)
- `fn run_cache(...)` — lines 385–448 (entire function)
- `fn run_rules(...)` — lines 450–638 (entire function)

What remains in `main.rs`:
1. Module-level doc comment (lines 1–3)
2. Import block (reduced)
3. `mod commands;` + `use commands::{run_cache, run_check, run_rules};`
4. CLI struct definitions: `Cli`, `Commands`, `CheckArgs`, `ColorChoice`, `OutputFormatArg`, `ProviderArg`, `CacheCommands`, `RulesCommands` + the `From` impls
5. `#[tokio::main] async fn main()` (lines 232–256)

### 4. Update `runner.rs` import (verify — likely no change needed)

`runner.rs` uses `crate::evaluator::{...}` which still resolves correctly because `lib.rs` still has `pub mod evaluator` pointing to the new directory module. **No change needed.**

### 5. Verify no other files reference old module paths

Check for any remaining references to `crate::llm`, `crate::agentic`, `crate::openrouter` (as top-level modules), `agent_rules::llm`, `agent_rules::agentic`, `agent_rules::openrouter`:

```bash
cd rust
rg 'crate::(llm|agentic|openrouter)' src/ --glob '!src/evaluator/'
rg 'agent_rules::(llm|agentic|openrouter)' src/ tests/
```

Both should return zero matches after all changes. If any remain, fix them.

## Acceptance Criteria

```bash
cd rust
cargo build 2>&1 | tail -1
# Should show: Finished ...
cargo nextest run 2>&1 | tail -5
# Should show all tests passing
cargo clippy -- -D warnings 2>&1 | tail -1
# Should show no warnings/errors
```

Exact commands:

```bash
cd rust && cargo build && echo "BUILD OK"
cd rust && cargo nextest run && echo "TESTS OK"
cd rust && cargo clippy -- -D warnings && echo "CLIPPY OK"
```

All three must exit 0.
