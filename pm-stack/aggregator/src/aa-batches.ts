// AA dataset-version index and per-batch cache shape (#3302).
//
// The selectable versions come from ainventory.sources rows with
// kind='drive-workbook' — the complete register. ingest_batches is optional
// metadata: recent workbook rows have no matching row, so a missing join
// must never hide a batch.
//
// Carriers of the same workbook (re-imports with different timestamps) are
// collapsed only when a stable fingerprint of the batch-bound semantic data
// matches. source_ref, inserted_counts, names and timestamps are NOT dedupe
// keys: the packet records repeated refs/count metadata that do not imply
// identical content.
import { createHash } from "node:crypto";
import type { AaBatchSummary, AaSection } from "./model.ts";
import { isBatchUuid } from "./aa-batch-sql.ts";

/** One raw ainventory.sources workbook row (AA_BATCH_INDEX_SQL shape). */
export interface AaBatchIndexRow {
  importBatch: string;
  name: string | null;
  importedAt: string | null;
  driveModifiedAt: string | null;
  ranAt: string | null;
  insertedCounts: Record<string, number> | null;
}

/** The on-disk shape of one cached batch payload. */
export interface AaBatchCacheEntry {
  batch: string;
  configFingerprint: string;
  fingerprint: string;
  builtAt: string;
  section: AaSection;
}

function str(v: unknown): string | null {
  return typeof v === "string" && v !== "" ? v : null;
}

function counts(v: unknown): Record<string, number> | null {
  if (typeof v !== "object" || v === null || Array.isArray(v)) return null;
  const out: Record<string, number> = {};
  for (const [k, n] of Object.entries(v)) {
    if (typeof n === "number" && Number.isFinite(n)) out[k] = n;
  }
  return Object.keys(out).length > 0 ? out : null;
}

/**
 * Parse AA_BATCH_INDEX_SQL's `batches` array. Rows without a uuid batch id
 * are dropped; everything else is optional metadata. Sorted newest first.
 */
export function parseBatchIndex(raw: unknown): AaBatchIndexRow[] {
  if (!Array.isArray(raw)) return [];
  const rows: AaBatchIndexRow[] = [];
  const seen = new Set<string>();
  for (const entry of raw) {
    if (typeof entry !== "object" || entry === null) continue;
    const r = entry as Record<string, unknown>;
    const importBatch = str(r.importBatch);
    if (!importBatch || !isBatchUuid(importBatch) || seen.has(importBatch)) continue;
    seen.add(importBatch);
    rows.push({
      importBatch,
      name: str(r.name),
      importedAt: str(r.importedAt),
      driveModifiedAt: str(r.driveModifiedAt),
      ranAt: str(r.ranAt),
      insertedCounts: counts(r.insertedCounts),
    });
  }
  return rows.sort((a, b) => (b.importedAt ?? "").localeCompare(a.importedAt ?? ""));
}

/** Human label: import date plus a shortened workbook name. */
export function batchLabel(row: Pick<AaBatchIndexRow, "name" | "importedAt" | "driveModifiedAt">): string {
  const date = (row.importedAt ?? row.driveModifiedAt ?? "").slice(0, 10);
  const name = row.name?.trim() ?? "";
  const short = name.length > 46 ? `${name.slice(0, 45)}…` : name;
  if (date && short) return `${date} · ${short}`;
  return date || short || "unnamed workbook";
}

/** Stable JSON: object keys sorted at every level. */
export function canonicalJson(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (typeof value === "object" && value !== null) {
    const entries = Object.entries(value as Record<string, unknown>)
      .filter(([, v]) => v !== undefined)
      .sort(([a], [b]) => a.localeCompare(b));
    return `{${entries.map(([k, v]) => `${JSON.stringify(k)}:${canonicalJson(v)}`).join(",")}}`;
  }
  return JSON.stringify(value) ?? "null";
}

/**
 * Fingerprint of the batch-bound semantic data only. `prices` is the
 * non-batch v_current_prices overlay and source timestamps are carrier
 * identity, so neither takes part. Two batches with the same fingerprint
 * render the same tables/charts and are offered once.
 */
export function aaBatchFingerprint(raw: {
  rankings?: unknown;
  thresholds?: unknown;
  configs?: unknown;
  testRunRows?: unknown;
  cai?: unknown;
  sources?: unknown;
}): string {
  const sources = Array.isArray(raw.sources)
    ? raw.sources.map((s) => {
        const r = (s ?? {}) as Record<string, unknown>;
        return { name: str(r.name), kind: str(r.kind) };
      })
    : [];
  const semantic = {
    rankings: raw.rankings ?? [],
    thresholds: raw.thresholds ?? [],
    configs: raw.configs ?? [],
    testRunRows: raw.testRunRows ?? [],
    cai: raw.cai ?? [],
    sources,
  };
  return createHash("sha256").update(canonicalJson(semantic)).digest("hex");
}

/**
 * Turn the index + known cache fingerprints into the section's selector
 * list. Newest first; a carrier whose fingerprint matches an already-kept
 * entry is dropped. Null fingerprints (not built yet) are never collapsed
 * because identity is unproven.
 */
export function summarizeBatches(
  rows: AaBatchIndexRow[],
  cache: Map<string, { fingerprint: string | null; cached: boolean }>,
): AaBatchSummary[] {
  const kept: AaBatchSummary[] = [];
  const seen = new Set<string>();
  for (const row of rows) {
    const meta = cache.get(row.importBatch);
    const fingerprint = meta?.fingerprint ?? null;
    if (fingerprint && seen.has(fingerprint)) continue;
    if (fingerprint) seen.add(fingerprint);
    kept.push({
      importBatch: row.importBatch,
      name: row.name,
      importedAt: row.importedAt,
      driveModifiedAt: row.driveModifiedAt,
      ranAt: row.ranAt,
      insertedCounts: row.insertedCounts,
      label: batchLabel(row),
      fingerprint,
      cached: meta?.cached ?? false,
      latest: kept.length === 0,
    });
  }
  return kept;
}

/** The per-batch cache filename; uuids only, so the name is path-safe. */
export function aaCacheFileName(batch: string): string {
  if (!isBatchUuid(batch)) throw new Error("aa cache: invalid batch uuid");
  return `aa-batch-${batch}.json`;
}

/** Structural check for a cached payload file. */
export function isAaBatchCacheEntry(v: unknown): v is AaBatchCacheEntry {
  if (typeof v !== "object" || v === null) return false;
  const e = v as Partial<AaBatchCacheEntry>;
  return (
    typeof e.batch === "string" &&
    isBatchUuid(e.batch) &&
    typeof e.configFingerprint === "string" &&
    typeof e.fingerprint === "string" &&
    typeof e.builtAt === "string" &&
    typeof e.section === "object" &&
    e.section !== null
  );
}
