import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync, readdirSync, unlinkSync } from "node:fs";
import { join } from "node:path";
import { FileVerdict, FileCheckRequest } from "./schema.js";

const DEFAULT_CACHE_DIR = ".agent-rules-cache";

const CACHE_VERSION = 2;

export interface CacheStats {
  totalEntries: number;
  totalHits: number;
  oldestEntryUnix: number | null;
  newestEntryUnix: number | null;
  cachePath: string | null;
}

export interface CacheInterface {
  get(cacheKey: string): FileVerdict | null;
  put(cacheKey: string, verdict: FileVerdict, model?: string): void;
  keyFor(request: FileCheckRequest, model: string): string;
  stats(): CacheStats;
  clear(): number;
}

interface CacheEntry {
  cache_key: string;
  file_path: string;
  rule_ids: string[];
  model: string;
  created_at: number;
  hit_count: number;
  verdict: FileVerdict;
}

function deriveKey(request: FileCheckRequest, model: string): string {
  const parts: string[] = [`version:${CACHE_VERSION}`, `model:${model}`];
  const sortedRules = [...request.rules].sort((a, b) => a.id.localeCompare(b.id));
  for (const rule of sortedRules) {
    parts.push(`rule:${rule.id}:${rule.severity}:${rule.prompt.trim()}`);
  }
  parts.push(`path:${request.file_path}`);
  parts.push(`content:${request.content ?? ""}`);
  parts.push(`diff:${request.diff}`);
  const payload = parts.join("\n");
  return createHash("sha256").update(payload, "utf-8").digest("hex");
}

export class CacheManager implements CacheInterface {
  private readonly cacheDir: string;

  constructor(cacheDir: string = DEFAULT_CACHE_DIR) {
    mkdirSync(cacheDir, { recursive: true });
    this.cacheDir = cacheDir;
  }

  keyFor(request: FileCheckRequest, model: string): string {
    return deriveKey(request, model);
  }

  private entryPath(key: string): string {
    return join(this.cacheDir, `${key}.json`);
  }

  get(cacheKey: string): FileVerdict | null {
    try {
      const raw = readFileSync(this.entryPath(cacheKey), "utf-8");
      const entry = JSON.parse(raw) as CacheEntry;
      entry.hit_count++;
      writeFileSync(this.entryPath(cacheKey), JSON.stringify(entry, null, 2));
      entry.verdict.cached = true;
      return entry.verdict;
    } catch {
      return null;
    }
  }

  put(cacheKey: string, verdict: FileVerdict, model: string = ""): void {
    const entry: CacheEntry = {
      cache_key: cacheKey,
      file_path: verdict.file_path,
      rule_ids: verdict.verdicts.map(v => v.rule_id),
      model,
      created_at: Date.now() / 1000,
      hit_count: 0,
      verdict,
    };
    writeFileSync(this.entryPath(cacheKey), JSON.stringify(entry, null, 2));
  }

  clear(): number {
    const files = readdirSync(this.cacheDir).filter(f => f.endsWith(".json"));
    for (const f of files) unlinkSync(join(this.cacheDir, f));
    return files.length;
  }

  stats(): CacheStats {
    const files = readdirSync(this.cacheDir).filter(f => f.endsWith(".json"));
    let totalHits = 0;
    let oldest: number | null = null;
    let newest: number | null = null;
    for (const f of files) {
      try {
        const entry = JSON.parse(readFileSync(join(this.cacheDir, f), "utf-8")) as CacheEntry;
        totalHits += entry.hit_count;
        if (oldest === null || entry.created_at < oldest) oldest = entry.created_at;
        if (newest === null || entry.created_at > newest) newest = entry.created_at;
      } catch { /* skip corrupt entries */ }
    }
    return {
      totalEntries: files.length,
      totalHits,
      oldestEntryUnix: oldest,
      newestEntryUnix: newest,
      cachePath: this.cacheDir,
    };
  }
}

export class NullCache implements CacheInterface {
  get(_cacheKey: string): FileVerdict | null {
    return null;
  }

  put(_cacheKey: string, _verdict: FileVerdict, _model?: string): void {}

  keyFor(request: FileCheckRequest, model: string): string {
    return deriveKey(request, model);
  }

  clear(): number {
    return 0;
  }

  stats(): CacheStats {
    return { totalEntries: 0, totalHits: 0, oldestEntryUnix: null, newestEntryUnix: null, cachePath: null };
  }
}
