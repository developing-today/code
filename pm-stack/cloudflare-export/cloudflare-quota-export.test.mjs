// Tests for the pure parsing core of cloudflare-quota-export.mjs.
// Run: node --test pm-stack/cloudflare-export/
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  classifyBilling,
  classifyR2,
  cycleStartUtc,
  API_REQUEST_TIMEOUT_MS,
  ExportError,
  failureDocument,
  foldKv,
  parseGraphqlUsage,
  R2_STORAGE_GROUP_LIMIT,
  run,
  successDocument,
} from "./cloudflare-quota-export.mjs";

// Shape captured from the live GraphQL analytics API, 2026-10-29 (#1048 probe).
const ACCOUNT = {
  workersInvocationsAdaptive: [{ sum: { requests: 75225, cpuTimeUs: 126395139 } }],
  kvOperationsAdaptiveGroups: [
    { dimensions: { actionType: "read" }, sum: { requests: 1420 } },
    { dimensions: { actionType: "write" }, sum: { requests: 20920 } },
    { dimensions: { actionType: "list" }, sum: { requests: 90 } },
  ],
  d1AnalyticsAdaptiveGroups: [{ sum: { rowsRead: 87175, rowsWritten: 122949 } }],
  queueMessageOperationsAdaptiveGroups: [{ sum: { billableOperations: 62123 } }],
  r2OperationsAdaptiveGroups: [
    { dimensions: { actionType: "ListObjects" }, sum: { requests: 165 } },
    { dimensions: { actionType: "GetObject" }, sum: { requests: 1000 } },
    { dimensions: { actionType: "PutObject" }, sum: { requests: 300 } },
    { dimensions: { actionType: "DeleteObject" }, sum: { requests: 59 } },
    { dimensions: { actionType: "HeadBucket" }, sum: { requests: 49 } },
  ],
  r2StorageAdaptiveGroups: [{ max: { payloadSize: 25139205306 } }],
  aiInferenceAdaptiveGroups: [{ sum: { totalNeurons: 4114515.31 } }],
};
const NOW = new Date("2026-10-29T10:00:00Z");
const START = cycleStartUtc(NOW);

test("API_REQUEST_TIMEOUT_MS: bounded below the 120s service deadline", () => {
  // cloudflare-quota-export.service has TimeoutStartSec=120; a stalled
  // request must abort well before that so main() can write a failure
  // document instead of being killed with a stale latest.json in place.
  assert.ok(API_REQUEST_TIMEOUT_MS > 0);
  assert.ok(API_REQUEST_TIMEOUT_MS < 120_000);
});

test("cycleStartUtc: first of the month, UTC", () => {
  assert.equal(START.toISOString(), "2026-10-01T00:00:00.000Z");
});

test("parseGraphqlUsage: every card metric id with the probed values", () => {
  const { metrics, r2Unclassified } = parseGraphqlUsage(ACCOUNT, { now: NOW, cycleStart: START });
  const byId = Object.fromEntries(metrics.map((m) => [m.id, m]));
  assert.equal(byId["workers-requests"].used, 75225);
  assert.equal(byId["workers-cpu-ms"].used, 126395); // µs → ms, rounded
  assert.equal(byId["kv-reads"].used, 1420);
  assert.equal(byId["kv-writes"].used, 21010); // write + list (card grouping)
  assert.equal(byId["d1-rows-read"].used, 87175);
  assert.equal(byId["d1-rows-written"].used, 122949);
  assert.equal(byId["queues-ops"].used, 62123);
  assert.equal(byId["r2-class-a"].used, 465); // ListObjects + PutObject
  assert.equal(byId["r2-class-b"].used, 1049); // GetObject + HeadBucket
  assert.equal(byId["r2-storage"].used, 25.14); // bytes → GB (decimal), 2dp
  // 4,114,515 neurons over 28.4167 elapsed days = 144,792/day.
  assert.equal(byId["workers-ai-neurons"].used, 144792);
  assert.match(byId["workers-ai-neurons"].detail, /4114515/);
  assert.deepEqual(r2Unclassified, []);
  for (const m of metrics) assert.equal(m.source, "cloudflare-graphql-analytics");
});

test("parseGraphqlUsage: r2-storage sums per-bucket peaks, not the max", () => {
  const twoBuckets = structuredClone(ACCOUNT);
  twoBuckets.r2StorageAdaptiveGroups = [
    { dimensions: { bucketName: "a" }, max: { payloadSize: 6_000_000_000 } },
    { dimensions: { bucketName: "b" }, max: { payloadSize: 6_000_000_000 } },
  ];
  const { metrics } = parseGraphqlUsage(twoBuckets, { now: NOW, cycleStart: START });
  const byId = Object.fromEntries(metrics.map((m) => [m.id, m]));
  assert.equal(byId["r2-storage"].used, 12); // 6 GB + 6 GB, not max 6 GB
});

test("parseGraphqlUsage: r2-storage total covers every bucket group, saturation is flagged", () => {
  // Detail states how many bucket groups the total covers.
  const twoBuckets = structuredClone(ACCOUNT);
  twoBuckets.r2StorageAdaptiveGroups = [
    { dimensions: { bucketName: "a" }, max: { payloadSize: 1_000_000_000 } },
    { dimensions: { bucketName: "b" }, max: { payloadSize: 2_000_000_000 } },
  ];
  const covered = parseGraphqlUsage(twoBuckets, { now: NOW, cycleStart: START });
  const coveredById = Object.fromEntries(covered.metrics.map((m) => [m.id, m]));
  assert.match(coveredById["r2-storage"].detail, /across 2 bucket group\(s\)/);
  assert.doesNotMatch(coveredById["r2-storage"].detail, /LOWER BOUND/);

  // A result at the query limit may exclude further buckets: the total
  // must say it is a lower bound instead of silently under-reporting.
  const saturated = structuredClone(ACCOUNT);
  saturated.r2StorageAdaptiveGroups = Array.from({ length: R2_STORAGE_GROUP_LIMIT }, (_, i) => ({
    dimensions: { bucketName: `b${i}` },
    max: { payloadSize: 1_000_000 },
  }));
  const sat = parseGraphqlUsage(saturated, { now: NOW, cycleStart: START });
  const satById = Object.fromEntries(sat.metrics.map((m) => [m.id, m]));
  assert.match(satById["r2-storage"].detail, /LOWER BOUND/);
  assert.equal(satById["r2-storage"].lowerBound, true);
  assert.equal(coveredById["r2-storage"].lowerBound, undefined);
});

test("classifyR2/foldKv: a missing groups field fails closed, not zero", () => {
  assert.throws(() => classifyR2(undefined), /r2OperationsAdaptiveGroups missing/);
  assert.throws(() => classifyR2(null), /r2OperationsAdaptiveGroups missing/);
  assert.throws(() => foldKv(undefined), /kvOperationsAdaptiveGroups missing/);
  assert.throws(() => foldKv(null), /kvOperationsAdaptiveGroups missing/);
  // Present-but-empty still means "no operations".
  assert.deepEqual(foldKv([]), { reads: 0, writes: 0 });
  assert.equal(classifyR2([]).classA, 0);
});

test("parseGraphqlUsage: shape drift fails closed", () => {
  assert.throws(() => parseGraphqlUsage({}, { now: NOW, cycleStart: START }), /unparseable|missing/);
  const drifted = structuredClone(ACCOUNT);
  drifted.workersInvocationsAdaptive = [{ sum: { requests: "many", cpuTimeUs: 1 } }];
  assert.throws(() => parseGraphqlUsage(drifted, { now: NOW, cycleStart: START }), /not a finite/);
  const missingStorage = structuredClone(ACCOUNT);
  delete missingStorage.r2StorageAdaptiveGroups;
  assert.throws(() => parseGraphqlUsage(missingStorage, { now: NOW, cycleStart: START }), /r2Storage/);
});

test("parseGraphqlUsage: empty r2 storage is zero, other metrics survive", () => {
  const noStorage = structuredClone(ACCOUNT);
  noStorage.r2StorageAdaptiveGroups = [];
  const { metrics } = parseGraphqlUsage(noStorage, { now: NOW, cycleStart: START });
  const byId = Object.fromEntries(metrics.map((m) => [m.id, m]));
  assert.equal(byId["r2-storage"].used, 0);
  assert.match(byId["r2-storage"].detail, /no R2 bucket storage observations/);
  assert.equal(byId["workers-requests"].used, 75225);
  assert.equal(byId["r2-class-a"].used, 465);
});

test("classifyR2: 401/403 auth failures are nonbillable, other failures still count", () => {
  const r = classifyR2([
    { dimensions: { actionType: "GetObject", responseStatusCode: 200 }, sum: { requests: 100 } },
    { dimensions: { actionType: "GetObject", responseStatusCode: 401 }, sum: { requests: 40 } },
    { dimensions: { actionType: "PutObject", responseStatusCode: 403 }, sum: { requests: 30 } },
    { dimensions: { actionType: "GetObject", responseStatusCode: 404 }, sum: { requests: 7 } },
    { dimensions: { actionType: "PutObject" }, sum: { requests: 5 } },
  ]);
  assert.equal(r.classB, 107); // 200 + billable 404
  assert.equal(r.classA, 5); // no-status group counted as before; 403 excluded
  assert.equal(r.nonbillable, 70);
  assert.deepEqual(r.unclassified, []);
});

test("classifyR2: unknown actions are reported, never guessed into a class", () => {
  const r = classifyR2([
    { dimensions: { actionType: "GetObject" }, sum: { requests: 5 } },
    { dimensions: { actionType: "SomeFutureOp" }, sum: { requests: 7 } },
  ]);
  assert.equal(r.classA, 0);
  assert.equal(r.classB, 5);
  assert.deepEqual(r.unclassified, ["SomeFutureOp"]);
});

test("classifyR2: LifecycleStorageTierTransition counts as Class A", () => {
  const r = classifyR2([
    { dimensions: { actionType: "LifecycleStorageTierTransition" }, sum: { requests: 11 } },
  ]);
  assert.equal(r.classA, 11);
  assert.deepEqual(r.unclassified, []);
});

test("classifyBilling: a transport-failed probe (status 0) is unknown, never invoice-only", () => {
  const r = classifyBilling([
    { endpoint: "a", status: 0 },
    { endpoint: "b", status: 403 },
  ]);
  assert.equal(r.status, "unknown");
});

test("foldKv: delete folds into the writes side with write/list", () => {
  const kv = foldKv([
    { dimensions: { actionType: "read" }, sum: { requests: 3 } },
    { dimensions: { actionType: "delete" }, sum: { requests: 4 } },
  ]);
  assert.deepEqual(kv, { reads: 3, writes: 4 });
});

test("classifyBilling: dual 403 settles invoice-only; anything else is honest", () => {
  const inv = classifyBilling([
    { endpoint: "/accounts/{account}/billing/profile", status: 403 },
    { endpoint: "/accounts/{account}/subscriptions", status: 403 },
  ]);
  assert.equal(inv.status, "invoice-only");
  assert.match(inv.detail, /not-yet-billed/);
  const mixed = classifyBilling([
    { endpoint: "a", status: 403 },
    { endpoint: "b", status: 500 },
  ]);
  assert.equal(mixed.status, "unknown");
  const ok = classifyBilling([
    { endpoint: "a", status: 200 },
    { endpoint: "b", status: 200 },
  ]);
  assert.equal(ok.status, "readable");
});

test("documents: success carries the spec fields; failure carries no values", () => {
  const parsed = parseGraphqlUsage(ACCOUNT, { now: NOW, cycleStart: START });
  const ok = successDocument({
    measuredAt: NOW.toISOString(),
    cycleStart: START,
    accountId: "acct",
    billing: classifyBilling([
      { endpoint: "a", status: 403 },
      { endpoint: "b", status: 403 },
    ]),
    parsed,
    r2Buckets: ["b1"],
  });
  assert.equal(ok.service, "cloudflare");
  assert.equal(ok.source, "file-export/cloudflare");
  assert.equal(ok.status, "ok");
  assert.equal(ok.metrics.length, 11);
  assert.equal(ok.billing.status, "invoice-only");

  const bad = failureDocument({
    attemptedAt: NOW.toISOString(),
    status: "token-scope",
    detail: "HTTP 403",
    extra: { missingScopes: ["Account Analytics Read"] },
  });
  assert.equal(bad.measuredAt, null);
  assert.equal("metrics" in bad, false);
  assert.deepEqual(bad.missingScopes, ["Account Analytics Read"]);
});

// #2608: GraphQL errors shape handling in run(). The stub answers the
// billing probes and R2 enumeration with benign bodies and returns the
// given body for /graphql; ACCOUNT is a valid parsed account node.
function stubFetchFor(gqlBody) {
  return async (url) => {
    const u = String(url);
    let body;
    if (u.includes("/graphql")) body = gqlBody;
    else if (u.includes("/r2/buckets")) body = { result: { buckets: [{ name: "b1" }] } };
    else body = {};
    return { status: 200, json: async () => body };
  };
}

async function runWithErrors(errors) {
  const prev = globalThis.fetch;
  globalThis.fetch = stubFetchFor({
    data: { viewer: { accounts: [ACCOUNT] } },
    ...(errors !== undefined ? { errors } : {}),
  });
  try {
    return await run({ token: "tok", accountId: "acct", now: NOW });
  } finally {
    globalThis.fetch = prev;
  }
}

async function expectExportError(errors, status, exitCode) {
  await assert.rejects(runWithErrors(errors), (e) => {
    assert.ok(e instanceof ExportError, `expected ExportError, got ${e}`);
    assert.equal(e.status, status);
    assert.equal(e.exitCode, exitCode);
    return true;
  });
}

test("run: GraphQL errors malformed shapes fail closed as unparseable (#2608)", async () => {
  await expectExportError("boom", "unparseable", 6);
  await expectExportError([null], "unparseable", 6);
  await expectExportError([{ message: 12345 }], "unparseable", 6);
  await expectExportError({ message: 12345 }, "unparseable", 6);
});

test("run: a single-object GraphQL errors classifies like the array form (#2608)", async () => {
  await expectExportError({ message: "auth: permission denied" }, "token-scope", 3);
  await expectExportError({ message: "some query error" }, "unparseable", 6);
});

test("run: GraphQL errors controls unchanged (#2608)", async () => {
  await expectExportError([{ message: "auth: permission denied" }], "token-scope", 3);
  const doc = await runWithErrors(undefined);
  assert.equal(doc.status, "ok");
  assert.equal(doc.metrics.length, 11);
});
