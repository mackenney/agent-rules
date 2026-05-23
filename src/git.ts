import { execFileSync } from "node:child_process";
import { resolve, join } from "node:path";
import { statSync, readFileSync } from "node:fs";
import { FileDiff } from "./schema.js";
import { DEFAULTS } from "./config.js";

const BINARY_EXTENSIONS = new Set([
  ".png", ".jpg", ".jpeg", ".gif", ".webp", ".ico", ".svg",
  ".pdf", ".zip", ".tar", ".gz", ".bz2", ".xz", ".whl",
  ".pyc", ".pyo", ".so", ".dylib", ".dll", ".exe",
  ".db", ".sqlite", ".sqlite3",
]);

export function getChangedFiles(
  baseRef: string,
  headRef: string = "HEAD",
  repoRoot: string = "."
): FileDiff[] {
  const root = resolve(repoRoot);

  const nameStatus = runGit(["diff", "--name-status", baseRef, headRef], root);

  const results: FileDiff[] = [];

  for (const line of nameStatus.split("\n")) {
    if (!line.trim()) continue;

    const parts = line.split("\t");
    const status = (parts[0] ?? "").trim();

    let filePath: string;
    if (status.startsWith("R") && parts.length >= 3) {
      filePath = parts[2]!;
    } else {
      filePath = parts[1]!;
    }

    const isDeleted = status === "D";
    const isNew = status === "A";

    if (isBinary(filePath)) {
      results.push({
        path: filePath,
        diff: "",
        content: null,
        is_binary: true,
        is_deleted: isDeleted,
        is_new: isNew,
      });
      continue;
    }

    let diffOutput: string;
    try {
      diffOutput = runGit(["diff", baseRef, headRef, "--", filePath], root);
    } catch {
      diffOutput = "";
    }

    if (diffOutput.includes("Binary files")) {
      results.push({
        path: filePath,
        diff: "",
        content: null,
        is_binary: true,
        is_deleted: isDeleted,
        is_new: isNew,
      });
      continue;
    }

    let content: string | null = null;
    if (!isDeleted) {
      try {
        content = runGit(["show", `${headRef}:${filePath}`], root);
        if (Buffer.byteLength(content, "utf-8") > DEFAULTS.maxFileBytes) {
          content = null;
        }
      } catch {
        const absPath = join(root, filePath);
        content = getFileContent(absPath);
      }
    }

    results.push({
      path: filePath,
      diff: diffOutput,
      content,
      is_binary: false,
      is_deleted: isDeleted,
      is_new: isNew,
    });
  }

  return results;
}

function isBinary(filePath: string): boolean {
  const ext = filePath.slice(filePath.lastIndexOf(".")).toLowerCase();
  return BINARY_EXTENSIONS.has(ext);
}

export function getFileContent(absPath: string): string | null {
  try {
    const stat = statSync(absPath);
    if (stat.size > DEFAULTS.maxFileBytes) return null;
    return readFileSync(absPath, { encoding: "utf-8" });
  } catch {
    return null;
  }
}

function runGit(args: string[], cwd: string): string {
  try {
    return execFileSync("git", args, { cwd, encoding: "utf-8" });
  } catch (err) {
    const e = err as { stderr?: string; message?: string };
    throw new Error(`git ${args.join(" ")} failed: ${e.stderr?.trim() ?? e.message}`);
  }
}

/** Read a file from a specific git ref. Returns null if the ref or path does not exist. */
export function getFileAtRef(relPath: string, ref: string, repoRoot: string): string | null {
  try {
    return runGit(["show", `${ref}:${relPath}`], resolve(repoRoot));
  } catch {
    return null;
  }
}
