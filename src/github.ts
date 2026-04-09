import { Octokit } from "@octokit/rest";

export const SENTINEL = "<!-- agent-rules-report -->";

export interface ParsedPrUrl {
  owner: string;
  repo: string;
  prNumber: number;
}

export function parsePrUrl(prUrl: string): ParsedPrUrl {
  const m = prUrl.trim().replace(/\/$/, "").match(
    /https:\/\/github\.com\/([^/]+)\/([^/]+)\/pull\/(\d+)/
  );
  if (!m) {
    throw new Error(
      `Cannot parse GitHub PR URL: ${prUrl}\nExpected format: https://github.com/owner/repo/pull/N`
    );
  }
  return { owner: m[1]!, repo: m[2]!, prNumber: parseInt(m[3]!, 10) };
}

export async function postPrComment(
  prUrl: string,
  body: string,
  token: string,
  updateExisting: boolean = true
): Promise<void> {
  const { owner, repo, prNumber } = parsePrUrl(prUrl);

  const octokit = new Octokit({ auth: token });

  if (updateExisting) {
    const comments = await listAllComments(octokit, owner, repo, prNumber);
    const existing = comments.find((c) => c.body?.includes(SENTINEL));
    if (existing) {
      await octokit.issues.updateComment({
        owner,
        repo,
        comment_id: existing.id,
        body,
      });
      return;
    }
  }

  await octokit.issues.createComment({ owner, repo, issue_number: prNumber, body });
}

async function listAllComments(
  octokit: Octokit,
  owner: string,
  repo: string,
  prNumber: number
): Promise<Array<{ id: number; body?: string }>> {
  const comments: Array<{ id: number; body?: string }> = [];
  let page = 1;
  while (true) {
    const { data } = await octokit.issues.listComments({
      owner,
      repo,
      issue_number: prNumber,
      per_page: 100,
      page,
    });
    comments.push(...data);
    if (data.length < 100) break;
    page++;
  }
  return comments;
}
