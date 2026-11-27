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
function fakeGitHub(
  createBehaviour: (installation: number) => Response,
  exchangeBehaviour?: (installation: number) => Response | null,
): {
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
      const exchangeResponse = exchangeBehaviour?.(installation);
      if (exchangeResponse) return exchangeResponse;
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

test("an installation-token permission refusal fails over before create", async () => {
  const { fetchFn, calls } = fakeGitHub(
    () => jsonResponse(201, { number: 12, html_url: "https://github.com/o/r/issues/12" }),
    (installation) =>
      installation === 111 ? jsonResponse(403, { message: "Resource not accessible by integration" }) : null,
  );
  const writer = new AppRotationGitHubWriter(config, fetchFn, () => NOW_MS);
  const created = await writer.createItem({ repo: "o/r", kind: "issue", title: "t", body: "b" });

  assert.equal(created.number, 12);
  assert.deepEqual(
    calls.map((call) => call.url.replace("https://api.github.com", "")),
    ["/app/installations/111/access_tokens", "/app/installations/222/access_tokens", "/repos/o/r/issues"],
  );
  assert.equal(calls[2].authorization, "Bearer installation-token-222");
});

test("a non-failover installation-token exchange failure stops before any write", async () => {
  const { fetchFn, calls } = fakeGitHub(
    () => jsonResponse(201, { number: 12, html_url: "https://github.com/o/r/issues/12" }),
    () => jsonResponse(500, { message: "GitHub is unavailable" }),
  );
  const writer = new AppRotationGitHubWriter(config, fetchFn, () => NOW_MS);

  await assert.rejects(
    () => writer.createItem({ repo: "o/r", kind: "issue", title: "t", body: "b" }),
    (error: unknown) => {
      assert.equal((error as { code?: string }).code, "github_token_exchange_failed");
      return true;
    },
  );
  assert.equal(calls.length, 1);
  assert.ok(calls[0].url.endsWith("/app/installations/111/access_tokens"));
});

test("a malformed installation-token response stops before any write", async () => {
  const { fetchFn, calls } = fakeGitHub(
    () => jsonResponse(201, { number: 12, html_url: "https://github.com/o/r/issues/12" }),
    () => jsonResponse(201, { token: "", expires_at: new Date(NOW_MS + 3600_000).toISOString() }),
  );
  const writer = new AppRotationGitHubWriter(config, fetchFn, () => NOW_MS);

  await assert.rejects(
    () => writer.createItem({ repo: "o/r", kind: "issue", title: "t", body: "b" }),
    (error: unknown) => {
      assert.equal((error as { code?: string }).code, "github_token_exchange_failed");
      return true;
    },
  );
  assert.equal(calls.length, 1);
  assert.ok(calls[0].url.endsWith("/app/installations/111/access_tokens"));
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
  const page = (n: number) => [{ number: n, html_url: `https://github.com/o/r/issues/${n}`, body: `b${n}` }];
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
  assert.deepEqual(
    items.map((i) => i.number),
    [1, 2, 3],
  );
  assert.equal(calls.filter((c) => c.url.includes("/issues?")).length, 3);
});

// --- The project surface (#12591): Projects V2 over GraphQL,
// through the same rotation and the same failover rule. The
// acceptance case: a forced rate-limit mid-flow (the add
// mutation, after the reads landed) fails over to the next
// installation WITHOUT duplicating the write.

interface FakeProjectItem {
  itemId: string;
  number: number;
  values: Record<string, string>;
}

/** A stateful fake GraphQL GitHub: token exchanges succeed per
 * installation; GraphQL calls are answered from a tiny project
 * model unless the supplied behaviour intercepts (per
 * installation + query). successfulAdds counts mutations GitHub
 * actually executed — the no-duplicate-writes oracle. */
function fakeGraphQL(
  behaviour?: (installation: number, query: string, variables: Record<string, unknown>) => Response | null,
): {
  fetchFn: FetchFn;
  items: FakeProjectItem[];
  successfulAdds: () => number;
} {
  const tokenToInstallation = new Map<string, number>();
  const items: FakeProjectItem[] = [{ itemId: "PVTI_gql_1", number: 7, values: { Status: "Todo" } }];
  let adds = 0;
  let itemCounter = 1;
  const projectPayload = () => ({
    data: {
      organization: {
        projectV2: {
          id: "PVT_gql",
          fields: {
            nodes: [
              {
                id: "F_status",
                name: "Status",
                options: [
                  { id: "opt_todo", name: "Todo" },
                  { id: "opt_done", name: "Done" },
                ],
              },
            ],
          },
          items: {
            pageInfo: { hasNextPage: false, endCursor: null },
            nodes: items.map((item) => ({
              id: item.itemId,
              content: {
                __typename: "Issue",
                number: item.number,
                repository: { nameWithOwner: "o/r" },
              },
              fieldValues: {
                nodes: Object.entries(item.values).map(([field, name]) => ({
                  name,
                  field: { name: field },
                })),
              },
            })),
          },
        },
      },
      user: null,
    },
  });
  const fetchFn: FetchFn = async (url, init) => {
    const headers = new Headers(init.headers);
    const exchange = /\/app\/installations\/(\d+)\/access_tokens$/.exec(url);
    if (exchange) {
      const installation = Number(exchange[1]);
      tokenToInstallation.set(`installation-token-${installation}`, installation);
      return jsonResponse(201, {
        token: `installation-token-${installation}`,
        expires_at: new Date(NOW_MS + 3600_000).toISOString(),
      });
    }
    const token = (headers.get("authorization") ?? "").replace(/^Bearer /, "");
    const installation = tokenToInstallation.get(token);
    assert.ok(installation, `GraphQL called with an unexchanged token`);
    const body = JSON.parse(String(init.body)) as {
      query: string;
      variables: Record<string, unknown>;
    };
    const intercepted = behaviour?.(installation, body.query, body.variables);
    if (intercepted) return intercepted;
    if (body.query.includes("addProjectV2ItemById")) {
      adds += 1;
      itemCounter += 1;
      const itemId = `PVTI_gql_${itemCounter}`;
      const number = Number(String(body.variables["contentId"]).replace("I_", ""));
      items.push({ itemId, number, values: {} });
      return jsonResponse(200, { data: { addProjectV2ItemById: { item: { id: itemId } } } });
    }
    if (body.query.includes("updateProjectV2ItemFieldValue")) {
      const item = items.find((i) => i.itemId === body.variables["itemId"]);
      if (item) {
        item.values["Status"] = body.variables["optionId"] === "opt_done" ? "Done" : "Todo";
      }
      return jsonResponse(200, {
        data: { updateProjectV2ItemFieldValue: { projectV2Item: { id: body.variables["itemId"] } } },
      });
    }
    if (body.query.includes("projectV2")) {
      return jsonResponse(200, projectPayload());
    }
    if (body.query.includes("repository(")) {
      const number = Number(body.variables["number"]);
      return jsonResponse(200, {
        data: {
          repository: {
            issue: number === 42 ? { id: "I_42" } : null,
            pullRequest: null,
          },
        },
      });
    }
    throw new Error(`fake GraphQL: unhandled query ${body.query.slice(0, 60)}`);
  };
  return { fetchFn, items, successfulAdds: () => adds };
}

test("the project surface reads state and resolves content over GraphQL", async () => {
  const { fetchFn } = fakeGraphQL();
  const surface = new AppRotationGitHubWriter(config, fetchFn, () => NOW_MS);
  const state = await surface.getProjectState("o", 3);
  assert.equal(state?.projectId, "PVT_gql");
  assert.deepEqual(state?.fields, [
    {
      id: "F_status",
      name: "Status",
      options: [
        { id: "opt_todo", name: "Todo" },
        { id: "opt_done", name: "Done" },
      ],
    },
  ]);
  assert.deepEqual(state?.items, [
    { itemId: "PVTI_gql_1", repo: "o/r", kind: "issue", number: 7, values: { Status: "Todo" } },
  ]);
  assert.equal(await surface.resolveContentId("o/r", "issue", 42), "I_42");
  assert.equal(await surface.resolveContentId("o/r", "issue", 99), null);
});

test("a forced rate-limit mid-flow fails over without duplicate writes", async () => {
  // Installation 111 serves the reads, then rate-limits the add
  // mutation — the middle of the flow. The rotation fails the
  // add over to 222, which executes it exactly once.
  const { fetchFn, items, successfulAdds } = fakeGraphQL((installation, query) => {
    if (installation === 111 && query.includes("addProjectV2ItemById")) {
      return jsonResponse(403, { message: "API rate limit exceeded" }, { "x-ratelimit-remaining": "0" });
    }
    return null;
  });
  const surface = new AppRotationGitHubWriter(config, fetchFn, () => NOW_MS);
  const state = await surface.getProjectState("o", 3);
  assert.equal(state?.items.length, 1);
  const contentId = await surface.resolveContentId("o/r", "issue", 42);
  const { itemId } = await surface.addProjectItem(state!.projectId, contentId!);
  await surface.setProjectItemSingleSelect(state!.projectId, itemId, "F_status", "opt_todo");

  assert.equal(successfulAdds(), 1, "the add mutation executed exactly once");
  assert.equal(items.filter((i) => i.number === 42).length, 1, "exactly one project item for #42");
  assert.equal(items.find((i) => i.number === 42)?.values["Status"], "Todo");
});

test("a GraphQL rate-limit errors payload at HTTP 200 also fails over", async () => {
  const { fetchFn } = fakeGraphQL((installation, query) => {
    if (installation === 111 && query.includes("projectV2")) {
      return jsonResponse(200, { errors: [{ message: "API rate limit exceeded for user ID 1" }] });
    }
    return null;
  });
  const surface = new AppRotationGitHubWriter(config, fetchFn, () => NOW_MS);
  const state = await surface.getProjectState("o", 3);
  assert.equal(state?.projectId, "PVT_gql");
});

test("a non-failover GraphQL failure surfaces value-free", async () => {
  const { fetchFn } = fakeGraphQL((_installation, query) => {
    if (query.includes("projectV2")) {
      return jsonResponse(200, {
        errors: [{ message: "Boom involving node PVT_secret_123 and token hunter2" }],
      });
    }
    return null;
  });
  const surface = new AppRotationGitHubWriter(config, fetchFn, () => NOW_MS);
  await assert.rejects(
    () => surface.getProjectState("o", 3),
    (e: unknown) => {
      assert.equal((e as { code?: string }).code, "github_project_failed");
      assert.ok(!(e as Error).message.includes("PVT_secret_123"), "provider values never surface");
      assert.ok(!(e as Error).message.includes("hunter2"), "provider values never surface");
      return true;
    },
  );
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
