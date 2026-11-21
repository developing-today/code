// MCP Events for ChatGPT: a ChatGPT chat subscribes to "message.created"
// and the bridge wakes it with a signed webhook whenever Drew sends
// ChatGPT a message (from the page or the JSON API).
//
// Protocol: OpenAI's MCP Events guide (developers.openai.com/plugins/
// build/mcp-events), which follows Standard Webhooks for signing.
//   events/list        one definition, message.created
//   events/subscribe   {name, arguments: {}, delivery: {mode: "webhook",
//                      url, secret: "whsec_..."}, ttlMs?, cursor?}
//                      -> {id, refreshBefore, cursor: null, truncated: false}
//   events/unsubscribe {name, arguments, delivery: {mode, url}} -> {}
// Subscribing verifies the callback first: a signed
// {"type": "verification", "challenge"} POST that must echo the challenge.
//
// Only the ChatGPT channel advertises events; Claude has no webhook to
// wake. Subscriptions belong to that channel and carry the ChatGPT token's
// binding, so rotating CHATGPT_TOKEN ends them. Callback hosts are limited
// to OpenAI's domains (EVENTS_CALLBACK_HOSTS can add exact hosts), always
// https on the default port, never redirected. A Worker can't pin the
// resolved IP the way the guide's Go example does; the host allowlist plus
// Cloudflare's egress (no private networks) stand in for it.
//
// Delivery is not replayable (cursor null): an event that fails all its
// attempts is lost, and read_messages with unread_only still finds the
// message. Each delivery and failure is noted in the sign-in event log.
import { channelBinding } from "./oauth.js";

export const EVENT_NAME = "message.created";
export const EVENT_CHANNELS = ["chatgpt"];
export const MAX_SUBSCRIPTIONS = 20;
export const DEFAULT_TTL_MS = 7 * 24 * 3600 * 1000;
export const MAX_TTL_MS = DEFAULT_TTL_MS;
export const VERIFY_TTL_MS = 10 * 60 * 1000;
export const OLD_SECRET_MS = 5 * 60 * 1000;
export const ATTEMPTS = 3;
export const BACKOFF_MS = [0, 1000, 4000];
export const TIMEOUT_MS = 10_000;
export const MAX_EVENT_BODY = 256 * 1024;
const CALLBACK_DOMAINS = ["openai.com", "chatgpt.com"];

export const EVENT_DEFINITION = {
  name: EVENT_NAME,
  description:
    "A new message for ChatGPT arrived on the message bridge, from Drew or (from_addr set) from another assistant " +
    "or a fleet agent. data carries the message; kind \"instruction\" means an instruction to act on. Answer with " +
    "send_message reply_to its message_id, and acknowledge it with ack_messages once handled.",
  delivery: ["webhook"],
  inputSchema: { type: "object", properties: {}, additionalProperties: false },
  payloadSchema: {
    type: "object",
    properties: {
      message_id: { type: "integer" },
      kind: { type: "string", enum: ["message", "instruction"] },
      sender: { type: ["string", "null"] },
      from_addr: { type: ["string", "null"], description: "Who sent it: null for Drew, channel:<name> or agent:<name>." },
      body: { type: "string" },
      ts: { type: "string" },
    },
    required: ["message_id", "kind", "body", "ts"],
    additionalProperties: false,
  },
};

/** A JSON-RPC error the MCP handler passes through as-is. */
export class EventError extends Error {
  constructor(code, message, data) {
    super(message);
    this.code = code;
    this.data = data;
  }
}
const invalid = (why) => new EventError(-32602, `Invalid event subscription: ${why}`);

const enc = new TextEncoder();
const b64 = (bytes) => btoa(String.fromCharCode(...bytes));
const hex = (bytes) => Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");

/** The secret's key bytes: whsec_ + strict base64 of 24 to 64 bytes, else null. */
export function secretKey(secret) {
  if (typeof secret !== "string" || !/^whsec_[A-Za-z0-9+/]+={0,2}$/.test(secret) || secret.length > 100) return null;
  try {
    const raw = secret.slice(6);
    const bytes = Uint8Array.from(atob(raw), (c) => c.charCodeAt(0));
    if (b64(bytes) !== raw || bytes.length < 24 || bytes.length > 64) return null;
    return bytes;
  } catch {
    return null;
  }
}

function allowedHosts(env) {
  return String(env.EVENTS_CALLBACK_HOSTS ?? "")
    .split(",")
    .map((h) => h.trim().toLowerCase())
    .filter(Boolean);
}

/** The canonical callback URL, or null: https, an OpenAI host, no credentials, port or fragment. */
export function callbackUrl(env, raw) {
  if (typeof raw !== "string" || raw.length > 2048) return null;
  let u;
  try {
    u = new URL(raw);
  } catch {
    return null;
  }
  if (u.protocol !== "https:" || u.username || u.password || u.port || u.hash || raw.includes("#")) return null;
  const host = u.hostname.toLowerCase();
  const ok =
    allowedHosts(env).includes(host) || CALLBACK_DOMAINS.some((d) => host === d || host.endsWith(`.${d}`));
  return ok ? u.href : null;
}

/** Standard Webhooks signature over id.timestamp.body, one per secret. */
export async function sign(secrets, id, timestamp, body) {
  const out = [];
  for (const secret of secrets) {
    const key = await crypto.subtle.importKey("raw", secretKey(secret), { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
    const mac = new Uint8Array(await crypto.subtle.sign("HMAC", key, enc.encode(`${id}.${timestamp}.${body}`)));
    out.push(`v1,${b64(mac)}`);
  }
  return out.join(" ");
}

/** Deterministic id from principal, callback URL, event name and (canonical, empty) arguments. */
async function subscriptionId(channel, url) {
  const digest = await crypto.subtle.digest("SHA-256", enc.encode(JSON.stringify([channel, url, EVENT_NAME, {}])));
  return `sub_${hex(new Uint8Array(digest)).slice(0, 40)}`;
}

const egress = (env) => (typeof env.EVENTS_EGRESS?.fetch === "function" ? env.EVENTS_EGRESS : globalThis);

/** One signed POST; resolves to the Response, never follows a redirect. */
async function post(env, sub, id, payload) {
  const body = JSON.stringify(payload);
  if (body.length > MAX_EVENT_BODY) throw new Error("event too large");
  const timestamp = String(Math.floor(Date.now() / 1000));
  const secrets = [sub.secret];
  if (sub.old_secret && sub.old_until > Date.now()) secrets.push(sub.old_secret);
  return egress(env).fetch(sub.url, {
    method: "POST",
    redirect: "manual",
    signal: AbortSignal.timeout(TIMEOUT_MS),
    headers: {
      "content-type": "application/json",
      "webhook-id": id,
      "webhook-timestamp": timestamp,
      "webhook-signature": await sign(secrets, id, timestamp, body),
      "x-mcp-subscription-id": sub.id,
    },
    body,
  });
}

function sameString(a, b) {
  if (typeof a !== "string" || typeof b !== "string" || a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return diff === 0;
}

const randomToken = (n = 24) => hex(crypto.getRandomValues(new Uint8Array(n)));

async function verifyCallback(env, sub) {
  const challenge = randomToken();
  let res;
  try {
    res = await post(env, sub, `msg_verification_${randomToken(12)}`, { type: "verification", challenge });
  } catch (err) {
    throw new EventError(-32015, "Callback verification failed", {
      reason: err?.name === "TimeoutError" ? "timeout" : "unreachable",
    });
  }
  let echoed = null;
  if (res.status >= 200 && res.status < 300) {
    try {
      echoed = JSON.parse((await res.text()).slice(0, 4096))?.challenge;
    } catch {
      echoed = null;
    }
  }
  if (!sameString(echoed, challenge)) {
    throw new EventError(-32015, "Callback verification failed", { reason: "challenge_failed", status: res.status });
  }
}

function checkShape(params, { needSecret }) {
  if (!params || typeof params !== "object") throw invalid("params must be an object");
  if (params.name !== EVENT_NAME) throw invalid(`unknown event ${String(params.name).slice(0, 60)}`);
  const args = params.arguments ?? {};
  if (!args || typeof args !== "object" || Array.isArray(args) || Object.keys(args).length) {
    throw invalid("message.created takes no arguments");
  }
  const d = params.delivery;
  if (!d || typeof d !== "object" || d.mode !== "webhook") throw invalid("delivery.mode must be webhook");
  if (needSecret && !secretKey(d.secret)) throw invalid("delivery.secret must be whsec_ and 24 to 64 base64 bytes");
}

/**
 * The events/* methods for one MCP channel, or null when that channel
 * has no events. store holds the subscriptions; note logs for the owner.
 */
export function eventsFor(env, store, channel, note = async () => {}) {
  if (!EVENT_CHANNELS.includes(channel)) return null;
  return {
    async list() {
      return { events: [EVENT_DEFINITION] };
    },

    async subscribe(params) {
      checkShape(params, { needSecret: true });
      const url = callbackUrl(env, params.delivery.url);
      if (!url) {
        let host = null;
        try {
          host = new URL(params.delivery.url).hostname.slice(0, 80);
        } catch {
          // not a URL
        }
        await note("events_refused", { channel, host, error: "callback host not allowed" });
        throw invalid("callback url must be https on an OpenAI host");
      }
      const ttl = params.ttlMs;
      if (ttl !== undefined && ttl !== null && (!Number.isSafeInteger(ttl) || ttl <= 0)) throw invalid("ttlMs must be a positive integer");
      const binding = await channelBinding(env, channel);
      if (binding === null) throw new EventError(-32603, "ChatGPT channel is not configured");
      const now = Date.now();
      const id = await subscriptionId(channel, url);
      const old = await store.eventSubGet(id);
      const live = old && old.expires > now && old.k === binding;
      const secret = params.delivery.secret;
      const sub = {
        id,
        channel,
        k: binding,
        url,
        secret,
        old_secret: null,
        old_until: null,
        expires: now + Math.min(ttl ?? DEFAULT_TTL_MS, MAX_TTL_MS),
        verified_until: null,
        created: live ? old.created : now,
      };
      if (live && old.secret !== secret) {
        sub.old_secret = old.secret;
        sub.old_until = now + OLD_SECRET_MS;
      } else if (live && old.old_secret && old.old_until > now) {
        sub.old_secret = old.old_secret;
        sub.old_until = old.old_until;
      }
      if (live && old.secret === secret && old.verified_until > now) {
        sub.verified_until = old.verified_until;
      } else {
        try {
          await verifyCallback(env, sub);
        } catch (err) {
          await note("events_refused", { channel, host: new URL(url).hostname, error: err.message, ...(err.data ?? {}) });
          throw err;
        }
        sub.verified_until = now + VERIFY_TTL_MS;
      }
      if (!(await store.eventSubPut(sub, MAX_SUBSCRIPTIONS))) {
        throw new EventError(-32000, "too many event subscriptions; unsubscribe one first");
      }
      await note("events_subscribed", { channel, host: new URL(url).hostname, renewed: Boolean(live) });
      const ttlGranted = sub.expires - now;
      return {
        id,
        refreshBefore: new Date(now + Math.floor(ttlGranted * 0.75)).toISOString(),
        cursor: null,
        truncated: false,
      };
    },

    async unsubscribe(params) {
      checkShape(params, { needSecret: false });
      const url = callbackUrl(env, params.delivery.url);
      if (url) {
        const removed = await store.eventSubDelete(await subscriptionId(channel, url));
        if (removed) await note("events_unsubscribed", { channel, host: new URL(url).hostname });
      }
      return {};
    },
  };
}

/**
 * Delivers message.created for each new inbound ChatGPT message to every
 * live subscription. Runs after the response (waitUntil); never throws.
 */
export async function deliverMessages(env, store, messages, note = async () => {}) {
  const wanted = messages.filter((m) => m.direction === "inbound" && EVENT_CHANNELS.includes(m.channel));
  if (!wanted.length) return;
  for (const channel of new Set(wanted.map((m) => m.channel))) {
    let subs;
    try {
      subs = await store.eventSubsLive(channel, Date.now());
    } catch {
      continue;
    }
    const binding = await channelBinding(env, channel);
    for (const sub of subs) {
      if (sub.k !== binding) {
        await store.eventSubDelete(sub.id);
        continue;
      }
      for (const m of wanted.filter((x) => x.channel === channel)) {
        const eventId = `evt_${m.id}_${randomToken(8)}`;
        const payload = {
          eventId,
          name: EVENT_NAME,
          timestamp: new Date().toISOString(),
          data: {
            message_id: m.id,
            kind: m.kind ?? "message",
            sender: m.sender ?? null,
            from_addr: m.from_addr ?? null,
            body: m.body,
            ts: m.ts,
          },
          cursor: null,
        };
        const outcome = await deliverOne(env, sub, eventId, payload);
        if (outcome.gone) await store.eventSubDelete(sub.id);
        await note(outcome.ok ? "event_delivered" : "event_failed", {
          channel,
          message_id: m.id,
          host: new URL(sub.url).hostname,
          status: outcome.status ?? null,
          attempts: outcome.attempts,
        });
      }
    }
  }
}

async function deliverOne(env, sub, eventId, payload) {
  let status = null;
  for (let attempt = 0; attempt < ATTEMPTS; attempt++) {
    if (BACKOFF_MS[attempt]) await new Promise((r) => setTimeout(r, BACKOFF_MS[attempt]));
    try {
      const res = await post(env, sub, eventId, payload);
      status = res.status;
      if (status >= 200 && status < 300) return { ok: true, status, attempts: attempt + 1 };
      // 410: the subscription is gone on ChatGPT's side; 413 and other
      // 4xx won't get better on a retry (408 and 429 might).
      if (status === 410) return { ok: false, gone: true, status, attempts: attempt + 1 };
      if (status >= 300 && status < 500 && status !== 408 && status !== 429) return { ok: false, status, attempts: attempt + 1 };
    } catch {
      status = null;
    }
  }
  return { ok: false, status, attempts: ATTEMPTS };
}
