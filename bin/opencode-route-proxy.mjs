#!/usr/bin/env node
// OpenCode Route Proxy: an OpenAI-compatible ROUTING door for the fleet's
// route pools, so OpenAI-chat-completions clients (OpenClaw's
// models.providers in particular) get their actual model by calling
// opencode-route/* — above all opencode-route/workhorse. #2229 mechanism
// 3, rebuilt routing-only per #2518.
//
// Semantic, stated plainly (Drew's directive, 2026-10-29, #2518):
// OpenCode does no work here besides routing — no session, no agent
// loop, no tools. The caller keeps its own loop and context; this door
// resolves the requested route against the same fleet config the
// OpenCode server uses (.opencode/opencode.jsonc: model_routes +
// provider blocks), picks the actual pool member (ordered / round-robin
// / weighted selection, variant remaps, disabled providers skipped,
// quota demotion from the route_attempt ledger, plus the known-exhausted seating gate #5295), expands provider
// routes (experimental.provider_routes, #3256) into their member
// providers, and makes ONE plain
// chat-completions call to that member's endpoint with the member's
// credential from the OpenCode credential store (opencode.db). The
// caller's messages are forwarded as-is — never flattened into a
// transcript, never wrapped in a session. Failover walks the resolved
// candidate order; every attempt is written to route_attempt with
// session_id NULL, so quota accounting and demotion see door traffic
// exactly like session traffic.
//
// The previous implementation of this file ran one OpenCode session per
// request (an agent behind a completions facade). That shape is gone;
// dispatching whole tasks to OpenCode remains available through the
// ACP / sessions_spawn paths (#2256, #2257), which is where OpenCode
// doing the work is the point.
//
// Env:
//   OPENCODE_ROUTE_PROXY_PORT        listen port (default 18750)
//   OPENCODE_ROUTE_PROXY_HOST        listen host (default 127.0.0.1)
//   OPENCODE_ROUTE_PROXY_TIMEOUT_MS  per-request cap (default 1200000)
//   OPENCODE_ROUTE_PROXY_LEAF_TIMEOUT_MS  first-response cap per leaf
//                                         (default 3000 ms; below the client
//                                         no-output window)
//   OPENCODE_ROUTE_PROXY_CONFIG      opencode.jsonc path (default:
//                                    ~/.local/share/opencode/opencode.jsonc,
//                                    else ../.opencode/opencode.jsonc
//                                    relative to this file)
//   OPENCODE_ROUTE_PROXY_RUNTIME_CONFIG
//                                    runtime overlay jsonc whose provider
//                                    catalogs are deep-merged over the
//                                    config's (default:
//                                    ~/.local/state/opencode/opencode-runtime.jsonc;
//                                    absent = no overlay) (#4544)
//   OPENCODE_ROUTE_PROXY_DB          opencode.db path (default:
//                                    ~/.local/share/opencode/opencode.db)
//   OPENCODE_ROUTE_PROXY_NOW_MS      pin the clock (epoch ms) used for
//                                    route window gates — tests and
//                                    incident replay; unset = wall clock
import { createServer } from "node:http";
import { readFileSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { DatabaseSync } from "node:sqlite";
import { randomBytes } from "node:crypto";
import { homePath } from "./home-dir.mjs";
import { parseProxyMillis, parseProxyPort } from "./proxy-port.mjs";
import { createRequestBodyCollector } from "./request-body.mjs";

const PORT = parseProxyPort("OPENCODE_ROUTE_PROXY_PORT", 18750);
const HOST = process.env.OPENCODE_ROUTE_PROXY_HOST || "127.0.0.1";
const TIMEOUT_MS = parseProxyMillis("OPENCODE_ROUTE_PROXY_TIMEOUT_MS", 1_200_000);
const LEAF_TIMEOUT_MS = parseProxyMillis(
  "OPENCODE_ROUTE_PROXY_LEAF_TIMEOUT_MS",
  3_000,
);
const HERE = dirname(fileURLToPath(import.meta.url));
const CONFIG_PATH =
  process.env.OPENCODE_ROUTE_PROXY_CONFIG ||
  homePath(process.env, "opencode-route-proxy (default OPENCODE_ROUTE_PROXY_CONFIG)", ".local/share/opencode/opencode.jsonc");
const CONFIG_FALLBACK = join(HERE, "..", ".opencode", "opencode.jsonc");
const RUNTIME_CONFIG_PATH =
  process.env.OPENCODE_ROUTE_PROXY_RUNTIME_CONFIG ||
  homePath(process.env, "opencode-route-proxy (default OPENCODE_ROUTE_PROXY_RUNTIME_CONFIG)", ".local/state/opencode/opencode-runtime.jsonc");
const DB_PATH =
  process.env.OPENCODE_ROUTE_PROXY_DB ||
  homePath(process.env, "opencode-route-proxy (default OPENCODE_ROUTE_PROXY_DB)", ".local/share/opencode/opencode.db");

// Providers built into OpenCode itself (no block in opencode.jsonc):
// instanceOf targets resolve here when the config has no block either.
// These entries are DEFAULTS, merged UNDER any config block: a block
// that sets its own baseURL/npm/models always wins, but an empty-shell
// block (models only, or instanceOf only) no longer dead-ends a leaf
// the way it did for openai / openai.alternate in #3256 — OpenCode
// tolerates those blocks via its built-in catalog, so the proxy must
// resolve the same endpoints. Endpoint sources (fork
// developing-today/opencode, packages/ai): openai —
// src/protocols/openai-chat.ts DEFAULT_BASE_URL (the models.dev
// catalog entry for @ai-sdk/openai carries no api URL; the SDK's own
// default applies); deepseek / groq / openrouter / meta — the
// baseURL constant in src/providers/<id>.ts of the same package.
// openai.alternate needs no entry of its own: it is instanceOf
// openai, and the chain below inherits this default through the
// base block.
const BUILTIN_PROVIDERS = {
  "opencode-go": { options: { baseURL: "https://opencode.ai/zen/go/v1" } },
  openai: { options: { baseURL: "https://api.openai.com/v1" } },
  deepseek: { options: { baseURL: "https://api.deepseek.com/v1" } },
  groq: { options: { baseURL: "https://api.groq.com/openai/v1" } },
  openrouter: { options: { baseURL: "https://openrouter.ai/api/v1" } },
  meta: { options: { baseURL: "https://api.meta.ai/v1" } },
};

// Cloudflare-fronted endpoints (opencode.ai among them) reject default
// library user agents (error 1010); a plain browser UA passes and is
// harmless to every other leaf, local bridges included.
const LEAF_UA =
  "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

const ADVERTISED_MODELS = [
  "workhorse",
  "speed",
  "specialist",
  "summit",
  "scout",
  "workhorse-weighted",
];

const MAX_ATTEMPTS = 6;
const DEMOTE_WINDOW_MS = 4 * 60 * 60 * 1000;
const DEMOTE_THRESHOLD = 3;
const STREAM_FIRST_CONTENT_WINDOW_MS = 250;
const STREAM_PREFACE_MAX_BYTES = 64 * 1024;
// Hard ceiling for a single in-progress data event in the relay: the
// preface window (above) rejects non-content prefaces, but one valid
// content event may be larger than the window, so its enforcement is
// deferred until the event completes (see relayStream). This ceiling
// still bounds a never-ending event.
const STREAM_EVENT_HARD_MAX_BYTES = 16 * 1024 * 1024;

// ---------------------------------------------------------------- config

// String-aware JSONC comment stripper (URLs in string values survive).
const stripJsonc = (src) => {
  let out = "";
  let i = 0;
  let inStr = false;
  let esc = false;
  while (i < src.length) {
    const c = src[i];
    const n = src[i + 1];
    if (inStr) {
      out += c;
      if (esc) esc = false;
      else if (c === "\\") esc = true;
      else if (c === '"') inStr = false;
      i++;
    } else if (c === '"') {
      inStr = true;
      out += c;
      i++;
    } else if (c === "/" && n === "/") {
      while (i < src.length && src[i] !== "\n") i++;
    } else if (c === "/" && n === "*") {
      i += 2;
      while (i < src.length && !(src[i] === "*" && src[i + 1] === "/")) i++;
      i += 2;
    } else {
      out += c;
      i++;
    }
  }
  return out;
};

const parseJsonc = (text) => {
  const stripped = stripJsonc(text);
  try {
    return JSON.parse(stripped);
  } catch {
    // Trailing commas are legal in the fleet's jsonc style.
    return JSON.parse(stripped.replace(/,\s*([}\]])/g, "$1"));
  }
};

const isPlainObj = (v) =>
  v !== null && typeof v === "object" && !Array.isArray(v);
// Deep-merge plain objects (overlay wins on scalars): mirrors the fork
// server's config layering for the runtime overlay (#4544), so the proxy
// resolves the same effective provider catalog the server does.
const deepMergePlain = (base, over) => {
  if (!isPlainObj(base) || !isPlainObj(over)) return over;
  const out = { ...base };
  for (const k of Object.keys(over)) out[k] = deepMergePlain(base[k], over[k]);
  return out;
};

let cfgCache = null;
const loadConfig = () => {
  let path = CONFIG_PATH;
  let st = null;
  try {
    st = statSync(path);
  } catch {
    path = CONFIG_FALLBACK;
    st = statSync(path);
  }
  let rtSt = null;
  try {
    rtSt = statSync(RUNTIME_CONFIG_PATH);
  } catch {
    rtSt = null;
  }
  const rtMtimeMs = rtSt ? rtSt.mtimeMs : null;
  if (
    cfgCache &&
    cfgCache.path === path &&
    cfgCache.mtimeMs === st.mtimeMs &&
    cfgCache.rtMtimeMs === rtMtimeMs
  )
    return cfgCache;
  const raw = parseJsonc(readFileSync(path, "utf8"));
  if (rtSt) {
    try {
      const rt = parseJsonc(readFileSync(RUNTIME_CONFIG_PATH, "utf8"));
      if (rt && typeof rt === "object" && rt.provider)
        raw.provider = deepMergePlain(raw.provider || {}, rt.provider);
    } catch {
      // Overlay unreadable: serve the tracked default alone rather than fail.
    }
  }
  const windows =
    raw.experimental?.model_route_windows || raw.model_route_windows || {};
  warnInvalidWindows(windows);
  cfgCache = {
    path,
    mtimeMs: st.mtimeMs,
    rtMtimeMs,
    providers: raw.provider || {},
    providerRoutes:
      raw.experimental?.provider_routes || raw.provider_routes || {},
    routes: raw.experimental?.model_routes || raw.model_routes || {},
    windows,
    disabled: new Set(raw.disabled_providers || []),
  };
  return cfgCache;
};

const expandEnv = (s) =>
  typeof s === "string"
    ? s.replace(
        /\{env:([A-Za-z0-9_]+)(?::-([^}]*))?\}/g,
        (_, v, d) => process.env[v] ?? d ?? "",
      )
    : s;

// One provider block with the built-in defaults merged under it:
// config fields (options included) win key by key; the built-in
// table only fills what the block omits.
const mergedDef = (cfg, id) => {
  const raw = Object.hasOwn(cfg.providers, id) ? cfg.providers[id] : undefined;
  const builtin = Object.hasOwn(BUILTIN_PROVIDERS, id) ? BUILTIN_PROVIDERS[id] : undefined;
  if (!raw) return builtin || null;
  if (!builtin) return raw;
  return {
    ...builtin,
    ...raw,
    options: { ...(builtin.options || {}), ...(raw.options || {}) },
    models: raw.models || builtin.models,
  };
};

// Effective provider definition, following instanceOf chains into the
// config's base block or the built-in table.
const providerDef = (cfg, id) => {
  let def = mergedDef(cfg, id);
  const seen = new Set([id]);
  while (def && def.instanceOf && !seen.has(def.instanceOf)) {
    seen.add(def.instanceOf);
    const base = mergedDef(cfg, def.instanceOf);
    if (!base) break;
    def = {
      ...base,
      ...def,
      options: { ...(base.options || {}), ...(def.options || {}) },
      models: def.models || base.models,
    };
  }
  return def || null;
};

const providerBaseURL = (cfg, id) => {
  const def = providerDef(cfg, id);
  const url = expandEnv(def?.options?.baseURL || "");
  return url ? url.replace(/\/$/, "") : null;
};

// ---------------------------------------------------------------- store

const openDb = () => {
  const db = new DatabaseSync(DB_PATH);
  db.exec("PRAGMA busy_timeout = 5000");
  return db;
};

const credCache = new Map(); // providerId -> { key, at }
const credentialKey = (providerId) => {
  const hit = credCache.get(providerId);
  if (hit && Date.now() - hit.at < 60_000) return hit.key;
  let key = null;
  let db = null;
  try {
    db = openDb();
    const row = db
      .prepare(
        "SELECT value FROM credential WHERE integration_id = ? AND active = 1 ORDER BY id DESC LIMIT 1",
      )
      .get(providerId);
    if (row?.value) {
      try {
        const parsed = JSON.parse(row.value);
        // API credentials store { key }; OAuth credentials (the plan
        // providers, e.g. openai) store { access } instead — that
        // access token is the member's bearer credential (#3256).
        if (typeof parsed?.key === "string" && parsed.key) key = parsed.key;
        else if (typeof parsed?.access === "string" && parsed.access)
          key = parsed.access;
      } catch {
        key = String(row.value);
      }
    }
  } catch {
    key = null;
  } finally {
    try {
      db?.close();
    } catch {}
  }
  credCache.set(providerId, { key, at: Date.now() });
  return key;
};

// The credential a leaf would call with: a static apiKey in its
// provider config, else its active credential-store row (#3228).
const leafCredential = (cfg, providerId) => {
  const def = providerDef(cfg, providerId);
  const staticKey = def?.options?.apiKey ? expandEnv(def.options.apiKey) : null;
  return staticKey || credentialKey(providerId);
};
const hasCredential = (cfg, providerId) => Boolean(leafCredential(cfg, providerId));

// (provider,model) pairs demoted by the doctrine: >=3 qualifying
// failures (quota detail recorded, a RateLimit tag, or an
// Authentication tag) inside the last 4 hours, counting only the
// current failure run: a success on the pair clears the run and
// lifts the demotion. The Authentication arm is #8048: a leg whose
// credential is dead fails 100% with provider.auth and never
// carried quota detail, so the quota-only filter never demoted it
// and it stayed first-choice while killing sessions.
const demotedPairs = () => {
  let db = null;
  try {
    db = openDb();
    const rows = db
      .prepare(
        `SELECT provider_id, model_id FROM route_attempt
         WHERE time_started > ? AND outcome != 'success'
           AND (quota IS NOT NULL OR error_tag IN ('RateLimit', 'Authentication'))
           AND time_started > COALESCE(
             (SELECT MAX(s.time_started) FROM route_attempt s
              WHERE s.provider_id = route_attempt.provider_id
                AND s.model_id = route_attempt.model_id
                AND s.outcome = 'success'), 0)
         GROUP BY provider_id, model_id HAVING COUNT(*) >= ?`,
      )
      .all(Date.now() - DEMOTE_WINDOW_MS, DEMOTE_THRESHOLD);
    return new Set(rows.map((r) => `${r.provider_id}/${r.model_id}`));
  } catch {
    return new Set();
  } finally {
    try {
      db?.close();
    } catch {}
  }
};

// The reset instant a quota detail promises, if it carries one
// (#5295): a Retry-After-derived relative wait (the shape
// classifyHttp records for 429s), or an absolute reset recorded under
// any of the equivalent field names. A quota record with no reset
// (e.g. a bare paymentRequired) cannot gate on a stated reset: how
// long it lasts is unknown, and demotion remains its only handling
// (the bounded inferred wall of #13693 is the separate, run-based
// handling for such records). When one record carries several reset
// shapes (#13693: Retry-After plus an x-ratelimit-reset header), the
// LATEST instant governs — the pair is walled until every promised
// reset has passed.
const quotaResetMs = (timeEnded, quotaText) => {
  let q;
  try {
    q = JSON.parse(quotaText);
  } catch {
    return null;
  }
  if (!q || typeof q !== "object") return null;
  let best = null;
  const consider = (ms) => {
    if (Number.isFinite(ms) && ms > 0)
      best = best === null ? ms : Math.max(best, ms);
  };
  // A relative retry delay runs from when the quota response was
  // RECEIVED (the attempt's time_ended), not from when the request
  // started: anchoring to the start would let a slow 429 response
  // consume its own backoff.
  if (Number.isFinite(q.retryAfterMs) && q.retryAfterMs > 0)
    consider(timeEnded + q.retryAfterMs);
  for (const k of ["resetsAtMs", "resetAtMs", "retryAtMs"]) consider(q[k]);
  for (const k of ["resetsAt", "resetAt", "retryAt"])
    consider(Date.parse(q[k]));
  return best;
};

// (provider,model) pairs whose binding quota window is KNOWN
// exhausted (#5295, #4370 acceptance (c)): the pair's current
// failure run carries a reset still in the future, and no success
// has been recorded for the pair since the run began. Unlike
// demotion — reactive (>=3 failures in 4h) and reorder-only — this
// is a seating-time exclusion known BEFORE the next failure: a
// monthly-walled leg at a known 100% must not be a first-choice
// candidate while any live candidate exists. The ledger is the
// only source (no new network dependency in the hot path); the
// lookback bounds the scan while covering monthly resets.
//
// #13693 RC-2: the gate judges the pair's current failure RUN —
// every quota-bearing failure since its last success, completion-
// time ordered as before — and takes the LATEST (maximum) reset
// any failure in the run carried. Providers vary whether a given
// 429 carries a reset; judging only the latest failure let a
// reset-less 429 dissolve a standing wall mid-flight.
const EXHAUSTED_LOOKBACK_MS = 40 * 24 * 60 * 60 * 1000;
const knownExhaustedPairs = (now) => {
  let db = null;
  try {
    db = openDb();
    const since = now - EXHAUSTED_LOOKBACK_MS;
    // All ordering is by COMPLETION time (time_ended): with
    // overlapping requests, start-time ordering can mistake a
    // success that finished before a quota response arrived for a
    // recovery after it.
    const quotaRows = db
      .prepare(
        `SELECT provider_id, model_id, time_ended, quota FROM route_attempt
         WHERE quota IS NOT NULL AND outcome != 'success' AND time_ended > ?
         ORDER BY time_ended ASC`,
      )
      .all(since);
    const successRows = db
      .prepare(
        `SELECT provider_id, model_id, MAX(time_ended) AS latest
         FROM route_attempt
         WHERE outcome = 'success' AND time_ended > ?
         GROUP BY provider_id, model_id`,
      )
      .all(since);
    const latestSuccess = new Map(
      successRows.map((r) => [`${r.provider_id}/${r.model_id}`, r.latest]),
    );
    // The current run per pair: failures completed after the pair's
    // latest success. The gate reset is the maximum reset the run
    // carries, so a later reset-less failure cannot dissolve it; a
    // success after every failure leaves the run empty.
    const runReset = new Map();
    for (const r of quotaRows) {
      const key = `${r.provider_id}/${r.model_id}`;
      const successAt = latestSuccess.get(key);
      if (successAt !== undefined && successAt > r.time_ended) continue;
      const reset = quotaResetMs(r.time_ended, r.quota);
      if (reset === null) continue;
      const prev = runReset.get(key);
      if (prev === undefined || reset > prev) runReset.set(key, reset);
    }
    const out = new Map();
    for (const [key, reset] of runReset)
      if (reset > now) out.set(key, reset);
    return out;
  } catch {
    return new Map();
  } finally {
    try {
      db?.close();
    } catch {}
  }
};

// A bounded INFERRED wall for reset-less quota walls (#13693 RC-1):
// a pair whose current failure run carries no parseable reset at
// all still stops being walked indefinitely once the run shows
// repeated plan-limit failure. A pair is inferred-walled when its
// current run — quota-bearing failures (quota IS NOT NULL, outcome
// != 'success', which includes 402 paymentRequired records) since
// its last success — contains >=3 failures with some three of them
// inside the demotion window (4h). The wall is bounded: it lasts
// INFERRED_WALL_BASE_MS from the run's LATEST failure, doubling per
// prior inferred wall for the pair inside the lookback, capped at
// INFERRED_WALL_MAX_MS. Expiry makes the next seating the probe,
// and a success after the latest failure clears the inference (the
// run it belonged to is then no longer the current run).
//
// Escalation rule (deterministic, from the ledger alone): process
// the pair's quota failures in completion order, grouped into
// runs. A run ends at a success, or — once it has itself qualified
// (>=3 failures with some three inside 4h, earning a window W) —
// at the first later failure arriving more than W after the run's
// previous failure (that wall had expired; the failure is a fresh
// probe and starts a new run). A run's escalation count is the
// number of earlier runs in the lookback that qualified; its
// window is BASE * 2^count, capped at MAX. Doctrine: repeated
// plan-limit failures only — never one failure, never unbounded.
const INFERRED_WALL_BASE_MS = 15 * 60 * 1000;
const INFERRED_WALL_MAX_MS = 2 * 60 * 60 * 1000;
const inferredWalledPairs = (now) => {
  let db = null;
  try {
    db = openDb();
    const since = now - EXHAUSTED_LOOKBACK_MS;
    const rows = db
      .prepare(
        `SELECT provider_id, model_id, time_ended, outcome FROM route_attempt
         WHERE time_ended > ? AND (outcome = 'success' OR quota IS NOT NULL)
         ORDER BY provider_id, model_id, time_ended ASC`,
      )
      .all(since);
    const byPair = new Map();
    for (const r of rows) {
      const key = `${r.provider_id}/${r.model_id}`;
      if (!byPair.has(key)) byPair.set(key, []);
      byPair.get(key).push(r);
    }
    const qualifies = (run) => {
      for (let i = 0; i + 2 < run.length; i++)
        if (run[i + 2] - run[i] <= DEMOTE_WINDOW_MS) return true;
      return false;
    };
    const out = new Map();
    for (const [key, events] of byPair) {
      let priorQualifying = 0;
      let run = []; // time_ended of the current run's quota failures
      let earnedMs = null; // window the current run earned, once qualified
      const closeRun = () => {
        if (qualifies(run)) priorQualifying++;
        run = [];
        earnedMs = null;
      };
      for (const e of events) {
        if (e.outcome === "success") {
          closeRun();
          continue;
        }
        if (
          run.length &&
          earnedMs !== null &&
          e.time_ended > run[run.length - 1] + earnedMs
        )
          closeRun();
        run.push(e.time_ended);
        if (earnedMs === null && qualifies(run))
          earnedMs = Math.min(
            INFERRED_WALL_BASE_MS * 2 ** priorQualifying,
            INFERRED_WALL_MAX_MS,
          );
      }
      if (earnedMs !== null) {
        const until = run[run.length - 1] + earnedMs;
        if (until > now) out.set(key, until);
      }
    }
    return out;
  } catch {
    return new Map();
  } finally {
    try {
      db?.close();
    } catch {}
  }
};

// Whether any route_attempt on one of the candidate pairs STARTED
// within the last 60s (#13693 RC-4 pacing evidence: a walk already
// paid failed attempts against the wall moments ago). Started-at,
// strictly inside the window: an attempt that began exactly 60s
// ago is the attempt that established the wall in the #5295
// fixtures, not a repeat walk into a known wall.
const recentAttemptOnCandidates = (candidates, now) => {
  let db = null;
  try {
    db = openDb();
    const clauses = candidates
      .map(() => "(provider_id = ? AND model_id = ?)")
      .join(" OR ");
    const params = [];
    for (const c of candidates) params.push(c.providerId, c.modelKey);
    const row = db
      .prepare(
        `SELECT 1 AS x FROM route_attempt WHERE time_started > ? AND (${clauses}) LIMIT 1`,
      )
      .get(now - 60_000, ...params);
    return Boolean(row);
  } catch {
    return false;
  } finally {
    try {
      db?.close();
    } catch {}
  }
};

// The bench a 401/403 earns (#12228): an auth failure marks the door
// DEAD — the _antigravity ToS 403 shape — so it must stop taking new
// sessions at once, not after demotion's three-failure threshold, and
// not only after a quota wall names a reset. There is no stated reset
// for an auth failure, so the bench is bounded: once it expires the
// next seating is the probe, and any recorded success re-admits the
// pair immediately (the probe succeeded). Same ledger-only source and
// completion-time ordering as the quota gate above.
const AUTH_BENCH_MS = 15 * 60 * 1000;
const authBenchedPairs = (now) => {
  let db = null;
  try {
    db = openDb();
    const since = now - EXHAUSTED_LOOKBACK_MS;
    const authRows = db
      .prepare(
        `SELECT provider_id, model_id, time_ended FROM route_attempt
         WHERE outcome != 'success' AND error_tag = 'Authentication'
           AND error_status IN (401, 403) AND time_ended > ?
           AND time_ended = (
             SELECT MAX(time_ended) FROM route_attempt r2
             WHERE r2.provider_id = route_attempt.provider_id
               AND r2.model_id = route_attempt.model_id
               AND r2.outcome != 'success'
               AND r2.error_tag = 'Authentication'
               AND r2.error_status IN (401, 403)
               AND r2.time_ended > ?
           )`,
      )
      .all(since, since);
    const successRows = db
      .prepare(
        `SELECT provider_id, model_id, MAX(time_ended) AS latest
         FROM route_attempt
         WHERE outcome = 'success' AND time_ended > ?
         GROUP BY provider_id, model_id`,
      )
      .all(since);
    const latestSuccess = new Map(
      successRows.map((r) => [`${r.provider_id}/${r.model_id}`, r.latest]),
    );
    const out = new Map();
    for (const r of authRows) {
      const until = r.time_ended + AUTH_BENCH_MS;
      if (until <= now) continue;
      const key = `${r.provider_id}/${r.model_id}`;
      const successAt = latestSuccess.get(key);
      if (successAt !== undefined && successAt > r.time_ended) continue;
      out.set(key, until);
    }
    return out;
  } catch {
    return new Map();
  } finally {
    try {
      db?.close();
    } catch {}
  }
};

// Exclude known-exhausted, auth-benched and inferred-walled
// candidates (#5295, #12228, #13693), following the credential-less
// exclusion pattern (#3228) — but fail-safe: if the gate would
// empty the candidate list, the demoted order is kept as it was,
// because a benched leg as last resort beats no route at all (the
// one exception is the reset-carrying pacing answer, RC-4 below).
// Gate decisions are recorded on stderr, the proxy's diagnostics
// stream.
const applyKnownExhaustedGate = (cfg, candidates) => {
  if (!candidates.length) return candidates;
  const now = nowMs();
  const exhausted = knownExhaustedPairs(now);
  const authBenched = authBenchedPairs(now);
  const inferred = inferredWalledPairs(now);
  if (!exhausted.size && !authBenched.size && !inferred.size)
    return candidates;
  const key = (c) => `${c.providerId}/${c.modelKey}`;
  // The inferred wall never stacks on a reset-carrying gate: a pair
  // with a stated reset is governed by that reset alone (#13693).
  const inferredFor = (c) =>
    inferred.has(key(c)) && !exhausted.has(key(c));
  const kept = candidates.filter(
    (c) =>
      !exhausted.has(key(c)) && !authBenched.has(key(c)) && !inferredFor(c),
  );
  // The fail-safe is evaluated against CREDENTIALED survivors: the
  // credential-less filter (#3228) runs after this gate, so a kept
  // list whose only members have no credential would still empty the
  // route — and the benched leaf must then stay as the last resort
  // instead of the request failing with "no credentialed members".
  if (!kept.some((c) => hasCredential(cfg, c.providerId))) {
    // Fail-safe pacing (#13693 RC-4): when EVERY excluded candidate
    // is excluded by a reset-carrying known-exhausted gate (not
    // inferred, not auth), the earliest such reset is >60s away,
    // and a walk already paid attempts on a candidate pair within
    // the last 60s, walking again only burns more failed attempts
    // against a known wall — answer 503 with Retry-After until the
    // earliest reset instead. In every other bypass case the
    // walk-anyway behaviour below is preserved exactly.
    const excluded = candidates.filter((c) => !kept.includes(c));
    if (excluded.length && excluded.every((c) => exhausted.has(key(c)))) {
      const earliestReset = Math.min(
        ...excluded.map((c) => exhausted.get(key(c))),
      );
      if (
        earliestReset - now > 60_000 &&
        recentAttemptOnCandidates(candidates, now)
      ) {
        console.error(
          `opencode-route-proxy: wall pacing — every candidate is ` +
            `known-exhausted until ${new Date(earliestReset).toISOString()}; ` +
            `answering 503 with Retry-After instead of walking (#13693)`,
        );
        const err = new LeafError(
          `all route candidates are quota-walled until ${new Date(earliestReset).toISOString()} (#13693)`,
          { tag: "RateLimit", status: 503 },
        );
        err.retryAfterMs = earliestReset - now;
        err.wallPaced = true;
        throw err;
      }
    }
    const reasons = [
      ...(exhausted.size ? ["known-exhausted"] : []),
      ...(authBenched.size ? ["auth bench"] : []),
      ...(inferred.size ? ["inferred wall"] : []),
    ].join(" + ");
    console.error(
      `opencode-route-proxy: ${reasons} gate bypassed — no credentialed ` +
        "candidate survives it; keeping the demoted order (#5295, #12228)",
    );
    return candidates;
  }
  for (const c of candidates) {
    if (exhausted.has(key(c)))
      console.error(
        `opencode-route-proxy: known-exhausted gate excluded ${key(c)} ` +
          `until ${new Date(exhausted.get(key(c))).toISOString()} (#5295)`,
      );
    else if (inferredFor(c))
      console.error(
        `opencode-route-proxy: inferred wall excluded ${key(c)} ` +
          `until ${new Date(inferred.get(key(c))).toISOString()} (#13693)`,
      );
    else if (authBenched.has(key(c)))
      console.error(
        `opencode-route-proxy: auth bench excluded ${key(c)} until ` +
          `${new Date(authBenched.get(key(c))).toISOString()} (#12228)`,
      );
  }
  return kept;
};

const recordAttempt = (a) => {
  let db = null;
  try {
    db = openDb();
    db.prepare(
      `INSERT INTO route_attempt (
         time_started, time_ended, session_id, assistant_message_id,
         route_id, provider_id, model_id, variant, outcome,
         error_tag, error_code, error_status, error_message, retryable,
         output_started, failed_over_to, first_token_ms, response_ms,
         tokens_per_second, tokens_input, tokens_output, tokens_reasoning,
         tokens_cache_read, tokens_cache_write, quota, cost_usd
       ) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)`,
    ).run(
      a.started,
      a.ended,
      null,
      null,
      a.routeId,
      a.providerId,
      a.modelId,
      a.variant ?? null,
      a.outcome,
      a.errorTag ?? null,
      a.errorCode ?? null,
      a.errorStatus ?? null,
      a.errorMessage ?? null,
      a.retryable == null ? null : a.retryable ? 1 : 0,
      a.outputStarted ? 1 : 0,
      a.failedOverTo ?? null,
      a.firstTokenMs ?? null,
      a.responseMs ?? null,
      a.tokensPerSecond ?? null,
      a.tokensInput ?? null,
      a.tokensOutput ?? null,
      a.tokensReasoning ?? null,
      a.tokensCacheRead ?? null,
      a.tokensCacheWrite ?? null,
      a.quota ?? null,
      a.costUsd ?? null,
    );
  } catch {
    // The ledger must never fail a completion.
  } finally {
    try {
      db?.close();
    } catch {}
  }
};

// ---------------------------------------------------------------- routes

const parseTargetRef = (ref) => {
  let variant = null;
  const h = ref.lastIndexOf("#");
  if (h > 0) {
    variant = ref.slice(h + 1);
    ref = ref.slice(0, h);
  }
  const s = ref.indexOf("/");
  return { providerId: ref.slice(0, s), modelKey: ref.slice(s + 1), variant };
};

// Route target window gates (#3225): a target may carry
// `window: "<name>"` or `window: { name, invert: true }`, naming a
// window in experimental.model_route_windows
// ({ daysOfWeek: ["mon", ...], start: "HH:MM", end: "HH:MM",
// timezone }). The gate survives normalization — dropping it here was
// the bug — and resolveRoute expands a gated target only while its
// window is active (or, with invert, only while it is NOT). A gate
// naming an unknown window never matches: fail closed, so a config
// typo surfaces as a loud no-members route instead of silently
// ungating the target.
const normalizeWindowGate = (w) => {
  if (typeof w === "string" && w) return { name: w, invert: false };
  if (w && typeof w.name === "string" && w.name)
    return { name: w.name, invert: w.invert === true };
  return null;
};

const normalizeTarget = (t) => {
  if (typeof t === "string")
    return { ...parseTargetRef(t), variantMap: null, windowGate: null };
  if (t && typeof t.model === "string")
    return {
      ...parseTargetRef(t.model),
      variantMap: t.variants || null,
      windowGate: normalizeWindowGate(t.window),
    };
  return null;
};

// Clock for window evaluation. OPENCODE_ROUTE_PROXY_NOW_MS pins
// "now" (epoch ms) for tests and incident replay; unset = wall clock.
const nowMs = () => {
  const pinned = Number(process.env.OPENCODE_ROUTE_PROXY_NOW_MS);
  return Number.isFinite(pinned) && pinned > 0 ? pinned : Date.now();
};

const parseHHMM = (s) => {
  const m = /^([0-9]{1,2}):([0-9]{2})$/.exec(typeof s === "string" ? s.trim() : "");
  if (!m) return null;
  const hour = Number(m[1]);
  const minute = Number(m[2]);
  if (hour > 23 || minute > 59) return null;
  return hour * 60 + minute;
};

const WEEKDAY_NAMES = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

const validTimeZone = (tz) => {
  try {
    new Intl.DateTimeFormat("en-US", { timeZone: tz });
    return true;
  } catch {
    return false;
  }
};

// Why a window entry can never match, or null when it is well formed.
// start/end must be HH:MM in range (#4259); a present `timezone` must
// be a real Intl time zone and a present `daysOfWeek` an array of
// weekday names (#11176). windowActive fails closed (never active) for
// every problem named here, so a typo'd window surfaces as a loud
// no-match rather than flipping a route gate to always-on/always-off/
// end-less, or (a string daysOfWeek) silently widening to every day.
const windowProblems = (win) => {
  const problems = [];
  if (parseHHMM(win?.start) === null || parseHHMM(win?.end) === null)
    problems.push(
      `start=${JSON.stringify(win?.start)} end=${JSON.stringify(win?.end)} ` +
        `(expected HH:MM with hour 00-23, minute 00-59)`,
    );
  if (win?.timezone != null && (typeof win.timezone !== "string" || !validTimeZone(win.timezone)))
    problems.push(`timezone=${JSON.stringify(win.timezone)} (expected an IANA time zone such as "America/Chicago")`);
  if (
    win?.daysOfWeek != null &&
    (!Array.isArray(win.daysOfWeek) ||
      !win.daysOfWeek.every(
        (d) => typeof d === "string" && WEEKDAY_NAMES.includes(d.trim().slice(0, 3).toLowerCase()),
      ))
  )
    problems.push(`daysOfWeek=${JSON.stringify(win.daysOfWeek)} (expected an array of weekday names such as ["mon", "fri"])`);
  return problems.length ? problems : null;
};

// At config load, name every model_route_windows entry that can never
// match, with the offending values, instead of silently evaluating it.
const warnInvalidWindows = (windows) => {
  if (!windows || typeof windows !== "object") return;
  for (const [name, win] of Object.entries(windows)) {
    const problems = windowProblems(win);
    if (problems) {
      console.error(
        `opencode-route-proxy: invalid model_route_windows["${name}"]: ` +
          `${problems.join("; ")}; window will never match`,
      );
    }
  }
};

// Is window `win` active at `now` (epoch ms)? Day and wall time are
// read in the window's own timezone; start inclusive, end exclusive;
// an end at or before start wraps past midnight. A wrapping window's
// after-midnight leg belongs to the day the window opened, so its
// daysOfWeek check uses the previous day (#3874): a Fri 22:00-02:00
// window is active Sat 01:00 and inactive Fri 01:00.
const windowActive = (win, now) => {
  if (!win || typeof win !== "object") return false;
  if (windowProblems(win)) return false;
  const start = parseHHMM(win.start);
  const end = parseHHMM(win.end);
  let parts;
  try {
    parts = new Intl.DateTimeFormat("en-US", {
      timeZone: win.timezone || "UTC",
      weekday: "short",
      hour: "2-digit",
      minute: "2-digit",
      hourCycle: "h23",
    }).formatToParts(new Date(now));
  } catch {
    return false;
  }
  const get = (type) => parts.find((part) => part.type === type)?.value;
  const day = String(get("weekday") || "").slice(0, 3).toLowerCase();
  const days =
    Array.isArray(win.daysOfWeek) && win.daysOfWeek.length
      ? win.daysOfWeek.map((d) => d.trim().slice(0, 3).toLowerCase())
      : null;
  const dayListed = (d) => !days || days.includes(d);
  const mins = Number(get("hour")) * 60 + Number(get("minute"));
  if (start <= end)
    return dayListed(day) && mins >= start && mins < end;
  if (mins >= start) return dayListed(day);
  if (mins < end) {
    if (!days) return true;
    const idx = WEEKDAY_NAMES.indexOf(day);
    return idx >= 0 && days.includes(WEEKDAY_NAMES[(idx + 6) % 7]);
  }
  return false;
};

const targetEligible = (cfg, t, now) => {
  if (!t.windowGate) return true;
  const active = windowActive(cfg.windows?.[t.windowGate.name], now);
  return t.windowGate.invert ? !active : active;
};

const rotation = new Map(); // routeId -> counter
const rotate = (arr, routeId) => {
  if (arr.length < 2) return arr;
  const n = (rotation.get(routeId) || 0) % arr.length;
  rotation.set(routeId, n + 1);
  return arr.slice(n).concat(arr.slice(0, n));
};

// Weighted ordering: repeated weighted sampling without replacement.
// Weight keys are the target refs as written in the route (no #variant).
const weightedOrder = (targets, weights) => {
  const pool = targets.map((t) => ({
    t,
    w: Math.max(0, Number(weights[t.refBase] ?? 1) || 0),
  }));
  const out = [];
  while (pool.length) {
    const total = pool.reduce((s, p) => s + p.w, 0);
    let idx = 0;
    if (total > 0) {
      let r = Math.random() * total;
      idx = pool.findIndex((p) => (r -= p.w) < 0);
      if (idx < 0) idx = pool.length - 1;
    }
    out.push(pool.splice(idx, 1)[0].t);
  }
  return out;
};

// Provider routes (#3256; fork grammar from developing-today/opencode
// PR #1134, experimental.provider_routes): a named group of concrete
// providers — { providers: [...], selection, weights } — that a route
// targets as an ordinary leaf, `<group>/<model>`. Resolution expands
// the group, per model, into its members' concrete leaves for that
// model, honoring the group's own selection: ordered = member order,
// round-robin = rotation per group+model, weighted = weighted draw
// with weights keyed by member provider id (re-keyed to
// `<member>/<model>` for weightedOrder). The consuming route's
// selection/weights apply to the group target as written, exactly as
// for a nested route: this expansion runs inline at the target's
// position in the route's ordered target list.
// A concrete provider always wins a reference: an id that names a
// config provider block (or a built-in) is never treated as a group.
const providerRouteDef = (cfg, id) => {
  if (Object.hasOwn(cfg.providers, id) || Object.hasOwn(BUILTIN_PROVIDERS, id)) return null;
  const g = cfg.providerRoutes?.[id];
  if (!g || !Array.isArray(g.providers)) return null;
  return g;
};

// Expand one provider-route target into `out` (same leaf shape as
// resolveRoute pushes). Member rules, mirroring the fork's v1:
// duplicates drop; a member naming another provider route or
// opencode-route drops (no nesting); an unknown member contributes
// no leaves; disabled members are skipped; a member whose catalog
// (its effective models map, instanceOf chains included) does not
// offer the target model is skipped for this dispatch.
const expandProviderRoute = (cfg, t, variant, out) => {
  const group = providerRouteDef(cfg, t.providerId);
  if (!group || cfg.disabled.has(t.providerId)) return;
  const members = [...new Set(group.providers)].filter(
    (m) =>
      typeof m === "string" &&
      m !== "opencode-route" &&
      (Object.hasOwn(cfg.providers, m) || Object.hasOwn(BUILTIN_PROVIDERS, m)) &&
      !cfg.disabled.has(m),
  );
  let leaves = [];
  for (const m of members) {
    const models = providerDef(cfg, m)?.models;
    if (models && !Object.hasOwn(models, t.modelKey)) continue;
    leaves.push({
      providerId: m,
      modelKey: t.modelKey,
      variant,
      refBase: `${m}/${t.modelKey}`,
    });
  }
  if (group.selection === "round-robin")
    leaves = rotate(leaves, `provider-route:${t.providerId}/${t.modelKey}`);
  else if (group.selection === "weighted") {
    const weights = {};
    for (const [member, w] of Object.entries(group.weights || {}))
      weights[`${member}/${t.modelKey}`] = w;
    leaves = weightedOrder(leaves, weights);
  }
  for (const leaf of leaves)
    out.push({
      providerId: leaf.providerId,
      modelKey: leaf.modelKey,
      variant: leaf.variant,
    });
};

// Resolve a route to its ordered leaf candidates:
// [{ providerId, modelKey, variant }]. Nested routes recurse; provider
// routes expand to their member providers; disabled providers are
// skipped; cycles are cut.
const resolveRoute = (cfg, routeId, variantIn = null) => {
  const now = nowMs();
  const out = [];
  const expand = (rid, vIn, seen) => {
    if (seen.has(rid)) return;
    seen.add(rid);
    const route = cfg.routes[rid];
    if (!route) return;
    let targets = (route.targets || [])
      .map(normalizeTarget)
      .filter(Boolean)
      .filter((t) => targetEligible(cfg, t, now))
      .map((t) => ({ ...t, refBase: `${t.providerId}/${t.modelKey}` }));
    if (route.selection === "round-robin") targets = rotate(targets, rid);
    else if (route.selection === "weighted")
      targets = weightedOrder(targets, route.weights || {});
    for (const t of targets) {
      let v = vIn;
      if (t.variantMap && v && t.variantMap[v]) v = t.variantMap[v];
      if (t.variant) v = t.variant;
      if (t.providerId === "opencode-route") expand(t.modelKey, v, seen);
      else if (providerRouteDef(cfg, t.providerId))
        expandProviderRoute(cfg, t, v, out);
      else if (!cfg.disabled.has(t.providerId))
        out.push({ providerId: t.providerId, modelKey: t.modelKey, variant: v });
    }
  };
  expand(routeId, variantIn, new Set());
  return out;
};

// Demoted leaves sink to the back of the candidate list (they stay
// reachable as a last resort; failover walks the same order).
const applyDemotion = (candidates, demoted) => {
  if (!demoted.size) return candidates;
  const key = (c) => `${c.providerId}/${c.modelKey}`;
  return [
    ...candidates.filter((c) => !demoted.has(key(c))),
    ...candidates.filter((c) => demoted.has(key(c))),
  ];
};

// ---------------------------------------------------------------- leaves

class LeafError extends Error {
  constructor(message, { tag, status = null, code = null, quota = null, retryable = 1 }) {
    super(message);
    this.tag = tag;
    this.status = status;
    this.code = code;
    this.quota = quota;
    this.retryable = retryable;
  }
}

const classifyHttp = (status, headers, bodyText) => {
  let quota = null;
  if (status === 429) {
    // Receipt time for this response: relative delays recorded here
    // are anchored by the ledger to the attempt's time_ended via
    // quotaResetMs, so an HTTP-date reset is stored as the DELTA
    // from receipt, keeping that anchor exact (#13693).
    const receiptMs = Date.now();
    let quotaObj = null;
    const retryAfter = headers.get("retry-after");
    if (retryAfter) {
      const secs = Number(retryAfter);
      let retryAfterMs = null;
      if (Number.isFinite(secs)) retryAfterMs = secs * 1000;
      else {
        // An HTTP-date Retry-After (RFC 9110 allows both shapes):
        // parse it as the instant it names (#13693 RC-1).
        const dateMs = Date.parse(retryAfter);
        if (Number.isFinite(dateMs)) retryAfterMs = dateMs - receiptMs;
      }
      quotaObj = {
        retryAfterMs,
        headers: { "retry-after": retryAfter },
      };
    }
    // Rate-limit reset headers (#13693 RC-1): >1e12 is epoch ms,
    // >1e9 is epoch s, anything smaller is seconds-from-now.
    const resetHeader =
      headers.get("x-ratelimit-reset") ?? headers.get("ratelimit-reset");
    if (resetHeader) {
      const v = Number(resetHeader);
      if (Number.isFinite(v) && v > 0) {
        const resetsAtMs =
          v > 1e12 ? v : v > 1e9 ? v * 1000 : receiptMs + v * 1000;
        quotaObj = { ...(quotaObj ?? {}), resetsAtMs };
      }
    }
    if (quotaObj) quota = JSON.stringify(quotaObj);
  }
  if (status === 429 && quota === null) {
    // A quota wall that names its reset in the BODY with no
    // Retry-After header still walls the leaf (#12228): the _muse
    // wall reads "Your usage window resets at 2026-11-16T00:00:00Z"
    // with no header, and dropping it kept new sessions routing
    // into the wall until demotion (reorder-only) kicked in.
    const resetMs = bodyResetMs(bodyText);
    if (resetMs !== null)
      quota = JSON.stringify({ resetsAt: new Date(resetMs).toISOString() });
  }
  if (status === 429)
    return new LeafError(`leaf HTTP 429: ${bodyText.slice(0, 200)}`, {
      tag: "RateLimit",
      status,
      quota,
    });
  if (status === 401 || status === 403)
    return new LeafError(`leaf HTTP ${status}: ${bodyText.slice(0, 200)}`, {
      tag: "Authentication",
      status,
      retryable: 0,
    });
  if (status === 402)
    return new LeafError(`leaf HTTP 402: ${bodyText.slice(0, 200)}`, {
      tag: "RateLimit",
      status,
      quota: quota ?? JSON.stringify({ paymentRequired: true }),
    });
  return new LeafError(`leaf HTTP ${status}: ${bodyText.slice(0, 200)}`, {
    tag: "ProviderInternal",
    status,
  });
};

// Duration units for the reset shapes real providers send
// (#13693): prose ("in 20 minutes"), keyed duration strings
// ("30s", Google-style retryDelay) and small-number seconds.
const DURATION_UNIT_MS = {
  ms: 1,
  s: 1000,
  sec: 1000,
  secs: 1000,
  second: 1000,
  seconds: 1000,
  m: 60_000,
  min: 60_000,
  mins: 60_000,
  minute: 60_000,
  minutes: 60_000,
  h: 3_600_000,
  hr: 3_600_000,
  hrs: 3_600_000,
  hour: 3_600_000,
  hours: 3_600_000,
};
const durationMs = (numText, unit) => {
  const mult = DURATION_UNIT_MS[String(unit).toLowerCase()];
  const n = Number(numText);
  return mult && Number.isFinite(n) && n > 0 ? n * mult : null;
};
const DURATION_RE =
  /(\d+(?:\.\d+)?)\s*(ms|seconds?|secs?|s|minutes?|mins?|m|hours?|hrs?|h)/.source;

// The reset instant a 429 BODY promises, if it names one (#12228).
// Shapes: explicit JSON fields ({"resetsAt": "<iso>"}, epoch seconds
// or ms under a reset/retry/renew key) or free prose with a reset
// word next to an ISO instant ("window resets at ...Z"). #13693
// adds the duration shapes: keyed pure-duration strings and
// small-number seconds under after/delay keys, and duration prose
// ("try again in 20 minutes") — durations run from RECEIPT (this
// call), which the caller records as an absolute instant. Only
// future instants are returned: a past instant cannot wall, and
// recording one would just add noise to the ledger. Returns epoch
// ms or null.
const bodyResetMs = (bodyText) => {
  if (!bodyText) return null;
  const receiptMs = Date.now();
  const futureMs = (ms) =>
    Number.isFinite(ms) && ms > Date.now() ? ms : null;
  try {
    const parsed = JSON.parse(bodyText);
    const findKeyed = (v, depth) => {
      if (depth > 4 || v === null || typeof v !== "object") return null;
      const entries = Array.isArray(v)
        ? v.map((item, i) => [i, item])
        : Object.entries(v);
      for (const [k, item] of entries) {
        if (
          (typeof item === "string" || typeof item === "number") &&
          /reset|retry|renew/i.test(String(k))
        ) {
          let ms;
          if (typeof item === "number") {
            // A small number under an after/delay key is a DURATION
            // in seconds ("retry_after": 30), not an epoch: epochs
            // that small predate the ledger entirely (#13693).
            ms =
              item > 0 && item < 1e9 && /after|delay/i.test(String(k))
                ? receiptMs + item * 1000
                : item > 1e12
                  ? item
                  : item > 1e9 && item < 1e11
                    ? item * 1000
                    : NaN;
          } else {
            // A pure duration string ("30s", "1.5s", "2m", "1h" —
            // Google's retryDelay shape) is a duration from
            // receipt, not an instant (#13693).
            const dur = new RegExp(`^${DURATION_RE}$`, "i").exec(
              item.trim(),
            );
                        const d = dur ? durationMs(dur[1], dur[2]) : null;
            ms = dur ? (d === null ? NaN : receiptMs + d) : Date.parse(item);
          }
          const ok = futureMs(ms);
          if (ok !== null) return ok;
        } else if (item !== null && typeof item === "object") {
          const inner = findKeyed(item, depth + 1);
          if (inner !== null) return inner;
        }
      }
      return null;
    };
    const keyed = findKeyed(parsed, 0);
    if (keyed !== null) return keyed;
  } catch {}
  // Duration prose: a verb + "in" + number + unit ("try again in
  // 20s / 2 minutes / 1.5h") is a duration from receipt (#13693).
  const durProse = bodyText.match(
    /(?:try again|retry|re-?try|attempt again|check back|wait)\b[^.\d]{0,40}?\bin\s+(\d+(?:\.\d+)?)\s*(ms|seconds?|secs?|s|minutes?|mins?|m|hours?|hrs?|h)\b/i,
  );
  if (durProse) {
    const d = durationMs(durProse[1], durProse[2]);
    const ok = d === null ? null : futureMs(receiptMs + d);
    if (ok !== null) return ok;
  }
  const prose = bodyText.match(
    /(?:reset|resum|renew|reinstat|available|back)[^\d]{0,20}(\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}(?::\d{2}(?:\.\d{1,3})?)?(?:Z|[+-]\d{2}:?\d{2})?)/i,
  );
  if (prose) {
    const ok = futureMs(Date.parse(prose[1]));
    if (ok !== null) return ok;
  }
  return null;
};

const classifyStreamError = (providerError, headers) => {
  const details =
    typeof providerError === "object" && providerError !== null
      ? providerError
      : { message: String(providerError) };
  const message =
    typeof details.message === "string"
      ? details.message
      : JSON.stringify(details);
  const status = Number(details.status ?? details.status_code);
  const type = typeof details.type === "string" ? details.type : "";
  const code = typeof details.code === "string" ? details.code : null;
  const quotaFailure =
    status === 429 || /rate.?limit|quota|insufficient_quota/i.test(
      `${type} ${code || ""} ${message}`,
    );
  if (quotaFailure) {
    const err = classifyHttp(429, headers, message);
    err.status = 429;
    err.code = code;
    let quota = {};
    try {
      quota = JSON.parse(err.quota || "{}");
    } catch {}
    err.quota = JSON.stringify({
      ...quota,
      status: 429,
      ...(type ? { type } : {}),
      ...(code ? { code } : {}),
    });
    return err;
  }
  return new LeafError(message, {
    tag: "ProviderInternal",
    status: Number.isFinite(status) ? status : null,
    code,
  });
};

const streamPayloadHasContent = (payload) => {
  if (!Array.isArray(payload?.choices)) return false;
  return payload.choices.some((choice) => {
    if (typeof choice?.text === "string" && choice.text.length > 0)
      return true;
    const delta = choice?.delta;
    if (!delta || typeof delta !== "object" || Array.isArray(delta))
      return false;
    return Object.entries(delta).some(([key, value]) => {
      if (key === "role" || value == null || value === "") return false;
      return !Array.isArray(value) || value.length > 0;
    });
  });
};

const newSseState = (started) => ({
  decoder: new TextDecoder(),
  tail: "",
  contentStarted: false,
  started,
  firstTokenMs: null,
  usage: null,
  done: false,
  streamError: null,
});

const inspectSseEvent = (event, state, headers) => {
  const data = event
    .split(/\r?\n/)
    .filter((line) => line.startsWith("data:"))
    .map((line) => line.slice(5).replace(/^ /, ""))
    .join("\n")
    .trim();
  if (!data) return;
  if (data === "[DONE]") {
    state.done = true;
    return;
  }
  let payload;
  try {
    payload = JSON.parse(data);
  } catch {
    // Preserve compatibility with providers that send non-JSON data events.
    if (!state.contentStarted) {
      state.contentStarted = true;
      state.firstTokenMs ??= Date.now() - state.started;
    }
    return;
  }
  if (payload?.usage) state.usage = payload.usage;
  if (payload?.error) {
    if (!state.streamError)
      state.streamError = {
        error: classifyStreamError(payload.error, headers),
        beforeContent: !state.contentStarted,
      };
    return;
  }
  if (streamPayloadHasContent(payload) && !state.contentStarted) {
    state.contentStarted = true;
    state.firstTokenMs = Date.now() - state.started;
  }
};

const inspectSseBytes = (value, state, headers, flush = false) => {
  const decoded = flush
    ? state.decoder.decode()
    : state.decoder.decode(value, { stream: true });
  state.tail += decoded;
  // Parse every complete event BEFORE bounding the buffer: trimming
  // first would cut the head off an event larger than the preface
  // window (a large tool-call argument delta is a single event),
  // removing its data: prefix so it is never recognised as content
  // (Codex P2 on PR #5558).
  const separator = /\r?\n\r?\n/g;
  let match;
  let start = 0;
  while ((match = separator.exec(state.tail))) {
    inspectSseEvent(state.tail.slice(start, match.index), state, headers);
    start = separator.lastIndex;
  }
  state.tail = state.tail.slice(start);
  if (state.tail.length > STREAM_PREFACE_MAX_BYTES) {
    // The tail is now a single partial event. Distinguish a large
    // content event from a non-content preface: everything from the
    // event's first data: line on is its payload and stays whole,
    // however large — the preface limits are enforced on raw bytes
    // by the callers (prepare/relay byte counts), not by mangling
    // this buffer. With no data: line yet, nothing here can be
    // recognised as content, so keep only the newest window.
    const dataStart = state.tail.search(/^data:/m);
    state.tail =
      dataStart === -1
        ? state.tail.slice(-STREAM_PREFACE_MAX_BYTES)
        : state.tail.slice(dataStart);
  }
  if (flush && state.tail.trim()) {
    inspectSseEvent(state.tail, state, headers);
    state.tail = "";
  }
};

const streamTransportError = (err) =>
  new LeafError(`leaf stream transport error: ${err?.message || err}`, {
    tag: "Transport",
    code: err?.cause?.code || err?.code || null,
  });

const readWithinWindow = async (pendingRead, ms, signal) => {
  if (signal.aborted) return { kind: "aborted", error: signal.reason };
  let timer;
  let abortHandler;
  const timeout = new Promise((resolve) => {
    timer = setTimeout(() => resolve({ kind: "timeout" }), ms);
  });
  const aborted = new Promise((resolve) => {
    abortHandler = () => resolve({ kind: "aborted", error: signal.reason });
    signal.addEventListener("abort", abortHandler, { once: true });
  });
  try {
    return await Promise.race([pendingRead, timeout, aborted]);
  } finally {
    clearTimeout(timer);
    signal.removeEventListener("abort", abortHandler);
  }
};

const prepareStream = async (leafRes, signal, started) => {
  if (!leafRes.body)
    return {
      error: new LeafError("leaf stream has no body", { tag: "Transport" }),
    };
  const reader = leafRes.body.getReader();
  const state = newSseState(started);
  const buffered = [];
  let bufferedBytes = 0;
  let pendingRead = null;
  const deadline = Date.now() + STREAM_FIRST_CONTENT_WINDOW_MS;
  const cancel = (reason) => {
    try {
      void reader.cancel(reason).catch(() => {});
    } catch {}
  };
  const prepared = (ended = false) => ({
    reader,
    state,
    buffered,
    pendingRead,
    ended,
  });

  for (;;) {
    if (state.streamError?.beforeContent) {
      cancel(state.streamError.error);
      return { error: state.streamError.error };
    }
    if (state.contentStarted) return prepared();
    if (state.done)
      return {
        error: new LeafError("leaf stream ended before content", {
          tag: "ProviderInternal",
        }),
      };
    if (bufferedBytes >= STREAM_PREFACE_MAX_BYTES) return prepared();
    const remaining = deadline - Date.now();
    if (remaining <= 0) return prepared();
    if (signal.aborted) {
      cancel(signal.reason);
      return { error: streamTransportError(signal.reason) };
    }

    pendingRead = reader.read().then(
      (result) => ({ kind: "read", result }),
      (error) => ({ kind: "error", error }),
    );
    const result = await readWithinWindow(pendingRead, remaining, signal);
    if (result.kind === "timeout") return prepared();
    pendingRead = null;
    if (result.kind === "aborted") {
      cancel(result.error);
      return { error: streamTransportError(result.error) };
    }
    if (result.kind === "error") return { error: streamTransportError(result.error) };
    if (result.result.done) {
      inspectSseBytes(null, state, leafRes.headers, true);
      if (state.streamError?.beforeContent)
        return { error: state.streamError.error };
      if (!state.contentStarted)
        return {
          error: new LeafError("leaf stream ended before content", {
            tag: "ProviderInternal",
          }),
        };
      return prepared(true);
    }
    const value = result.result.value;
    buffered.push(value);
    bufferedBytes += value.byteLength;
    inspectSseBytes(value, state, leafRes.headers);
  }
};

// One plain chat-completions call to a leaf provider. Returns the fetch
// Response (unchecked status handled by the caller for failover).
const callLeaf = async (cfg, leaf, body, wantStream, signal, sessionLabel) => {
  const baseURL = providerBaseURL(cfg, leaf.providerId);
  if (!baseURL)
    throw new LeafError(`no endpoint configured for provider ${leaf.providerId}`, {
      tag: "UnknownProvider",
      retryable: 0,
    });
  const def = providerDef(cfg, leaf.providerId);
  const apiKey = leafCredential(cfg, leaf.providerId);
  if (!apiKey)
    throw new LeafError(`no credential for provider ${leaf.providerId}`, {
      tag: "Authentication",
      retryable: 0,
    });
  const modelDef = def?.models?.[leaf.modelKey];
  const payload = {
    ...body,
    model: modelDef?.id || leaf.modelKey,
    stream: wantStream,
  };
  delete payload.provider;
  const eff = leaf.variant
    ? modelDef?.variants?.[leaf.variant]?.options?.reasoning_effort
    : null;
  if (eff) payload.reasoning_effort = eff;
  if (wantStream)
    payload.stream_options = {
      ...(body.stream_options || {}),
      include_usage: true,
    };
  const headers = {
    "content-type": "application/json",
    authorization: `Bearer ${apiKey}`,
    "user-agent": LEAF_UA,
  };
  // The OpenCode Go gateway attributes plan usage by this header; it is
  // an attribution label minted per request, not a local session.
  if (baseURL.includes("opencode.ai"))
    headers["x-opencode-session"] = sessionLabel;
  let res;
  try {
    res = await fetch(`${baseURL}/chat/completions`, {
      method: "POST",
      headers,
      body: JSON.stringify(payload),
      signal,
    });
  } catch (err) {
    throw new LeafError(`leaf transport error: ${err?.message || err}`, {
      tag: "Transport",
      code: err?.cause?.code || null,
    });
  }
  return res;
};

const usageFields = (usage) => {
  if (!usage || typeof usage !== "object") return {};
  return {
    tokensInput: usage.prompt_tokens ?? null,
    tokensOutput: usage.completion_tokens ?? null,
    tokensReasoning: usage.completion_tokens_details?.reasoning_tokens ?? null,
    tokensCacheRead: usage.prompt_tokens_details?.cached_tokens ?? null,
    tokensCacheWrite:
      usage.prompt_tokens_details?.cache_creation_tokens ?? null,
    costUsd: typeof usage.cost === "number" ? usage.cost : null,
  };
};

// ---------------------------------------------------------------- core

// Run one completion through the route's candidates. For streaming, a
// 2xx candidate is inspected through its first content delta (or a
// bounded preface window); pre-content failures continue the walk, then
// onStream takes over and returns the collected stream stats.
const runCompletion = async (routeId, body, clientSignal, hooks = {}) => {
  const cfg = loadConfig();
  if (!cfg.routes[routeId])
    throw new LeafError(`unknown route: ${routeId}`, {
      tag: "InvalidRequest",
      retryable: 0,
    });
  const wantStream = Boolean(body.stream);
  let candidates = resolveRoute(cfg, routeId, hooks.variantIn ?? null);
  candidates = applyDemotion(candidates, demotedPairs());
  candidates = applyKnownExhaustedGate(cfg, candidates);
  if (!candidates.length)
    throw new LeafError(`route ${routeId} has no available members`, {
      tag: "UnknownProvider",
      retryable: 0,
    });
  // Credential-less leaves are skipped like disabled providers (#3228):
  // a walk never spends one of its MAX_ATTEMPTS on a provider whose
  // credential lookup comes back empty (inactive or missing row), and
  // such a leaf never surfaces its misleading Authentication error as
  // the run's terminal failure.
  candidates = candidates.filter((c) => hasCredential(cfg, c.providerId));
  if (!candidates.length)
    throw new LeafError(`route ${routeId} has no credentialed members`, {
      tag: "UnknownProvider",
      retryable: 0,
    });
  const deadlineCtl = new AbortController();
  const timer = setTimeout(
    () => deadlineCtl.abort(new Error("opencode-route-proxy deadline")),
    TIMEOUT_MS,
  );
  const signal = clientSignal
    ? AbortSignal.any([deadlineCtl.signal, clientSignal])
    : deadlineCtl.signal;
  const sessionLabel = `ses_${randomBytes(12).toString("hex")}`;
  const attempts = candidates.slice(0, MAX_ATTEMPTS);
  let lastErr = null;
  try {
    for (let i = 0; i < attempts.length; i++) {
      const leaf = attempts[i];
      const started = Date.now();
      const next = attempts[i + 1];
      const failedOverTo = next ? `${next.providerId}/${next.modelKey}` : null;
      let res;
      const leafDeadlineCtl = new AbortController();
      const leafTimer = setTimeout(
        () =>
          leafDeadlineCtl.abort(
            new Error("opencode-route-proxy leaf first-response timeout"),
          ),
        LEAF_TIMEOUT_MS,
      );
      const leafSignal = AbortSignal.any([signal, leafDeadlineCtl.signal]);
      try {
        res = await callLeaf(
          cfg,
          leaf,
          body,
          wantStream,
          leafSignal,
          sessionLabel,
        );
      } catch (err) {
        const ended = Date.now();
        const leafTimedOut = !signal.aborted && leafDeadlineCtl.signal.aborted;
        recordAttempt({
          started,
          ended,
          routeId,
          providerId: leaf.providerId,
          modelId: leaf.modelKey,
          variant: leaf.variant,
          outcome: signal.aborted || leafTimedOut ? "timeout" : "failure",
          errorTag: err.tag || "Transport",
          errorCode: err.code ?? null,
          errorMessage: String(err.message || err).slice(0, 500),
          retryable: err.retryable ?? 1,
          outputStarted: false,
          failedOverTo,
          responseMs: ended - started,
          quota: err.quota ?? null,
        });
        lastErr = err;
        if (signal.aborted) throw err;
        continue;
      } finally {
        clearTimeout(leafTimer);
      }
      if (!res.ok) {
        const ended = Date.now();
        const text = await res.text().catch(() => "");
        const err = classifyHttp(res.status, res.headers, text);
        recordAttempt({
          started,
          ended,
          routeId,
          providerId: leaf.providerId,
          modelId: leaf.modelKey,
          variant: leaf.variant,
          outcome: "failure",
          errorTag: err.tag,
          errorStatus: err.status,
          errorMessage: String(err.message || err).slice(0, 500),
          retryable: err.retryable,
          outputStarted: false,
          failedOverTo,
          responseMs: ended - started,
          quota: err.quota,
        });
        lastErr = err;
        continue;
      }
      // Success path.
      if (wantStream) {
        const prepared = await prepareStream(res, signal, started);
        if (prepared.error) {
          const ended = Date.now();
          const err = prepared.error;
          recordAttempt({
            started,
            ended,
            routeId,
            providerId: leaf.providerId,
            modelId: leaf.modelKey,
            variant: leaf.variant,
            outcome: signal.aborted ? "timeout" : "failure",
            errorTag: err.tag || "ProviderInternal",
            errorCode: err.code ?? null,
            errorStatus: err.status,
            errorMessage: String(err.message || err).slice(0, 500),
            retryable: err.retryable ?? 1,
            outputStarted: false,
            failedOverTo,
            responseMs: ended - started,
            quota: err.quota ?? null,
          });
          lastErr = err;
          if (signal.aborted) throw err;
          continue;
        }
        const stats = await hooks.onStream(res, prepared);
        if (stats?.precommitError) {
          const ended = Date.now();
          const err = stats.precommitError;
          recordAttempt({
            started,
            ended,
            routeId,
            providerId: leaf.providerId,
            modelId: leaf.modelKey,
            variant: leaf.variant,
            outcome: signal.aborted ? "timeout" : "failure",
            errorTag: err.tag || "ProviderInternal",
            errorCode: err.code ?? null,
            errorStatus: err.status,
            errorMessage: String(err.message || err).slice(0, 500),
            retryable: err.retryable ?? 1,
            outputStarted: false,
            failedOverTo,
            responseMs: ended - started,
            quota: err.quota ?? null,
          });
          lastErr = err;
          if (signal.aborted) throw err;
          continue;
        }
        const ended = Date.now();
        const err = stats?.errorDetails ?? null;
        recordAttempt({
          started,
          ended,
          routeId,
          providerId: leaf.providerId,
          modelId: leaf.modelKey,
          variant: leaf.variant,
          outcome: stats?.error ? "failure" : "success",
          errorTag: err?.tag ?? (stats?.error ? "ProviderInternal" : null),
          errorCode: err?.code ?? null,
          errorStatus: err?.status ?? null,
          errorMessage: stats?.error
            ? String(stats.error).slice(0, 500)
            : null,
          retryable: err?.retryable,
          outputStarted: stats?.contentStarted ?? false,
          quota: err?.quota ?? null,
          firstTokenMs: stats?.firstTokenMs ?? null,
          responseMs: ended - started,
          ...(stats?.usage ? usageFields(stats.usage) : {}),
        });
        return { streamed: true };
      }
      const text = await res.text();
      const ended = Date.now();
      let json = null;
      try {
        json = JSON.parse(text);
      } catch {}
      recordAttempt({
        started,
        ended,
        routeId,
        providerId: leaf.providerId,
        modelId: leaf.modelKey,
        variant: leaf.variant,
        outcome: "success",
        outputStarted: true,
        responseMs: ended - started,
        ...(json?.usage ? usageFields(json.usage) : {}),
      });
      return { json, text };
    }
    throw lastErr || new Error(`route ${routeId}: all candidates failed`);
  } finally {
    clearTimeout(timer);
  }
};

// ---------------------------------------------------------------- http

const sendJson = (res, status, obj) => {
  if (res.destroyed || res.writableEnded) return;
  const body = JSON.stringify(obj);
  res.writeHead(status, { "content-type": "application/json" });
  res.end(body);
};
const sendError = (res, status, message) =>
  sendJson(res, status, { error: { message, type: "proxy_error" } });

// A wall-paced answer (#13693 RC-4): 503 with Retry-After until the
// earliest known reset, and a JSON error body naming the wall — the
// same { error: { message, type } } shape as every other proxy
// error, plus the retry-after header sendJson does not carry.
const sendWallPaced = (res, err) => {
  if (res.destroyed || res.writableEnded) return;
  const secs = Math.max(1, Math.ceil((err.retryAfterMs ?? 0) / 1000));
  res.writeHead(503, {
    "content-type": "application/json",
    "retry-after": String(secs),
  });
  res.end(
    JSON.stringify({
      error: { message: String(err?.message || err), type: "proxy_error" },
    }),
  );
};

const isPlainObject = (v) =>
  typeof v === "object" && v !== null && !Array.isArray(v);

const parseRouteModel = (raw) => {
  const model = typeof raw === "string" && raw ? raw : "workhorse";
  const id = model.startsWith("opencode-route/")
    ? model.slice("opencode-route/".length)
    : model;
  const h = id.indexOf("#");
  return h > 0
    ? { routeId: id.slice(0, h), variant: id.slice(h + 1) }
    : { routeId: id, variant: null };
};

// Relay the already-inspected preface and the rest of the SSE stream.
// A failure before a content delta is still a candidate failure; after a
// content delta has reached the client, retrying would splice responses.
const relayStream = async (prepared, res, headers) => {
  const { reader, state, buffered } = prepared;
  let pendingRead = prepared.pendingRead;
  let errorDetails = state.streamError?.error ?? null;
  let preContent = state.contentStarted ? [] : [...buffered];
  let preContentBytes = preContent.reduce((n, value) => n + value.byteLength, 0);
  const precommitFailure = (err) => ({
    usage: state.usage,
    firstTokenMs: state.firstTokenMs,
    contentStarted: false,
    precommitError: err,
  });
  const write = (value) => {
    if (!res.destroyed && !res.writableEnded) res.write(value);
  };

  if (state.contentStarted)
    for (const value of buffered) write(value);
  while (!prepared.ended) {
    const next = pendingRead
      ? await pendingRead
      : await reader.read().then(
          (result) => ({ kind: "read", result }),
          (err) => ({ kind: "error", error: err }),
        );
    pendingRead = null;
    if (next.kind === "error") {
      const err = streamTransportError(next.error);
      if (!state.contentStarted) {
        try {
          void reader.cancel(err).catch(() => {});
        } catch {}
        return precommitFailure(err);
      }
      errorDetails = err;
      break;
    }
    if (next.result.done) {
      inspectSseBytes(null, state, headers, true);
      if (state.streamError) errorDetails = state.streamError.error;
      if (!state.contentStarted)
        return precommitFailure(
          errorDetails ||
            new LeafError("leaf stream ended before content", {
              tag: "ProviderInternal",
            }),
        );
      break;
    }

    const value = next.result.value;
    inspectSseBytes(value, state, headers);
    if (state.streamError?.beforeContent && !state.contentStarted) {
      try {
        void reader.cancel(state.streamError.error).catch(() => {});
      } catch {}
      return precommitFailure(state.streamError.error);
    }
    if (!state.contentStarted) {
      preContent.push(value);
      preContentBytes += value.byteLength;
      if (preContentBytes > STREAM_PREFACE_MAX_BYTES) {
        // Distinguish a large content event from a non-content
        // preface (Codex P2 on PR #5558): while the inspector's
        // partial event already carries a data: payload, the event
        // has not been judged yet — defer the limit until it
        // completes. This matters when the prepare window expired
        // mid-event, so the relay's byte count restarted part-way
        // through one valid event. A hard ceiling still bounds a
        // never-ending event.
        if (
          state.tail.includes("data:") &&
          preContentBytes <= STREAM_EVENT_HARD_MAX_BYTES
        )
          continue;
        const err = new LeafError(
          "leaf stream preface exceeded the pre-content limit",
          { tag: "ProviderInternal" },
        );
        // Cancel the rejected upstream before failing over (Codex
        // P2 on PR #5558): without this, a successful replacement
        // clears the shared deadline timer and the rejected response
        // is left generating with no cancellation path — the
        // adjacent pre-content SSE-error path above already cancels.
        try {
          void reader.cancel(err).catch(() => {});
        } catch {}
        return precommitFailure(err);
      }
      continue;
    }
    for (const chunk of preContent) write(chunk);
    preContent = [];
    write(value);
    if (state.streamError) errorDetails = state.streamError.error;
  }
  return {
    usage: state.usage,
    firstTokenMs: state.firstTokenMs,
    contentStarted: state.contentStarted,
    error: errorDetails?.message ?? null,
    errorDetails,
  };
};

const handleChatCompletions = async (req, res, body) => {
  if (!isPlainObject(body))
    return sendError(res, 400, "request body must be a JSON object");
  if (!Array.isArray(body.messages))
    return sendError(res, 400, "messages must be an array");
  if (body.messages.some((m) => !isPlainObject(m)))
    return sendError(res, 400, "each message must be an object");
  const { routeId, variant: reqVariant } = parseRouteModel(body.model);
  const modelName = `opencode-route/${routeId}`;
  const clientCtl = new AbortController();
  res.on("close", () => {
    if (!res.writableEnded)
      clientCtl.abort(new Error("client disconnected"));
  });

  if (body.stream) {
    let headersSent = false;
    try {
      await runCompletion(routeId, body, clientCtl.signal, {
        variantIn: reqVariant,
        onStream: async (leafRes, prepared) => {
          if (!headersSent) {
            res.writeHead(200, {
              "content-type": "text/event-stream",
              "cache-control": "no-cache",
              connection: "keep-alive",
            });
            headersSent = true;
          }
          return relayStream(prepared, res, leafRes.headers);
        },
      });
      if (!res.destroyed && !res.writableEnded) res.end();
    } catch (err) {
      if (err?.wallPaced && !headersSent) return sendWallPaced(res, err);
      if (!headersSent) {
        return sendError(res, 502, String(err?.message || err));
      }
      if (!res.destroyed && !res.writableEnded) {
        res.write(
          `data: ${JSON.stringify({ error: { message: String(err?.message || err), type: "proxy_error" } })}\n\n`,
        );
        res.end();
      }
    }
    return;
  }

  try {
    const { json, text } = await runCompletion(routeId, body, clientCtl.signal, { variantIn: reqVariant });
    if (json) {
      json.model = modelName;
      return sendJson(res, 200, json);
    }
    if (!res.destroyed && !res.writableEnded) {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(text);
    }
  } catch (err) {
    if (err?.wallPaced) return sendWallPaced(res, err);
    return sendError(res, 502, String(err?.message || err));
  }
};

const server = createServer(async (req, res) => {
  res.on("error", () => {});
  try {
    const url = new URL(req.url, `http://${HOST}:${PORT}`);
    if (req.method === "GET" && url.pathname === "/health") {
      return sendJson(res, 200, { ok: true, service: "opencode-route-proxy" });
    }
    if (req.method === "GET" && url.pathname === "/v1/models") {
      return sendJson(res, 200, {
        object: "list",
        data: ADVERTISED_MODELS.map((id) => ({
          id,
          object: "model",
          created: 0,
          owned_by: "opencode-route",
        })),
      });
    }
    if (req.method === "POST" && url.pathname === "/v1/chat/completions") {
      // Collect the body as bytes and decode UTF-8 once, after the full
      // body arrives (#4335): per-chunk string coercion corrupts multibyte
      // characters split across TCP chunks. Cap 50 MB like the sibling
      // proxies (bin/request-body.mjs collector) with a 413 past the cap.
      const bodyCollector = createRequestBodyCollector(50 * 1024 * 1024);
      req.on("data", (chunk) => {
        if (bodyCollector.tooLarge) return;
        if (!bodyCollector.push(chunk)) {
          sendError(res, 413, "request body too large");
          req.destroy();
        }
      });
      req.on("end", async () => {
        if (bodyCollector.tooLarge) return;
        let body = {};
        try {
          body = JSON.parse(bodyCollector.text() || "{}");
        } catch {
          return sendError(res, 400, "invalid JSON body");
        }
        await handleChatCompletions(req, res, body);
      });
      return;
    }
    return sendError(res, 404, `no route for ${req.method} ${url.pathname}`);
  } catch (err) {
    return sendError(res, 500, String(err?.message || err));
  }
});

server.listen(PORT, HOST, () => {
  console.log(`opencode-route-proxy listening on http://${HOST}:${PORT}`);
});
