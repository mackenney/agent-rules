# agent-rules

> Directory-scoped AI rule enforcement for PR reviews.

`agent-rules` enforces custom coding rules at the **directory level** during PR review. Each changed file in a PR is checked against rules defined in `.agent-rules.toml` files in the repository. Rules cascade from the repo root to subdirectories; child rules override parent rules by ID.

```
src/
  .agent-rules.toml          # rules for all of src/
  api/
    .agent-rules.toml        # extends root rules + adds API-specific rules
    controllers/
      auth.ts                # checked against merged rule set
  models/
    user.ts                  # checked against src/ rules only
```

## Implementations

| Language | Directory | Status |
|----------|-----------|--------|
| TypeScript | [`typescript/`](typescript/) | Stable |
| Rust | [`rust/`](rust/) | In development |

Each implementation is a full peer — same CLI surface, same `.agent-rules.toml` format, same verdict model. The behavioral contract is in [`SPEC.md`](SPEC.md).

## Key Features

- **Directory-scoped rules** — cascade from root to subdirectories, child rules override parent by ID
- **Two-pass evaluation** — fast stateless pass first; only escalates `needs-more-context` to agentic tool-use pass
- **Provider-neutral evaluator protocol** — `StatelessEvaluator` / `AgenticEvaluator` interfaces decouple the runner from any specific SDK
- **Content-hash cache** — skip re-checking files/rules that haven't changed (flat JSON files keyed by SHA-256)
- **Parallel execution** — all files checked concurrently with configurable concurrency limit
- **GitHub CI + local parity** — same tool, same results everywhere

## Quick Start

### TypeScript

```bash
cd typescript
npm install
npm run build
node dist/cli.js check --base main --head HEAD
```

Requires Node.js 20.6+. Set `ANTHROPIC_API_KEY` before running.

### Rust

```bash
cd rust
cargo build --release
./target/release/agent-rules check --base main --head HEAD
```

Set `ANTHROPIC_API_KEY` before running.

## Usage

```bash
# Check changed files between branches
agent-rules check --base main --head HEAD

# Check specific files
agent-rules check --files src/api/auth.ts src/models/user.ts

# Output formats
agent-rules check --output json    # structured JSON
agent-rules check --output github  # GitHub PR comment markdown

# Treat warn-severity violations as blocking
agent-rules check --warn-as-error

# Post results as GitHub PR comment
agent-rules check --pr https://github.com/org/repo/pull/42 --post-comment

# Rule management
agent-rules rules list --path src/api/controller.ts
agent-rules rules validate

# Cache management
agent-rules cache stats
agent-rules cache clear
```

## Repository Layout

```
agent-rules/
├── typescript/          # TypeScript implementation
│   ├── src/
│   ├── tests/
│   ├── test-repo/       # fixture repo for integration tests
│   └── package.json
├── rust/                # Rust implementation
│   ├── src/
│   └── Cargo.toml
├── docs/                # shared documentation
├── examples/            # shared — .agent-rules.toml examples, CI configs
├── SPEC.md              # behavioral contract for all implementations
└── README.md
```

## Specification

[`SPEC.md`](SPEC.md) is the canonical behavioral contract. It covers:

- Rule definition format and TOML schema
- Rule cascading and resolution
- Two-pass evaluation (stateless → agentic)
- Evaluator protocol (`StatelessEvaluator`, `AgenticEvaluator`, `CheckInfra`)
- Verdict model and aggregation
- Caching, concurrency, output formats
- Full CLI contract and exit codes
- Verifiable conditions (falsifiable invariants)

## Examples

See [`examples/`](examples/) for:
- [`github-actions.yml`](examples/github-actions.yml) — CI workflow
- [`repo-with-rules/`](examples/repo-with-rules/) — sample `.agent-rules.toml` files

## Documentation

- [`docs/ci-caching.md`](docs/ci-caching.md) — cache persistence in CI

## License

MIT
