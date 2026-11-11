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
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { DatabaseSync } from "node:sqlite";
import { randomBytes } from "node:crypto";
import { parseProxyMillis, parseProxyPort } from "./proxy-port.mjs";

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
  join(homedir(), ".local/share/opencode/opencode.jsonc");
const CONFIG_FALLBACK = join(HERE, "..", ".opencode", "opencode.jsonc");
const RUNTIME_CONFIG_PATH =
  process.env.OPENCODE_ROUTE_PROXY_RUNTIME_CONFIG ||
  join(homedir(), ".local/state/opencode/opencode-runtime.jsonc");
const DB_PATH =
  process.env.OPENCODE_ROUTE_PROXY_DB ||
  join(homedir(), ".local/share/opencode/opencode.db");

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
// (e.g. a bare paymentRequired) cannot gate: how long it lasts is
// unknown, and demotion remains its only handling.
const quotaResetMs = (timeEnded, quotaText) => {
  let q;
  try {
    q = JSON.parse(quotaText);
  } catch {
    return null;
  }
  if (!q || typeof q !== "object") return null;
  // A relative retry delay runs from when the quota response was
  // RECEIVED (the attempt's time_ended), not from when the request
  // started: anchoring to the start would let a slow 429 response
  // consume its own backoff.
  if (Number.isFinite(q.retryAfterMs) && q.retryAfterMs > 0)
    return timeEnded + q.retryAfterMs;
  for (const k of ["resetsAtMs", "resetAtMs", "retryAtMs"])
    if (Number.isFinite(q[k]) && q[k] > 0) return q[k];
  for (const k of ["resetsAt", "resetAt", "retryAt"]) {
    const ms = Date.parse(q[k]);
    if (Number.isFinite(ms)) return ms;
  }
  return null;
};

// (provider,model) pairs whose binding quota window is KNOWN
// exhausted (#5295, #4370 acceptance (c)): the pair's latest
// quota-bearing failure in the ledger carries a reset still in the
// future, and no success has been recorded for the pair since that
// failure. Unlike demotion — reactive (>=3 failures in 4h) and
// reorder-only — this is a seating-time exclusion known BEFORE the
// next failure: a monthly-walled leg at a known 100% must not be a
// first-choice candidate while any live candidate exists. The ledger
// is the only source (no new network dependency in the hot path);
// the lookback bounds the scan while covering monthly resets.
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
        `SELECT provider_id, model_id, time_started, time_ended, quota FROM route_attempt
         WHERE quota IS NOT NULL AND outcome != 'success' AND time_ended > ?
           AND time_ended = (
             SELECT MAX(time_ended) FROM route_attempt r2
             WHERE r2.provider_id = route_attempt.provider_id
               AND r2.model_id = route_attempt.model_id
               AND r2.quota IS NOT NULL AND r2.outcome != 'success'
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
    for (const r of quotaRows) {
      const reset = quotaResetMs(r.time_ended, r.quota);
      if (reset === null || reset <= now) continue;
      const key = `${r.provider_id}/${r.model_id}`;
      const successAt = latestSuccess.get(key);
      if (successAt !== undefined && successAt > r.time_ended) continue;
      out.set(key, reset);
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

// Exclude known-exhausted candidates (#5295), following the
// credential-less exclusion pattern (#3228) — but fail-safe: if the
// gate would empty the candidate list, the demoted order is kept as
// it was, because a known-exhausted leg as last resort beats no
// route at all. Gate decisions are recorded on stderr, the proxy's
// diagnostics stream.
const applyKnownExhaustedGate = (cfg, candidates) => {
  if (!candidates.length) return candidates;
  const exhausted = knownExhaustedPairs(nowMs());
  if (!exhausted.size) return candidates;
  const key = (c) => `${c.providerId}/${c.modelKey}`;
  const kept = candidates.filter((c) => !exhausted.has(key(c)));
  // The fail-safe is evaluated against CREDENTIALED survivors: the
  // credential-less filter (#3228) runs after this gate, so a kept
  // list whose only members have no credential would still empty the
  // route — and the exhausted leaf must then stay as the last resort
  // instead of the request failing with "no credentialed members".
  if (!kept.some((c) => hasCredential(cfg, c.providerId))) {
    console.error(
      "opencode-route-proxy: known-exhausted gate bypassed — no credentialed " +
        "candidate survives it; keeping the demoted order (#5295)",
    );
    return candidates;
  }
  for (const c of candidates)
    if (exhausted.has(key(c)))
      console.error(
        `opencode-route-proxy: known-exhausted gate excluded ${key(c)} ` +
          `until ${new Date(exhausted.get(key(c))).toISOString()} (#5295)`,
      );
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

// At config load, name any model_route_windows entry whose start/end
// is not a valid HH:MM (hour 00-23, minute 00-59) instead of silently
// evaluating it (#4259). windowActive fails closed (never active) for
// these entries, so a typo'd window surfaces as a loud no-match rather
// than flipping a route gate to always-on/always-off/end-less.
const warnInvalidWindows = (windows) => {
  if (!windows || typeof windows !== "object") return;
  for (const [name, win] of Object.entries(windows)) {
    const start = parseHHMM(win?.start);
    const end = parseHHMM(win?.end);
    if (start === null || end === null) {
      console.error(
        `opencode-route-proxy: invalid model_route_windows["${name}"]: ` +
          `start=${JSON.stringify(win?.start)} end=${JSON.stringify(win?.end)} ` +
          `(expected HH:MM with hour 00-23, minute 00-59); window will never match`,
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
const WEEKDAY_NAMES = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
const windowActive = (win, now) => {
  if (!win || typeof win !== "object") return false;
  const start = parseHHMM(win.start);
  const end = parseHHMM(win.end);
  if (start === null || end === null) return false;
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
      ? win.daysOfWeek.map((d) => String(d).slice(0, 3).toLowerCase())
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
  const retryAfter = headers.get("retry-after");
  if (status === 429 && retryAfter) {
    const secs = Number(retryAfter);
    quota = JSON.stringify({
      retryAfterMs: Number.isFinite(secs) ? secs * 1000 : null,
      headers: { "retry-after": retryAfter },
    });
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
      let raw = "";
      for await (const part of req) raw += part;
      let body = {};
      try {
        body = JSON.parse(raw || "{}");
      } catch {
        return sendError(res, 400, "invalid JSON body");
      }
      return await handleChatCompletions(req, res, body);
    }
    return sendError(res, 404, `no route for ${req.method} ${url.pathname}`);
  } catch (err) {
    return sendError(res, 500, String(err?.message || err));
  }
});

server.listen(PORT, HOST, () => {
  console.log(`opencode-route-proxy listening on http://${HOST}:${PORT}`);
});
