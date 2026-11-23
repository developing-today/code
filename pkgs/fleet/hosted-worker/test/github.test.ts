// GitHub rotation writer tests (#12585). Oracles: the native fleet
// gh rotation rule (#8887, design §1/§2) — installation tokens are
// tried in order; rate-limit and permission refusals fail over to
// the next installation; other failures surface — and the GitHub App
// JWT contract (RS256, iss = App id, ≤10-minute lifetime). All
// traffic is a fake fetch: no test touches the network.

import assert from "node:assert/strict";
import { test } from "node:test";
import type { GitHubAppConfig } from "../src/config.ts";
import { AppRotationGitHubWriter, type FetchFn } from "../src/github.ts";

const NOW_MS = Date.parse("2026-11-22T20:00:00.000Z");

const appKeyPair = await crypto.subtle.generateKey(
  {
    name: "RSASSA-PKCS1-v1_5",
    modulusLength: 2048,
    publicExponent: new Uint8Array([1, 0, 1]),
    hash: "SHA-256",
  },
  true,
  ["sign", "verify"],
);
const appPkcs8 = new Uint8Array(await crypto.subtle.exportKey("pkcs8", appKeyPair.privateKey));

const config: GitHubAppConfig = {
  appId: "123456",
  privateKeyPkcs8: appPkcs8,
  installationIds: [111, 222],
};

interface RecordedCall {
  url: string;
  authorization: string | null;
}

function jsonResponse(status: number, body: unknown, headers: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", ...headers },
  });
}

/** A fake GitHub: token exchanges succeed per installation; creates
 * are answered by the supplied per-installation behaviour. */
function fakeGitHub(createBehaviour: (installation: number) => Response): {
  fetchFn: FetchFn;
  calls: RecordedCall[];
} {
  const calls: RecordedCall[] = [];
  const tokenToInstallation = new Map<string, number>();
  const fetchFn: FetchFn = async (url, init) => {
    const headers = new Headers(init.headers);
    calls.push({ url, authorization: headers.get("authorization") });
    const exchange = /\/app\/installations\/(\d+)\/access_tokens$/.exec(url);
    if (exchange) {
      const installation = Number(exchange[1]);
      const token = `installation-token-${installation}`;
      tokenToInstallation.set(token, installation);
      return jsonResponse(201, {
        token,
        expires_at: new Date(NOW_MS + 3600_000).toISOString(),
      });
    }
    const auth = headers.get("authorization") ?? "";
    const token = auth.replace(/^Bearer /, "");
    const installation = tokenToInstallation.get(token);
    assert.ok(installation, `create called with an unexchanged token: ${auth}`);
    return createBehaviour(installation);
  };
  return { fetchFn, calls };
}

test("the App JWT is RS256 with iss = App id and a ≤10-minute lifetime, and verifies", async () => {
  const { fetchFn } = fakeGitHub(() => jsonResponse(201, { number: 1, html_url: "https://x/1" }));
  const writer = new AppRotationGitHubWriter(config, fetchFn, () => NOW_MS);
  const jwt = await writer.mintAppJwt();
  const [headerPart, payloadPart, signaturePart] = jwt.split(".");
  const decode = (part: string) =>
    JSON.parse(atob(part.replace(/-/g, "+").replace(/_/g, "/"))) as Record<string, number | string>;
  assert.equal(decode(headerPart).alg, "RS256");
  const payload = decode(payloadPart);
  assert.equal(payload.iss, "123456");
  assert.ok(Number(payload.exp) - Number(payload.iat) <= 600, "App JWT lifetime must be ≤ 10 minutes");

  const b64ToBytes = (text: string) => {
    const bin = atob(text.replace(/-/g, "+").replace(/_/g, "/"));
    return Uint8Array.from(bin, (c) => c.charCodeAt(0));
  };
  const ok = await crypto.subtle.verify(
    "RSASSA-PKCS1-v1_5",
    appKeyPair.publicKey,
    b64ToBytes(signaturePart),
    new TextEncoder().encode(`${headerPart}.${payloadPart}`),
  );
  assert.equal(ok, true);
});

test("a create succeeds on the first installation when it answers 201", async () => {
  const { fetchFn, calls } = fakeGitHub(() =>
    jsonResponse(201, { number: 7, html_url: "https://github.com/o/r/issues/7" }),
  );
  const writer = new AppRotationGitHubWriter(config, fetchFn, () => NOW_MS);
  const created = await writer.createItem({ repo: "o/r", kind: "issue", title: "t", body: "b" });
  assert.deepEqual(created, { number: 7, url: "https://github.com/o/r/issues/7" });
  // One exchange (installation 111), one create; installation 222 untouched.
  assert.equal(calls.filter((c) => c.url.includes("access_tokens")).length, 1);
  assert.ok(calls[0].url.includes("/app/installations/111/access_tokens"));
});

test("a rate-limit refusal fails over to the next installation", async () => {
  const { fetchFn } = fakeGitHub((installation) =>
    installation === 111
      ? jsonResponse(403, { message: "API rate limit exceeded" }, { "x-ratelimit-remaining": "0" })
      : jsonResponse(201, { number: 9, html_url: "https://github.com/o/r/issues/9" }),
  );
  const writer = new AppRotationGitHubWriter(config, fetchFn, () => NOW_MS);
  const created = await writer.createItem({ repo: "o/r", kind: "issue", title: "t", body: "b" });
  assert.equal(created.number, 9);
});

test("a permission refusal (resource not accessible) fails over", async () => {
  const { fetchFn } = fakeGitHub((installation) =>
    installation === 111
      ? jsonResponse(403, { message: "Resource not accessible by integration" })
      : jsonResponse(201, { number: 3, html_url: "https://github.com/o/r/pull/3" }),
  );
  const writer = new AppRotationGitHubWriter(config, fetchFn, () => NOW_MS);
  const created = await writer.createItem({
    repo: "o/r",
    kind: "pr",
    title: "t",
    body: "b",
    head: "feat",
    base: "main",
  });
  assert.equal(created.number, 3);
});

test("a non-failover failure surfaces instead of rotating", async () => {
  const { fetchFn, calls } = fakeGitHub(() => jsonResponse(422, { message: "Validation Failed" }));
  const writer = new AppRotationGitHubWriter(config, fetchFn, () => NOW_MS);
  await assert.rejects(
    () => writer.createItem({ repo: "o/r", kind: "issue", title: "t", body: "b" }),
    (e: unknown) => {
      assert.equal((e as { code?: string }).code, "github_create_failed");
      return true;
    },
  );
  // Only installation 111 was tried.
  assert.equal(calls.filter((c) => c.url.includes("/repos/")).length, 1);
});

test("when every installation refuses with failover cases, the create fails closed", async () => {
  const { fetchFn } = fakeGitHub(() => jsonResponse(429, { message: "secondary rate limit" }));
  const writer = new AppRotationGitHubWriter(config, fetchFn, () => NOW_MS);
  await assert.rejects(
    () => writer.createItem({ repo: "o/r", kind: "issue", title: "t", body: "b" }),
    (e: unknown) => {
      assert.equal((e as { code?: string }).code, "github_create_failed");
      assert.match((e as Error).message, /every installation/);
      return true;
    },
  );
});

test("installation tokens are cached across creates within their lifetime", async () => {
  const { fetchFn, calls } = fakeGitHub(() =>
    jsonResponse(201, { number: 1, html_url: "https://github.com/o/r/issues/1" }),
  );
  const writer = new AppRotationGitHubWriter(config, fetchFn, () => NOW_MS);
  await writer.createItem({ repo: "o/r", kind: "issue", title: "a", body: "b" });
  await writer.createItem({ repo: "o/r", kind: "issue", title: "c", body: "d" });
  assert.equal(calls.filter((c) => c.url.includes("access_tokens")).length, 1);
});

// --- The read side (#12587): the same rotation class serves the
// query surface's GitHubItemSource over the same tokens and the
// same failover rule.

/** A fake GitHub for reads: token exchanges succeed per
 * installation; GETs are answered by the supplied behaviour, which
 * sees the installation whose token the call carries. */
function fakeGitHubReads(readBehaviour: (installation: number, url: string) => Response): {
  fetchFn: FetchFn;
  calls: RecordedCall[];
} {
  const calls: RecordedCall[] = [];
  const tokenToInstallation = new Map<string, number>();
  const fetchFn: FetchFn = async (url, init) => {
    const headers = new Headers(init.headers);
    calls.push({ url, authorization: headers.get("authorization") });
    const exchange = /\/app\/installations\/(\d+)\/access_tokens$/.exec(url);
    if (exchange) {
      const installation = Number(exchange[1]);
      const token = `installation-token-${installation}`;
      tokenToInstallation.set(token, installation);
      return jsonResponse(201, {
        token,
        expires_at: new Date(NOW_MS + 3600_000).toISOString(),
      });
    }
    const auth = headers.get("authorization") ?? "";
    const token = auth.replace(/^Bearer /, "");
    const installation = tokenToInstallation.get(token);
    assert.ok(installation, `read called with an unexchanged token: ${auth}`);
    return readBehaviour(installation, url);
  };
  return { fetchFn, calls };
}

test("getItem reads an issue and a PR through the rotation, kinds from the pull_request marker", async () => {
  const { fetchFn } = fakeGitHubReads((_installation, url) => {
    if (url.endsWith("/issues/7")) {
      return jsonResponse(200, { number: 7, html_url: "https://github.com/o/r/issues/7", body: "issue body" });
    }
    if (url.endsWith("/issues/8")) {
      return jsonResponse(200, {
        number: 8,
        html_url: "https://github.com/o/r/pull/8",
        body: "pr body",
        pull_request: { url: "https://api.github.com/o/r/pulls/8" },
      });
    }
    return jsonResponse(404, { message: "Not Found" });
  });
  const reader = new AppRotationGitHubWriter(config, fetchFn, () => NOW_MS);
  assert.deepEqual(await reader.getItem("o/r", "issue", 7), {
    repo: "o/r",
    kind: "issue",
    number: 7,
    url: "https://github.com/o/r/issues/7",
    body: "issue body",
  });
  assert.equal((await reader.getItem("o/r", "pr", 8))?.body, "pr body");
  // Kind is checked against the marker: #8 is not an issue.
  assert.equal(await reader.getItem("o/r", "issue", 8), null);
  // A GitHub 404 is a null, not an error.
  assert.equal(await reader.getItem("o/r", "issue", 99), null);
});

test("listItems follows the next link and stops at the page cap", async () => {
  const page = (n: number) => [
    { number: n, html_url: `https://github.com/o/r/issues/${n}`, body: `b${n}` },
  ];
  const { fetchFn, calls } = fakeGitHubReads((_installation, url) => {
    if (url.includes("page=2")) {
      return jsonResponse(200, page(2), {
        link: `<https://api.github.com/repos/o/r/issues?state=all&per_page=100&page=3>; rel="next"`,
      });
    }
    if (url.includes("page=3")) {
      return jsonResponse(200, page(3));
    }
    return jsonResponse(200, page(1), {
      link: `<https://api.github.com/repos/o/r/issues?state=all&per_page=100&page=2>; rel="next"`,
    });
  });
  const reader = new AppRotationGitHubWriter(config, fetchFn, () => NOW_MS);
  const items = await reader.listItems("o/r");
  assert.deepEqual(items.map((i) => i.number), [1, 2, 3]);
  assert.equal(calls.filter((c) => c.url.includes("/issues?")).length, 3);
});

test("reads fail over on rate-limit refusals and surface real failures", async () => {
  const { fetchFn } = fakeGitHubReads((installation, url) => {
    if (installation === 111) return jsonResponse(429, { message: "secondary rate limit" });
    return jsonResponse(200, { number: 7, html_url: "https://github.com/o/r/issues/7", body: "b" });
  });
  const reader = new AppRotationGitHubWriter(config, fetchFn, () => NOW_MS);
  assert.equal((await reader.getItem("o/r", "issue", 7))?.body, "b");

  const failing = fakeGitHubReads(() => jsonResponse(500, { message: "boom" }));
  const reader2 = new AppRotationGitHubWriter(config, failing.fetchFn, () => NOW_MS);
  await assert.rejects(
    () => reader2.getItem("o/r", "issue", 7),
    (e: unknown) => {
      assert.equal((e as { code?: string }).code, "github_read_failed");
      return true;
    },
  );
});
