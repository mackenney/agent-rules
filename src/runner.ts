import Anthropic from "@anthropic-ai/sdk";
import { CacheInterface } from "./cache.js";
import { DEFAULTS } from "./config.js";
import {
  ContextHint,
  FileCheckRequest,
  FileVerdict,
  PRReport,
  ReportStats,
  Rule,
  RuleVerdict,
  Verdict,
  fileVerdictOverall,
  prReportOverallVerdict,
} from "./schema.js";
import type { ProgressReporter } from "./reporter.js";
import { createSemaphore } from "./concurrency.js";
import { AgenticOpts, runStateless, runAgenticEscalation } from "./llm.js";
import { parseVerdicts } from "./verdict-parser.js";

export const DEFAULT_MAX_CONCURRENT = DEFAULTS.maxConcurrent;
export const DEFAULT_MAX_AGENTIC_CONCURRENT = DEFAULTS.maxAgenticConcurrent;
export const DEFAULT_TIMEOUT_MS = DEFAULTS.timeoutMs;
export const DEFAULT_AGENTIC_TIMEOUT_MS = DEFAULTS.agenticTimeoutMs;

// Re-exports
export { createSemaphore } from "./concurrency.js";
export { parseVerdicts } from "./verdict-parser.js";

/**
 * Infrastructure dependencies for running checks.
 * Includes the LLM client, cache layer, and optional progress reporter.
 */
export interface CheckInfra {
  /** Anthropic SDK client instance */
  client: Anthropic;
  /** Cache implementation (CacheManager or NullCache) */
  cache: CacheInterface;
  /** Optional progress reporter for TTY updates */
  progress?: ProgressReporter;
}

/**
 * Configuration options for check execution.
 * All fields are optional with sensible defaults.
 */
export interface CheckConfig {
  /** Model for stateless pass (default: claude-haiku-4-5) */
  model?: string;
  /** Timeout in ms for stateless LLM calls (default: 60000) */
  timeoutMs?: number;
  /** Max concurrent LLM calls in checkPr (default: 10) */
  maxConcurrent?: number;
  /** Max concurrent agentic escalations (default: 2) */
  maxAgenticConcurrent?: number;
  /** Log prompts and responses to stderr (default: false) */
  trace?: boolean;
  /** Agentic escalation configuration */
  agentic?: AgenticConfig;
}

/**
 * Configuration for agentic escalation (pi sessions).
 * apiKey is NOT here — it's read from process.env in llm.ts.
 */
export interface AgenticConfig {
  /** Model for agentic pass (default: claude-sonnet-4-6) */
  model?: string;
  /** Timeout in ms for pi session (default: 180000) */
  timeoutMs?: number;
  /** Allow bash tool in pi session (default: false) */
  allowBash?: boolean;
}

/**
 * PR metadata for report generation.
 */
export interface PRMeta {
  /** GitHub PR URL (optional, for report labeling) */
  prUrl?: string;
  /** Base git ref for diff context (default: HEAD~1) */
  baseRef?: string;
  /** Head git ref (default: HEAD) */
  headRef?: string;
}

export async function checkFile(
  request: FileCheckRequest,
  infra: CheckInfra,
  config: CheckConfig = {},
  cacheSystemPrompt: boolean = false,
  cacheFileContext: boolean = false,
  agenticSemaphore?: ReturnType<typeof createSemaphore>,
  releaseStateless?: () => void,
): Promise<FileVerdict> {
  const { client, cache, progress } = infra;
  const model = config.model ?? DEFAULTS.statelessModel;
  const timeoutMs = config.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  const trace = config.trace ?? false;
  const agenticOpts: AgenticOpts = {
    timeoutMs: config.agentic?.timeoutMs ?? DEFAULT_AGENTIC_TIMEOUT_MS,
    allowBash: config.agentic?.allowBash ?? false,
    model: config.agentic?.model ?? DEFAULTS.agenticModel,
  };
  const startMs = Date.now();

  const cacheKey = cache.keyFor(request, model);
  const cached = cache.get(cacheKey);
  if (cached) return cached;

  const ruleLabel = `${request.file_path} [${request.rules[0]?.id ?? "?"}]`;
  progress?.onCallStart(ruleLabel);

  const statelessVerdicts = await runStateless(client, request, model, timeoutMs, cacheSystemPrompt, cacheFileContext, trace, progress);
  releaseStateless?.();
  progress?.onCallDone(ruleLabel);

  const terminalVerdicts: RuleVerdict[] = [];
  const escalatedRules: Rule[] = [];
  const hints: (ContextHint | null)[] = [];

  const ruleMap = new Map(request.rules.map((r) => [r.id, r]));

  for (const rv of statelessVerdicts) {
    const rule = ruleMap.get(rv.rule_id);
    if (rv.verdict === "needs-more-context") {
      if (rule && rule.context === "agentic") {
        escalatedRules.push(rule);
        hints.push(rv.context_hint ?? null);
      } else {
        terminalVerdicts.push({
          ...rv,
          verdict: "fail" as Verdict,
          reasoning:
            rv.reasoning + " [collapsed from needs-more-context: stateless rule]",
        });
      }
    } else {
      terminalVerdicts.push(rv);
    }
  }

  let agenticVerdicts: RuleVerdict[] = [];
  for (let i = 0; i < escalatedRules.length; i++) {
    const rule = escalatedRules[i]!;
    const ruleHints = hints[i] != null ? [hints[i]!] : [];
    const agenticLabel = `${request.file_path} [${rule.id}] [agentic]`;
    progress?.addTotal(1);
    progress?.onCallStart(agenticLabel);
    await agenticSemaphore?.acquire();
    try {
      const verdicts = await runAgenticEscalation(
        { ...request, rules: [rule] },
        [rule],
        ruleHints,
        agenticOpts,
        progress,
        trace
      );
      agenticVerdicts.push(...verdicts);
    } finally {
      agenticSemaphore?.release();
    }
    progress?.onCallDone(agenticLabel);
  }

  const allVerdicts = [...terminalVerdicts, ...agenticVerdicts];
  const durationMs = Date.now() - startMs;

  const fileVerdict: FileVerdict = {
    file_path: request.file_path,
    verdicts: allVerdicts,
    cached: false,
    check_duration_ms: durationMs,
    agentic_escalations: escalatedRules.length,
  };

  cache.put(cacheKey, fileVerdict, model);
  return fileVerdict;
}

export async function checkPr(
  files: FileCheckRequest[],
  infra: CheckInfra,
  config: CheckConfig = {},
  meta: PRMeta = {}
): Promise<PRReport> {
  const { client, cache, progress } = infra;
  const model = config.model ?? DEFAULTS.statelessModel;
  const maxConcurrent = config.maxConcurrent ?? DEFAULT_MAX_CONCURRENT;
  const maxAgenticConcurrent = config.maxAgenticConcurrent ?? DEFAULT_MAX_AGENTIC_CONCURRENT;
  const trace = config.trace ?? false;
  const prUrl = meta.prUrl ?? null;
  const baseRef = meta.baseRef ?? "HEAD~1";
  const headRef = meta.headRef ?? "HEAD";
  const startMs = Date.now();
  const semaphore = createSemaphore(maxConcurrent);
  const agenticSemaphore = createSemaphore(maxAgenticConcurrent);

  // Flatten to one (file, rule) task per rule across all files.
  // Each task becomes a single LLM call so the model gives full attention to one rule.
  const ruleTasks: Array<{ req: FileCheckRequest; cacheFileCtx: boolean }> = [];
  for (const f of files) {
    const cacheFileCtx = f.rules.length > 1;
    for (const rule of f.rules) {
      ruleTasks.push({ req: { ...f, rules: [rule] }, cacheFileCtx });
    }
  }

  // System prompt is cached whenever there is more than one LLM call to make.
  const cacheSystemPrompt = ruleTasks.length > 1;

  const ruleResults = await Promise.all(
    ruleTasks.map(async ({ req, cacheFileCtx }) => {
      await semaphore.acquire();
      // releaseStateless lets checkFile free this slot as soon as the stateless
      // pass completes, before any agentic escalation begins.
      let released = false;
      const releaseStateless = () => {
        if (!released) { released = true; semaphore.release(); }
      };
      try {
        return await checkFile(req, infra, config, cacheSystemPrompt, cacheFileCtx, agenticSemaphore, releaseStateless);
      } finally {
        releaseStateless();
      }
    })
  );

  // Merge per-rule FileVerdicts back into per-file FileVerdicts.
  const verdictsByFile = new Map<string, FileVerdict>();
  for (const f of files) {
    verdictsByFile.set(f.file_path, {
      file_path: f.file_path,
      verdicts: [],
      cached: true,  // flipped to false if any rule misses cache
      check_duration_ms: 0,
      agentic_escalations: 0,
    });
  }
  for (const rv of ruleResults) {
    const entry = verdictsByFile.get(rv.file_path)!;
    entry.verdicts.push(...rv.verdicts);
    entry.check_duration_ms += rv.check_duration_ms;
    entry.agentic_escalations += rv.agentic_escalations;
    if (!rv.cached) entry.cached = false;
  }
  const results = files.map((f) => verdictsByFile.get(f.file_path)!);

  const cacheHits = results.filter((r) => r.cached).length;
  const agenticEscalations = results.reduce((sum, r) => sum + r.agentic_escalations, 0);
  const passCount = results.filter((r) => fileVerdictOverall(r) === "pass").length;
  const warnCount = results.filter((r) => fileVerdictOverall(r) === "warn").length;
  const errorCount = results.filter((r) => fileVerdictOverall(r) === "error").length;
  const durationMs = Date.now() - startMs;

  const stats: ReportStats = {
    total_files: files.length,
    files_with_rules: files.filter((f) => f.rules.length > 0).length,
    cache_hits: cacheHits,
    agentic_escalations: agenticEscalations,
    pass_count: passCount,
    warn_count: warnCount,
    error_count: errorCount,
    duration_ms: durationMs,
  };

  return {
    pr_url: prUrl,
    base_ref: baseRef,
    head_ref: headRef,
    files: results,
    model_used: model,
    stats,
  };
}
