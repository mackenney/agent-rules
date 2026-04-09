export const DEFAULTS = {
  statelessModel: "claude-haiku-4-5",
  agenticModel: "claude-sonnet-4-6",
  maxConcurrent: 10,
  timeoutMs: 60_000,
  agenticTimeoutMs: 180_000,
  maxStatelessTokens: 2048,
  maxDiffChars: 8_000,
  maxContentChars: 20_000,
  maxFileBytes: 100_000,
  cacheDir: ".agent-rules-cache",
} as const;
