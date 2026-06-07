# Step 03: CLI Provider Flag

## Context

### Overall Objective

Add OpenRouter as a second LLM provider. This step adds the `--provider` CLI flag to the `check` subcommand, enabling users to select between `anthropic` (default) and `openrouter`.

### Phase Context

Wave 3 — depends on step-01 (`Provider` enum) and step-02 (which already modified `main.rs` to add `mod openrouter;`). This step only adds the CLI argument parsing; the actual wiring of provider selection into `run_check()` happens in step-04.

### This Step

Add a `--provider` flag to `CheckArgs` using a `ProviderArg` clap `ValueEnum`, mirroring the existing `OutputFormatArg` pattern. Add a model-slash guard that errors when `--model` contains `/` but `--provider` is `anthropic`.

## Prerequisites

- step-01 complete (`Provider` enum exists in `config.rs`)
- step-02 complete (`mod openrouter;` added to `main.rs` — step-03 modifies `main.rs` further)

## Files to Read Before Starting

- `rust/src/main.rs` — full file; focus on `CheckArgs` struct (lines 76-169), `OutputFormatArg` enum (lines 171-186), and `run_check()` function (lines 254+)
- `rust/src/config.rs` — `Provider` enum, `DEFAULT_OPENROUTER_MODEL`

## Implementation

### Task 1: Add `ProviderArg` enum to `main.rs`

Add after the existing `OutputFormatArg` enum and its `From` impl (after line 186):

```rust
#[derive(Clone, Copy, ValueEnum)]
enum ProviderArg {
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
```

Note: clap `ValueEnum` derives lowercase strings by default, so `Openrouter` maps to `--provider openrouter` on the CLI. The variant is `Openrouter` (not `OpenRouter`) because clap derives the CLI value from the variant name in kebab-case.

### Task 2: Add `--provider` field to `CheckArgs`

Add after the `model` field (around line 116):

```rust
    /// LLM provider: anthropic, openrouter
    #[arg(long, default_value = "anthropic")]
    provider: ProviderArg,
```

### Task 3: Add model-slash guard in `run_check()`

In `run_check()`, after the `config` struct is built (around line 293), before the `stateless` evaluator is created, add:

```rust
    let provider: Provider = args.provider.into();

    if provider == Provider::Anthropic && config.model.contains('/') {
        bail!(
            "Model '{}' looks like an OpenRouter model (contains '/'). \
             Did you mean --provider openrouter?",
            config.model
        );
    }
```

Note: The `provider` variable is declared but not yet used for provider selection — step-04 does that wiring. For now, suppress the unused variable warning by prefixing with underscore: `let _provider: Provider = args.provider.into();`. Actually, since the guard uses it immediately, no underscore needed.

Wait — the `config` struct literal is built before this point and includes `args.model`. The guard should go right after the config struct literal is complete. But we also need to store `provider` in `config.provider`. Do that here:

Add `provider: args.provider.into(),` to the `CheckConfig` struct literal in `run_check()` (after the `model: args.model,` line).

Then place the guard after the config struct is built:

```rust
    if config.provider == Provider::Anthropic && config.model.contains('/') {
        bail!(
            "Model '{}' looks like an OpenRouter model (contains '/'). \
             Did you mean --provider openrouter?",
            config.model
        );
    }
```

### Task 4: Ensure imports are correct

The import line in `main.rs` should already have `Provider` from step-01:

```rust
use crate::config::{get_api_key, CheckConfig, OutputFormat, Provider};
```

If not present, add `Provider` to this import.

## Acceptance Criteria

- [ ] `cd rust && cargo build 2>&1` exits 0
- [ ] `cd rust && cargo clippy -- -D warnings 2>&1` exits 0
- [ ] `cd rust && cargo nextest run 2>&1` exits 0
- [ ] `cd rust && cargo build && ./target/debug/agent-rules check --help 2>&1 | grep -q 'provider'` exits 0
- [ ] `cd rust && cargo build && ./target/debug/agent-rules check --help 2>&1 | grep -q 'openrouter'` exits 0
- [ ] `grep -q 'ProviderArg' rust/src/main.rs` exits 0
- [ ] `grep -q 'provider: ProviderArg' rust/src/main.rs` exits 0
- [ ] `grep -q "looks like an OpenRouter model" rust/src/main.rs` exits 0

## Reviewer Instructions

1. Run all acceptance criteria commands
2. Verify `--provider` has `default_value = "anthropic"` (not required, has default)
3. Verify the model-slash guard triggers `bail!` (exit 3) when `--provider anthropic --model some/model`
4. Verify `config.provider` is populated from `args.provider.into()`
5. Verify no changes to the actual provider selection logic in `run_check()` — step-04 does that

## Rollback

```bash
cd rust
git checkout -- src/main.rs
```
