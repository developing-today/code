// #15483: the #5863 storage contract, the #5868 budget ladder and the
// #5867 backend manifest on the hosted store. The pure module
// (src/storage-contract.ts) first, then the same rules through the
// real UserDO, in the harness style of usage.test.ts and
// bridge-do.test.ts (in-memory SQLite, in-memory bucket).
//
// Oracles:
//   #5863 doc/2026-11-17_decision_id5863_storage_contract.md
//   #5868 doc/2026-11-08_decision_id5868_cost_limits_guardrails.md
//   #5867 doc/2026-11-12_decision_id5867_backend_migration.md
import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import {
  BACKEND_CAPABILITIES,
  CONTRACT_VERSION,
  DEFAULT_LADDER_THRESHOLDS,
  DEGRADE_ORDER,
  HDR_PLACEMENT_GENERATION,
  HDR_READ_AT_WATERMARK,
  HDR_STORAGE_WATERMARK,
  LADDER_POLICY,
  LADDER_STEPS,
  OP_CLASSES,
  REFUSAL_HTTP_STATUS,
  REFUSAL_KINDS,
  admit,
  advanceManifest,
  checkContractVersion,
  checkPlacementGeneration,
  checkReadAtWatermark,
  checkSnapshotDelta,
  createManifest,
  forkRefusal,
  ladderStep,
  missingCapabilities,
  opAllowed,
  parseWatermark,
  validThresholds,
  validateManifest,
  worstStep,
  type LadderStep,
  type StorageRefusal,
} from "../src/storage-contract.ts";
import { HDR_BINDING, HDR_IROH, HDR_JURISDICTION, HDR_ROUTED, UserDO } from "../src/user-do.ts";
import { authTranscript, bodyHashHex } from "../src/bridge-auth.ts";
import { bytesToHex, importIdKey, type IdKey } from "../src/ed25519.ts";
import { signEvent, type SignedEvent } from "../src/event.ts";
import { encodeBatch } from "../src/bridge-session.ts";

const A = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const STORE = { store: A, backend: "do_sqlite" as const };

// ===== pure: the #5868 ladder =========================================

test("ladder: each step is entered at its threshold", () => {
  // #5868 decision 3: warn at 50 %, degraded at 75 %, refused at 90 %;
  // 100 % is still `refused` (the hard limit is admit's, below).
  const cases: [number, LadderStep][] = [
    [0, "normal"],
    [49, "normal"],
    [50, "warning"],
    [74, "warning"],
    [75, "degraded"],
    [89, "degraded"],
    [90, "refused"],
    [100, "refused"],
    [120, "refused"],
  ];
  for (const [used, step] of cases) assert.equal(ladderStep(used, 100), step, `${used} of 100`);
  // Fractions, not absolute numbers: the same steps at another ceiling.
  assert.equal(ladderStep(9, 10), "refused");
  assert.equal(ladderStep(7, 10), "warning");
  // #5868 decision 2: budgets are configuration; no ceiling, no ladder.
  assert.equal(ladderStep(1_000_000, null), "normal");
  // A zero ceiling is fully spent.
  assert.equal(ladderStep(0, 0), "refused");
  assert.deepEqual(DEFAULT_LADDER_THRESHOLDS, { warning: 0.5, degraded: 0.75, refused: 0.9, hysteresis: 0.1 });
  assert.equal(validThresholds(DEFAULT_LADDER_THRESHOLDS), true);
  assert.equal(validThresholds({ warning: 0.8, degraded: 0.75, refused: 0.9, hysteresis: 0.1 }), false);
});

test("ladder: hysteresis holds a step until 10 points below its entry, so it never flaps", () => {
  // #5868 decision 3 ("clear each state only after usage is below its
  // entry threshold by at least 10 percentage points") and invariant 5
  // (drive a shard across a threshold repeatedly; assert no flapping).
  let step: LadderStep = "normal";
  const seen: LadderStep[] = [];
  for (const used of [74, 76, 74, 76, 74, 70, 66, 65, 76, 64]) {
    step = ladderStep(used, 100, step);
    seen.push(step);
  }
  assert.deepEqual(seen, [
    "warning", // 74: below the 75 % entry
    "degraded", // 76: entered
    "degraded", // 74: held (exit is below 65 %)
    "degraded",
    "degraded",
    "degraded", // 70: held
    "degraded", // 66: held
    "degraded", // 65: held, exactly at the exit line
    "degraded",
    "warning", // 64: cleared
  ]);
  // Refused clears below 80 %, and a big drop lands on the step its
  // own hysteresis allows.
  assert.equal(ladderStep(81, 100, "refused"), "refused");
  assert.equal(ladderStep(79, 100, "refused"), "degraded");
  assert.equal(ladderStep(62, 100, "refused"), "warning");
  assert.equal(ladderStep(10, 100, "refused"), "normal");
  // Hysteresis never holds a step the store was not in: going up
  // needs the full entry threshold.
  assert.equal(ladderStep(85, 100, "warning"), "degraded");
  assert.equal(ladderStep(85, 100, "degraded"), "degraded");
});

test("ladder: per-step policy, journal appends allowed at every step", () => {
  // #5868 decision 4: degraded sheds (a) advisory, (b) non-journal bulk
  // reads, (c) deferrable non-journal writes; refused (decision 3, 90 %)
  // refuses non-journal work; (d) journal appends are never shed.
  for (const step of ["normal", "warning"] as const) {
    for (const op of OP_CLASSES) assert.equal(opAllowed(step, op), true, `${step} ${op}`);
  }
  assert.deepEqual(
    OP_CLASSES.filter((op) => !opAllowed("degraded", op)),
    ["deferrable_write", "bulk_read", "advisory_read"],
  );
  assert.deepEqual(
    OP_CLASSES.filter((op) => !opAllowed("refused", op)),
    ["write", "deferrable_write", "bulk_read", "advisory_read"],
  );
  for (const step of LADDER_STEPS) {
    assert.equal(LADDER_POLICY[step].journal_append, true, `journal appends at ${step}`);
    // Read-only and exportable at cap (#3811 / PR #10132), and deletes
    // are how a capped user recovers headroom.
    assert.equal(LADDER_POLICY[step].read, true);
    assert.equal(LADDER_POLICY[step].export, true);
    assert.equal(LADDER_POLICY[step].reclaim, true);
  }
});

test("ladder: the degrade order is a fixed prefix order, as a property", () => {
  // #5868 invariant 4: "advisory reads shed before journal writes,
  // always; the order is a property test, not a comment".
  assert.deepEqual(DEGRADE_ORDER, ["advisory_read", "bulk_read", "deferrable_write", "write", "journal_append"]);
  let previousShed = 0;
  for (const step of LADDER_STEPS) {
    const shed = DEGRADE_ORDER.filter((op) => !opAllowed(step, op));
    // Whatever a step sheds is a prefix of the order …
    assert.deepEqual(shed, DEGRADE_ORDER.slice(0, shed.length), step);
    // … that only grows as the steps worsen …
    assert.ok(shed.length >= previousShed, step);
    previousShed = shed.length;
    // … and never reaches the journal.
    assert.equal(shed.includes("journal_append"), false, step);
  }
  // #5868 decision 7: a world is at its worst shard's step.
  assert.equal(worstStep(["normal", "degraded", "warning"]), "degraded");
  assert.equal(worstStep([]), "normal");
});

test("admit: at the refused step a journal append succeeds and a non-journal write is a typed limit refusal", () => {
  // #5868 decision 3 (at 90 % "reserve the remaining write budget for
  // signed journal appends"), decision 5 (the refusal names store,
  // backend, dimension, threshold and operator action), #5863 §5.1.
  const journal = admit({ op: "journal_append", delta: 5, used: 92, ceiling: 100, ...STORE });
  assert.equal(journal.ok, true);
  assert.equal(journal.ok && journal.value.step, "refused");

  const write = admit({ op: "write", delta: 5, used: 92, ceiling: 100, ...STORE });
  assert.equal(write.ok, false);
  const refusal = (write as { refusal: StorageRefusal }).refusal;
  assert.deepEqual(refusal, {
    kind: "limit",
    limit: "max_stored_bytes",
    dimension: "stored_bytes",
    step: "refused",
    op: "write",
    used: 92,
    ceiling: 100,
    requested: 5,
    threshold: 0.9,
    store: A,
    backend: "do_sqlite",
    action: "the remaining budget is reserved for journal appends; delete data or raise the ceiling",
  });

  // Degraded sheds deferrable writes but not client writes.
  assert.equal(admit({ op: "deferrable_write", delta: 1, used: 80, ceiling: 100, ...STORE }).ok, false);
  assert.equal(admit({ op: "write", delta: 1, used: 80, ceiling: 100, ...STORE }).ok, true);
  // Reclaim passes at refused.
  assert.equal(admit({ op: "reclaim", delta: -10, used: 100, ceiling: 100, ...STORE }).ok, true);
  // No ceiling configured: everything passes at normal.
  const free = admit({ op: "write", delta: 1e9, used: 1e9, ceiling: null, ...STORE });
  assert.deepEqual(free, { ok: true, value: { step: "normal" } });
});

test("admit: the hard ceiling refuses every class, journal appends included, and says so", () => {
  // #5868 decision 3 ("at 100 % … a typed refusal for all writes") and
  // invariant 1 (a refused journal append is reported, never
  // swallowed); #5863 §4.1 (a store asked to exceed a declared limit
  // refuses with the typed limit refusal).
  for (const op of ["journal_append", "write"] as const) {
    const r = admit({ op, delta: 11, used: 90, ceiling: 100, ...STORE });
    assert.equal(r.ok, false, op);
    const refusal = (r as { refusal: StorageRefusal }).refusal;
    assert.equal(refusal.kind, "limit");
    assert.equal(refusal.kind === "limit" && refusal.threshold, 1);
    assert.equal(refusal.kind === "limit" && refusal.op, op);
  }
  // Exactly filling the ceiling is allowed for a journal append.
  assert.equal(admit({ op: "journal_append", delta: 10, used: 90, ceiling: 100, ...STORE }).ok, true);
});

// ===== pure: the #5863 contract =======================================

test("typed refusals: exactly the five kinds, each with an HTTP status", () => {
  // #5863 §5.1: every operation fails only as limit, unavailable,
  // fork, version or verification.
  assert.deepEqual([...REFUSAL_KINDS], ["limit", "unavailable", "fork", "version", "verification"]);
  assert.deepEqual(Object.keys(REFUSAL_HTTP_STATUS).sort(), [...REFUSAL_KINDS].sort());
  // limit keeps the storage ceiling's established 413 (PR #10132).
  assert.equal(REFUSAL_HTTP_STATUS.limit, 413);
  assert.equal(REFUSAL_HTTP_STATUS.unavailable, 503);
});

test("typed refusals: fork carries both entries, never an overwrite", () => {
  // #5863 §2.1: a typed `fork` refusal carrying both entries.
  const r = forkRefusal("author", 7, { id: "a", bytes_hex: "00" }, { id: "b", bytes_hex: null });
  assert.deepEqual(r, {
    kind: "fork",
    author: "author",
    seq: 7,
    first: { id: "a", bytes_hex: "00" },
    second: { id: "b", bytes_hex: null },
  });
});

test("typed refusals: version — newer refused, older only through a recorded migration", () => {
  // #5863 §5.3.
  assert.equal(CONTRACT_VERSION, 1);
  assert.equal(checkContractVersion(1), null);
  assert.deepEqual(checkContractVersion(2), { kind: "version", supported: 1, received: 2, direction: "newer" });
  assert.equal(checkContractVersion("1")?.kind, "verification");
  assert.equal(checkContractVersion(0)?.kind, "verification");
});

test("read at watermark: serve at ≥ N, or refuse unavailable carrying the current watermark", () => {
  // #5863 §2.3 (the store serves a view including every entry up to N
  // or refuses with its current watermark) and §2.4 (head).
  const head = { seq: 5, timestamp: "2026-11-26T00:00:00.000Z", watermark: 5 };
  assert.deepEqual(checkReadAtWatermark(null, head), { ok: true, value: 5 });
  assert.deepEqual(checkReadAtWatermark(3, head), { ok: true, value: 5 });
  assert.deepEqual(checkReadAtWatermark(5, head), { ok: true, value: 5 });
  const ahead = checkReadAtWatermark(6, head);
  assert.equal(ahead.ok, false);
  const refusal = (ahead as { refusal: StorageRefusal }).refusal;
  assert.equal(refusal.kind, "unavailable");
  assert.equal(refusal.kind === "unavailable" && refusal.reason, "behind_watermark");
  assert.equal(refusal.kind === "unavailable" && refusal.watermark, 5);
  assert.equal(refusal.kind === "unavailable" && refusal.requested, 6);
  // A replica's lag is a lower watermark, never stale data.
  const replica = { seq: 9, timestamp: null, watermark: 4 };
  assert.equal(checkReadAtWatermark(5, replica).ok, false);

  assert.deepEqual(parseWatermark(null), { ok: true, value: null });
  assert.deepEqual(parseWatermark("0"), { ok: true, value: 0 });
  assert.deepEqual(parseWatermark("42"), { ok: true, value: 42 });
  for (const bad of ["-1", "1.5", "x", "99999999999999999"]) assert.equal(parseWatermark(bad).ok, false, bad);
});

test("snapshot plus delta: contiguous replay reaches the target; gaps and short deltas are typed refusals", () => {
  // #5863 §3.2 (snapshot + the delta after its sequence reproduce the
  // store exactly), §3.3 (never repaired silently), §5.3 (version).
  const snap = { contract_version: CONTRACT_VERSION, at_seq: 10 };
  assert.deepEqual(checkSnapshotDelta(snap, [11, 12, 13], 13), { ok: true, value: 13 });
  assert.deepEqual(
    checkSnapshotDelta(snap, [11, 12, 13, 14], 12),
    { ok: true, value: 12 },
    "entries past the target are not applied",
  );
  assert.deepEqual(checkSnapshotDelta(snap, [], 10), { ok: true, value: 10 });

  const gap = checkSnapshotDelta(snap, [11, 13], 13);
  assert.equal(!gap.ok && gap.refusal.kind, "verification");
  assert.equal(!gap.ok && gap.refusal.kind === "verification" && gap.refusal.reason, "sequence_gap");

  const misaligned = checkSnapshotDelta(snap, [10, 11], 11);
  assert.equal(
    !misaligned.ok && misaligned.refusal.kind === "verification" && misaligned.refusal.reason,
    "sequence_misaligned",
  );

  const short = checkSnapshotDelta(snap, [11, 12], 15);
  assert.equal(!short.ok && short.refusal.kind, "unavailable");
  assert.equal(!short.ok && short.refusal.kind === "unavailable" && short.refusal.watermark, 12);

  const newer = checkSnapshotDelta({ contract_version: CONTRACT_VERSION + 1, at_seq: 0 }, [], 0);
  assert.equal(!newer.ok && newer.refusal.kind, "version");
});

test("capability flags are the declared values, and gate placement only", () => {
  // #5863 §4.2: D1 offline_append false, embedded_replica false,
  // workers_native true; Turso false, true, false.
  assert.deepEqual(BACKEND_CAPABILITIES.d1, { offline_append: false, embedded_replica: false, workers_native: true });
  assert.deepEqual(BACKEND_CAPABILITIES.turso, {
    offline_append: false,
    embedded_replica: true,
    workers_native: false,
  });
  assert.deepEqual(BACKEND_CAPABILITIES.do_sqlite, {
    offline_append: false,
    embedded_replica: false,
    workers_native: true,
  });
  assert.deepEqual(missingCapabilities("d1", { embedded_replica: true }), ["embedded_replica"]);
  assert.deepEqual(missingCapabilities("turso", { embedded_replica: true }), []);
  assert.deepEqual(missingCapabilities("turso", { workers_native: true, offline_append: false }), ["workers_native"]);
});

// ===== pure: the #5867 manifest =======================================

test("manifest: created at generation 1, validated, advanced, and checked against a writer's generation", () => {
  // #5867 §2.2 (backend, placement generation, cutover sequence),
  // §2.3 (a write carries the resolved generation; a stale one is
  // refused), §3.6 (cutover publishes a new generation).
  const m = createManifest({ unit: A, backend: "do_sqlite", recordedAt: "2026-11-26T00:00:00.000Z" });
  assert.deepEqual(m, {
    manifest_version: 1,
    unit: A,
    backend: "do_sqlite",
    generation: 1,
    cutover_seq: 0,
    contract_version: CONTRACT_VERSION,
    recorded_at: "2026-11-26T00:00:00.000Z",
    signature: null,
  });
  assert.deepEqual(validateManifest(JSON.parse(JSON.stringify(m)), A), { ok: true, value: m });

  const next = advanceManifest(m, { backend: "turso", cutoverSeq: 40, recordedAt: "2026-11-27T00:00:00.000Z" });
  assert.equal(next.ok && next.value.generation, 2);
  assert.equal(next.ok && next.value.backend, "turso");
  assert.equal(next.ok && next.value.cutover_seq, 40);
  if (next.ok) {
    const back = advanceManifest(next.value, {
      backend: "do_sqlite",
      cutoverSeq: 39,
      recordedAt: "2026-11-28T00:00:00.000Z",
    });
    assert.equal(!back.ok && back.refusal.kind, "verification", "a cutover never moves backwards");
  }

  assert.equal(checkPlacementGeneration(m, null), null);
  assert.equal(checkPlacementGeneration(m, 1), null);
  const stale = checkPlacementGeneration(m, 0);
  assert.equal(stale?.kind, "unavailable");
  assert.equal(stale?.kind === "unavailable" && stale.reason, "placement_stale");
  assert.equal(stale?.kind === "unavailable" && stale.generation, 1);
});

test("manifest: D1 placement preserves the tenant database id", () => {
  const base = createManifest({
    unit: A,
    backend: "d1",
    databaseId: "d1-tenant-a",
    recordedAt: "2026-11-26T00:00:00.000Z",
  });
  const checked = validateManifest({ ...base, database_id: "d1-tenant-a" }, A);
  assert.equal(checked.ok, true);
  if (checked.ok) {
    assert.equal((checked.value as unknown as { database_id?: string }).database_id, "d1-tenant-a");
  }
});

test("manifest: D1 placement without a database id fails closed", () => {
  const base = createManifest({
    unit: A,
    backend: "d1",
    databaseId: "d1-tenant-a",
    recordedAt: "2026-11-26T00:00:00.000Z",
  });
  const checked = validateManifest({ ...base, database_id: undefined }, A);
  assert.equal(checked.ok, false);
  assert.equal(!checked.ok && checked.refusal.kind === "unavailable" && checked.refusal.reason, "placement_unknown");
});

test("manifest: D1 cutover records its database id and clears it on cutover away", () => {
  const current = createManifest({
    unit: A,
    backend: "do_sqlite",
    recordedAt: "2026-11-26T00:00:00.000Z",
  });
  const missingId = advanceManifest(current, {
    backend: "d1",
    cutoverSeq: 10,
    recordedAt: "2026-11-27T00:00:00.000Z",
  });
  assert.equal(missingId.ok, false);

  const toD1 = advanceManifest(current, {
    backend: "d1",
    databaseId: "d1-tenant-a",
    cutoverSeq: 10,
    recordedAt: "2026-11-27T00:00:00.000Z",
  });
  assert.equal(toD1.ok, true);
  if (toD1.ok) assert.equal(toD1.value.database_id, "d1-tenant-a");

  const awayFromD1 = advanceManifest(toD1.value, {
    backend: "turso",
    cutoverSeq: 20,
    recordedAt: "2026-11-28T00:00:00.000Z",
  });
  assert.equal(awayFromD1.ok, true);
  if (awayFromD1.ok) assert.equal("database_id" in awayFromD1.value, false);
});

test("manifest: missing, malformed or conflicting placement fails closed as placement_unknown", () => {
  // #5867 §2.4: "Missing, invalidly signed, conflicting, or too-stale
  // placement data fails closed with a typed unavailable/placement-
  // unknown result"; a newer record is a #5863 §5.3 version refusal.
  const good = createManifest({
    unit: A,
    backend: "d1",
    databaseId: "d1-tenant-a",
    recordedAt: "2026-11-26T00:00:00.000Z",
  });
  const bad: unknown[] = [
    null,
    "manifest",
    [],
    { ...good, backend: "postgres" },
    { ...good, backend: "turso" },
    { ...good, generation: 0 },
    { ...good, generation: 1.5 },
    { ...good, cutover_seq: -1 },
    { ...good, unit: "" },
    { ...good, recorded_at: "yesterday" },
    { ...good, signature: 7 },
    { ...good, manifest_version: undefined },
  ];
  for (const value of bad) {
    const r = validateManifest(value);
    assert.equal(r.ok, false, JSON.stringify(value));
    assert.equal(
      !r.ok && r.refusal.kind === "unavailable" && r.refusal.reason,
      "placement_unknown",
      JSON.stringify(value),
    );
  }
  const conflicting = validateManifest(good, "b".repeat(32));
  assert.equal(
    !conflicting.ok && conflicting.refusal.kind === "unavailable" && conflicting.refusal.reason,
    "placement_unknown",
  );
  const newer = validateManifest({ ...good, manifest_version: 2 });
  assert.equal(!newer.ok && newer.refusal.kind, "version");
  const newerContract = validateManifest({ ...good, contract_version: 2 });
  assert.equal(!newerContract.ok && newerContract.refusal.kind, "version");
});

// ===== through the UserDO =============================================

function fakeSql() {
  const db = new DatabaseSync(":memory:");
  return {
    exec(query: string, ...params: unknown[]) {
      const stmt = db.prepare(query);
      if (/^\s*select/i.test(query)) {
        const rows = stmt.all(...(params as never[])) as Record<string, unknown>[];
        return { toArray: () => rows };
      }
      stmt.run(...(params as never[]));
      return { toArray: () => [] as Record<string, unknown>[] };
    },
  };
}

function fakeBucket() {
  const objects = new Map<string, Uint8Array>();
  return {
    objects,
    async head(key: string) {
      const v = objects.get(key);
      return v ? { size: v.byteLength } : null;
    },
    async get(key: string) {
      const v = objects.get(key);
      if (!v) return null;
      return {
        size: v.byteLength,
        body: new ReadableStream({
          start(c) {
            c.enqueue(v);
            c.close();
          },
        }),
      };
    },
    async put(key: string, value: ArrayBuffer | Uint8Array) {
      objects.set(key, value instanceof Uint8Array ? value : new Uint8Array(value));
    },
    async delete(key: string) {
      objects.delete(key);
    },
  };
}

function makeDO(env: Record<string, unknown> = {}, sql = fakeSql()) {
  const state = {
    id: { name: A },
    storage: { sql },
    blockConcurrencyWhile: async <T>(fn: () => Promise<T>): Promise<T> => fn(),
  };
  return { doA: new UserDO(state as never, { BLOBS: fakeBucket(), ...env } as never), sql };
}

function routed(path: string, init: RequestInit = {}, irohHex: string | null = null): Request {
  const headers = new Headers(init.headers);
  headers.set(HDR_BINDING, A);
  headers.set(HDR_JURISDICTION, "us");
  headers.set(HDR_ROUTED, "restricted");
  if (irohHex !== null) headers.set(HDR_IROH, irohHex);
  return new Request("https://do" + path, { ...init, headers });
}

interface RefusalBody {
  error: string;
  message: string;
  refusal: StorageRefusal;
}

async function budgetStepOf(d: UserDO): Promise<string> {
  const r = await d.fetch(routed("/v1/usage"));
  assert.equal(r.status, 200);
  return ((await r.json()) as { budget_step: string }).budget_step;
}

test("DO: usage moves through the ladder, and the refused step refuses KV growth with a typed refusal", async () => {
  // #5868 decision 3 through the hosted store: the storage ceiling
  // (PR #10132) is the plan ceiling the ladder's fractions are of.
  const { doA } = makeDO({ USER_CEILING_BYTES: "100" });
  assert.equal(await budgetStepOf(doA), "normal");
  assert.equal((await doA.fetch(routed("/v1/kv/a", { method: "PUT", body: "x".repeat(50) }))).status, 200);
  assert.equal(await budgetStepOf(doA), "warning");
  assert.equal((await doA.fetch(routed("/v1/kv/b", { method: "PUT", body: "x".repeat(25) }))).status, 200);
  assert.equal(await budgetStepOf(doA), "degraded");
  // Degraded still admits client writes (only deferrable work is shed).
  assert.equal((await doA.fetch(routed("/v1/kv/c", { method: "PUT", body: "x".repeat(15) }))).status, 200);
  assert.equal(await budgetStepOf(doA), "refused");

  const refused = await doA.fetch(routed("/v1/kv/d", { method: "PUT", body: "x" }));
  assert.equal(refused.status, 413);
  const body = (await refused.json()) as RefusalBody;
  assert.equal(body.error, "budget_refused");
  assert.deepEqual(body.refusal, {
    kind: "limit",
    limit: "max_stored_bytes",
    dimension: "stored_bytes",
    step: "refused",
    op: "write",
    used: 90,
    ceiling: 100,
    requested: 1,
    threshold: DEFAULT_LADDER_THRESHOLDS.refused,
    store: A,
    backend: "do_sqlite",
    action: "the remaining budget is reserved for journal appends; delete data or raise the ceiling",
  });

  // Reads stay available at the refused step; deletes recover.
  assert.equal((await doA.fetch(routed("/v1/kv/a"))).status, 200);
  assert.equal((await doA.fetch(routed("/v1/kv/c", { method: "DELETE" }))).status, 200);
  // 75 % after holding `refused`: hysteresis lands on degraded, which
  // admits the write again.
  assert.equal(await budgetStepOf(doA), "degraded");
  assert.equal((await doA.fetch(routed("/v1/kv/d", { method: "PUT", body: "x" }))).status, 200);
});

// --- bridge helpers (as in bridge-do.test.ts) -------------------------

async function challenge(doA: UserDO, irohHex: string): Promise<string> {
  const res = await doA.fetch(routed("/v1/bridge/challenge", {}, irohHex));
  assert.equal(res.status, 200);
  return ((await res.json()) as { nonce: string }).nonce;
}

async function proofHeaders(key: IdKey, nonce: string, body: Uint8Array): Promise<Record<string, string>> {
  const transcript = authTranscript(A, nonce, "push", bodyHashHex(body));
  const signature = await key.sign(transcript);
  return { "x-id-nonce": nonce, "x-id-proof": bytesToHex(signature) };
}

async function chainOf(key: IdKey, count: number): Promise<SignedEvent[]> {
  const events: SignedEvent[] = [];
  for (let seq = 0; seq < count; seq++) {
    events.push(
      await signEvent(key, {
        seq,
        backlink: seq === 0 ? null : events[seq - 1].idBytes,
        kind: "message",
        body: new TextEncoder().encode(`body ${seq}`),
      }),
    );
  }
  return events;
}

async function push(doA: UserDO, key: IdKey, events: SignedEvent[]): Promise<Response> {
  const batch = encodeBatch(events.map((e) => e.bytes));
  const nonce = await challenge(doA, key.publicHex);
  const headers = await proofHeaders(key, nonce, batch);
  return doA.fetch(routed("/v1/bridge/push", { method: "POST", headers, body: batch as BodyInit }, key.publicHex));
}

test("DO: at the refused step a signed journal append (bridge push) still succeeds", async () => {
  // #15483 done-when: "journal appends still succeed at the refused
  // step" — #5868 decision 3 (the 90–100 % band is reserved for
  // signed journal appends) and decision 4 (d) (never shed).
  const key = await importIdKey(new Uint8Array(32).fill(71));
  const author = await importIdKey(new Uint8Array(32).fill(72));
  const events = await chainOf(author, 2);
  const eventBytes = events[0].bytes.length;
  const { doA } = makeDO({ USER_CEILING_BYTES: String(10 * eventBytes) });

  assert.equal(
    (await doA.fetch(routed("/v1/kv/fill", { method: "PUT", body: "x".repeat(9 * eventBytes) }))).status,
    200,
  );
  assert.equal(await budgetStepOf(doA), "refused");
  const kv = await doA.fetch(routed("/v1/kv/more", { method: "PUT", body: "x" }));
  assert.equal(kv.status, 413);
  assert.equal(((await kv.json()) as RefusalBody).error, "budget_refused");

  const ok = await push(doA, key, [events[0]]);
  assert.equal(ok.status, 200);
  const okBody = (await ok.json()) as { outcomes: { kind: string }[] };
  assert.deepEqual(
    okBody.outcomes.map((o) => o.kind),
    ["accepted"],
  );
  // The append committed: the store's watermark advanced past the KV write.
  assert.equal(ok.headers.get(HDR_STORAGE_WATERMARK), "2");

  // The ceiling itself is the one thing that refuses a journal append,
  // and it does so as a typed, reported refusal (#5868 invariant 1).
  const full = await push(doA, key, [events[1]]);
  assert.equal(full.status, 413);
  const fullBody = (await full.json()) as RefusalBody;
  assert.equal(fullBody.error, "ceiling_exceeded");
  assert.equal(fullBody.refusal.kind, "limit");
  assert.equal(fullBody.refusal.kind === "limit" && fullBody.refusal.op, "journal_append");
  assert.equal(fullBody.refusal.kind === "limit" && fullBody.refusal.threshold, 1);
});

test("DO: reads honour a requested watermark and refuse one the store has not reached", async () => {
  // #5863 §2.3: read-your-writes is a read at the sequence the write
  // returned; a store below the requested watermark refuses with
  // `unavailable` carrying its current watermark.
  const { doA } = makeDO();
  const put = await doA.fetch(routed("/v1/kv/k", { method: "PUT", body: "v" }));
  assert.equal(put.status, 200);
  assert.equal(put.headers.get(HDR_STORAGE_WATERMARK), "1");

  const at = await doA.fetch(routed("/v1/kv/k", { headers: { [HDR_READ_AT_WATERMARK]: "1" } }));
  assert.equal(at.status, 200);
  assert.equal(at.headers.get(HDR_STORAGE_WATERMARK), "1");
  assert.equal(((await at.json()) as { value: string }).value, "v");

  const ahead = await doA.fetch(routed("/v1/kv/k", { headers: { [HDR_READ_AT_WATERMARK]: "2" } }));
  assert.equal(ahead.status, 503);
  const body = (await ahead.json()) as RefusalBody;
  assert.equal(body.error, "watermark_unavailable");
  assert.equal(body.refusal.kind, "unavailable");
  assert.equal(body.refusal.kind === "unavailable" && body.refusal.reason, "behind_watermark");
  assert.equal(body.refusal.kind === "unavailable" && body.refusal.watermark, 1);
  assert.equal(body.refusal.kind === "unavailable" && body.refusal.requested, 2);

  const malformed = await doA.fetch(routed("/v1/kv/k", { headers: { [HDR_READ_AT_WATERMARK]: "soon" } }));
  assert.equal(malformed.status, 400);
  assert.equal(((await malformed.json()) as { error: string }).error, "invalid_watermark");

  // Deletes and further writes advance the head.
  assert.equal((await doA.fetch(routed("/v1/kv/k", { method: "DELETE" }))).headers.get(HDR_STORAGE_WATERMARK), "2");
  const storage = (await (await doA.fetch(routed("/v1/storage"))).json()) as {
    head: { seq: number; watermark: number };
  };
  assert.equal(storage.head.seq, 2);
  assert.equal(storage.head.watermark, 2);
});

test("DO: the store records and serves its backend manifest, refusing stale or invalid placement", async () => {
  // #5867 §2.2 (the manifest names backend, generation, cutover
  // sequence), §2.3 (stale generation refused), §2.4 (invalid
  // placement fails closed); #5863 §4 (declared limits and flags).
  const { doA, sql } = makeDO({ USER_CEILING_BYTES: "1000" });
  const res = await doA.fetch(routed("/v1/storage"));
  assert.equal(res.status, 200);
  const storage = (await res.json()) as {
    contract_version: number;
    manifest: { unit: string; backend: string; generation: number; cutover_seq: number; contract_version: number };
    capabilities: unknown;
    limits: { max_stored_bytes: number | null; retention_floor_days: number | null };
    budget: { step: string; ceiling: number | null };
  };
  assert.equal(storage.contract_version, CONTRACT_VERSION);
  assert.equal(storage.manifest.unit, A);
  assert.equal(storage.manifest.backend, "do_sqlite");
  assert.equal(storage.manifest.generation, 1);
  assert.equal(storage.manifest.cutover_seq, 0);
  assert.equal(storage.manifest.contract_version, CONTRACT_VERSION);
  assert.deepEqual(storage.capabilities, BACKEND_CAPABILITIES.do_sqlite);
  assert.equal(storage.limits.max_stored_bytes, 1000);
  assert.equal(storage.limits.retention_floor_days, 30);
  assert.equal(storage.budget.step, "normal");
  assert.equal(storage.budget.ceiling, 1000);

  // The manifest is persisted: a second object over the same store
  // serves the same record.
  const again = makeDO({}, sql).doA;
  const second = (await (await again.fetch(routed("/v1/storage"))).json()) as { manifest: unknown };
  assert.deepEqual(second.manifest, storage.manifest);

  // A writer presenting the current generation passes; a stale one is refused.
  const current = await doA.fetch(
    routed("/v1/kv/k", { method: "PUT", body: "v", headers: { [HDR_PLACEMENT_GENERATION]: "1" } }),
  );
  assert.equal(current.status, 200);
  const stale = await doA.fetch(
    routed("/v1/kv/k", { method: "PUT", body: "w", headers: { [HDR_PLACEMENT_GENERATION]: "0" } }),
  );
  assert.equal(stale.status, 503);
  const staleBody = (await stale.json()) as RefusalBody;
  assert.equal(staleBody.error, "placement_stale");
  assert.equal(staleBody.refusal.kind === "unavailable" && staleBody.refusal.reason, "placement_stale");
  assert.equal(
    ((await (await doA.fetch(routed("/v1/kv/k"))).json()) as { value: string }).value,
    "v",
    "the stale write did not land",
  );

  // A corrupted placement record fails every route closed.
  sql.exec("UPDATE meta SET value = ? WHERE key = ?", JSON.stringify({ backend: "do_sqlite" }), "backend_manifest");
  const broken = await doA.fetch(routed("/v1/kv/k"));
  assert.equal(broken.status, 503);
  const brokenBody = (await broken.json()) as RefusalBody;
  assert.equal(brokenBody.error, "placement_unknown");
  assert.equal(brokenBody.refusal.kind === "unavailable" && brokenBody.refusal.reason, "placement_unknown");
});
