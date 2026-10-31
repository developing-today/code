// The DNS/domain register (#902) as data. The register of record is a
// Markdown file produced by the #902 pipeline (a table sorted by next
// renewal); the collector parses it into a DnsSection for the fleet
// feed and the aggregator renders that — the page never edits the
// register. The parser keys off the register table's header row, so a
// re-worded prose section elsewhere in the file cannot corrupt it;
// if #902's job ever emits register.json, this parser is the fallback,
// not the competitor.
import { addDaysYmd, chicagoYmd } from "./derive.ts";
import type { DnsEntry, DnsSection } from "./model.ts";

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
    if (!domain || !/^\d{4}-\d{2}-\d{2}$/.test(renewal)) continue;
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

/** Structural check shared by the collector and the live source. */
export function isDnsSection(v: unknown): v is DnsSection {
  if (typeof v !== "object" || v === null) return false;
  const s = v as Partial<DnsSection>;
  return (
    typeof s.generatedAt === "string" &&
    Number.isFinite(Date.parse(s.generatedAt)) &&
    Array.isArray(s.entries) &&
    s.entries.length > 0 &&
    s.entries.every(
      (e) =>
        typeof e === "object" &&
        e !== null &&
        typeof (e as DnsEntry).domain === "string" &&
        typeof (e as DnsEntry).renewal === "string",
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
  return {
    nextRenewal: entries.find((e) => e.renewal >= today) ?? entries[0] ?? null,
    active: entries.filter((e) => e.status === "active"),
    earmarked: entries.filter((e) => e.status === "earmarked"),
    unused: entries.filter((e) => e.status === "unused"),
    renewingSoon: entries.filter((e) => e.renewal >= today && e.renewal <= cutoff),
  };
}
