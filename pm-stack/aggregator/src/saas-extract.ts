// Family-2 extractors over the stewardship quota view's Part (b)
// (hub design §2.3/§3): the sibling of quota-extract.ts, with the same
// fail-soft dated-read discipline. quota-view.md is a local-record
// snapshot — it performs no live checks — so every card here carries
// the freshest dated read it could find, and a service whose patterns
// stop matching renders as an identity shell instead of vanishing or
// inventing numbers. Manual/console reads are a first-class source
// class (§2); nothing here pretends to be a live meter.
//
// Backblaze B2, Fly.io, and Baseten are deliberately NOT services
// here: B2 is omitted until an account exists (§1.4), and Fly/Baseten
// are unheld candidates with no meter and no renewal date.

import type { ChargeRow, DatedRead, PaygEstimate, PaygLine, SaasGroup, SaasService, UsageMetric } from "./model.ts";
import { sectionOf } from "./quota-extract.ts";

const DATE_RE = /\d{4}-\d{2}-\d{2}/g;

/** Every ISO date in a text, ascending. */
function datesIn(text: string): string[] {
  return [...text.matchAll(DATE_RE)].map((m) => m[0]).sort();
}

/** The freshest ISO date in a text, or null. */
function freshestDate(text: string): string | null {
  const ds = datesIn(text);
  return ds.length ? ds[ds.length - 1] : null;
}

/** A `**Label:**` bullet's body text (to the next bullet or section end). */
function bullet(section: string, labelPattern: RegExp): string {
  const lines = section.split("\n");
  let start = -1;
  for (let i = 0; i < lines.length; i++) {
    if (/^-\s/.test(lines[i]) && labelPattern.test(lines[i])) {
      start = i;
      break;
    }
  }
  if (start < 0) return "";
  const out = [lines[start].replace(/^-\s/, "")];
  for (let i = start + 1; i < lines.length; i++) {
    if (/^-\s/.test(lines[i]) || /^#{2,3} /.test(lines[i])) break;
    out.push(lines[i]);
  }
  return out.join("\n").trim();
}

/** One row of the Summary (b) table, by its first cell. */
function summaryRow(doc: string, firstCell: RegExp): string[] | null {
  const summary = sectionOf(doc, /^## Summary \(b\)/);
  for (const line of summary.split("\n")) {
    if (!line.startsWith("|")) continue;
    const cells = line
      .split("|")
      .slice(1, -1)
      .map((c) => c.trim());
    if (cells.length >= 5 && firstCell.test(cells[0])) return cells;
  }
  return null;
}

/** Dated reads: lines carrying an ISO date, freshest first, trimmed. */
function readsOf(section: string, cap = 14): DatedRead[] {
  const reads: DatedRead[] = [];
  for (const raw of section.split("\n")) {
    const line = raw.replace(/^-\s/, "").replace(/\*\*/g, "").trim();
    const at = freshestDate(line);
    if (at && line.length > 12) reads.push({ at, text: line.slice(0, 400) });
  }
  return reads.sort((a, b) => b.at.localeCompare(a.at)).slice(0, cap);
}

/** Lines that record a disagreement between sources — shown, not resolved. */
function conflictsOf(section: string): string[] {
  const out: string[] = [];
  for (const raw of section.split("\n")) {
    const line = raw.replace(/^-\s/, "").replace(/\*\*/g, "").trim();
    if (/conflict|discrepancy|disagree/i.test(line) && line.length > 12) out.push(line.slice(0, 400));
  }
  return out;
}

/**
 * The section's PAYG estimate bullet, Decision-004 style:
 * "low $X / high $Y per month; counted as used = 1.2 × high = $Z".
 * Returns the estimate, or a note explaining its absence.
 */
/** Strip a PAYG bullet's bold label (two shapes: "**PAYG estimate:** …" and "**PAYG estimate (…): …**"). */
function stripPaygLabel(text: string): string {
  return text.replace(/^\*\*PAYG estimate[^*]*:\*\*\s*/, "").replace(/^\*\*PAYG estimate[^*]*:\s*/, "");
}

function paygOf(section: string): { estimate: PaygEstimate | null; note?: string } {
  const text = bullet(section, /\*\*PAYG estimate/);
  if (!text) return { estimate: null };
  const m = /low \$(\d+(?:\.\d+)?) \/ high \$(\d+(?:\.\d+)?) per month/i.exec(text);
  if (m) {
    return {
      estimate: {
        low: Number(m[1]),
        high: Number(m[2]),
        period: "per month",
        note: stripPaygLabel(text).replace(/\*\*/g, "").slice(0, 300),
      },
    };
  }
  if (/not estimable from local records/i.test(text)) {
    return { estimate: null, note: "not estimable from local records" };
  }
  if (/not applicable/i.test(text)) {
    const why = stripPaygLabel(text)
      .replace(/^not applicable\s*—\s*/i, "")
      .split(".")[0];
    return { estimate: null, note: `not applicable — ${why}`.slice(0, 220) };
  }
  return { estimate: null };
}

function metric(partial: Partial<UsageMetric> & Pick<UsageMetric, "id" | "label" | "unit">): UsageMetric {
  const used = partial.used ?? null;
  const limit = partial.limit ?? null;
  const percent =
    partial.percent ??
    (used != null && limit != null && limit > 0 ? Math.round((used / limit) * 1000) / 10 : undefined);
  return { highlight: false, ...partial, used, limit, percent };
}

/** A number like "10M", "25B", "500k", "10" from allowance prose. */
function scaled(num: string, suffix?: string): number {
  const n = Number(num.replace(/,/g, ""));
  switch ((suffix ?? "").toLowerCase()) {
    case "k":
      return n * 1_000;
    case "m":
      return n * 1_000_000;
    case "b":
      return n * 1_000_000_000;
    default:
      return n;
  }
}

const ALLOW = /([\d,.]+)\s*([kMB])?\b/;

/** First "<num>[suffix] <words>" allowance figure in a text, or null. */
function allowance(text: string, after: RegExp): number | null {
  const m = after.exec(text);
  if (!m) return null;
  const n = ALLOW.exec(m[1] ?? "");
  return n ? scaled(n[1], n[2]) : null;
}

// --- per-service metric extractors ----------------------------------------

type MetricExtractor = (section: string, doc: string) => UsageMetric[];

const cloudflareMetrics: MetricExtractor = (section) => {
  const allowances = bullet(section, /\*\*Monthly allowances:\*\*/);
  const r2 = bullet(section, /\*\*R2 \/ daily allowances/);
  const usage = bullet(section, /\*\*Usage \/ pace:\*\*/);
  const both = `${allowances}\n${r2}`;
  const out: UsageMetric[] = [];
  const r2gb = /~(\d+(?:\.\d+)?) GB/.exec(usage);
  out.push(
    metric({
      id: "workers-requests",
      label: "Workers requests",
      unit: "count",
      limit: allowance(both, /Workers ([\d,.]+[kMB]?) requests/),
      highlight: true,
      note: "Current-cycle usage not read — billing/analytics scope is not established for the day-to-day token.",
    }),
    metric({
      id: "workers-cpu-ms",
      label: "Workers CPU time",
      unit: "ms",
      limit: allowance(both, /([\d,.]+[kMB]?) CPU-ms/),
      highlight: true,
    }),
    metric({
      id: "kv-reads",
      label: "KV reads",
      unit: "count",
      limit: allowance(both, /KV ([\d,.]+[kMB]?) reads/),
      highlight: true,
    }),
    metric({
      id: "kv-writes",
      label: "KV writes/deletes/lists",
      unit: "count",
      limit: allowance(both, /([\d,.]+[kMB]?) writes\/deletes\/lists/),
      highlight: true,
    }),
    metric({
      id: "d1-rows-read",
      label: "D1 rows read",
      unit: "rows",
      limit: allowance(both, /D1 ([\d,.]+[kMB]?) rows read/),
      highlight: true,
    }),
    metric({
      id: "d1-rows-written",
      label: "D1 rows written",
      unit: "rows",
      limit: allowance(both, /([\d,.]+[kMB]?) written/),
    }),
    metric({
      id: "queues-ops",
      label: "Queues operations",
      unit: "ops",
      limit: allowance(both, /Queues ([\d,.]+[kMB]?) ops/),
      highlight: true,
    }),
    metric({
      id: "r2-storage",
      label: "R2 storage",
      unit: "gb",
      used: r2gb ? Number(r2gb[1]) : null,
      limit: allowance(both, /R2 Standard ([\d,.]+[kMB]?) GB-mo/),
      highlight: true,
      note: "Footprint read from the Tracker inventory; bucket inventory needs refresh.",
    }),
    metric({
      id: "r2-class-a",
      label: "R2 Class A operations",
      unit: "ops",
      limit: allowance(both, /([\d,.]+[kMB]?) Class A/),
      highlight: true,
    }),
    metric({
      id: "r2-class-b",
      label: "R2 Class B operations",
      unit: "ops",
      limit: allowance(both, /([\d,.]+[kMB]?) Class B/),
      highlight: true,
    }),
    metric({
      id: "workers-ai-neurons",
      label: "Workers AI neurons / day",
      unit: "neurons",
      limit: allowance(both, /Workers AI ([\d,.]+[kMB]?) Neurons\/day/),
      highlight: true,
    }),
    // The long tail — detail view only (§1.5).
    metric({
      id: "logs-events",
      label: "Workers logs events",
      unit: "count",
      limit: allowance(both, /logs ([\d,.]+[kMB]?),/),
    }),
    metric({ id: "kv-storage", label: "KV storage", unit: "gb", limit: allowance(both, /([\d,.]+[kMB]?) GB; D1/) }),
    metric({
      id: "d1-storage",
      label: "D1 storage",
      unit: "gb",
      limit: allowance(both, /written, ([\d,.]+[kMB]?) GB/),
    }),
    metric({
      id: "do-requests",
      label: "Durable Objects requests",
      unit: "count",
      limit: allowance(both, /Durable Objects ([\d,.]+[kMB]?) requests/),
    }),
    metric({
      id: "do-duration",
      label: "Durable Objects duration",
      unit: "other",
      limit: allowance(both, /([\d,.]+[kMB]?) GB-s/),
    }),
    metric({
      id: "workflows-steps",
      label: "Workflows steps",
      unit: "count",
      limit: allowance(both, /Workflows ([\d,.]+[kMB]?) steps/),
    }),
    metric({
      id: "vectorize-queried",
      label: "Vectorize queried dimensions",
      unit: "count",
      limit: allowance(both, /Vectorize ([\d,.]+[kMB]?) queried/),
    }),
    metric({
      id: "vectorize-stored",
      label: "Vectorize stored dimensions",
      unit: "count",
      limit: allowance(both, /([\d,.]+[kMB]?) stored dimensions/),
    }),
    metric({
      id: "containers-memory",
      label: "Containers memory",
      unit: "hours",
      limit: allowance(both, /Containers ([\d,.]+[kMB]?) GiB-hours/),
    }),
    metric({
      id: "containers-vcpu",
      label: "Containers vCPU-minutes",
      unit: "minutes",
      limit: allowance(both, /([\d,.]+[kMB]?) vCPU-min/),
    }),
    metric({
      id: "containers-disk",
      label: "Containers disk",
      unit: "hours",
      limit: allowance(both, /([\d,.]+[kMB]?) GB-hours disk/),
    }),
  );
  const counts = /(\d+) Workers, (\d+) R2 buckets[^;]*; (\d+) D1, (\d+) KV, (\d+) Queues/.exec(usage);
  if (counts) {
    out.push(
      metric({ id: "workers-count", label: "Workers deployed", unit: "count", used: Number(counts[1]) }),
      metric({ id: "r2-buckets", label: "R2 buckets", unit: "count", used: Number(counts[2]) }),
      metric({ id: "d1-count", label: "D1 databases", unit: "count", used: Number(counts[3]) }),
      metric({ id: "kv-count", label: "KV namespaces", unit: "count", used: Number(counts[4]) }),
      metric({ id: "queues-count", label: "Queues", unit: "count", used: Number(counts[5]) }),
    );
  }
  return out.filter((m) => m.used != null || m.limit != null);
};

const tursoMetrics: MetricExtractor = (section) => {
  const plan = bullet(section, /\*\*Plan \/ price \/ allowance:\*\*/);
  const usage = bullet(section, /\*\*Usage \/ dates:\*\*/);
  const db = /(\d+) DB/.exec(usage);
  const zero = /0 B, 0 usage/.test(usage);
  return [
    metric({
      id: "databases",
      label: "Databases",
      unit: "count",
      used: db ? Number(db[1]) : null,
      limit: allowance(plan, /(100) DBs/),
      highlight: true,
    }),
    metric({
      id: "storage",
      label: "Storage",
      unit: "gb",
      used: zero ? 0 : null,
      limit: allowance(plan, /([\d,.]+) GB storage/),
      highlight: true,
      note: zero ? "0 B at the 2026-10-20 dashboard read; later mirror activity is not an org-wide meter." : undefined,
    }),
    metric({
      id: "rows-read",
      label: "Rows read / month",
      unit: "rows",
      limit: allowance(plan, /([\d,.]+[kMB]?) reads\/month/),
      highlight: true,
    }),
    metric({
      id: "rows-written",
      label: "Rows written / month",
      unit: "rows",
      limit: allowance(plan, /([\d,.]+[kMB]?) writes\/month/),
      highlight: true,
    }),
    metric({
      id: "sync",
      label: "Sync / month",
      unit: "gb",
      limit: allowance(plan, /([\d,.]+) GB syncs\/month/),
      highlight: true,
    }),
  ].filter((m) => m.used != null || m.limit != null);
};

const supabaseMetrics: MetricExtractor = () => [
  metric({
    id: "db-size",
    label: "Database size",
    unit: "gb",
    highlight: true,
    note: "Free-plan numerical limits are not recorded in local records; account meters are not verified from this host.",
  }),
  metric({
    id: "egress",
    label: "Egress",
    unit: "gb",
    highlight: true,
    note: "Not measured — manual dashboard read needed.",
  }),
  metric({
    id: "storage",
    label: "Storage",
    unit: "gb",
    highlight: true,
    note: "Not measured — manual dashboard read needed.",
  }),
];

const githubMetrics: MetricExtractor = (section) => {
  const plan = bullet(section, /\*\*Plan \/ price \/ allowance:\*\*/);
  const usage = bullet(section, /\*\*Usage \/ dates:\*\*/);
  const comped = /(\d[\d,]*) Linux \+ (\d+) macOS minutes/.exec(usage);
  const out: UsageMetric[] = [
    metric({
      id: "actions-billable-minutes",
      label: "Actions billable minutes",
      unit: "minutes",
      limit: allowance(plan, /([\d,.]+) Actions minutes\/month/),
      highlight: true,
      note: "No billable-minute volume outside the included/comped bucket is recorded; billing API needs a billing-scoped token (§2.2).",
    }),
    metric({
      id: "codespaces-hours",
      label: "Codespaces core-hours",
      unit: "hours",
      limit: allowance(plan, /([\d,.]+) Codespaces core-hours/),
      highlight: true,
    }),
    metric({
      id: "codespaces-storage",
      label: "Codespaces storage",
      unit: "gb",
      limit: allowance(plan, /([\d,.]+) GB storage\/month/),
      highlight: true,
      note: "Usage unknown in local records.",
    }),
    metric({
      id: "packages",
      label: "Packages storage",
      unit: "gb",
      limit: allowance(plan, /([\d,.]+) GB Packages/),
      highlight: true,
    }),
  ];
  if (comped) {
    out.push(
      metric({
        id: "actions-comped-minutes",
        label: "Actions minutes (comped, October)",
        unit: "minutes",
        used: Number(comped[1].replace(/,/g, "")) + Number(comped[2]),
        note: "Fully comped/discounted — net $0; comped minutes do not consume the plan allowance.",
      }),
    );
  }
  return out.filter((m) => m.used != null || m.limit != null || m.id === "actions-billable-minutes");
};

const modalMetrics: MetricExtractor = (section) => {
  const plan = bullet(section, /\*\*Plan \/ price \/ allowance:\*\*/);
  const credits = /\$(\d+(?:\.\d+)?) monthly compute-credit/.exec(plan);
  return [
    metric({
      id: "compute-credits",
      label: "Compute credits / month",
      unit: "usd",
      limit: credits ? Number(credits[1]) : 30,
      highlight: true,
      note: "No run hours or credit balance recorded — manual dashboard read (§2.2).",
    }),
  ];
};

const ociMetrics: MetricExtractor = (section) => {
  const plan = bullet(section, /\*\*Plan \/ price \/ allowance:\*\*/);
  return [
    metric({
      id: "ocpu-hours",
      label: "A1 OCPU-hours / month",
      unit: "hours",
      limit: allowance(plan, /([\d,.]+) OCPU-hours/),
    }),
    metric({
      id: "gb-hours",
      label: "A1 GB-hours / month",
      unit: "hours",
      limit: allowance(plan, /([\d,.]+) GB-hours\/month/),
    }),
    metric({ id: "block-storage", label: "Block storage", unit: "gb", limit: allowance(plan, /([\d,.]+) GB block/) }),
    metric({
      id: "egress",
      label: "Egress / month",
      unit: "gb",
      limit: allowance(plan, /([\d,.]+) TB egress/) ? allowance(plan, /([\d,.]+) TB egress/)! * 1000 : null,
    }),
    metric({
      id: "object-storage",
      label: "Object storage",
      unit: "gb",
      limit: allowance(plan, /([\d,.]+) GB object storage/),
    }),
  ].filter((m) => m.limit != null);
};

// --- service table ----------------------------------------------------------

interface ServiceSpec {
  id: string;
  name: string;
  group: SaasGroup;
  heading: RegExp;
  summaryKey: RegExp;
  status: SaasService["status"];
  cadenceDays: number;
  metrics: MetricExtractor;
  shellNote: string;
}

const SERVICE_SPECS: ServiceSpec[] = [
  {
    id: "cloudflare",
    name: "Cloudflare",
    group: "compute",
    heading: /^### Cloudflare — Workers platform/,
    summaryKey: /^Cloudflare platform/,
    status: "active",
    cadenceDays: 7,
    metrics: cloudflareMetrics,
    shellNote: "Workers Paid platform; no current read extracted.",
  },
  {
    id: "turso",
    name: "Turso",
    group: "database",
    heading: /^### Turso$/,
    summaryKey: /^Turso/,
    status: "active",
    cadenceDays: 7,
    metrics: tursoMetrics,
    shellNote: "Starter Free; no current read extracted.",
  },
  {
    id: "supabase",
    name: "Supabase",
    group: "database",
    heading: /^### Supabase$/,
    summaryKey: /^Supabase/,
    status: "active",
    cadenceDays: 7,
    metrics: supabaseMetrics,
    shellNote: "Free plan (muse-dev); numerical quotas unknown in local records.",
  },
  {
    id: "github",
    name: "GitHub",
    group: "dev-platform",
    heading: /^### GitHub$/,
    summaryKey: /^GitHub/,
    status: "active",
    cadenceDays: 7,
    metrics: githubMetrics,
    shellNote: "Pro $4/mo; no current read extracted.",
  },
  {
    id: "vercel",
    name: "Vercel",
    group: "compute",
    heading: /^### Vercel$/,
    summaryKey: /^Vercel/,
    status: "unknown",
    cadenceDays: 7,
    metrics: () => [],
    shellNote: "In use for web deploys; tier, price and caps unknown — the first read establishes the baseline (§2.2).",
  },
  {
    id: "modal",
    name: "Modal",
    group: "compute",
    heading: /^### Modal$/,
    summaryKey: /^Modal/,
    status: "active",
    cadenceDays: 31,
    metrics: modalMetrics,
    shellNote: "Starter $0/mo with $30/mo compute credits; usage unknown.",
  },
  {
    id: "oci",
    name: "OCI Always Free",
    group: "compute",
    heading: /^### OCI Always Free$/,
    summaryKey: /^OCI/,
    status: "no-account",
    cadenceDays: 31,
    metrics: ociMetrics,
    shellNote: "No account — signup blocked; placeholder card (§1.4).",
  },
  {
    id: "bitwarden",
    name: "Bitwarden Secrets Manager",
    group: "account-tooling",
    heading: /^### Bitwarden$/,
    summaryKey: /^Bitwarden/,
    status: "active",
    cadenceDays: 7,
    metrics: () => [],
    shellNote:
      "Counts only in this hub — secret names/values never enter it. No count recorded yet; plan/price unknown.",
  },
];

/** Extract every family-2 service from quota-view.md Part (b). */
export function extractSaasServices(doc: string): SaasService[] {
  const services: SaasService[] = [];
  for (const spec of SERVICE_SPECS) {
    const section = sectionOf(doc, spec.heading);
    const row = summaryRow(doc, spec.summaryKey);
    const planCell = row?.[1] ?? "";
    const shell: SaasService = {
      id: spec.id,
      name: spec.name,
      group: spec.group,
      plan: planCell || spec.shellNote,
      status: spec.status,
      metrics: [],
      paygEstimate: null,
      measuredAt: null,
      sourceClass: "manual",
      sourceNote: "quota-view.md Part (b) — local-record snapshot; no live checks performed",
      nextDate: row?.[4] && row[4] !== "unknown" ? row[4] : null,
      cadenceDays: spec.cadenceDays,
      conflicts: [],
      reads: [],
      usageNote: spec.shellNote,
    };
    if (!section) {
      services.push(shell);
      continue;
    }
    try {
      const payg = paygOf(section);
      const usageText = bullet(section, /\*\*Usage \/ dates:\*\*/).replace(/^\*\*Usage \/ dates:\*\*\s*/, "");
      // measuredAt = the freshest date in the section's own "Last
      // checked" record — NOT the freshest date anywhere in the
      // section, which can be a future operational date (a mirror
      // review, a cycle end) rather than a read.
      const measuredAt = freshestDate(bullet(section, /\*\*Last checked:\*\*/)) ?? freshestDate(section);
      services.push({
        ...shell,
        metrics: spec.metrics(section, doc),
        paygEstimate: payg.estimate,
        paygNote: payg.note,
        measuredAt,
        conflicts: conflictsOf(section),
        reads: readsOf(section),
        usageNote: usageText ? usageText.slice(0, 600) : spec.shellNote,
        billedUsd: spec.id === "supabase" ? 0 : undefined,
      });
    } catch {
      services.push(shell);
    }
  }
  // The Cloudflare zone is part of the one Cloudflare card (Q6): fold
  // its dated reads + conflict lines into the platform service.
  const cf = services.find((s) => s.id === "cloudflare");
  const zone = sectionOf(doc, /^### Cloudflare — zone/);
  if (cf && zone) {
    cf.reads = [...readsOf(zone), ...cf.reads].sort((a, b) => b.at.localeCompare(a.at)).slice(0, 14);
    cf.conflicts = [...conflictsOf(zone), ...cf.conflicts];
    const zoneDate = freshestDate(bullet(zone, /\*\*Last checked:\*\*/)) ?? freshestDate(zone);
    if (zoneDate && (!cf.measuredAt || zoneDate > cf.measuredAt)) cf.measuredAt = zoneDate;
  }
  return services;
}

// --- Cloudflare live export overlay (#1048) ----------------------------------

/** One metric row of a cloudflare-quota-export document. */
export interface CloudflareExportMetric {
  id: string;
  used: number;
  unit?: string;
  source?: string;
  detail?: string;
  /** `used` is a lower bound (capped query), not an exact total. */
  lowerBound?: boolean;
}

/** The file-export document written by pm-stack/cloudflare-export. */
export interface CloudflareExportDoc {
  service?: string;
  source?: string;
  status?: string;
  measuredAt?: string | null;
  cycleStart?: string;
  billing?: { status?: string };
  metrics?: CloudflareExportMetric[];
}

/** Freshness bound for preferring export values: the #1151 tick is
 * 15 minutes, so > 30 min means the producer is unreachable/stale. */
export const CLOUDFLARE_EXPORT_STALE_MS = 30 * 60 * 1000;

/**
 * Prefer a fresh cloudflare-export document's live values on the
 * Cloudflare card (#1048): matching metric ids get `used` (and the
 * derived percent) from the export, with the measured-at + source on
 * the metric note. Limits stay the tracker's — the export carries no
 * limits. A missing, failed, unparseable, or stale document changes
 * nothing: the prose/null behaviour is the fallback, never an error.
 */
export function applyCloudflareExport(
  services: SaasService[],
  doc: CloudflareExportDoc | null,
  nowMs: number = Date.now(),
): SaasService[] {
  if (!doc || doc.status !== "ok" || !doc.measuredAt || !Array.isArray(doc.metrics)) return services;
  const measuredMs = Date.parse(doc.measuredAt);
  if (!Number.isFinite(measuredMs)) return services;
  if (nowMs - measuredMs > CLOUDFLARE_EXPORT_STALE_MS || measuredMs > nowMs + 5 * 60 * 1000) return services;
  // A tick that ran just before a UTC month rollover carries the
  // previous cycle's totals; overlaying them would compare a full month
  // against a fresh one's limits. Documents without cycleStart (older
  // producers) stay backwards compatible.
  if (doc.cycleStart) {
    const cycleMs = Date.parse(doc.cycleStart);
    if (!Number.isFinite(cycleMs)) return services;
    const cycle = new Date(cycleMs);
    const current = new Date(nowMs);
    if (cycle.getUTCFullYear() !== current.getUTCFullYear() || cycle.getUTCMonth() !== current.getUTCMonth()) {
      return services;
    }
  }
  const cf = services.find((s) => s.id === "cloudflare");
  if (!cf) return services;
  const byId = new Map(
    doc.metrics
      .filter((m) => m && typeof m.id === "string" && typeof m.used === "number" && Number.isFinite(m.used))
      .map((m) => [m.id, m]),
  );
  if (!byId.size) return services;
  cf.metrics = cf.metrics.map((m) => {
    const live = byId.get(m.id);
    if (!live) return m;
    return {
      ...m,
      used: live.used,
      ...(live.lowerBound ? { lowerBound: true } : {}),
      percent: m.limit != null && m.limit > 0 ? Math.round((live.used / m.limit) * 1000) / 10 : m.percent,
      note: `Live via ${live.source ?? doc.source ?? "file-export/cloudflare"}, measured ${doc.measuredAt}.${live.detail ? ` ${live.detail}` : ""}`,
    };
  });
  cf.measuredAt = doc.measuredAt.slice(0, 10);
  cf.sourceClass = "api";
  cf.sourceNote = `Cloudflare live collector (#1048): ${doc.source ?? "file-export/cloudflare"}, measured ${doc.measuredAt}; limits + plan from quota-view.md Part (b)${
    doc.billing?.status === "invoice-only"
      ? "; billing invoice-only (subscriptions/billing 403 with the day-to-day token)"
      : ""
  }`;
  return services;
}

// --- Supabase live export overlay (#1050) -----------------------------------

/** One metric row of a supabase-quota-export document. */
export interface SupabaseExportMetric {
  id: string;
  label?: string;
  /** null = the probe could not measure it this tick (a documented gap). */
  used: number | null;
  limit?: number | null;
  unit?: string;
  source?: string;
  detail?: string;
  /** "approaching-paid-trigger" when used >= 80% of the Free limit. */
  flag?: string;
}

/** The file-export document written by pm-stack/supabase-export. */
export interface SupabaseExportDoc {
  service?: string;
  source?: string;
  /** ok | partial (some scope gaps recorded) — measurements are present. */
  status?: string;
  measuredAt?: string | null;
  project?: string;
  ref?: string;
  metrics?: SupabaseExportMetric[];
  billing?: { plan?: string; billedUsd?: number; measured?: boolean; source?: string };
  scope?: { endpoint?: string; path?: string; status?: number; note?: string }[];
}

/** Freshness bound mirroring the #1151 tick: > 30 min means the producer is
 * unreachable/stale and the card falls back to its recorded prose. */
export const SUPABASE_EXPORT_STALE_MS = 30 * 60 * 1000;

/**
 * Prefer a fresh supabase-export document on the Supabase card (#1050).
 * Matching metric ids take `used`, `limit`, `unit` and a measured-at/source
 * note from the export; a metric the probe could not measure keeps today's
 * null-but-recorded behaviour with the export's gap detail. `billedUsd`
 * becomes the export's measured addon read instead of the hardcoded 0 —
 * or unknown (null) when the accepted export carries no valid billing
 * measurement, so a failed billing probe never renders as $0.00 billed.
 * Missing, failed, unparseable or stale documents change nothing.
 */
export function applySupabaseExport(
  services: SaasService[],
  doc: SupabaseExportDoc | null,
  nowMs: number = Date.now(),
): SaasService[] {
  if (!doc || (doc.status !== "ok" && doc.status !== "partial")) return services;
  if (!doc.measuredAt || !Array.isArray(doc.metrics)) return services;
  const measuredMs = Date.parse(doc.measuredAt);
  if (!Number.isFinite(measuredMs)) return services;
  if (nowMs - measuredMs > SUPABASE_EXPORT_STALE_MS || measuredMs > nowMs + 5 * 60 * 1000) return services;
  const supabase = services.find((s) => s.id === "supabase");
  if (!supabase) return services;
  const byId = new Map(
    doc.metrics.filter((m) => m && typeof m.id === "string").map((m) => [m.id, m]),
  );
  if (!byId.size) return services;
  supabase.metrics = supabase.metrics.map((m) => {
    const live = byId.get(m.id);
    if (!live) return m;
    const unit = live.unit === "gb" || live.unit === "pct" || live.unit === "count" ? live.unit : m.unit;
    const used = typeof live.used === "number" && Number.isFinite(live.used) ? live.used : null;
    const limit = typeof live.limit === "number" && Number.isFinite(live.limit) ? live.limit : m.limit;
    const parts: string[] = [];
    if (used == null) {
      parts.push(`Not measured this tick${live.detail ? ` — ${live.detail}` : ""}`);
    } else {
      parts.push(`Live via ${live.source ?? doc.source ?? "file-export/supabase"}, measured ${doc.measuredAt}.`);
      if (live.detail) parts.push(live.detail);
      if (live.flag === "approaching-paid-trigger") {
        parts.push("Approaching-paid trigger: at or above 80% of the Free-plan limit (Drew's $30/mo ceiling is the decision context).");
      }
    }
    return {
      ...m,
      used,
      limit,
      unit,
      percent: used != null && limit != null && limit > 0 ? Math.round((used / limit) * 1000) / 10 : undefined,
      note: parts.join(" ").slice(0, 600),
    };
  });
  if (doc.billing && doc.billing.measured !== false && typeof doc.billing.billedUsd === "number" && Number.isFinite(doc.billing.billedUsd)) {
    supabase.billedUsd = doc.billing.billedUsd;
  } else {
    // Codex P2 (PR #2369 re-review, 2026-10-29): an accepted export with no
    // valid billing measurement must not keep the hardcoded 0 from
    // extractSaasServices — the card is stamped fresh/api below, so $0.00
    // would read as a measured billed amount. Unknown (null) renders as
    // no billed line at all.
    supabase.billedUsd = null;
  }
  supabase.plan = doc.billing?.plan ? `Free plan (${doc.project ?? "supabase"}); measured ${doc.measuredAt}` : supabase.plan;
  supabase.measuredAt = doc.measuredAt.slice(0, 10);
  supabase.sourceClass = "api";
  const gapCount = (doc.scope ?? []).filter((s) => typeof s.status === "number" && s.status >= 400).length;
  supabase.sourceNote = `Supabase usage collector (#1050): ${doc.source ?? "file-export/supabase"}, measured ${doc.measuredAt}; ${
    gapCount ? `${gapCount} documented scope gap(s) — see the export's scope array` : "every probed endpoint answered"
  }; limits from the Free-plan record.`;
  return services;
}

// --- PAYG lines (§1.2) -------------------------------------------------------

/**
 * The PAYG dollars card's lines. Sources: the Meta Model API section of
 * quota-view.md (console figures) and, when supplied, the provider
 * registry's billing-incidents section (the Cloudflare gateway accrual
 * the two sources price differently — shown, not resolved).
 */
export function extractPaygLines(doc: string, registryText = ""): PaygLine[] {
  const lines: PaygLine[] = [];
  const meta = sectionOf(doc, /^### Meta Model API/);
  const payg = /PAYG Oct 19–25: \$(\d+(?:\.\d+)?) accrued \/ \$(\d+(?:\.\d+)?) collected/.exec(meta);
  if (payg) {
    const accrued = Number(payg[1]);
    const collected = Number(payg[2]);
    lines.push({
      id: "meta-payg",
      label: "Meta Model API PAYG",
      accruedUsd: accrued,
      collectedUsd: collected,
      unbilledUsd: Math.round((accrued - collected) * 100) / 100,
      note: "Oct 19–25 console read (dev.meta.ai, 2026-10-25 09:38 CDT): spill past the plan's short window bills automatically — no charge-past-plan disable exists on the account.",
      measuredAt: "2026-10-25",
      sourceClass: "console",
    });
  }
  const gw = /\$(\d+(?:\.\d+)?) per budget alert vs \$(\d+(?:\.\d+)?) per gateway cost attribution/.exec(registryText);
  if (gw) {
    lines.push({
      id: "cloudflare-gateway",
      label: "Cloudflare AI Gateway accrual (DeepSeek V4 Pro, Oct 24–25)",
      accruedUsd: null,
      estimate: {
        low: Number(gw[1]),
        high: Number(gw[2]),
        period: "Oct 24–25 accrual",
        note: "Budget alert vs gateway cost attribution disagree — both shown; the Nov 2 invoice is authoritative.",
      },
      unbilledUsd: Math.round(Number(gw[2]) * 1.2 * 100) / 100,
      note: "Gateway providers were disabled fleet-wide 2026-10-25; accrual is closed, the invoice lands Nov 2.",
      measuredAt: "2026-10-25",
      sourceClass: "estimate",
    });
  }
  const cf = extractSaasServices(doc).find((s) => s.id === "cloudflare");
  if (cf?.paygEstimate) {
    lines.push({
      id: "cloudflare-platform-excess",
      label: "Cloudflare platform metered excess (estimate)",
      accruedUsd: null,
      estimate: cf.paygEstimate,
      unbilledUsd: Math.round(cf.paygEstimate.high * 1.2 * 100) / 100,
      note: "Estimate at the recorded footprint, excluding the $5/mo base (counted as subscription). Every meter sits inside its included allotment in the underlying read, so the low is $0.",
      measuredAt: cf.measuredAt,
      sourceClass: "estimate",
    });
  }
  return lines;
}

// --- Charges evidence (§2.4) --------------------------------------------------

/**
 * Charges observed in the local records (card alerts + receipt-class
 * evidence preserved in quota-view.md and the registry). Evidence, not
 * accounting: each row names its source. Empty = the strip explains
 * itself rather than showing a bare zero.
 */
export function extractCharges(doc: string, registryText = ""): ChargeRow[] {
  const rows: ChargeRow[] = [];
  const both = `${doc}\n${registryText}`;
  const visa = /Visa\s*(?:•\s*)?(\d{4})/.exec(both);
  const method = visa ? `Visa •${visa[1]}` : "card";
  const metaCharges =
    /\$(\d+(?:\.\d+)?) charged(?:[^$]*)\+ \$(\d+(?:\.\d+)?) charged/.exec(both) ??
    /\$(\d+(?:\.\d+)?) at 05:15 CDT \+ \$(\d+(?:\.\d+)?) at 07:47 CDT/.exec(both);
  if (metaCharges) {
    rows.push(
      {
        at: "2026-10-25",
        label: "Meta PAYG spill",
        amountUsd: Number(metaCharges[1]),
        method,
        source: "Gmail card alert (registry billing incidents)",
      },
      {
        at: "2026-10-25",
        label: "Meta PAYG spill",
        amountUsd: Number(metaCharges[2]),
        method,
        source: "Gmail card alert (registry billing incidents)",
      },
    );
  }
  // Single dated Meta charge incidents recorded after the 2026-10-25
  // pair: the registry's billing-incident bullets ("[observed — Gmail
  // card alert, <date> …] Meta PAYG (METAPAY) — **$X charged …**
  // (<Card> card ending NNNN …)") and the quota view's Meta section
  // ("Newer dated charge (<date>): $X charged … on <Card> card ending
  // NNNN"). The same charge is written in both records, so rows are
  // deduped on date+amount+card — each incident appears exactly once,
  // with its own date, amount, and payment method.
  const seenIncidents = new Set(rows.map((r) => `${r.at}|${r.amountUsd}|${r.method}`));
  const pushIncident = (at: string, amount: string, cardName: string, last4: string) => {
    const row: ChargeRow = {
      at,
      label: "Meta PAYG spill",
      amountUsd: Number(amount),
      method: `${cardName.trim()} •${last4}`,
      source: "Gmail card alert (registry billing incidents)",
    };
    const key = `${row.at}|${row.amountUsd}|${row.method}`;
    if (!seenIncidents.has(key)) {
      seenIncidents.add(key);
      rows.push(row);
    }
  };
  for (const m of both.matchAll(
    /\[observed — Gmail card alert, (\d{4}-\d{2}-\d{2}) [^\]]*\] Meta PAYG \(METAPAY\) —\s*\*\*\$(\d+(?:\.\d+)?) charged[^*]*\*\* \(([A-Za-z ]+?) card ending (\d{4})/g,
  )) {
    pushIncident(m[1], m[2], m[3], m[4]);
  }
  for (const m of both.matchAll(
    /Newer dated charge \((\d{4}-\d{2}-\d{2})\): \$(\d+(?:\.\d+)?) charged[^.]*?on ([A-Za-z ]+?) card ending (\d{4})/g,
  )) {
    pushIncident(m[1], m[2], m[3], m[4]);
  }
  const cline = sectionOf(doc, /^### ClinePass/);
  const clineCharge = /Cline Bot Inc\., \*\*\$(\d+(?:\.\d+)?)\*\* via Link, invoice (\S+)/.exec(cline);
  if (clineCharge) {
    rows.push({
      at: "2026-10-05",
      label: "ClinePass annual (Cline Bot Inc.)",
      amountUsd: Number(clineCharge[1]),
      method: "Link",
      source: `invoice ${clineCharge[2]} (quota-view.md ClinePass read)`,
    });
  }
  const chatgpt = sectionOf(doc, /^### ChatGPT — main/);
  const mystery = /Unexplained \$(\d+(?:\.\d+)?) Discover charge dated (\d{4}-\d{2}-\d{2})/.exec(chatgpt);
  if (mystery) {
    rows.push({
      at: mystery[2],
      label: "Unconfirmed charge (not confirmed API spending)",
      amountUsd: Number(mystery[1]),
      method: "Discover",
      source: "quota-view.md ChatGPT read",
    });
  }
  return rows.sort((a, b) => b.at.localeCompare(a.at));
}

// --- Subscriptions strip (Q5: renewal-only rows) ------------------------------

/** Flat subscriptions with no meter: plan + next renewal only (§1.4). */
export function extractSubscriptions(doc: string): { name: string; plan: string; renewsAt: string | null }[] {
  const out: { name: string; plan: string; renewsAt: string | null }[] = [];
  const flat = sectionOf(doc, /^### Other \/ flat/);
  if (/Amazon RxPass/.test(flat)) {
    out.push({ name: "Amazon RxPass", plan: "$5/mo flat pharmacy", renewsAt: null });
  }
  const wispr = sectionOf(doc, /^### Wispr Flow/);
  if (wispr) {
    const price = /Flow Pro annual \$(\d+(?:\.\d+)?)\/yr/.exec(wispr);
    const renews = /renews (\d{4}-\d{2}-\d{2})/.exec(wispr);
    out.push({
      name: "Wispr Flow",
      plan: price ? `Pro $${price[1]}/yr` : "Pro (annual)",
      renewsAt: renews ? renews[1] : null,
    });
  }
  return out;
}
