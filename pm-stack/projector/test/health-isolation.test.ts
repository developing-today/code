// Regression guard for #9850: /health stalled for >=90s windows while
// the projector process stayed alive. The mechanism: one libSQL client
// shared between the request handlers and the reconcile pass, whose
// per-row write round trips serialized on that client — a health query
// queued behind thousands of reconcile upserts. The fix gives /health
// (and the reconcile trigger) dedicated clients in production wiring,
// batches the reconcile writes, and bounds the health probe itself.
//
// This test pins the isolation property: while a write flood is still
// running on the main client, /health answered through the dedicated
// health client returns promptly — it does not wait for the flood.
import { describe, test, expect, beforeAll, afterAll } from "bun:test";
import { createClient, type Client } from "@libsql/client";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { migrate, upsertComment } from "../src/store";
import { handleRequest, HEALTH_SNAPSHOT_STALE_MS, type ServerEnv } from "../src/server";

let dir: string;
let db: Client;
let healthDb: Client;
const env: ServerEnv = {};

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type CommentInput = Parameters<typeof upsertComment>[1];

function commentRow(i: number): CommentInput {
  return {
    id: String(10_000 + i),
    ref: "developing-today/code#9850",
    kind: "issue_comment",
    author: "loadgen",
    body: `flood row ${i}`,
    created_at: "2026-11-14T00:00:00Z",
    updated_at: "2026-11-14T00:00:00Z",
    url: null,
    path: null,
    line: null,
    diff_hunk: null,
    review_state: null,
    submitted_at: null,
    commit_id: null,
    deleted: 0,
    source_updated_at: "2026-11-14T00:00:00Z",
  };
}

/** A client whose every call takes `delayMs` — the shape of the remote
 *  libSQL round trips a reconcile flood consists of. Sequential calls
 *  on it serialize, exactly like the production client under load. */
function slowClient(real: Client, delayMs: number): Client {
  return {
    execute: async (...a: Parameters<Client["execute"]>) => {
      await sleep(delayMs);
      return real.execute(...a);
    },
    batch: async (...a: Parameters<Client["batch"]>) => {
      await sleep(delayMs);
      return real.batch(...a);
    },
  } as unknown as Client;
}

beforeAll(async () => {
  dir = mkdtempSync(join(tmpdir(), "projector-health-isolation-"));
  const url = `file:${join(dir, "test.db")}`;
  db = createClient({ url });
  healthDb = createClient({ url });
  await migrate(db);
});

afterAll(() => {
  db.close();
  healthDb.close();
  rmSync(dir, { recursive: true, force: true });
});

describe("/health isolation (#9850)", () => {
  test("health answers through its own client while a write flood is still running on the main client", async () => {
    const floodClient = slowClient(db, 150);
    let floodDone = false;
    const flood = (async () => {
      // 24 sequential writes x 150ms ≈ 3.6s of serialized flood —
      // a small stand-in for a reconcile pass's thousands of rows.
      for (let i = 0; i < 24; i++) {
        await upsertComment(floodClient, commentRow(i));
      }
      floodDone = true;
    })();

    const started = Date.now();
    const res = await handleRequest(new Request("http://127.0.0.1:7801/health"), floodClient, env, {
      healthDb,
    });
    const elapsed = Date.now() - started;

    expect(res.status).toBe(200);
    const body = (await res.json()) as { ok: boolean; items: number; comments: number; links: number };
    expect(body.ok).toBe(true);
    expect(typeof body.items).toBe("number");
    expect(typeof body.comments).toBe("number");
    expect(typeof body.links).toBe("number");
    // The point of the guard: the answer arrived while the flood was
    // still running, in a small fraction of the flood's duration.
    expect(floodDone).toBe(false);
    expect(elapsed).toBeLessThan(1500);

    await flood;
    expect(floodDone).toBe(true);
    const after = await healthDb.execute("SELECT COUNT(*) AS n FROM pmp_comments");
    expect(Number((after.rows[0] as unknown as { n: number }).n)).toBe(24);
  });

  test("health keeps its response shape on the plain shared-client path", async () => {
    const res = await handleRequest(new Request("http://127.0.0.1:7801/health"), db, env);
    expect(res.status).toBe(200);
    const body = (await res.json()) as { ok: boolean; comments: number };
    expect(body.ok).toBe(true);
    expect(body.comments).toBe(24);
  });
});

describe("/health snapshot + reconcile gate (#9850 round 2)", () => {
  test("health serves the background snapshot and does no DB work on the request path", async () => {
    const wedgedDb = {
      execute: async () => {
        throw new Error("db wedged");
      },
      batch: async () => {
        throw new Error("db wedged");
      },
    } as unknown as Client;
    const res = await handleRequest(new Request("http://127.0.0.1:7801/health"), wedgedDb, env, {
      healthDb: wedgedDb,
      healthSnapshot: () => ({ items: 11, comments: 22, links: 33, at: Date.now() }),
    });
    expect(res.status).toBe(200);
    const body = (await res.json()) as {
      ok: boolean;
      items: number;
      comments: number;
      links: number;
      snapshotAgeMs: number;
    };
    expect(body.ok).toBe(true);
    expect(body.items).toBe(11);
    expect(body.comments).toBe(22);
    expect(body.links).toBe(33);
    expect(body.snapshotAgeMs).toBeGreaterThanOrEqual(0);
  });

  test("a stale snapshot answers 503, not a healthy answer", async () => {
    const res = await handleRequest(new Request("http://127.0.0.1:7801/health"), db, env, {
      healthSnapshot: () => ({ items: 1, comments: 2, links: 3, at: Date.now() - HEALTH_SNAPSHOT_STALE_MS - 1_000 }),
    });
    expect(res.status).toBe(503);
    const body = (await res.json()) as { ok: boolean; error: string };
    expect(body.ok).toBe(false);
    expect(body.error).toBe("health_snapshot_stale");
  });

  test("a /reconcile trigger during an in-flight pass is a no-op success", async () => {
    const res = await handleRequest(
      new Request("http://127.0.0.1:7801/reconcile", {
        method: "POST",
        headers: { authorization: "Bearer test-token" },
      }),
      db,
      { PM_PROJECTOR_API_TOKEN: "test-token", PM_REPOS: "" },
      { beginReconcile: () => false, endReconcile: () => {} },
    );
    expect(res.status).toBe(200);
    const body = (await res.json()) as { ok: boolean; skipped?: string };
    expect(body.ok).toBe(true);
    expect(body.skipped).toBe("reconcile_in_progress");
  });
});
