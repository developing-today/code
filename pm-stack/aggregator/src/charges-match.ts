// charges-match (#1057): the pure core of the usage-hub charges
// auto-matcher. The daily collector (scripts/collect-fleet-feed.ts)
// reads Plaid transactions and Gmail receipt/card alerts, normalizes
// both into ChargeCandidates, and calls matchCharges: one ChargeRow
// per real charge, source-labelled, deduped across sources, matched
// to a hub service where one fits, with unmatched SaaS/AI-looking
// charges returned separately for the attention strip.
//
// Pure and deterministic by design: the same inputs always produce
// the same feed section, so re-running the collector for a period
// republishes identical rows instead of appending duplicates (the
// feed section is replaced, never appended). Persisted
// classification state (which unmatched charges already surfaced)
// lives in the collector, not here — this module only decides what
// a charge IS, never what was already shown.
//
// No balances or debt data pass through here: candidates are charge
// evidence only (date, merchant, amount, payment-method label).

import type { ChargeRow } from "./model.ts";

export type ChargeSourceKind = "plaid" | "gmail";

/** One observed charge from one source, before dedupe/matching. */
export interface ChargeCandidate {
  source: ChargeSourceKind;
  /** ISO date the charge posted / the alert was sent. */
  at: string;
  /** Raw merchant/descriptor text as the source reported it. */
  merchant: string;
  /** Positive = money out (a charge). Non-positive candidates are ignored. */
  amountUsd: number;
  /** Payment method as recorded, e.g. "Visa •7535", when known. */
  method?: string;
}

/** A hub service a charge can be matched to (name + known aliases). */
export interface ChargeServiceRef {
  name: string;
  aliases?: string[];
}

/** A deduped charge that matched no hub service but looks SaaS/AI. */
export interface UnmatchedCharge {
  merchant: string;
  normalized: string;
  amountUsd: number;
  at: string;
  source: ChargeSourceKind;
}

export interface ChargeMatchResult {
  /** One row per deduped charge, date order. */
  rows: ChargeRow[];
  /** Deduped, service-unmatched, SaaS/AI-looking charges (attention). */
  unmatchedSaas: UnmatchedCharge[];
}

/** Charges from two sources for the same purchase post within days. */
export const DEDUPE_WINDOW_DAYS = 3;

/** Whether a string is a real YYYY-MM-DD date in the proleptic Gregorian calendar. */
export function isRealCalendarDate(value: string): boolean {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!match) return false;

  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  if (year < 1 || month < 1 || month > 12 || day < 1) return false;

  const leapYear = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
  const daysInMonth = [31, leapYear ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  return day <= daysInMonth[month - 1]!;
}

const DROP_TOKENS = new Set([
  "US",
  "USA",
  "INC",
  "LLC",
  "LTD",
  "CO",
  "COM",
  "WWW",
  "HTTP",
  "HTTPS",
  "POS",
  "ACH",
  "DEBIT",
  "PURCHASE",
]);

/** Id-like descriptor tokens (mixed letters+digits, e.g. UC5DPA22Y4). */
function isIdLike(token: string): boolean {
  return token.length >= 6 && /[A-Z]/.test(token) && /\d/.test(token);
}

/**
 * Normalize a raw merchant descriptor to a comparable token string:
 * "OPENAI *CHATGPT SUB" -> "OPENAI CHATGPT SUB",
 * "METAPAY*UC5DPA22Y4 US" -> "METAPAY",
 * "CURSOR.COM" -> "CURSOR".
 */
export function normalizeMerchantName(raw: string): string {
  const upper = raw.toUpperCase().replace(/[*]/g, " ");
  const tokens = upper
    .replace(/[^A-Z0-9]+/g, " ")
    .split(/\s+/)
    .filter((t) => t.length > 0)
    .filter((t) => !DROP_TOKENS.has(t))
    .filter((t) => !/^\d+$/.test(t))
    .filter((t) => !isIdLike(t));
  return tokens.join(" ");
}

function squash(normalized: string): string {
  return normalized.replace(/\s+/g, "");
}

function daysBetween(a: string, b: string): number {
  const ms = Math.abs(Date.parse(a) - Date.parse(b));
  return Math.floor(ms / 86_400_000);
}

/** Merchant token strings that mark a charge as SaaS/AI-looking. */
const SAAS_MARKERS = [
  "OPENAI",
  "CHATGPT",
  "ANTHROPIC",
  "CLAUDE",
  "DEEPSEEK",
  "PERPLEXITY",
  "MIDJOURNEY",
  "XAI",
  "GROK",
  "CURSOR",
  "GITHUB",
  "GITLAB",
  "VERCEL",
  "NETLIFY",
  "RAILWAY",
  "SUPABASE",
  "PLANETSCALE",
  "TURSO",
  "MODAL",
  "REPLICATE",
  "HUGGINGFACE",
  "LINEAR",
  "NOTION",
  "FIGMA",
  "ATLASSIAN",
  "DATADOG",
  "SENTRY",
  "CLOUDFLARE",
  "DIGITALOCEAN",
  "LINODE",
  "STRIPE",
  "PADDLE",
  // The fleet's own AI tooling, seen in the October 2026 reads:
  // charges to these are exactly the unmatched SaaS/AI charges the
  // hub exists to surface.
  "META",
  "CLINE",
  "OPENCODE",
  "OPENDESIGN",
  "COMMANDCODE",
  "WINDSURF",
  "CODEIUM",
];

/** Heuristic: does this normalized merchant look like a SaaS/AI charge? */
export function isSaasLooking(normalized: string): boolean {
  const flat = squash(normalized);
  return SAAS_MARKERS.some((marker) => flat.includes(marker));
}

/** The hub service a normalized merchant belongs to, if any. */
export function matchService(normalized: string, services: ChargeServiceRef[]): ChargeServiceRef | undefined {
  const flat = squash(normalized);
  const tokenSet = new Set(normalized.split(" ").filter(Boolean));
  for (const service of services) {
    for (const candidate of [service.name, ...(service.aliases ?? [])]) {
      const norm = normalizeMerchantName(candidate);
      if (!norm) continue;
      const flatNorm = squash(norm);
      if (flatNorm.length >= 4 && flat.includes(flatNorm)) return service;
      const tokens = norm.split(" ").filter(Boolean);
      if (tokens.length > 0 && tokens.every((t) => tokenSet.has(t))) return service;
    }
  }
  return undefined;
}

interface Cluster {
  amountUsd: number;
  normalized: string;
  plaid?: ChargeCandidate;
  gmail?: ChargeCandidate;
  first: ChargeCandidate;
}

/**
 * Dedupe across sources and match to services. Plaid is the source
 * of record on a tie: when both sources saw the charge, the row is
 * labelled "Plaid" and carries the Plaid date/method.
 */
export function matchCharges(candidates: ChargeCandidate[], services: ChargeServiceRef[]): ChargeMatchResult {
  const charges = candidates
    .filter((c) => c.amountUsd > 0 && c.at && c.merchant)
    .map((c) => ({ ...c, amountUsd: Math.round(c.amountUsd * 100) / 100 }));

  const clusters: Cluster[] = [];
  // Plaid first so a cluster's record fields come from Plaid when present.
  const ordered = [...charges].sort((a, b) =>
    a.source === b.source ? a.at.localeCompare(b.at) : a.source === "plaid" ? -1 : 1,
  );
  for (const candidate of ordered) {
    const normalized = normalizeMerchantName(candidate.merchant);
    // Only pair observations from different sources. When normalization
    // removes every token, also require the raw descriptor to agree so
    // unrelated numeric merchant IDs cannot collapse into one charge.
    const existing = clusters.find(
      (cl) =>
        cl.amountUsd === candidate.amountUsd &&
        cl.normalized === normalized &&
        (candidate.source === "plaid" ? !cl.plaid : !cl.gmail) &&
        (normalized !== "" || cl.first.merchant.trim().toUpperCase() === candidate.merchant.trim().toUpperCase()) &&
        daysBetween(cl.first.at, candidate.at) <= DEDUPE_WINDOW_DAYS,
    );
    if (existing) {
      if (candidate.source === "plaid" && !existing.plaid) existing.plaid = candidate;
      if (candidate.source === "gmail" && !existing.gmail) existing.gmail = candidate;
      continue;
    }
    clusters.push({
      amountUsd: candidate.amountUsd,
      normalized,
      plaid: candidate.source === "plaid" ? candidate : undefined,
      gmail: candidate.source === "gmail" ? candidate : undefined,
      first: candidate,
    });
  }

  const rows: ChargeRow[] = [];
  const unmatchedSaas: UnmatchedCharge[] = [];
  for (const cluster of clusters) {
    const record = cluster.plaid ?? cluster.first;
    const service = matchService(cluster.normalized, services);
    rows.push({
      at: record.at,
      label: service ? service.name : record.merchant.trim(),
      amountUsd: cluster.amountUsd,
      method: cluster.plaid?.method ?? cluster.gmail?.method ?? "card",
      source: cluster.plaid ? "Plaid" : "Gmail alert",
    });
    if (!service && isSaasLooking(cluster.normalized)) {
      unmatchedSaas.push({
        merchant: record.merchant.trim(),
        normalized: cluster.normalized,
        amountUsd: cluster.amountUsd,
        at: record.at,
        source: cluster.plaid ? "plaid" : "gmail",
      });
    }
  }
  rows.sort((a, b) => a.at.localeCompare(b.at) || a.label.localeCompare(b.label));
  unmatchedSaas.sort((a, b) => a.at.localeCompare(b.at) || a.merchant.localeCompare(b.merchant));
  return { rows, unmatchedSaas };
}
