# Step 02 — Create `commands/` Submodule

## Objective

Extract the three command handler functions from `main.rs` into `commands/` submodule:
- `commands/mod.rs` — re-exports
- `commands/check.rs` — `run_check()`
- `commands/cache.rs` — `run_cache()`
- `commands/rules.rs` — `run_rules()`

The `commands/` module is declared in `main.rs` (binary crate), NOT in `lib.rs`. These handlers are binary-only; they import from the library crate via `agent_rules::`.

## Parallelizable

Yes — touches no files from Step 01. Can run in parallel.

## Actions

### 1. Create `rust/src/commands/mod.rs`

```rust
//! CLI subcommand handlers

mod cache;
mod check;
mod rules;

pub use cache::run_cache;
pub use check::run_check;
pub use rules::run_rules;
```

### 2. Create `rust/src/commands/check.rs`

Extract `run_check()` (main.rs lines 258–383) along with the imports it needs. This function is `async`.

Key: `run_check` references types from `main.rs` — `CheckArgs`, `Provider`, `OutputFormat`, etc. Some come from `agent_rules::*`, some are defined locally in `main.rs` (`CheckArgs`, `ProviderArg`). Since `CheckArgs` stays in `main.rs`, pass it by value and keep the import from `super` or accept the struct fields directly.

**Strategy:** `CheckArgs` and CLI types stay in `main.rs`. `run_check` takes `CheckArgs` and `&Stylesheet` as parameters. Import `CheckArgs` via `super::CheckArgs`.

```rust
//! Handler for `agent-rules check`

use std::sync::Arc;

use anyhow::{bail, Context, Result};

use agent_rules::cache::Cache;
use agent_rules::config::{get_api_key, CheckConfig, OutputFormat, Provider};
use agent_rules::evaluator::{
    AgenticEvaluator, AnthropicClient, OpenRouterClient, PiAgenticEvaluator, StatelessEvaluator,
};
use agent_rules::git::get_repo_root;
use agent_rules::progress::{create_progress_reporter, NullProgress};
use agent_rules::reporter::{exit_code_for_report, print_report, Stylesheet};
use agent_rules::runner::{check_pr, CheckInfra};

use super::CheckArgs;

pub async fn run_check(args: CheckArgs, colors: &Stylesheet) -> Result<i32> {
    // ... exact body from main.rs lines 259–383, unchanged
}
```

Note: After the evaluator refactor (Step 01 + 03), the imports use `agent_rules::evaluator::AnthropicClient` etc. During Step 02, write the imports targeting the final paths — the code won't compile until Step 03 anyway.

### 3. Create `rust/src/commands/cache.rs`

Extract `run_cache()` (main.rs lines 385–448).

```rust
//! Handler for `agent-rules cache`

use anyhow::Result;
use owo_colors::OwoColorize;

use agent_rules::cache::CacheManager;
use agent_rules::git::get_repo_root;
use agent_rules::reporter::Stylesheet;

use super::CacheCommands;

pub fn run_cache(command: CacheCommands, colors: &Stylesheet) -> Result<i32> {
    // ... exact body from main.rs lines 386–448, unchanged
}
```

### 4. Create `rust/src/commands/rules.rs`

Extract `run_rules()` (main.rs lines 450–638).

```rust
//! Handler for `agent-rules rules`

use anyhow::Result;
use owo_colors::OwoColorize;

use agent_rules::parser::{parse_rule_file, validate_rule, RULE_FILE_NAME};
use agent_rules::reporter::Stylesheet;
use agent_rules::resolver::{find_all_rule_files, resolve_rules_for_file};
use agent_rules::schema::Severity;

use super::RulesCommands;

pub fn run_rules(command: RulesCommands, colors: &Stylesheet) -> Result<i32> {
    // ... exact body from main.rs lines 451–638, unchanged
}
```

### Important details for command extraction

Each command file imports the CLI arg structs from `super::` (i.e., from `commands/mod.rs`, which re-exports from `main.rs`). But `mod commands` is declared in `main.rs`, so `super::` refers to the binary crate root — the structs in `main.rs`.

The structs that need to be visible to `commands/`:
- `CheckArgs` — used by `check.rs`
- `CacheCommands` — used by `cache.rs`
- `RulesCommands` — used by `rules.rs`

These must have `pub(crate)` visibility in `main.rs` (they're currently private, but `pub(crate)` within the binary crate is sufficient since `mod commands` is a submodule of the binary crate root).

## DO NOT do in this step

- Do not modify `main.rs` yet (Step 03 handles removal of old functions and import changes)
- Do not modify `lib.rs`

## Acceptance Criteria

```bash
cd rust
test -f src/commands/mod.rs
test -f src/commands/check.rs
test -f src/commands/cache.rs
test -f src/commands/rules.rs
```

Note: Build won't succeed until Step 03 wires everything together.
