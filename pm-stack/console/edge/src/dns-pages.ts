// Hosted /dns (#14968 slice 2): the #902 domain register, served by
// this edge from the fleet-platform D1 snapshot the laptop collector
// publishes (aggregator/src/dns-publisher.ts), instead of the host.
//
// The caller must verify the owner BEFORE calling this handler (the
// same contract as ledger-pages.ts). This module is present only
// when the Worker's DNS_PAGES flag reads "on"; without it the paths
// stay the host's, exactly as before. The snapshot lives in the same
// D1 database as the ledgers, read through the same LEDGER_DB
// binding — no new binding, no new store.
//
// The register of record stays the #902 pipeline's: the snapshot row
// carries the exact DnsSection the collector validated, its
// generatedAt, and its source string, beside the publisher's
// receivedAt. Freshness (#14065: no tier renders invented data to
// fill a gap): every answer exposes register generatedAt, publisher
// receivedAt, and source; a missing, malformed, or older-than-15-
// minute snapshot (three missed collector intervals) fails visibly
// with a 503 — never a stale page dressed as current.

import type { LedgerPagesIo } from "./ledger-pages.ts";

export interface DnsEntry {
  domain: string;
  registrar: string;
  dnsHost: string;
  /** Next renewal date, "YYYY-MM-DD". */
  renewal: string;
  /** "ON" | "OFF" | "unknown" — as the register states it. */
  autoRenew: string;
  /** List renewal price in USD, when the register names one. */
  priceUsd: number | null;
  priorYearCost: string;
  /** The register's "Where it's used" evidence cell. */
  used: string;
  /** The register's "Plan / status" cell. */
  plan: string;
  /** The register's "Review?" cell. */
  review: string;
  /** Derived from `plan`: in use, earmarked for a project, or unused. */
  status: "active" | "earmarked" | "unused";
}

export interface DnsSection {
  /** ISO timestamp of the collector run that parsed the register. */
  generatedAt: string;
  /** Where the register came from (file + pipeline owner). */
  source: string;
  /** Sum of list renewal prices across the register, USD/yr. */
  totalAnnualListUsd: number;
  entries: DnsEntry[];
}

export const DNS_PAGE_PATHS = new Set(["/dns", "/api/dns.json"]);

/** The publisher's single-row snapshot (aggregator/src/dns-publisher.ts). */
export const DNS_SNAPSHOT_SQL = "select section_json, generated_at, received_at, source from dns_snapshots where id = 1";

/** Three missed ~5-minute collector intervals. */
export const DNS_SNAPSHOT_MAX_AGE_MS = 15 * 60_000;

/** How far ahead of the reader's clock a receivedAt may sit before
 * it is not freshness but a broken clock (the #2242 skew bound). */
export const DNS_SNAPSHOT_MAX_FUTURE_SKEW_MS = 5 * 60_000;

// --- Validation (a port of the aggregator's gate, aggregator/src/dns.ts) ---

function isRealDate(year: number, month: number, day: number): boolean {
  if (month < 1 || month > 12 || day < 1 || day > 31) return false;
  const d = new Date(Date.UTC(year, month - 1, day));
  return d.getUTCFullYear() === year && d.getUTCMonth() === month - 1 && d.getUTCDate() === day;
}

/** A string that is a real ISO calendar date or timestamp (the
 * aggregator's isIsoDate contract, validate.ts). */
export function isIsoDate(v: unknown): v is string {
  if (typeof v !== "string") return false;
  const dateOnly = /^(\d{4})-(\d{2})-(\d{2})$/.exec(v);
  if (dateOnly) return isRealDate(Number(dateOnly[1]), Number(dateOnly[2]), Number(dateOnly[3]));
  const dt = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(\.\d+)?(Z|[+-]\d{2}:\d{2})$/.exec(v);
  if (!dt) return false;
  if (!isRealDate(Number(dt[1]), Number(dt[2]), Number(dt[3]))) return false;
  if (Number(dt[4]) > 23 || Number(dt[5]) > 59 || Number(dt[6]) > 60) return false;
  return Number.isFinite(Date.parse(v));
}

/** The aggregator's structural gate, ported whole: a snapshot whose
 * section fails it is malformed data, and the surface fails visibly
 * rather than render a shape it cannot stand behind. */
export function isDnsSection(v: unknown): v is DnsSection {
  if (typeof v !== "object" || v === null) return false;
  const s = v as Partial<DnsSection>;
  return (
    isIsoDate(s.generatedAt) &&
    typeof s.totalAnnualListUsd === "number" &&
    Number.isFinite(s.totalAnnualListUsd) &&
    s.totalAnnualListUsd >= 0 &&
    Array.isArray(s.entries) &&
    s.entries.length > 0 &&
    s.entries.every(
      (e) =>
        typeof e === "object" &&
        e !== null &&
        typeof (e as DnsEntry).domain === "string" &&
        isIsoDate((e as DnsEntry).renewal) &&
        (() => {
          const priceUsd = (e as DnsEntry).priceUsd;
          return (
            priceUsd === null ||
            priceUsd === undefined ||
            (typeof priceUsd === "number" && Number.isFinite(priceUsd) && priceUsd >= 0)
          );
        })() &&
        ((e as DnsEntry).status === "active" ||
          (e as DnsEntry).status === "earmarked" ||
          (e as DnsEntry).status === "unused"),
    )
  );
}

// --- The snapshot -------------------------------------------------------------

export interface DnsSnapshot {
  section: DnsSection;
  /** The register's own generation time (the section's generatedAt). */
  generatedAt: string;
  /** When the publisher's write reached D1. */
  receivedAt: string;
  /** The register's provenance (the section's source). */
  source: string;
}

/** One stored snapshot row, decoded; null when the row is not a
 * snapshot this reader can stand behind (bad JSON, a section the
 * gate refuses, or provenance columns that disagree with the
 * section they carry). */
export function dnsSnapshotFromRow(row: Record<string, unknown>): DnsSnapshot | null {
  if (typeof row.section_json !== "string") return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(row.section_json);
  } catch {
    return null;
  }
  if (!isDnsSection(parsed)) return null;
  if (row.generated_at !== parsed.generatedAt || row.source !== parsed.source) return null;
  if (typeof row.received_at !== "string" || !Number.isFinite(Date.parse(row.received_at))) return null;
  return { section: parsed, generatedAt: parsed.generatedAt, receivedAt: row.received_at, source: parsed.source };
}

// --- Rendering (the ledger-pages.ts house shapes, kept beside them) -----------

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

function unavailable(path: string, reason: string, detail: string): Response {
  if (path === "/api/dns.json") return json({ error: `dns snapshot ${reason}`, detail }, 503);
  return page(
    "DNS register",
    `<h1>DNS register unavailable</h1><p class="context">${escapeHtml(detail)} No empty or stale result is being shown.</p>`,
    503,
  );
}

// --- The handler ---------------------------------------------------------------

export async function dnsPage(request: Request, io: LedgerPagesIo): Promise<Response | null> {
  const url = new URL(request.url);
  const path = url.pathname;
  if (request.method !== "GET" || !DNS_PAGE_PATHS.has(path)) return null;
  const readAt = new Date(io.now()).toISOString();

  let rows: Record<string, unknown>[];
  try {
    rows = await io.query(DNS_SNAPSHOT_SQL);
  } catch {
    return unavailable(path, "unavailable", "The fleet-platform DNS snapshot read did not answer.");
  }
  if (rows.length === 0) {
    return unavailable(
      path,
      "missing",
      "No DNS snapshot has been published to the fleet-platform database yet. The register itself is unchanged; only its hosted copy is absent.",
    );
  }
  const snapshot = dnsSnapshotFromRow(rows[0]);
  if (!snapshot) {
    return unavailable(path, "malformed", "The stored DNS snapshot is not a valid register section.");
  }

  const now = io.now();
  const receivedMs = Date.parse(snapshot.receivedAt);
  const ageMs = now - receivedMs;
  const freshness = {
    readAt,
    registerGeneratedAt: snapshot.generatedAt,
    publisherReceivedAt: snapshot.receivedAt,
    source: snapshot.source,
    snapshotAgeMs: ageMs,
    maxAgeMs: DNS_SNAPSHOT_MAX_AGE_MS,
  };
  if (ageMs < -DNS_SNAPSHOT_MAX_FUTURE_SKEW_MS) {
    return unavailable(
      path,
      "malformed",
      `The stored DNS snapshot is dated in the future (publisher received ${snapshot.receivedAt}).`,
    );
  }
  if (ageMs > DNS_SNAPSHOT_MAX_AGE_MS) {
    if (path === "/api/dns.json") return json({ error: "dns snapshot stale", freshness }, 503);
    return page(
      "DNS register",
      `<h1>DNS register snapshot is stale</h1><p class="context">The last snapshot the publisher wrote arrived ${escapeHtml(ageLabel(snapshot.receivedAt, now))} ago (publisher received ${escapeHtml(snapshot.receivedAt)}; register generated ${escapeHtml(snapshot.generatedAt)}; source ${escapeHtml(snapshot.source)}). The limit is 15 minutes — three missed collector intervals. No stale result is being shown as current.</p>`,
      503,
    );
  }

  const { section } = snapshot;
  if (path === "/api/dns.json") {
    return json({
      generatedAt: section.generatedAt,
      source: section.source,
      totalAnnualListUsd: section.totalAnnualListUsd,
      entries: section.entries,
      freshness,
    });
  }
  const counts = {
    active: section.entries.filter((e) => e.status === "active").length,
    earmarked: section.entries.filter((e) => e.status === "earmarked").length,
    unused: section.entries.filter((e) => e.status === "unused").length,
  };
  const rowsHtml = section.entries
    .map(
      (e) => `<tr><td>${escapeHtml(e.domain)}</td><td>${escapeHtml(e.registrar)}</td><td>${escapeHtml(e.dnsHost)}</td><td>${escapeHtml(e.renewal)}</td><td>${escapeHtml(e.autoRenew)}</td><td>${
        e.priceUsd === null ? "—" : `$${e.priceUsd.toFixed(2)}`
      }</td><td>${escapeHtml(e.status)}</td></tr>`,
    )
    .join("");
  return page(
    "DNS register",
    `<h1>DNS register</h1><p class="context">Hosted read of the #902 register snapshot at ${escapeHtml(readAt)}. Register generated ${escapeHtml(snapshot.generatedAt)}; snapshot published ${escapeHtml(snapshot.receivedAt)} (${escapeHtml(ageLabel(snapshot.receivedAt, now))} ago). Source: ${escapeHtml(snapshot.source)}.</p>
<p class="context">${section.entries.length} domains — ${counts.active} active, ${counts.earmarked} earmarked, ${counts.unused} unused. Total list renewal cost $${section.totalAnnualListUsd.toFixed(2)}/yr.</p>
<table><tr><th>Domain</th><th>Registrar</th><th>DNS host</th><th>Next renewal</th><th>Auto-renew</th><th>List price</th><th>Status</th></tr>${rowsHtml}</table>`,
  );
}
