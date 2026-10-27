// The AA cost explorer as data. The source of record is the Supabase
// `ainventory` schema (the AA workbook import: v_rankings,
// v_threshold_metrics, benchmark_results, and the Test_Runs sheet in
// sheet_rows, all read through v_latest_workbook_batch). The collector
// queries it, assembles the section with the helpers here, and embeds
// it in the fleet feed; the aggregator renders the section as-is.
// Everything in this module is pure and unit-tested (test/aa.test.ts),
// like src/dns.ts for the register.
import type {
  AaAbsentSection,
  AaConfig,
  AaCoverage,
  AaFeedSection,
  AaRanking,
  AaSection,
  AaSourceInfo,
  AaTestRun,
  AaThreshold,
} from "./model.ts";

// --- scalar coercion ---------------------------------------------------------
// The Supabase Management API serializes numeric columns as JSON
// strings, so every number crosses a coercion gate here; anything
// unparseable becomes null ("not measured"), never 0.

export function num(v: unknown): number | null {
  if (typeof v === "number") return Number.isFinite(v) ? v : null;
  if (typeof v === "string" && v.trim() !== "") {
    const n = Number(v);
    return Number.isFinite(n) ? n : null;
  }
  return null;
}

function str(v: unknown): string | null {
  return typeof v === "string" && v !== "" ? v : null;
}

function boolish(v: unknown): boolean {
  return v === true || v === 1 || v === "1" || v === "true";
}

// --- Test_Runs sheet mapping -------------------------------------------------

/** One raw sheet_rows row for the Test_Runs sheet: cells as stored. */
export interface SheetRow {
  rowIndex: number;
  cells: unknown[];
}

const TEST_RUN_FIELDS: Record<string, keyof AaTestRun> = {
  test_run_key: "testRunKey",
  model_variant_key: "modelVariantKey",
  snapshot_date: "snapshotDate",
  raw_score: "intelligence",
  cost_per_task_usd: "costPerTaskUsd",
  output_speed_tps: "outputSpeedTps",
  ttft_seconds: "ttftSeconds",
  first_answer_seconds: "firstAnswerSeconds",
  e2e_500_seconds: "e2e500Seconds",
  time_per_task_seconds: "timePerTaskSeconds",
};

/**
 * Map raw Test_Runs rows to timing records, keyed off the sheet's own
 * header row (the row carrying "test_run_key" among its cells) — a
 * workbook column reorder changes nothing here. Rows without a key
 * are dropped; empty cells become nulls.
 */
export function testRunsFromSheetRows(rows: SheetRow[]): AaTestRun[] {
  const header = rows.find(
    (r) => Array.isArray(r.cells) && r.cells.includes("test_run_key") && r.cells.includes("ttft_seconds"),
  );
  if (!header) return [];
  const col = new Map<string, number>();
  header.cells.forEach((name, i) => {
    if (typeof name === "string" && name in TEST_RUN_FIELDS) col.set(name, i);
  });
  const out: AaTestRun[] = [];
  for (const row of rows) {
    if (row === header || !Array.isArray(row.cells)) continue;
    const get = (name: string): unknown => {
      const i = col.get(name);
      return i == null ? undefined : row.cells[i];
    };
    const testRunKey = str(get("test_run_key"));
    if (!testRunKey) continue;
    out.push({
      testRunKey,
      modelVariantKey: str(get("model_variant_key")) ?? "",
      snapshotDate: str(get("snapshot_date")),
      intelligence: num(get("raw_score")),
      costPerTaskUsd: num(get("cost_per_task_usd")),
      outputSpeedTps: num(get("output_speed_tps")),
      ttftSeconds: num(get("ttft_seconds")),
      firstAnswerSeconds: num(get("first_answer_seconds")),
      e2e500Seconds: num(get("e2e_500_seconds")),
      timePerTaskSeconds: num(get("time_per_task_seconds")),
    });
  }
  return out.sort((a, b) => a.testRunKey.localeCompare(b.testRunKey));
}

// --- .opencode/opencode.jsonc coverage inputs ---------------------------------

/** Strip // and block comments without touching string literals
 * (the config carries URLs and names full of slashes), and drop the
 * trailing commas JSONC allows. */
export function stripJsonComments(text: string): string {
  let out = "";
  let i = 0;
  let inStr = false;
  while (i < text.length) {
    const c = text[i];
    if (inStr) {
      out += c;
      if (c === "\\" && i + 1 < text.length) {
        out += text[i + 1];
        i += 2;
        continue;
      }
      if (c === '"') inStr = false;
      i += 1;
      continue;
    }
    if (c === '"') {
      inStr = true;
      out += c;
      i += 1;
      continue;
    }
    if (c === "/" && text[i + 1] === "/") {
      while (i < text.length && text[i] !== "\n") i += 1;
      continue;
    }
    if (c === "/" && text[i + 1] === "*") {
      i += 2;
      while (i + 1 < text.length && !(text[i] === "*" && text[i + 1] === "/")) i += 1;
      i += 2;
      continue;
    }
    out += c;
    i += 1;
  }
  return dropTrailingCommas(out);
}

/** Drop commas whose next non-whitespace char closes an object/array
 * (JSONC allows trailing commas; JSON.parse does not). String-aware. */
function dropTrailingCommas(text: string): string {
  let out = "";
  let inStr = false;
  for (let k = 0; k < text.length; k++) {
    const c = text[k];
    if (inStr) {
      out += c;
      if (c === "\\" && k + 1 < text.length) {
        out += text[k + 1];
        k += 1;
        continue;
      }
      if (c === '"') inStr = false;
      continue;
    }
    if (c === '"') {
      inStr = true;
      out += c;
      continue;
    }
    if (c === ",") {
      let m = k + 1;
      while (m < text.length && /\s/.test(text[m])) m += 1;
      if (text[m] === "}" || text[m] === "]") continue;
    }
    out += c;
  }
  return out;
}

export interface OpencodeCoverageInputs {
  /** Full catalog ids, "provider/model…" (model part may contain /). */
  catalog: string[];
  /** Full leaf model ids reachable from any model route, nested
   * opencode-route/* targets expanded, #variant pins stripped. */
  pooled: string[];
}

/**
 * Extract the provider catalog and the pooled leaf set from the repo's
 * .opencode/opencode.jsonc. Returns null when the file does not parse —
 * coverage is best-effort and its absence is stated, never guessed.
 */
export function parseOpencodeCoverageInputs(text: string): OpencodeCoverageInputs | null {
  let cfg: unknown;
  try {
    cfg = JSON.parse(stripJsonComments(text));
  } catch {
    return null;
  }
  if (typeof cfg !== "object" || cfg === null) return null;
  const root = cfg as { provider?: unknown; experimental?: { model_routes?: unknown } };

  const catalog: string[] = [];
  if (typeof root.provider === "object" && root.provider !== null) {
    for (const [providerId, providerCfg] of Object.entries(root.provider as Record<string, unknown>)) {
      const models = (providerCfg as { models?: unknown } | null)?.models;
      if (typeof models === "object" && models !== null) {
        for (const modelId of Object.keys(models as Record<string, unknown>)) {
          catalog.push(`${providerId}/${modelId}`);
        }
      }
    }
  }

  const routes = new Map<string, unknown[]>();
  const rawRoutes = root.experimental?.model_routes;
  if (typeof rawRoutes === "object" && rawRoutes !== null) {
    for (const [routeId, route] of Object.entries(rawRoutes as Record<string, unknown>)) {
      const targets = (route as { targets?: unknown } | null)?.targets;
      if (Array.isArray(targets)) routes.set(routeId, targets);
    }
  }
  const pooled = new Set<string>();
  const expand = (routeId: string, seen: Set<string>): void => {
    if (seen.has(routeId)) return;
    seen.add(routeId);
    for (const target of routes.get(routeId) ?? []) {
      const ref =
        typeof target === "string"
          ? target
          : typeof target === "object" && target !== null && typeof (target as { model?: unknown }).model === "string"
            ? ((target as { model: string }).model as string)
            : null;
      if (!ref) continue;
      const bare = ref.split("#")[0];
      if (bare.startsWith("opencode-route/")) expand(bare.slice("opencode-route/".length), seen);
      else pooled.add(bare);
    }
  };
  for (const routeId of routes.keys()) expand(routeId, new Set());
  return { catalog: catalog.sort(), pooled: [...pooled].sort() };
}

// --- name matching -------------------------------------------------------------
// AA display names ("GPT-6 Luna", "DeepSeek V4 Pro 0813") vs config ids
// ("openai/gpt-6-luna", "deepseek/deepseek-v4-pro"): normalize both to
// space-joined lowercase alnum tokens, drop serving-variant noise
// tokens, then apply the alias map for the known mismatches. A miss
// means "not matched", never "does not exist".

const NOISE_TOKENS = new Set(["contributor", "free", "fast", "thinking", "vision", "exp", "preview"]);

/** Normalized-name aliases: variant spelling -> canonical spelling. */
const MODEL_ALIASES = new Map<string, string>([
  // AA names the checkpoint; the config ids drop the date suffix.
  ["deepseek v4 pro 0813", "deepseek v4 pro"],
  // OpenDesign's leaf kept the pre-1017 family name for the same model.
  ["deepseek v4 flash", "deepseek v4 1 flash"],
  // Meta's config ids carry the 1.0 version the AA family name drops.
  ["muse wonder 1 0", "muse wonder"],
  // AA's workbook family for Qwen's max checkpoint carries its date.
  ["qwen3 8 max 0902", "qwen3 8 max"],
  // Word order differs between the AA family and the config ids.
  ["claude haiku 4 5", "claude 4 5 haiku"],
]);

export function normalizeModelName(name: string): string {
  const tokens = name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, " ")
    .split(/\s+/)
    .filter((t) => t !== "" && !NOISE_TOKENS.has(t));
  const joined = tokens.join(" ");
  return MODEL_ALIASES.get(joined) ?? joined;
}

/** The model part of a full "provider/model…" id. Some providers
 * nest a vendor prefix inside the model id ("commandcode/deepseek/…",
 * "cloudflare/anthropic/…") — the leaf segment is the model. */
function modelPart(id: string): string {
  const first = id.indexOf("/");
  if (first < 0) return id;
  const rest = id.slice(first + 1);
  const last = rest.lastIndexOf("/");
  return last >= 0 ? rest.slice(last + 1) : rest;
}

const byAlpha = (a: string, b: string): number => a.toLowerCase().localeCompare(b.toLowerCase());

/**
 * Compare the AA-measured model names against the catalog + pooled
 * sets. Display names in the first three lists are the AA/workbook
 * names; the fourth lists catalog ids verbatim (that is where a human
 * goes looking for them in the config).
 */
export function computeAaCoverage(measuredNames: string[], inputs: OpencodeCoverageInputs): AaCoverage {
  const catalogNorm = new Set(inputs.catalog.map((id) => normalizeModelName(modelPart(id))));
  const pooledNorm = new Set(inputs.pooled.map((id) => normalizeModelName(modelPart(id))));
  const measuredNotPooled: string[] = [];
  const catalogNotPooledMeasured: string[] = [];
  const measuredNotInCatalog: string[] = [];
  const seenMeasured = new Set<string>();
  for (const name of measuredNames) {
    const n = normalizeModelName(name);
    if (seenMeasured.has(n)) continue;
    seenMeasured.add(n);
    if (pooledNorm.has(n)) continue;
    measuredNotPooled.push(name);
    if (catalogNorm.has(n)) catalogNotPooledMeasured.push(name);
    else measuredNotInCatalog.push(name);
  }
  const unmeasured = new Map<string, string>();
  for (const id of inputs.catalog) {
    const n = normalizeModelName(modelPart(id));
    if (pooledNorm.has(n) || seenMeasured.has(n) || unmeasured.has(n)) continue;
    unmeasured.set(n, id);
  }
  return {
    measuredNotPooled: measuredNotPooled.sort(byAlpha),
    catalogNotPooledMeasured: catalogNotPooledMeasured.sort(byAlpha),
    measuredNotInCatalog: measuredNotInCatalog.sort(byAlpha),
    catalogNotPooledUnmeasured: [...unmeasured.values()].sort(byAlpha),
  };
}

// --- section assembly + validation ---------------------------------------------

/** The collector's raw Supabase payload (one jsonb object; snake-case
 * keys are renamed to the section's camelCase by the SQL itself). */
export interface AaRawPayload {
  batch?: unknown;
  sources?: unknown;
  rankings?: unknown;
  thresholds?: unknown;
  configs?: unknown;
  testRunRows?: unknown;
}

function rankingFromRaw(v: unknown): AaRanking | null {
  if (typeof v !== "object" || v === null) return null;
  const r = v as Record<string, unknown>;
  if (typeof r.testRunKey !== "string" || typeof r.family !== "string") return null;
  return {
    rowIndex: num(r.rowIndex) ?? 0,
    testRunKey: r.testRunKey,
    modelVariantKey: str(r.modelVariantKey) ?? "",
    family: r.family,
    creator: str(r.creator) ?? "",
    effort: str(r.effort),
    benchmarkVersionKey: str(r.benchmarkVersionKey),
    rawScore: num(r.rawScore),
    adjustedScore: num(r.adjustedScore),
    costTask: num(r.costTask),
    timeTaskSec: num(r.timeTaskSec),
    speedTps: num(r.speedTps),
    qualStatus: str(r.qualStatus),
    availability: str(r.availability),
    eligible: boolish(r.eligible),
    intelEffPerUsd: num(r.intelEffPerUsd),
    usdPerUsableUnit: num(r.usdPerUsableUnit),
    pointsPerDollar: num(r.pointsPerDollar),
    pointsPerSecond: num(r.pointsPerSecond),
    balancedValue: num(r.balancedValue),
    smartValue: num(r.smartValue),
    speedValue: num(r.speedValue),
    rankBalanced: num(r.rankBalanced),
    rankSmart: num(r.rankSmart),
    rankSpeed: num(r.rankSpeed),
    rankIntelligence: num(r.rankIntelligence),
    rankLowCost: num(r.rankLowCost),
  };
}

function thresholdFromRaw(v: unknown): AaThreshold | null {
  if (typeof v !== "object" || v === null) return null;
  const t = v as Record<string, unknown>;
  const target = num(t.target);
  if (typeof t.testRunKey !== "string" || target === null) return null;
  return {
    testRunKey: t.testRunKey,
    target,
    usable: boolish(t.usable),
    levelledScore: num(t.levelledScore),
    costTask: num(t.costTask),
    timeMin: num(t.timeMin),
    usdPerUsablePoint: num(t.usdPerUsablePoint),
  };
}

function configFromRaw(v: unknown): AaConfig | null {
  if (typeof v !== "object" || v === null) return null;
  const c = v as Record<string, unknown>;
  if (typeof c.modelName !== "string" || typeof c.benchmark !== "string") return null;
  return {
    modelName: c.modelName,
    benchmark: c.benchmark,
    effort: str(c.effort),
    intelligence: num(c.intelligence),
    costPerTaskUsd: num(c.costPerTaskUsd),
    speedTps: num(c.speedTps),
  };
}

function sourceFromRaw(v: unknown): AaSourceInfo | null {
  if (typeof v !== "object" || v === null) return null;
  const s = v as Record<string, unknown>;
  if (typeof s.name !== "string") return null;
  return {
    name: s.name,
    kind: str(s.kind) ?? "",
    importedAt: str(s.importedAt),
    driveModifiedAt: str(s.driveModifiedAt),
  };
}

function sheetRowsFromRaw(v: unknown): SheetRow[] {
  if (!Array.isArray(v)) return [];
  const out: SheetRow[] = [];
  for (const row of v) {
    if (typeof row !== "object" || row === null) continue;
    const r = row as { rowIndex?: unknown; cells?: unknown };
    if (Array.isArray(r.cells)) out.push({ rowIndex: num(r.rowIndex) ?? 0, cells: r.cells });
  }
  return out;
}

/**
 * Assemble the feed section from the Supabase payload + the repo's
 * opencode config text (null/unparseable = coverage omitted, stated in
 * the section by its absence). Returns null when the payload is not
 * recognizably the AA data — the collector degrades to the absent form.
 */
export function aaSectionFromSupabase(
  raw: AaRawPayload,
  opencodeConfigText: string | null,
  generatedAt: string,
): AaSection | null {
  if (typeof raw.batch !== "string" || !Array.isArray(raw.rankings) || !Array.isArray(raw.configs)) return null;
  const rankings = raw.rankings.map(rankingFromRaw).filter((r): r is AaRanking => r !== null);
  const configs = raw.configs.map(configFromRaw).filter((c): c is AaConfig => c !== null);
  if (rankings.length === 0 || configs.length === 0) return null;
  const thresholds = Array.isArray(raw.thresholds)
    ? raw.thresholds.map(thresholdFromRaw).filter((t): t is AaThreshold => t !== null)
    : [];
  const sources = Array.isArray(raw.sources)
    ? raw.sources.map(sourceFromRaw).filter((s): s is AaSourceInfo => s !== null)
    : [];
  const testRuns = testRunsFromSheetRows(sheetRowsFromRaw(raw.testRunRows));
  const measuredNames = [...new Set([...rankings.map((r) => r.family), ...configs.map((c) => c.modelName)])];
  let coverage: AaCoverage | undefined;
  if (opencodeConfigText) {
    const inputs = parseOpencodeCoverageInputs(opencodeConfigText);
    if (inputs) coverage = computeAaCoverage(measuredNames, inputs);
  }
  return {
    status: "ok",
    generatedAt,
    batch: raw.batch,
    sources,
    rankings,
    thresholds,
    configs,
    testRuns,
    ...(coverage ? { coverage } : {}),
  };
}

/** Structural check shared by the collector and the live source. */
export function isAaSection(v: unknown): v is AaFeedSection {
  if (typeof v !== "object" || v === null) return false;
  const s = v as Partial<AaSection> & Partial<AaAbsentSection>;
  if (s.status === "absent") return typeof s.reason === "string" && typeof s.generatedAt === "string";
  if (s.status !== "ok") return false;
  return (
    typeof s.generatedAt === "string" &&
    typeof s.batch === "string" &&
    Array.isArray(s.rankings) &&
    s.rankings.length > 0 &&
    s.rankings.every(
      (r) => typeof r === "object" && r !== null && typeof r.testRunKey === "string" && typeof r.family === "string",
    ) &&
    Array.isArray(s.configs) &&
    Array.isArray(s.testRuns)
  );
}

export interface AaSummary {
  /** Test_Runs snapshot dates present in the section, min..max. */
  snapshotVintage: { min: string; max: string } | null;
  /** testRunKey -> highest target intelligence at which it is usable. */
  usableUpTo: Map<string, number>;
}

export function summarizeAa(section: AaSection): AaSummary {
  const dates = section.testRuns
    .map((r) => r.snapshotDate)
    .filter((d): d is string => d !== null)
    .sort();
  const usableUpTo = new Map<string, number>();
  for (const t of section.thresholds) {
    if (!t.usable) continue;
    const cur = usableUpTo.get(t.testRunKey);
    if (cur === undefined || t.target > cur) usableUpTo.set(t.testRunKey, t.target);
  }
  return {
    snapshotVintage: dates.length ? { min: dates[0], max: dates[dates.length - 1] } : null,
    usableUpTo,
  };
}

/** The absent form, for the collector's degrade path. */
export function aaAbsent(reason: string, generatedAt: string): AaAbsentSection {
  return { status: "absent", reason, generatedAt };
}
