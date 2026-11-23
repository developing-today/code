// Query door tests (#12587). Oracles: the issue's acceptance
// (filters return correct results against a hand-checked sample
// spanning both editions' items; a body block hand-edited after
// stamping shows a divergence finding with the body content
// winning; the collector can consume the surface — a service token
// scoped to the query door) and the governing rules: #9060 §13
// (served from the DB, GitHub fallback for fields not yet
// ingested), #9060 §15 (divergence reported, bodies win on
// content), design §7.1 (the filter set), and the door idioms
// (tenancy-scoped results, unconfigured store fails closed,
// malformed filters refuse rather than matching everything).

import assert from "node:assert/strict";
import { test } from "node:test";
import { sha256Hex, type AuthDeps } from "../src/auth.ts";
import { configFromEnv } from "../src/config.ts";
import { FixtureGitHubWriter } from "../src/github.ts";
import { parseMetadataBlock, renderBodyWithBlock } from "../src/metadata.ts";
import { dispatch } from "../src/router.ts";
import { FixtureSessionRegistry } from "../src/sessions.ts";
import { importStampSigner, signStamp, type Provenance } from "../src/stamp-sig.ts";
import {
  extractKeys,
  FixtureMetadataStore,
  type MetadataRecord,
} from "../src/stamping.ts";
import { AUDIENCE, BINDING_A, BINDING_B, ISSUER, bearerReq, makeHarness } from "./helpers.ts";

const h = await makeHarness();

const REPO = "developing-today/code";
const FIXED_ISO = "2026-11-22T20:00:00.000Z";
const STAMP_KEY_ID = "stamp-test-1";
// This suite's fixture service tokens (concatenated so no
// credential-shaped literal sits in the source).
const QUERY_TOKEN = ["collector", "token", "a", "0123456789abcdef"].join("-");
const STAMP_ONLY_TOKEN = ["stamponly", "token", "a", "0123456789abcdef"].join("-");

const SES_A = `ses_${"a".repeat(32)}`;
const SES_B = `ses_${"b".repeat(32)}`;
const SES_X = `ses_${"c".repeat(32)}`;

function b64url(bytes: Uint8Array): string {
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

const stampKeyPair = await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"]);
const stampPkcs8 = new Uint8Array(await crypto.subtle.exportKey("pkcs8", stampKeyPair.privateKey));
const stampRawPub = new Uint8Array(await crypto.subtle.exportKey("raw", stampKeyPair.publicKey));

const tenancyFixture = JSON.stringify({
  subjects: [
    { issuer: ISSUER, subject: "user-sub-1", binding_id: BINDING_A, active: true },
    { issuer: ISSUER, subject: "user-sub-2", binding_id: BINDING_B, active: true },
  ],
  service_tokens: [
    { sha256: await sha256Hex(QUERY_TOKEN), binding_id: BINDING_A, routes: ["query"], active: true, label: "collector-test" },
    { sha256: await sha256Hex(STAMP_ONLY_TOKEN), binding_id: BINDING_A, routes: ["stamping"], active: true, label: "stamping-only" },
  ],
});

const config = configFromEnv({
  RP_ISSUER: ISSUER,
  RP_AUDIENCE: AUDIENCE,
  TENANCY_FIXTURE_JSON: tenancyFixture,
  STAMP_SIGNING_KEY_PKCS8: b64url(stampPkcs8),
  STAMP_SIGNING_KEY_ID: STAMP_KEY_ID,
  STAMP_VERIFY_KEYS_JSON: JSON.stringify({ [STAMP_KEY_ID]: b64url(stampRawPub) }),
});
assert.ok(config.stamp.signing, "stamp signing material must parse");
const signer = await importStampSigner(config.stamp.signing);

const store = new FixtureMetadataStore();
const writer = new FixtureGitHubWriter();
const deps: AuthDeps = {
  config,
  jwksFetcher: h.deps.jwksFetcher,
  sessions: new FixtureSessionRegistry(),
  metadataStore: store,
  githubWriter: writer,
  githubItems: writer,
};

const tokenA = await h.sign(h.rpPayload());
const tokenB = await h.sign(h.rpPayload({ sub: "user-sub-2" }));

// --- The hand-checked sample. Hosted-edition blocks carry the §5
// additive fields (auth_subject, stamp_sig — really signed here);
// the native-edition block carries only the §12 stamp set. Every
// seeded DB row starts with an identical body on GitHub; the
// divergence cases then mutate the writer's copy (the hand-edit).

function bodyText(block: Record<string, unknown>, text: string): string {
  return renderBodyWithBlock({ block: null, before: text, after: "" }, block);
}

async function seed(
  number: number,
  kind: "issue" | "pr",
  bindingId: string,
  block: Record<string, unknown>,
  opts: { db?: boolean; github?: boolean } = {},
): Promise<void> {
  const url = `https://github.com/${REPO}/${kind === "pr" ? "pull" : "issues"}/${number}`;
  if (opts.db !== false) {
    const record: MetadataRecord = {
      repo: REPO,
      kind,
      number,
      url,
      bindingId,
      metadata: block,
      extracted: extractKeys(block),
      insertedAt: FIXED_ISO,
    };
    await store.insert(record);
  }
  if (opts.github !== false) {
    writer.items.push({
      repo: REPO,
      kind,
      title: `sample ${number}`,
      body: bodyText(block, `Sample item ${number}`),
      number,
      url,
    });
  }
}

async function hostedBlock(
  provenance: Partial<Provenance>,
  rest: Record<string, unknown> = {},
): Promise<Record<string, unknown>> {
  const signed = await signStamp(
    {
      queue_id: null,
      scheduled_start: null,
      picked_up_at: null,
      work_started_at: null,
      created_at: FIXED_ISO,
      session_id: null,
      lane: null,
      host: null,
      worktree: null,
      model: null,
      lane_start_issues: [],
      workspace_ref: null,
      auth_subject: BINDING_A,
      stamped_by: "fleet-hosted@0.1.0",
      ...provenance,
    },
    signer,
  );
  return {
    pm_meta: 1,
    direct_parent: null,
    links: [],
    lineage: { design: null, roadmap_item: null, priority: null },
    ...rest,
    provenance: signed,
  };
}

// #101 — hosted issue, lane code-a on amd-6, session SES_A, queue q-1.
await seed(101, "issue", BINDING_A, await hostedBlock(
  { session_id: SES_A, lane: "code-a", host: "amd-6", queue_id: "q-1" },
  {
    direct_parent: { repo: REPO, number: 9081, kind: "design" },
    links: [{ target: { repo: REPO, number: 12583 }, relation: "parent", status: "current" }],
  },
));
// #102 — native-edition PR: §12 provenance only (no auth_subject,
// no stamp_sig), lane code-b, parent is #101, one historical link.
await seed(102, "pr", BINDING_A, {
  pm_meta: 1,
  direct_parent: { repo: REPO, number: 101, kind: "issue" },
  links: [{ target: { repo: REPO, number: 101 }, relation: "child", status: "historical" }],
  lineage: { design: null, roadmap_item: null, priority: null },
  provenance: {
    queue_id: null,
    session_id: SES_B,
    lane: "code-b",
    host: "laptop-2",
    worktree: null,
    picked_up_at: null,
    work_started_at: null,
    created_at: FIXED_ISO,
    model: null,
    lane_start_issues: [],
  },
});
// #103 — hosted issue, same session/lane/host as #101, queue q-2,
// with the link-pair trap: relation "parent" on one link (status
// historical), status "current" on another (relation "blocks").
await seed(103, "issue", BINDING_A, await hostedBlock(
  { session_id: SES_A, lane: "code-a", host: "amd-6", queue_id: "q-2" },
  {
    links: [
      { target: { repo: REPO, number: 500 }, relation: "parent", status: "historical" },
      { target: { repo: REPO, number: 501 }, relation: "blocks", status: "current" },
    ],
  },
));
// #104 — another tenancy's item whose fields match A's favourite
// filters (lane code-a, host amd-6): A must never see it.
await seed(104, "issue", BINDING_B, await hostedBlock(
  { session_id: SES_X, lane: "code-a", host: "amd-6", auth_subject: BINDING_B },
));
// #105 — hosted item GitHub-only (not yet ingested), A's tenancy.
await seed(105, "issue", BINDING_A, await hostedBlock(
  { session_id: SES_A, lane: "code-a", host: "amd-6" },
), { db: false });
// #106 — GitHub-only item stamped for B: A's fallback must skip it.
await seed(106, "issue", BINDING_B, await hostedBlock(
  { session_id: SES_X, lane: "code-a", host: "amd-6", auth_subject: BINDING_B },
), { db: false });
// #107 — divergence case (caller-owned edit): direct_parent changed
// in the body after stamping.
const block107 = await hostedBlock({ session_id: `ses_${"d".repeat(32)}`, lane: "code-d", host: "amd-6" });
await seed(107, "issue", BINDING_A, block107);
// #108 — divergence case (provenance edit): lane rewritten in the
// body after stamping, which also breaks the stamp signature.
const block108 = await hostedBlock({ session_id: `ses_${"e".repeat(32)}`, lane: "code-e", host: "amd-6" });
await seed(108, "issue", BINDING_A, block108);
// #109 — divergence case: the block was removed from the body.
const block109 = await hostedBlock({ session_id: `ses_${"f".repeat(32)}`, lane: "code-f", host: "amd-6" });
await seed(109, "issue", BINDING_A, block109);

function editWriterBody(number: number, body: string): void {
  const item = writer.items.find((i) => i.number === number);
  assert.ok(item, `writer item ${number} must exist`);
  item.body = body;
}

editWriterBody(107, bodyText(
  { ...block107, direct_parent: { repo: REPO, number: 9999, kind: "issue" } },
  "Sample item 107",
));
{
  const edited108 = JSON.parse(JSON.stringify(block108)) as Record<string, unknown>;
  (edited108["provenance"] as Record<string, unknown>)["lane"] = "code-z";
  editWriterBody(108, bodyText(edited108, "Sample item 108"));
}
editWriterBody(109, "Sample item 109 — block removed by hand.");

// --- Query helpers.

interface ResultItem {
  repo: string;
  kind: string;
  number: number;
  metadata: Record<string, unknown>;
  source: string;
  body_checked: boolean;
  divergence: { fields: string[] } | null;
  stamp: { status: string; reason?: string } | null;
}
interface QueryDoc {
  filter: Record<string, unknown>;
  count: number;
  results: ResultItem[];
}

async function getQuery(token: string, params: string, d: AuthDeps = deps): Promise<{ status: number; doc: QueryDoc }> {
  const res = await dispatch(bearerReq(`/v1/query?${params}`, token), d);
  return { status: res.status, doc: (await res.json()) as QueryDoc };
}

async function postQuery(token: string, body: unknown, d: AuthDeps = deps): Promise<{ status: number; doc: QueryDoc & { error?: string } }> {
  const res = await dispatch(
    bearerReq("/v1/query", token, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: typeof body === "string" ? body : JSON.stringify(body),
    }),
    d,
  );
  return { status: res.status, doc: (await res.json()) as QueryDoc & { error?: string } };
}

function numbers(doc: QueryDoc): number[] {
  return doc.results.map((r) => r.number);
}

// --- Filter correctness over the hand-checked sample.

test("filters by direct parent, in both reference forms", async () => {
  const byString = await getQuery(tokenA, `direct_parent=${encodeURIComponent(`${REPO}#9081`)}`);
  assert.equal(byString.status, 200);
  assert.deepEqual(numbers(byString.doc), [101]);

  const byObject = await postQuery(tokenA, { direct_parent: { repo: REPO, number: 101, kind: "issue" } });
  assert.equal(byObject.status, 200);
  assert.deepEqual(numbers(byObject.doc), [102]);
});

test("filters by session, lane, host, queue id, and kind", async () => {
  assert.deepEqual(numbers((await getQuery(tokenA, `session_id=${SES_A}`)).doc), [101, 103]);
  assert.deepEqual(numbers((await getQuery(tokenA, `session_id=${SES_B}`)).doc), [102]);
  assert.deepEqual(numbers((await getQuery(tokenA, "lane=code-a")).doc), [101, 103]);
  assert.deepEqual(numbers((await getQuery(tokenA, "host=amd-6")).doc), [101, 103, 107, 108, 109]);
  assert.deepEqual(numbers((await getQuery(tokenA, "queue_id=q-1")).doc), [101]);
  assert.deepEqual(numbers((await getQuery(tokenA, "kind=pr")).doc), [102]);
  // Combined filters AND together.
  assert.deepEqual(numbers((await getQuery(tokenA, "lane=code-a&host=laptop-2")).doc), []);
  assert.deepEqual(numbers((await postQuery(tokenA, { lane: "code-a", host: "amd-6" })).doc), [101, 103]);
  // A filter that matches nothing is an empty result, not an error.
  const none = await getQuery(tokenA, "lane=no-such-lane");
  assert.equal(none.status, 200);
  assert.equal(none.doc.count, 0);
});

test("link relation/status match as a pair on the same link", async () => {
  assert.deepEqual(numbers((await getQuery(tokenA, "link_relation=parent")).doc), [101, 103]);
  assert.deepEqual(numbers((await getQuery(tokenA, "link_status=current")).doc), [101, 103]);
  // The pair: #103 has relation parent (historical) and status
  // current (on a blocks link) — separately true, jointly false.
  assert.deepEqual(numbers((await getQuery(tokenA, "link_relation=parent&link_status=current")).doc), [101]);
  assert.deepEqual(numbers((await getQuery(tokenA, "link_relation=blocks&link_status=current")).doc), [103]);
  assert.deepEqual(
    numbers((await getQuery(tokenA, `link_target=${encodeURIComponent(`${REPO}#101`)}`)).doc),
    [102],
  );
});

test("results span both editions, with stamp status per edition", async () => {
  const { doc } = await getQuery(tokenA, "host=amd-6");
  const byNumber = new Map(doc.results.map((r) => [r.number, r]));
  // Hosted item: really signed at seed time, body agrees with DB.
  assert.equal(byNumber.get(101)?.stamp?.status, "verified");
  assert.equal(byNumber.get(101)?.divergence, null);
  assert.equal(byNumber.get(101)?.body_checked, true);
  assert.equal(byNumber.get(101)?.source, "db");
  // The native-edition item (by its own filter): no stamp_sig in
  // its §12 provenance, so the stamp status is unsigned, not an
  // error — and its content serves identically.
  const native = await getQuery(tokenA, "host=laptop-2");
  assert.deepEqual(numbers(native.doc), [102]);
  assert.equal(native.doc.results[0].stamp?.status, "unsigned");
  assert.equal(native.doc.results[0].divergence, null);
});

test("GET and POST answer the same document for the same filter", async () => {
  const viaGet = await getQuery(tokenA, "lane=code-a");
  const viaPost = await postQuery(tokenA, { lane: "code-a" });
  assert.deepEqual(viaPost.doc, viaGet.doc);
});

// --- Tenancy scoping.

test("results are scoped to the caller's tenancy", async () => {
  // #104 matches lane code-a + host amd-6 but belongs to B.
  assert.deepEqual(numbers((await getQuery(tokenA, "lane=code-a")).doc), [101, 103]);
  // B sees its own item, and A's session is an empty answer for B —
  // indistinguishable from a session that does not exist.
  assert.deepEqual(numbers((await getQuery(tokenB, `session_id=${SES_X}`)).doc), [104]);
  assert.deepEqual(numbers((await getQuery(tokenB, `session_id=${SES_A}`)).doc), []);
  // A service token scoped to the query door (the collector's
  // shape) queries as its own tenancy.
  const res = await dispatch(bearerReq("/v1/query?lane=code-a", QUERY_TOKEN), deps);
  assert.equal(res.status, 200);
  assert.deepEqual(numbers((await res.json()) as QueryDoc), [101, 103]);
});

test("a service token not scoped to the query door is refused", async () => {
  const res = await dispatch(bearerReq("/v1/query?lane=code-a", STAMP_ONLY_TOKEN), deps);
  assert.equal(res.status, 403);
  assert.equal(((await res.json()) as { error: string }).error, "service_token_route_scope");
});

// --- GitHub fallback (#9060 §13).

test("GitHub fallback serves not-yet-ingested items, attributed by stamp", async () => {
  // Without a repo the fallback cannot run (GitHub is scanned per
  // repo): the session query answers from the DB alone.
  assert.deepEqual(numbers((await getQuery(tokenA, `session_id=${SES_A}`)).doc), [101, 103]);
  // With the repo named, #105 joins from GitHub; #106 (B's stamp)
  // does not, and the ingested items are not duplicated.
  const { doc } = await getQuery(tokenA, `repo=${REPO}&session_id=${SES_A}`);
  assert.deepEqual(numbers(doc), [101, 103, 105]);
  const fallback = doc.results.find((r) => r.number === 105);
  assert.equal(fallback?.source, "github");
  assert.equal(fallback?.body_checked, true);
  assert.equal(fallback?.divergence, null);
  assert.equal(fallback?.stamp?.status, "verified");
});

// --- Divergence (#9060 §15).

test("a hand-edited body block shows a divergence finding and the body wins", async () => {
  const { doc } = await getQuery(tokenA, "lane=code-d");
  assert.deepEqual(numbers(doc), [107]);
  const result = doc.results[0];
  assert.deepEqual(result.divergence, { fields: ["direct_parent"] });
  // The served content is the body's (edited) value, not the DB's.
  assert.deepEqual(result.metadata["direct_parent"], { repo: REPO, number: 9999, kind: "issue" });
  // A caller-owned edit does not break the stamp.
  assert.equal(result.stamp?.status, "verified");
});

test("a hand-edited provenance shows divergence and a diverged stamp", async () => {
  const { doc } = await getQuery(tokenA, `session_id=${`ses_${"e".repeat(32)}`}`);
  assert.deepEqual(numbers(doc), [108]);
  const result = doc.results[0];
  assert.deepEqual(result.divergence, { fields: ["provenance"] });
  assert.equal((result.metadata["provenance"] as Record<string, unknown>)["lane"], "code-z");
  assert.equal(result.stamp?.status, "diverged");
  assert.equal(result.stamp?.reason, "signature_mismatch");
});

test("a body whose block was removed reports pm_meta_block divergence and serves the DB content", async () => {
  const { doc } = await getQuery(tokenA, "lane=code-f");
  assert.deepEqual(numbers(doc), [109]);
  const result = doc.results[0];
  assert.deepEqual(result.divergence, { fields: ["pm_meta_block"] });
  assert.equal(result.body_checked, true);
  // No body content exists to win: the DB row's Metadata serves.
  assert.equal((result.metadata["provenance"] as Record<string, unknown>)["lane"], "code-f");
});

// --- Failure shapes.

test("an unconfigured metadata store fails closed with 503", async () => {
  const bare: AuthDeps = { config, jwksFetcher: h.deps.jwksFetcher, sessions: new FixtureSessionRegistry() };
  const { status, doc } = await getQuery(tokenA, "lane=code-a", bare);
  assert.equal(status, 503);
  assert.equal((doc as { error?: string }).error, "metadata_store_not_configured");
});

test("without a GitHub item source the door serves DB rows unchecked and runs no fallback", async () => {
  const dbOnly: AuthDeps = { ...deps, githubItems: undefined };
  const { status, doc } = await getQuery(tokenA, `repo=${REPO}&session_id=${SES_A}`, dbOnly);
  assert.equal(status, 200);
  assert.deepEqual(numbers(doc), [101, 103]);
  assert.ok(doc.results.every((r) => r.body_checked === false && r.divergence === null));
});

test("malformed filters refuse with 400 invalid_query, never a silent match-all", async () => {
  for (const params of [
    "bogus=1",
    "direct_parent=not-a-ref",
    "direct_parent=foo%23bar",
    "kind=epic",
    "repo=noslash",
    "lane=",
    "lane=code-a&lane=code-b",
  ]) {
    const { status, doc } = await getQuery(tokenA, params);
    assert.equal(status, 400, params);
    assert.equal((doc as { error?: string }).error, "invalid_query", params);
  }
  const badKey = await postQuery(tokenA, { bogus: 1 });
  assert.equal(badKey.status, 400);
  assert.equal(badKey.doc.error, "invalid_query");
  const badRef = await postQuery(tokenA, { direct_parent: { repo: REPO } });
  assert.equal(badRef.status, 400);
  assert.equal(badRef.doc.error, "invalid_query");
  const arrayBody = await postQuery(tokenA, [1, 2, 3]);
  assert.equal(arrayBody.status, 400);
  assert.equal(arrayBody.doc.error, "invalid_body");
  const notJson = await postQuery(tokenA, "{nope");
  assert.equal(notJson.status, 400);
  assert.equal(notJson.doc.error, "invalid_body");
});

test("the seeded sample's bodies parse back (fixture sanity)", async () => {
  // Guards the sample itself: every writer body except #109's
  // carries a parseable block, so a green suite above is evidence
  // about the surface, not about a broken fixture.
  for (const item of writer.items) {
    if (item.number === 109) continue;
    assert.ok(parseMetadataBlock(item.body).block !== null, `item ${item.number}`);
  }
});
