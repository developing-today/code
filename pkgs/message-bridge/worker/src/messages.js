// Message model shared by the MCP tools, the page and the JSON API.
//
// direction is from the connector's point of view:
//   outbound  sent by Claude through the connector (send_message)
//   inbound   sent to Claude, from the page or the JSON API
// source says which door it came through: "mcp", "page" or "api".
// Inbound messages start unread; read_messages(mark_read) or
// ack_messages marks them read, so Claude can tell what is new.

export const MAX_BODY = 16000;
export const MAX_SENDER = 80;
export const DEFAULT_LIMIT = 50;
export const MAX_LIMIT = 200;
export const DIRECTIONS = ["inbound", "outbound"];
export const SOURCES = ["mcp", "page", "api"];

export class InputError extends Error {}

/** Validates and normalizes a new message; throws InputError. */
export function normalizeNew({ body, sender, direction, source }) {
  if (typeof body !== "string" || body.trim() === "") throw new InputError("body must be a non-empty string");
  if (body.length > MAX_BODY) throw new InputError(`body is longer than ${MAX_BODY} characters`);
  if (!DIRECTIONS.includes(direction)) throw new InputError(`direction must be one of ${DIRECTIONS.join(", ")}`);
  if (!SOURCES.includes(source)) throw new InputError(`source must be one of ${SOURCES.join(", ")}`);
  let s = null;
  if (sender !== undefined && sender !== null && sender !== "") {
    if (typeof sender !== "string") throw new InputError("sender must be a string");
    s = sender.trim().slice(0, MAX_SENDER) || null;
  }
  return { body, sender: s, direction, source };
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
  return { direction, sinceId, beforeId, limit, unreadOnly: Boolean(q.unread_only), order };
}

/**
 * In-memory store with the same interface as the Durable Object, used
 * by the tests and as the reference for the SQL in index.js.
 */
export class MemoryStore {
  constructor(now = () => new Date().toISOString()) {
    this.rows = [];
    this.now = now;
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
        (!q.unreadOnly || (r.direction === "inbound" && r.read_at === null)),
    );
    rows = q.order === "desc" ? rows.slice().reverse() : rows;
    return { messages: rows.slice(0, q.limit).map((r) => ({ ...r })), stats: await this.stats() };
  }
  async ack({ up_to_id, ids } = {}) {
    const wanted = ackSelector({ up_to_id, ids });
    const ts = this.now();
    let acked = 0;
    for (const r of this.rows) {
      if (r.direction === "inbound" && r.read_at === null && wanted(r.id)) {
        r.read_at = ts;
        acked++;
      }
    }
    return { acked, stats: await this.stats() };
  }
  async stats() {
    return {
      total: this.rows.length,
      inbound: this.rows.filter((r) => r.direction === "inbound").length,
      outbound: this.rows.filter((r) => r.direction === "outbound").length,
      unread_inbound: this.rows.filter((r) => r.direction === "inbound" && r.read_at === null).length,
      last_id: this.rows.length ? this.rows[this.rows.length - 1].id : 0,
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
