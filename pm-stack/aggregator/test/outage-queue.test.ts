import { afterEach, describe, expect, test } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { NewAction } from "../src/actions.ts";
import { EventsStore } from "../src/events.ts";
import type { NewAuditEvent } from "../src/events.ts";
import type { DashAuthor } from "../src/model.ts";
import {
  OutageQueueStore,
  confirmOutageWrite,
  isStackLocalAddress,
  outageQueueEligible,
  queueOutageWrite,
  rejectOutageWrite,
} from "../src/outage-queue.ts";

const dirs: string[] = [];
async function setup() {
  const dir = await mkdtemp(join(tmpdir(), "outage-queue-"));
  dirs.push(dir);
  const queueFile = join(dir, "outage-queue.json");
  return {
    dir,
    queueFile,
    events: new EventsStore(join(dir, "events.json")),
    store: new OutageQueueStore(queueFile),
  };
}

afterEach(async () => {
  await Promise.all(dirs.splice(0).map((dir) => rm(dir, { recursive: true, force: true })));
});

const T0 = new Date("2026-11-23T06:00:00.000Z");
const DREW: DashAuthor = { kind: "drew", id: "drew", label: "Drew" };
const AGENT: DashAuthor = { kind: "agent", id: "access|member-17", label: "member@example.com" };

function newAction(overrides: Partial<NewAction> = {}): NewAction {
  return {
    clientToken: "tok-9608-unit-1",
    kind: "triage",
    verb: "snooze",
    objectRef: "gh:developing-today/code#9608",
    payload: { until: "2026-11-24T00:00:00.000Z" },
    author: DREW,
    ...overrides,
  };
}

// Oracle: design #1253 section 6 — during an entry/IdP outage, eligible
// writes queue stamped provenance: outage-queued with the claimed
// identity kept a claim, held out of verified-authorship surfaces
// until the claimant confirms by a fresh login (re-stamped verified)
// or rejects; asserted authorship as a fallback does not exist and
// anything else fails closed.
describe("aggregator outage queueing (#9608)", () => {
  test("a queued write is stamped outage-queued, keeps the claim, and is audited before it persists", async () => {
    const { store, queueFile } = await setup();
    let writesAtAuditTime = -1;
    const events = {
      record: async (_event: NewAuditEvent) => {
        writesAtAuditTime = (await new OutageQueueStore(queueFile).list()).length;
      },
    };
    const { record, created } = await queueOutageWrite({ store, events, action: newAction(), now: T0 });
    expect(created).toBe(true);
    expect(writesAtAuditTime).toBe(0);
    expect(record.provenance).toBe("outage-queued");
    expect(record.state).toBe("queued");
    expect(record.claimedAuthor).toEqual(DREW);
    expect(record.action.author).toEqual(DREW);
    expect(record.queuedAt).toBe(T0.toISOString());
    // A fresh store instance reads the same record back from disk.
    expect(await new OutageQueueStore(queueFile).get(record.id)).toMatchObject({ id: record.id, state: "queued" });
  });

  test("the queued audit event names the claim as unverified", async () => {
    const { store, events } = await setup();
    const { record } = await queueOutageWrite({ store, events, action: newAction(), now: T0 });
    const listed = await events.list({ kind: "outage-write-queued" });
    expect(listed).toHaveLength(1);
    expect(listed[0]).toMatchObject({
      actor: "claimed:drew:drew",
      ref: "gh:developing-today/code#9608",
      attribution: { confidence: "unknown", asserted: true },
    });
    expect(listed[0]!.summary).toContain(record.id);
    expect(listed[0]!.summary).toContain("outage-queued");
  });

  test("if the audit write fails, nothing is queued (fail closed)", async () => {
    const { store } = await setup();
    const events = {
      record: async (_event: NewAuditEvent) => {
        throw new Error("events store unavailable");
      },
    };
    await expect(queueOutageWrite({ store, events, action: newAction(), now: T0 })).rejects.toThrow(
      "events store unavailable",
    );
    expect(await store.list()).toHaveLength(0);
  });

  test("queueing is idempotent by clientToken: a repost returns the existing record, unaudited twice", async () => {
    const { store, events } = await setup();
    const first = await queueOutageWrite({ store, events, action: newAction(), now: T0 });
    const second = await queueOutageWrite({ store, events, action: newAction(), now: T0 });
    expect(second.created).toBe(false);
    expect(second.record.id).toBe(first.record.id);
    expect(await store.list()).toHaveLength(1);
    expect(await events.list({ kind: "outage-write-queued" })).toHaveLength(1);
  });

  test("eligibility: confirmable claims and consumer-applied writes queue; the rest fail closed", () => {
    expect(outageQueueEligible(newAction())).toBe(true); // drew, triage
    expect(outageQueueEligible(newAction({ author: AGENT }))).toBe(true); // agent claim
    expect(
      outageQueueEligible(newAction({ kind: "status-request", verb: "request-status", payload: { status: "x" } })),
    ).toBe(true);
    expect(
      outageQueueEligible(
        newAction({ kind: "preference", verb: "set-delegation-default", payload: { standingAuto: true } }),
      ),
    ).toBe(true);
    // A muse claim has no Access derivation, so it can never be confirmed.
    expect(outageQueueEligible(newAction({ author: { kind: "muse", id: "muse", label: "Muse" } }))).toBe(false);
    // The aggregator-applied view verbs cannot honour their contract from a queue.
    expect(
      outageQueueEligible(newAction({ kind: "preference", verb: "delete-view", payload: { viewId: "v1" } })),
    ).toBe(false);
    expect(
      outageQueueEligible(
        newAction({ kind: "preference", verb: "save-view", payload: { viewName: "V", viewFilters: {} } }),
      ),
    ).toBe(false);
  });

  test("stack-local addresses: loopback, tailnet, and private ranges only", () => {
    for (const local of [
      "127.0.0.1",
      "127.0.0.53",
      "::1",
      "::ffff:127.0.0.1",
      "100.99.113.103", // amd-6 tailnet
      "100.64.0.1",
      "100.127.255.255",
      "10.0.0.3",
      "172.16.0.9",
      "172.31.255.255",
      "192.168.1.5",
      "fd7a:115c:a1e0::1",
      "fe80::1",
    ]) {
      expect(isStackLocalAddress(local)).toBe(true);
    }
    for (const remote of [
      "8.8.8.8",
      "100.128.0.1", // just outside the CGNAT /10
      "100.63.255.255",
      "172.32.0.1",
      "192.167.0.1",
      "203.0.113.9",
      "",
      "not-an-address",
      undefined,
      null,
    ]) {
      expect(isStackLocalAddress(remote)).toBe(false);
    }
  });

  test("confirmation releases the write under the VERIFIED author and marks the record confirmed", async () => {
    const { store, events } = await setup();
    const { record } = await queueOutageWrite({ store, events, action: newAction(), now: T0 });
    let releasedAction: NewAction | null = null;
    const result = await confirmOutageWrite({
      store,
      events,
      id: record.id,
      derivedAuthor: { kind: "drew", id: "drew", label: "Drew (verified)" },
      release: async (action) => {
        releasedAction = action;
        return { ok: true, actionId: "da-released-1" };
      },
      now: T0,
    });
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.actionId).toBe("da-released-1");
      expect(result.record.state).toBe("confirmed");
      expect(result.record.resolvedActionId).toBe("da-released-1");
    }
    // The re-stamp: the released action carries the derived author,
    // not the claim (the claim's label differs and must not survive).
    expect(releasedAction).not.toBeNull();
    expect(releasedAction!.author).toEqual({ kind: "drew", id: "drew", label: "Drew (verified)" });
    expect(releasedAction!.clientToken).toBe("tok-9608-unit-1");
    const confirmations = await events.list({ kind: "outage-write-confirmed" });
    expect(confirmations).toHaveLength(1);
    expect(confirmations[0]).toMatchObject({ actor: "drew:drew", attribution: { confidence: "authenticated" } });
    // A second confirmation is refused: the record already resolved.
    const again = await confirmOutageWrite({
      store,
      events,
      id: record.id,
      derivedAuthor: DREW,
      release: async () => ({ ok: true, actionId: "da-other" }),
      now: T0,
    });
    expect(again).toMatchObject({ ok: false, status: 409 });
  });

  test("confirmation by a different identity is refused and the write stays queued", async () => {
    const { store, events } = await setup();
    const { record } = await queueOutageWrite({ store, events, action: newAction({ author: AGENT }), now: T0 });
    let releaseCalled = false;
    const result = await confirmOutageWrite({
      store,
      events,
      id: record.id,
      derivedAuthor: DREW, // the owner is not this claim's claimant
      release: async () => {
        releaseCalled = true;
        return { ok: true, actionId: "da-nope" };
      },
      now: T0,
    });
    expect(result).toMatchObject({ ok: false, status: 403 });
    expect(releaseCalled).toBe(false);
    expect((await store.get(record.id))?.state).toBe("queued");
    expect(await events.list({ kind: "outage-write-confirmed" })).toHaveLength(0);
  });

  test("a failed release leaves the write queued; an unknown id is a 404", async () => {
    const { store, events } = await setup();
    const { record } = await queueOutageWrite({ store, events, action: newAction(), now: T0 });
    const failed = await confirmOutageWrite({
      store,
      events,
      id: record.id,
      derivedAuthor: DREW,
      release: async () => ({ ok: false, status: 400, error: "item check failed" }),
      now: T0,
    });
    expect(failed).toMatchObject({ ok: false, status: 400, error: "item check failed" });
    expect((await store.get(record.id))?.state).toBe("queued");
    const missing = await confirmOutageWrite({
      store,
      events,
      id: "oq_missing",
      derivedAuthor: DREW,
      release: async () => ({ ok: true, actionId: "da-x" }),
      now: T0,
    });
    expect(missing).toMatchObject({ ok: false, status: 404 });
  });

  test("if the confirmation audit write fails, the write stays queued (a retry is safe)", async () => {
    const { store } = await setup();
    const { record } = await queueOutageWrite({
      store,
      events: { record: async () => {} },
      action: newAction(),
      now: T0,
    });
    const failingEvents = {
      record: async (_event: NewAuditEvent) => {
        throw new Error("audit sink down");
      },
    };
    const result = await confirmOutageWrite({
      store,
      events: failingEvents,
      id: record.id,
      derivedAuthor: DREW,
      release: async () => ({ ok: true, actionId: "da-released-1" }),
      now: T0,
    });
    expect(result).toMatchObject({ ok: false, status: 500 });
    expect((await store.get(record.id))?.state).toBe("queued");
  });

  test("rejection: the claimant rejects; the owner may reject another's claim; a stranger may not", async () => {
    const { store, events } = await setup();
    const own = await queueOutageWrite({
      store,
      events,
      action: newAction({ clientToken: "tok-9608-unit-own" }),
      now: T0,
    });
    const ownResult = await rejectOutageWrite({ store, events, id: own.record.id, derivedAuthor: DREW, now: T0 });
    expect(ownResult).toMatchObject({ ok: true, byOwner: false });
    expect((await store.get(own.record.id))?.state).toBe("rejected");

    const agents = await queueOutageWrite({
      store,
      events,
      action: newAction({ clientToken: "tok-9608-unit-agent", author: AGENT }),
      now: T0,
    });
    const stranger = await rejectOutageWrite({
      store,
      events,
      id: agents.record.id,
      derivedAuthor: { kind: "agent", id: "access|someone-else", label: "Other" },
      now: T0,
    });
    expect(stranger).toMatchObject({ ok: false, status: 403 });
    expect((await store.get(agents.record.id))?.state).toBe("queued");
    const byOwner = await rejectOutageWrite({ store, events, id: agents.record.id, derivedAuthor: DREW, now: T0 });
    expect(byOwner).toMatchObject({ ok: true, byOwner: true });
    expect((await store.get(agents.record.id))?.state).toBe("rejected");
    const rejections = await events.list({ kind: "outage-write-rejected" });
    expect(rejections).toHaveLength(2);
    expect(rejections[1]!.summary).toContain("by the verified owner");
  });
});
