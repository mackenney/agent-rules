import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { mkdirSync, writeFileSync, rmSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { getChangedFiles, getFileContent } from "../src/git.js";
import { DEFAULTS } from "../src/config.js";

function createTmpDir(): string {
  const dir = join(tmpdir(), `agent-rules-git-test-${Date.now()}-${Math.random().toString(36).slice(2)}`);
  mkdirSync(dir, { recursive: true });
  return dir;
}

function git(cwd: string, args: string[]): string {
  return execFileSync("git", args, { cwd, encoding: "utf-8" });
}

function initGitRepo(dir: string): void {
  git(dir, ["init", "-b", "main"]);
  git(dir, ["config", "user.email", "test@test.com"]);
  git(dir, ["config", "user.name", "Test"]);
}

describe("getChangedFiles", () => {
  let repoRoot: string;

  beforeEach(() => {
    repoRoot = createTmpDir();
    initGitRepo(repoRoot);
  });

  afterEach(() => {
    rmSync(repoRoot, { recursive: true, force: true });
  });

  it("detects added files with status A and is_new: true", () => {
    writeFileSync(join(repoRoot, "initial.txt"), "initial");
    git(repoRoot, ["add", "."]);
    git(repoRoot, ["commit", "-m", "initial"]);

    writeFileSync(join(repoRoot, "new.ts"), "const x = 1;");
    git(repoRoot, ["add", "."]);
    git(repoRoot, ["commit", "-m", "add new file"]);

    const files = getChangedFiles("HEAD~1", "HEAD", repoRoot);
    expect(files).toHaveLength(1);
    expect(files[0]?.path).toBe("new.ts");
    expect(files[0]?.is_new).toBe(true);
    expect(files[0]?.is_deleted).toBe(false);
    expect(files[0]?.content).toBe("const x = 1;");
  });

  it("detects deleted files with status D and is_deleted: true", () => {
    writeFileSync(join(repoRoot, "todelete.ts"), "const x = 1;");
    git(repoRoot, ["add", "."]);
    git(repoRoot, ["commit", "-m", "add file"]);

    git(repoRoot, ["rm", "todelete.ts"]);
    git(repoRoot, ["commit", "-m", "delete file"]);

    const files = getChangedFiles("HEAD~1", "HEAD", repoRoot);
    expect(files).toHaveLength(1);
    expect(files[0]?.path).toBe("todelete.ts");
    expect(files[0]?.is_deleted).toBe(true);
    expect(files[0]?.content).toBeNull();
  });

  it("detects modified files with diff content", () => {
    writeFileSync(join(repoRoot, "modify.ts"), "const x = 1;");
    git(repoRoot, ["add", "."]);
    git(repoRoot, ["commit", "-m", "add file"]);

    writeFileSync(join(repoRoot, "modify.ts"), "const x = 2;");
    git(repoRoot, ["add", "."]);
    git(repoRoot, ["commit", "-m", "modify file"]);

    const files = getChangedFiles("HEAD~1", "HEAD", repoRoot);
    expect(files).toHaveLength(1);
    expect(files[0]?.path).toBe("modify.ts");
    expect(files[0]?.diff).toContain("-const x = 1;");
    expect(files[0]?.diff).toContain("+const x = 2;");
    expect(files[0]?.content).toBe("const x = 2;");
  });

  it("detects renamed files using the new name", () => {
    writeFileSync(join(repoRoot, "old.ts"), "const x = 1;");
    git(repoRoot, ["add", "."]);
    git(repoRoot, ["commit", "-m", "add file"]);

    git(repoRoot, ["mv", "old.ts", "new.ts"]);
    git(repoRoot, ["commit", "-m", "rename file"]);

    const files = getChangedFiles("HEAD~1", "HEAD", repoRoot);
    const newFile = files.find((f) => f.path === "new.ts");
    expect(newFile).toBeDefined();
  });

  it("marks files with binary extensions as is_binary: true", () => {
    writeFileSync(join(repoRoot, "initial.txt"), "initial");
    git(repoRoot, ["add", "."]);
    git(repoRoot, ["commit", "-m", "initial"]);

    writeFileSync(join(repoRoot, "image.png"), Buffer.from([0x89, 0x50, 0x4e, 0x47]));
    git(repoRoot, ["add", "."]);
    git(repoRoot, ["commit", "-m", "add binary"]);

    const files = getChangedFiles("HEAD~1", "HEAD", repoRoot);
    expect(files).toHaveLength(1);
    expect(files[0]?.path).toBe("image.png");
    expect(files[0]?.is_binary).toBe(true);
    expect(files[0]?.content).toBeNull();
  });

  it("returns null content for files exceeding maxFileBytes", () => {
    writeFileSync(join(repoRoot, "initial.txt"), "initial");
    git(repoRoot, ["add", "."]);
    git(repoRoot, ["commit", "-m", "initial"]);

    const largeContent = "x".repeat(DEFAULTS.maxFileBytes + 1000);
    writeFileSync(join(repoRoot, "large.ts"), largeContent);
    git(repoRoot, ["add", "."]);
    git(repoRoot, ["commit", "-m", "add large file"]);

    const files = getChangedFiles("HEAD~1", "HEAD", repoRoot);
    expect(files).toHaveLength(1);
    expect(files[0]?.content).toBeNull();
  });

  it("returns empty array for no changes between refs", () => {
    writeFileSync(join(repoRoot, "file.ts"), "const x = 1;");
    git(repoRoot, ["add", "."]);
    git(repoRoot, ["commit", "-m", "add file"]);

    const files = getChangedFiles("HEAD", "HEAD", repoRoot);
    expect(files).toEqual([]);
  });
});

describe("getFileContent", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = createTmpDir();
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("returns file content for readable files", () => {
    const path = join(tmpDir, "test.ts");
    writeFileSync(path, "const x = 1;");
    expect(getFileContent(path)).toBe("const x = 1;");
  });

  it("returns null for nonexistent files", () => {
    const path = join(tmpDir, "nonexistent.ts");
    expect(getFileContent(path)).toBeNull();
  });

  it("returns null for files exceeding maxFileBytes", () => {
    const path = join(tmpDir, "large.ts");
    writeFileSync(path, "x".repeat(DEFAULTS.maxFileBytes + 1));
    expect(getFileContent(path)).toBeNull();
  });
});
