import { describe, expect, test } from "bun:test";
import { AlertsStore, type Incident, type IncidentEvent } from "../src/alerts.ts";
import { validateHealthHook, type HealthHookRecord } from "../src/health-hook.ts";
import type { SqlClient } from "../src/sql-client.ts";

// Oracle: the C3 design's incident-record shape
// (doc/2026-11-20T00-46-00Z_design_c3-health-hooks/note.md, "Report
// format"), in the exact shape the runner (pm-stack/health-hooks/
// run.py) builds it — the Slice D harness record. The down record
// below is that shape verbatim, envelope fields included.
const downPost = {
  source: "health-hook",
  key: "health-hook:pm-stack-reachable:11111111-1111-4111-8111-111111111111",
  severity: "critical",
  title: "Health hook: PM stack reachable is down",
  body: "PM stack unreachable: connect ECONNREFUSED",
  target: "pm-stack-reachable",
  state: "down",
  caller: "lane-launch",
  firedAt: "2026-11-22T10:00:00Z",
  observedAt: "2026-11-22T09:59:58Z",
  incidentId: "11111111-1111-4111-8111-111111111111",
  recoveryOf: null,
  evidence: {
    confirmFailure: true,
    reachability: {
      ok: false,
      status: null,
      error: "connect ECONNREFUSED",
      latencyMs: 12,
      tcpOk: false,
      tcpError: "connect ECONNREFUSED",
    },
    serviceState: {
      unit: "pm-aggregator",
      activeState: "failed",
      subState: "failed",
      result: "exit-code",
      exitTimestamp: "2026-11-22T09:58:00Z",
    },
    authHealth: { status: "resolved", detail: "secret resolved by key" },
    recentChanges: { commits: ["abc1234 fix"], gitOk: true, deployMarkerMtime: null, gitSyncOk: null },
    hostHealth: { collectedAt: "2026-11-22T09:55:00Z", unitsDown: ["pm-aggregator"], present: true },
  },
  stackHealth: {
    serviceId: "pm-stack-reachable",
    name: "PM stack reachable",
    state: "down",
    since: "2026-11-22T10:00:00Z",
    detail: "PM stack unreachable: connect ECONNREFUSED",
  },
} as const;

const recoveryPost = {
  ...downPost,
  severity: "info",
  title: "Health hook: PM stack reachable recovered",
  body: "PM stack reachable recovered after incident 11111111-1111-4111-8111-111111111111",
  state: "up",
  firedAt: "2026-11-22T10:07:00Z",
  observedAt: null,
  incidentId: "22222222-2222-4222-8222-222222222222",
  recoveryOf: "11111111-1111-4111-8111-111111111111",
  evidence: {
    confirmFailure: false,
    reachability: { ok: true, status: 200, error: null, latencyMs: 30, tcpOk: true, tcpError: null },
    serviceState: {
      unit: "pm-aggregator",
      activeState: "active",
      subState: "running",
      result: "success",
      exitTimestamp: null,
    },
    authHealth: { status: "resolved", detail: "secret resolved by key" },
    recentChanges: { commits: ["abc1234 fix"], gitOk: true, deployMarkerMtime: null, gitSyncOk: null },
    hostHealth: { collectedAt: "2026-11-22T10:05:00Z", unitsDown: [], present: true },
  },
  stackHealth: {
    serviceId: "pm-stack-reachable",
    name: "PM stack reachable",
    state: "up",
    since: "2026-11-22T10:07:00Z",
    detail: "PM stack reachable recovered after incident 11111111-1111-4111-8111-111111111111",
  },
};

describe("validateHealthHook (oracle: C3 design record shape)", () => {
  test("a Slice D-shaped down record validates and normalizes", () => {
    const v = validateHealthHook(downPost);
    expect(v.ok).toBe(true);
    if (!v.ok) return;
    expect(v.value.target).toBe("pm-stack-reachable");
    expect(v.value.state).toBe("down");
    expect(v.value.incidentId).toBe(downPost.incidentId);
    expect(v.value.recoveryOf).toBeNull();
    expect(v.value.evidence).toEqual(downPost.evidence);
    expect(v.value.stackHealth).toEqual(downPost.stackHealth);
    // Envelope duplicates live on the incident row, not the record.
    expect(v.value).not.toHaveProperty("source");
    expect(v.value).not.toHaveProperty("title");
  });

  test("a recovery record validates with its recoveryOf lineage", () => {
    const v = validateHealthHook(recoveryPost);
    expect(v.ok).toBe(true);
    if (!v.ok) return;
    expect(v.value.state).toBe("up");
    expect(v.value.recoveryOf).toBe(downPost.incidentId);
    expect(v.value.observedAt).toBeNull();
  });

  test("a record without the stackHealth member still validates", () => {
    const { stackHealth: _drop, ...rest } = downPost;
    const v = validateHealthHook(rest);
    expect(v.ok).toBe(true);
    if (!v.ok) return;
    expect(v.value.stackHealth).toBeUndefined();
  });

  test("malformed records are rejected with the field named", () => {
    const cases: Array<[unknown, string]> = [
      [null, "healthHook must be an object"],
      [[], "healthHook must be an object"],
      [{ ...downPost, target: " " }, "healthHook.target required"],
      [{ ...downPost, target: undefined }, "healthHook.target required"],
      [{ ...downPost, state: "sideways" }, 'healthHook.state must be "down" | "up" | "transient"'],
      [{ ...downPost, caller: "" }, "healthHook.caller required"],
      [{ ...downPost, firedAt: 42 }, "healthHook.firedAt required"],
      [{ ...downPost, incidentId: "" }, "healthHook.incidentId required"],
      [{ ...downPost, observedAt: undefined }, "healthHook.observedAt required"],
      [{ ...downPost, observedAt: 42 }, "healthHook.observedAt must be a string or null"],
      [{ ...downPost, recoveryOf: undefined }, "healthHook.recoveryOf required"],
      [{ ...downPost, recoveryOf: 7 }, "healthHook.recoveryOf must be a string or null"],
      [{ ...downPost, evidence: [] }, "healthHook.evidence must be an object"],
      [{ ...downPost, evidence: undefined }, "healthHook.evidence must be an object"],
      [{ ...downPost, title: 3 }, "healthHook.title must be a string"],
      [
        { ...downPost, stackHealth: { ...downPost.stackHealth, state: "sideways" } },
        'healthHook.stackHealth.state must be "down" | "up"',
      ],
    ];
    for (const [input, error] of cases) {
      const v = validateHealthHook(input);
      expect(v.ok).toBe(false);
      if (!v.ok) expect(v.error).toBe(error);
    }
  });

  test("evidence members are not deep-validated (probe shapes evolve)", () => {
    const v = validateHealthHook({
      ...downPost,
      evidence: { brandNewProbe: { nested: [1, 2, 3] }, authHealth: { status: "error", detail: "boom" } },
    });
    expect(v.ok).toBe(true);
  });
});

// --- Store round-trip (oracle: #12602 acceptance — a drill down +
// recovery pair reads back with incidentId/recoveryOf lineage and
// full evidence intact). The fake implements exactly the statements
// AlertsStore issues for post / close-by-key / per-incident events,
// parsing the sqlStr/sqlJson literals the store builds.

function parseValues(sql: string, count: number): Array<string | null> {
  const start = sql.indexOf("values (") + "values (".length;
  const out: Array<string | null> = [];
  let i = start;
  while (out.length < count) {
    while (sql[i] === " " || sql[i] === "\n" || sql[i] === ",") i++;
    if (sql.startsWith("null", i)) {
      out.push(null);
      i += 4;
      continue;
    }
    if (sql[i] !== "'") throw new Error(`fake db: expected literal at ${i} in: ${sql}`);
    let val = "";
    i++;
    while (i < sql.length) {
      if (sql[i] === "'") {
        if (sql[i + 1] === "'") {
          val += "'";
          i += 2;
          continue;
        }
        i++;
        break;
      }
      val += sql[i];
      i++;
    }
    out.push(val);
  }
  return out;
}

function allLiterals(sql: string): string[] {
  const out: string[] = [];
  let i = 0;
  while (i < sql.length) {
    if (sql[i] !== "'") {
      i++;
      continue;
    }
    let val = "";
    i++;
    while (i < sql.length) {
      if (sql[i] === "'") {
        if (sql[i + 1] === "'") {
          val += "'";
          i += 2;
          continue;
        }
        i++;
        break;
      }
      val += sql[i];
      i++;
    }
    out.push(val);
  }
  return out;
}

function fakeAlertsDb() {
  const incidents: Incident[] = [];
  const events: IncidentEvent[] = [];
  let incidentSeq = 0;
  let eventSeq = 0;
  let clock = 0;
  const now = () => `2026-11-22T10:00:${String(clock++).padStart(2, "0")}Z`;
  const db: SqlClient = {
    ref: "fake",
    async query<T>(sql: string): Promise<T[]> {
      const flat = sql.trim().replace(/\s+/g, " ");
      if (flat.startsWith("insert into alerts.incidents")) {
        const [source, key, severity, title, body] = parseValues(sql, 5);
        if (incidents.some((r) => r.source === source && r.incident_key === key && r.status !== "closed")) {
          return [];
        }
        const row: Incident = {
          id: `00000000-0000-4000-8000-${String(++incidentSeq).padStart(12, "0")}`,
          source: source!,
          incident_key: key!,
          severity: severity as Incident["severity"],
          title: title!,
          body,
          status: "open",
          first_seen_at: now(),
          last_seen_at: now(),
          delivered_at: null,
          escalated_at: null,
          acked_at: null,
          closed_at: null,
          occurrences: 1,
        };
        incidents.push(row);
        return [row] as T[];
      }
      if (flat.startsWith("insert into alerts.incident_events")) {
        const [incidentId, kind, detailJson] = parseValues(sql, 3);
        events.push({
          id: ++eventSeq,
          incident_id: incidentId!,
          at: now(),
          kind: kind as IncidentEvent["kind"],
          detail: detailJson ? (JSON.parse(detailJson) as Record<string, unknown>) : null,
        });
        return [];
      }
      if (flat.startsWith("update alerts.incidents set occurrences")) {
        const [source, key] = allLiterals(sql);
        const row = incidents.find((r) => r.source === source && r.incident_key === key && r.status !== "closed");
        if (!row) return [];
        row.occurrences += 1;
        row.last_seen_at = now();
        return [row] as T[];
      }
      if (flat.startsWith("update alerts.incidents set status = 'closed'")) {
        const id = allLiterals(sql)[1];
        const row = incidents.find((r) => r.id === id && r.status !== "closed");
        if (!row) return [];
        row.status = "closed";
        row.closed_at = now();
        return [row] as T[];
      }
      if (flat.startsWith("select * from alerts.incidents where id =")) {
        const [id] = allLiterals(sql);
        return incidents.filter((r) => r.id === id) as T[];
      }
      if (flat.startsWith("select * from alerts.incidents where source =")) {
        const [source, key] = allLiterals(sql);
        return incidents.filter((r) => r.source === source && r.incident_key === key && r.status !== "closed") as T[];
      }
      if (flat.startsWith("select * from alerts.incident_events where incident_id =")) {
        const [id] = allLiterals(sql);
        return events.filter((e) => e.incident_id === id) as T[];
      }
      throw new Error(`fake db: unexpected query: ${flat}`);
    },
  };
  return { db, incidents, events };
}

describe("healthHook ledger storage (oracle: #12602 acceptance drill)", () => {
  const key = downPost.key;
  const downRecord = (validateHealthHook(downPost) as { ok: true; value: HealthHookRecord }).value;
  const recoveryRecord = (validateHealthHook(recoveryPost) as { ok: true; value: HealthHookRecord }).value;

  test("a down + recovery pair stores both records against the incident, lineage intact", async () => {
    const { db } = fakeAlertsDb();
    const store = new AlertsStore(db);

    const down = await store.postIncident(
      { source: "health-hook", key, severity: "critical", title: downPost.title, body: downPost.body },
      { healthHook: downRecord },
    );
    expect(down.created).toBe(true);

    let events = await store.listIncidentEvents(down.incident.id);
    expect(events.map((e) => e.kind)).toEqual(["opened"]);
    expect(events[0].detail?.healthHook).toEqual(downRecord);
    // The opened detail's pre-existing members are unchanged.
    expect(events[0].detail?.source).toBe("health-hook");
    expect(events[0].detail?.key).toBe(key);
    expect(events[0].detail?.severity).toBe("critical");

    // The recovery post reuses the outage key (runner behaviour), so
    // it dedupes onto the same incident instead of opening a second.
    const recovery = await store.postIncident(
      { source: "health-hook", key, severity: "info", title: recoveryPost.title, body: recoveryPost.body },
      { healthHook: recoveryRecord },
    );
    expect(recovery.created).toBe(false);
    expect(recovery.incident.id).toBe(down.incident.id);

    const closed = await store.closeOpenIncidentByKey("health-hook", key, { healthHook: recoveryRecord });
    expect(closed.transitioned).toBe(true);
    expect(closed.incident?.status).toBe("closed");

    events = await store.listIncidentEvents(down.incident.id);
    expect(events.map((e) => e.kind)).toEqual(["opened", "closed"]);
    const storedRecovery = events[1].detail?.healthHook as HealthHookRecord;
    expect(storedRecovery.recoveryOf).toBe(downRecord.incidentId);
    expect(storedRecovery.incidentId).toBe(recoveryRecord.incidentId);
    expect(storedRecovery.evidence).toEqual(recoveryRecord.evidence);
  });

  test("posts without the extension keep the opened/closed details unchanged", async () => {
    const { db } = fakeAlertsDb();
    const store = new AlertsStore(db);
    const posted = await store.postIncident({ source: "stack-sentinel", key: "k1", severity: "warning", title: "t" });
    const closed = await store.closeIncident(posted.incident.id);
    expect(closed.transitioned).toBe(true);
    const events = await store.listIncidentEvents(posted.incident.id);
    expect(events.map((e) => e.kind)).toEqual(["opened", "closed"]);
    expect(events[0].detail).toEqual({ source: "stack-sentinel", key: "k1", severity: "warning" });
    expect(events[1].detail).toEqual({});
  });

  test("listIncidentEvents returns only the incident's own events", async () => {
    const { db } = fakeAlertsDb();
    const store = new AlertsStore(db);
    const a = await store.postIncident({ source: "s", key: "a", severity: "info", title: "a" });
    await store.postIncident({ source: "s", key: "b", severity: "info", title: "b" });
    const events = await store.listIncidentEvents(a.incident.id);
    expect(events).toHaveLength(1);
    expect(events[0].incident_id).toBe(a.incident.id);
  });
});
