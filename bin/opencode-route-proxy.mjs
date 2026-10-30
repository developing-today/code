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
// quota demotion from the route_attempt ledger), and makes ONE plain
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
//   OPENCODE_ROUTE_PROXY_CONFIG      opencode.jsonc path (default:
//                                    ~/.local/share/opencode/opencode.jsonc,
//                                    else ../.opencode/opencode.jsonc
//                                    relative to this file)
//   OPENCODE_ROUTE_PROXY_DB          opencode.db path (default:
//                                    ~/.local/share/opencode/opencode.db)
import { createServer } from "node:http";
import { readFileSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { DatabaseSync } from "node:sqlite";
import { randomBytes } from "node:crypto";
import { parseProxyPort } from "./proxy-port.mjs";

const PORT = parseProxyPort("OPENCODE_ROUTE_PROXY_PORT", 18750);
const HOST = process.env.OPENCODE_ROUTE_PROXY_HOST || "127.0.0.1";
const TIMEOUT_MS = Number(process.env.OPENCODE_ROUTE_PROXY_TIMEOUT_MS || 1_200_000);
const HERE = dirname(fileURLToPath(import.meta.url));
const CONFIG_PATH =
  process.env.OPENCODE_ROUTE_PROXY_CONFIG ||
  join(homedir(), ".local/share/opencode/opencode.jsonc");
const CONFIG_FALLBACK = join(HERE, "..", ".opencode", "opencode.jsonc");
const DB_PATH =
  process.env.OPENCODE_ROUTE_PROXY_DB ||
  join(homedir(), ".local/share/opencode/opencode.db");

// Providers built into OpenCode itself (no block in opencode.jsonc):
// instanceOf targets resolve here when the config has no block either.
const BUILTIN_PROVIDERS = {
  "opencode-go": { options: { baseURL: "https://opencode.ai/zen/go/v1" } },
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
  if (cfgCache && cfgCache.path === path && cfgCache.mtimeMs === st.mtimeMs)
    return cfgCache;
  const raw = parseJsonc(readFileSync(path, "utf8"));
  cfgCache = {
    path,
    mtimeMs: st.mtimeMs,
    providers: raw.provider || {},
    routes: raw.experimental?.model_routes || raw.model_routes || {},
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

// Effective provider definition, following instanceOf chains into the
// config's base block or the built-in table.
const providerDef = (cfg, id) => {
  let def = cfg.providers[id];
  const seen = new Set([id]);
  while (def && def.instanceOf && !seen.has(def.instanceOf)) {
    seen.add(def.instanceOf);
    const base = cfg.providers[def.instanceOf] || BUILTIN_PROVIDERS[def.instanceOf];
    if (!base) break;
    def = {
      ...base,
      ...def,
      options: { ...(base.options || {}), ...(def.options || {}) },
      models: def.models || base.models,
    };
  }
  if (!def) def = BUILTIN_PROVIDERS[id] || null;
  return def;
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
        if (typeof parsed?.key === "string" && parsed.key) key = parsed.key;
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

// (provider,model) pairs demoted by the doctrine: >=3 quota failures
// (quota detail recorded, or a RateLimit tag) inside the last 4 hours.
const demotedPairs = () => {
  let db = null;
  try {
    db = openDb();
    const rows = db
      .prepare(
        `SELECT provider_id, model_id FROM route_attempt
         WHERE time_started > ? AND outcome != 'success'
           AND (quota IS NOT NULL OR error_tag = 'RateLimit')
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

const normalizeTarget = (t) => {
  if (typeof t === "string") return { ...parseTargetRef(t), variantMap: null };
  if (t && typeof t.model === "string")
    return { ...parseTargetRef(t.model), variantMap: t.variants || null };
  return null;
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

// Resolve a route to its ordered leaf candidates:
// [{ providerId, modelKey, variant }]. Nested routes recurse; disabled
// providers are skipped; cycles are cut.
const resolveRoute = (cfg, routeId, variantIn = null) => {
  const out = [];
  const expand = (rid, vIn, seen) => {
    if (seen.has(rid)) return;
    seen.add(rid);
    const route = cfg.routes[rid];
    if (!route) return;
    let targets = (route.targets || [])
      .map(normalizeTarget)
      .filter(Boolean)
      .map((t) => ({ ...t, refBase: `${t.providerId}/${t.modelKey}` }));
    if (route.selection === "round-robin") targets = rotate(targets, rid);
    else if (route.selection === "weighted")
      targets = weightedOrder(targets, route.weights || {});
    for (const t of targets) {
      let v = vIn;
      if (t.variantMap && v && t.variantMap[v]) v = t.variantMap[v];
      if (t.variant) v = t.variant;
      if (t.providerId === "opencode-route") expand(t.modelKey, v, seen);
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
  const staticKey = def?.options?.apiKey ? expandEnv(def.options.apiKey) : null;
  const apiKey = staticKey || credentialKey(leaf.providerId);
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

// Run one completion through the route's candidates. Streaming is
// two-phase: candidates are tried until one returns a 2xx (nothing has
// been sent to the client yet), then onStream(res) takes over the
// response and this returns the collected stream stats.
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
  if (!candidates.length)
    throw new LeafError(`route ${routeId} has no available members`, {
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
      try {
        res = await callLeaf(cfg, leaf, body, wantStream, signal, sessionLabel);
      } catch (err) {
        const ended = Date.now();
        recordAttempt({
          started,
          ended,
          routeId,
          providerId: leaf.providerId,
          modelId: leaf.modelKey,
          variant: leaf.variant,
          outcome: signal.aborted ? "timeout" : "failure",
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
        const stats = await hooks.onStream(res);
        const ended = Date.now();
        recordAttempt({
          started,
          ended,
          routeId,
          providerId: leaf.providerId,
          modelId: leaf.modelKey,
          variant: leaf.variant,
          outcome: stats?.error ? "failure" : "success",
          errorTag: stats?.error ? "ProviderInternal" : null,
          errorMessage: stats?.error
            ? String(stats.error).slice(0, 500)
            : null,
          outputStarted: true,
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

// Relay a leaf SSE stream verbatim to the client while collecting the
// final usage block and timing. Returns { usage, firstTokenMs, error }.
const relayStream = async (leafRes, res) => {
  const started = Date.now();
  let firstTokenMs = null;
  let usage = null;
  let error = null;
  const reader = leafRes.body.getReader();
  const decoder = new TextDecoder();
  let tail = "";
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      if (firstTokenMs === null) firstTokenMs = Date.now() - started;
      if (!res.destroyed && !res.writableEnded) res.write(value);
      // Keep only the tail: usage rides in the final chunk(s).
      tail = (tail + decoder.decode(value, { stream: true })).slice(-8192);
    }
  } catch (err) {
    error = err?.message || String(err);
  }
  for (const line of tail.split("\n")) {
    if (!line.startsWith("data:")) continue;
    const data = line.slice(5).trim();
    if (!data || data === "[DONE]") continue;
    try {
      const parsed = JSON.parse(data);
      if (parsed.usage) usage = parsed.usage;
      if (parsed.error) error = parsed.error.message || JSON.stringify(parsed.error);
    } catch {}
  }
  return { usage, firstTokenMs, error };
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
        onStream: async (leafRes) => {
          res.writeHead(200, {
            "content-type": "text/event-stream",
            "cache-control": "no-cache",
            connection: "keep-alive",
          });
          headersSent = true;
          const stats = await relayStream(leafRes, res);
          if (!res.destroyed && !res.writableEnded) res.end();
          return stats;
        },
      });
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
