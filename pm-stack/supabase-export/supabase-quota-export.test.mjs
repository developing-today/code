// Offline tests for pm-stack/supabase-export (#1050 / queue task 2985).
// No network, no Bitwarden: the Supabase API is an injected fake fetch.
import { describe, expect, test } from "bun:test";
import {
  ExportError,
  FREE_LIMITS,
  PAID_TRIGGER_RATIO,
  billedFromAddons,
  buildMetrics,
  failureDocument,
  parseDbSize,
  parseRequestCount,
  run,
} from "./supabase-quota-export.mjs";

const REF = "nltgxorqwxhgfzseiirt";

function response(status, body) {
  return {
    status,
    text: async () => (typeof body === "string" ? body : JSON.stringify(body)),
  };
}

/** Fake Management API: muse-dev answers like the live probe of 2026-10-29. */
function fakeFetch({ dbBytes = 1_153_939_123, bucketCount = 0, addonsStatus = 200, addons = [] } = {}) {
  const calls = [];
  const impl = async (url, init) => {
    calls.push({ url, method: init?.method ?? "GET", authorization: init?.headers?.Authorization });
    // The real Management API refuses unauthenticated calls; the fake does
    // too, so a probe that forgets the bearer token cannot pass silently.
    if (init?.headers?.Authorization !== "Bearer sbp_fake") return response(401, { message: "Unauthorized" });
    const path = url.replace("https://api.supabase.com/v1", "");
    if (path === `/projects/${REF}`) {
      return response(200, {
        ref: REF,
        name: "muse-dev",
        organization_slug: "crwkmndylaagtbprrljn",
        status: "ACTIVE_HEALTHY",
      });
    }
    if (path === `/projects/${REF}/database/query`) return response(dbBytes == null ? 500 : 201, [{ bytes: dbBytes }]);
    if (path === `/projects/${REF}/storage/buckets`) return response(200, Array.from({ length: bucketCount }, (_, i) => ({ id: `b${i}` })));
    if (path === `/projects/${REF}/analytics/endpoints/usage.api-requests-count`) return response(200, { result: [{ count: 185893 }] });
    if (path === `/projects/${REF}/billing/addons`) return response(addonsStatus, { selected_addons: addons, available_addons: [] });
    if (path === `/organizations/crwkmndylaagtbprrljn`) return response(403, { message: "Forbidden" });
    return response(404, { message: `Cannot GET ${path}` });
  };
  return { impl, calls };
}

describe("supabase-quota-export", () => {
  test("measures db size against the Free limit and flags the paid trigger", async () => {
    const { impl } = fakeFetch();
    const doc = await run({ token: "sbp_fake", ref: REF, fetchImpl: impl });
    expect(doc.service).toBe("supabase");
    expect(doc.source).toBe("file-export/supabase");
    const db = doc.metrics.find((m) => m.id === "db-size");
    expect(db.used).toBeCloseTo(1.154, 2);
    expect(db.limit).toBe(FREE_LIMITS["db-size"]);
    expect(db.used / db.limit).toBeGreaterThanOrEqual(PAID_TRIGGER_RATIO);
    expect(db.flag).toBe("approaching-paid-trigger");
    expect(db.source).toContain("pg_database_size");
  });

  test("every Management API call, probes included, carries the bearer token", async () => {
    const { impl, calls } = fakeFetch();
    await run({ token: "sbp_fake", ref: REF, fetchImpl: impl });
    expect(calls.length).toBeGreaterThan(5);
    expect(calls.every((c) => c.authorization === "Bearer sbp_fake")).toBe(true);
  });

  test("records the egress scope gap without inventing a value", async () => {
    const { impl } = fakeFetch();
    const doc = await run({ token: "sbp_fake", ref: REF, fetchImpl: impl });
    const egress = doc.metrics.find((m) => m.id === "egress");
    expect(egress.used).toBeNull();
    expect(egress.limit).toBe(FREE_LIMITS.egress);
    expect(egress.detail).toContain("185893");
    expect(egress.flag).toBeUndefined();
    const egressProbe = doc.scope.find((s) => s.path.endsWith("usage.egress"));
    expect(egressProbe.status).toBe(404);
    const orgProbe = doc.scope.find((s) => s.endpoint === "organizations/:slug");
    expect(orgProbe.status).toBe(403);
    expect(orgProbe.note).toContain("scope gap");
  });

  test("storage is zero when the project has no buckets, and billing is a measured addon read", async () => {
    const { impl } = fakeFetch({ bucketCount: 0 });
    const doc = await run({ token: "sbp_fake", ref: REF, fetchImpl: impl });
    const storage = doc.metrics.find((m) => m.id === "storage");
    expect(storage.used).toBe(0);
    expect(storage.limit).toBe(FREE_LIMITS.storage);
    expect(doc.billing).toMatchObject({ plan: "free", billedUsd: 0, measured: true });
    expect(doc.status).toBe("partial"); // 403/404 probes were recorded
  });

  test("an unreadable db size leaves used null rather than zero", async () => {
    const { impl } = fakeFetch({ dbBytes: null });
    const doc = await run({ token: "sbp_fake", ref: REF, fetchImpl: impl });
    const db = doc.metrics.find((m) => m.id === "db-size");
    expect(db.used).toBeNull();
    expect(db.flag).toBeUndefined();
  });

  test("a token the API refuses is a scope-denied failure with no values", async () => {
    const impl = async () => response(401, { message: "Unauthorized" });
    const err = await run({ token: "sbp_fake", ref: REF, fetchImpl: impl }).catch((e) => e);
    expect(err).toBeInstanceOf(ExportError);
    expect(err.status).toBe("scope-denied");
    expect(err.exitCode).toBe(4);
  });

  test("a missing token is a no-token failure", async () => {
    const err = await run({ token: null, ref: REF }).catch((e) => e);
    expect(err.status).toBe("no-token");
  });

  test("failure documents carry no metric or billing values", () => {
    const doc = failureDocument({ attemptedAt: "2026-10-29T10:00:00Z", status: "fetch-failed", detail: "HTTP 502" });
    expect(doc.metrics).toBeUndefined();
    expect(doc.billing).toBeUndefined();
    expect(doc.measuredAt).toBeNull();
  });

  test("parsers reject shape drift", () => {
    expect(() => parseDbSize([{ bytes: "nope" }])).toThrow(ExportError);
    expect(parseRequestCount({ result: [{ count: "12" }] })).toBe(12);
    expect(parseRequestCount({ result: [] })).toBeNull();
    expect(() => billedFromAddons({})).toThrow(ExportError);
    expect(billedFromAddons({ selected_addons: [{ price: { amount: 25 } }] })).toBe(25);
  });

  test("billedFromAddons fails closed on unpriced or negative selected addons (#2492)", () => {
    expect(() => billedFromAddons({ selected_addons: [{}] })).toThrow(ExportError);
    expect(() => billedFromAddons({ selected_addons: [{ price: {} }] })).toThrow(ExportError);
    expect(() => billedFromAddons({ selected_addons: [{ price: { amount: null } }] })).toThrow(ExportError);
    expect(() => billedFromAddons({ selected_addons: [{ price: { amount: "25" } }] })).toThrow(ExportError);
    expect(() => billedFromAddons({ selected_addons: [{ price: { amount: -25 } }] })).toThrow(ExportError);
    expect(() => billedFromAddons({ selected_addons: [{ price: { amount: 25 } }, { price: {} }] })).toThrow(ExportError);
    expect(billedFromAddons({ selected_addons: [] })).toBe(0);
    expect(billedFromAddons({ selected_addons: [{ price: { amount: 0 } }] })).toBe(0);
    expect(billedFromAddons({ selected_addons: [{ price: { amount: 25 } }, { price: { amount: 5 } }] })).toBe(30);
  });

  test("a drifted addon yields billing null while metrics still publish (#2492)", async () => {
    const { impl } = fakeFetch({ addons: [{ price: { amount: 25 } }, { name: "unpriced-addon" }] });
    const doc = await run({ token: "sbp_fake", ref: REF, fetchImpl: impl });
    expect(doc.billing).toBeNull();
    expect(doc.metrics.find((m) => m.id === "db-size").used).toBeCloseTo(1.154, 2);
  });

  test("buildMetrics never emits a paid trigger for an unmeasured metric", () => {
    const metrics = buildMetrics({ dbBytes: null, bucketCount: null, requestTotal: null });
    expect(metrics.every((m) => m.flag === undefined)).toBe(true);
  });
});
