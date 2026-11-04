// Hot list (#5204): append-only projector event ledger + consumer
// checkpoints. Covers redelivery idempotency, distinct deliveries,
// multi-commit pushes, ordering, bounded pagination inside a pinned
// high-water window, and monotonic checkpoint ack rules.
import { describe, test, expect, beforeAll, afterAll } from "bun:test";
import { createClient, type Client } from "@libsql/client";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  migrate,
  appendHotEvents,
  listHotEvents,
  hotHighWater,
  getConsumerCheckpoint,
  ackConsumerCheckpoint,
  upsertItem as storeUpsertItem,
  upsertComment as storeUpsertComment,
  markCommentDeleted as storeMarkCommentDeleted,
  recordPush as storeRecordPush,
  type HotEventInput,
} from "../src/store";
import { applyEvent, type ProjectorStore } from "../src/projector";
import { handleRequest, type ServerEnv } from "../src/server";

let db: Client;
let dir: string;

const ev = (over: Partial<HotEventInput>): HotEventInput => ({
  eventId: "d1:issue:1",
  deliveryId: "d1",
  repo: "developing-today/code",
  event: "issues",
  action: "opened",
  ref: "developing-today/code#1",
  actor: "dezren39",
  sourceUrl: "https://github.com/developing-today/code/issues/1",
  sourceTimestamp: "2026-10-25T00:00:00Z",
  ...over,
});

function libsqlStore(dbx: Client): ProjectorStore {
  return {
    upsertItem: (i) => storeUpsertItem(dbx, i),
    upsertComment: (c) => storeUpsertComment(dbx, c),
    markCommentDeleted: (c) => storeMarkCommentDeleted(dbx, c),
    recordPush: (p) => storeRecordPush(dbx, p),
    appendHotEvents: (events) => appendHotEvents(dbx, events),
  };
}

beforeAll(async () => {
  dir = mkdtempSync(join(tmpdir(), "pm-projector-hot-"));
  db = createClient({ url: `file:${join(dir, "test.db")}` });
  await migrate(db);
});

afterAll(() => {
  db.close();
  rmSync(dir, { recursive: true, force: true });
});

describe("store: hot events", () => {
  test("redelivery of the same delivery appends nothing twice; distinct deliveries stay distinct", async () => {
    expect(await appendHotEvents(db, [ev({})])).toBe(1);
    expect(await appendHotEvents(db, [ev({})])).toBe(0); // same event_id
    expect(await appendHotEvents(db, [ev({ eventId: "d2:issue:1", deliveryId: "d2" })])).toBe(1);
    const rows = await listHotEvents(db, {});
    expect(rows.length).toBe(2);
    expect(rows[0].seq).toBeLessThan(rows[1].seq); // ascending, monotonic
    expect(rows.map((r) => r.delivery_id)).toEqual(["d1", "d2"]);
  });

  test("pagination is bounded, exclusive-cursor, and capped by the pinned high-water", async () => {
    await appendHotEvents(db, [
      ev({ eventId: "d3:issue:2", deliveryId: "d3", ref: "developing-today/code#2" }),
      ev({ eventId: "d4:issue:3", deliveryId: "d4", ref: "developing-today/code#3" }),
      ev({ eventId: "d5:issue:4", deliveryId: "d5", ref: "developing-today/code#4" }),
    ]);
    // seqs are monotonic but not dense: the ignored duplicate insert above
    // consumed one AUTOINCREMENT value. Work with the actual seqs.
    const all = await listHotEvents(db, { after: 0, limit: 100 });
    expect(all.length).toBe(5);
    const seqs = all.map((r) => r.seq);
    const hw = await hotHighWater(db);
    expect(hw).toBe(seqs[4]);
    const page1 = await listHotEvents(db, { after: 0, limit: 2, highWater: hw });
    expect(page1.map((r) => r.seq)).toEqual(seqs.slice(0, 2));
    const page2 = await listHotEvents(db, { after: page1[1].seq, limit: 2, highWater: hw });
    expect(page2.map((r) => r.seq)).toEqual(seqs.slice(2, 4));
    // A window pinned before the last event existed never sees it.
    const pinned = await listHotEvents(db, { after: 0, limit: 100, highWater: seqs[3] });
    expect(pinned.map((r) => r.seq)).toEqual(seqs.slice(0, 4));
  });
});

describe("store: consumer checkpoints", () => {
  test("ack is monotonic and cannot pass the issued high-water", async () => {
    expect(await getConsumerCheckpoint(db, "sweep-a")).toBe(0);
    expect(await ackConsumerCheckpoint(db, "sweep-a", 3, 5)).toBe(3);
    expect(await ackConsumerCheckpoint(db, "sweep-a", 2, 5)).toBe(3); // stale ack: no rewind
    expect(await ackConsumerCheckpoint(db, "sweep-a", 5, 5)).toBe(5);
    await expect(ackConsumerCheckpoint(db, "sweep-a", 6, 5)).rejects.toThrow("checkpoint_past_high_water");
    expect(await getConsumerCheckpoint(db, "sweep-a")).toBe(5); // failed ack changed nothing
  });

  test("ack rejects a caller-minted future high-water against server state", async () => {
    // Codex finding on PR #5327: comparing only the two caller-supplied
    // numbers lets a consumer ack cursor == highWater far past the actual
    // ledger and silently skip events that arrive later under those seqs.
    const hw = await hotHighWater(db);
    await expect(ackConsumerCheckpoint(db, "sweep-forged", hw + 1000, hw + 1000)).rejects.toThrow(
      "high_water_past_server",
    );
    expect(await getConsumerCheckpoint(db, "sweep-forged")).toBe(0); // forged ack persisted nothing
  });
});

describe("projector: hot mapping", () => {
  test("a push appends one hot event per commit", async () => {
    const before = await hotHighWater(db);
    await applyEvent(
      libsqlStore(db),
      "push",
      {
        ref: "refs/heads/main",
        before: "aaa",
        after: "ccc",
        pusher: { name: "dezren39" },
        sender: { login: "dezren39" },
        repository: { full_name: "developing-today/code" },
        commits: [
          {
            id: "aaa111",
            url: "https://github.com/developing-today/code/commit/aaa111",
            timestamp: "2026-10-25T01:00:00Z",
            author: { username: "dezren39" },
          },
          {
            id: "bbb222",
            url: "https://github.com/developing-today/code/commit/bbb222",
            timestamp: "2026-10-25T01:01:00Z",
            author: { username: "dezren39" },
          },
        ],
      },
      [],
      "push-delivery-1",
    );
    const rows = await listHotEvents(db, { after: before });
    expect(rows.length).toBe(2);
    expect(rows.map((r) => r.event_id)).toEqual(["push-delivery-1:commit:aaa111", "push-delivery-1:commit:bbb222"]);
    expect(rows[0].ref).toBe("refs/heads/main");
    // Redelivering the same push delivery appends nothing.
    await applyEvent(
      libsqlStore(db),
      "push",
      {
        ref: "refs/heads/main",
        before: "aaa",
        after: "ccc",
        pusher: { name: "dezren39" },
        repository: { full_name: "developing-today/code" },
        commits: [{ id: "aaa111", url: "x", timestamp: "2026-10-25T01:00:00Z", author: { username: "dezren39" } }],
      },
      [],
      "push-delivery-1",
    );
    expect((await listHotEvents(db, { after: before })).length).toBe(2);
  });

  test("issue and comment events append normalized rows without raw bodies", async () => {
    const before = await hotHighWater(db);
    const repo = { full_name: "developing-today/code" };
    await applyEvent(
      libsqlStore(db),
      "issues",
      {
        action: "opened",
        repository: repo,
        sender: { login: "dezren39" },
        issue: {
          id: 4242,
          number: 77,
          title: "hot",
          state: "open",
          html_url: "https://github.com/developing-today/code/issues/77",
          url: "https://api.github.com/repos/developing-today/code/issues/77",
          user: { login: "dezren39" },
          labels: [],
          assignees: [],
          created_at: "2026-10-25T02:00:00Z",
          updated_at: "2026-10-25T02:00:00Z",
        },
      },
      [],
      "issue-delivery-1",
    );
    await applyEvent(
      libsqlStore(db),
      "issue_comment",
      {
        action: "deleted",
        repository: repo,
        sender: { login: "dezren39" },
        issue: {
          id: 4242,
          number: 77,
          title: "hot",
          state: "open",
          html_url: "https://github.com/developing-today/code/issues/77",
          url: "https://api.github.com/repos/developing-today/code/issues/77",
          user: { login: "dezren39" },
          labels: [],
          assignees: [],
          created_at: "2026-10-25T02:00:00Z",
          updated_at: "2026-10-25T02:05:00Z",
        },
        comment: {
          id: 9001,
          body: "secret body text that must not be copied into the ledger",
          html_url: "https://github.com/developing-today/code/issues/77#issuecomment-9001",
          user: { login: "dezren39" },
          created_at: "2026-10-25T02:05:00Z",
          updated_at: "2026-10-25T02:05:00Z",
        },
      },
      [],
      "comment-delivery-1",
    );
    const rows = await listHotEvents(db, { after: before });
    expect(rows.length).toBe(2);
    expect(rows[0]).toMatchObject({
      event: "issues",
      action: "opened",
      ref: "developing-today/code#77",
      actor: "dezren39",
    });
    expect(rows[1]).toMatchObject({ event: "issue_comment", action: "deleted", ref: "developing-today/code#77" });
    expect(JSON.stringify(rows)).not.toContain("secret body text");
  });
});

describe("server: hot routes", () => {
  const env: ServerEnv = { PM_PROJECTOR_API_TOKEN: "tok" };

  test("GET /hot issues a high-water and pages inside it; checkpoint ack enforces it", async () => {
    const res = await handleRequest(new Request("http://x/hot?after=0&limit=2"), db, env);
    expect(res.status).toBe(200);
    const body = (await res.json()) as any;
    expect(body.events.length).toBe(2);
    expect(body.next_cursor).toBe(body.events[1].seq);
    expect(body.high_water).toBe(await hotHighWater(db));

    const noAuth = await handleRequest(
      new Request("http://x/hot/checkpoints", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ consumer: "sweep-b", cursor: 1, highWater: body.high_water }),
      }),
      db,
      env,
    );
    expect(noAuth.status).toBe(401);

    const past = await handleRequest(
      new Request("http://x/hot/checkpoints", {
        method: "POST",
        headers: { "content-type": "application/json", authorization: "Bearer tok" },
        body: JSON.stringify({ consumer: "sweep-b", cursor: body.high_water + 1, highWater: body.high_water }),
      }),
      db,
      env,
    );
    expect(past.status).toBe(400);

    const ok = await handleRequest(
      new Request("http://x/hot/checkpoints", {
        method: "POST",
        headers: { "content-type": "application/json", authorization: "Bearer tok" },
        body: JSON.stringify({ consumer: "sweep-b", cursor: body.next_cursor, highWater: body.high_water }),
      }),
      db,
      env,
    );
    expect(ok.status).toBe(200);
    expect(((await ok.json()) as any).cursor).toBe(body.next_cursor);

    const read = await handleRequest(new Request("http://x/hot/checkpoints?consumer=sweep-b"), db, env);
    expect(((await read.json()) as any).cursor).toBe(body.next_cursor);
  });

  test("GET /hot clamps a caller-pinned high_water to the server ledger; forged ack pair is rejected", async () => {
    const hw = await hotHighWater(db);
    const res = await handleRequest(new Request(`http://x/hot?after=0&limit=1&high_water=${hw + 1000}`), db, env);
    expect(res.status).toBe(200);
    expect(((await res.json()) as any).high_water).toBe(hw); // issued bound is server-controlled

    const forged = await handleRequest(
      new Request("http://x/hot/checkpoints", {
        method: "POST",
        headers: { "content-type": "application/json", authorization: "Bearer tok" },
        body: JSON.stringify({ consumer: "sweep-forged", cursor: hw + 1000, highWater: hw + 1000 }),
      }),
      db,
      env,
    );
    expect(forged.status).toBe(400);
    expect(((await forged.json()) as any).error).toBe("high_water_past_server");
    const read = await handleRequest(new Request("http://x/hot/checkpoints?consumer=sweep-forged"), db, env);
    expect(((await read.json()) as any).cursor).toBe(0);
  });
});
