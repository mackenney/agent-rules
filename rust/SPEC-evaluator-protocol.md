# Evaluator Protocol Specification

> The key words **MUST**, **MUST NOT**, **SHOULD**, **SHOULD NOT**, and **MAY** are used in this document per [RFC 2119](https://datatracker.ietf.org/doc/html/rfc2119).
>
> **Status**: Canonical specification for the Rust implementation.

---

## Motivation

The current implementation fuses three concerns into a single module (`llm.ts`):

1. **Provider binding** — the Anthropic SDK client is a required field of `CheckInfra`, the public infrastructure type, coupling every caller of `checkFile` / `checkPr` to a specific SDK.
2. **Stateless evaluation** — prompt construction, tool-call marshaling, retry/backoff, and verdict extraction are all Anthropic-specific and co-located.
3. **Agentic evaluation** — the pi-coding-agent session runtime, its auth model, and the normalization fallback are hardwired.

This makes it impossible to:
- Unit-test the orchestration layer (`runner.ts`) without instantiating a real or mocked Anthropic client.
- Replace either evaluation pass with a different model provider or agentic runtime.
- Extend the package (e.g., add an OpenAI stateless evaluator) without forking internal modules.

This proposal extracts a **provider-neutral protocol** for both passes and redefines `CheckInfra` in terms of that protocol. The existing Anthropic and pi implementations become concrete classes that satisfy the protocol.

---

## Evaluator Protocols

### `StatelessEvaluator`

Responsible for evaluating a single `(file, rule)` pair without filesystem access. Each pair MUST be evaluated in isolation — one evaluator call per rule.

**Why isolation is required:**
- **Correctness**: batching multiple rules into one LLM call creates silent failure modes — the model may omit verdicts for some rules, stop after partial coverage, or emit unrecognisable rule identifiers. Any such omission falls back to a conservative `fail`, making a model oversight indistinguishable from a genuine policy violation.
- **Focus**: a single-rule call gives the model its full context window and attention for one question. Multi-rule prompts increase complexity and risk cross-rule interference in the reasoning.

The `evaluate(request, rule) → RuleVerdict` signature encodes this guarantee by construction: the `rule_id` of the returned verdict is assigned from the `rule` argument by the implementation, not inferred from the model output.

```typescript
interface StatelessEvaluator {
  evaluate(
    request: FileCheckRequest,
    rule: Rule,
    opts: StatelessEvalOpts,
  ): Promise<RuleVerdict[]>;
}
```

#### `StatelessEvalOpts`

| Field | Type | Description |
|-------|------|-------------|
| `timeoutMs` | `number` | Per-call deadline in milliseconds. |
| `trace` | `boolean` | When `true`, the evaluator SHOULD log prompts and raw responses to stderr. |
| `hints` | `CacheHints` | Advisory signals from the runner about call volume; used to enable prompt caching (see [Cache Hints](#cache-hints)). |

#### Contract

- The evaluator MUST return exactly one `RuleVerdict` per call. The `rule_id` of that verdict MUST equal `rule.id`. Implementations MUST use the explicit `rule` parameter (not iterate `request.rules`) in all code paths, including the retry-exhausted fallback, to enforce this guarantee by construction.
- The evaluator MAY return `verdict = "needs-more-context"`. When it does, it SHOULD populate `context_hint`. Note: the `submit_verdict` tool schema does not enforce this at the JSON Schema level — `context_hint` is not in the `required` array. Implementations that rely on the tool schema MUST add runtime validation or an `if/then` conditional constraint. When `context_hint` is absent, the runner escalates with null hints (no `read_files`, no `question`).
- The evaluator MUST NOT access the filesystem or any external resource beyond its configured LLM provider.
- On a retryable failure (timeout, rate-limit, transient server error), the evaluator MUST retry internally before returning. After all retries are exhausted, it MUST return a fallback `fail` verdict with `confidence = 0.0` and `reasoning = "LLM call failed"`. It MUST NOT throw.
- On a non-retryable failure (authentication error, unexpected client error), the evaluator MUST throw. The runner is responsible for handling the exception.
- The evaluator MUST NOT mutate the `request` or `rule` arguments.

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

---

### `AgenticEvaluator`

Responsible for evaluating a single (file, rule) pair with filesystem read access. Invoked only on agentic escalation.

```typescript
interface AgenticEvaluator {
  evaluate(
    request: FileCheckRequest,
    rule: Rule,
    hints: ContextHint[],
    opts: AgenticEvalOpts,
  ): Promise<RuleVerdict[]>;
}
```

#### `AgenticEvalOpts`

| Field | Type | Description |
|-------|------|-------------|
| `timeoutMs` | `number` | Session deadline in milliseconds. |
| `allowBash` | `boolean` | Whether a shell execution tool is permitted in the session. |
| `trace` | `boolean` | When `true`, the evaluator SHOULD log the task prompt and final agent output to stderr. (Currently a separate function parameter in `runAgenticEscalation`; moved into opts by this proposal.) |
| `model` | `string` | Model identifier for the agentic session (e.g., `"claude-sonnet-4-6"`). Corresponds to `AgenticOpts.model` in the current implementation. |

#### Contract

- The evaluator MUST return exactly one `RuleVerdict` per call. The `rule_id` of that verdict MUST equal `rule.id`.
- The evaluator MUST NOT return `verdict = "needs-more-context"`. If the underlying agent emits it despite instruction, the evaluator MUST collapse it to `fail`.
- The evaluator MUST tag all returned verdicts with `from_agentic = true`.
- If the session times out, the evaluator MUST abort the session (call `session.abort()`). After abort, the evaluator falls through to the normal verdict-parsing path on whatever text was buffered before the timeout. If the buffer produces no parseable verdict (`parseVerdicts` returns a `"JSON parse error"` fallback), the normalization pass MAY still fire against the partial buffer. If normalization also fails, the evaluator MUST return a fallback `fail` verdict with `confidence = 0.0`. The evaluator MUST NOT throw on timeout.
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

## `CheckInfra` Redefinition

The public infrastructure type is redefined to depend on evaluator protocols, not provider SDKs.

```typescript
interface CheckInfra {
  /** Evaluator for the stateless pass. */
  stateless: StatelessEvaluator;
  /**
   * Evaluator for the agentic escalation pass.
   * When absent, any rule that would escalate has its needs-more-context verdict
   * collapsed to fail, as if context = "stateless".
   */
  agentic?: AgenticEvaluator;
  /** Cache implementation (CacheManager or NullCache). */
  cache: CacheInterface;
  /** Optional progress reporter for TTY updates. */
  progress?: ProgressReporter;
}
```

The `Anthropic` SDK client is no longer a field of `CheckInfra`. It becomes an internal detail of `AnthropicStatelessEvaluator`.

---

## Reference Implementations

Two concrete implementations MUST be shipped with the package as defaults.

### `AnthropicStatelessEvaluator`

Satisfies `StatelessEvaluator` using the Anthropic Messages API with forced tool-use (`submit_verdict` tool).

Construction:

```typescript
new AnthropicStatelessEvaluator({
  client: Anthropic,   // Anthropic SDK client instance
  model: string,       // Model name, e.g. "claude-haiku-4-5"
})
```

Implementation responsibilities (not part of the public protocol):
- Builds the prompt using the standard `PromptBuilder` output.
- Forces verdict extraction via `tool_choice: { type: "tool", name: "submit_verdict" }`.
- Activates `cache_control: { type: "ephemeral" }` on the system prompt and/or file context block when `CacheHints` signals shared reuse.
- Retries on timeout, HTTP 429, and HTTP 5xx with exponential backoff (3 attempts total, base delay 1 s).

### `PiAgenticEvaluator`

Satisfies `AgenticEvaluator` using pi-coding-agent sessions.

Construction:

```typescript
new PiAgenticEvaluator({
  model: string,       // Model name, e.g. "claude-sonnet-4-6"
  apiKey: string,      // Provider API key (not read from process.env by the protocol layer)
})
```

Implementation responsibilities (not part of the public protocol):
- Builds the task using the standard `PromptBuilder` output.
- Constructs a pi session with `read`, `grep`, `find`, `ls` tools (plus `bash` when `allowBash` is set).
- Subscribes to `message_update` events to collect the agent's text output.
- On session completion, attempts to parse a terminal verdict from the final accumulated text.
- If parsing fails, issues a normalization call (single Messages API call with forced tool-use) against the same model.
- On timeout, calls `session.abort()` before returning the fallback verdict.

---

## API Key Ownership

Under the current design, `ANTHROPIC_API_KEY` is read from `process.env` inside `llm.ts`. This couples the evaluator to the process environment.

Under this proposal:
- The runner and `CheckInfra` are **never** responsible for reading API keys.
- The **caller** (CLI layer or programmatic user) reads the key from the environment and passes it to the evaluator constructor.
- Evaluator constructors MUST accept the key as an explicit parameter. They MUST NOT fall back to reading `process.env` silently.
- The CLI layer remains responsible for reading `ANTHROPIC_API_KEY` from the environment and exiting 3 when it is absent.

This makes the evaluators usable in non-CLI contexts (tests, programmatic use) without polluting `process.env`.

---

## Interaction with the Two-Pass Protocol

The two-pass orchestration in `runner.ts` is unchanged in behavior. The runner calls evaluators through the protocol; it has no knowledge of provider specifics.

The routing logic remains:

```
stateless.evaluate(request, rule, opts)
  → "pass" or "fail"          → terminal; stored as-is
  → "needs-more-context"
      + rule.context = "stateless"  → collapse to "fail"
      + rule.context = "agentic"
          + infra.agentic present   → agentic.evaluate(request, rule, hints, opts)
          + infra.agentic absent    → collapse to "fail"  ← NEW: this branch does not exist in the
                                                              current runner; it must be added in migration step 4
```

The runner MUST NOT inspect or depend on any field of `CheckInfra` beyond `stateless`, `agentic`, `cache`, and `progress`.

---

## Migration Path

This is a breaking change to the `CheckInfra` public type. Migration requires:

1. Introduce `StatelessEvaluator`, `AgenticEvaluator`, `PromptBuilder`, `StatelessEvalOpts`, `AgenticEvalOpts`, and `CacheHints` as exported types. Note: `StatelessEvalOpts` and `CacheHints` are new; `AgenticEvalOpts` consolidates the current `AgenticOpts` fields and adds `model` and `trace`.
2. Implement `AnthropicStatelessEvaluator` and `PiAgenticEvaluator`.
3. Redefine `CheckInfra` with the new fields.
4. Update `checkFile` and `checkPr` to call through the protocol instead of calling `runStateless` / `runAgenticEscalation` directly. Add the `infra.agentic` absent-guard in the `needs-more-context` / `context = "agentic"` routing branch (this is new runtime behavior).
5. Update the CLI layer to construct the concrete evaluators and inject them.
6. Remove `runStateless`, `runAgenticEscalation`, and the `Anthropic` import from `runner.ts`.
7. `llm.ts` becomes the implementation file for `AnthropicStatelessEvaluator` and `PiAgenticEvaluator`; it is no longer imported by `runner.ts`.

Callers that currently pass `{ client: new Anthropic(), cache, progress }` to `CheckInfra` MUST migrate to:

```typescript
const evaluator = new AnthropicStatelessEvaluator({ client: new Anthropic({ apiKey }), model });
const agenticEvaluator = new PiAgenticEvaluator({ model: agenticModel, apiKey });
checkPr(files, { stateless: evaluator, agentic: agenticEvaluator, cache, progress }, config, meta);
```

---

## Verifiable Conditions

The following conditions are directly falsifiable once this protocol is adopted:

1. `checkFile` called with a `CheckInfra` that has no `Anthropic` import in scope MUST compile without errors.
2. A `StatelessEvaluator` stub that always returns `verdict = "pass"` MUST result in all `FileVerdict` objects having `verdict = "pass"` when passed to `checkPr`.
3. A `StatelessEvaluator` that returns `verdict = "needs-more-context"` for a `context = "stateless"` rule MUST produce `verdict = "fail"` in the stored result, with no call to `AgenticEvaluator.evaluate`.
4. A `StatelessEvaluator` that returns `verdict = "needs-more-context"` for a `context = "agentic"` rule, when `infra.agentic` is absent, MUST produce `verdict = "fail"` in the stored result. **This is new behavior introduced by the protocol** — the current runner unconditionally escalates agentic-context rules. This branch must be added during migration.
5. A `StatelessEvaluator` that throws a non-retryable error MUST propagate that exception out of `checkFile`.
6. A `StatelessEvaluator` that exhausts retries MUST return a `fail` verdict with `confidence = 0.0` — `checkFile` MUST NOT throw.
7. `AgenticEvaluator.evaluate` MUST NOT be called when the stateless verdict is `pass` or `fail`, even when `rule.context = "agentic"`.
8. All verdicts returned by `AgenticEvaluator.evaluate` MUST have `from_agentic = true` in the stored result.
9. A `AgenticEvaluator` that returns `verdict = "needs-more-context"` MUST have that verdict collapsed to `fail` by the runner before storage.
10. Two different `StatelessEvaluator` implementations passed to `checkPr` with identical inputs MUST produce cache entries under the same key (the cache key is evaluator-agnostic).
