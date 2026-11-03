// The DNS/domain register (#902) as data. The register of record is
// produced by the #902 pipeline in two forms: register.json (slice 2 —
// the authoritative source for the fields it carries: domain,
// registrar, DNS host, next renewal, auto-renew, list price) and the
// register Markdown table, which still carries the editorial columns
// the JSON does not (prior-year cost, where-used, plan/status, review)
// and is the fallback when register.json is missing or malformed.
// The collector builds the DnsSection the fleet feed carries and the
// aggregator renders — the page never edits the register. The Markdown
// parser keys off the register table's header row, so a re-worded
// prose section elsewhere in the file cannot corrupt it.
import { addDaysYmd, chicagoYmd } from "./derive.ts";
import type { DnsEntry, DnsSection } from "./model.ts";
import { isIsoDate } from "./validate.ts";

/** Classification from the register's "Plan / status" cell. */
export function classifyDnsStatus(plan: string): DnsEntry["status"] {
  if (/^active\b/i.test(plan.trim())) return "active";
  if (/earmark/i.test(plan)) return "earmarked";
  return "unused";
}

function cleanCell(s: string): string {
  return s.replace(/\*\*/g, "").trim();
}

/** "$11.08" -> 11.08; anything else -> null. */
export function parsePriceUsd(s: string): number | null {
  const m = /^\$([\d,]+(?:\.\d+)?)$/.exec(s.trim());
  return m ? Number(m[1].replace(/,/g, "")) : null;
}

/** Parse the register Markdown into entries (renewal-sorted). */
export function parseDnsRegister(md: string): DnsEntry[] {
  const lines = md.split("\n");
  const headerIdx = lines.findIndex(
    (l) => l.trimStart().startsWith("|") && /^\|\s*Domain\s*\|/.test(l.trim()) && l.includes("Next renewal"),
  );
  if (headerIdx < 0) return [];
  const out: DnsEntry[] = [];
  for (let i = headerIdx + 1; i < lines.length; i++) {
    const line = lines[i].trim();
    if (!line.startsWith("|")) {
      if (out.length > 0) break; // table ended
      continue; // separator row area
    }
    if (/^\|[\s|:-]+\|$/.test(line)) continue; // |---| separator
    const cells = line.split("|").slice(1, -1).map(cleanCell);
    if (cells.length < 10) continue;
    const [domain, registrar, dnsHost, renewal, autoRenew, price, priorYearCost, used, plan, review] = cells;
    // Shape alone is not enough: an impossible calendar date (2026-02-30,
    // 2026-13-01) must be refused like a malformed row, or it string-sorts
    // into the register, counts toward the annual total, and poisons
    // nextRenewal / renewingSoon (#2199). isIsoDate re-checks the
    // components, so leap dates (2024-02-29) still parse.
    if (!domain || !/^\d{4}-\d{2}-\d{2}$/.test(renewal) || !isIsoDate(renewal)) continue;
    out.push({
      domain,
      registrar,
      dnsHost,
      renewal,
      autoRenew,
      priceUsd: parsePriceUsd(price),
      priorYearCost,
      used,
      plan,
      review,
      status: classifyDnsStatus(plan),
    });
  }
  return out.sort((a, b) => a.renewal.localeCompare(b.renewal) || a.domain.localeCompare(b.domain));
}

/** Assemble the feed section from register Markdown. */
export function dnsSectionFromRegister(md: string, generatedAt: string, source: string): DnsSection | null {
  const entries = parseDnsRegister(md);
  if (entries.length === 0) return null;
  const totalAnnualListUsd = Math.round(entries.reduce((sum, e) => sum + (e.priceUsd ?? 0), 0) * 100) / 100;
  return { generatedAt, source, totalAnnualListUsd, entries };
}

/** One domain row of register.json as the #902 job emits it. */
export interface DnsRegisterJsonDomain {
  domain: string;
  registrar: string;
  dns_hosted_at: string;
  next_renewal: string;
  auto_renew: boolean | null;
  renewal_price_usd: number | null;
  source?: string;
}

/**
 * Assemble the feed section from register.json (slice 2 of #902).
 * The JSON is authoritative for the fields it carries; the editorial
 * columns it does not carry (prior-year cost, where-used, plan/status,
 * review) come from the Markdown parse (`mdEntries`, keyed by domain)
 * so the rendered page is unchanged for current register data. A
 * domain with no Markdown row renders with an empty plan, i.e.
 * classified unused. Returns null on ANY malformation — unparseable
 * JSON, a missing/empty domains list, a duplicate domain, or a row
 * whose core fields are mistyped — so the caller falls back to the
 * Markdown parse instead of serving a half-invented register.
 */
export function dnsSectionFromRegisterJson(
  jsonText: string,
  generatedAt: string,
  source: string,
  mdEntries: DnsEntry[] = [],
): DnsSection | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(jsonText);
  } catch {
    return null;
  }
  if (typeof parsed !== "object" || parsed === null) return null;
  const domains = (parsed as { domains?: unknown }).domains;
  if (!Array.isArray(domains) || domains.length === 0) return null;
  const mdByDomain = new Map(mdEntries.map((e) => [e.domain, e]));
  const seen = new Set<string>();
  const entries: DnsEntry[] = [];
  for (const raw of domains as Partial<DnsRegisterJsonDomain>[]) {
    if (typeof raw !== "object" || raw === null) return null;
    const {
      domain,
      registrar,
      dns_hosted_at: dnsHost,
      next_renewal: renewal,
      auto_renew: autoRenew,
      renewal_price_usd: price,
    } = raw;
    if (typeof domain !== "string" || domain.length === 0 || seen.has(domain)) return null;
    if (typeof registrar !== "string" || typeof dnsHost !== "string") return null;
    if (typeof renewal !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(renewal)) return null;
    if (price !== null && price !== undefined && (typeof price !== "number" || !Number.isFinite(price))) return null;
    if (autoRenew !== null && autoRenew !== undefined && typeof autoRenew !== "boolean") return null;
    seen.add(domain);
    const md = mdByDomain.get(domain);
    const plan = md?.plan ?? "";
    entries.push({
      domain,
      registrar,
      dnsHost,
      renewal,
      autoRenew: autoRenew === true ? "ON" : autoRenew === false ? "OFF" : "unknown",
      priceUsd: price ?? null,
      priorYearCost: md?.priorYearCost ?? "",
      used: md?.used ?? "",
      plan,
      review: md?.review ?? "",
      status: classifyDnsStatus(plan),
    });
  }
  entries.sort((a, b) => a.renewal.localeCompare(b.renewal) || a.domain.localeCompare(b.domain));
  const totalAnnualListUsd = Math.round(entries.reduce((sum, e) => sum + (e.priceUsd ?? 0), 0) * 100) / 100;
  return { generatedAt, source, totalAnnualListUsd, entries };
}

/** Structural check shared by the collector and the live source. */
export function isDnsSection(v: unknown): v is DnsSection {
  if (typeof v !== "object" || v === null) return false;
  const s = v as Partial<DnsSection>;
  return (
    isIsoDate(s.generatedAt) &&
    // totalAnnualListUsd is dereferenced with .toFixed on the /dns and
    // usage render paths, so the gate must guarantee a finite number
    // (#2412) — a missing/string/null/NaN total certifies a section the
    // renderers cannot render.
    typeof s.totalAnnualListUsd === "number" &&
    Number.isFinite(s.totalAnnualListUsd) &&
    // Money is bounded below as well as finite (#4226): a negative
    // total renders as negative spend and records a negative
    // snapshot value downstream.
    s.totalAnnualListUsd >= 0 &&
    Array.isArray(s.entries) &&
    s.entries.length > 0 &&
    s.entries.every(
      (e) =>
        typeof e === "object" &&
        e !== null &&
        typeof (e as DnsEntry).domain === "string" &&
        typeof (e as DnsEntry).renewal === "string" &&
        // priceUsd is number|null in the model: null/absent stays valid,
        // but a present value must be a finite number (#2412).
        ((e as DnsEntry).priceUsd === null ||
          (e as DnsEntry).priceUsd === undefined ||
          (typeof (e as DnsEntry).priceUsd === "number" &&
            Number.isFinite((e as DnsEntry).priceUsd) &&
            // A present price must also be non-negative (#4226).
            (e as DnsEntry).priceUsd >= 0)) &&
        // status feeds summarizeDns buckets; an unknown/missing status
        // under-counts silently, so the gate owns the enum (#2412).
        ((e as DnsEntry).status === "active" ||
          (e as DnsEntry).status === "earmarked" ||
          (e as DnsEntry).status === "unused"),
    )
  );
}

export interface DnsSummary {
  nextRenewal: DnsEntry | null;
  active: DnsEntry[];
  earmarked: DnsEntry[];
  unused: DnsEntry[];
  /** Entries renewing within `days` of `now` (spend horizon). */
  renewingSoon: DnsEntry[];
  /** Counts derived from the register's per-entry auto-renew values. */
  autoRenew: { on: number; off: number; unknown: number };
}

export function summarizeDns(section: DnsSection, now = new Date(), soonDays = 45): DnsSummary {
  const entries = section.entries;
  // "Today" and the spend horizon are Chicago calendar days, the
  // house convention every other surface follows (derive.ts):
  // deriving them from toISOString() used the UTC day, so every
  // evening from 19:00 CDT a domain renewing that Chicago day
  // vanished from nextRenewal and renewingSoon (#1684).
  const today = chicagoYmd(now);
  const cutoff = addDaysYmd(today, soonDays);
  const autoRenew = entries.reduce(
    (counts, entry) => {
      const value = typeof entry.autoRenew === "string" ? entry.autoRenew.trim().toLowerCase() : "";
      if (value === "on") counts.on++;
      else if (value === "off") counts.off++;
      else counts.unknown++;
      return counts;
    },
    { on: 0, off: 0, unknown: 0 },
  );
  return {
    nextRenewal: entries.find((e) => e.renewal >= today) ?? entries[0] ?? null,
    active: entries.filter((e) => e.status === "active"),
    earmarked: entries.filter((e) => e.status === "earmarked"),
    unused: entries.filter((e) => e.status === "unused"),
    renewingSoon: entries.filter((e) => e.renewal >= today && e.renewal <= cutoff),
    autoRenew,
  };
}
