import { describe, it, expect, vi, beforeEach } from "vitest";
import { parsePrUrl, postPrComment, SENTINEL } from "../src/github.js";

const mockListComments = vi.fn();
const mockCreateComment = vi.fn();
const mockUpdateComment = vi.fn();

vi.mock("@octokit/rest", () => ({
  Octokit: vi.fn().mockImplementation(() => ({
    issues: {
      listComments: mockListComments,
      createComment: mockCreateComment,
      updateComment: mockUpdateComment,
    },
  })),
}));

describe("parsePrUrl", () => {
  it("parses valid GitHub PR URL", () => {
    const result = parsePrUrl("https://github.com/owner/repo/pull/123");
    expect(result).toEqual({ owner: "owner", repo: "repo", prNumber: 123 });
  });

  it("handles URL with trailing slash", () => {
    const result = parsePrUrl("https://github.com/owner/repo/pull/456/");
    expect(result).toEqual({ owner: "owner", repo: "repo", prNumber: 456 });
  });

  it("throws for invalid URL format", () => {
    expect(() => parsePrUrl("https://github.com/owner/repo")).toThrow(
      /Cannot parse GitHub PR URL/
    );
  });

  it("throws for non-GitHub URL", () => {
    expect(() => parsePrUrl("https://gitlab.com/owner/repo/pull/1")).toThrow(
      /Cannot parse GitHub PR URL/
    );
  });

  it("includes expected format hint in error message", () => {
    expect(() => parsePrUrl("invalid")).toThrow(
      /Expected format: https:\/\/github\.com\/owner\/repo\/pull\/N/
    );
  });
});

describe("postPrComment", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("creates new comment when no existing sentinel found", async () => {
    mockListComments.mockResolvedValue({ data: [] });
    mockCreateComment.mockResolvedValue({});

    await postPrComment(
      "https://github.com/owner/repo/pull/1",
      `${SENTINEL}\nTest body`,
      "test-token"
    );

    expect(mockCreateComment).toHaveBeenCalledWith({
      owner: "owner",
      repo: "repo",
      issue_number: 1,
      body: `${SENTINEL}\nTest body`,
    });
    expect(mockUpdateComment).not.toHaveBeenCalled();
  });

  it("updates existing comment when sentinel found", async () => {
    mockListComments.mockResolvedValue({
      data: [
        { id: 100, body: "unrelated comment" },
        { id: 200, body: `${SENTINEL}\nOld content` },
      ],
    });
    mockUpdateComment.mockResolvedValue({});

    await postPrComment(
      "https://github.com/owner/repo/pull/1",
      `${SENTINEL}\nNew content`,
      "test-token"
    );

    expect(mockUpdateComment).toHaveBeenCalledWith({
      owner: "owner",
      repo: "repo",
      comment_id: 200,
      body: `${SENTINEL}\nNew content`,
    });
    expect(mockCreateComment).not.toHaveBeenCalled();
  });
});
