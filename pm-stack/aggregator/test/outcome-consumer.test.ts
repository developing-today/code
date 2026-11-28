import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { GithubHotEvent } from "../src/activity.ts";
import { FileOutcomeBindingSource, GithubOutcomeConsumer, type OutcomeBindingSource } from "../src/outcome-consumer.ts";
import { OutcomeFactStore, rollupOutcomeFacts, type OutcomeBinding } from "../src/outcome-attribution.ts";

/** Fixture hot-list rows in the projector's normalized shape (#5204),
 *  mirroring test/activity-github-7084.test.ts. */
function hotEvent(seq: number, id: string, partial: Partial<GithubHotEvent> = {}): GithubHotEvent {
  return {
    seq,
    event_id: id,
    delivery_id: `delivery-${id}`,
    repo: "developing-today/code",
    event: "issues",
    action: "opened",
    ref: `developing-today/code#${seq}`,
    actor: "dezren39",
    source_url: `https://github.com/developing-today/code/issues/${seq}`,
    source_timestamp: `2026-11-22T12:00:${String(seq % 60).padStart(2, "0")}.000Z`,
    received_at: "2026-11-22T12:01:00.000Z",
    ...partial,
  };
}

interface FakeLedger {
  rows: GithubHotEvent[];
  cursor: number;
  ackCalls: { cursor: number; highWater: number }[];
}

/** Minimal projector hot-list protocol, per-consumer checkpoint held in
 *  the ledger; mirrors the fake in test/activity-github-7084.test.ts. */
function projector(rows: GithubHotEvent[], opts: { failPages?: boolean } = {}) {
  const ledger: FakeLedger = { rows: [...rows], cursor: 0, ackCalls: [] };
  const fetchMock = (async (input: string | URL | Request, init?: RequestInit) => {
    const url = new URL(String(input));
    const highWater = Math.max(0, ...ledger.rows.map((row) => row.seq));
    if (url.pathname === "/hot/checkpoints" && (!init?.method || init.method === "GET")) {
      return Response.json({ consumer: url.searchParams.get("consumer"), cursor: ledger.cursor });
    }
    if (url.pathname === "/hot" && (!init?.method || init.method === "GET")) {
      if (opts.failPages) return Response.json({ error: "unavailable" }, { status: 503 });
      const after = Number(url.searchParams.get("after") ?? 0);
      const limit = Number(url.searchParams.get("limit") ?? 200);
      const pinned = Number(url.searchParams.get("high_water") ?? highWater);
      const events = ledger.rows
        .filter((row) => row.seq > after && row.seq <= Math.min(pinned, highWater))
        .sort((a, b) => a.seq - b.seq)
        .slice(0, limit);
      return Response.json({
        events,
        high_water: Math.min(pinned, highWater),
        next_cursor: events.at(-1)?.seq ?? after,
      });
    }
    if (url.pathname === "/hot/checkpoints" && init?.method === "POST") {
      const body = JSON.parse(String(init.body)) as { consumer: string; cursor: number; highWater: number };
      if (body.cursor > body.highWater || body.highWater > highWater) {
        return Response.json({ error: "checkpoint_past_high_water" }, { status: 400 });
      }
      ledger.ackCalls.push({ cursor: body.cursor, highWater: body.highWater });
      ledger.cursor = Math.max(ledger.cursor, body.cursor);
      return Response.json({ consumer: body.consumer, cursor: ledger.cursor });
    }
    return Response.json({ error: "not_found" }, { status: 404 });
  }) as typeof fetch;
  return { ledger, fetchMock };
}

const NO_BINDINGS: OutcomeBindingSource = { resolve: async () => [] };

describe("GithubOutcomeConsumer (#10910 live cadence wiring)", () => {
  let dir: string;
  let factsFile: string;
  let bindingsFile: string;

  beforeEach(async () => {
    dir = await mkdtemp(join(tmpdir(), "outcome-consumer-"));
    factsFile = join(dir, "outcome-facts.jsonl");
    bindingsFile = join(dir, "outcome-bindings.jsonl");
  });

  afterEach(async () => {
    await rm(dir, { recursive: true, force: true });
  });

  function consumer(
    fetchMock: typeof fetch,
    opts: { bindings?: OutcomeBindingSource; pageSize?: number; maxPages?: number } = {},
  ): GithubOutcomeConsumer {
    return new GithubOutcomeConsumer({
      baseUrl: "http://projector.test",
      token: "test-token",
      store: new OutcomeFactStore(factsFile),
      bindings: opts.bindings ?? NO_BINDINGS,
      fetch: fetchMock,
      pageSize: opts.pageSize ?? 2,
      maxPages: opts.maxPages ?? 20,
    });
  }

  test("a sweep projects facts into the ledger, advances the cursor, and acks the checkpoint", async () => {
    const rows = [
      hotEvent(1, "d1:issue:1001"),
      hotEvent(2, "d2:pr:2001", {
        event: "pull_request",
        ref: "developing-today/code#202",
        source_url: "https://github.com/developing-today/code/pull/202",
      }),
      hotEvent(3, "d3:commit:abc123", { event: "push", action: null, ref: "refs/heads/main", source_url: null }),
    ];
    const { ledger, fetchMock } = projector(rows);
    const result = await consumer(fetchMock).sync();

    expect(result.state).toBe("current");
    expect(result.appended).toBe(3);
    expect(result.cursor).toBe(3);
    expect(result.checkpointCursor).toBe(3);
    expect(result.highWater).toBe(3);
    expect(ledger.ackCalls).toEqual([{ cursor: 3, highWater: 3 }]);

    const store = new OutcomeFactStore(factsFile);
    expect(await store.cursor()).toBe(3);
    const facts = await store.readFacts();
    expect(facts.map((fact) => fact.outcomeKind)).toEqual(["issue-created", "pr-created", "commit-pushed"]);
    // No binding source records: every fact is explicitly unattributed.
    expect(facts.every((fact) => fact.method === "unattributed" && fact.reason === "no-binding")).toBe(true);
    const rollup = rollupOutcomeFacts(facts);
    expect(rollup.totalByKind?.["issue-created"]).toBe(1);
    expect(rollup.unattributedByReason.get("no-binding")).toBe(3);
  });

  test("bindings from the file source attach job/session/model identities; unbound events stay unattributed", async () => {
    const binding: OutcomeBinding = {
      eventId: "d1:issue:1001",
      taskId: "20799",
      jobId: "schedule-9",
      sessionId: "ses_root",
      rootSessionId: "ses_root",
      providerID: "anthropic",
      modelID: "claude-sonnet",
      method: "queue-binding",
    };
    await writeFile(bindingsFile, `${JSON.stringify(binding)}\n`, "utf8");
    const rows = [hotEvent(1, "d1:issue:1001"), hotEvent(2, "d2:issue:1002")];
    const { fetchMock } = projector(rows);
    const result = await consumer(fetchMock, { bindings: new FileOutcomeBindingSource(bindingsFile) }).sync();
    expect(result.state).toBe("current");

    const facts = await new OutcomeFactStore(factsFile).readFacts();
    const bound = facts.find((fact) => fact.eventId === "d1:issue:1001");
    expect(bound?.taskId).toBe("20799");
    expect(bound?.jobId).toBe("schedule-9");
    expect(bound?.sessionId).toBe("ses_root");
    expect(bound?.providerID).toBe("anthropic");
    expect(bound?.modelID).toBe("claude-sonnet");
    expect(bound?.method).toBe("queue-binding");
    expect(bound?.reason).toBeNull();
    const unbound = facts.find((fact) => fact.eventId === "d2:issue:1002");
    expect(unbound?.method).toBe("unattributed");
    expect(unbound?.reason).toBe("no-binding");

    const rollup = rollupOutcomeFacts(facts);
    expect(rollup.byJob.get("20799")?.["issue-created"]).toBe(1);
    expect(rollup.bySession.get("ses_root")?.["issue-created"]).toBe(1);
    expect(rollup.byModel.get("anthropic\u0000claude-sonnet")?.["issue-created"]).toBe(1);
  });

  test("a missing bindings file is an empty source, not a failure", async () => {
    const { fetchMock } = projector([hotEvent(1, "d1:issue:1001")]);
    const result = await consumer(fetchMock, {
      bindings: new FileOutcomeBindingSource(join(dir, "absent.jsonl")),
    }).sync();
    expect(result.state).toBe("current");
    expect(result.appended).toBe(1);
    const facts = await new OutcomeFactStore(factsFile).readFacts();
    expect(facts[0]?.reason).toBe("no-binding");
  });

  test("replay appends nothing: a second sync and a restarted consumer are no-ops", async () => {
    const rows = [hotEvent(1, "d1:issue:1001"), hotEvent(2, "d2:issue:1002")];
    const { ledger, fetchMock } = projector(rows);
    const first = await consumer(fetchMock).sync();
    expect(first.appended).toBe(2);

    const second = await consumer(fetchMock).sync();
    expect(second.state).toBe("current");
    expect(second.appended).toBe(0);

    const restarted = await consumer(fetchMock).sync();
    expect(restarted.appended).toBe(0);
    const facts = await new OutcomeFactStore(factsFile).readFacts();
    expect(facts).toHaveLength(2);
    expect(ledger.cursor).toBe(2);
  });

  test("a failed sweep records a source-gap fact, moves no cursor, acks nothing, and recovers next tick", async () => {
    const rows = [hotEvent(1, "d1:issue:1001")];
    const broken = projector(rows, { failPages: true });
    const failed = await consumer(broken.fetchMock).sync();
    expect(failed.state).toBe("outage");
    expect(failed.error).toContain("projector_http_503");
    expect(failed.appended).toBe(0);
    expect(failed.cursor).toBe(0);
    expect(broken.ledger.ackCalls).toEqual([]);

    const store = new OutcomeFactStore(factsFile);
    expect(await store.cursor()).toBe(0);
    const gapFacts = await store.readFacts();
    expect(gapFacts).toHaveLength(1);
    expect(gapFacts[0]?.method).toBe("source-gap");
    // The gap is a record of failure, never activity: rollups exclude it.
    expect(rollupOutcomeFacts(gapFacts).totalByKind?.["issue-created"]).toBe(0);

    const healthy = projector(rows);
    const recovered = await consumer(healthy.fetchMock).sync();
    expect(recovered.state).toBe("current");
    expect(recovered.appended).toBe(1);
    const facts = await store.readFacts();
    expect(facts.filter((fact) => fact.method !== "source-gap")).toHaveLength(1);
    expect(await store.cursor()).toBe(1);
  });

  test("page budget: a partial sweep keeps durable facts, acks nothing, and resumes from the ledger cursor", async () => {
    const rows = [hotEvent(1, "d1:issue:1001"), hotEvent(2, "d2:issue:1002"), hotEvent(3, "d3:issue:1003")];
    const { ledger, fetchMock } = projector(rows);
    const partial = await consumer(fetchMock, { pageSize: 1, maxPages: 2 }).sync();
    expect(partial.state).toBe("lagging");
    expect(partial.error).toBe("sweep_exceeded_2_pages");
    expect(partial.appended).toBe(2);
    expect(partial.cursor).toBe(2);
    expect(ledger.ackCalls).toEqual([]);

    const resumed = await consumer(fetchMock, { pageSize: 2, maxPages: 20 }).sync();
    expect(resumed.state).toBe("current");
    expect(resumed.appended).toBe(1);
    expect(resumed.cursor).toBe(3);
    expect(ledger.ackCalls).toEqual([{ cursor: 3, highWater: 3 }]);
    expect(await new OutcomeFactStore(factsFile).readFacts()).toHaveLength(3);
  });

  test("a corrupt bindings file fails the sweep visibly instead of rendering as no-bindings", async () => {
    await writeFile(bindingsFile, "{not json}\n", "utf8");
    const { ledger, fetchMock } = projector([hotEvent(1, "d1:issue:1001")]);
    const result = await consumer(fetchMock, { bindings: new FileOutcomeBindingSource(bindingsFile) }).sync();
    expect(result.state).toBe("outage");
    expect(result.error).toBe("outcome_binding_source_corrupt");
    expect(result.appended).toBe(0);
    expect(ledger.ackCalls).toEqual([]);
    const facts = await new OutcomeFactStore(factsFile).readFacts();
    expect(facts.filter((fact) => fact.method !== "source-gap")).toHaveLength(0);
  });

  test("the ledger file is append-only JSONL in sweep order", async () => {
    const { fetchMock } = projector([hotEvent(1, "d1:issue:1001"), hotEvent(2, "d2:issue:1002")]);
    await consumer(fetchMock).sync();
    const lines = (await readFile(factsFile, "utf8")).trim().split("\n");
    expect(lines).toHaveLength(2);
    const parsed = lines.map((line) => JSON.parse(line) as { seq: number; factId: string });
    expect(parsed.map((fact) => fact.seq)).toEqual([1, 2]);
    expect(parsed[0]?.factId).toBe("outcome:v1:d1:issue:1001");
  });
});
