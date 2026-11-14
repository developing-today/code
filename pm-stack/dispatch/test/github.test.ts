// Issue #7101: pm-dispatch's GitHub App client must validate every API
// response body before use. A wrong-shape 2xx used to be shape-trusted:
// an installation {} minted against /installations/undefined, a token {}
// was sent as "Bearer undefined", a create {} returned a phantom issue
// as success, a lookup {} fabricated an issue state, and null bodies
// escaped as raw TypeErrors (the create one then duplicated the issue
// via the gh fallback). These tests stub fetch in the style of
// test/attach_verify.test.ts. The repo slug is a nonexistent one, so
// the gh CLI fallback fails deterministically (and can never create a
// real issue) in any environment.
//
// Oracle: the #7101 acceptance — every malformed body raises GitHubError
// naming the endpoint and the malformed field; a malformed 2xx create
// never triggers the gh fallback create; malformed installation/token
// replies still fall back with the reason preserved; well-formed
// controls are unchanged.
import { describe, expect, test } from "bun:test";
import { generateKeyPairSync } from "node:crypto";
import { GitHubClient, GitHubError } from "../src/github.ts";

const REPO = "pm-dispatch-7101/nonexistent"; // a slug no gh route can succeed on: the CLI fallback fails deterministically and can never create anything, in any environment
const { privateKey } = generateKeyPairSync("rsa", { modulusLength: 2048 });
const pem = privateKey.export({ format: "pem", type: "pkcs8" }) as string;
const realFetch = globalThis.fetch;

// --- fetch stub ----------------------------------------------------------
interface Bodies {
  installation?: unknown;
  token?: unknown;
  create?: unknown;
  lookup?: unknown;
}
const FUTURE = new Date(Date.now() + 3_600_000).toISOString();
const GOOD: Required<Bodies> = {
  installation: { id: 42 },
  token: { token: "test-token", expires_at: FUTURE },
  create: { number: 123, html_url: `https://github.com/${REPO}/issues/123` },
  lookup: { number: 3007, state: "open", html_url: `https://github.com/${REPO}/issues/3007` },
};
let seenUrls: string[] = [];
let seenAuth: string[] = [];

function stub(bodies: Bodies) {
  const b: Required<Bodies> = { ...GOOD, ...bodies };
  seenUrls = [];
  seenAuth = [];
  globalThis.fetch = (async (input: unknown, init?: { headers?: Record<string, string> }) => {
    const url = String(input);
    seenUrls.push(url);
    const auth = init?.headers?.Authorization;
    if (typeof auth === "string") seenAuth.push(auth);
    const reply = (body: unknown, status = 200) =>
      new Response(body === undefined ? "null" : JSON.stringify(body), { status });
    if (url.endsWith(`/repos/${REPO}/installation`)) return reply(b.installation);
    if (url.includes("/app/installations/")) return reply(b.token, 201);
    if (url.endsWith(`/repos/${REPO}/issues`)) return reply(b.create, 201);
    if (url.includes(`/repos/${REPO}/issues/`)) return reply(b.lookup);
    throw new Error(`unexpected fetch in test: ${url}`);
  }) as typeof fetch;
}

async function withStub<T>(bodies: Bodies, fn: () => Promise<T>): Promise<T> {
  stub(bodies);
  try {
    return await fn();
  } finally {
    globalThis.fetch = realFetch;
  }
}

const client = () => new GitHubClient("12345", pem);

describe("GitHubClient response-shape validation (#7101)", () => {
  test("well-formed control: create returns the real issue ref", async () => {
    const out = await withStub({}, () => client().createIssue(REPO, "t", "b", []));
    expect(out).toEqual({ number: 123, url: `https://github.com/${REPO}/issues/123` });
    expect(seenAuth.some((a) => a === "Bearer test-token")).toBe(true);
  });

  test("well-formed control: lookup returns the issue info", async () => {
    const info = await withStub({}, () => client().getIssue(REPO, 3007));
    expect(info).toEqual({
      number: 3007,
      state: "open",
      isPullRequest: false,
      url: `https://github.com/${REPO}/issues/3007`,
    });
  });

  test("installation {} raises GitHubError naming the endpoint and 'id'; no mint to undefined", async () => {
    const err = await withStub({ installation: {} }, () =>
      client()
        .createIssue(REPO, "t", "b", [])
        .then(
          () => null,
          (e: unknown) => e,
        ),
    );
    // installation/token malformation may fall back to gh; the fake gh
    // fails, so the combined error must preserve the malformed reason.
    expect(err).toBeInstanceOf(GitHubError);
    expect((err as Error).message).toContain("installation");
    expect((err as Error).message).toContain("'id'");
    expect(seenUrls.some((u) => u.includes("undefined"))).toBe(false);
  });

  test("installation null body raises GitHubError, not a raw TypeError", async () => {
    const err = await withStub({ installation: undefined }, () =>
      client()
        .createIssue(REPO, "t", "b", [])
        .then(
          () => null,
          (e: unknown) => e,
        ),
    );
    expect(err).toBeInstanceOf(GitHubError);
    expect(err).not.toBeInstanceOf(TypeError);
    expect((err as Error).message).toContain("malformed");
  });

  test("token {} raises naming 'token'; no Bearer undefined is ever sent", async () => {
    const err = await withStub({ token: {} }, () =>
      client()
        .createIssue(REPO, "t", "b", [])
        .then(
          () => null,
          (e: unknown) => e,
        ),
    );
    expect(err).toBeInstanceOf(GitHubError);
    expect((err as Error).message).toContain("'token'");
    expect(seenAuth.some((a) => a.includes("undefined"))).toBe(false);
  });

  test("token numeric raises naming 'token'", async () => {
    const err = await withStub({ token: { token: 123, expires_at: FUTURE } }, () =>
      client()
        .createIssue(REPO, "t", "b", [])
        .then(
          () => null,
          (e: unknown) => e,
        ),
    );
    expect(err).toBeInstanceOf(GitHubError);
    expect((err as Error).message).toContain("'token'");
    expect(seenAuth.some((a) => a === "Bearer 123")).toBe(false);
    expect(seenUrls.some((u) => u.endsWith(`/repos/${REPO}/issues`))).toBe(false);
  });

  test("token with unparseable expires_at raises naming 'expires_at'", async () => {
    const err = await withStub({ token: { token: "test-token", expires_at: "not-a-date" } }, () =>
      client()
        .createIssue(REPO, "t", "b", [])
        .then(
          () => null,
          (e: unknown) => e,
        ),
    );
    expect(err).toBeInstanceOf(GitHubError);
    expect((err as Error).message).toContain("'expires_at'");
  });

  test("create {} surfaces a malformed-body GitHubError and does NOT fall back to a gh create", async () => {
    const err = await withStub({ create: {} }, () =>
      client()
        .createIssue(REPO, "t", "b", [])
        .then(
          () => null,
          (e: unknown) => e,
        ),
    );
    // A gh fallback would wrap the failure in a combined "failed via
    // app ... and via gh" GitHubError; the bare malformed-body error
    // (no "via gh" text) proves the fallback never ran.
    expect(err).toBeInstanceOf(GitHubError);
    expect((err as Error).message).toContain("malformed");
    expect((err as Error).message).toContain("'number'");
    expect((err as Error).message).not.toContain("via gh");
  });

  test("create with wrong-typed fields surfaces a malformed-body GitHubError, no gh create", async () => {
    const err = await withStub({ create: { number: "abc", html_url: 42 } }, () =>
      client()
        .createIssue(REPO, "t", "b", [])
        .then(
          () => null,
          (e: unknown) => e,
        ),
    );
    expect(err).toBeInstanceOf(GitHubError);
    expect((err as Error).message).toContain("malformed");
  });

  test("create null body surfaces a malformed-body GitHubError, no gh create, no TypeError", async () => {
    const err = await withStub({ create: undefined }, () =>
      client()
        .createIssue(REPO, "t", "b", [])
        .then(
          () => null,
          (e: unknown) => e,
        ),
    );
    expect(err).toBeInstanceOf(GitHubError);
    expect((err as Error).message).toContain("malformed");
    expect(err).not.toBeInstanceOf(TypeError);
  });

  test("lookup {} does not fabricate an issue", async () => {
    const out = await withStub({ lookup: {} }, () =>
      client()
        .getIssue(REPO, 3007)
        .then(
          (v) => ({ v }),
          (e: unknown) => ({ e }),
        ),
    );
    // Never a fabricated IssueInfo and never a raw TypeError: either the
    // gh route's definitive 404 governs (null) or the combined
    // GitHubError preserves the malformed-body reason.
    if ("v" in out) {
      expect(out.v).toBeNull();
    } else {
      expect(out.e).toBeInstanceOf(GitHubError);
      expect((out.e as Error).message).toContain("malformed");
      expect((out.e as Error).message).toContain("'number'");
    }
  });

  test("lookup null body never escapes as a raw TypeError", async () => {
    const out = await withStub({ lookup: undefined }, () =>
      client()
        .getIssue(REPO, 3007)
        .then(
          (v) => ({ v }),
          (e: unknown) => ({ e }),
        ),
    );
    if ("v" in out) {
      expect(out.v).toBeNull();
    } else {
      expect(out.e).toBeInstanceOf(GitHubError);
      expect(out.e).not.toBeInstanceOf(TypeError);
      expect((out.e as Error).message).toContain("malformed");
    }
  });
});
