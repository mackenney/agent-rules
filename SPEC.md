# agent-rules Specification

> The key words **MUST**, **MUST NOT**, **SHOULD**, **SHOULD NOT**, and **MAY** are used in this document per [RFC 2119](https://datatracker.ietf.org/doc/html/rfc2119).

---

## Purpose

`agent-rules` is a directory-scoped AI rule-enforcement tool for PR reviews. Given a set of changed files (from a git diff or explicit list), it evaluates each file against the rules that apply to it, using an LLM as the evaluator. The result is a structured verdict report that can block merges, post GitHub PR comments, or be consumed by downstream tooling.

---

## Non-Goals

- `agent-rules` does not parse or understand source code semantically; the LLM does.
- It does not manage secrets, rotate API keys, or authenticate users.
- It does not write to or modify the repository being checked.
- It does not perform linting, formatting, or auto-fixing.
- It does not define what constitutes a good or bad rule; that is the rule author's responsibility.

---

## Core Mental Model

1. **Rules live in `.agent-rules.toml` files**, one per directory, anywhere in the repository tree.
2. **Rules cascade**: for a given file, all rule files from the repository root down to the file's directory are collected and merged. Child rules override parent rules by ID.
3. **Each applicable rule is evaluated independently** in its own LLM call. No rule influences another rule's verdict.
4. **Two evaluation passes exist**: a fast stateless pass (no file I/O) and a slower agentic pass (can read files). The stateless pass runs first; only `needs-more-context` signals on agentic-typed rules trigger escalation.
5. **Results are cached** by content hash so re-running on unchanged files costs nothing.

---

## Rule Definition Format

### File Structure

Rules are defined in TOML files named exactly `.agent-rules.toml`. Each file is a flat TOML document with the following top-level fields:

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `version` | string | `"1"` | File format version. Currently informational only. |
| `inherit_mode` | `"merge"` \| `"replace"` | `"merge"` | Controls how this file interacts with rules from parent directories (see Rule Cascading). |
| `rules` | array of Rule | `[]` | The rules defined in this file. |

### Rule Fields

| Field | Type | Default | Required | Description |
|-------|------|---------|----------|-------------|
| `id` | string (min length 1) | — | Yes | Unique within the file it is defined in, and MUST be unique across all unrelated rule files in the repository (see Validation Constraints). Convention: `category/description`. |
| `name` | string | — | Yes | Human-readable rule name. |
| `severity` | `"warn"` \| `"error"` | `"warn"` | No | Outcome severity when the LLM returns `fail`. |
| `enabled` | boolean | `true` | No | If `false`, the rule is never evaluated. |
| `context` | `"stateless"` \| `"agentic"` | `"stateless"` | No | Evaluation mode. `agentic` allows the evaluator to read repository files. |
| `prompt` | string | — | Yes | The instruction given to the LLM evaluator. |
| `glob-include` | string[] | `["**/*"]` | No | Glob patterns (relative to repo root) that a file MUST match for this rule to apply. |
| `glob-exclude` | string[] | `[]` | No | Glob patterns. If any match the file, this rule is skipped. |
| `examples` | array of Example | `[]` | No | Few-shot examples included in the prompt. |
| `needs_more_context_when` | string | `""` | No | Guidance to the LLM on when to emit `needs-more-context`. Included in prompts regardless of `context` setting; on `context = "stateless"` rules, any resulting `needs-more-context` verdict is collapsed to `fail`. |

**TOML key aliasing**: In the TOML file, `glob-include` and `glob-exclude` (kebab-case) are accepted as aliases for the internal `glob_include` / `glob_exclude` names.

### Example Fields

| Field | Type | Description |
|-------|------|-------------|
| `description` | string | Short description of the example. |
| `code` | string | Code snippet. |
| `verdict` | `"pass"` \| `"fail"` | Expected verdict for this example. |

### Validation Constraints

- A rule's `id` MUST be non-empty.
- Duplicate rule IDs within a single `.agent-rules.toml` file MUST be rejected as a parse error.
- The same rule ID MUST NOT appear in two rule files that are not in an ancestor-descendant directory relationship. Two files are *related* when one's directory is a strict ancestor of the other's; files at the same level or on different directory branches are *unrelated*. The `rules validate` command MUST detect and report such cross-file ID conflicts.
- Unknown TOML fields on a rule are silently ignored (stripped).
- An invalid `severity` value MUST produce a schema validation error.
- A TOML syntax error in a rule file MUST produce a parse error.

---

## Rule Cascading & Resolution

### Discovery

To find the rules applicable to a given file path:

1. Compute the sequence of directories from the repository root down to the file's containing directory, inclusive on both ends.
2. For each directory in that sequence (root first, deepest last), if a `.agent-rules.toml` exists, collect it in order.
3. If the file path is outside the repository root, only the root-level `.agent-rules.toml` (if present) is consulted.

**Directory traversal skips**: `.git`, `node_modules`, `.next`, `dist`, `__pycache__`, `.cache` are not walked during global rule-file discovery (`rules validate` command), but this does not affect per-file rule resolution which walks only ancestor directories.

### Merging

Rules from collected files are merged into a map keyed by rule `id`, processing files in order from root to deepest:

1. If a file has `inherit_mode = "replace"`, the accumulated map MUST be cleared before processing that file's rules.
2. Each rule in the file is upserted into the map: a rule with the same `id` as an existing entry replaces it.

The final rule set is the values of the map after processing all files.

### Filtering

After merging, only rules that satisfy all of the following are included in the applicable set:

- `enabled` is `true`.
- The file path matches at least one pattern in `glob_include` (evaluated against the file path relative to the repository root, with dot-files included).
- The file path does NOT match any pattern in `glob_exclude`.

Glob matching MUST use the same semantics as the `micromatch` library (extglob, `**` for any path segment depth, `dot: true`).

### Source Grounding

When a head git ref is provided, rule files SHOULD be loaded from that ref rather than from disk:

- If the git ref version and the disk version differ (trailing whitespace stripped), a warning MUST be emitted to stderr.
- If `--strict-rules` is set and a discrepancy is found, the check MUST abort with an error.
- If the rule file exists at the git ref but not on disk, the git ref version is used.
- If the rule file does not exist at the git ref but exists on disk, the disk version is used with a warning (or an error under `--strict-rules`).
- If neither exists, the rule file is skipped.

---

## Two-Pass Evaluation

Each (file, rule) pair is evaluated independently. The evaluation follows a two-pass protocol:

### Pass 1 — Stateless

- The LLM is given: the file path, an annotated unified diff, the full file content (line-numbered), and the rule (name, severity, prompt, examples, escalation guidance).
- The LLM has no access to any file system or external tools.
- The LLM MUST emit exactly one of: `pass`, `fail`, or `needs-more-context`.
- If the LLM returns `needs-more-context` for a rule with `context = "stateless"`, the verdict MUST be collapsed to `fail` (conservative default). The reasoning is annotated with `[collapsed from needs-more-context: stateless rule]`.
- If the LLM returns `needs-more-context` for a rule with `context = "agentic"`, escalation to Pass 2 is triggered — but only when `CheckInfra.agentic` is present. When no agentic evaluator is configured, the verdict is collapsed to `fail`.
- Any terminal verdict (`pass` or `fail`) from Pass 1 is final; Pass 2 is NOT triggered even for `context = "agentic"` rules.

### Pass 2 — Agentic Escalation

Triggered only when Pass 1 returns `needs-more-context` on an `agentic`-typed rule and `CheckInfra.agentic` is set.

- The agent session is given the same file block, rule, and any `context_hint` (list of suggested files to read and a question) produced by the stateless pass.
- The agent has access to file-reading tools: `read`, `grep`, `find`, `ls`. If `--allow-bash` is set, a `bash` tool is also available.
- The agent MUST reach a terminal verdict (`pass` or `fail`). The agentic task prompt MUST explicitly forbid emitting `needs-more-context`.
- If the agent's final message is not parseable JSON with a valid verdict, a normalization LLM call is made: the agent's raw output is sent to the same model (the agentic model) with a forced tool-use call to extract a structured verdict.
- If normalization also fails, the verdict falls back to `fail` with `confidence = 0.0`.
- If the agentic session times out, it MUST be aborted. The evaluator then attempts to parse whatever text was buffered before the timeout; if no valid verdict can be extracted (including after a normalization attempt), the verdict falls back to `fail`. The evaluator MUST NOT throw on timeout.
- Any `needs-more-context` emitted by the agentic pass despite the instruction is collapsed to `fail`.
- All verdicts produced by Pass 2 are tagged `from_agentic = true`.

### Prompt Construction

**File block** (shared between both passes):
- `FILE: <path>`
- `CHANGED LINES (unified diff with absolute new-file line numbers):` — present only when the diff is non-empty. The diff is annotated so that each line carries the absolute line number in the final file from the `+` hunk counter.
- `FULL FILE CONTENT (each line prefixed "N | "):` — present only when content is non-null and non-empty. Lines are numbered starting at 1.

**Size limits and skip behavior**: Files that exceed configured size limits are skipped entirely — no LLM call is made — and a warning is emitted to stderr for each applicable (rule, file) pair. Three limits apply independently; exceeding any one causes the file to be skipped for all its rules:

1. **Byte size limit** (default: 100,000 bytes, configurable with `--max-file-bytes`): Applied at file-collection time. Files whose UTF-8 byte length exceeds this threshold are flagged as oversized; no content or diff evaluation occurs.
2. **Diff character limit** (default: 8,000 characters, configurable with `--max-diff-chars`): Checked after file collection. Files whose diff string length exceeds this limit are skipped.
3. **Content character limit** (default: 20,000 characters, configurable with `--max-content-chars`): Checked after file collection. Non-null file content whose string length exceeds this limit causes the file to be skipped.

Warning format — one line per affected (rule, file) pair, written to stderr:

```
warning: (rule-id, file-path) - file skipped: byte size (N bytes) exceeds --max-file-bytes LIMIT
warning: (rule-id, file-path) - file skipped: diff length N chars exceeds --max-diff-chars LIMIT
warning: (rule-id, file-path) - file skipped: content length N chars exceeds --max-content-chars LIMIT
```

Each warning names the limit that was breached, the governing flag, and the actual measured size.

Files that pass all three limits are evaluated with their full diff and content — no truncation is applied.

**Rule section**: Rule name, severity, prompt instruction, escalation guidance (if non-empty), and examples (if any) with PASS/FAIL labels.

**Prompt caching**: When there are multiple LLM calls for a check run, the system prompt is marked for caching. When a file has more than one applicable rule, the file-context block is also marked for caching (shared across all rules for that file).

---

## Evaluator Protocol

The runner is decoupled from any specific LLM provider or agentic runtime via two protocol interfaces: `StatelessEvaluator` and `AgenticEvaluator`. `CheckInfra` is defined in terms of these protocols, not provider SDKs.

### `StatelessEvaluator`

Responsible for evaluating a single `(file, rule)` pair without filesystem access. Each pair MUST be evaluated in isolation — one evaluator call per rule.

**Why isolation is required:**
- **Correctness**: batching multiple rules into one LLM call creates silent failure modes — the model may omit verdicts for some rules, stop after partial coverage, or emit unrecognisable rule identifiers. Any such omission falls back to a conservative `fail`, making a model oversight indistinguishable from a genuine policy violation.
- **Focus**: a single-rule call gives the model its full context window and attention for one question. Multi-rule prompts increase complexity and risk cross-rule interference in the reasoning.

The `evaluate(file, rule) → RuleVerdict` signature encodes this guarantee by construction: the `rule_id` of the returned verdict is assigned from the `rule` argument by the implementation, not inferred from the model output.

**TypeScript interface:**
```typescript
interface StatelessEvaluator {
  evaluate(
    request: FileCheckRequest,
    rule: Rule,
    opts: StatelessEvalOpts,
  ): Promise<RuleVerdict>;
}
```

**Rust trait:**
```rust
#[async_trait]
pub trait StatelessEvaluator: Send + Sync {
    async fn evaluate(
        &self,
        file_path: &str,
        diff: &str,
        content: Option<&str>,
        rule: &Rule,
        is_new_file: bool,
        opts: &StatelessEvalOpts,
    ) -> Result<RuleVerdict, LlmError>;
}
```

#### `StatelessEvalOpts`

| Field | Type | Description |
|-------|------|-------------|
| `timeoutMs` / `timeout` | `number` / `Duration` | Per-call deadline. |
| `model` | `string` | Model identifier (e.g., `"claude-haiku-4-5"`). |
| `trace` | `boolean` | When `true`, the evaluator SHOULD log prompts and raw responses to stderr. |
| `hints` | `CacheHints` | Advisory signals from the runner about call volume; used to enable prompt caching. |

#### Cache Hints

`CacheHints` is a provider-neutral advisory type. It carries no Anthropic-specific fields.

```typescript
interface CacheHints {
  /** True when the same system prompt will be reused across multiple calls in this run. */
  sharedSystemPrompt: boolean;
  /** True when the same file context will be reused across multiple rule calls for this file. */
  sharedFileContext: boolean;
}
```

Evaluator implementations MAY use these hints to activate provider-specific prompt caching (e.g., Anthropic's `cache_control` blocks). The runner MUST NOT know or care whether caching is activated.

#### Contract

- The evaluator MUST return exactly one `RuleVerdict` per call. The `rule_id` of that verdict MUST equal `rule.id`. Implementations MUST assign `rule_id` from the `rule` argument — never from model output — in all code paths including the retry-exhausted fallback.
- The evaluator MAY return `verdict = "needs-more-context"`. When it does, it SHOULD populate `context_hint`. Note: the `submit_verdict` tool schema does not enforce this at the JSON Schema level — `context_hint` is not in the `required` array. Implementations that rely on the tool schema MUST add runtime validation. When `context_hint` is absent, the runner escalates with null hints (no `read_files`, no `question`).
- The evaluator MUST NOT access the filesystem or any external resource beyond its configured LLM provider.
- On a retryable failure (timeout, rate-limit, transient server error), the evaluator MUST retry internally before returning. After all retries are exhausted, it MUST return a fallback `fail` verdict with `confidence = 0.0` and `reasoning = "LLM call failed"`. It MUST NOT throw.
- On a non-retryable failure (authentication error, unexpected client error), the evaluator MUST throw. The runner is responsible for handling the exception.
- The evaluator MUST NOT mutate the `request` or `rule` arguments.

---

### `AgenticEvaluator`

Responsible for evaluating a single (file, rule) pair with filesystem read access. Invoked only on agentic escalation.

**TypeScript interface:**
```typescript
interface AgenticEvaluator {
  evaluate(
    request: FileCheckRequest,
    rule: Rule,
    hints: ContextHint[],
    opts: AgenticEvalOpts,
  ): Promise<RuleVerdict>;
}
```

**Rust trait:**
```rust
#[async_trait]
pub trait AgenticEvaluator: Send + Sync {
    async fn evaluate(
        &self,
        file_path: &str,
        diff: &str,
        content: Option<&str>,
        rule: &Rule,
        hints: &[ContextHint],
        opts: &AgenticEvalOpts,
    ) -> Result<RuleVerdict, LlmError>;
}
```

#### `AgenticEvalOpts`

| Field | Type | Description |
|-------|------|-------------|
| `timeoutMs` / `timeout` | `number` / `Duration` | Session deadline. |
| `model` | `string` | Model identifier for the agentic session (e.g., `"claude-sonnet-4-6"`). |
| `allowBash` / `allow_bash` | `boolean` | Whether a shell execution tool is permitted in the session. |
| `trace` | `boolean` | When `true`, the evaluator SHOULD log the task prompt and final agent output to stderr. |

#### Contract

- The evaluator MUST return exactly one `RuleVerdict` per call. The `rule_id` of that verdict MUST equal `rule.id`.
- The evaluator MUST NOT return `verdict = "needs-more-context"`. If the underlying agent emits it despite instruction, the evaluator MUST collapse it to `fail`.
- The evaluator MUST tag all returned verdicts with `from_agentic = true`.
- If the session times out, the evaluator MUST abort the session. After abort, the evaluator attempts to parse whatever text was buffered before the timeout; if no valid verdict can be extracted (including after a normalization attempt against the partial buffer), the evaluator MUST return a fallback `fail` verdict with `confidence = 0.0`. The evaluator MUST NOT throw on timeout.
- If the agent's final output is not directly parseable into a structured verdict, the evaluator SHOULD attempt a normalization pass (a secondary LLM call with forced tool-use that extracts a structured verdict from the raw agent output). The normalization pass uses the same model as the agentic session (`opts.model`). If normalization also fails, the evaluator MUST return a fallback `fail` verdict.
- The evaluator MUST NOT throw except on non-retryable, non-timeout failures (e.g., authentication errors).

---

### `PromptBuilder`

An optional extensibility point for evaluators that accept the standard text-based prompt format.

```typescript
interface PromptBuilder {
  buildFileContext(request: FileCheckRequest, rule: Rule): string;
  buildRuleSection(rule: Rule): string;
  buildAgenticTask(request: FileCheckRequest, rule: Rule, hints: ContextHint[]): string;
}
```

#### Contract

- `buildFileContext` MUST produce a string containing the file path, the annotated unified diff (when non-empty), and the numbered full file content (when non-null and non-empty). This is the cacheable prefix — it MUST be identical for the same file regardless of which rule is being evaluated.
- `buildRuleSection` MUST produce a string containing the rule name, severity, prompt instruction, escalation guidance (when non-empty), and examples (when present). This is the non-cacheable suffix.
- `buildAgenticTask` MUST produce a complete, self-contained task string combining the file block, rule section, any context hints, and instructions to reach a terminal verdict.
- Implementations MUST NOT include `needs-more-context` as an acceptable verdict in the agentic task prompt.
- `PromptBuilder` is NOT a required field of any public interface. Evaluator implementations that do not use a text-based prompt format MAY ignore it entirely.

---

### `CheckInfra`

The public infrastructure type. Defined in terms of evaluator protocols, not provider SDKs.

```typescript
interface CheckInfra {
  /** Evaluator for the stateless pass. */
  stateless: StatelessEvaluator;
  /**
   * Evaluator for the agentic escalation pass.
   * When absent, any needs-more-context on an agentic rule is collapsed to fail.
   */
  agentic?: AgenticEvaluator;
  /** Cache implementation (CacheManager or NullCache). */
  cache: CacheInterface;
  /** Optional progress reporter for TTY updates. */
  progress?: ProgressReporter;
}
```

The runner MUST NOT inspect or depend on any field of `CheckInfra` beyond `stateless`, `agentic`, `cache`, and `progress`.

The routing logic:

```
stateless.evaluate(request, rule, opts)
  → "pass" or "fail"          → terminal; stored as-is
  → "needs-more-context"
      + rule.context = "stateless"  → collapse to "fail"
      + rule.context = "agentic"
          + infra.agentic present   → agentic.evaluate(request, rule, hints, opts)
          + infra.agentic absent    → collapse to "fail"
```

---

### Reference Implementations

Two concrete implementations MUST be shipped with each implementation of this package as defaults.

#### `AnthropicStatelessEvaluator`

Satisfies `StatelessEvaluator` using the Anthropic Messages API with forced tool-use (`submit_verdict` tool).

Implementation responsibilities:
- Builds the prompt using the standard `PromptBuilder` output.
- Forces verdict extraction via `tool_choice: { type: "tool", name: "submit_verdict" }`.
- Activates `cache_control: { type: "ephemeral" }` on the system prompt and/or file context block when `CacheHints` signals shared reuse.
- Retries on timeout, HTTP 429, and HTTP 5xx with exponential backoff (3 attempts total, base delay 1 s).

#### `PiAgenticEvaluator`

Satisfies `AgenticEvaluator` using pi-coding-agent sessions.

Implementation responsibilities:
- Builds the task using the standard `PromptBuilder` output.
- Constructs a pi session with `read`, `grep`, `find`, `ls` tools (plus `bash` when `allowBash` is set).
- Subscribes to session output events to collect the agent's text output.
- On session completion, attempts to parse a terminal verdict from the final accumulated text.
- If parsing fails, issues a normalization call (single Messages API call with forced tool-use) against the same model (`opts.model`).
- On timeout, aborts the session before returning the fallback verdict.

### API Key Ownership

- The runner and `CheckInfra` are **never** responsible for reading API keys.
- The **caller** (CLI layer or programmatic user) reads the key from the environment and passes it to the evaluator constructor.
- Evaluator constructors MUST accept the key as an explicit parameter. They MUST NOT fall back to reading `process.env` silently.
- The CLI layer remains responsible for reading `ANTHROPIC_API_KEY` from the environment and exiting 3 when it is absent.

---

## Verdict Model

### LLM Verdicts (internal)

| Verdict | Meaning |
|---------|---------|
| `pass` | Rule is satisfied. No violation found. |
| `fail` | Rule is violated. |
| `needs-more-context` | The LLM cannot determine compliance without reading other files. This is an internal routing signal only; it is never stored in final results. |

### Display Verdicts (external)

The display verdict is computed from the LLM verdict and rule severity:

| LLM verdict | Rule severity | Display verdict | Blocks merge? |
|-------------|---------------|-----------------|---------------|
| `pass` | any | `pass` | No |
| `fail` | `warn` | `warn` | No (unless `--warn-as-error`) |
| `fail` | `error` | `error` | Yes (exit 2) |

### Aggregation

- **File-level**: `error` if any rule verdict is `fail`+`error`; else `warn` if any is `fail`+`warn`; else `pass`.
- **Report-level**: `error` if any file is `error`; else `warn` if any file is `warn`; else `pass`.

### Verdict Fields

Each rule verdict carries:

| Field | Type | Description |
|-------|------|-------------|
| `rule_id` | string | ID of the rule that was evaluated. |
| `verdict` | `pass`\|`fail` | Raw LLM verdict (in stored results, always `pass` or `fail`; `needs-more-context` is an internal routing signal that is collapsed or escalated before storage). |
| `rule_severity` | `warn`\|`error` | Severity copied from the rule at evaluation time. |
| `confidence` | float [0, 1] | LLM-reported certainty. At runtime, defaults to `0.5` if the LLM response lacks a numeric value; `0.0` for fallback (post-retry-exhaustion) verdicts. Schema deserialization default (e.g., old cache entries lacking the field): `1.0`. |
| `reasoning` | string | 1–3 sentence explanation, newlines normalized to spaces. |
| `line_refs` | integer[] | Absolute line numbers in the final file where violations occur. Empty for pass. |
| `context_hint` | object\|null | Files suggested and question asked when `needs-more-context` was emitted. |
| `from_agentic` | boolean | `true` if the verdict came from the agentic pass. |

---

## Caching

### Cache Key

The cache key is the SHA-256 hex digest of the following fields, joined by newlines:

```
version:<CACHE_VERSION>
model:<model_name>
rule:<id>:<severity>:<trimmed_prompt>   (one line per rule, sorted by rule id ascending)
path:<file_path>
content:<file_content or empty string>
diff:<diff_string>
```

The cache key is deterministic and order-independent with respect to the rule list.

Changing the model, any rule's ID/severity/prompt (trimmed), the file path, the file content, or the diff produces a different cache key.

### Storage

Cache entries are stored as individual flat JSON files, one per cache key, named `<sha256_hex>.json`. The default cache directory is `.agent-rules-cache/` relative to the repository root. The cache directory is created automatically if it does not exist.

Each entry records: cache key, file path, rule IDs, model, creation timestamp (Unix seconds), hit count, and the full `FileVerdict` object.

Cache entries are keyed and stored at the **(file, rule)** granularity. Each file is decomposed into one evaluation task per rule; each task produces its own cache entry.

### Cache Hit Behavior

On a cache hit, the stored `FileVerdict` is returned immediately with `cached = true`, skipping all LLM calls for that (file, rule) combination. Each cache read increments the stored `hit_count`.

### File-Level `cached` Flag

A `FileVerdict` is marked `cached = true` at the file level only if ALL per-rule tasks for that file were served from cache. A single cache miss on any rule causes `cached = false` for the entire file.

### Cache Invalidation

There is no automatic cache eviction. Cache entries persist until `cache clear` is run. Cache entries with a different `CACHE_VERSION` produce a different key and are never hit (stale entries are orphaned but not automatically deleted).

### NullCache

When `--no-cache` is specified, a no-op cache is used: `get` always returns null, `put` is a no-op, `clear` returns 0.

---

## Concurrency Model

### Per-check parallelism

`checkPr` processes all (file, rule) tasks concurrently with two separate semaphores:

- **Stateless semaphore**: limits concurrent stateless LLM calls. Default: 10. Configurable with `--max-concurrent`.
- **Agentic semaphore**: limits concurrent agentic escalations. Default: 2. Configurable with `--agentic-concurrency`.

The stateless semaphore slot is released as soon as the stateless pass completes, before any agentic escalation begins. This allows other stateless calls to proceed while an agentic session runs.

### Fan-out decomposition

A file with N applicable rules is decomposed into N independent (file, rule) tasks, each issued as a separate LLM call. Results are merged back into a single `FileVerdict` per file after all tasks complete.

### Retry logic (stateless pass)

On a retryable error (timeout, HTTP 429, or HTTP 5xx), the stateless pass retries up to 3 times total (2 retries after the first attempt). The delay between attempts follows exponential backoff: 1s, 2s (base × 2^attempt).

Non-retryable errors (any other HTTP error including 4xx auth/client errors) are re-thrown immediately.

After all retries are exhausted, a fallback `fail` verdict is returned with `confidence = 0.0` and `reasoning = "LLM call failed"`.

---

## Output Formats

### Text (default)

Human-readable output, two modes:

- **Concise** (default): one line per violation, format: `file[:line]: warning[rule-id] [agentic] reasoning (N%)`. Pass verdicts are silent. The `:line` segment is present only when the verdict includes at least one line reference. The `[agentic]` badge is present only when `from_agentic = true`.
- **Verbose** (`--verbose` or `--trace`): annotate-snippets style with `-->` location pointer, source context lines, and caret underlines at violation sites. Source is read from the head git ref when available, otherwise from disk. An `[agentic]` badge is appended to the rule code when `from_agentic = true`.

Violations are sorted by file path (lexicographic), then by first line reference ascending.

A summary line is always printed: file count, issue count (with error/warn breakdown), model name, and duration. The error/warn counts in the text summary are per-rule-violation counts, not per-file aggregated counts. The `(N cached)` suffix is only appended to the summary when `cache_hits > 0`.

### JSON (`--output json`)

The full `PRReport` object as pretty-printed JSON, with an additional top-level `overall_verdict` field (`"pass"`, `"warn"`, or `"error"`).

### GitHub Comment (`--output github`)

Markdown formatted for GitHub PR comments:
- `## <icon> agent-rules — <VERDICT>` header
- `PR: <url-or-ref-range>` label line: the PR URL when `--pr` is provided, or `` `<base_ref>..<head_ref>` `` when derived from git refs
- Summary line: pass/warn/error file counts, cache hits (counts reflect file-level aggregated verdicts, not individual rule violations)
- Per-file `<details>` blocks with a verdict table (rule ID, display verdict+icon, confidence %, reasoning)
- Pipe characters in reasoning are escaped as `\|`

### GitHub Step Summary

When `GITHUB_STEP_SUMMARY` environment variable is set (GitHub Actions), a summary is appended to that file. Format: `## <icon> agent-rules — <VERDICT>` header; `**PR:** <url-or-ref-range>` label line; stats table (pass/warn/error/cached/duration); issues table (file, rule, verdict, truncated reasoning ≤120 chars) — omitted when there are no violations.

### GitHub Workflow Annotations

When running inside GitHub Actions (`GITHUB_ACTIONS` env var set), inline code annotations are emitted to stdout using the `::error` / `::warning` workflow command format for each non-pass verdict, referencing the file and, when line references are present, the first and last line numbers.

---

## CLI Contract

### `check` command

```
agent-rules check [options]
```

Evaluates changed files against applicable rules.

**File selection** (mutually exclusive):
- `--files <paths...>`: check the specified files explicitly (no diff; content read from disk).
- `--base <ref>` / `--head <ref>`: compute changed files from a git diff between two refs. Default: `main` / `HEAD`.

**Filtering**:
- `--dir-filter <dirs...>`: restrict checked files to those under the specified directories. Multiple values are additive (OR semantics). Comma-separated values within a single argument are also accepted.

**Options**:

| Flag | Default | Description |
|------|---------|-------------|
| `--base <ref>` | `main` | Base git ref for diff. |
| `--head <ref>` | `HEAD` | Head git ref. |
| `--pr <url>` | — | GitHub PR URL. Used for report labels and `--post-comment`. Does NOT affect which files are checked. |
| `--files <paths...>` | — | Check specific files instead of git diff. |
| `--repo <path>` | CWD | Repository root. |
| `--dir-filter <dirs...>` | — | Restrict to files under these directories. |
| `--output <format>` | `text` | Output format: `text`, `json`, `github`. |
| `--warn-as-error` | `false` | Exit 1 when any warn-severity violation is found. Does not override exit code 2 when error-severity violations are also present; exit 2 takes precedence. |
| `--no-cache` | — | Disable cache. |
| `--model <name>` | `claude-haiku-4-5` | Model for stateless pass. |
| `--max-concurrent <n>` | `10` | Max parallel stateless LLM calls. |
| `--max-file-bytes <n>` | `100000` | Skip files whose UTF-8 byte size exceeds this limit. |
| `--max-diff-chars <n>` | `8000` | Skip files whose diff string length exceeds this limit. |
| `--max-content-chars <n>` | `20000` | Skip files whose content string length exceeds this limit. |
| `--timeout <ms>` | `60000` | Timeout for each stateless LLM call. |
| `--agentic-model <model>` | `claude-sonnet-4-6` | Model for agentic escalation. |
| `--agentic-timeout <ms>` | `180000` | Timeout for agentic sessions. |
| `--agentic-concurrency <n>` | `2` | Max parallel agentic escalations. |
| `--allow-bash` | `false` | Allow bash tool in agentic sessions. |
| `--post-comment` | `false` | Post results as a GitHub PR comment (requires `--pr` and `GITHUB_TOKEN`). |
| `--strict-rules` | `false` | Abort if any `.agent-rules.toml` on disk differs from the head ref. |
| `--verbose` | `false` | Show full annotate-snippets diagnostic output. |
| `--trace` | `false` | Print raw prompts and LLM responses to stderr. Implies `--verbose`. |

**File filtering before rule evaluation**:
- In git-diff mode, binary files are detected both by file extension (e.g., `.png`, `.pdf`) and by the presence of `Binary files` in the diff output, and MUST be skipped. In `--files` mode, no binary detection is performed; all explicitly-listed files are treated as text.
- Files that exceed any configured size limit (`--max-file-bytes`, `--max-diff-chars`, `--max-content-chars`) MUST be skipped entirely. A warning MUST be emitted to stderr for each (rule, file) pair that is skipped, naming the limit breached, the governing flag, and the actual size. No LLM call is made for oversized files.
- Files with no applicable rules MUST be skipped (no LLM call made).

**Exit codes**:

| Code | Condition |
|------|-----------|
| 0 | All checks pass (or all are warn-only without `--warn-as-error`) |
| 1 | Warn-severity violation found AND `--warn-as-error` is set |
| 2 | Error-severity violation found |
| 3 | Configuration/invocation error (missing API key, git error, bad flag, etc.) |

### `rules list` command

```
agent-rules rules list --path <file> [--repo <path>]
```

Resolves and prints the rules that would apply to the given file path. No LLM calls are made.

### `rules validate` command

```
agent-rules rules validate [--repo <path>]
```

Walks the repository tree for all `.agent-rules.toml` files and validates each. Skips `.git`, `node_modules`, `.next`, `dist`, `__pycache__`, `.cache`. Performs two levels of validation:

1. **Per-file**: parse errors, schema errors, within-file duplicate IDs.
2. **Cross-file**: the same rule ID MUST NOT appear in two files that are not in an ancestor-descendant directory relationship. Any such conflict MUST be reported.

Exits 1 if any file fails to parse or validate, or if any cross-file ID conflict is found.

### `cache stats` command

```
agent-rules cache stats [--repo <path>]
```

Prints cache statistics: total entries, total hits, oldest entry age, cache directory path.

### `cache clear` command

```
agent-rules cache clear [--repo <path>] [-y]
```

Deletes all cache entries. Prompts for confirmation unless `-y` is passed.

### Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `ANTHROPIC_API_KEY` | Yes (for `check`) | API key for the LLM provider. |
| `GITHUB_TOKEN` | Yes (for `--post-comment`) | GitHub token with PR write access. |
| `GITHUB_STEP_SUMMARY` | CI only | File path for GitHub Actions step summary. Written automatically when set. |
| `GITHUB_ACTIONS` | CI only | Detected automatically. Enables workflow annotation output and CI progress mode. |

### GitHub PR Comment Upsert

When `--post-comment` is used, the comment body MUST include a sentinel marker (`<!-- agent-rules-report -->`). On each run, the system searches existing PR comments for one containing this sentinel and updates it. If none is found, a new comment is created.

---

## Behavioral Invariants

1. **A `pass` verdict never blocks merge**, regardless of rule severity or `--warn-as-error`.
2. **An `error`-severity `fail` always exits 2**, regardless of `--warn-as-error`.
3. **`needs-more-context` is never surfaced in output as a final verdict**. It is always collapsed to `fail` before the result is returned.
4. **Rules with `enabled = false` are never evaluated**.
5. **Binary files are never evaluated** (in git-diff mode; see CLI Contract for `--files` mode behavior).
6. **A file with no applicable rules generates no LLM calls and no verdicts**.
7. **Each `(file, rule)` tuple MUST be evaluated in its own isolated LLM call.** One rule's evaluation does not see another rule's verdict.
8. **A terminal stateless verdict (`pass` or `fail`) is never re-evaluated in the agentic pass**, even if the rule has `context = "agentic"`.
9. **The cache key is deterministic**: identical inputs always produce the same cache key, regardless of rule order in the input list.
10. **LLM call failures fall back to `fail`**, not `pass`. The system fails conservatively.
11. **The stateless pass is retried on timeout, 429, and 5xx errors** (up to 3 total attempts, exponential backoff). Client errors (4xx other than 429) are not retried.
12. **Agentic session timeout causes abort**. If no valid verdict was accumulated before timeout (including after a normalization attempt), the verdict falls back to `fail`.
13. **Files exceeding any configured size limit are skipped entirely** (`--max-file-bytes`, `--max-diff-chars`, `--max-content-chars`). A warning is emitted to stderr for each skipped (rule, file) pair. No LLM call is made and no verdict is produced.
14. **No truncation is applied** to diff or content. Files within limits are evaluated with their full diff and content.
15. **The byte-size limit** (`--max-file-bytes`) is evaluated against UTF-8 byte length and is checked at file-collection time, before rule resolution.
16. **PR report stats (`pass_count`, `warn_count`, `error_count`) count files, not rule verdicts**. The counts reflect file-level aggregated verdicts.
17. **When `CheckInfra.agentic` is absent**, `needs-more-context` on any `context = "agentic"` rule is collapsed to `fail`, as if the rule were stateless.
18. **The normalization pass uses the agentic model** (`opts.model`), not the stateless model.

---

## TODO / DECISION-NEEDED

### 1. `scope` field silently ignored

**Observation**: Test fixtures and some TOML examples include `scope = "file"` or `scope = "repo"` on rules. The schema does not define a `scope` field; it is stripped by Zod without error.

**Alternatives**:
- (A) Formally remove `scope` — it is dead weight; document that it is ignored.
- (B) Implement `scope = "repo"` as a future feature where rules run once per repo, not per file.
- (C) Emit a deprecation warning when `scope` is present.

**Decision needed**: Is `scope` intentionally reserved for a future feature, or should it be removed entirely?

---

### 2. `files_with_rules` stat always equals `total_files`

**Observation**: `stats.files_with_rules` counts how many file requests have at least one rule (`rules.length > 0`). However, files with no applicable rules are filtered out *before* building the request list — so every file in `requests` already has `rules.length > 0`. This makes `files_with_rules` always equal to `total_files` in the output.

**Alternatives**:
- (A) Remove `files_with_rules` from the report.
- (B) Track "total files scanned before rule filtering" separately and use that as `total_files`, with `files_with_rules` being the post-filter count.
- (C) Keep as-is and document it as redundant.

**Decision needed**: What should `total_files` and `files_with_rules` represent?

---

### 3. `needs-more-context` semantics on `context = "stateless"` rules with `needs_more_context_when`

**Observation**: The `needs_more_context_when` field is included in the rule section sent to the LLM regardless of the rule's `context` value. On a stateless rule, the system prompt instructs the model to use `needs-more-context` sparingly, but nothing prevents the LLM from emitting it on a stateless rule that has `needs_more_context_when` set.

**Current behavior**: `needs-more-context` on a stateless rule collapses to `fail`, regardless of `needs_more_context_when`.

**Decision needed**: Should `needs_more_context_when` be stripped from the prompt when `context = "stateless"` to avoid confusing the LLM? Or is the current "just collapse it" behavior the intended contract?

---

### 4. Behavior of `--files` with deleted or renamed files

**Observation**: When `--files` is used, the system reads each file from disk directly (no git diff). There is no `diff` field (it is set to `""`), no `is_deleted`/`is_new` metadata, and content may be `null` if the file is missing or too large. For git-diff mode, deleted files have `content = null` and are sent to rule evaluation.

**Decision needed**: Should deleted files (with `is_deleted = true`) be skipped entirely, or should they be evaluated with only the diff (no content)? The current code evaluates them with `content = null`.

---

## Verifiable Conditions

The following conditions are directly falsifiable from the implementation:

1. A rule file with two rules sharing the same `id` MUST throw an error containing "Duplicate rule id".
2. A rule file with a missing `id` field MUST throw a schema validation error.
3. A child rule file with `inherit_mode = "replace"` results in exactly the child's rules; no parent rules are present.
4. A child rule file with `inherit_mode = "merge"` and a rule ID matching a parent rule results in the child version overriding the parent.
5. A rule with `enabled = false` is never returned by rule resolution.
6. A rule with `glob-include = ["**/*.ts"]` does not apply to `.py` files.
7. The cache key for two requests with rules `[A, B]` and `[B, A]` (same rules, different order) MUST be identical.
8. A stateless LLM `needs-more-context` on a `context = "stateless"` rule MUST produce `verdict = "fail"` with reasoning containing "collapsed from needs-more-context".
9. A stateless LLM `needs-more-context` on a `context = "agentic"` rule, when `CheckInfra.agentic` is present, MUST trigger the agentic pass.
10. A stateless LLM `needs-more-context` on a `context = "agentic"` rule, when `CheckInfra.agentic` is absent, MUST produce `verdict = "fail"` with no call to any agentic evaluator.
11. If all (file, rule) pairs hit the cache, no LLM calls are made and `file.cached = true` for all files.
12. If any (file, rule) pair misses the cache, `file.cached = false` for that file.
13. The exit code is 2 when any file has a `fail` + `error`-severity verdict.
14. The exit code is 1 (not 2) when the worst verdict is `fail` + `warn` and `--warn-as-error` is set.
15. The exit code is 0 when the worst verdict is `fail` + `warn` and `--warn-as-error` is not set.
16. `--post-comment` without `--pr` MUST exit 3.
17. Missing `ANTHROPIC_API_KEY` MUST exit 3 before making any LLM calls.
18. After exhausting all retries on 429, a `fail` verdict with `confidence = 0.0` and `reasoning = "LLM call failed"` MUST be returned (not an exception).
19. A 403 API error MUST propagate as an exception (not retried, not converted to a fallback verdict). A stateless-pass timeout MUST be retried (not re-thrown).
20. Two unrelated rule files (neither is an ancestor of the other in the directory hierarchy) that both define a rule with the same `id` MUST be reported as a conflict by `rules validate`.
21. A file whose byte size exceeds `--max-file-bytes` MUST be skipped. The warning MUST include the actual byte count, the flag name, and the configured limit.
22. A file whose diff length exceeds `--max-diff-chars` MUST be skipped. The warning MUST include the actual char count. Same pattern for `--max-content-chars`.
23. A file within all three limits MUST be evaluated with its full, untruncated diff and content.
24. `checkFile` called with a `CheckInfra` whose `stateless` field is a stub (no SDK dependency) MUST compile and run without errors.
25. A `StatelessEvaluator` stub that always returns `verdict = "pass"` MUST result in all `FileVerdict` objects having `verdict = "pass"` when passed to `checkPr`.
26. A `StatelessEvaluator` that throws a non-retryable error MUST propagate that exception out of `checkFile`.
27. A `StatelessEvaluator` that exhausts retries MUST return a `fail` verdict with `confidence = 0.0` — `checkFile` MUST NOT throw.
28. `AgenticEvaluator.evaluate` MUST NOT be called when the stateless verdict is `pass` or `fail`, even when `rule.context = "agentic"`.
29. All verdicts returned by `AgenticEvaluator.evaluate` MUST have `from_agentic = true` in the stored result.
30. An `AgenticEvaluator` that returns `verdict = "needs-more-context"` MUST have that verdict collapsed to `fail` by the evaluator before it reaches the runner.
31. Two different `StatelessEvaluator` implementations passed to `checkPr` with identical inputs MUST produce cache entries under the same key (the cache key is evaluator-agnostic).
