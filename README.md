# agent-rules

> Directory-scoped AI rule enforcement for PR reviews.

## What is this?

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

## Verdict Model

The LLM emits three verdicts:

| Verdict | Meaning |
|---------|---------|
| `pass` | No violation found |
| `fail` | Rule violated |
| `needs-more-context` | Internal routing signal — escalates to agentic pass |

The **display outcome** is computed from the LLM verdict plus the rule's `severity`:

| LLM verdict | Rule severity | Display outcome | Blocks merge? |
|-------------|---------------|-----------------|---------------|
| `pass` | any | `pass` | No |
| `fail` | `warn` | `warn` | No (unless `--warn-as-error`) |
| `fail` | `error` | `error` | Yes (exit 2) |

`needs-more-context` is an internal routing signal only. If the agentic pass cannot resolve it, it collapses to `fail` (conservative default).

## Key Features

- **Directory-scoped rules** — cascade from root to subdirectories, child rules override parent by ID
- **Two-pass evaluation** — fast stateless pass first; only escalates `needs-more-context` to agentic tool-use pass
- **Content-hash cache** — skip re-checking files/rules that haven't changed (flat JSON files keyed by SHA-256, critical for CI cost)
- **Parallel execution** — all files checked concurrently with configurable concurrency limit
- **GitHub CI + local parity** — same tool, same results everywhere

## Installation

```bash
npm install -g agent-rules
# or run directly
npx agent-rules check --base main
```

Requires Node.js 20.6+.

## Usage

```bash
# Check changed files between branches
agent-rules check --base main --head HEAD

# Check specific files
agent-rules check --files src/api/auth.ts src/models/user.ts

# Output formats
agent-rules check --output json    # structured JSON
agent-rules check --output github  # GitHub PR comment markdown

# Treat warn-severity violations as blocking (exit 1 on warn)
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

### `check` flags

| Flag | Default | Description |
|------|---------|-------------|
| `--base <ref>` | `main` | Base git ref for diff |
| `--head <ref>` | `HEAD` | Head git ref |
| `--pr <url>` | — | GitHub PR URL |
| `--files <paths...>` | — | Check specific files instead of git diff |
| `--repo <path>` | CWD | Repository root |
| `--output <format>` | `text` | Output format: `text`, `json`, `github` |
| `--warn-as-error` | `false` | Exit 1 on warn-severity violations (errors always exit 2) |
| `--no-cache` | — | Disable cache |
| `--model <name>` | `claude-haiku-4-5` | Override LLM model for stateless pass |
| `--max-concurrent <n>` | `10` | Max parallel LLM calls |
| `--verbose` | `false` | Show full diagnostic output with source context |
| `--trace` | `false` | Print raw prompts and LLM responses to stderr; implies `--verbose` |
| `--post-comment` | `false` | Post results as a GitHub PR comment |
| `--allow-bash` | `false` | Enable bash tool in agentic escalation |
| `--agentic-timeout <ms>` | `180000` | Timeout for agentic escalation |
| `--agentic-model <model>` | `claude-sonnet-4-6` | Model for agentic escalation |

## Rule Format

Rules are defined in `.agent-rules.toml` files using flat TOML `[[rules]]` tables:

```toml
version = "1"
inherit_mode = "merge"  # "merge" (default) or "replace"

[[rules]]
id = "security/no-raw-sql"
name = "No Raw SQL Queries"
severity = "error"          # "warn" or "error"
glob-include = ["**/*.ts"]
prompt = """
Check if this file contains raw SQL queries built with string interpolation
or concatenation. Parameterized queries only.
"""

[[rules.examples]]
description = "String interpolation in SQL"
code = 'db.query(`SELECT * FROM ${table}`)'
verdict = "fail"

[[rules.examples]]
description = "Parameterized query"
code = 'db.query("SELECT * FROM users WHERE id = $1", [id])'
verdict = "pass"

[[rules]]
id = "arch/no-db-in-controller"
name = "No Direct DB Access in Controllers"
severity = "warn"
glob-include = ["src/api/controllers/**"]
context = "agentic"          # escalates to tool-use pass for full context
needs_more_context_when = """
  The import path resolves to an external service layer, not a DB model.
"""
prompt = """
Check if this controller directly imports or queries database models.
Controllers should only call service/repository layer functions.
"""
```

### Rule fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `id` | string | required | Unique identifier (e.g. `security/no-raw-sql`) |
| `name` | string | required | Human-readable name |
| `severity` | `warn`\|`error` | `warn` | Outcome severity when the LLM returns `fail` |
| `enabled` | bool | `true` | Whether this rule is active |
| `context` | `stateless`\|`agentic` | `stateless` | Use `agentic` for rules needing file access |
| `prompt` | string | required | Instruction for the LLM reviewer |
| `glob-include` | string[] | `["**/*"]` | Files this rule applies to |
| `glob-exclude` | string[] | `[]` | Files to skip |
| `examples` | array | `[]` | Few-shot examples for the LLM |
| `needs_more_context_when` | string | `""` | Guidance to the LLM on when to emit `needs-more-context`. Only meaningful when `context = "agentic"`; on stateless rules the verdict collapses to `fail` regardless |

## GitHub Actions Integration

See `examples/github-actions.yml` for a complete workflow with split cache restore/save (ensures the cache is written even when the check fails on an error verdict).  The minimal setup:

```yaml
name: agent-rules PR check

on:
  pull_request:
    types: [opened, synchronize, reopened]

permissions:
  contents: read
  pull-requests: write

jobs:
  check:
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0

      - uses: actions/setup-node@v4
        with:
          node-version: "20"

      - run: npm ci

      - name: Run agent-rules
        env:
          ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
          npx agent-rules check \
            --base ${{ github.event.pull_request.base.sha }} \
            --head ${{ github.event.pull_request.head.sha }} \
            --pr   ${{ github.event.pull_request.html_url }} \
            --post-comment
```

## Development

```bash
npm install
npm run build     # tsc → dist/
npm test          # vitest unit tests
npm run test:integration  # requires ANTHROPIC_API_KEY
```

## Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `ANTHROPIC_API_KEY` | Yes | Anthropic API key |
| `GITHUB_TOKEN` | For `--post-comment` | GitHub token with PR write access |
| `GITHUB_STEP_SUMMARY` | CI only | Written automatically in GitHub Actions |


