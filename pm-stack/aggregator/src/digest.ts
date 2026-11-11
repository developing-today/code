// Scheduled digest (#1278, part of epic #1184): a daily digest — and a
// weekly roll-up on the configured day — generated from the SAME
// canonical projection the dashboard pages render (the Project pool,
// the Needs-Drew census, and the quota view). No separate reporting
// pipeline: buildDigest is a pure function of those inputs, so the
// chat delivery (digestText) and the archived dashboard page
// (renderDigest in render.ts, fed by the JSON archive this module
// writes) can never drift from each other or from the pages.
//
// Honesty rules, mirroring the rest of the stack:
// - The header states the freshness of every facet the digest was
//   derived from (state + age), never an unqualified "live".
// - A section whose source is missing, stale, or incomplete renders
//   anyway, marked stale with its age — a missed source degrades the
//   digest visibly instead of silently omitting the section.
// - Every item in every section carries the link it came from.

import { mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import type { NeedsDrewCensus } from "./needs-drew.ts";
import type { ProjectPoolItem, ProvenanceDetail, QuotaPool, QuotaView } from "./model.ts";
import { poolState, windowPercent } from "./derive.ts";

export type DigestKind = "daily" | "weekly";

export interface DigestItem {
  ref: string;
  title: string;
  url: string;
  /** One-line context: status, blocker, reason, or quota math. */
  note: string;
}

export interface DigestSection {
  id: "moved" | "blocked" | "needsDrew" | "done" | "quota";
  title: string;
  /** True when the section's source was missing/stale/incomplete. */
  stale: boolean;
  /** Why the section is degraded, when it is; null when healthy. */
  staleNote: string | null;
  /** Age of the section's source data at build time, when known. */
  sourceAgeMs: number | null;
  items: DigestItem[];
  /** True when items were capped at DIGEST_SECTION_LIMIT. */
  truncated: boolean;
}

export interface DigestFacetFreshness {
  facet: string;
  state: string;
  ageMs: number | null;
}

export interface Digest {
  kind: DigestKind;
  /** YYYY-MM-DD (UTC) the digest covers — its archive identity. */
  date: string;
  generatedAt: string;
  /** Start of the activity window the moved/done sections cover. */
  windowStart: string;
  windowHours: number;
  /** The canonical page this digest is archived as. */
  archiveRoute: string;
  freshness: DigestFacetFreshness[];
  sections: DigestSection[];
}

export interface DigestInputs {
  pool: ProjectPoolItem[];
  /** Null = the census source could not be read at all. */
  needsDrew: NeedsDrewCensus | null;
  /** Null = the quota view could not be built. */
  quota: QuotaView | null;
  /** Structured per-facet freshness, when the source provides it. */
  provenanceDetail?: ProvenanceDetail | null;
  /** Whether the fleet feed actually carried a projects section
   * (Source.projectsAvailable); false = the pool is unavailable, not
   * empty. Absent = the source cannot say. */
  poolAvailable?: boolean;
  /** The caller's source mode: a "fixtures" facet state is the honest
   * expected state in fixtures mode, but in live mode it means the
   * facet was never primed — stale, not healthy. */
  sourceKind?: "live" | "fixtures";
  now: Date;
  kind: DigestKind;
}

export const DIGEST_SECTION_LIMIT = 20;
/** A facet older than this marks its sections stale in a daily digest. */
export const DIGEST_STALE_AGE_MS = 2 * 60 * 60 * 1000;

const WINDOW_HOURS: Record<DigestKind, number> = { daily: 24, weekly: 24 * 7 };

function isoDay(d: Date): string {
  return d.toISOString().slice(0, 10);
}

function ageText(ageMs: number | null): string {
  if (ageMs === null || !Number.isFinite(ageMs)) return "age unknown";
  const mins = Math.max(0, Math.round(ageMs / 60000));
  if (mins < 60) return `${mins}m old`;
  const hours = Math.floor(mins / 60);
  if (hours < 48) return `${hours}h ${mins % 60}m old`;
  return `${Math.floor(hours / 24)}d ${hours % 24}h old`;
}

function facetStale(
  detail: ProvenanceDetail | null | undefined,
  facet: "items" | "quota",
  staleAgeMs: number,
  sourceKind: "live" | "fixtures" | undefined,
): { stale: boolean; note: string | null; ageMs: number | null } {
  const f = detail?.[facet];
  if (!f) return { stale: false, note: null, ageMs: null };
  const ageMs = typeof f.ageMs === "number" ? f.ageMs : null;
  if (f.state === "fixtures" && sourceKind === "live") {
    return {
      stale: true,
      note: `source facet “${facet}” is still at its fixture default in live mode — the facet was never primed, so its freshness is unmeasured`,
      ageMs,
    };
  }
  if (f.state !== "live" && f.state !== "fixtures") {
    return { stale: true, note: `source facet “${facet}” is ${f.state} (${f.text})`, ageMs };
  }
  if (ageMs !== null && ageMs > staleAgeMs) {
    return { stale: true, note: `source facet “${facet}” is ${ageText(ageMs)}`, ageMs };
  }
  return { stale: false, note: null, ageMs };
}

function cap(items: DigestItem[]): { items: DigestItem[]; truncated: boolean } {
  return { items: items.slice(0, DIGEST_SECTION_LIMIT), truncated: items.length > DIGEST_SECTION_LIMIT };
}

export function buildDigest(inputs: DigestInputs): Digest {
  const { now, kind } = inputs;
  const windowHours = WINDOW_HOURS[kind];
  const windowStartMs = now.getTime() - windowHours * 60 * 60 * 1000;
  const inWindow = (iso: string | undefined): boolean => {
    if (!iso) return false;
    const t = Date.parse(iso);
    return Number.isFinite(t) && t >= windowStartMs && t <= now.getTime();
  };
  const staleAgeMs = kind === "weekly" ? DIGEST_STALE_AGE_MS * 7 : DIGEST_STALE_AGE_MS;

  const poolFreshBase = facetStale(inputs.provenanceDetail, "items", staleAgeMs, inputs.sourceKind);
  const poolFresh =
    inputs.poolAvailable === false
      ? {
          stale: true,
          note: "fleet feed carried no projects section — the pool is unavailable, not empty; pool sections are unverified",
          ageMs: poolFreshBase.ageMs,
        }
      : poolFreshBase;
  const byRef = new Map(inputs.pool.map((i) => [i.ref, i] as const));
  const byUpdatedDesc = (a: ProjectPoolItem, b: ProjectPoolItem) =>
    (b.updatedAt ?? "").localeCompare(a.updatedAt ?? "");

  // --- Moved: open items touched inside the window ----------------------
  const movedAll = inputs.pool
    .filter((i) => i.state === "open" && inWindow(i.updatedAt))
    .sort(byUpdatedDesc)
    .map(
      (i): DigestItem => ({
        ref: i.ref,
        title: i.title,
        url: i.issueUrl,
        note: `${i.status ?? "no project status"} · updated ${i.updatedAt ?? "unknown"}`,
      }),
    );
  const moved = cap(movedAll);

  // --- Done: closed/merged inside the window ----------------------------
  // Selection is by completion time (mergedAt/closedAt), not the last
  // update: a comment on an old closed issue must not resurrect it as
  // "done today" (#5279 Codex). Items whose projection carried no
  // completion stamp fall back to updatedAt.
  const completedAt = (i: ProjectPoolItem) => i.mergedAt ?? i.closedAt ?? i.updatedAt;
  const doneAll = inputs.pool
    .filter((i) => (i.state === "closed" || i.state === "merged") && inWindow(completedAt(i)))
    .sort((a, b) => (completedAt(b) ?? "").localeCompare(completedAt(a) ?? ""))
    .map(
      (i): DigestItem => ({
        ref: i.ref,
        title: i.title,
        url: i.issueUrl,
        note: `${i.state} · completed ${completedAt(i) ?? "unknown"}`,
      }),
    );
  const done = cap(doneAll);

  // --- Blocked: open items with a live (open or unknown) blocker --------
  const blockedAll: DigestItem[] = [];
  for (const i of inputs.pool) {
    if (i.state !== "open" || !i.blockedBy?.length) continue;
    const live = i.blockedBy.filter((ref) => {
      const target = byRef.get(ref);
      return !target || target.state === "open";
    });
    if (!live.length) continue;
    blockedAll.push({
      ref: i.ref,
      title: i.title,
      url: i.issueUrl,
      note: `blocked by ${live
        .map((ref) => (byRef.has(ref) ? ref : `${ref} (state unknown — not in the current pool)`))
        .join(", ")}`,
    });
  }
  blockedAll.sort((a, b) => a.ref.localeCompare(b.ref));
  const blocked = cap(blockedAll);

  // --- Needs Drew: the label census, reasons stated or honestly absent --
  let needsSection: DigestSection;
  if (!inputs.needsDrew) {
    needsSection = {
      id: "needsDrew",
      title: "Needs Drew",
      stale: true,
      staleNote: "Needs-Drew census unavailable — the census source could not be read",
      sourceAgeMs: null,
      items: [],
      truncated: false,
    };
  } else {
    const censusAgeMs = now.getTime() - Date.parse(inputs.needsDrew.fetchedAt);
    const censusStale = !inputs.needsDrew.complete || !Number.isFinite(censusAgeMs) || censusAgeMs > staleAgeMs;
    const items = cap(
      inputs.needsDrew.items.map(
        (i): DigestItem => ({
          ref: i.ref,
          title: i.title,
          url: i.url,
          note: i.reason ?? `labelled ${i.needsLabels.join(", ")} — no reason recorded on the item`,
        }),
      ),
    );
    needsSection = {
      id: "needsDrew",
      title: "Needs Drew",
      stale: censusStale,
      staleNote: !inputs.needsDrew.complete
        ? "Needs-Drew census is incomplete (a repo/label query failed or truncated)"
        : censusStale
          ? `Needs-Drew census is ${ageText(Number.isFinite(censusAgeMs) ? censusAgeMs : null)}`
          : null,
      sourceAgeMs: Number.isFinite(censusAgeMs) ? censusAgeMs : null,
      items: items.items,
      truncated: items.truncated,
    };
  }

  // --- Quota outlook: walled / near-wall pools, walls predicted ---------
  const quotaFresh = facetStale(inputs.provenanceDetail, "quota", staleAgeMs, inputs.sourceKind);
  let quotaSection: DigestSection;
  if (!inputs.quota) {
    quotaSection = {
      id: "quota",
      title: "Quota outlook",
      stale: true,
      staleNote: "quota view unavailable — the quota source could not be read",
      sourceAgeMs: null,
      items: [],
      truncated: false,
    };
  } else {
    const items: DigestItem[] = [];
    for (const pool of inputs.quota.pools) {
      const note = quotaPoolNote(pool, now);
      if (note) {
        items.push({ ref: pool.id, title: `${pool.provider} — ${pool.plan}`, url: "/quota", note });
      }
    }
    const cappedQ = cap(items);
    const degradedProviders = inputs.quota.pools
      .filter(
        (p) =>
          (p.freshness?.state ?? p.state) === "error" ||
          (p.freshness?.state ?? p.state) === "stale" ||
          (p.freshness?.state ?? p.state) === "lapsed",
      )
      .map((p) => p.provider);
    quotaSection = {
      id: "quota",
      title: "Quota outlook",
      stale: quotaFresh.stale || degradedProviders.length > 0,
      staleNote:
        quotaFresh.note ??
        (degradedProviders.length
          ? `provider reads degraded for ${[...new Set(degradedProviders)].join(", ")} — affected pools are marked in their notes`
          : null),
      sourceAgeMs: quotaFresh.ageMs,
      items: cappedQ.items,
      truncated: cappedQ.truncated,
    };
  }

  const poolSection = (
    id: DigestSection["id"],
    title: string,
    built: { items: DigestItem[]; truncated: boolean },
  ): DigestSection => ({
    id,
    title,
    stale: poolFresh.stale,
    staleNote: poolFresh.note,
    sourceAgeMs: poolFresh.ageMs,
    items: built.items,
    truncated: built.truncated,
  });

  const freshness: DigestFacetFreshness[] = [];
  if (inputs.provenanceDetail) {
    for (const [facet, f] of Object.entries(inputs.provenanceDetail)) {
      freshness.push({ facet, state: f.state, ageMs: typeof f.ageMs === "number" ? f.ageMs : null });
    }
  }

  const date = isoDay(now);
  return {
    kind,
    date,
    generatedAt: now.toISOString(),
    windowStart: new Date(windowStartMs).toISOString(),
    windowHours,
    archiveRoute: kind === "weekly" ? `/digest?date=${date}&kind=weekly` : `/digest?date=${date}`,
    freshness,
    sections: [
      poolSection("moved", kind === "weekly" ? "Moved this week" : "Moved today", moved),
      poolSection("blocked", "Blocked", blocked),
      needsSection,
      poolSection("done", kind === "weekly" ? "Done this week" : "Done today", done),
      quotaSection,
    ],
  };
}

/** Why a pool belongs in the outlook, or null when it has headroom. */
function quotaPoolNote(pool: QuotaPool, now: Date): string | null {
  const state = poolState(pool);
  const worst = pool.windows.reduce((m, w) => Math.max(m, windowPercent(w)), 0);
  const parts: string[] = [];
  // Provider freshness (#5279 Codex): quota-current marks a pool
  // error/stale/lapsed while retaining its historical windows, but poolState()
  // derives only utilization — without this, a failed pool with low
  // windows would vanish from the outlook and a stale high reading
  // would read as a current forecast.
  const degraded = pool.freshness?.state ?? (pool.state === "error" || pool.state === "stale" || pool.state === "lapsed" ? pool.state : null);
  if (degraded === "error") {
    parts.push(
      `provider read error${pool.freshness?.error ? `: ${pool.freshness.error}` : ""} — current readings unavailable; any figures below are the last successful read`,
    );
  } else if (degraded === "lapsed") {
    parts.push(
      `provider lapsed${pool.freshness?.error ? `: ${pool.freshness.error}` : ""} — plan cancelled; no current readings`,
    );
  } else if (degraded === "stale") {
    parts.push(
      `provider read stale${pool.freshness?.measuredAt ? ` (measured ${pool.freshness.measuredAt})` : ""} — figures are the last read, not current`,
    );
  }
  if (state === "walled") parts.push(pool.wallLiftsAt ? `walled until ${pool.wallLiftsAt}` : "walled");
  else if (state === "near") parts.push(`near wall — fullest window ${Math.round(worst)}% used`);
  // Wall prediction: at the measured burn, a token window fills before
  // it resets — the digest says so before it happens, not after.
  if (pool.burnTokensPerMin && pool.burnTokensPerMin > 0) {
    for (const w of pool.windows) {
      if (w.unit !== "tokens" || !w.limit || !w.resetsAt) continue;
      const remaining = w.limit - w.used;
      if (remaining <= 0) continue;
      const wallAt = now.getTime() + (remaining / pool.burnTokensPerMin) * 60000;
      if (wallAt < Date.parse(w.resetsAt)) {
        parts.push(
          `at ${Math.round(pool.burnTokensPerMin)} tok/min the ${w.name} window fills in ~${Math.round((wallAt - now.getTime()) / 3600000)}h from now (reset: ${w.resetsAt})`,
        );
        break;
      }
    }
  }
  if (!parts.length && worst >= 80) parts.push(`fullest window ${Math.round(worst)}% used`);
  return parts.length ? parts.join("; ") : null;
}

// --- Chat rendering --------------------------------------------------------

/** The chat delivery: the same content as the archived page, as text. */
export function digestText(digest: Digest): string {
  const lines: string[] = [];
  lines.push(
    `PM ${digest.kind} digest — ${digest.date} (generated ${digest.generatedAt}, window ${digest.windowHours}h from ${digest.windowStart})`,
  );
  if (digest.freshness.length) {
    lines.push(
      `Data freshness: ${digest.freshness.map((f) => `${f.facet}=${f.state}${f.ageMs !== null ? ` (${ageText(f.ageMs)})` : ""}`).join(" · ")}`,
    );
  } else {
    lines.push("Data freshness: not measured by this source — treat every section as unverified.");
  }
  lines.push(`Archived as: ${digest.archiveRoute}`);
  for (const s of digest.sections) {
    lines.push("");
    lines.push(`## ${s.title}${s.stale ? ` — STALE: ${s.staleNote ?? "source degraded"}` : ""}`);
    if (!s.items.length) lines.push(s.stale ? "(unavailable — see the stale note above)" : "(nothing)");
    for (const i of s.items) lines.push(`- ${i.title} — ${i.url} — ${i.note}`);
    if (s.truncated) lines.push(`- … capped at ${DIGEST_SECTION_LIMIT} items`);
  }
  return lines.join("\n");
}

// --- Archive ---------------------------------------------------------------
// One JSON file per digest in the digests dir, named by kind + date:
// the archive IS the dashboard page's data — /digest?date=<date> renders the
// stored digest verbatim, so a re-read shows exactly what was delivered.

export function digestArchiveName(kind: DigestKind, date: string): string {
  return `${kind}-${date}.json`;
}

export async function writeDigestArchive(dir: string, digest: Digest): Promise<string> {
  await mkdir(dir, { recursive: true });
  const file = join(dir, digestArchiveName(digest.kind, digest.date));
  await writeFile(file, `${JSON.stringify(digest, null, 2)}\n`, "utf8");
  return file;
}

export async function readDigestArchive(dir: string, date: string, kind: DigestKind = "daily"): Promise<Digest | null> {
  try {
    const raw = await readFile(join(dir, digestArchiveName(kind, date)), "utf8");
    const parsed = JSON.parse(raw) as Digest;
    return parsed && parsed.date === date && Array.isArray(parsed.sections) ? parsed : null;
  } catch {
    return null;
  }
}

export async function listDigestArchives(dir: string): Promise<{ kind: DigestKind; date: string; route: string }[]> {
  let names: string[];
  try {
    names = await readdir(dir);
  } catch {
    return [];
  }
  const out: { kind: DigestKind; date: string; route: string }[] = [];
  for (const name of names) {
    const m = /^(daily|weekly)-(\d{4}-\d{2}-\d{2})\.json$/.exec(name);
    if (m)
      out.push({
        kind: m[1] as DigestKind,
        date: m[2],
        route: m[1] === "weekly" ? `/digest?date=${m[2]}&kind=weekly` : `/digest?date=${m[2]}`,
      });
  }
  return out.sort((a, b) => b.date.localeCompare(a.date) || a.kind.localeCompare(b.kind));
}
