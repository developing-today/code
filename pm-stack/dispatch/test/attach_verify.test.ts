// Issue #3007: pm-dispatch must verify the issue_number attach target
// before enqueueing. A nonexistent number, a PR number, or a closed
// issue used to attach "successfully" (201 + task enqueued against a
// bogus target). These tests stub the GitHub lookup/client — no network.
import { describe, expect, test } from "bun:test";
import { generateKeyPairSync } from "node:crypto";
import { verifyAttachTarget, type AttachTargetInfo } from "../src/server.ts";
import { GitHubClient, isNotFoundError } from "../src/github.ts";

const REPO = "developing-today/code";
const found = (partial: Partial<AttachTargetInfo>): (() => Promise<AttachTargetInfo | null>) =>
  async () => ({ state: "open", isPullRequest: false, ...partial });

describe("verifyAttachTarget (#3007)", () => {
  test("nonexistent number rejects 404 (invalid target, not a lookup failure)", async () => {
    const out = await verifyAttachTarget({
      appInstalled: true,
      repo: REPO,
      issueNumber: 999999,
      lookup: async () => null,
    });
    expect(out.outcome).toBe("reject");
    if (out.outcome === "reject") {
      expect(out.status).toBe(404);
      expect(out.error).toContain("nothing was enqueued");
    }
  });

  test("PR number rejects 422", async () => {
    const out = await verifyAttachTarget({
      appInstalled: true,
      repo: REPO,
      issueNumber: 3143,
      lookup: found({ isPullRequest: true }),
    });
    expect(out.outcome).toBe("reject");
    if (out.outcome === "reject") {
      expect(out.status).toBe(422);
      expect(out.error).toContain("pull request");
    }
  });

  test("closed issue rejects 422", async () => {
    const out = await verifyAttachTarget({
      appInstalled: true,
      repo: REPO,
      issueNumber: 123,
      lookup: found({ state: "closed" }),
    });
    expect(out.outcome).toBe("reject");
    if (out.outcome === "reject") {
      expect(out.status).toBe(422);
      expect(out.error).toContain("closed");
    }
  });

  test("open issue is the happy path: verified", async () => {
    const out = await verifyAttachTarget({
      appInstalled: true,
      repo: REPO,
      issueNumber: 3007,
      lookup: found({}),
    });
    expect(out.outcome).toBe("verified");
  });

  test("GitHub lookup failure is a retryable 502, distinct from an invalid target", async () => {
    const out = await verifyAttachTarget({
      appInstalled: true,
      repo: REPO,
      issueNumber: 3007,
      lookup: async () => {
        throw new Error("github issue lookup failed (HTTP 503)");
      },
    });
    expect(out.outcome).toBe("lookup_failed");
    if (out.outcome === "lookup_failed") {
      expect(out.status).toBe(502);
      expect(out.error).toContain("nothing was enqueued");
    }
  });

  test("app-not-installed repo: unverified is explicit and the lookup is never attempted", async () => {
    let lookups = 0;
    const out = await verifyAttachTarget({
      appInstalled: false,
      repo: "developing-today/zettel",
      issueNumber: 5,
      lookup: async () => {
        lookups++;
        return { state: "open", isPullRequest: false };
      },
    });
    expect(out.outcome).toBe("unverified");
    if (out.outcome === "unverified") expect(out.warning).toContain("NOT verified");
    expect(lookups).toBe(0);
  });
});

// --- GitHubClient.getIssue with a stubbed fetch (app route) -----------

const { privateKey } = generateKeyPairSync("rsa", { modulusLength: 2048 });
const pem = privateKey.export({ format: "pem", type: "pkcs8" }) as string;

const realFetch = globalThis.fetch;

function stubGitHub(issueResponse: () => Response) {
  globalThis.fetch = (async (input: unknown) => {
    const url = String(input);
    if (url.endsWith(`/repos/${REPO}/installation`)) {
      return new Response(JSON.stringify({ id: 42 }), { status: 200 });
    }
    if (url.includes("/app/installations/42/access_tokens")) {
      return new Response(
        JSON.stringify({ token: "test-token", expires_at: new Date(Date.now() + 3_600_000).toISOString() }),
        { status: 201 },
      );
    }
    if (url.includes(`/repos/${REPO}/issues/`)) return issueResponse();
    throw new Error(`unexpected fetch in test: ${url}`);
  }) as typeof fetch;
}

describe("GitHubClient.getIssue with stubbed GitHub (#3007)", () => {
  test("open issue parses; pull_request marker absent", async () => {
    stubGitHub(
      () =>
        new Response(JSON.stringify({ number: 3007, state: "open", html_url: `https://github.com/${REPO}/issues/3007` }), {
          status: 200,
        }),
    );
    try {
      const client = new GitHubClient("12345", pem);
      const info = await client.getIssue(REPO, 3007);
      expect(info).not.toBeNull();
      expect(info!.state).toBe("open");
      expect(info!.isPullRequest).toBe(false);
    } finally {
      globalThis.fetch = realFetch;
    }
  });

  test("PR number parses with isPullRequest true", async () => {
    stubGitHub(
      () =>
        new Response(
          JSON.stringify({
            number: 3143,
            state: "open",
            html_url: `https://github.com/${REPO}/pull/3143`,
            pull_request: { url: "https://api.github.com/x" },
          }),
          { status: 200 },
        ),
    );
    try {
      const client = new GitHubClient("12345", pem);
      const info = await client.getIssue(REPO, 3143);
      expect(info).not.toBeNull();
      expect(info!.isPullRequest).toBe(true);
    } finally {
      globalThis.fetch = realFetch;
    }
  });

  test("404 returns null (definitive not-found, no throw)", async () => {
    stubGitHub(() => new Response(JSON.stringify({ message: "Not Found" }), { status: 404 }));
    try {
      const client = new GitHubClient("12345", pem);
      expect(await client.getIssue(REPO, 999999)).toBeNull();
    } finally {
      globalThis.fetch = realFetch;
    }
  });
});

describe("isNotFoundError (#3007, Codex P2 on PR #3149)", () => {
  test("an HTTP 404 status in the gh error is a not-found", () => {
    expect(isNotFoundError("gh api repos/x/issues/9 failed: gh: Not Found (HTTP 404)")).toBe(true);
  });

  test("issue NUMBER 404 in the echoed path is not a not-found", () => {
    expect(isNotFoundError("gh api repos/x/issues/404 failed: gh: Forbidden (HTTP 403)")).toBe(false);
    expect(isNotFoundError("gh api repos/x/issues/404 failed: gh: rate limit (HTTP 429)")).toBe(false);
    expect(isNotFoundError("gh api repos/x/issues/404 failed: gh: server error (HTTP 500)")).toBe(false);
  });
});
