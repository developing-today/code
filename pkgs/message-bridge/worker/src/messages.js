// Message model shared by the MCP tools, the page and the JSON API.
//
// channel says which assistant the message is with: "claude" (the
// Claude custom connector) or "chatgpt" (the ChatGPT connector, which
// reaches this store through its own private Site and its own token).
// Both connectors run the same tools over the same store; each sees
// only its own channel.
//
// direction is from that assistant's point of view:
//   outbound  sent by the assistant through its connector (send_message)
//   inbound   sent to the assistant, from the page or the JSON API
// source says which door it came through: "mcp", "page" or "api".
// Inbound messages start unread; read_messages(mark_read) or
// ack_messages marks them read, so the assistant can tell what is new.
//
// kind is "message" (the default) or "instruction". An instruction is
// an inbound message Drew sends as direction for one or more agents; it
// is stored and targeted like any message, but the MCP tools hand it to
// the agent without its sender. The page and the JSON API (Drew's own
// views) still show the sender.

export const MAX_BODY = 16000;
export const MAX_SENDER = 80;
export const DEFAULT_LIMIT = 50;
export const MAX_LIMIT = 200;
export const DIRECTIONS = ["inbound", "outbound"];
export const SOURCES = ["mcp", "page", "api"];
export const CHANNELS = ["claude", "chatgpt"];
export const CHANNEL_NAMES = { claude: "Claude", chatgpt: "ChatGPT" };
export const KINDS = ["message", "instruction"];

export class InputError extends Error {}

/** Validates and normalizes a new message; throws InputError. */
export function normalizeNew({ body, sender, direction, source, channel = "claude", kind }) {
  if (typeof body !== "string" || body.trim() === "") throw new InputError("body must be a non-empty string");
  if (body.length > MAX_BODY) throw new InputError(`body is longer than ${MAX_BODY} characters`);
  if (!DIRECTIONS.includes(direction)) throw new InputError(`direction must be one of ${DIRECTIONS.join(", ")}`);
  if (!SOURCES.includes(source)) throw new InputError(`source must be one of ${SOURCES.join(", ")}`);
  if (!CHANNELS.includes(channel)) throw new InputError(`channel must be one of ${CHANNELS.join(", ")}`);
  const k = normalizeKind(kind);
  if (k === "instruction" && direction !== "inbound") throw new InputError("only inbound messages can be instructions");
  let s = null;
  if (sender !== undefined && sender !== null && sender !== "") {
    if (typeof sender !== "string") throw new InputError("sender must be a string");
    s = sender.trim().slice(0, MAX_SENDER) || null;
  }
  return { body, sender: s, direction, source, channel, kind: k };
}

/** Validates a new message's kind; empty means "message". Throws InputError. */
export function normalizeKind(kind) {
  if (kind === undefined || kind === null || kind === "") return "message";
  if (!KINDS.includes(kind)) throw new InputError(`kind must be one of ${KINDS.join(", ")}`);
  return kind;
}

/** Validates a kind filter: "all" or one kind; throws InputError. */
export function normalizeKindFilter(kind) {
  const k = kind ?? "all";
  if (k === "" || k === "all") return "all";
  if (!KINDS.includes(k)) throw new InputError(`kind must be all or one of ${KINDS.join(", ")}`);
  return k;
}

/**
 * What an agent sees of a stored message through its connector. An
 * instruction carries no sender: Drew sees who it was from on the page,
 * the agent only that it is an instruction.
 */
export function forAgent(m) {
  if (m?.kind !== "instruction") return m;
  const { sender: _hidden, ...rest } = m;
  return rest;
}

/** Validates a channel filter: "all" or one channel; throws InputError. */
export function normalizeChannel(channel) {
  const c = channel ?? "all";
  if (c !== "all" && !CHANNELS.includes(c)) throw new InputError(`channel must be all or one of ${CHANNELS.join(", ")}`);
  return c;
}

/**
 * The page and the JSON API take the recipients as `to`: one channel, a
 * list, or a comma-separated string. Default is Claude alone, so older
 * callers keep their meaning. Returns distinct channels in a fixed order.
 */
export function normalizeRecipients(to) {
  if (to === undefined || to === null || to === "") return ["claude"];
  const list = Array.isArray(to) ? to : String(to).split(",");
  const wanted = new Set();
  for (const raw of list) {
    if (typeof raw !== "string") throw new InputError("to must name claude, chatgpt or both");
    const c = raw.trim().toLowerCase();
    if (!CHANNELS.includes(c)) throw new InputError(`to must name ${CHANNELS.join(" or ")}, not ${JSON.stringify(raw)}`);
    wanted.add(c);
  }
  if (!wanted.size) throw new InputError("pick at least one of claude and chatgpt");
  return CHANNELS.filter((c) => wanted.has(c));
}

/** Validates list options; throws InputError. */
export function normalizeQuery(q = {}) {
  const direction = q.direction ?? "all";
  if (direction !== "all" && !DIRECTIONS.includes(direction)) {
    throw new InputError("direction must be inbound, outbound or all");
  }
  let sinceId = 0;
  if (q.since_id !== undefined && q.since_id !== null && q.since_id !== "") {
    sinceId = Number(q.since_id);
    if (!Number.isInteger(sinceId) || sinceId < 0) throw new InputError("since_id must be a non-negative integer");
  }
  let beforeId = null;
  if (q.before_id !== undefined && q.before_id !== null && q.before_id !== "") {
    beforeId = Number(q.before_id);
    if (!Number.isInteger(beforeId) || beforeId < 1) throw new InputError("before_id must be a positive integer");
  }
  let limit = DEFAULT_LIMIT;
  if (q.limit !== undefined && q.limit !== null && q.limit !== "") {
    limit = Number(q.limit);
    if (!Number.isInteger(limit) || limit < 1) throw new InputError("limit must be a positive integer");
    limit = Math.min(limit, MAX_LIMIT);
  }
  const order = q.order ?? "asc";
  if (order !== "asc" && order !== "desc") throw new InputError("order must be asc or desc");
  const channel = normalizeChannel(q.channel);
  const kind = normalizeKindFilter(q.kind);
  return { direction, sinceId, beforeId, limit, unreadOnly: Boolean(q.unread_only), order, channel, kind };
}

/**
 * In-memory store with the same interface as the Durable Object, used
 * by the tests and as the reference for the SQL in index.js.
 */
export const OAUTH_MAX_CLIENTS = 500;
export const OAUTH_MAX_EVENTS = 100;

export class MemoryStore {
  constructor(now = () => new Date().toISOString()) {
    this.rows = [];
    this.now = now;
    this.claimed = new Map();
    this.families = new Map();
    this.clients = new Map();
    this.events = [];
  }
  async oauthClientAdd(id, meta) {
    if (this.clients.size >= OAUTH_MAX_CLIENTS) return false;
    this.clients.set(id, { meta: JSON.parse(JSON.stringify(meta)), used: null });
    return true;
  }
  async oauthClientGet(id, touch = false) {
    const c = this.clients.get(id);
    if (!c) return null;
    if (touch) c.used = Date.now();
    return JSON.parse(JSON.stringify(c.meta));
  }
  async oauthLog(kind, detail) {
    this.events.unshift({ ts: this.now(), kind, ...detail });
    this.events.length = Math.min(this.events.length, OAUTH_MAX_EVENTS);
  }
  async oauthEvents() {
    return this.events.map((e) => ({ ...e }));
  }
  // OAuth bookkeeping (oauth.js): single-use codes and refresh-token
  // families, same semantics as the Durable Object's SQL.
  async oauthClaim(jti, exp) {
    if (this.claimed.has(jti)) return false;
    this.claimed.set(jti, exp);
    return true;
  }
  async oauthFamilyStart(fam, jti, exp) {
    this.families.set(fam, { jti, exp, revoked: false });
  }
  async oauthFamilyRotate(fam, jti, next, exp) {
    const f = this.families.get(fam);
    if (!f || f.revoked) return "unknown";
    if (f.jti !== jti) {
      f.revoked = true;
      return "replay";
    }
    Object.assign(f, { jti: next, exp });
    return "ok";
  }
  async add(msg) {
    const m = normalizeNew(msg);
    const row = { id: this.rows.length + 1, ts: this.now(), ...m, read_at: null };
    this.rows.push(row);
    return { ...row };
  }
  async list(query) {
    const q = normalizeQuery(query);
    let rows = this.rows.filter(
      (r) =>
        r.id > q.sinceId &&
        (q.beforeId === null || r.id < q.beforeId) &&
        (q.direction === "all" || r.direction === q.direction) &&
        (q.channel === "all" || r.channel === q.channel) &&
        (q.kind === "all" || r.kind === q.kind) &&
        (!q.unreadOnly || (r.direction === "inbound" && r.read_at === null)),
    );
    rows = q.order === "desc" ? rows.slice().reverse() : rows;
    return { messages: rows.slice(0, q.limit).map((r) => ({ ...r })), stats: await this.stats(q.channel) };
  }
  async ack({ up_to_id, ids, channel } = {}) {
    const wanted = ackSelector({ up_to_id, ids });
    const c = normalizeChannel(channel);
    const ts = this.now();
    let acked = 0;
    for (const r of this.rows) {
      if (r.direction === "inbound" && r.read_at === null && (c === "all" || r.channel === c) && wanted(r.id)) {
        r.read_at = ts;
        acked++;
      }
    }
    return { acked, stats: await this.stats(c) };
  }
  async stats(channel) {
    const c = normalizeChannel(channel);
    const rows = this.rows.filter((r) => c === "all" || r.channel === c);
    return {
      total: rows.length,
      inbound: rows.filter((r) => r.direction === "inbound").length,
      outbound: rows.filter((r) => r.direction === "outbound").length,
      unread_inbound: rows.filter((r) => r.direction === "inbound" && r.read_at === null).length,
      unread_instructions: rows.filter((r) => r.kind === "instruction" && r.read_at === null).length,
      last_id: rows.length ? rows[rows.length - 1].id : 0,
    };
  }
}

/** Builds the id predicate for ack; throws InputError on bad input. */
export function ackSelector({ up_to_id, ids } = {}) {
  const hasUpTo = up_to_id !== undefined && up_to_id !== null;
  const hasIds = Array.isArray(ids) && ids.length > 0;
  if (!hasUpTo && !hasIds) throw new InputError("give up_to_id or a non-empty ids list");
  if (hasUpTo && (!Number.isInteger(up_to_id) || up_to_id < 0)) {
    throw new InputError("up_to_id must be a non-negative integer");
  }
  if (hasIds && !ids.every((i) => Number.isInteger(i) && i > 0)) throw new InputError("ids must be positive integers");
  if (hasIds && ids.length > 1000) throw new InputError("at most 1000 ids at once");
  const set = new Set(hasIds ? ids : []);
  return (id) => (hasUpTo && id <= up_to_id) || set.has(id);
}
