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
