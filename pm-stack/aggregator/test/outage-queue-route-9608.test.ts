import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { spawn, type Subprocess } from "bun";
import { existsSync, mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

// #9608 route wiring for outage queueing (design #1253 section 6).
// Boots the real server with Access CONFIGURED: a write presented
// without a verifiable Access JWT is the outage case. From the
// loopback (stack-local) test client, an eligible write must queue
// as an unverified outage-queued claim — visible only on the queue
// surface, never in the actions store — while ineligible writes keep
// the enforcer's 401, and confirm/reject refuse without a fresh
// verified login, leaving the record queued.
let proc: Subprocess<"ignore", "pipe", "inherit"> | null = null;
let port = 0;
let actionsFile = "";
let outageQueueFile = "";
let queueId = "";

const DREW = { kind: "drew", id: "drew", label: "Drew" };

function actionBody(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    clientToken: "tok-9608-route-1",
    kind: "triage",
    verb: "snooze",
    objectRef: "gh:developing-today/code#9608",
    payload: { until: "2026-11-24T00:00:00.000Z" },
    author: DREW,
    ...overrides,
  };
}

async function post(path: string, body: unknown): Promise<Response> {
  return fetch(`http://127.0.0.1:${port}${path}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

beforeAll(async () => {
  const dir = mkdtempSync(join(tmpdir(), "outage-queue-route-"));
  actionsFile = join(dir, "actions.json");
  outageQueueFile = join(dir, "outage-queue.json");
  port = 5900 + Math.floor(Math.random() * 800);
  proc = spawn({
    cmd: ["bun", "run", "src/server.ts"],
    cwd: import.meta.dir + "/..",
    env: {
      ...Bun.env,
      PM_AGG_PORT: String(port),
      PM_AGG_FRESHNESS_PORT: String(port + 1000),
      PM_AGG_SOURCE: "fixtures",
      PM_AGG_STORE_BACKEND: "file",
      PM_AGG_ACCESS_TEAM_DOMAIN: "team.cloudflareaccess.com",
      PM_AGG_ACCESS_AUD: "aggregator-aud",
      PM_AGG_OWNER_IDS: "drew@example.com",
      PM_AGG_ACTIONS_FILE: actionsFile,
      PM_AGG_OUTAGE_QUEUE_FILE: outageQueueFile,
      PM_AGG_EVENTS_FILE: join(dir, "events.json"),
      PM_AGG_IDENTITY_REGISTRY_FILE: join(dir, "identity-registry.json"),
      PM_AGG_BREAK_GLASS_FILE: join(dir, "break-glass-tokens.json"),
      PM_AGG_REPORT_RECORDS_FILE: join(dir, "report-records.jsonl"),
      // The server opens its metadata DB at boot; in a clean venue the
      // default state directory does not exist, so point it at the
      // temp dir (the pre-existing boot suites rely on the laptop's
      // state dir instead and fail without it — not this suite's bug).
      PM_METADATA_DB_PATH: join(dir, "pm-metadata.sqlite"),
    },
    stdout: "pipe",
    stderr: "inherit",
  });
  const deadline = Date.now() + 8000;
  let ready = false;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/api/health`);
      if (res.ok) {
        ready = true;
        break;
      }
    } catch {
      /* not up yet */
    }
    await Bun.sleep(120);
  }
  expect(ready).toBe(true);
}, 60_000);

afterAll(() => {
  proc?.kill();
});

describe("outage queueing routes (#9608)", () => {
  test("an eligible write with no verifiable identity queues as an unverified claim (202), held out of the actions store", async () => {
    const res = await post("/api/actions", actionBody());
    expect(res.status).toBe(202);
    const body = (await res.json()) as {
      queued: boolean;
      provenance: string;
      queue: { id: string; state: string; provenance: string; claimedAuthor: unknown };
    };
    expect(body.queued).toBe(true);
    expect(body.provenance).toBe("outage-queued");
    expect(body.queue.state).toBe("queued");
    expect(body.queue.provenance).toBe("outage-queued");
    expect(body.queue.claimedAuthor).toEqual(DREW);
    queueId = body.queue.id;
    expect(queueId.startsWith("oq_")).toBe(true);

    // Held out of verified-authorship surfaces: the actions store —
    // the consumers' poll — never sees it.
    const actionsRes = await fetch(`http://127.0.0.1:${port}/api/actions.json?state=all`);
    const actionsBody = (await actionsRes.json()) as { actions: Array<{ clientToken: string }> };
    expect(actionsBody.actions.some((a) => a.clientToken === "tok-9608-route-1")).toBe(false);
    const actionsOnDisk = existsSync(actionsFile) ? readFileSync(actionsFile, "utf8") : "";
    expect(actionsOnDisk).not.toContain("tok-9608-route-1");
    const queueOnDisk = readFileSync(outageQueueFile, "utf8");
    expect(queueOnDisk).toContain("tok-9608-route-1");
    expect(queueOnDisk).toContain("outage-queued");

    // The queue's own read surface shows the claim, and only there.
    const queueRes = await fetch(`http://127.0.0.1:${port}/api/actions/outage-queue.json?state=queued`);
    const queueBody = (await queueRes.json()) as { queue: Array<{ id: string; state: string }> };
    expect(queueBody.queue.map((w) => w.id)).toContain(queueId);
    const bogus = await fetch(`http://127.0.0.1:${port}/api/actions/outage-queue.json?state=bogus`);
    expect(bogus.status).toBe(400);
  }, 30_000);

  test("ineligible writes fail closed with the enforcer's 401, exactly as before", async () => {
    // A muse claim can never be confirmed by a fresh Access login.
    const muse = await post(
      "/api/actions",
      actionBody({ clientToken: "tok-9608-route-2", author: { kind: "muse", id: "muse", label: "Muse" } }),
    );
    expect(muse.status).toBe(401);
    // The aggregator-applied view verbs cannot queue.
    const view = await post(
      "/api/actions",
      actionBody({
        clientToken: "tok-9608-route-3",
        kind: "preference",
        verb: "delete-view",
        payload: { viewId: "v1" },
      }),
    );
    expect(view.status).toBe(401);
    const queueRes = await fetch(`http://127.0.0.1:${port}/api/actions/outage-queue.json`);
    const queueBody = (await queueRes.json()) as { queue: Array<{ action: { clientToken: string } }> };
    expect(queueBody.queue.some((w) => w.action.clientToken === "tok-9608-route-2")).toBe(false);
    expect(queueBody.queue.some((w) => w.action.clientToken === "tok-9608-route-3")).toBe(false);
  }, 30_000);

  test("a batch item queues per-item while the batch still answers 200", async () => {
    const res = await post("/api/actions/batch", {
      actions: [actionBody({ clientToken: "tok-9608-route-4" })],
    });
    expect(res.status).toBe(200);
    const body = (await res.json()) as {
      results: Array<{ ok: boolean; queued?: boolean; queueId?: string }>;
      failed: number;
    };
    expect(body.results).toHaveLength(1);
    expect(body.results[0]).toMatchObject({ ok: true, queued: true });
    expect(body.results[0]!.queueId!.startsWith("oq_")).toBe(true);
    expect(body.failed).toBe(0);
    // And it still never entered the actions store.
    const actionsOnDisk = existsSync(actionsFile) ? readFileSync(actionsFile, "utf8") : "";
    expect(actionsOnDisk).not.toContain("tok-9608-route-4");
  }, 30_000);

  test("confirm and reject refuse without a fresh verified login; the write stays queued", async () => {
    const confirm = await post("/api/actions/outage-queue/confirm", { id: queueId });
    expect(confirm.status).toBe(401);
    const reject = await post("/api/actions/outage-queue/reject", { id: queueId });
    expect(reject.status).toBe(401);
    const missing = await post("/api/actions/outage-queue/confirm", { id: "oq_missing" });
    expect(missing.status).toBe(404);
    const queueRes = await fetch(`http://127.0.0.1:${port}/api/actions/outage-queue.json?state=queued`);
    const queueBody = (await queueRes.json()) as { queue: Array<{ id: string; state: string }> };
    expect(queueBody.queue.find((w) => w.id === queueId)?.state).toBe("queued");
  }, 30_000);
});
