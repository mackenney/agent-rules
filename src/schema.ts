import { z } from "zod";

export const VerdictSchema = z.enum(["pass", "fail", "needs-more-context"]);
export type Verdict = z.infer<typeof VerdictSchema>;

export const SeveritySchema = z.enum(["warn", "error"]);
export type Severity = z.infer<typeof SeveritySchema>;

export type DisplayVerdict = "pass" | "warn" | "error";

export const RuleExampleSchema = z.object({
  description: z.string().default(""),
  code: z.string(),
  verdict: z.enum(["pass", "fail"]),
});
export type RuleExample = z.infer<typeof RuleExampleSchema>;

export const RuleSchema = z.object({
  id: z.string().min(1),
  name: z.string(),
  severity: SeveritySchema.default("warn"),
  enabled: z.boolean().default(true),
  context: z.enum(["stateless", "agentic"]).default("stateless"),
  prompt: z.string(),
  glob_include: z.array(z.string()).default(["**/*"]),
  glob_exclude: z.array(z.string()).default([]),
  examples: z.array(RuleExampleSchema).default([]),
  needs_more_context_when: z.string().default(""),
});
export type Rule = z.infer<typeof RuleSchema>;

export const RuleFileSchema = z.object({
  version: z.string().default("1"),
  inherit_mode: z.enum(["merge", "replace"]).default("merge"),
  rules: z.array(RuleSchema).default([]),
  source_path: z.string().default(""),
});
export type RuleFile = z.infer<typeof RuleFileSchema>;

export const ContextHintSchema = z.object({
  read_files: z.array(z.string()).default([]),
  question: z.string().default(""),
});
export type ContextHint = z.infer<typeof ContextHintSchema>;

export const RuleVerdictSchema = z.object({
  rule_id: z.string(),
  verdict: VerdictSchema,
  rule_severity: SeveritySchema.default("warn"),
  confidence: z.number().min(0).max(1).default(1.0),
  reasoning: z.string().default(""),
  line_refs: z.array(z.number().int()).default([]),
  context_hint: ContextHintSchema.nullable().default(null),
  from_agentic: z.boolean().default(false),
});
export type RuleVerdict = z.infer<typeof RuleVerdictSchema>;

export const ReportStatsSchema = z.object({
  total_files: z.number().int().default(0),
  files_with_rules: z.number().int().default(0),
  cache_hits: z.number().int().default(0),
  agentic_escalations: z.number().int().default(0),
  pass_count: z.number().int().default(0),
  warn_count: z.number().int().default(0),
  error_count: z.number().int().default(0),
  duration_ms: z.number().int().default(0),
});
export type ReportStats = z.infer<typeof ReportStatsSchema>;

export const FileVerdictSchema = z.object({
  file_path: z.string(),
  verdicts: z.array(RuleVerdictSchema).default([]),
  cached: z.boolean().default(false),
  check_duration_ms: z.number().int().default(0),
  agentic_escalations: z.number().int().default(0),
});
export type FileVerdict = z.infer<typeof FileVerdictSchema>;

export const PRReportSchema = z.object({
  pr_url: z.string().nullable().default(null),
  base_ref: z.string().default("HEAD~1"),
  head_ref: z.string().default("HEAD"),
  files: z.array(FileVerdictSchema).default([]),
  model_used: z.string().default(""),
  stats: ReportStatsSchema.default({}),
});
export type PRReport = z.infer<typeof PRReportSchema>;

export const FileDiffSchema = z.object({
  path: z.string(),
  diff: z.string(),
  content: z.string().nullable().default(null),
  is_binary: z.boolean().default(false),
  is_deleted: z.boolean().default(false),
  is_new: z.boolean().default(false),
});
export type FileDiff = z.infer<typeof FileDiffSchema>;

export const FileCheckRequestSchema = z.object({
  file_path: z.string(),
  diff: z.string(),
  content: z.string().nullable().default(null),
  rules: z.array(RuleSchema),
  repo_root: z.string().default("."),
});
export type FileCheckRequest = z.infer<typeof FileCheckRequestSchema>;

export function effectiveSeverity(rv: RuleVerdict): DisplayVerdict {
  if (rv.verdict === "pass") return "pass";
  return rv.rule_severity === "error" ? "error" : "warn";
}

export function fileVerdictOverall(fv: FileVerdict): DisplayVerdict {
  const severities = fv.verdicts.map(effectiveSeverity);
  if (severities.includes("error")) return "error";
  if (severities.includes("warn")) return "warn";
  return "pass";
}

export function prReportOverallVerdict(report: PRReport): DisplayVerdict {
  if (report.files.some((f) => fileVerdictOverall(f) === "error")) return "error";
  if (report.files.some((f) => fileVerdictOverall(f) === "warn")) return "warn";
  return "pass";
}
