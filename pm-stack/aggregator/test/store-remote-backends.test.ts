// #6815: the four aggregator stores move to a remote store (Turso) behind
// PM_AGG_STORE_BACKEND. These tests pin the one-backend-at-a-time
// contract: in turso mode the store reads/writes only its remote and
// creates no local canonical file; in file mode no remote is touched.
//
// The fakes are the oracles: each implements the store's remote
// interface (an in-memory canonical copy), so a fresh boot with an empty
// local state dir can be checked to reconstruct entirely from the remote.
import { describe, expect, test } from "bun:test";
import { existsSync, mkdtempSync, readdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { ActionsStore, type ActionsRemote, type DashAction } from "../src/actions.ts";
import {
  IngestionStore,
  type ActivityRemote,
  type GapRecord,
  type IngestionFile,
  type SourceCheckpoint,
} from "../src/activity.ts";
import { AaLastGoodStore, type DocumentRemote } from "../src/aa-last-good.ts";
import { SnapshotStore, snapshotKey, type SnapshotsRemote } from "../src/snapshots.ts";
import type { DashComment, SnapshotRecord } from "../src/model.ts";
import { CommentsStore, type CommentsRemote } from "../src/comments.ts";
import { AA_LAST_GOOD_DOCUMENT, parseStoreBackend } from "../src/store-backend.ts";
import {
  actionFromRow,
  actionRowValues,
  commentFromRow,
  commentRowValues,
  createTursoActionsRemote,
  createTursoCommentsRemote,
  createTursoDocumentRemote,
  createTursoSnapshotsRemote,
  snapshotFromRow,
  snapshotRowValues,
} from "../src/remote-stores.ts";
import type { SqlClient } from "../src/sql-client.ts";

const tmp = (label: string): string => mkdtempSync(join(tmpdir(), `agg-stores-${label}-`));

const drew = { kind: "drew" as const, id: "drew", label: "Drew" };

class FakeActionsRemote implements ActionsRemote {
  rows: DashAction[] = [];
  async loadAll(): Promise<DashAction[]> {
    return structuredClone(this.rows);
  }
  async saveAll(actions: readonly DashAction[]): Promise<void> {
    this.rows = structuredClone([...actions]);
  }
}

class FakeActivityRemote implements ActivityRemote {
  data: IngestionFile | null = null;
  saves = 0;
  async load(): Promise<IngestionFile | null> {
    return this.data ? structuredClone(this.data) : null;
  }
  async save(data: IngestionFile): Promise<void> {
    this.saves++;
    this.data = structuredClone(data);
  }
}

class FakeSnapshotsRemote implements SnapshotsRemote {
  rows = new Map<string, SnapshotRecord>();
  async readAll(): Promise<SnapshotRecord[]> {
    return [...this.rows.values()].map((r) => structuredClone(r)).sort((a, b) => Date.parse(a.ts) - Date.parse(b.ts));
  }
  async append(records: readonly SnapshotRecord[]): Promise<number> {
    let inserted = 0;
    for (const r of records) {
      const key = snapshotKey(r);
      if (this.rows.has(key)) continue;
      this.rows.set(key, structuredClone(r));
      inserted++;
    }
    return inserted;
  }
}

class FakeDocumentRemote implements DocumentRemote {
  docs = new Map<string, { version: number; document: unknown }>();
  async load(store: string) {
    const hit = this.docs.get(store);
    return hit ? structuredClone(hit) : null;
  }
  async save(store: string, version: number, document: unknown): Promise<void> {
    this.docs.set(store, { version, document: structuredClone(document) });
  }
}

function snapshot(over: Partial<SnapshotRecord> = {}): SnapshotRecord {
  return {
    ts: "2026-11-01T00:00:00.000Z",
    serviceId: "openai",
    metricId: "credits",
    value: 10,
    unit: "usd",
    source: "api",
    observedBy: "collector",
    ...over,
  };
}

function checkpoint(over: Partial<SourceCheckpoint> = {}): SourceCheckpoint {
  return {
    source: "kaneo",
    lastSourceId: "d-1",
    lastEventAt: "2026-11-01T00:00:00.000Z",
    deliveries: 1,
    duplicates: 0,
    updatedAt: "2026-11-01T00:00:00.000Z",
    ...over,
  };
}

function gap(over: Partial<GapRecord> = {}): GapRecord {
  return {
    id: "gap-1",
    source: "kaneo",
    from: "2026-11-01T00:00:00.000Z",
    to: "2026-11-01T01:00:00.000Z",
    status: "open",
    detectedAt: "2026-11-01T01:00:00.000Z",
    ...over,
  };
}

describe("PM_AGG_STORE_BACKEND parsing", () => {
  test("defaults to file, accepts the two names, rejects anything else", () => {
    expect(parseStoreBackend({})).toBe("file");
    expect(parseStoreBackend({ PM_AGG_STORE_BACKEND: "  " })).toBe("file");
    expect(parseStoreBackend({ PM_AGG_STORE_BACKEND: "file" })).toBe("file");
    expect(parseStoreBackend({ PM_AGG_STORE_BACKEND: "turso" })).toBe("turso");
    expect(() => parseStoreBackend({ PM_AGG_STORE_BACKEND: "postgres" })).toThrow(/PM_AGG_STORE_BACKEND/);
    expect(() => parseStoreBackend({ PM_AGG_STORE_BACKEND: "TURSO" })).toThrow(/PM_AGG_STORE_BACKEND/);
  });

  test("the retired supabase backend fails loudly, naming the valid values", () => {
    expect(() => parseStoreBackend({ PM_AGG_STORE_BACKEND: "supabase" })).toThrow(
      'Invalid PM_AGG_STORE_BACKEND="supabase"; expected "file" or "turso"',
    );
  });
});

describe("actions store remote backend", () => {
  test("writes only the remote and never creates the local file", async () => {
    const dir = tmp("actions");
    const file = join(dir, "actions.json");
    const remote = new FakeActionsRemote();
    const store = new ActionsStore(file, undefined, remote);
    const res = await store.add({
      clientToken: "ct-remote-1-abcdefgh",
      kind: "triage",
      verb: "acknowledge",
      objectRef: "orch:item-1",
      payload: {},
      author: drew,
    });
    expect(res.created).toBe(true);
    expect(remote.rows).toHaveLength(1);
    expect(remote.rows[0].id).toBe(res.action.id);
    expect(existsSync(file)).toBe(false);
    expect(readdirSync(dir)).toEqual([]);

    // A fresh store over the same remote reconstructs the state.
    const again = new ActionsStore(file, undefined, remote);
    expect((await again.all()).map((a) => a.id)).toEqual([res.action.id]);
  });
});

describe("activity store remote backend", () => {
  test("loads from and persists to the remote, no local file", async () => {
    const dir = tmp("activity");
    const file = join(dir, "activity-ingestion.json");
    const remote = new FakeActivityRemote();
    remote.data = {
      version: 1,
      checkpoints: { kaneo: checkpoint() },
      seen: { kaneo: ["d-1"] },
      gaps: [gap()],
    };
    const store = new IngestionStore(file, remote);
    expect(await store.checkpoint("kaneo")).toEqual(checkpoint());
    expect(await store.seenIds("kaneo")).toEqual(["d-1"]);
    expect((await store.gaps())[0].id).toBe("gap-1");

    await store.recordGap("kaneo", "2026-11-01T02:00:00.000Z", "2026-11-01T03:00:00.000Z");
    expect(remote.saves).toBeGreaterThan(0);
    expect(remote.data?.gaps.map((g) => g.id)).toContain("gap-1");
    expect(existsSync(file)).toBe(false);
    expect(readdirSync(dir)).toEqual([]);
  });
});

describe("usage snapshots store remote backend", () => {
  test("appends deduped records to the remote and never creates the dir", async () => {
    const dir = join(tmp("snapshots"), "snapshots");
    const remote = new FakeSnapshotsRemote();
    const store = new SnapshotStore(dir, remote);
    const first = snapshot();
    const dup = snapshot();
    const second = snapshot({ ts: "2026-11-01T01:00:00.000Z", value: 11 });
    expect(await store.append([first, dup, second])).toBe(2);
    expect(await store.append([first, second])).toBe(0);
    expect(remote.rows.size).toBe(2);

    const fresh = new SnapshotStore(dir, remote);
    expect((await fresh.readAll()).map(snapshotKey)).toEqual([snapshotKey(first), snapshotKey(second)]);
    expect(await fresh.rollupKeys()).toEqual([]);

    const report = await fresh.compact({ now: new Date("2026-11-15T00:00:00Z") });
    expect(report.months).toEqual([]);
    expect(existsSync(dir)).toBe(false);
  });
});

describe("aa-last-good store remote backend", () => {
  test("document mode round-trips and writes no file", async () => {
    const dir = tmp("aa");
    const file = join(dir, "aa-last-good.json");
    const remote = new FakeDocumentRemote();
    const store = new AaLastGoodStore(file, remote);
    expect(await store.read()).toBeNull();
    await store.write({ batch: "b1", section: { status: "ok" } });
    expect(remote.docs.get(AA_LAST_GOOD_DOCUMENT)?.document).toEqual({ batch: "b1", section: { status: "ok" } });
    expect(await store.read()).toEqual({ batch: "b1", section: { status: "ok" } });
    expect(existsSync(file)).toBe(false);
    expect(readdirSync(dir)).toEqual([]);
  });

  test("file mode still round-trips when no remote is selected", async () => {
    const dir = tmp("aa-file");
    const file = join(dir, "aa-last-good.json");
    const store = new AaLastGoodStore(file);
    await store.write({ batch: "b2" });
    expect(await store.read()).toEqual({ batch: "b2" });
  });
});

describe("empty local state dir boots from the remote (#6815 acceptance)", () => {
  test("a fresh state dir reconstructs all four stores and stays empty", async () => {
    const stateDir = tmp("fresh-boot");
    const actionsRemote = new FakeActionsRemote();
    actionsRemote.rows = [
      {
        id: "da-seed",
        clientToken: "ct-seed-1-abcdefgh",
        kind: "triage",
        verb: "acknowledge",
        objectRef: "orch:seed",
        payload: {},
        author: drew,
        createdAt: "2026-11-01T00:00:00.000Z",
        state: "new",
      },
    ];
    const activityRemote = new FakeActivityRemote();
    activityRemote.data = {
      version: 1,
      checkpoints: { kaneo: checkpoint() },
      seen: { kaneo: ["d-1"] },
      gaps: [],
    };
    const snapshotsRemote = new FakeSnapshotsRemote();
    await snapshotsRemote.append([snapshot()]);
    const docRemote = new FakeDocumentRemote();
    await docRemote.save(AA_LAST_GOOD_DOCUMENT, 1, { batch: "seeded" });

    const actions = new ActionsStore(join(stateDir, "actions.json"), undefined, actionsRemote);
    const activity = new IngestionStore(join(stateDir, "activity-ingestion.json"), activityRemote);
    const snapshots = new SnapshotStore(join(stateDir, "snapshots"), snapshotsRemote);
    const aa = new AaLastGoodStore(join(stateDir, "aa-last-good.json"), docRemote);

    expect((await actions.all()).map((a) => a.id)).toEqual(["da-seed"]);
    expect((await activity.checkpoints()).map((c) => c.source)).toEqual(["kaneo"]);
    expect(await snapshots.readAll()).toHaveLength(1);
    expect(await aa.read()).toEqual({ batch: "seeded" });

    // Nothing local was created by any of those reads.
    expect(readdirSync(stateDir)).toEqual([]);
  });
});

// --- remote-stores SQL mapping (recording client oracle) --------------

function recordingClient(responder: (sql: string) => unknown[] = () => []): {
  seen: string[];
  client: SqlClient;
} {
  const seen: string[] = [];
  return {
    seen,
    client: {
      ref: "test",
      async query<T = Record<string, unknown>>(sql: string): Promise<T[]> {
        seen.push(sql);
        return responder(sql) as T[];
      },
    },
  };
}

describe("remote store row mapping", () => {
  test("action row -> DashAction -> row values preserves the #3142 contract", () => {
    const row = {
      id: "da-1",
      client_token: "ct-1-abcdefgh",
      kind: "triage",
      verb: "acknowledge",
      object_ref: "orch:item-1",
      target_ref: null,
      session_id: null,
      session_source: null,
      payload: { note: "ok" },
      author: drew,
      created_at: "2026-11-01T00:00:00.000Z",
      state: "new",
      applied_at: null,
      applied_by_run: null,
      result_note: null,
      supersedes_action_id: null,
      duplicate_of_action_id: null,
    };
    const action = actionFromRow(row);
    expect(action.clientToken).toBe("ct-1-abcdefgh");
    expect(action.appliedAt).toBeUndefined();
    expect(action.supersedes).toBeUndefined();
    const values = actionRowValues(action);
    expect(values).toContain("'da-1'");
    expect(values).toContain("'ct-1-abcdefgh'");
    expect(values).toContain("'2026-11-01T00:00:00.000Z'::timestamptz");
  });

  test("legacy null client_token stays readable (#6103)", () => {
    const action = actionFromRow({
      id: "da-legacy",
      client_token: null,
      kind: "triage",
      verb: "acknowledge",
      object_ref: "orch:x",
      payload: {},
      author: drew,
      created_at: "2026-11-01T00:00:00.000Z",
      state: "applied",
    });
    expect("clientToken" in action).toBe(false);
  });

  test("snapshot row round-trips including the value pair", () => {
    const record = snapshot({ value: { low: 1, high: 2 }, unit: "usd-est", limit: 5 });
    const row = {
      snapshot_key: snapshotKey(record),
      ts: record.ts,
      service_id: record.serviceId,
      metric_id: record.metricId,
      unit: record.unit,
      source: record.source,
      value: record.value,
      limit_value: record.limit,
      resets_at: null,
      observed_by: record.observedBy,
    };
    expect(snapshotFromRow(row)).toEqual(record);
    expect(snapshotRowValues(record)).toContain(snapshotKey(record));
  });

  test("actions remote orders inserts parent-first and upserts by id", async () => {
    const rec = recordingClient(() => []);
    const remote = createTursoActionsRemote(async () => rec.client);
    const child: DashAction = {
      id: "da-child",
      clientToken: "ct-2-abcdefgh",
      kind: "triage",
      verb: "acknowledge",
      objectRef: "orch:item-1",
      payload: {},
      author: drew,
      createdAt: "2026-11-01T01:00:00.000Z",
      state: "new",
      supersedes: "da-parent",
    };
    const parent: DashAction = {
      ...child,
      id: "da-parent",
      clientToken: "ct-3-abcdefgh",
      createdAt: "2026-11-01T00:00:00.000Z",
      supersedes: undefined,
    };
    await remote.saveAll([child, parent]);
    expect(rec.seen).toHaveLength(1);
    expect(rec.seen[0]).toContain("on conflict (id) do update");
    // The row tuples, not the bare ids (the child's supersedes column
    // also contains the parent id).
    expect(rec.seen[0].indexOf("('da-parent',")).toBeLessThan(rec.seen[0].indexOf("('da-child',"));
  });

  test("actions remote ships only changed rows after the first save (#14865)", async () => {
    const rec = recordingClient(() => []);
    const remote = createTursoActionsRemote(async () => rec.client);
    const first: DashAction = {
      id: "da-1",
      kind: "triage",
      verb: "acknowledge",
      objectRef: "orch:item-1",
      payload: {},
      author: drew,
      createdAt: "2026-11-01T00:00:00.000Z",
      state: "new",
    };
    const second: DashAction = { ...first, id: "da-2", createdAt: "2026-11-01T01:00:00.000Z" };
    await remote.saveAll([first, second]);
    expect(rec.seen).toHaveLength(1);
    expect(rec.seen[0].match(/\('da-/g)).toHaveLength(2);
    // An identical re-save (the store persists its full set on every
    // mutation) ships nothing at all.
    await remote.saveAll([first, second]);
    expect(rec.seen).toHaveLength(1);
    // One state transition ships exactly that one row.
    const applied: DashAction = { ...second, state: "applied", appliedAt: "2026-11-01T02:00:00.000Z" };
    await remote.saveAll([first, applied]);
    expect(rec.seen).toHaveLength(2);
    expect(rec.seen[1].match(/\('da-/g)).toHaveLength(1);
    expect(rec.seen[1]).toContain("('da-2',");
    expect(rec.seen[1]).not.toContain("('da-1',");
  });

  test("actions remote does not re-ship rows it just loaded (#14865)", async () => {
    const row = {
      id: "da-1",
      client_token: "ct-1-abcdefgh",
      kind: "triage",
      verb: "acknowledge",
      object_ref: "orch:item-1",
      target_ref: null,
      session_id: null,
      session_source: null,
      payload: { note: "ok" },
      author: drew,
      created_at: "2026-11-01T00:00:00.000Z",
      state: "new",
      applied_at: null,
      applied_by_run: null,
      result_note: null,
      supersedes_action_id: null,
      duplicate_of_action_id: null,
    };
    const rec = recordingClient((sql) => (sql.startsWith("select") ? [row] : []));
    const remote = createTursoActionsRemote(async () => rec.client);
    const loaded = await remote.loadAll();
    await remote.saveAll(loaded);
    expect(rec.seen.filter((s) => s.includes("insert into"))).toHaveLength(0);
  });

  test("snapshots remote inserts with on-conflict-do-nothing and counts returned rows", async () => {
    const rec = recordingClient(() => [{ snapshot_key: "k" }]);
    const remote = createTursoSnapshotsRemote(async () => rec.client);
    expect(await remote.append([snapshot()])).toBe(1);
    expect(rec.seen[0]).toContain("on conflict (snapshot_key) do nothing returning snapshot_key");
  });

  test("document remote upserts the aa-last-good document with the writer recorded", async () => {
    const rec = recordingClient((sql) => (sql.startsWith("select") ? [{ version: 1, document: { batch: "b" } }] : []));
    const remote = createTursoDocumentRemote(async () => rec.client, "pm-aggregator-migrate");
    expect(await remote.load(AA_LAST_GOOD_DOCUMENT)).toEqual({ version: 1, document: { batch: "b" } });
    await remote.save(AA_LAST_GOOD_DOCUMENT, 1, { batch: "b" });
    expect(rec.seen[1]).toContain("on conflict (store) do update");
    expect(rec.seen[1]).toContain("'pm-aggregator-migrate'");
  });

  test("turso mode without a client fails closed instead of using a local file", async () => {
    const remote = createTursoActionsRemote(async () => null);
    await expect(remote.loadAll()).rejects.toThrow(/refusing to fall back/);
  });
});

// ---------------------------------------------------------------------------
// #1744 Move 1, step 2: dashboard comments -> public.shared_comments.
// The oracle is the same one-backend contract as #6815: the fake remote
// is the canonical copy, so a store in turso mode must reconstruct from
// it alone and never create the local file; the row-mapping oracle is
// the DashComment record shape itself (every field round-trips).
// ---------------------------------------------------------------------------

class FakeCommentsRemote implements CommentsRemote {
  rows: DashComment[] = [];
  saves = 0;
  async loadAll(): Promise<DashComment[]> {
    return structuredClone(this.rows);
  }
  async saveAll(comments: readonly DashComment[]): Promise<void> {
    this.saves++;
    this.rows = structuredClone([...comments]);
  }
}

function comment(over: Partial<DashComment> = {}): DashComment {
  return {
    id: "dc-test-1",
    objectRef: "orch:item-1",
    kind: "note",
    author: drew,
    authorVerified: false,
    provenance: "asserted-legacy",
    body: "hello",
    createdAt: "2026-11-01T00:00:00.000Z",
    ...over,
  };
}

describe("CommentsStore remote backend (#1744)", () => {
  test("turso mode reconstructs from the remote and never creates the file", async () => {
    const dir = tmp("comments-remote");
    const file = join(dir, "comments.json");
    const remote = new FakeCommentsRemote();
    remote.rows = [comment({ id: "dc-seed" })];
    const store = new CommentsStore(file, undefined, remote);
    expect((await store.all()).map((c) => c.id)).toEqual(["dc-seed"]);
    expect(existsSync(file)).toBe(false);
    expect(readdirSync(dir)).toEqual([]);
  });

  test("add, triage and github-result transitions persist through the remote only", async () => {
    const dir = tmp("comments-remote-write");
    const file = join(dir, "comments.json");
    const remote = new FakeCommentsRemote();
    const store = new CommentsStore(file, undefined, remote);
    const msg = await store.add({ objectRef: "orch:item-2", kind: "message", author: drew, body: "ping" });
    expect(remote.rows.map((c) => c.id)).toContain(msg.id);
    expect(await store.markTriaged([msg.id])).toBe(1);
    const gh = await store.add({
      objectRef: "orch:item-3",
      kind: "github",
      author: drew,
      body: "post me",
      githubRef: "developing-today/code#1" as DashComment["githubRef"],
    });
    await store.markGithubResult(gh.id, {
      url: "https://github.com/developing-today/code/issues/1#issuecomment-1",
    });
    const fresh = new CommentsStore(file, undefined, remote);
    const all = await fresh.all();
    expect(all.find((c) => c.id === msg.id)?.triageState).toBe("triaged");
    expect(all.find((c) => c.id === gh.id)?.githubState).toBe("posted");
    expect(all.find((c) => c.id === gh.id)?.githubUrl).toContain("issuecomment-1");
    expect(remote.saves).toBeGreaterThan(0);
    expect(existsSync(file)).toBe(false);
  });

  test("comment rows round-trip every DashComment field", () => {
    const full = comment({
      kind: "github",
      authorVerified: true,
      provenance: "verified",
      linkedIdentity: { source: "cron", id: "job-1" },
      githubRef: "developing-today/code#7" as DashComment["githubRef"],
      githubState: "failed",
      githubError: "boom",
      githubFailedAt: "2026-11-01T01:00:00.000Z",
    });
    const values = commentRowValues(full);
    expect(values).toContain("'dc-test-1'");
    expect(values).toContain("'2026-11-01T00:00:00.000Z'::timestamptz");
    const row = {
      id: full.id,
      object_ref: full.objectRef,
      kind: full.kind,
      author: full.author,
      author_verified: true,
      provenance: "verified",
      linked_identity: full.linkedIdentity,
      body: full.body,
      created_at: full.createdAt,
      triage_state: null,
      github_ref: full.githubRef,
      github_state: "failed",
      github_url: null,
      github_error: "boom",
      github_failed_at: full.githubFailedAt,
    };
    expect(commentFromRow(row)).toEqual(full);
  });

  test("comments remote upserts by id into public.shared_comments", async () => {
    const rec = recordingClient(() => []);
    const remote = createTursoCommentsRemote(async () => rec.client);
    await remote.saveAll([comment()]);
    expect(rec.seen).toHaveLength(1);
    expect(rec.seen[0]).toContain("insert into public.shared_comments");
    expect(rec.seen[0]).toContain("on conflict (id) do update");
    expect(rec.seen[0]).toContain("triage_state = excluded.triage_state");
  });

  test("comments remote ships only changed rows after the first save (#14865)", async () => {
    const rec = recordingClient(() => []);
    const remote = createTursoCommentsRemote(async () => rec.client);
    const c1 = comment({ id: "dc-a" });
    const c2 = comment({ id: "dc-b", createdAt: "2026-11-01T01:00:00.000Z" });
    await remote.saveAll([c1, c2]);
    expect(rec.seen).toHaveLength(1);
    expect(rec.seen[0].match(/\('dc-/g)).toHaveLength(2);
    // An identical re-save ships nothing at all.
    await remote.saveAll([c1, c2]);
    expect(rec.seen).toHaveLength(1);
    // One triage transition ships exactly that one row.
    const triaged: DashComment = { ...c1, kind: "message", triageState: "triaged" };
    await remote.saveAll([triaged, c2]);
    expect(rec.seen).toHaveLength(2);
    expect(rec.seen[1].match(/\('dc-/g)).toHaveLength(1);
    expect(rec.seen[1]).toContain("('dc-a',");
    expect(rec.seen[1]).not.toContain("('dc-b',");
  });

  test("comments remote does not re-ship rows it just loaded (#14865)", async () => {
    const row = {
      id: "dc-seed",
      object_ref: "orch:item-1",
      kind: "note",
      author: drew,
      author_verified: false,
      provenance: "asserted-legacy",
      linked_identity: null,
      body: "hello",
      created_at: "2026-11-01T00:00:00.000Z",
      triage_state: null,
      github_ref: null,
      github_state: null,
      github_url: null,
      github_error: null,
      github_failed_at: null,
    };
    const rec = recordingClient((sql) => (sql.startsWith("select") ? [row] : []));
    const remote = createTursoCommentsRemote(async () => rec.client);
    const loaded = await remote.loadAll();
    await remote.saveAll(loaded);
    expect(rec.seen.filter((s) => s.includes("insert into"))).toHaveLength(0);
  });

  test("comments remote without a client fails closed instead of using a local file", async () => {
    const remote = createTursoCommentsRemote(async () => null);
    await expect(remote.loadAll()).rejects.toThrow(/refusing to fall back/);
  });
});
