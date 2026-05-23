#!/usr/bin/env node
import { Command } from "commander";
import { resolve, join, isAbsolute } from "node:path";
import { statSync } from "node:fs";
import Anthropic from "@anthropic-ai/sdk";
import chalk from "chalk";
import { CacheManager, NullCache } from "./cache.js";
import { getChangedFiles, getFileContent } from "./git.js";
import { postPrComment, SENTINEL } from "./github.js";
import {
  printReport,
  formatJson,
  formatGithubComment,
  createProgressReporter,
} from "./reporter.js";
import { maybeWriteStepSummary, emitWorkflowAnnotations } from "./ci.js";
import { resolveRules, findAllRuleFiles } from "./resolver.js";
import { loadRuleFile, allRules } from "./parser.js";
import { checkPr, type CheckInfra, type CheckConfig, type AgenticConfig, type PRMeta, DEFAULT_AGENTIC_TIMEOUT_MS, DEFAULT_TIMEOUT_MS } from "./runner.js";
import { DEFAULTS } from "./config.js";
import {
  FileCheckRequest,
  FileDiff,
  PRReport,
  DisplayVerdict,
  prReportOverallVerdict,
} from "./schema.js";

function getRepoRoot(repo?: string): string {
  return repo ? resolve(repo) : process.cwd();
}

function computeExitCode(overall: DisplayVerdict, warnAsError: boolean): number {
  if (overall === "error") return 2;
  if (overall === "warn" && warnAsError) return 1;
  return 0;
}

const program = new Command();

program
  .name("agent-rules")
  .description("Directory-scoped AI rule enforcement for PR reviews.")
  .version("0.1.0");

program
  .command("check")
  .description("Run rule checks against changed or specified files.")
  .option("--base <ref>", "Base git ref for diff", "main")
  .option("--head <ref>", "Head git ref", "HEAD")
  .option("--pr <url>", "GitHub PR URL for report labels and --post-comment (does not drive the diff)")
  .option("--files <paths...>", "Check specific files instead of git diff")
  .option("--repo <path>", "Repository root")
  .option("--dir-filter <dirs...>", "Only check files under these directories (additive, comma-separated or repeated)")
  .option("--output <format>", "Output format: text, json, github", "text")
  .option("--warn-as-error", "Treat warn-severity violations as errors (exit 1 on warn)", false)
  .option("--no-cache", "Disable cache")
  .option("--model <name>", "Override LLM model", DEFAULTS.statelessModel)
  .option("--max-concurrent <n>", "Max parallel stateless LLM calls", String(DEFAULTS.maxConcurrent))
  .option("--max-file-bytes <n>", "Skip files whose byte size exceeds this limit", String(DEFAULTS.maxFileBytes))
  .option("--max-diff-chars <n>", "Skip files whose diff exceeds this character count", String(DEFAULTS.maxDiffChars))
  .option("--max-content-chars <n>", "Skip files whose content exceeds this character count", String(DEFAULTS.maxContentChars))
  .option("--verbose", "Show full diagnostic output with source context", false)
  .option("--trace", "Also print raw prompts and LLM responses to stderr (implies --verbose)", false)
  .option("--post-comment", "Post results as a GitHub PR comment", false)
  .option("--allow-bash", "Enable bash tool in agentic escalation", false)
  .option("--timeout <ms>", "Timeout for stateless LLM calls in ms", String(DEFAULTS.timeoutMs))
  .option("--agentic-timeout <ms>", "Timeout for agentic escalation in ms", String(DEFAULTS.agenticTimeoutMs))
  .option("--agentic-concurrency <n>", "Max parallel agentic escalations", String(DEFAULTS.maxAgenticConcurrent))
  .option("--agentic-model <model>", "Model for agentic escalation", DEFAULTS.agenticModel)
  .option("--strict-rules", "Exit with error if any .agent-rules.toml on disk differs from the head ref", false)
  .action(async (opts) => {
    const repoRoot = getRepoRoot(opts.repo as string | undefined);

    const apiKey = process.env["ANTHROPIC_API_KEY"] ?? "";
    if (!apiKey) {
      console.error(
        "Error: ANTHROPIC_API_KEY environment variable is not set.\nSet it with: export ANTHROPIC_API_KEY=sk-ant-..."
      );
      process.exit(3);
    }

    const maxConcurrent = parseInt(opts.maxConcurrent as string, 10);
    if (!Number.isFinite(maxConcurrent) || maxConcurrent < 1) {
      console.error("Error: --max-concurrent must be a positive integer");
      process.exit(3);
    }

    const maxFileBytes = parseInt(opts.maxFileBytes as string, 10);
    if (!Number.isFinite(maxFileBytes) || maxFileBytes < 1) {
      console.error("Error: --max-file-bytes must be a positive integer");
      process.exit(3);
    }
    const maxDiffChars = parseInt(opts.maxDiffChars as string, 10);
    if (!Number.isFinite(maxDiffChars) || maxDiffChars < 1) {
      console.error("Error: --max-diff-chars must be a positive integer");
      process.exit(3);
    }
    const maxContentChars = parseInt(opts.maxContentChars as string, 10);
    if (!Number.isFinite(maxContentChars) || maxContentChars < 1) {
      console.error("Error: --max-content-chars must be a positive integer");
      process.exit(3);
    }

    const outputOpt = opts.output as string | undefined;
    if (outputOpt !== undefined && !["text", "json", "github"].includes(outputOpt)) {
      console.error(`Error: --output must be 'text', 'json', or 'github', got '${outputOpt}'`);
      process.exit(3);
    }

    let fileDiffs: FileDiff[] = [];

    if (opts.files && (opts.files as string[]).length > 0) {
      for (const fp of opts.files as string[]) {
        const absPath = isAbsolute(fp) ? fp : join(repoRoot, fp);
        let content: string | null = null;
        let is_oversized = false;
        let oversized_bytes: number | null = null;
        try {
          const st = statSync(absPath);
          if (st.size > maxFileBytes) {
            is_oversized = true;
            oversized_bytes = st.size;
          } else {
            content = getFileContent(absPath, maxFileBytes);
          }
        } catch {
          // file not on disk — content stays null
        }
        fileDiffs.push({
          path: fp,
          diff: "",
          content,
          is_binary: false,
          is_deleted: false,
          is_new: false,
          is_oversized,
          oversized_bytes,
        });
      }
    } else {
      try {
        fileDiffs = getChangedFiles(opts.base as string, opts.head as string, repoRoot, maxFileBytes);
      } catch (err) {
        console.error(`git error: ${(err as Error).message}`);
        process.exit(3);
      }
    }

    const rawDirFilters: string[] = ([] as string[])
      .concat(opts.dirFilter ?? [])
      .flatMap((d: string) => d.split(","))
      .map((d: string) => d.trim())
      .filter(Boolean);
    const dirFilters = rawDirFilters.map((d) => {
      const abs = isAbsolute(d) ? d : join(repoRoot, d);
      return abs.endsWith("/") ? abs : abs + "/";
    });

    if (dirFilters.length > 0) {
      fileDiffs = fileDiffs.filter((fd) => {
        const abs = join(repoRoot, fd.path) + (fd.path.endsWith("/") ? "" : "");
        return dirFilters.some((dir) => (abs + "/").startsWith(dir) || abs.startsWith(dir));
      });
    }

    if (fileDiffs.length === 0) {
      console.log("No changed files found.");
      process.exit(0);
    }

    const requests: FileCheckRequest[] = [];
    for (const fd of fileDiffs) {
      if (fd.is_binary) continue;
      const rules = resolveRules(fd.path, repoRoot, {
        headRef: opts.head as string,
        strictRules: Boolean(opts.strictRules),
      });
      if (rules.length === 0) continue;

      const diffTooLong = fd.diff.length > maxDiffChars;
      const contentTooLong = fd.content !== null && fd.content.length > maxContentChars;
      if (fd.is_oversized || diffTooLong || contentTooLong) {
        let reason: string;
        if (fd.is_oversized) {
          const actual = fd.oversized_bytes !== null ? ` (${fd.oversized_bytes} bytes)` : "";
          reason = `byte size${actual} exceeds --max-file-bytes ${maxFileBytes}`;
        } else if (diffTooLong) {
          reason = `diff length ${fd.diff.length} chars exceeds --max-diff-chars ${maxDiffChars}`;
        } else {
          reason = `content length ${fd.content!.length} chars exceeds --max-content-chars ${maxContentChars}`;
        }
        for (const rule of rules) {
          process.stderr.write(`warning: (${rule.id}, ${fd.path}) - file skipped: ${reason}\n`);
        }
        continue;
      }

      requests.push({
        file_path: fd.path,
        diff: fd.diff,
        content: fd.content,
        rules,
        repo_root: repoRoot,
      });
    }

    if (requests.length === 0) {
      console.log("No rules apply to any changed file.");
      process.exit(0);
    }

    const client = new Anthropic({ apiKey });
    const cache =
      opts.cache === false
        ? new NullCache()
        : new CacheManager(join(repoRoot, DEFAULTS.cacheDir));

    const agenticConfig: AgenticConfig = {
      timeoutMs: parseInt(opts.agenticTimeout as string, 10) || undefined,
      allowBash: Boolean(opts.allowBash) || undefined,
      model: (opts.agenticModel as string) || undefined,
    };

    const totalRules = requests.reduce((sum, r) => sum + r.rules.length, 0);
    const progress = createProgressReporter(totalRules);

    const trace = Boolean(opts.trace);
    const infra: CheckInfra = { client, cache, progress };
    const checkConfig: CheckConfig = {
      model: opts.model as string,
      maxConcurrent: parseInt(opts.maxConcurrent as string, 10),
      maxAgenticConcurrent: parseInt(opts.agenticConcurrency as string, 10) || undefined,
      timeoutMs: parseInt(opts.timeout as string, 10) || undefined,
      trace,
      agentic: agenticConfig,
    };
    const prMeta: PRMeta = {
      prUrl: (opts.pr as string | undefined) ?? undefined,
      baseRef: opts.base as string,
      headRef: opts.head as string,
    };

    let report!: PRReport;
    try {
      report = await checkPr(requests, infra, checkConfig, prMeta);
    } finally {
      progress.stop();
    }

    const output = opts.output as string;
    if (output === "json") {
      process.stdout.write(formatJson(report) + "\n");
    } else if (output === "github") {
      process.stdout.write(formatGithubComment(report) + "\n");
    } else {
      printReport(report, (opts.verbose as boolean) || trace, repoRoot);
    }

    maybeWriteStepSummary(report);
    emitWorkflowAnnotations(report);

    if (opts.postComment) {
      if (!opts.pr) {
        console.error("Error: --post-comment requires --pr <GitHub PR URL>");
        process.exit(3);
      }
      const githubToken = process.env["GITHUB_TOKEN"] ?? "";
      if (!githubToken) {
        console.error("Error: GITHUB_TOKEN is not set — required for --post-comment");
        process.exit(3);
      }
      try {
        const body = `${SENTINEL}\n${formatGithubComment(report)}`;
        await postPrComment(opts.pr as string, body, githubToken);
        console.log("✓ Posted agent-rules comment to PR");
      } catch (err) {
        console.error(`Error posting PR comment: ${(err as Error).message}`);
        process.exit(3);
      }
    }

    const overall = prReportOverallVerdict(report);
    process.exit(computeExitCode(overall, Boolean(opts.warnAsError)));
  });

const cacheCmd = program.command("cache").description("Cache management commands.");

cacheCmd
  .command("stats")
  .description("Show cache statistics.")
  .option("--repo <path>", "Repository root")
  .action((opts) => {
    const repoRoot = getRepoRoot(opts.repo as string | undefined);
    const cacheDir = join(repoRoot, DEFAULTS.cacheDir);

    try {
      statSync(cacheDir);
    } catch {
      console.log("No cache found.");
      process.exit(0);
    }

    try {
      const mgr = new CacheManager(cacheDir);
      const stats = mgr.stats();
      const oldest = stats.oldestEntryUnix;

      let oldestAge = "n/a";
      if (oldest) {
        const ageS = Date.now() / 1000 - oldest;
        if (ageS < 3600) oldestAge = `${Math.round(ageS / 60)}m ago`;
        else if (ageS < 86400) oldestAge = `${(ageS / 3600).toFixed(1)}h ago`;
        else oldestAge = `${(ageS / 86400).toFixed(1)}d ago`;
      }

      console.log(`Cache statistics  (${stats.cachePath ?? cacheDir})`);
      console.log(`  Total entries : ${stats.totalEntries}`);
      console.log(`  Total hits    : ${stats.totalHits}`);
      console.log(`  Oldest entry  : ${oldestAge}`);
    } catch (err) {
      console.error(`Error reading cache: ${(err as Error).message}`);
      process.exit(1);
    }
  });

cacheCmd
  .command("clear")
  .description("Clear all cache entries.")
  .option("--repo <path>", "Repository root")
  .option("-y, --yes", "Skip confirmation", false)
  .action(async (opts) => {
    const repoRoot = getRepoRoot(opts.repo as string | undefined);
    const cacheDir = join(repoRoot, DEFAULTS.cacheDir);

    try {
      statSync(cacheDir);
    } catch {
      console.log("No cache found — nothing to clear.");
      process.exit(0);
    }

    if (!opts.yes) {
      const { createInterface } = await import("node:readline");
      const rl = createInterface({ input: process.stdin, output: process.stdout });
      const answer = await new Promise<string>((resolveP) => {
        rl.question("Clear all cache entries? [y/N] ", resolveP);
      });
      rl.close();
      if (!answer.toLowerCase().startsWith("y")) {
        console.log("Aborted.");
        process.exit(0);
      }
    }

    try {
      const mgr = new CacheManager(cacheDir);
      const count = mgr.clear();
      console.log(`Cleared ${count} cache entries.`);
    } catch (err) {
      console.error(`Error clearing cache: ${(err as Error).message}`);
      process.exit(1);
    }
  });

const rulesCmd = program.command("rules").description("Rule management commands.");

rulesCmd
  .command("list")
  .description("Show which rules apply to a given file path.")
  .option("--path <file>", "File path to resolve rules for", ".")
  .option("--repo <path>", "Repository root")
  .action((opts) => {
    const repoRoot = getRepoRoot(opts.repo as string | undefined);

    try {
      const applicable = resolveRules(opts.path as string, repoRoot);  // rules list cmd: no ref grounding needed

      if (applicable.length === 0) {
        console.log(`No rules apply to '${opts.path}'.`);
        process.exit(0);
      }

      console.log(`${applicable.length} rule(s) apply to '${opts.path}':`);
      for (const rule of applicable) {
        const colored = rule.severity === "warn" ? chalk.yellow(rule.severity) : chalk.red(rule.severity);
        console.log(`  ${colored}  ${rule.id}  ${rule.name}`);
      }
    } catch (err) {
      console.error(`Error resolving rules: ${(err as Error).message}`);
      process.exit(1);
    }
  });

rulesCmd
  .command("validate")
  .description("Walk repo for .agent-rules.toml files and validate each.")
  .option("--repo <path>", "Repository root")
  .action((opts) => {
    const repoRoot = getRepoRoot(opts.repo as string | undefined);
    const ruleFiles = findAllRuleFiles(repoRoot);

    if (ruleFiles.length === 0) {
      console.log("No .agent-rules.toml files found.");
      process.exit(0);
    }

    let errorsFound = false;
    let totalRules = 0;

    for (const rfPath of ruleFiles.sort()) {
      const rel = rfPath.slice(repoRoot.length + 1);
      try {
        const ruleFile = loadRuleFile(rfPath);
        const rules = allRules(ruleFile);
        totalRules += rules.length;
        console.log(`✓ ${rel}  ${rules.length} rule(s)`);
        for (const r of rules) {
          console.log(`    • ${r.id}  ${r.name}`);
        }
      } catch (err) {
        errorsFound = true;
        console.error(`✗ ${rel}  ${(err as Error).message}`);
      }
    }

    console.log(`\nValidated ${ruleFiles.length} file(s), ${totalRules} rule(s) total.`);
    if (errorsFound) process.exit(1);
  });

program.parseAsync(process.argv).catch((err: unknown) => {
  console.error((err as Error).message);
  process.exit(3);
});
