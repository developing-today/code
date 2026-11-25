import { describe, expect, test } from "bun:test";
import {
  createLedgerQuery,
  ledgerPage,
  rollupNotificationSends,
  type LedgerPagesIo,
  type NotificationSendRow,
} from "../edge/src/ledger-pages.ts";

const NOW = Date.parse("2026-11-25T12:00:00Z");
const now = () => NOW;

function ioWith(rows: Record<string, Record<string, unknown>[]>): LedgerPagesIo {
  return {
    now,
    query: async (sql: string) => {
      for (const [needle, canned] of Object.entries(rows)) {
        if (sql.includes(needle)) return canned;
      }
      throw new Error(`unexpected sql: ${sql}`);
    },
  };
}

const INCIDENT = {
  id: "inc-1",
  source: "stack-sentinel",
  incident_key: "svc:pm-aggregator",
  severity: "critical",
  title: "Aggregator <down> & out",
  body: "The aggregator is failing its health check.",
  status: "open",
  first_seen_at: "2026-11-25T10:00:00Z",
  last_seen_at: "2026-11-25T11:00:00Z",
  delivered_at: "2026-11-25T10:01:00Z",
  escalated_at: null,
  acked_at: null,
  closed_at: null,
  occurrences: 3,
};

const EVENT = {
  id: 7,
  incident_id: "inc-1",
  at: "2026-11-25T10:00:30Z",
  kind: "opened",
  detail: { via: "sentinel" },
  source: "stack-sentinel",
  incident_key: "svc:pm-aggregator",
  title: "Aggregator <down> & out",
};

describe("alerts surface", () => {
  test("json twin carries incidents, events and freshness", async () => {
    const io = ioWith({ alerts_incident_events: [EVENT], alerts_incidents: [INCIDENT] });
    const res = await ledgerPage(new Request("https://pm.security.cab/api/alerts.json"), io);
    expect(res?.status).toBe(200);
    const body = (await res!.json()) as any;
    expect(body.incidents).toHaveLength(1);
    expect(body.incidents[0].title).toBe("Aggregator <down> & out");
    expect(body.events[0].detail).toEqual({ via: "sentinel" });
    expect(body.freshness.readAt).toBe("2026-11-25T12:00:00.000Z");
    expect(body.freshness.newestIncidentAt).toBe("2026-11-25T10:00:00Z");
    expect(body.freshness.newestEventAt).toBe("2026-11-25T10:00:30Z");
  });

  test("html page escapes titles and shows freshness", async () => {
    const io = ioWith({ alerts_incident_events: [EVENT], alerts_incidents: [INCIDENT] });
    const res = await ledgerPage(new Request("https://pm.security.cab/alerts"), io);
    expect(res?.status).toBe(200);
    const html = await res!.text();
    expect(html).toContain("Aggregator &lt;down&gt; &amp; out");
    expect(html).not.toContain("<down>");
    expect(html).toContain("Newest ledger row: 2026-11-25T10:00:30Z");
    expect(html).toContain("3 occurrences");
  });

  test("a failed read is a 503, never an empty page dressed as current", async () => {
    const io: LedgerPagesIo = {
      now,
      query: async () => {
        throw new Error("ledger down");
      },
    };
    const html = await ledgerPage(new Request("https://pm.security.cab/alerts"), io);
    expect(html?.status).toBe(503);
    expect(await html!.text()).toContain("No empty or stale result is being shown");
    const api = await ledgerPage(new Request("https://pm.security.cab/api/alerts.json"), io);
    expect(api?.status).toBe(503);
    expect(((await api!.json()) as any).error).toBe("alerts store unavailable");
  });
});

const SENDS: NotificationSendRow[] = [
  { class: "incident", object_ref: "inc-1", incident_id: "inc-1", outcome: "sent", created_at: "2026-11-24T12:00:00Z", acted_on_at: "2026-11-24T13:00:00Z" },
  { class: "incident", object_ref: "inc-2", incident_id: "inc-2", outcome: "sent", created_at: "2026-11-24T12:00:00Z", acted_on_at: null },
  { class: "quota-wall", object_ref: "pool-1", incident_id: null, outcome: "sent", created_at: "2026-11-20T12:00:00Z", acted_on_at: "2026-11-22T12:00:00Z" },
  { class: "incident", object_ref: "inc-3", incident_id: "inc-3", outcome: "gone", created_at: "2026-11-24T12:00:00Z", acted_on_at: null },
  { class: "incident", object_ref: "inc-4", incident_id: "inc-4", outcome: "sent", acted_on_at: null },
  { class: "incident", object_ref: "inc-5", incident_id: "inc-5", outcome: "sent", created_at: "2026-10-01T00:00:00Z", acted_on_at: null },
  // The live Turso table's defaulted send-time column is `at`: a row
  // carrying only `at` is dated (2026-11-25 IW-339 probe).
  { class: "incident", object_ref: "inc-6", incident_id: "inc-6", outcome: "sent", at: "2026-11-24T12:00:00Z", acted_on_at: "2026-11-24T12:30:00Z" },
];

describe("notifications roll-up (ported derivation)", () => {
  test("counting rules: sent only, 24h ack window, rolling window, undated", () => {
    const rollup = rollupNotificationSends(SENDS, new Date(NOW));
    expect(rollup.asOf).toBe("2026-11-25T12:00:00.000Z");
    expect(rollup.windowStart).toBe("2026-10-28T12:00:00.000Z");
    expect(rollup.undated).toBe(1);
    expect(rollup.classes.map((c) => c.class)).toEqual(["incident", "quota-wall"]);
    const incident = rollup.classes[0];
    expect(incident.sent).toBe(3);
    expect(incident.acted).toBe(2);
    expect(incident.ratio).toBe(2 / 3);
    expect(incident.proposal).toBe(false);
    expect(incident.weeks).toHaveLength(4);
    expect(incident.weeks[3]).toMatchObject({ sent: 3, acted: 2 });
    const quota = rollup.classes[1];
    // The ack landed 48h after the send: outside the 24h window.
    expect(quota.sent).toBe(1);
    expect(quota.acted).toBe(0);
    expect(quota.ratio).toBe(0);
    expect(quota.proposal).toBe(true);
  });

  test("json twin and html page render the roll-up with freshness", async () => {
    const io = ioWith({ alerts_notification_sends: SENDS as unknown as Record<string, unknown>[] });
    const api = await ledgerPage(new Request("https://pm.security.cab/api/notifications.json"), io);
    expect(api?.status).toBe(200);
    const body = (await api!.json()) as any;
    expect(body.classes).toHaveLength(2);
    expect(body.freshness.newestSendAt).toBe("2026-11-24T12:00:00Z");
    const html = await ledgerPage(new Request("https://pm.security.cab/notifications"), io);
    const text = await html!.text();
    expect(text).toContain("quota-wall");
    expect(text).toContain("default-off proposal (display only)");
  });
});

const COMMENT_ROW = {
  id: "c-1",
  object_ref: "issue:14968",
  kind: "note",
  author: { kind: "drew" },
  author_verified: 1,
  provenance: null,
  linked_identity: { provider: "github", login: "dezren39" },
  body: "Looks good — <ship> it",
  created_at: "2026-11-25T09:30:00Z",
  triage_state: "triaged",
  github_ref: null,
  github_state: null,
  github_url: null,
  github_error: null,
  github_failed_at: null,
};

describe("comments surface", () => {
  test("json twin maps the stored row to the comment shape", async () => {
    const io = ioWith({ shared_comments: [COMMENT_ROW] });
    const res = await ledgerPage(new Request("https://pm.security.cab/api/comments.json"), io);
    expect(res?.status).toBe(200);
    const body = (await res!.json()) as any;
    expect(body.comments).toHaveLength(1);
    const comment = body.comments[0];
    expect(comment.id).toBe("c-1");
    expect(comment.objectRef).toBe("issue:14968");
    expect(comment.authorVerified).toBe(true);
    expect(comment.linkedIdentity).toEqual({ provider: "github", login: "dezren39" });
    expect(comment.triageState).toBe("triaged");
    expect(comment.githubRef).toBeUndefined();
    expect(body.freshness.newestCommentAt).toBe("2026-11-25T09:30:00Z");
  });

  test("html page renders author kind and escapes the body", async () => {
    const io = ioWith({ shared_comments: [COMMENT_ROW] });
    const res = await ledgerPage(new Request("https://pm.security.cab/comments"), io);
    const html = await res!.text();
    expect(html).toContain("drew");
    expect(html).toContain("(verified)");
    expect(html).toContain("Looks good — &lt;ship&gt; it");
  });
});

describe("routing", () => {
  test("other paths and non-GET methods are not this module's", async () => {
    const io = ioWith({});
    expect(await ledgerPage(new Request("https://pm.security.cab/decisions"), io)).toBeNull();
    expect(
      await ledgerPage(new Request("https://pm.security.cab/alerts", { method: "POST" }), io),
    ).toBeNull();
  });
});

describe("ledger query client", () => {
  test("posts a pipeline execute and decodes typed cells + JSON columns", async () => {
    let seen: { url: string; init: RequestInit } | null = null;
    const fetchImpl = (async (url: string, init: RequestInit) => {
      seen = { url, init };
      return new Response(
        JSON.stringify({
          results: [
            {
              type: "ok",
              response: {
                type: "execute",
                result: {
                  cols: [{ name: "n" }, { name: "detail" }],
                  rows: [
                    [
                      { type: "integer", value: "42" },
                      { type: "text", value: "{\"via\":\"sentinel\"}" },
                    ],
                  ],
                },
              },
            },
            { type: "ok", response: { type: "close" } },
          ],
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      );
    }) as unknown as typeof fetch;
    const query = createLedgerQuery(fetchImpl, "libsql://fleet-platform.example", "token-by-name");
    const rows = await query("select 42");
    expect(rows).toEqual([{ n: 42, detail: { via: "sentinel" } }]);
    expect(seen!.url).toBe("https://fleet-platform.example/v2/pipeline");
    expect((seen!.init.headers as Record<string, string>).Authorization).toBe("Bearer token-by-name");
    const body = JSON.parse(seen!.init.body as string);
    expect(body.requests[0].stmt.sql).toBe("select 42");
    expect(body.requests[0].stmt.want_rows).toBe(true);
  });

  test("a missing credential is an error, never an unauthenticated read", async () => {
    const query = createLedgerQuery((async () => new Response("{}")) as unknown as typeof fetch, "https://x.example");
    await expect(query("select 1")).rejects.toThrow("missing server credential");
  });
});
