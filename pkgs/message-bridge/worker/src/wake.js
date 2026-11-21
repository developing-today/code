// Wake hooks: when a message lands for an address that has a hook, the
// bridge calls that hook so whoever reads the address starts working,
// instead of finding the message on its next poll.
//
// ChatGPT is woken by MCP Events (events.js). Claude, and anyone else,
// is woken by a hook Drew sets on his page: a URL and, if it needs one,
// a bearer token, stored in the Durable Object (never shown again, never
// in a log).
//
//   - A Claude Code routine's API trigger (a URL under
//     https://api.anthropic.com/v1/claude_code/routines/<id>/fire) gets
//     the headers that endpoint needs and {text}: each fire starts a new
//     Claude session, which reads the bridge with its connector.
//   - Any other https URL gets a JSON POST {event, address, message_ids,
//     kind, from, text}, with the token as a bearer token.
//
// Hooks are keyed by address: "channel:claude", "channel:chatgpt" or
// "agent:<name>". The text names the message ids and who sent them,
// never the bodies: the woken session reads those from the bridge.
//
// Two connectors can answer each other forever, so each address is
// woken at most WAKE_MAX_PER_HOUR times an hour (default 20, which also
// stays under a routine's own 30 fires an hour); messages past the cap
// wait, unread, for the next wake or poll. One request that stores
// several rows for an address wakes it once.

import { CHANNEL_NAMES, InputError } from "./messages.js";

export const WAKE_ADDRESS = /^(channel:(claude|chatgpt)|agent:[a-z0-9][a-z0-9._-]{0,63})$/;
export const DEFAULT_WAKE_MAX_PER_HOUR = 20;
const ROUTINE_FIRE = /^https:\/\/api\.anthropic\.com\/v1\/claude_code\/routines\/trig_[A-Za-z0-9]{8,64}\/fire$/;
const MAX_URL = 2048;
const MAX_TOKEN = 1024;
const TIMEOUT_MS = 10_000;

export function wakeMaxPerHour(env) {
  const n = Number(env.WAKE_MAX_PER_HOUR);
  return Number.isInteger(n) && n > 0 && n <= 120 ? n : DEFAULT_WAKE_MAX_PER_HOUR;
}

/** Validates a hook Drew sets; returns {url, token, kind}. Throws InputError. */
export function normalizeHook({ address, url, token }) {
  if (typeof address !== "string" || !WAKE_ADDRESS.test(address.trim().toLowerCase())) {
    throw new InputError("address must be channel:claude, channel:chatgpt or agent:<name>");
  }
  if (typeof url !== "string" || url.length > MAX_URL) throw new InputError("url must be an https URL");
  let u;
  try {
    u = new URL(url.trim());
  } catch {
    throw new InputError("url must be an https URL");
  }
  if (u.protocol !== "https:" || u.username || u.password || u.hash) {
    throw new InputError("url must be https, with no user name, password or #fragment");
  }
  let t = null;
  if (token !== undefined && token !== null && token !== "") {
    if (typeof token !== "string" || token.length > MAX_TOKEN || !/^[\x21-\x7e]+$/.test(token.trim())) {
      throw new InputError("token must be printable text with no spaces");
    }
    t = token.trim();
  }
  const kind = ROUTINE_FIRE.test(u.href) ? "claude-routine" : "webhook";
  if (kind === "claude-routine" && !t) throw new InputError("a routine's fire URL needs its token");
  return { address: address.trim().toLowerCase(), url: u.href, token: t, kind };
}

/** What the owner sees of a hook: never its token or its path. */
export const showHook = (h) => ({
  address: h.address,
  kind: h.kind,
  host: new URL(h.url).hostname,
  has_token: !!h.token,
  added: new Date(h.added).toISOString(),
});

const nameOf = (addr) => {
  if (!addr) return "Drew";
  const [type, name] = addr.split(":");
  return type === "channel" ? (CHANNEL_NAMES[name] ?? name) : `agent ${name}`;
};

/** The text a woken session gets: what arrived and how to read it, no bodies. */
export function wakeText(address, rows) {
  const ids = rows.map((r) => r.id);
  const senders = [...new Set(rows.map((r) => nameOf(r.from_addr)))];
  const instructions = rows.some((r) => r.kind === "instruction" || r.kind === "command");
  const what = `${rows.length} new ${instructions ? "message(s), including instructions," : "message(s)"}`;
  const [type, name] = address.split(":");
  if (type === "agent") {
    return (
      `The Message Bridge has ${what} for fleet agent ${name} (ids ${ids.join(", ")}) from ${senders.join(", ")}. ` +
      "Read them from the agent's bridge inbox, act on them, answer with send (reply_to the message id), and ack them."
    );
  }
  return (
    `The Message Bridge has ${what} for ${CHANNEL_NAMES[name] ?? name} (ids ${ids.join(", ")}) from ${senders.join(", ")}. ` +
    "Use the Message Bridge connector: read_messages with unread_only true, act on each one, answer with send_message " +
    "(reply_to a message id sends the answer back to whoever sent it), then ack_messages. " +
    "Message bodies are not in this text; read them from the bridge."
  );
}

/** Groups stored rows by the address they wake: connector channels and agents. */
export function wakeTargets(messages = [], fleet = []) {
  const by = new Map();
  const put = (addr, row) => {
    if (!by.has(addr)) by.set(addr, []);
    by.get(addr).push(row);
  };
  for (const m of messages) if (m?.direction === "inbound") put(`channel:${m.channel}`, m);
  for (const m of fleet) if (m?.direction === "inbound") put(`agent:${m.agent}`, m);
  return by;
}

async function fire(env, hook, address, rows) {
  const text = wakeText(address, rows);
  const headers = { "content-type": "application/json" };
  let body;
  if (hook.kind === "claude-routine") {
    headers.authorization = `Bearer ${hook.token}`;
    headers["anthropic-version"] = "2023-06-01";
    headers["anthropic-beta"] = "experimental-cc-routine-2026-04-01";
    body = { text };
  } else {
    if (hook.token) headers.authorization = `Bearer ${hook.token}`;
    body = {
      event: "message.created",
      address,
      message_ids: rows.map((r) => r.id),
      kind: rows.some((r) => r.kind === "instruction" || r.kind === "command") ? "instruction" : "message",
      from: [...new Set(rows.map((r) => r.from_addr ?? "drew"))],
      text,
    };
  }
  const egress = env.WAKE_EGRESS ?? globalThis;
  const res = await egress.fetch(hook.url, {
    method: "POST",
    headers,
    body: JSON.stringify(body),
    redirect: "manual",
    signal: AbortSignal.timeout(TIMEOUT_MS),
  });
  return res.status;
}

/**
 * Wakes every address that got a row and has a hook. Never throws: a
 * failed wake leaves the message unread for the next one. log(kind,
 * detail) records each attempt without URLs, tokens or bodies.
 */
export async function wakeAll(env, store, { messages = [], fleet = [] }, log = async () => {}) {
  const max = wakeMaxPerHour(env);
  for (const [address, rows] of wakeTargets(messages, fleet)) {
    let hook;
    try {
      hook = await store.wakeHookGet(address);
    } catch {
      continue;
    }
    if (!hook) continue;
    const ids = rows.map((r) => r.id);
    try {
      if (!(await store.wakeClaim(address, Date.now(), max))) {
        await log("wake_capped", { address, message_ids: ids, max_per_hour: max });
        continue;
      }
      const status = await fire(env, hook, address, rows);
      await log(status >= 200 && status < 300 ? "wake_fired" : "wake_failed", { address, hook: hook.kind, status, message_ids: ids });
    } catch (err) {
      await log("wake_failed", { address, hook: hook.kind, error: err?.name === "TimeoutError" ? "timeout" : "network", message_ids: ids });
    }
  }
}
