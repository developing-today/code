// Tests for cloudflare-quota-export parsing and CLI entry-point behavior.
// Run: node --test pm-stack/cloudflare-export/cloudflare-quota-export.test.mjs
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";
import {
  aiNeuronsDayAlerts,
  aiNeuronsDayAlertThreshold,
  AI_NEURONS_DAY_ALERT_THRESHOLD_DEFAULT,
  AI_NEURONS_DAY_GROUP_LIMIT,
  classifyBilling,
  classifyR2,
  cycleStartUtc,
  API_REQUEST_TIMEOUT_MS,
  ExportError,
  parseArgs,
  failureDocument,
  foldKv,
  listR2Buckets,
  parseGraphqlUsage,
  AI_NEURONS_GROUP_LIMIT,
  D1_GROUP_LIMIT,
  KV_GROUP_LIMIT,
  QUEUES_GROUP_LIMIT,
  R2_OPERATIONS_GROUP_LIMIT,
  R2_STORAGE_GROUP_LIMIT,
  WORKERS_GROUP_LIMIT,
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
  // Grouped rows sum to the same 4,114,515.31 cycle total: the
  // Oct 24–25 2026 burn shape from #2287 (Oct 24 alone ~2.99M on the
  // deepseek model via the gateway).
  aiInferenceAdaptiveGroupsByDay: [
    { dimensions: { date: "2026-10-24", modelId: "@cf/deepseek-ai/deepseek-v4-pro-0813" }, sum: { totalNeurons: 2990000 } },
    { dimensions: { date: "2026-10-24", modelId: "@cf/meta/llama-3.1-8b-instruct" }, sum: { totalNeurons: 100000 } },
    { dimensions: { date: "2026-10-25", modelId: "@cf/deepseek-ai/deepseek-v4-pro-0813" }, sum: { totalNeurons: 1024515.31 } },
  ],
};
const NOW = new Date("2026-10-29T10:00:00Z");
const START = cycleStartUtc(NOW);
const EXPORTER_PATH = fileURLToPath(new URL("./cloudflare-quota-export.mjs", import.meta.url));

test("API_REQUEST_TIMEOUT_MS: bounded below the 120s service deadline", () => {
  // cloudflare-quota-export.service has TimeoutStartSec=120; a stalled
  // request must abort well before that so main() can write a failure
  // document instead of being killed with a stale latest.json in place.
  assert.ok(API_REQUEST_TIMEOUT_MS > 0);
  assert.ok(API_REQUEST_TIMEOUT_MS < 120_000);
});

test("CLI: runs main via file and directory symlinks (#4325)", () => {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "cloudflare-export-entrypoint-"));
  const links = path.join(home, "links");
  fs.mkdirSync(links);
  const fileLink = path.join(links, "export.mjs");
  const dirLink = path.join(links, "export-dir");
  fs.symlinkSync(EXPORTER_PATH, fileLink);
  fs.symlinkSync(path.dirname(EXPORTER_PATH), dirLink, "dir");
  const env = { ...process.env, HOME: home, CLOUDFLARE_API_TOKEN: "" };

  try {
    for (const [name, entry] of [
      ["direct", EXPORTER_PATH],
      ["file-symlink", fileLink],
      ["directory-symlink", path.join(dirLink, path.basename(EXPORTER_PATH))],
    ]) {
      const outDir = path.join(home, name);
      const result = spawnSync(process.execPath, [entry, "--out-dir", outDir], {
        env,
        encoding: "utf8",
        timeout: 10000,
      });
      assert.equal(result.status, 2, `${name}: ${result.stderr}`);
      const doc = JSON.parse(fs.readFileSync(path.join(outDir, "latest.json"), "utf8"));
      assert.equal(doc.service, "cloudflare");
      assert.equal(doc.status, "no-token");
    }
  } finally {
    fs.rmSync(home, { recursive: true, force: true });
  }
});

test("importing cloudflare-quota-export.mjs as a library does not run main (#4325)", () => {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "cloudflare-export-import-"));
  try {
    const result = spawnSync(process.execPath, [
      "--input-type=module", "-e", `await import(${JSON.stringify(pathToFileURL(EXPORTER_PATH).href)});`,
    ], {
      env: { ...process.env, HOME: home, CLOUDFLARE_API_TOKEN: "" },
      encoding: "utf8",
      timeout: 5000,
    });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stdout, "");
    assert.equal(result.stderr, "");
    assert.equal(fs.existsSync(path.join(home, "atlas", "cloudflare-export")), false);
  } finally {
    fs.rmSync(home, { recursive: true, force: true });
  }
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

// #6880: every grouped dataset with an explicit query limit flags
// its metrics lowerBound at the limit (the r2-storage pattern) and
// stays unflagged one below it — no at-limit sum publishes as exact.
const SATURATION_CASES = [
  {
    dataset: "workersInvocationsAdaptive",
    limit: WORKERS_GROUP_LIMIT,
    group: () => ({ sum: { requests: 1, cpuTimeUs: 1000 } }),
    metricIds: ["workers-requests", "workers-cpu-ms"],
  },
  {
    dataset: "kvOperationsAdaptiveGroups",
    limit: KV_GROUP_LIMIT,
    group: () => ({ dimensions: { actionType: "read" }, sum: { requests: 1 } }),
    metricIds: ["kv-reads", "kv-writes"],
  },
  {
    dataset: "d1AnalyticsAdaptiveGroups",
    limit: D1_GROUP_LIMIT,
    group: () => ({ sum: { rowsRead: 1, rowsWritten: 1 } }),
    metricIds: ["d1-rows-read", "d1-rows-written"],
  },
  {
    dataset: "queueMessageOperationsAdaptiveGroups",
    limit: QUEUES_GROUP_LIMIT,
    group: () => ({ sum: { billableOperations: 1 } }),
    metricIds: ["queues-ops"],
  },
  {
    dataset: "r2OperationsAdaptiveGroups",
    limit: R2_OPERATIONS_GROUP_LIMIT,
    group: () => ({ dimensions: { actionType: "GetObject" }, sum: { requests: 1 } }),
    metricIds: ["r2-class-a", "r2-class-b"],
  },
  {
    dataset: "aiInferenceAdaptiveGroups",
    limit: AI_NEURONS_GROUP_LIMIT,
    group: () => ({ sum: { totalNeurons: 10 } }),
    metricIds: ["workers-ai-neurons"],
  },
];

for (const { dataset, limit, group, metricIds } of SATURATION_CASES) {
  test(`parseGraphqlUsage: ${dataset} at its query limit flags lowerBound (#6880)`, () => {
    const atLimit = structuredClone(ACCOUNT);
    atLimit[dataset] = Array.from({ length: limit }, group);
    const satById = Object.fromEntries(
      parseGraphqlUsage(atLimit, { now: NOW, cycleStart: START }).metrics.map((m) => [m.id, m]),
    );
    for (const id of metricIds) assert.equal(satById[id].lowerBound, true, id);

    const belowLimit = structuredClone(ACCOUNT);
    belowLimit[dataset] = Array.from({ length: limit - 1 }, group);
    const belowById = Object.fromEntries(
      parseGraphqlUsage(belowLimit, { now: NOW, cycleStart: START }).metrics.map((m) => [m.id, m]),
    );
    for (const id of metricIds) assert.equal(belowById[id].lowerBound, undefined, id);
  });
}

test("classifyR2: saturation is reported for r2OperationsAdaptiveGroups (#6880)", () => {
  const groups = Array.from({ length: R2_OPERATIONS_GROUP_LIMIT }, () => ({
    dimensions: { actionType: "GetObject" },
    sum: { requests: 1 },
  }));
  const atLimit = classifyR2(groups);
  assert.equal(atLimit.saturated, true);
  assert.equal(atLimit.classB, R2_OPERATIONS_GROUP_LIMIT);
  assert.equal(classifyR2(groups.slice(1)).saturated, false);
});

test("parseGraphqlUsage: ai cycle-total saturation is flagged in prose too (#6880)", () => {
  const atLimit = structuredClone(ACCOUNT);
  atLimit.aiInferenceAdaptiveGroups = Array.from({ length: AI_NEURONS_GROUP_LIMIT }, () => ({
    sum: { totalNeurons: 10 },
  }));
  const byId = Object.fromEntries(
    parseGraphqlUsage(atLimit, { now: NOW, cycleStart: START }).metrics.map((m) => [m.id, m]),
  );
  assert.equal(byId["workers-ai-neurons"].lowerBound, true);
  assert.match(byId["workers-ai-neurons"].detail, /LOWER BOUND/);
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

test("aiNeuronsDayAlerts: an Oct 24-shaped day fires with top-model attribution (#2287)", () => {
  const { alerts, saturated } = aiNeuronsDayAlerts(ACCOUNT.aiInferenceAdaptiveGroupsByDay, 50000);
  assert.equal(saturated, false);
  assert.equal(alerts.length, 2); // both burn days exceed 50k
  assert.deepEqual(alerts[0], {
    kind: "workers-ai-neurons-day",
    date: "2026-10-24",
    totalNeurons: 3090000,
    threshold: 50000,
    topModels: [
      { model: "@cf/deepseek-ai/deepseek-v4-pro-0813", neurons: 2990000 },
      { model: "@cf/meta/llama-3.1-8b-instruct", neurons: 100000 },
    ],
  });
  assert.equal(alerts[1].date, "2026-10-25");
  assert.equal(alerts[1].totalNeurons, 1024515); // fractional day total rounds for display
});

test("aiNeuronsDayAlerts: days at or under the threshold stay silent (#2287)", () => {
  const quiet = [
    { dimensions: { date: "2026-10-24", modelId: "@cf/meta/llama-3.1-8b-instruct" }, sum: { totalNeurons: 10000 } },
    { dimensions: { date: "2026-10-25", modelId: "@cf/meta/llama-3.1-8b-instruct" }, sum: { totalNeurons: 9999 } },
  ];
  assert.deepEqual(aiNeuronsDayAlerts(quiet, 50000), { alerts: [], saturated: false });
  // Exactly at the threshold does not fire (the rule is exceeds).
  assert.deepEqual(aiNeuronsDayAlerts(quiet, 10000).alerts, []);
  // No usage at all (empty groups) is a legitimate silent zero.
  assert.deepEqual(aiNeuronsDayAlerts([], 50000), { alerts: [], saturated: false });
});

test("aiNeuronsDayAlerts: top models are capped at 3, ordered by neurons (#2287)", () => {
  const groups = ["m1", "m2", "m3", "m4"].map((modelId, i) => ({
    dimensions: { date: "2026-10-24", modelId },
    sum: { totalNeurons: (4 - i) * 100000 },
  }));
  const { alerts } = aiNeuronsDayAlerts(groups, 50000);
  assert.equal(alerts.length, 1);
  assert.deepEqual(alerts[0].topModels.map((m) => m.model), ["m1", "m2", "m3"]);
});

test("aiNeuronsDayAlerts: a missing or drifted grouped field fails closed (#2287)", () => {
  assert.throws(() => aiNeuronsDayAlerts(undefined, 50000), /aiInferenceAdaptiveGroupsByDay groups missing/);
  assert.throws(
    () => aiNeuronsDayAlerts([{ dimensions: { date: "24 Oct", modelId: "m" }, sum: { totalNeurons: 1 } }], 50000),
    /dimensions\.date/,
  );
  assert.throws(
    () => aiNeuronsDayAlerts([{ dimensions: { date: "2026-10-24" }, sum: { totalNeurons: 1 } }], 50000),
    /dimensions\.modelId/,
  );
  const missing = structuredClone(ACCOUNT);
  delete missing.aiInferenceAdaptiveGroupsByDay;
  assert.throws(() => parseGraphqlUsage(missing, { now: NOW, cycleStart: START }), /aiInferenceAdaptiveGroupsByDay/);
});

test("aiNeuronsDayAlerts: saturation is flagged, never silently truncated (#2287)", () => {
  const groups = Array.from({ length: AI_NEURONS_DAY_GROUP_LIMIT }, (_, i) => ({
    dimensions: { date: "2026-10-24", modelId: `m${i}` },
    sum: { totalNeurons: 100 },
  }));
  const { alerts, saturated } = aiNeuronsDayAlerts(groups, 50000);
  assert.equal(saturated, true);
  assert.equal(alerts.length, 1);
  assert.equal(alerts[0].lowerBound, true);
  // The document carries the saturation flag even when nothing fires.
  const parsed = parseGraphqlUsage(
    { ...structuredClone(ACCOUNT), aiInferenceAdaptiveGroupsByDay: groups },
    { now: NOW, cycleStart: START, aiThreshold: Number.MAX_SAFE_INTEGER },
  );
  const doc = successDocument({
    measuredAt: NOW.toISOString(),
    cycleStart: START,
    accountId: "acct",
    billing: { status: "invoice-only" },
    parsed,
    r2Buckets: [],
  });
  assert.equal(doc.aiNeuronsByDaySaturated, true);
  assert.equal("alerts" in doc, false);
});

test("aiNeuronsDayAlertThreshold: env override with honest fallback (#2287)", () => {
  assert.equal(AI_NEURONS_DAY_ALERT_THRESHOLD_DEFAULT, 50000);
  assert.equal(aiNeuronsDayAlertThreshold({}), 50000);
  assert.equal(aiNeuronsDayAlertThreshold({ CLOUDFLARE_AI_NEURONS_DAY_ALERT_THRESHOLD: "12345" }), 12345);
  assert.equal(aiNeuronsDayAlertThreshold({ CLOUDFLARE_AI_NEURONS_DAY_ALERT_THRESHOLD: "" }), 50000);
  assert.equal(aiNeuronsDayAlertThreshold({ CLOUDFLARE_AI_NEURONS_DAY_ALERT_THRESHOLD: "abc" }), 50000);
  assert.equal(aiNeuronsDayAlertThreshold({ CLOUDFLARE_AI_NEURONS_DAY_ALERT_THRESHOLD: "-5" }), 50000);
  // Zero is a typo, never a threshold: every spelling falls back to the
  // default, so a single neuron can never trip the daily alert (#4381).
  for (const zero of ["0", "-0", "0.0", " 0 ", "0e5"]) {
    assert.equal(
      aiNeuronsDayAlertThreshold({ CLOUDFLARE_AI_NEURONS_DAY_ALERT_THRESHOLD: zero }),
      50000,
      `zero spelling ${JSON.stringify(zero)} must fall back to the default`,
    );
  }
  // Spellings that denote a positive number are accepted as that number;
  // pinned so the parse cannot drift silently either way.
  assert.equal(aiNeuronsDayAlertThreshold({ CLOUDFLARE_AI_NEURONS_DAY_ALERT_THRESHOLD: "0x10" }), 16);
  assert.equal(aiNeuronsDayAlertThreshold({ CLOUDFLARE_AI_NEURONS_DAY_ALERT_THRESHOLD: "0.5" }), 0.5);
  // The gate a zeroed threshold would have opened: at the threshold exactly
  // nothing fires, one neuron above it does.
  const oneDay = (neurons) => [
    { dimensions: { date: "2026-11-01", modelId: "@cf/meta/llama" }, sum: { totalNeurons: neurons } },
  ];
  assert.deepEqual(aiNeuronsDayAlerts(oneDay(50000), 50000).alerts, []);
  assert.equal(aiNeuronsDayAlerts(oneDay(50001), 50000).alerts.length, 1);
  // A custom threshold actually gates the alert through parseGraphqlUsage.
  const parsed = parseGraphqlUsage(ACCOUNT, { now: NOW, cycleStart: START, aiThreshold: 4000000 });
  assert.deepEqual(parsed.aiNeurons.alerts, []);
});

test("documents: alerts array present only when a day is firing (#2287)", () => {
  const billing = classifyBilling([
    { endpoint: "a", status: 403 },
    { endpoint: "b", status: 403 },
  ]);
  const firing = successDocument({
    measuredAt: NOW.toISOString(),
    cycleStart: START,
    accountId: "acct",
    billing,
    parsed: parseGraphqlUsage(ACCOUNT, { now: NOW, cycleStart: START }),
    r2Buckets: [],
  });
  assert.equal(firing.alerts.length, 2);
  assert.equal(firing.alerts[0].kind, "workers-ai-neurons-day");

  const quietAccount = structuredClone(ACCOUNT);
  quietAccount.aiInferenceAdaptiveGroupsByDay = [
    { dimensions: { date: "2026-10-24", modelId: "@cf/meta/llama-3.1-8b-instruct" }, sum: { totalNeurons: 5000 } },
  ];
  const quiet = successDocument({
    measuredAt: NOW.toISOString(),
    cycleStart: START,
    accountId: "acct",
    billing,
    parsed: parseGraphqlUsage(quietAccount, { now: NOW, cycleStart: START }),
    r2Buckets: [],
  });
  assert.equal("alerts" in quiet, false);
});

test("measurement strings use plain decimals; request counts remain integers (#2791)", () => {
  const invalid = ["", "   ", " 12 ", "0x10", "1e2", "1.5"];
  for (const value of invalid) {
    assert.throws(
      () => classifyR2([{ dimensions: { actionType: "GetObject" }, sum: { requests: value } }]),
      (error) => error instanceof ExportError && error.status === "unparseable" && error.exitCode === 6 &&
        error.message.includes("r2.GetObject.requests"),
    );
    assert.throws(
      () => foldKv([{ dimensions: { actionType: "read" }, sum: { requests: value } }]),
      (error) => error instanceof ExportError && error.status === "unparseable" && error.exitCode === 6 &&
        error.message.includes("kv.read.requests"),
    );
    const bad = structuredClone(ACCOUNT);
    bad.workersInvocationsAdaptive[0].sum.requests = value;
    assert.throws(
      () => parseGraphqlUsage(bad, { now: NOW, cycleStart: START }),
      (error) => error instanceof ExportError && error.status === "unparseable" && error.exitCode === 6 &&
        error.message.includes("workers.requests"),
    );
  }

  for (const value of ["", "   ", " 25139205306 ", "0x10", "1e2"]) {
    const bad = structuredClone(ACCOUNT);
    bad.r2StorageAdaptiveGroups[0].max.payloadSize = value;
    assert.throws(
      () => parseGraphqlUsage(bad, { now: NOW, cycleStart: START }),
      (error) => error instanceof ExportError && error.status === "unparseable" && error.exitCode === 6 &&
        error.message.includes("r2storage.payloadSize"),
    );
  }

  const decimalPayload = structuredClone(ACCOUNT);
  decimalPayload.r2StorageAdaptiveGroups[0].max.payloadSize = "25139205306.5";
  assert.equal(
    Object.fromEntries(parseGraphqlUsage(decimalPayload, { now: NOW, cycleStart: START }).metrics.map((m) => [m.id, m]))[
      "r2-storage"
    ].used,
    25.14,
  );
  assert.equal(classifyR2([{ dimensions: { actionType: "GetObject" }, sum: { requests: "12" } }]).classB, 12);
  assert.equal(foldKv([{ dimensions: { actionType: "read" }, sum: { requests: "12" } }]).reads, 12);
  assert.equal(classifyR2([{ dimensions: { actionType: "GetObject" }, sum: { requests: 1.5 } }]).classB, 1.5);

  // Neurons are a fractional measurement, not a count (Codex P2 on PR #3488):
  // a plain-decimal string total parses exactly like the numeric fixture.
  const decimalNeurons = structuredClone(ACCOUNT);
  decimalNeurons.aiInferenceAdaptiveGroups[0].sum.totalNeurons = "4114515.31";
  assert.equal(
    Object.fromEntries(parseGraphqlUsage(decimalNeurons, { now: NOW, cycleStart: START }).metrics.map((m) => [m.id, m]))[
      "workers-ai-neurons"
    ].used,
    144792,
  );
  // Drifted spellings still fail closed for neurons, naming the ai field.
  for (const value of ["", "   ", " 4114515.31 ", "0x10", "1e2"]) {
    const bad = structuredClone(ACCOUNT);
    bad.aiInferenceAdaptiveGroups[0].sum.totalNeurons = value;
    assert.throws(
      () => parseGraphqlUsage(bad, { now: NOW, cycleStart: START }),
      (error) => error instanceof ExportError && error.status === "unparseable" && error.exitCode === 6 &&
        error.message.includes("ai.totalNeurons"),
    );
  }
});

// #2574: R2 bucket enumeration is explicit — unknown is null plus a
// detail, never a silent [] or a truncated page. Bucket pages are
// consumed in order for /r2/buckets; billing probes and GraphQL stay
// benign so the document's metrics prove the degradation is isolated.
function bucketPage(names, { page = 1, totalPages } = {}) {
  return {
    status: 200,
    body: {
      result: { buckets: names.map((name) => ({ name })) },
      ...(totalPages === undefined ? {} : {
        result_info: { page, per_page: 100, total_pages: totalPages, count: names.length, total_count: 150 },
      }),
    },
  };
}

async function runWithBuckets(bucketPages) {
  let i = 0;
  const prev = globalThis.fetch;
  globalThis.fetch = async (url) => {
    const u = String(url);
    if (u.includes("/r2/buckets")) {
      const p = bucketPages[Math.min(i++, bucketPages.length - 1)];
      return { status: p.status, json: async () => p.body };
    }
    if (u.includes("/graphql")) {
      return { status: 200, json: async () => ({ data: { viewer: { accounts: [ACCOUNT] } } }) };
    }
    return { status: 200, json: async () => ({}) };
  };
  try {
    return await run({ token: "tok", accountId: "acct", now: NOW });
  } finally {
    globalThis.fetch = prev;
  }
}

test("run: an HTTP 500 on buckets keeps metrics and marks enumeration unknown (#2574)", async () => {
  const doc = await runWithBuckets([{ status: 500, body: {} }]);
  assert.equal(doc.status, "ok");
  assert.equal(doc.metrics.length, 11); // GraphQL metrics survive
  assert.equal(doc.r2Buckets, null);
  assert.match(doc.r2BucketsDetail, /HTTP 500/);
});

test("run: missing or malformed result.buckets marks enumeration unknown (#2574)", async () => {
  for (const body of [{}, { result: {} }, { result: { buckets: "nope" } }, { result: { buckets: null } }]) {
    const doc = await runWithBuckets([{ status: 200, body }]);
    assert.equal(doc.status, "ok");
    assert.equal(doc.r2Buckets, null);
    assert.match(doc.r2BucketsDetail, /missing or malformed/);
  }
});

test("run: pagination follows every reported page (#2574)", async () => {
  const first = Array.from({ length: 100 }, (_, i) => `b${i}`);
  const rest = Array.from({ length: 50 }, (_, i) => `b${100 + i}`);
  const doc = await runWithBuckets([bucketPage(first, { page: 1, totalPages: 2 }), bucketPage(rest, { page: 2, totalPages: 2 })]);
  assert.equal(doc.status, "ok");
  assert.equal(doc.r2Buckets.length, 150);
  assert.equal(doc.r2Buckets[0], "b0");
  assert.equal(doc.r2Buckets[149], "b149");
  assert.equal("r2BucketsDetail" in doc, false);
});

test("run: a later-page failure publishes unknown, never the partial list (#2574)", async () => {
  const first = Array.from({ length: 100 }, (_, i) => `b${i}`);
  const doc = await runWithBuckets([
    bucketPage(first, { page: 1, totalPages: 2 }),
    { status: 500, body: {} },
  ]);
  assert.equal(doc.status, "ok");
  assert.equal(doc.r2Buckets, null); // not the 100-name partial page
  assert.match(doc.r2BucketsDetail, /page 2/);
});

test("run: a malformed later page publishes unknown, never the partial list (#2574)", async () => {
  const first = Array.from({ length: 100 }, (_, i) => `b${i}`);
  const doc = await runWithBuckets([
    bucketPage(first, { page: 1, totalPages: 2 }),
    { status: 200, body: { result: { buckets: "nope" } } },
  ]);
  assert.equal(doc.r2Buckets, null);
  assert.match(doc.r2BucketsDetail, /page 2/);
});

test("run: a full page with no pagination metadata is unknown, not complete (#2574)", async () => {
  const full = Array.from({ length: 100 }, (_, i) => `b${i}`);
  const doc = await runWithBuckets([bucketPage(full)]);
  assert.equal(doc.r2Buckets, null);
  assert.match(doc.r2BucketsDetail, /truncated/);
  // A short page without metadata is still provably complete (existing shape).
  const short = await runWithBuckets([bucketPage(["b1"])]);
  assert.deepEqual(short.r2Buckets, ["b1"]);
  assert.equal("r2BucketsDetail" in short, false);
});

test("run: 401/403 on buckets still throws the token-scope contract (#2574)", async () => {
  await assert.rejects(runWithBuckets([{ status: 403, body: {} }]), (e) => {
    assert.ok(e instanceof ExportError);
    assert.equal(e.status, "token-scope");
    assert.equal(e.exitCode, 3);
    return true;
  });
});

// Strict argv validation (#3469): a value flag followed by another flag
// (or by nothing) is a usage error naming the flag, raised before any
// directory creation or API call; no token is consumed twice.
function expectUsage(argv, flag) {
  assert.throws(
    () => parseArgs(argv),
    (error) => {
      assert.ok(error instanceof ExportError, `expected ExportError, got ${error}`);
      assert.equal(error.status, "usage");
      assert.equal(error.exitCode, 2);
      assert.match(error.message, new RegExp(flag.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
      return true;
    },
    `parseArgs(${JSON.stringify(argv)}) should be a usage error naming ${flag}`,
  );
}

test("parseArgs: the control invocation parses, account-id optional (#3469)", () => {
  assert.deepEqual(parseArgs(["--out-dir", "out", "--account-id", "probeacct"]), {
    outDir: "out",
    accountId: "probeacct",
  });
  const def = parseArgs([]);
  assert.match(def.outDir, /cloudflare-export$/);
  assert.equal(def.accountId, undefined);
});

test("parseArgs: flag-as-value is a usage error naming the flag (#3469)", () => {
  // The exact probe from the issue: --account-id must not double as the
  // out-dir value and the account flag.
  expectUsage(["--out-dir", "--account-id", "probeacct"], "--out-dir");
  expectUsage(["--account-id", "--out-dir", "out"], "--account-id");
});

test("parseArgs: a trailing flag with no value is a usage error, not a silent fallback (#3469)", () => {
  expectUsage(["--out-dir"], "--out-dir");
  expectUsage(["--out-dir", "out", "--account-id"], "--account-id");
});

test("parseArgs: no token is consumed twice (#3469)", () => {
  // Old opt() returned "--account-id" as the out-dir AND "probeacct" as
  // the account. Now the single-pass parse refuses outright instead of
  // splitting the token stream two ways.
  expectUsage(["--out-dir", "--account-id", "probeacct"], "--out-dir");
});

test("parseArgs: unknown flags and bare positionals are usage errors (#3469)", () => {
  expectUsage(["--bogus", "x"], "--bogus");
  expectUsage(["--out-dir=out"], "--out-dir=out");
  expectUsage(["out"], "out");
});

// Property-based coverage for parseArgs (#3469, Codex P1 on PR #9009).
// Oracle: the declared argv grammar — a stream is zero or more
// (value-flag, value) pairs; a value is a non-empty token not starting
// with "--"; a repeated flag's last occurrence wins. Streams are
// generated from that grammar (valid) or by injecting exactly one
// grammar violation (malformed), so the expected outcome is known by
// construction, independent of the parser under test. Deterministic
// seeded RNG: failures reproduce exactly.
function mulberry32(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

test("parseArgs property: generated valid streams parse, last value wins, no token consumed twice (#3469)", () => {
  const rand = mulberry32(0x3469c);
  const flags = ["--out-dir", "--account-id"];
  const plainValues = ["out", "a/b", "x", "-single", "probeacct", "19222"];
  const defaults = parseArgs([]);
  for (let iter = 0; iter < 250; iter++) {
    const pairs = [];
    const n = Math.floor(rand() * 5);
    for (let k = 0; k < n; k++) {
      pairs.push([flags[Math.floor(rand() * flags.length)], plainValues[Math.floor(rand() * plainValues.length)]]);
    }
    const argv = pairs.flat();
    const parsed = parseArgs(argv);
    const last = (flag) => pairs.filter(([f]) => f === flag).at(-1)?.[1];
    assert.equal(parsed.outDir, last("--out-dir") ?? defaults.outDir, JSON.stringify(argv));
    assert.equal(parsed.accountId, last("--account-id"), JSON.stringify(argv));
    // No-token-twice invariant: no parsed value is itself a flag token.
    for (const v of [parsed.outDir, parsed.accountId]) if (v !== undefined) assert.ok(!String(v).startsWith("--"), JSON.stringify(argv));
  }
});

test("parseArgs property: generated malformed streams are usage errors naming the offending flag (#3469)", () => {
  const rand = mulberry32(0x3469d);
  const flags = ["--out-dir", "--account-id"];
  for (let iter = 0; iter < 250; iter++) {
    const kind = Math.floor(rand() * 3);
    const flag = flags[Math.floor(rand() * flags.length)];
    if (kind === 0) {
      const other = flags[Math.floor(rand() * flags.length)];
      expectUsage([flag, other, "x"], flag);
    } else if (kind === 1) {
      expectUsage(["--out-dir", "out", flag], flag);
    } else {
      const bogus = "--bogus-" + Math.floor(rand() * 1000);
      expectUsage(["--out-dir", "out", bogus, "x"], bogus);
    }
  }
});
