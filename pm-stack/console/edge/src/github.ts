export interface PullRequestSummary {
  number: number;
  title: string;
  draft: boolean;
  url: string;
  created: string;
  author: string;
  branch: string;
}

/** Read the same bounded open-PR view the laptop console presents. */
export async function readOpenPullRequests(
  repo: string,
  token: string,
  fetchImpl: typeof fetch = fetch,
): Promise<PullRequestSummary[]> {
  const response = await fetchImpl(`https://api.github.com/repos/${repo}/pulls?state=open&per_page=60`, {
    headers: {
      accept: "application/vnd.github+json",
      authorization: `Bearer ${token}`,
      "user-agent": "pm-console-edge",
      "x-github-api-version": "2022-11-28",
    },
    signal: AbortSignal.timeout(20_000),
  });
  if (!response.ok) throw new Error(`GitHub answered http ${response.status}`);
  const pulls = (await response.json()) as {
    number: number;
    title: string;
    draft: boolean;
    html_url: string;
    created_at: string;
    user: { login: string };
    head: { ref: string };
  }[];
  if (!Array.isArray(pulls)) throw new Error("GitHub returned an invalid pull request list");
  return pulls.map((pull) => ({
    number: pull.number,
    title: pull.title,
    draft: pull.draft,
    url: pull.html_url,
    created: pull.created_at.slice(0, 16),
    author: pull.user.login,
    branch: pull.head.ref,
  }));
}
