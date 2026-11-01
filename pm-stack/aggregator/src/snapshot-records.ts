// Derivation of §4 snapshot records from the structures the hub
// already extracts — the writers' shared core (hub design §4):
//   (a) the fleet-feed collector derives records from the hub sections
//       of the feed it is about to publish (SaaS services, PAYG lines,
//       the DNS register totals) and embeds them in the feed; the
//       laptop aggregator's background writer tick lands them in its
//       local store (never a page/read path).
//   (b) stewardship-cycle, after a quota-view refresh, runs
//       scripts/append-usage-snapshots.ts, which derives records from
//       the same extractors (pools included) into the outbox the
//       collector drains into the feed.
//   (c) the keeper's check-usage appends its own observations to the
//       same outbox (its Python mirrors recordsFromProviderUsage).
//
// Every record carries its OBSERVATION date (the dated read's own
// date), never the derivation time — that is what makes re-runs
// dedupe in the store instead of spamming it: the store key is
// (service, metric, ts, value), so an unchanged read re-derives the
// identical record and appends nothing.
import { POOL_SPECS, sectionOf } from "./quota-extract.ts";
import { parseIsoDate } from "./validate.ts";
import type {
  DnsSection,
  PaygLine,
  QuotaPool,
  SaasService,
  SnapshotRecord,
  UsageUnit,
} from "./model.ts";

const DATE_RE = /\d{4}-\d{2}-\d{2}/g;

/** A date-only observation, stamped midday UTC — the backfill's convention. */
export function dayTs(date: string): string {
  return `${date}T12:00:00Z`;
}

/** Normalize a read's measuredAt (date or datetime) to a record ts. */
export function tsFor(measuredAt: string): string {
  return measuredAt.length <= 10 ? dayTs(measuredAt.slice(0, 10)) : measuredAt;
}

/** The freshest ISO date in a text, or null. Only REAL calendar
 * dates compete (#2264): an impossible date like 2026-02-30 or
 * 2026-13-01 in prose must not win the freshness comparison. */
export function freshestDate(text: string): string | null {
  const ds = [...text.matchAll(DATE_RE)].map((m) => m[0]).filter((d) => parseIsoDate(d) !== null).sort();
  return ds.length ? ds[ds.length - 1] : null;
}

/**
 * The freshest date in a section's "**Last checked:**" bullet, or
 * null. Sections also carry operational dates that are NOT reads
 * (renewals, cycle ends — sometimes future ones), so the read date
 * comes from the Last-checked record when there is one, mirroring
 * saas-extract's measuredAt discipline.
 */
export function lastCheckedDate(section: string): string | null {
  const lines = section.split("\n");
  let start = -1;
  for (let i = 0; i < lines.length; i++) {
    if (/\*\*Last checked:\*\*/.test(lines[i])) {
      start = i;
      break;
    }
  }
  if (start < 0) return null;
  const out = [lines[start]];
  for (let i = start + 1; i < lines.length; i++) {
    if (/^-\s/.test(lines[i]) || /^#{2,3} /.test(lines[i])) break;
    out.push(lines[i]);
  }
  return freshestDate(out.join("\n"));
}

/** Snapshot units the store accepts; other metric units have no clean mapping. */
const UNIT_MAP: Partial<Record<UsageUnit, SnapshotRecord["unit"]>> = {
  count: "count",
  gb: "gb",
  usd: "usd",
  cny: "cny",
  pct: "pct",
  minutes: "minutes",
  hours: "hours",
};

/** Family-2 services: every measured metric becomes one record at the
 * service's read date, plus the PAYG estimate as a usd-est pair. */
export function recordsFromSaasServices(services: SaasService[], observedBy: string): SnapshotRecord[] {
  const out: SnapshotRecord[] = [];
  for (const svc of services) {
    if (!svc.measuredAt) continue; // no dated read — nowhere honest to place it
    const ts = tsFor(svc.measuredAt);
    for (const m of svc.metrics) {
      if (m.used == null) continue;
      const unit = UNIT_MAP[m.unit];
      if (!unit) continue;
      out.push({
        ts,
        serviceId: svc.id,
        metricId: m.id,
        value: m.used,
        unit,
        ...(m.limit != null ? { limit: m.limit } : {}),
        source: svc.sourceClass,
        observedBy,
      });
    }
    if (svc.paygEstimate) {
      out.push({
        ts,
        serviceId: svc.id,
        // The backfill recorded Cloudflare's platform excess under this
        // exact id; keep it so the series continues instead of forking.
        metricId: svc.id === "cloudflare" ? "platform-excess-est" : "payg-estimate",
        value: { low: svc.paygEstimate.low, high: svc.paygEstimate.high },
        unit: "usd-est",
        source: "estimate",
        observedBy,
      });
    }
  }
  return out;
}

/** PAYG lines: accrued dollars and estimate pairs, keyed to the ids
 * the backfill already used so the series continues. */
export function recordsFromPaygLines(lines: PaygLine[], observedBy: string): SnapshotRecord[] {
  const out: SnapshotRecord[] = [];
  for (const line of lines) {
    if (!line.measuredAt) continue;
    const ts = tsFor(line.measuredAt);
    if (line.accruedUsd != null) {
      out.push({
        ts,
        serviceId: line.id === "meta-payg" ? "meta:high-usage" : line.id,
        metricId: "payg-accrued",
        value: line.accruedUsd,
        unit: "usd",
        source: line.sourceClass,
        observedBy,
      });
    }
    if (line.estimate) {
      // cloudflare-platform-excess duplicates the Cloudflare service's
      // own estimate (extractPaygLines derives the line FROM it) — the
      // service record above already carries it.
      if (line.id === "cloudflare-platform-excess") continue;
      out.push({
        ts,
        serviceId: line.id === "cloudflare-gateway" ? "cloudflare" : line.id,
        metricId: line.id === "cloudflare-gateway" ? "gateway-accrual-est" : "payg-accrual-est",
        value: { low: line.estimate.low, high: line.estimate.high },
        unit: "usd-est",
        source: "estimate",
        observedBy,
      });
    }
  }
  return out;
}

/**
 * The DeepSeek PAYG reserve's balance read (#1052): the dated ¥
 * observation the v1 backfill skipped because the unit enum had no
 * CNY. It is stored as unit "cny" with the yuan figure kept exactly
 * as read — there is no FX rate here and never a converted USD value.
 * The metric id is the gatherer's own window name ("credits_balance")
 * so a backfilled point and a live gathered point are one series.
 *
 * Fail-soft: a section without a ¥ balance, or without any date to
 * stamp it, yields no record — the caller warns instead of guessing.
 */
export function recordsFromDeepseekBalance(quotaViewText: string, observedBy: string): SnapshotRecord[] {
  const section = sectionOf(quotaViewText, /DeepSeek/i);
  if (!section) return [];
  const line = section.split("\n").find((l) => /¥/.test(l));
  if (!line) return [];
  const amount = /¥\s*([\d,]+(?:\.\d+)?)/.exec(line);
  if (!amount) return [];
  const value = Number(amount[1].replace(/,/g, ""));
  if (!Number.isFinite(value)) return [];
  const date = /\d{4}-\d{2}-\d{2}/.exec(line)?.[0] ?? lastCheckedDate(section) ?? freshestDate(section);
  if (!date) return [];
  return [
    {
      ts: dayTs(date),
      serviceId: "deepseek:api",
      metricId: "credits_balance",
      value,
      unit: "cny",
      source: "api",
      observedBy,
    },
  ];
}

/** Family-1 pools: window percentages / USD usage, stamped with the
 * freshest date in each pool's own quota-view section (the read's
 * date — pools carry no measuredAt of their own). */
export function recordsFromPools(pools: QuotaPool[], quotaViewText: string, observedBy: string): SnapshotRecord[] {
  const out: SnapshotRecord[] = [];
  for (const pool of pools) {
    if (!pool.windows.length) continue;
    const spec = POOL_SPECS.find((s) => s.id === pool.id);
    const section = spec ? sectionOf(quotaViewText, spec.heading) : "";
    const date = lastCheckedDate(section) ?? freshestDate(section);
    if (!date) continue;
    const ts = dayTs(date);
    for (const w of pool.windows) {
      if (w.percent != null) {
        out.push({
          ts,
          serviceId: pool.id,
          metricId: w.name,
          value: w.percent,
          unit: "pct",
          limit: 100,
          ...(w.resetsAt ? { resetsAt: w.resetsAt } : {}),
          source: "manual",
          observedBy,
        });
      } else if (w.unit === "usd") {
        out.push({
          ts,
          serviceId: pool.id,
          metricId: w.name,
          value: w.used,
          unit: "usd",
          ...(w.limit != null ? { limit: w.limit } : {}),
          ...(w.resetsAt ? { resetsAt: w.resetsAt } : {}),
          source: "manual",
          observedBy,
        });
      }
    }
  }
  return out;
}

/** DNS register totals. The register carries no read date of its own,
 * so the record is stamped with the register file's mtime — stable
 * across collections until the register is actually edited, which is
 * exactly when a new point belongs in the series. */
export function recordsFromDns(dns: DnsSection, registerMtimeIso: string, observedBy: string): SnapshotRecord[] {
  return [
    {
      ts: registerMtimeIso,
      serviceId: "domains",
      metricId: "total-annual-usd",
      value: dns.totalAnnualListUsd,
      unit: "usd",
      source: "manual",
      observedBy,
    },
    {
      ts: registerMtimeIso,
      serviceId: "domains",
      metricId: "domain-count",
      value: dns.entries.length,
      unit: "count",
      source: "manual",
      observedBy,
    },
  ];
}

/**
 * Keeper check-usage observations (writer (c)) from its snapshot JSON.
 * check-usage.py appends these same records from Python; this mirror
 * exists for the append CLI's --from-provider-usage mode (hand runs
 * against an existing snapshot file). Codex records appear only when
 * the usage endpoint actually answered (it usually 403s — then there
 * is no observation, and none is invented).
 */
export function recordsFromProviderUsage(snap: unknown, observedBy: string): SnapshotRecord[] {
  if (typeof snap !== "object" || snap === null) return [];
  const s = snap as {
    generated_at?: unknown;
    sessions?: unknown;
    codex_usage?: { usage?: { rate_limit?: { primary_window?: unknown; secondary_window?: unknown } } };
  };
  const ts = typeof s.generated_at === "string" ? s.generated_at : null;
  if (!ts || Number.isNaN(Date.parse(ts))) return [];
  const out: SnapshotRecord[] = [];
  if (Array.isArray(s.sessions)) {
    out.push({
      ts,
      serviceId: "opencode-server",
      metricId: "sessions-listed",
      value: s.sessions.length,
      unit: "count",
      source: "api",
      observedBy,
    });
  }
  const rl = s.codex_usage?.usage?.rate_limit;
  const windowMetric = (w: unknown): void => {
    if (typeof w !== "object" || w === null) return;
    const win = w as { used_percent?: unknown; limit_window_seconds?: unknown };
    if (typeof win.used_percent !== "number") return;
    const metricId =
      win.limit_window_seconds === 604_800 ? "7-day" : win.limit_window_seconds === 18_000 ? "5-hour" : null;
    if (!metricId) return;
    out.push({
      ts,
      serviceId: "openai:codex",
      metricId,
      value: win.used_percent,
      unit: "pct",
      limit: 100,
      source: "api",
      observedBy,
    });
  };
  windowMetric(rl?.primary_window);
  windowMetric(rl?.secondary_window);
  return out;
}
