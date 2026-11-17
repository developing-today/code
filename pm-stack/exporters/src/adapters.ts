// Adapter loader for the quota watchdog (#6032, design #6031).
//
// An adapter is a per-provider JSON file in the adapters directory. It
// carries everything provider-specific — origins allowlist, CDP port,
// producer document path, cookie names (names only, never values),
// login hints, class-map overrides, thresholds — so the watchdog core
// (watchdog.ts + watchdog-state.ts) contains zero provider names.
//
// Adapter files are data, not code: the loader validates them
// structurally and reports every defect instead of throwing, because a
// broken adapter must degrade to "that source is unobserved" (a
// watchdog-visible error), never to a crashed tick that leaves every
// other source unwatched.

import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import type { FailureClass, Rung } from "./watchdog-state.ts";
import { FAILURE_CLASSES, RUNGS } from "./watchdog-state.ts";

export interface WatchdogAdapter {
  /** Source name — the key used in the state file and history. */
  name: string;
  enabled: boolean;
  /** Producer document (`~/atlas/<provider>-export/latest.json` shape).
   *  Supports $HOME and ${ENV_VAR} expansion. Optional when
   *  heartbeatKey is set. */
  latestPath?: string;
  /** Key into the gatherer heartbeat's perProvider map, used when the
   *  producer has no document of its own (its read lives inside the
   *  gatherer) and as corroborating evidence otherwise. */
  heartbeatKey?: string;
  /** Chrome DevTools Protocol port of the supervised browser. */
  cdpPort?: number;
  /** systemd user unit that supervises the browser, for rung 2's
   *  bounded restart. */
  browserUnit?: string;
  /** Browser automation may touch only these origins. */
  origins: string[];
  /** Cookie names whose PRESENCE rung 2/3 check. Values are never
   *  read into the watchdog process. */
  cookieNames: string[];
  /** Where rung 3 starts the provider's own SSO login flow. */
  loginUrl?: string;
  /** Substrings (URL or page title) that mean a human-only challenge:
   *  rung 3 stops dead when one appears. */
  challengeMarkers: string[];
  /** Producer status string -> failure class overrides. */
  classMap: Record<string, FailureClass>;
  /** Producer status string -> rung overrides (via decide's classMap
   *  is class-keyed; this maps a status onto a class first). */
  rungMap: Partial<Record<FailureClass, Rung>>;
  /** A producer document older than this is a stale read. */
  staleAfterMs: number;
  /** Rate-limit cooldown written to disk on rung 0. */
  cooldownMs: number;
  /** Repo that owns the adapter — rung 4 files its incident there. */
  issueRepo: string;
}

const DEFAULT_STALE_AFTER_MS = 30 * 60_000; // two missed 15-min ticks
const DEFAULT_COOLDOWN_MS = 15 * 60_000;

function expandPath(p: string, env: Record<string, string | undefined>): string {
  return p
    .replace(/\$\{([A-Za-z_][A-Za-z0-9_]*)\}/g, (_, k: string) => env[k] ?? "")
    .replace(/\$HOME\b/g, env.HOME ?? "")
    .replace(/^~\//, env.HOME ? `${env.HOME}/` : "~/");
}

function strList(v: unknown): string[] {
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
}

function isFailureClass(v: unknown): v is FailureClass {
  return typeof v === "string" && (FAILURE_CLASSES as readonly string[]).includes(v);
}

function isRung(v: unknown): v is Rung {
  return typeof v === "number" && (RUNGS as readonly number[]).includes(v);
}

/**
 * Validate one parsed adapter document. Returns the normalized adapter
 * or the list of defects (never throws — see the header).
 */
export function parseAdapter(
  raw: unknown,
  env: Record<string, string | undefined> = process.env,
): { adapter: WatchdogAdapter | null; errors: string[] } {
  const errors: string[] = [];
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) {
    return { adapter: null, errors: ["adapter is not a JSON object"] };
  }
  const r = raw as Record<string, unknown>;
  const name = typeof r.name === "string" && r.name.length > 0 ? r.name : null;
  if (!name) errors.push("missing or empty `name`");
  const latestPath = typeof r.latestPath === "string" ? expandPath(r.latestPath, env) : undefined;
  const heartbeatKey = typeof r.heartbeatKey === "string" ? r.heartbeatKey : undefined;
  if (!latestPath && !heartbeatKey) {
    errors.push("adapter declares neither `latestPath` nor `heartbeatKey` — nothing to observe");
  }
  const issueRepo = typeof r.issueRepo === "string" && /^[^/\s]+\/[^/\s]+$/.test(r.issueRepo) ? r.issueRepo : null;
  if (!issueRepo) errors.push("missing or malformed `issueRepo` (owner/repo)");
  const cdpPort = typeof r.cdpPort === "number" && r.cdpPort > 0 && r.cdpPort < 65536 ? r.cdpPort : undefined;
  if (r.cdpPort !== undefined && cdpPort === undefined) errors.push("`cdpPort` is not a valid port");
  const origins = strList(r.origins);
  for (const o of origins) {
    if (!/^https?:\/\/[^/\s]+/.test(o)) errors.push(`origin is not an http(s) origin: ${o}`);
  }
  const classMap: Record<string, FailureClass> = {};
  if (typeof r.classMap === "object" && r.classMap !== null) {
    for (const [k, v] of Object.entries(r.classMap as Record<string, unknown>)) {
      if (isFailureClass(v)) classMap[k] = v;
      else errors.push(`classMap[${JSON.stringify(k)}] is not a failure class: ${JSON.stringify(v)}`);
    }
  }
  const rungMap: Partial<Record<FailureClass, Rung>> = {};
  if (typeof r.rungMap === "object" && r.rungMap !== null) {
    for (const [k, v] of Object.entries(r.rungMap as Record<string, unknown>)) {
      if (isFailureClass(k) && isRung(v)) rungMap[k] = v;
      else errors.push(`rungMap[${JSON.stringify(k)}] is not class->rung: ${JSON.stringify(v)}`);
    }
  }
  const posMs = (v: unknown, dflt: number) =>
    typeof v === "number" && Number.isFinite(v) && v > 0 ? v : dflt;
  if (errors.length > 0 || !name || !issueRepo) return { adapter: null, errors };
  return {
    errors,
    adapter: {
      name,
      enabled: r.enabled !== false,
      latestPath,
      heartbeatKey,
      cdpPort,
      browserUnit: typeof r.browserUnit === "string" ? r.browserUnit : undefined,
      origins,
      cookieNames: strList(r.cookieNames),
      loginUrl: typeof r.loginUrl === "string" ? r.loginUrl : undefined,
      challengeMarkers: strList(r.challengeMarkers),
      classMap,
      rungMap,
      staleAfterMs: posMs(r.staleAfterMs, DEFAULT_STALE_AFTER_MS),
      cooldownMs: posMs(r.cooldownMs, DEFAULT_COOLDOWN_MS),
      issueRepo,
    },
  };
}

/**
 * Load every `*.json` adapter in a directory, in filename order.
 * A missing directory is an empty result with one error entry — the
 * caller decides whether that is fatal (it is, for a oneshot whose
 * whole job is watching the configured sources).
 */
export async function loadAdapters(
  dir: string,
  env: Record<string, string | undefined> = process.env,
): Promise<{ adapters: WatchdogAdapter[]; errors: string[] }> {
  const adapters: WatchdogAdapter[] = [];
  const errors: string[] = [];
  let files: string[];
  try {
    files = (await readdir(dir)).filter((f) => f.endsWith(".json")).sort();
  } catch (e) {
    return { adapters, errors: [`adapters dir unreadable: ${dir}: ${e instanceof Error ? e.message : String(e)}`] };
  }
  const seen = new Set<string>();
  for (const file of files) {
    const path = join(dir, file);
    let parsed: unknown;
    try {
      parsed = JSON.parse(await readFile(path, "utf8"));
    } catch (e) {
      errors.push(`${file}: invalid JSON: ${e instanceof Error ? e.message : String(e)}`);
      continue;
    }
    const { adapter, errors: perFile } = parseAdapter(parsed, env);
    for (const err of perFile) errors.push(`${file}: ${err}`);
    if (!adapter) continue;
    if (seen.has(adapter.name)) {
      errors.push(`${file}: duplicate source name ${JSON.stringify(adapter.name)} — skipped`);
      continue;
    }
    seen.add(adapter.name);
    if (adapter.enabled) adapters.push(adapter);
  }
  return { adapters, errors };
}
