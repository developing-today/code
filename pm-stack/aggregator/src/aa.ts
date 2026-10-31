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
  AaCaiRow,
  AaCatalogEntry,
  AaConfig,
  AaCoverage,
  AaFeedSection,
  AaPoint,
  AaPrice,
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
export function modelPart(id: string): string {
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

// --- explorer points (charts + perspectives) -----------------------------------
// The Cost Explorer's client renders from these points: rankings rows
// joined with their Test_Runs timing rows, enriched with the derived
// values the reference design (the ChatGPT-built frontier charts in
// ~/workspace/ai-subscriptions/) plotted. All derivation is here, in
// tested code; the page's script only scales, sorts, and draws.

export const AA_EFFORT_ORDER: Record<string, number> = {
  "non-reasoning": 0,
  low: 1,
  medium: 2,
  high: 3,
  xhigh: 4,
  max: 5,
  reasoning: 6,
};

/** Sort effort labels along the ladder (unknown labels last, alpha). */
export function effortRank(effort: string | null): number {
  if (effort == null) return 99;
  return AA_EFFORT_ORDER[effort.toLowerCase()] ?? 50;
}

interface MetricRow {
  key: string;
  score: number | null;
  cost: number | null;
  time: number | null;
}

/** Non-dominated keys on one axis pair (both axes "lower wins" after
 * the caller's sign flips). Rows missing either axis never qualify. */
function paretoKeys(rows: MetricRow[], pick: (r: MetricRow) => [number, number] | null): Set<string> {
  const pts = rows
    .map((r) => ({ key: r.key, xy: pick(r) }))
    .filter((p): p is { key: string; xy: [number, number] } => p.xy !== null);
  const out = new Set<string>();
  for (const p of pts) {
    const dominated = pts.some(
      (q) => q !== p && q.xy[0] <= p.xy[0] && q.xy[1] <= p.xy[1] && (q.xy[0] < p.xy[0] || q.xy[1] < p.xy[1]),
    );
    if (!dominated) out.add(p.key);
  }
  return out;
}

/**
 * Join rankings with Test_Runs timings and compute the explorer's
 * derived fields. Score prefers the adjusted score; cost/time prefer
 * the rankings view's own columns and fall back to the timing row's —
 * the same numbers the server tables show, never an estimate.
 */
export function aaPoints(section: AaSection): AaPoint[] {
  const runs = new Map(section.testRuns.map((t) => [t.testRunKey, t]));
  const base: MetricRow[] = section.rankings.map((r) => {
    const t = runs.get(r.testRunKey);
    return {
      key: r.testRunKey,
      score: r.adjustedScore ?? r.rawScore,
      cost: r.costTask ?? t?.costPerTaskUsd ?? null,
      time: r.timeTaskSec ?? t?.timePerTaskSeconds ?? null,
    };
  });
  const frontierCost = paretoKeys(base, (r) => (r.cost !== null && r.score !== null ? [r.cost, -r.score] : null));
  const frontierTime = paretoKeys(base, (r) => (r.time !== null && r.score !== null ? [r.time, -r.score] : null));
  const frontierCostTime = paretoKeys(base, (r) => (r.cost !== null && r.time !== null ? [r.cost, r.time] : null));
  const label = (r: AaRanking): string => `${r.family}${r.effort ? ` ${r.effort}` : ""}`;
  return section.rankings.map((r, i) => {
    const b = base[i];
    const t = runs.get(r.testRunKey);
    const beatenBy =
      b.score !== null && b.cost !== null && b.time !== null
        ? section.rankings
            .filter((q, j) => {
              const o = base[j];
              return (
                j !== i &&
                o.score !== null &&
                o.cost !== null &&
                o.time !== null &&
                o.score >= (b.score as number) &&
                o.cost <= (b.cost as number) &&
                o.time <= (b.time as number) &&
                (o.score > (b.score as number) || o.cost < (b.cost as number) || o.time < (b.time as number))
              );
            })
            .map(label)
        : [];
    return {
      key: r.testRunKey,
      family: r.family,
      creator: r.creator,
      effort: r.effort,
      benchmark: r.benchmarkVersionKey,
      score: b.score,
      cost: b.cost,
      time: b.time,
      tps: r.speedTps ?? t?.outputSpeedTps ?? null,
      ttft: t?.ttftSeconds ?? null,
      pointsPerDollar: r.pointsPerDollar,
      pointsPerSecond: r.pointsPerSecond,
      scorePerDollar: b.score !== null && b.cost !== null && b.cost > 0 ? b.score / b.cost : null,
      scorePerSecond: b.score !== null && b.time !== null && b.time > 0 ? b.score / b.time : null,
      eligible: r.eligible,
      qualStatus: r.qualStatus,
      harness: null,
      tokensPerTask: null,
      components: null,
      frontierCost: frontierCost.has(b.key),
      frontierTime: frontierTime.has(b.key),
      frontierCostTime: frontierCostTime.has(b.key),
      beatenBy,
    };
  });
}

// --- Coding Agent Index points (the "Coding agents" perspective) ------------
// The ChatGPT explorers keep the Coding Agent Index as its own
// benchmark family and never convert between it and the Intelligence
// Index. Here it rides as a second point set in the same shape: score =
// CAI score, cost = published task cost, time = average agent wall
// time per task. Frontier flags and beaten-by are computed within the
// CAI population only, exactly like aaPoints does within its own.

/** Creator slugs in CAI observation keys -> display names. */
const CAI_CREATORS: Record<string, string> = {
  anthropic: "Anthropic",
  openai: "OpenAI",
  google: "Google",
  meta: "Meta",
  deepseek: "DeepSeek",
  "z-ai": "Z AI",
  zai: "Z AI",
  alibaba: "Alibaba",
  xai: "xAI",
  moonshotai: "Moonshot AI",
  xiaomi: "Xiaomi",
  mistral: "Mistral AI",
};

/** "GPT-6.1 Sol (xhigh)" + effort "xhigh" -> "GPT-6.1 Sol". The suffix
 * is stripped only when it restates the row's own effort label, so a
 * name that merely ends in parentheses survives untouched. */
export function caiFamily(modelName: string, effort: string | null): string {
  if (effort && effort.toLowerCase() !== "not stated") {
    const suffix = `(${effort})`;
    if (modelName.toLowerCase().endsWith(suffix.toLowerCase())) {
      return modelName.slice(0, modelName.length - suffix.length).trim();
    }
  }
  return modelName;
}

export function aaCaiPoints(rows: AaCaiRow[]): AaPoint[] {
  const base: MetricRow[] = rows.map((r) => ({
    key: `cai|${r.version ?? ""}|${r.modelName}`,
    score: r.score,
    cost: r.cost,
    time: r.wallSec,
  }));
  const frontierCost = paretoKeys(base, (r) => (r.cost !== null && r.score !== null ? [r.cost, -r.score] : null));
  const frontierTime = paretoKeys(base, (r) => (r.time !== null && r.score !== null ? [r.time, -r.score] : null));
  const frontierCostTime = paretoKeys(base, (r) => (r.cost !== null && r.time !== null ? [r.cost, r.time] : null));
  const label = (r: AaCaiRow): string => `${r.family}${r.effort ? ` ${r.effort}` : ""}`;
  return rows.map((r, i) => {
    const b = base[i];
    const beatenBy =
      b.score !== null && b.cost !== null && b.time !== null
        ? rows
            .filter((q, j) => {
              const o = base[j];
              return (
                j !== i &&
                o.score !== null &&
                o.cost !== null &&
                o.time !== null &&
                o.score >= (b.score as number) &&
                o.cost <= (b.cost as number) &&
                o.time <= (b.time as number) &&
                (o.score > (b.score as number) || o.cost < (b.cost as number) || o.time < (b.time as number))
              );
            })
            .map(label)
        : [];
    return {
      key: b.key,
      family: r.family,
      creator: r.creator,
      effort: r.effort,
      benchmark: `Coding Agent Index ${r.version ?? ""}`.trim(),
      score: b.score,
      cost: b.cost,
      time: b.time,
      tps: null,
      ttft: null,
      pointsPerDollar: null,
      pointsPerSecond: null,
      scorePerDollar: b.score !== null && b.cost !== null && b.cost > 0 ? b.score / b.cost : null,
      scorePerSecond: b.score !== null && b.time !== null && b.time > 0 ? b.score / b.time : null,
      // CAI rows are published results; the workbook's "eligible" flag
      // is a rankings-view concept and does not exist for them.
      eligible: true,
      qualStatus: null,
      harness: r.harness,
      tokensPerTask: r.tokensPerTask,
      components:
        r.deepSwe !== null || r.terminalBench !== null || r.sweAtlas !== null
          ? { deepSwe: r.deepSwe, terminalBench: r.terminalBench, sweAtlas: r.sweAtlas }
          : null,
      frontierCost: frontierCost.has(b.key),
      frontierTime: frontierTime.has(b.key),
      frontierCostTime: frontierCostTime.has(b.key),
      beatenBy,
    };
  });
}

// --- fleet catalog (Model Explorer tab) -----------------------------------------
// Plan facts per provider door. Source of record: the provider-quota
// registry (~/workspace/goals/keep-the-four-repos-healthy/hidden_files/
// provider-quota-registry.md) + ai-plans-2026-10-20.md, as of
// 2026-10-27 — the collector carries them because the page must render
// from the feed, never from prose. Update this table WITH the
// registry when a plan changes. A provider with no entry shows its
// config name and no multiplier ("not recorded"), never a guess.

export interface AaPlanFact {
  plan: string;
  /** List-rate dollars of usage per $1 paid, when the plan meters so. */
  multiplier: number | null;
  note: string;
}

export const AA_PLAN_FACTS: Record<string, AaPlanFact> = {
  "opencode-go.personal": {
    plan: "OpenCode Go — Personal",
    multiplier: 6,
    note: "$10/mo buys a $60/mo list-rate meter (6×); Flash promo cap $80 (≈8×). Cancelled — period ends 2026-11-02.",
  },
  "opencode-go.lab": {
    plan: "OpenCode Go — Lab",
    multiplier: 6,
    note: "$10/mo buys a $60/mo list-rate meter (6×); Flash promo cap $80 (≈8×). Cancelled — period ends 2026-11-06.",
  },
  "opencode-go": {
    plan: "OpenCode Go",
    multiplier: 6,
    note: "$10/mo buys a $60/mo list-rate meter (6×); Flash promo cap $80 (≈8×).",
  },
  commandcode: {
    plan: "Command Code GOAT",
    multiplier: 7,
    note: "$10/mo → $70 credits (7×); the entry Go tier is $1 → $10 credits (10×). Tier on the account is unconfirmed.",
  },
  "cline-pass": {
    plan: "ClinePass",
    multiplier: null,
    note: "$9.99/mo flat, open-weight models; quota ≈ 2–5× the same spend at list API rates.",
  },
  openai: {
    plan: "ChatGPT plan",
    multiplier: null,
    note: "OpenAI plan quota — the fleet's final reserve, spent last by design.",
  },
  "openai.alternate": {
    plan: "ChatGPT plan (student)",
    multiplier: null,
    note: "Second ChatGPT account door; resolves through the fork's built-in catalog.",
  },
  "claude-code": {
    plan: "Claude plan",
    multiplier: null,
    note: "Claude Code door. Plan is on the free tier for now — provider disabled in config; the proxy and catalog stay in place.",
  },
  deepseek: {
    plan: "DeepSeek direct — PAYG reserve",
    multiplier: 1,
    note: "api.deepseek.com at list rates (peak ≈2× for some hours, off-peak half). Reserve only — never a routine route.",
  },
  meta: {
    plan: "Meta API",
    multiplier: null,
    note: "Muse Spark / Wonder doors; pools use the -contributor ids wherever a Meta model is seated.",
  },
  "antigravity-ls": {
    plan: "Antigravity (Google AI plan)",
    multiplier: null,
    note: "Warm language-server door to the Antigravity quotas — the fleet's Claude-model supply (Sonnet/Opus plan pools).",
  },
  "antigravity-agy": {
    plan: "Antigravity (agy CLI)",
    multiplier: null,
    note: "Fallback door if the language server is unavailable — disabled in config.",
  },
  google: {
    plan: "Google (token path)",
    multiplier: null,
    note: "Gemini API-key path — disabled in config by policy; plan Gemini runs through Antigravity.",
  },
  opendesign: {
    plan: "OpenDesign (AMR)",
    multiplier: null,
    note: "Design-code bridge catalog; not enumerable upstream (API-key gated) — entries are config-curated.",
  },
  "openai-codex": {
    plan: "ChatGPT via codex proxy",
    multiplier: null,
    note: "Fronts the codex proxy (:18744) — sits in disabled_providers (door question filed as issue #1317).",
  },
  chatgpt: {
    plan: "ChatGPT via codex proxy",
    multiplier: null,
    note: "Fronts the codex proxy (:18744) — sits in disabled_providers (door question filed as issue #1317).",
  },
  "cloudflare-ai-gateway": {
    plan: "Cloudflare AI Gateway (PAYG)",
    multiplier: 1,
    note: "Metered gateway — disabled in config; never a plan route.",
  },
};

/**
 * Published OpenCode Go per-model monthly caps (USD), keyed by
 * normalized model name. Source: the official Go docs structure as
 * compiled in the AA value report (2026-10-20) — $80 DeepSeek V4.1
 * Flash; $60 most open models; $30 Qwen3.8 Flash; $15 the premium
 * tier; $12 the Contributor variants. The fleet's older workspaces
 * appear to meter one pooled bar instead (report reconciliation
 * caveat) — the page labels these "published" wherever it uses them.
 */
export const AA_GO_CAPS: Record<string, number> = {
  // Keys are normalizeModelName output ("deepseek-v4.1-flash" normalizes
  // to "deepseek v4 1 flash" — digit pairs stay separate tokens).
  "deepseek v4 1 flash": 80,
  "mimo v2 6 flash": 60,
  "glm 5 3 flash": 60,
  "glm 5 2": 60,
  "minimax m3": 60,
  "qwen3 8 flash": 30,
  "gpt 6 luna": 15,
  "gpt 5 6 luna": 15,
  "deepseek v4 pro": 15,
  "mimo v2 6 pro": 15,
  "glm 5 3": 15,
  "qwen3 8 max": 15,
  "kimi k3": 15,
  "grok 4 7": 15,
  "claude haiku 5 5": 15,
  "muse spark 1 3 contributor": 12,
  "gemini 4 argon contributor": 12,
};

interface RawModelEntry {
  name?: unknown;
  limit?: { context?: unknown; output?: unknown } | null;
  variants?: unknown;
}

/** Route id -> the leaf model ids it reaches (nested routes expanded). */
function routeLeafSets(cfg: { experimental?: { model_routes?: unknown } }): Map<string, Set<string>> {
  const routes = new Map<string, unknown[]>();
  const rawRoutes = cfg.experimental?.model_routes;
  if (typeof rawRoutes === "object" && rawRoutes !== null) {
    for (const [routeId, route] of Object.entries(rawRoutes as Record<string, unknown>)) {
      const targets = (route as { targets?: unknown } | null)?.targets;
      if (Array.isArray(targets)) routes.set(routeId, targets);
    }
  }
  const out = new Map<string, Set<string>>();
  const expand = (routeId: string, seen: Set<string>, acc: Set<string>): void => {
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
      if (bare.startsWith("opencode-route/")) expand(bare.slice("opencode-route/".length), seen, acc);
      else acc.add(bare);
    }
  };
  for (const routeId of routes.keys()) {
    const acc = new Set<string>();
    expand(routeId, new Set(), acc);
    out.set(routeId, acc);
  }
  return out;
}

/**
 * Build the Model Explorer catalog from the repo config + the AA
 * measured rows (for the per-model AA match). Returns null when the
 * config does not parse — the tab degrades to a stated absence, like
 * coverage. Name matching reuses the coverage normalizer/aliases, so
 * a miss means "not matched", never "not measured anywhere".
 */
/** Rates for one catalog model, grouped from the price event log. */
interface CatalogRate {
  bands: NonNullable<AaCatalogEntry["rate"]>["bands"];
  windowed: boolean;
}

/**
 * Group current prices by model: bands sorted by context floor, and a
 * lookup by full id plus one by bare model part (models.dev provider
 * slugs differ from fleet provider ids — "opencode-go.personal" vs
 * "opencode-go" — so the model part is the reliable join key; the
 * coverage normalizer arbitrates collisions by keeping the first).
 */
function catalogRates(prices: AaPrice[]): Map<string, CatalogRate> {
  const byModel = new Map<string, AaPrice[]>();
  for (const p of prices) {
    const list = byModel.get(p.modelName) ?? [];
    list.push(p);
    byModel.set(p.modelName, list);
  }
  const out = new Map<string, CatalogRate>();
  for (const [modelName, rows] of byModel) {
    const rate: CatalogRate = {
      bands: rows
        .slice()
        .sort((a, b) => (a.contextMinTokens ?? 0) - (b.contextMinTokens ?? 0))
        .map((p) => ({
          minTokens: p.contextMinTokens,
          maxTokens: p.contextMaxTokens,
          input: p.inputPer1M,
          output: p.outputPer1M,
          cacheRead: p.cacheReadPer1M,
          cacheWrite: p.cacheWritePer1M,
          window: p.windowKind,
        })),
      windowed: rows.some((p) => p.windowKind !== null && p.windowKind !== "flat"),
    };
    out.set(modelName, rate);
    const part = modelPart(modelName);
    const partKey = `part:${normalizeModelName(part)}`;
    if (!out.has(partKey)) out.set(partKey, rate);
  }
  return out;
}

/** The disabled_providers set of a parsed opencode config root. */
function disabledProvidersFromRoot(root: { disabled_providers?: unknown }): Set<string> {
  return new Set(
    Array.isArray(root.disabled_providers)
      ? root.disabled_providers.filter((d): d is string => typeof d === "string")
      : [],
  );
}

/**
 * The deployed opencode config's disabled_providers set, parsed with the
 * same JSONC handling as the AA catalog (#1114). An unparseable config
 * yields an empty set — callers fail soft and keep every pool, exactly as
 * the catalog does when the config cannot be read.
 */
export function disabledProvidersFromConfig(text: string): Set<string> {
  let cfg: unknown;
  try {
    cfg = JSON.parse(stripJsonComments(text));
  } catch {
    return new Set();
  }
  if (typeof cfg !== "object" || cfg === null) return new Set();
  return disabledProvidersFromRoot(cfg as { disabled_providers?: unknown });
}

export function aaCatalogFromConfig(
  text: string,
  measured: { configs: AaConfig[]; rankings: AaRanking[] },
  prices?: AaPrice[],
): AaCatalogEntry[] | null {
  let cfg: unknown;
  try {
    cfg = JSON.parse(stripJsonComments(text));
  } catch {
    return null;
  }
  if (typeof cfg !== "object" || cfg === null) return null;
  const root = cfg as {
    provider?: unknown;
    disabled_providers?: unknown;
    experimental?: { model_routes?: unknown };
  };
  if (typeof root.provider !== "object" || root.provider === null) return null;
  const disabled = disabledProvidersFromRoot(root);
  const leafSets = routeLeafSets(root);
  const rates = prices ? catalogRates(prices) : null;
  const routesByLeaf = new Map<string, string[]>();
  for (const [routeId, leaves] of leafSets) {
    for (const leaf of leaves) {
      const list = routesByLeaf.get(leaf) ?? [];
      list.push(routeId);
      routesByLeaf.set(leaf, list);
    }
  }
  // Best AA measurement per normalized model name: highest-scoring
  // config row; time from the family's fastest timed ranked row.
  const bestConfig = new Map<string, AaConfig>();
  for (const c of measured.configs) {
    const n = normalizeModelName(c.modelName);
    const cur = bestConfig.get(n);
    if (!cur || (c.intelligence ?? -1) > (cur.intelligence ?? -1)) bestConfig.set(n, c);
  }
  const bestRanking = new Map<string, AaRanking>();
  const fastestRanking = new Map<string, AaRanking>();
  for (const r of measured.rankings) {
    const n = normalizeModelName(r.family);
    const cur = bestRanking.get(n);
    if (!cur || (r.adjustedScore ?? r.rawScore ?? -1) > (cur.adjustedScore ?? cur.rawScore ?? -1)) {
      bestRanking.set(n, r);
    }
    if (r.timeTaskSec !== null) {
      const fast = fastestRanking.get(n);
      if (!fast || r.timeTaskSec < (fast.timeTaskSec as number)) fastestRanking.set(n, r);
    }
  }
  const entries: AaCatalogEntry[] = [];
  for (const [providerId, providerCfg] of Object.entries(root.provider as Record<string, unknown>)) {
    const models = (providerCfg as { models?: unknown } | null)?.models;
    if (typeof models !== "object" || models === null) continue;
    const providerName =
      typeof (providerCfg as { name?: unknown }).name === "string"
        ? ((providerCfg as { name: string }).name as string)
        : providerId;
    const fact = AA_PLAN_FACTS[providerId];
    for (const [modelId, rawEntry] of Object.entries(models as Record<string, unknown>)) {
      const entry = (typeof rawEntry === "object" && rawEntry !== null ? rawEntry : {}) as RawModelEntry;
      const id = `${providerId}/${modelId}`;
      const norm = normalizeModelName(modelPart(id));
      const conf = bestConfig.get(norm);
      const rank = bestRanking.get(norm);
      const variants =
        typeof entry.variants === "object" && entry.variants !== null
          ? Object.keys(entry.variants as Record<string, unknown>).sort(
              (a, b) => effortRank(a) - effortRank(b) || a.localeCompare(b),
            )
          : [];
      entries.push({
        id,
        provider: providerId,
        providerName,
        model: modelId,
        name: typeof entry.name === "string" && entry.name !== "" ? entry.name : modelId,
        contextLimit: num(entry.limit?.context),
        outputLimit: num(entry.limit?.output),
        variants,
        routes: (routesByLeaf.get(id) ?? []).sort(),
        disabled: disabled.has(providerId),
        plan: fact?.plan ?? null,
        multiplier: fact?.multiplier ?? null,
        planNote: fact?.note ?? null,
        goCapUsd: providerId.startsWith("opencode-go") ? (AA_GO_CAPS[norm] ?? null) : null,
        aaScore: conf?.intelligence ?? rank?.adjustedScore ?? rank?.rawScore ?? null,
        aaCostPerTask: conf?.costPerTaskUsd ?? rank?.costTask ?? null,
        aaTimePerTask: fastestRanking.get(norm)?.timeTaskSec ?? null,
        ...(rates ? { rate: rates.get(id) ?? rates.get(`part:${norm}`) ?? null } : {}),
      });
    }
  }
  return entries.sort((a, b) => byAlpha(a.id, b.id));
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
  cai?: unknown;
  prices?: unknown;
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

function caiFromRaw(v: unknown): AaCaiRow | null {
  if (typeof v !== "object" || v === null) return null;
  const r = v as Record<string, unknown>;
  if (typeof r.modelName !== "string" || r.modelName === "") return null;
  const effort = str(r.effort);
  const slug = str(r.creatorSlug);
  return {
    modelName: r.modelName,
    family: caiFamily(r.modelName, effort),
    creator: slug ? (CAI_CREATORS[slug.toLowerCase()] ?? slug) : "",
    effort,
    version: str(r.version),
    score: num(r.score),
    cost: num(r.cost),
    wallSec: num(r.wallSec),
    tokensPerTask: num(r.tokensPerTask),
    harness: str(r.harness),
    deepSwe: num(r.deepSwe),
    terminalBench: num(r.terminalBench),
    sweAtlas: num(r.sweAtlas),
    snapshotDate: str(r.snapshotDate),
    sourceUrl: str(r.sourceUrl),
  };
}

function priceFromRaw(v: unknown): AaPrice | null {
  if (typeof v !== "object" || v === null) return null;
  const p = v as Record<string, unknown>;
  if (typeof p.modelName !== "string" || p.modelName === "") return null;
  return {
    modelName: p.modelName,
    inputPer1M: num(p.inputPer1M),
    outputPer1M: num(p.outputPer1M),
    cacheReadPer1M: num(p.cacheReadPer1M),
    cacheWritePer1M: num(p.cacheWritePer1M),
    contextMinTokens: num(p.contextMinTokens),
    contextMaxTokens: num(p.contextMaxTokens),
    windowKind: str(p.windowKind),
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
  const cai = Array.isArray(raw.cai) ? raw.cai.map(caiFromRaw).filter((r): r is AaCaiRow => r !== null) : undefined;
  const prices = Array.isArray(raw.prices)
    ? raw.prices.map(priceFromRaw).filter((p): p is AaPrice => p !== null)
    : undefined;
  const measuredNames = [...new Set([...rankings.map((r) => r.family), ...configs.map((c) => c.modelName)])];
  let coverage: AaCoverage | undefined;
  let catalog: AaCatalogEntry[] | undefined;
  if (opencodeConfigText) {
    const inputs = parseOpencodeCoverageInputs(opencodeConfigText);
    if (inputs) coverage = computeAaCoverage(measuredNames, inputs);
    catalog = aaCatalogFromConfig(opencodeConfigText, { configs, rankings }, prices) ?? undefined;
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
    ...(catalog ? { catalog } : {}),
    ...(cai && cai.length ? { cai } : {}),
    ...(prices && prices.length ? { prices } : {}),
  };
}

/** Structural check shared by the collector and the live source. */
export function isAaSection(v: unknown): v is AaFeedSection {
  if (typeof v !== "object" || v === null) return false;
  const s = v as Partial<AaSection> & Partial<AaAbsentSection>;
  if (s.status === "absent")
    return typeof s.reason === "string" && typeof s.generatedAt === "string" && Number.isFinite(Date.parse(s.generatedAt));
  if (s.status !== "ok") return false;
  return (
    typeof s.generatedAt === "string" &&
    Number.isFinite(Date.parse(s.generatedAt)) &&
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
