// Hosted read-only M4 ledger surfaces (#14968, CF-5b): /alerts,
// /notifications and /comments (plus their /api/*.json twins), served
// by this edge from the fleet-platform ledger instead of the host.
//
// The caller must verify the owner BEFORE calling this handler (the
// same contract as decision-pages.ts, #14967). This module is present
// only when the Worker's LEDGER_PAGES flag reads "on"; without it the
// paths stay the host's, exactly as before.
//
// Data plane: the tables are the ones the laptop aggregator's
// AlertsStore and comments store read (aggregator/src/alerts.ts,
// remote-stores.ts), under their SQLite names: alerts_incidents,
// alerts_incident_events, alerts_notification_sends, shared_comments.
// Design §2 of doc/2026-11-25_design_cloudflare-first-migration.md
// originally kept these ledgers in the fleet-platform Turso; the
// owner ruling of 2026-11-26 on #14866 superseded that — the target
// is now the D1 database named fleet-platform (the same schema,
// imported whole), selected by the Worker's LEDGER_STORE var reading
// "d1". The Turso path stays beside it, selected by LEDGER_STORE
// reading anything else, so the flip back is the same var.
//
// Freshness (#14065: no tier renders invented data to fill a gap):
// every answer carries the read time and the newest row timestamp of
// its own ledger; a failed read answers 503 — never a stale copy,
// never an empty page dressed as current.
//
// The /notifications roll-up is a port of the pure derivation in
// aggregator/src/notifications.ts (rollupNotificationSends): same
// counting rules, pinned by the tests here — only outcome = "sent"
// rows are sends, a send is acted-on when its ack lands within 24h,
// weeks are rolling 7-day buckets, four of them, and a class under
// the 10% acted-on threshold is flagged as a display-only default-off
// proposal (nothing here mutates a preference or a default).
//
// One deliberate divergence: the send timestamp. logSend inserts no
// timestamp column, so the send time is the table's defaulted column
// — in the live Turso table that column is named `at` (verified by a
// read probe 2026-11-25: 271 rows, every one carrying `at`, none
// carrying sent_at/created_at). The aggregator's sendTimestamp reads
// only sent_at/created_at, so against this ledger it dates nothing;
// the port falls back to `at` after those two, in the same order.

import { timed } from "../../src/timed.ts";

export interface LedgerPagesIo {
  /** Run one read-only statement against the ledger; decoded rows. */
  query: (sql: string) => Promise<Record<string, unknown>[]>;
  now: () => number;
}

export const LEDGER_PAGE_PATHS = new Set([
  "/alerts",
  "/api/alerts.json",
  "/notifications",
  "/api/notifications.json",
  "/comments",
  "/api/comments.json",
]);

const JSON_COLUMNS = new Set(["detail", "author", "linked_identity"]);

/** One statement against a libSQL/Turso HTTP endpoint (the /v2/pipeline
 * protocol, ported from aggregator/src/turso-stores.ts): the caller
 * supplies the address and the credential by name from the Worker's
 * environment; SQL here is fixed text with inline integer limits only —
 * no request input ever reaches a statement. */
export function createLedgerQuery(fetchImpl: typeof fetch, url: string, token?: string): LedgerPagesIo["query"] {
  return async (sql: string): Promise<Record<string, unknown>[]> => {
    if (!url) throw new Error("ledger address not configured");
    if (!token) throw new Error("missing server credential");
    const endpoint = new URL(url.replace(/^libsql:/, "https:"));
    if (endpoint.protocol !== "https:" || endpoint.username || endpoint.password)
      throw new Error("invalid ledger address");
    // The house deadline helper (src/timed.ts): a raw AbortSignal
    // timeout leaves its timer running and keeps the object in
    // memory — a test over everything the Worker runs forbids it.
    const out = await timed(30_000, async (signal) => {
      const res = await fetchImpl(`${endpoint.href.replace(/\/$/, "")}/v2/pipeline`, {
        method: "POST",
        headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
        body: JSON.stringify({
          requests: [{ type: "execute", stmt: { sql, args: [], want_rows: true } }, { type: "close" }],
        }),
        signal,
      });
      if (!res.ok) throw new Error(`ledger query failed (HTTP ${res.status})`);
      return (await res.json()) as any;
    });
    const first = out?.results?.[0];
    const result = first?.response?.result;
    if (first?.type !== "ok" || !Array.isArray(result?.cols) || !Array.isArray(result?.rows)) {
      // Do not echo server SQL/errors: rows can carry subscription keys.
      throw new Error("ledger query failed or returned a malformed result");
    }
    return result.rows.map((cells: any[]) => {
      if (cells.length !== result.cols.length) throw new Error("malformed ledger row");
      const row: Record<string, unknown> = {};
      cells.forEach((cell, i) => {
        let value: unknown;
        if (cell.type === "null") value = null;
        else if (cell.type === "text") value = cell.value;
        else if (cell.type === "integer" || cell.type === "float") {
          value = Number(cell.value);
          if (!Number.isFinite(value) || (cell.type === "integer" && !Number.isSafeInteger(value)))
            throw new Error("unsafe ledger numeric value");
        } else throw new Error("unsupported ledger cell");
        const name = result.cols[i].name as string;
        row[name] = JSON_COLUMNS.has(name) && typeof value === "string" ? safeJson(value) : value;
      });
      return row;
    });
  };
}

/** The slice of a D1 database binding this module needs (the
 * platform's D1Database satisfies it structurally): one prepared
 * statement, read whole. SQL here is the same fixed text the Turso
 * path runs — no request input ever reaches a statement. */
export interface LedgerD1Database {
  prepare: (sql: string) => {
    all: () => Promise<{ results?: Record<string, unknown>[] }>;
  };
}

/** The same read against the D1 copy of the ledger (owner ruling
 * 2026-11-26 on #14866: D1 is the ledgers' home now). Rows come back
 * decoded by the binding; the decoding rules are the Turso path's —
 * the JSON text columns are parsed, numbers must be finite (and safe
 * integers when whole), and anything else (a blob, a nested value)
 * fails the read, so a surface answers 503 rather than render a
 * shape it does not understand. */
export function createD1LedgerQuery(db: LedgerD1Database | undefined): LedgerPagesIo["query"] {
  return async (sql: string): Promise<Record<string, unknown>[]> => {
    if (!db) throw new Error("ledger store not configured");
    const out = await db.prepare(sql).all();
    if (!out || !Array.isArray(out.results)) throw new Error("ledger query failed or returned a malformed result");
    return out.results.map((raw) => {
      const row: Record<string, unknown> = {};
      for (const [name, value] of Object.entries(raw)) {
        if (value === null) row[name] = null;
        else if (typeof value === "string") row[name] = JSON_COLUMNS.has(name) ? safeJson(value) : value;
        else if (typeof value === "number") {
          if (!Number.isFinite(value) || (Number.isInteger(value) && !Number.isSafeInteger(value)))
            throw new Error("unsafe ledger numeric value");
          row[name] = value;
        } else throw new Error("unsupported ledger cell");
      }
      return row;
    });
  };
}

function safeJson(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

// --- Shapes (the aggregator's, aggregator/src/alerts.ts) -------------------

export interface Incident {
  id: string;
  source: string;
  incident_key: string;
  severity: "critical" | "warning" | "info";
  title: string;
  body: string | null;
  status: "open" | "acked" | "closed";
  first_seen_at: string;
  last_seen_at: string;
  delivered_at: string | null;
  escalated_at: string | null;
  acked_at: string | null;
  closed_at: string | null;
  occurrences: number;
}

export interface IncidentEvent {
  id: number;
  incident_id: string;
  at: string;
  kind: string;
  detail: Record<string, unknown> | null;
  source?: string;
  incident_key?: string;
  title?: string;
}

export interface NotificationSendRow {
  class: string;
  object_ref: string;
  incident_id: string | null;
  outcome: string;
  sent_at?: string | null;
  created_at?: string | null;
  /** The live Turso table's defaulted send-time column (see header). */
  at?: string | null;
  acted_on_at: string | null;
}

// --- The notifications roll-up (port of aggregator/src/notifications.ts) ----

export const ACTED_WINDOW_MS = 24 * 60 * 60 * 1000;
export const ROLLING_WEEKS = 4;
export const WEEK_MS = 7 * 24 * 60 * 60 * 1000;
export const PROPOSAL_THRESHOLD = 0.1;

export function sendTimestamp(row: NotificationSendRow): string | null {
  return row.sent_at ?? row.created_at ?? row.at ?? null;
}

export interface NotificationWeekRollup {
  start: string;
  sent: number;
  acted: number;
  ratio: number | null;
}

export interface NotificationClassRollup {
  class: string;
  weeks: NotificationWeekRollup[];
  sent: number;
  acted: number;
  ratio: number | null;
  proposal: boolean;
}

export interface NotificationRollup {
  asOf: string;
  windowStart: string;
  classes: NotificationClassRollup[];
  undated: number;
}

export function rollupNotificationSends(
  rows: NotificationSendRow[],
  now: Date,
  weeks: number = ROLLING_WEEKS,
): NotificationRollup {
  const asOfMs = now.getTime();
  const windowStartMs = asOfMs - weeks * WEEK_MS;
  const byClass = new Map<string, { sent: number[]; acted: number[] }>();
  let undated = 0;
  for (const row of rows) {
    if (row.outcome !== "sent") continue;
    const tsRaw = sendTimestamp(row);
    const ts = tsRaw ? Date.parse(tsRaw) : Number.NaN;
    if (!Number.isFinite(ts)) {
      undated += 1;
      continue;
    }
    if (ts < windowStartMs || ts > asOfMs) continue;
    const bucket = Math.min(weeks - 1, Math.floor((ts - windowStartMs) / WEEK_MS));
    let entry = byClass.get(row.class);
    if (!entry) {
      entry = { sent: new Array<number>(weeks).fill(0), acted: new Array<number>(weeks).fill(0) };
      byClass.set(row.class, entry);
    }
    entry.sent[bucket] += 1;
    if (row.acted_on_at) {
      const actedMs = Date.parse(row.acted_on_at);
      if (Number.isFinite(actedMs) && actedMs >= ts && actedMs - ts <= ACTED_WINDOW_MS) {
        entry.acted[bucket] += 1;
      }
    }
  }
  const classes: NotificationClassRollup[] = [...byClass.entries()]
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    .map(([cls, entry]) => {
      const weekRollups: NotificationWeekRollup[] = entry.sent.map((sent, i) => ({
        start: new Date(windowStartMs + i * WEEK_MS).toISOString(),
        sent,
        acted: entry.acted[i],
        ratio: sent > 0 ? entry.acted[i] / sent : null,
      }));
      const sent = entry.sent.reduce((a, b) => a + b, 0);
      const acted = entry.acted.reduce((a, b) => a + b, 0);
      const ratio = sent > 0 ? acted / sent : null;
      return {
        class: cls,
        weeks: weekRollups,
        sent,
        acted,
        ratio,
        proposal: ratio !== null && ratio < PROPOSAL_THRESHOLD,
      };
    });
  return {
    asOf: now.toISOString(),
    windowStart: new Date(windowStartMs).toISOString(),
    classes,
    undated,
  };
}

// --- Comments (aggregator/src/remote-stores.ts commentFromRow) --------------

export interface LedgerComment {
  id: string;
  objectRef: string;
  kind: string;
  author: unknown;
  authorVerified: boolean;
  body: string;
  createdAt: string;
  provenance?: string;
  linkedIdentity?: unknown;
  triageState?: string;
  githubRef?: string;
  githubState?: string;
  githubUrl?: string;
  githubError?: string;
  githubFailedAt?: string;
}

function commentFromRow(row: Record<string, unknown>): LedgerComment {
  const comment: LedgerComment = {
    id: String(row.id),
    objectRef: String(row.object_ref),
    kind: String(row.kind),
    author: row.author ?? null,
    // SQLite (the Turso dialect) returns the boolean as 1/0.
    authorVerified: row.author_verified === true || row.author_verified === 1,
    body: String(row.body),
    createdAt: String(row.created_at),
  };
  if (typeof row.provenance === "string") comment.provenance = row.provenance;
  if (row.linked_identity && typeof row.linked_identity === "object") comment.linkedIdentity = row.linked_identity;
  if (typeof row.triage_state === "string") comment.triageState = row.triage_state;
  if (typeof row.github_ref === "string") comment.githubRef = row.github_ref;
  if (typeof row.github_state === "string") comment.githubState = row.github_state;
  if (typeof row.github_url === "string") comment.githubUrl = row.github_url;
  if (typeof row.github_error === "string") comment.githubError = row.github_error;
  if (typeof row.github_failed_at === "string") comment.githubFailedAt = row.github_failed_at;
  return comment;
}

function authorLabel(author: unknown): string {
  if (typeof author === "string") return author;
  if (author && typeof author === "object") {
    const a = author as Record<string, unknown>;
    for (const key of ["name", "login", "kind", "id"]) {
      if (typeof a[key] === "string" && a[key]) return a[key] as string;
    }
  }
  return "unknown";
}

// --- Rendering ---------------------------------------------------------------

function escapeHtml(value: unknown): string {
  return String(value).replace(
    /[&<>"']/g,
    (char) =>
      ({
        "&": "&amp;",
        "<": "&lt;",
        ">": "&gt;",
        '"': "&quot;",
        "'": "&#39;",
      })[char]!,
  );
}

function page(title: string, body: string, status = 200): Response {
  return new Response(
    `<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1"><title>${escapeHtml(title)} — PM Stack</title>
<style>body{font:16px/1.5 system-ui,sans-serif;background:#0b0f14;color:#e6eaf0;max-width:58rem;margin:5vh auto;padding:0 1rem}a{color:#79c0ff}.context{color:#9da7b1}.item{border:1px solid #30363d;border-radius:.6rem;padding:1rem;margin:.8rem 0;background:#11161d}code{overflow-wrap:anywhere}table{border-collapse:collapse}td,th{border:1px solid #30363d;padding:.3rem .6rem;text-align:right}td:first-child,th:first-child{text-align:left}</style></head><body><main>${body}</main></body></html>`,
    {
      status,
      headers: {
        "Content-Type": "text/html; charset=utf-8",
        "Cache-Control": "no-store",
        "Content-Security-Policy":
          "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
        "X-Content-Type-Options": "nosniff",
        "Referrer-Policy": "no-referrer",
      },
    },
  );
}

function unavailable(surface: string): Response {
  return page(
    surface,
    `<h1>${escapeHtml(surface)} unavailable</h1><p class="context">The fleet-platform ledger read did not answer. No empty or stale result is being shown.</p>`,
    503,
  );
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json", "Cache-Control": "no-store" },
  });
}

function ageLabel(at: string, now: number): string {
  const seconds = Math.max(0, Math.floor((now - Date.parse(at)) / 1000));
  if (seconds <= 60) return `${seconds} seconds`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} minute${minutes === 1 ? "" : "s"}`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} hour${hours === 1 ? "" : "s"}`;
  const days = Math.floor(hours / 24);
  return `${days} day${days === 1 ? "" : "s"}`;
}

function freshnessLine(readAtIso: string, newest: string | null, now: number): string {
  const newestText = newest
    ? `Newest ledger row: ${escapeHtml(newest)} (${ageLabel(newest, now)} ago).`
    : "The ledger holds no rows for this surface yet.";
  return `<p class="context">Hosted read of the fleet-platform ledger at ${escapeHtml(readAtIso)}. ${newestText}</p>`;
}

function newestOf(values: (string | null | undefined)[]): string | null {
  let newest: string | null = null;
  for (const v of values) {
    if (typeof v === "string" && v && (newest === null || v > newest)) newest = v;
  }
  return newest;
}

function excerpt(text: string | null, max: number): string {
  if (!text) return "";
  return text.length > max ? `${text.slice(0, max)}…` : text;
}

// --- The handler ---------------------------------------------------------------

const INCIDENTS_SQL = "select * from alerts_incidents order by (status = 'closed'), first_seen_at desc limit 100";
const EVENTS_SQL =
  "select e.*, i.source, i.incident_key, i.title from alerts_incident_events e join alerts_incidents i on i.id = e.incident_id order by e.id desc limit 100";
const SENDS_SQL = "select * from alerts_notification_sends";
const COMMENTS_SQL =
  "select id, object_ref, kind, author, author_verified, provenance, linked_identity, body, created_at, triage_state, github_ref, github_state, github_url, github_error, github_failed_at from shared_comments order by created_at desc limit 200";

export async function ledgerPage(request: Request, io: LedgerPagesIo): Promise<Response | null> {
  const url = new URL(request.url);
  const path = url.pathname;
  if (request.method !== "GET" || !LEDGER_PAGE_PATHS.has(path)) return null;
  const readAt = new Date(io.now()).toISOString();

  if (path === "/alerts" || path === "/api/alerts.json") {
    let incidents: Incident[];
    let events: IncidentEvent[];
    try {
      const [incidentRows, eventRows] = await Promise.all([io.query(INCIDENTS_SQL), io.query(EVENTS_SQL)]);
      incidents = incidentRows as unknown as Incident[];
      events = eventRows as unknown as IncidentEvent[];
    } catch {
      return path === "/alerts" ? unavailable("Alerts") : json({ error: "alerts store unavailable" }, 503);
    }
    const freshness = {
      readAt,
      newestIncidentAt: newestOf(incidents.map((i) => i.first_seen_at)),
      newestEventAt: newestOf(events.map((e) => e.at)),
    };
    if (path === "/api/alerts.json") return json({ incidents, events, freshness });
    const now = io.now();
    const items = incidents
      .map(
        (
          i,
        ) => `<div class="item"><strong>${escapeHtml(i.severity)} · ${escapeHtml(i.status)}</strong> — ${escapeHtml(i.title)}<br>
<span class="context">${escapeHtml(i.source)} / ${escapeHtml(i.incident_key)} · first seen ${ageLabel(i.first_seen_at, now)} ago · last seen ${ageLabel(i.last_seen_at, now)} ago · ${i.occurrences} occurrence${i.occurrences === 1 ? "" : "s"}</span>${
          i.body ? `<br>${escapeHtml(excerpt(i.body, 300))}` : ""
        }</div>`,
      )
      .join("");
    const eventItems = events
      .map(
        (e) => `<div class="item"><strong>${escapeHtml(e.kind)}</strong> — ${escapeHtml(e.title ?? e.incident_id)}<br>
<span class="context">${escapeHtml(e.at)} (${ageLabel(e.at, now)} ago)${e.source ? ` · ${escapeHtml(e.source)}` : ""}</span></div>`,
      )
      .join("");
    return page(
      "Alerts",
      `<h1>Alerts</h1>${freshnessLine(readAt, freshness.newestEventAt ?? freshness.newestIncidentAt, now)}
<h2>Incidents</h2>${items || `<p class="context">No incidents in the ledger.</p>`}
<h2>Recent events</h2>${eventItems || `<p class="context">No events in the ledger.</p>`}`,
    );
  }

  if (path === "/notifications" || path === "/api/notifications.json") {
    let sends: NotificationSendRow[];
    try {
      sends = (await io.query(SENDS_SQL)) as unknown as NotificationSendRow[];
    } catch {
      return path === "/notifications"
        ? unavailable("Notifications")
        : json({ error: "alerts store unavailable" }, 503);
    }
    const rollup = rollupNotificationSends(sends, new Date(io.now()));
    const freshness = { readAt, newestSendAt: newestOf(sends.map((s) => sendTimestamp(s))) };
    if (path === "/api/notifications.json") return json({ ...rollup, freshness });
    const rows = rollup.classes
      .map((c) => {
        const weeks = c.weeks.map((w) => `<td>${w.sent} / ${w.acted}</td>`).join("");
        return `<tr><td>${escapeHtml(c.class)}</td>${weeks}<td>${c.sent} / ${c.acted}</td><td>${
          c.ratio === null ? "—" : `${Math.round(c.ratio * 100)}%`
        }</td><td>${c.proposal ? "default-off proposal (display only)" : ""}</td></tr>`;
      })
      .join("");
    return page(
      "Notifications",
      `<h1>Notifications</h1>${freshnessLine(readAt, freshness.newestSendAt, io.now())}
<p class="context">Sent-vs-acted-on over the alert throat's send ledger, rolling 7-day weeks ending ${escapeHtml(rollup.asOf)}. Only outcome = "sent" rows are sends; a send counts as acted-on when its ack lands within 24 hours. A class under 10% acted-on is flagged as a default-off proposal — display only; nothing here changes a preference or a default.</p>
<table><tr><th>Class</th><th>Week 1 sent/acted</th><th>Week 2</th><th>Week 3</th><th>Week 4</th><th>Total</th><th>Acted-on</th><th></th></tr>${rows}</table>
<p class="context">${rollup.undated} ledger row${rollup.undated === 1 ? "" : "s"} skipped: no usable send timestamp.</p>`,
    );
  }

  // /comments and /api/comments.json
  let comments: LedgerComment[];
  let rawNewest: string | null;
  try {
    const rows = await io.query(COMMENTS_SQL);
    comments = rows.map(commentFromRow);
    rawNewest = newestOf(rows.map((r) => r.created_at as string));
  } catch {
    return path === "/comments" ? unavailable("Comments") : json({ error: "comments store unavailable" }, 503);
  }
  const freshness = { readAt, newestCommentAt: rawNewest };
  if (path === "/api/comments.json") return json({ comments, freshness });
  const now = io.now();
  const items = comments
    .map(
      (
        c,
      ) => `<div class="item"><strong>${escapeHtml(authorLabel(c.author))}</strong>${c.authorVerified ? " (verified)" : ""} · ${escapeHtml(c.kind)} · <code>${escapeHtml(c.objectRef)}</code><br>
<span class="context">${escapeHtml(c.createdAt)} (${ageLabel(c.createdAt, now)} ago)${c.triageState ? ` · triage: ${escapeHtml(c.triageState)}` : ""}${c.githubState ? ` · github: ${escapeHtml(c.githubState)}` : ""}</span><br>${escapeHtml(excerpt(c.body, 400))}</div>`,
    )
    .join("");
  return page(
    "Comments",
    `<h1>Comments</h1>${freshnessLine(readAt, freshness.newestCommentAt, now)}${items || `<p class="context">No comments in the ledger.</p>`}`,
  );
}
