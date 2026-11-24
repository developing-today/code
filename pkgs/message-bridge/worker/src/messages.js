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
// an inbound message for an assistant or agent to act on; it is stored
// and targeted like any message, but whoever receives it gets it
// without its sender: no sender label, no source and from_addr null, the
// same as an instruction Drew sends himself. Receivers treat every
// instruction as Drew's. The page and the JSON API (Drew's own views)
// still show who sent it.
//
// from_addr says who sent an inbound row when it wasn't Drew: another
// connector ("channel:claude", "channel:chatgpt") or a fleet agent
// ("agent:<name>"); see relay.js. It is null for everything Drew sends.
// The bridge keeps it on instructions too, so reply_to still sends the
// answer back to whoever passed the instruction on; the receiver just
// never sees it. Rows with from_addr come in through source "mcp" (a
// connector) or "fleet" (an agent).

export const MAX_BODY = 16000;
export const MAX_SENDER = 80;
export const DEFAULT_LIMIT = 50;
export const MAX_LIMIT = 200;
export const DIRECTIONS = ["inbound", "outbound"];
export const SOURCES = ["mcp", "page", "api", "fleet"];
export const CHANNELS = ["claude", "chatgpt"];
export const CHANNEL_NAMES = { claude: "Claude", chatgpt: "ChatGPT" };
export const KINDS = ["message", "instruction"];

export class InputError extends Error {}

/** Validates and normalizes a new message; throws InputError. */
export function normalizeNew({ body, sender, direction, source, channel = "claude", kind, from_addr }) {
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
  let from = null;
  if (from_addr !== undefined && from_addr !== null) {
    if (typeof from_addr !== "string" || !FROM_ADDR.test(from_addr)) {
      throw new InputError("from_addr must be channel:<name>, agent:<name> or service:<name>");
    }
    if (from_addr === `channel:${channel}`) throw new InputError("a connector can't send to its own channel");
    if (direction !== "inbound" || (source !== "mcp" && source !== "fleet")) {
      throw new InputError("from_addr is only for messages a connector or an agent sends");
    }
    from = from_addr;
  } else if (direction === "inbound" && (source === "mcp" || source === "fleet")) {
    throw new InputError("an inbound message from a connector or an agent must name from_addr");
  }
  return { body, sender: s, direction, source, channel, kind: k, from_addr: from };
}

const FROM_ADDR = /^(channel:(claude|chatgpt)|(agent|service):[a-z0-9][a-z0-9._-]{0,63})$/;

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
 * instruction carries no sender: no label, no source and from_addr null,
 * so it reads exactly like one Drew sent. Drew sees who it was from on
 * the page.
 */
export function forAgent(m) {
  if (m && !("from_addr" in m)) m = { ...m, from_addr: null };
  if (m?.kind !== "instruction") return m;
  const { sender: _sender, source: _source, ...rest } = m;
  return { ...rest, from_addr: null };
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
// A used client stays while its refresh tokens can (a year from last use).
export const OAUTH_CLIENT_IDLE = 366 * 86400;
// When the registry is full, a never-used registration older than this
// makes room, so a flood of registrations can't lock the owner out.
export const OAUTH_PENDING_GRACE = 600;
// Registrations one network address may make per grace window, so a single
// source can't keep the registry full; a real client registers once or twice.
export const OAUTH_REG_PER_SOURCE = 10;

export class MemoryStore {
  constructor(now = () => new Date().toISOString()) {
    this.rows = [];
    this.now = now;
    this.claimed = new Map();
    this.families = new Map();
    this.clients = new Map();
    this.registrations = [];
    this.events = [];
    this.subs = new Map();
    this.hooks = new Map();
    this.wakes = [];
    this.noteCounters = new Map();
    this.noteTokens = new Map();
    this.noteRows = [];
    this.noteNextId = 1;
    this.distributions = new Map();
    this.fleetCredentials = new Map();
    this.enrollTokens = new Map();
  }
  // Per-agent fleet credentials + bootstrap enroll tokens (fleet.js,
  // #14368), same semantics as the Durable Object: hashes only.
  async fleetCredentialAuth(token_hash) {
    const c = this.fleetCredentials.get(token_hash);
    return c && !c.revoked_at ? { agent: c.agent, scopes: [...c.scopes], addr: c.addr ?? null } : null;
  }
  async fleetCredentialPut({ token_hash, agent, scopes, created, addr = null }) {
    this.fleetCredentials.set(token_hash, {
      agent,
      scopes: [...scopes],
      created: created ?? this.now(),
      revoked_at: null,
      addr,
    });
    return { agent, scopes: [...scopes], addr };
  }
  async fleetCredentialRevoke(agent) {
    let n = 0;
    for (const c of this.fleetCredentials.values()) {
      if (c.agent === agent && !c.revoked_at) {
        c.revoked_at = this.now();
        n++;
      }
    }
    return n;
  }
  async enrollTokenMint(row) {
    this.enrollTokens.set(row.token_hash, { ...row, groups: [...row.groups], used_at: null, revoked_at: null });
    return { agent: row.agent, kind: row.kind, groups: [...row.groups], notes_identity: row.notes_identity, expires_at: row.expires_at, created: row.created };
  }
  async enrollTokenGet(token_hash) {
    const r = this.enrollTokens.get(token_hash);
    return r ? { ...r, groups: [...r.groups] } : null;
  }
  async enrollTokenClaim(token_hash, agent, now) {
    const r = this.enrollTokens.get(token_hash);
    if (!r || r.agent !== agent || r.used_at || r.revoked_at || !(r.expires_at > now)) return null;
    r.used_at = now;
    return { agent: r.agent, kind: r.kind, groups: [...r.groups], notes_identity: r.notes_identity };
  }
  async enrollTokenRevoke(token_hash) {
    const r = this.enrollTokens.get(token_hash);
    if (!r || r.used_at || r.revoked_at) return false;
    r.revoked_at = this.now();
    return true;
  }
  async noteRevoke(identity) {
    let n = 0;
    for (const [hash, t] of this.noteTokens) {
      if (t.identity === identity) {
        this.noteTokens.delete(hash);
        n++;
      }
    }
    return n;
  }
  // Class/name notes (notes.js), same semantics as the Durable Object.
  async noteRegister({ identity, token_hash, can_send, can_inbox }) {
    if (!this.noteCounters.has(identity)) this.noteCounters.set(identity, 0);
    this.noteTokens.set(token_hash, { identity, can_send, can_inbox });
    return { identity, can_send, can_inbox };
  }
  async noteAuth(token_hash) {
    const a = this.noteTokens.get(token_hash);
    return a ? { ...a } : null;
  }
  async noteIdentities() {
    return [...this.noteCounters.keys()].sort();
  }
  async noteDeliver({ from, recipients, note, batch = null, pattern = null, limit_n = null, matched = null }) {
    const notes = [];
    for (const recipient of recipients) {
      if (!this.noteCounters.has(recipient)) continue;
      const row = { id: this.noteNextId++, ts: this.now(), recipient, from, note, batch, ack_at: null, work_state: null, work_task_id: null };
      this.noteRows.push(row);
      this.noteCounters.set(recipient, this.noteCounters.get(recipient) + 1);
      notes.push({ id: row.id, recipient, from, note, ts: row.ts, batch });
    }
    if (batch) {
      this.distributions.set(batch, {
        batch,
        sender_addr: from,
        pattern,
        limit_n,
        matched: matched ?? notes.map((n) => n.recipient),
        delivered: notes.map((n) => n.recipient),
        ts: this.now(),
      });
    }
    return { delivered: notes.length, recipients: notes.map((n) => n.recipient), notes };
  }
  // Broadening a notes distribution (notes.js), same semantics as the
  // Durable Object: only the original sender, only the remainder. The
  // caller resolves a broaden call's new address to extra_matched.
  async noteBroaden({ from, batch, extra_matched = [], pattern = null, limit }) {
    const dist = this.distributions.get(batch);
    if (!dist) return { error: `no distribution recorded for batch ${batch}` };
    if (dist.sender_addr !== from) return { error: "only the original sender can broaden this distribution" };
    const sample = this.noteRows.find((r) => r.batch === batch);
    if (!sample) return { error: `batch ${batch} has no notes to broaden` };
    const matched = [...new Set([...dist.matched, ...extra_matched])].sort();
    const deliveredSet = new Set(dist.delivered);
    const pending = matched.filter((n) => !deliveredSet.has(n));
    const recipients = limit ? pending.slice(0, limit) : pending;
    const notes = [];
    for (const recipient of recipients) {
      if (!this.noteCounters.has(recipient)) continue;
      const row = { id: this.noteNextId++, ts: this.now(), recipient, from: sample.from, note: sample.note, batch, ack_at: null };
      this.noteRows.push(row);
      this.noteCounters.set(recipient, this.noteCounters.get(recipient) + 1);
      deliveredSet.add(recipient);
      notes.push({ id: row.id, recipient, from: row.from, note: row.note, ts: row.ts, batch });
    }
    const delivered = [...deliveredSet].sort();
    this.distributions.set(batch, {
      ...dist,
      pattern: pattern ?? dist.pattern,
      limit_n: limit ?? dist.limit_n,
      matched,
      delivered,
      ts: this.now(),
    });
    return {
      batch,
      matched: matched.length,
      delivered: notes.length,
      recipients: notes.map((n) => n.recipient),
      remaining: matched.length - delivered.length,
      notes,
    };
  }
  async noteInbox(identity) {
    return this.noteRows
      .filter((r) => r.recipient === identity && r.ack_at === null)
      .map((r) => ({ id: r.id, from: r.from, note: r.note, ts: r.ts, work_state: r.work_state, work_task_id: r.work_task_id }));
  }
  // Work-item lifecycle (design §4.6): the receiving identity moves its
  // own note forward; another identity's note is simply not found.
  // State-dependent refusals come back as { error }, like the fleet ops.
  async noteWork(identity, { id, state, task_id }) {
    const row = this.noteRows.find((r) => r.id === id && r.recipient === identity);
    if (!row) return { error: `note ${id} is not in ${identity}'s inbox` };
    const err = workTransitionError(row.work_state, state);
    if (err) return { error: err };
    row.work_state = state;
    row.work_task_id = task_id ?? row.work_task_id ?? null;
    return { id: row.id, work_state: row.work_state, work_task_id: row.work_task_id };
  }
  async noteAck(identity, { ids, up_to_id }) {
    const ts = this.now();
    let acked = 0;
    for (const r of this.noteRows) {
      if (r.recipient !== identity || r.ack_at !== null) continue;
      if ((up_to_id !== null && r.id <= up_to_id) || ids.includes(r.id)) {
        r.ack_at = ts;
        acked++;
      }
    }
    return { acked };
  }
  async noteSignal(identity) {
    return this.noteCounters.has(identity)
      ? { registered: true, counter: this.noteCounters.get(identity) }
      : { registered: false, counter: 0 };
  }
  // Owner reads (pipeline.js): every note row in the table's own shape
  // ({id, ts, recipient, sender, body, ack_at}); sender/receiver here
  // are exact identity matches, address matching lives in pipeline.js.
  async noteOwnerList(q = {}) {
    let rows = this.noteRows.filter(
      (r) =>
        (q.since_id === undefined || q.since_id === null || r.id > q.since_id) &&
        (q.before_id === undefined || q.before_id === null || r.id < q.before_id) &&
        (!q.unread_only || r.ack_at === null) &&
        (!q.sender || r.from === q.sender) &&
        (!q.receiver || r.recipient === q.receiver),
    );
    rows = q.order === "desc" ? rows.slice().reverse() : rows;
    return rows
      .slice(0, q.limit ?? 50)
      .map((r) => ({ id: r.id, ts: r.ts, recipient: r.recipient, sender: r.from, body: r.note, ack_at: r.ack_at, work_state: r.work_state, work_task_id: r.work_task_id }));
  }
  // Owner directory (pipeline.js): one row per registered identity,
  // scopes folded over its tokens (any token granting a scope grants
  // it), plus the wake-signal counter.
  async noteDirectory() {
    return [...this.noteCounters.keys()].sort().map((identity) => {
      let can_send = false;
      let can_inbox = false;
      for (const t of this.noteTokens.values()) {
        if (t.identity !== identity) continue;
        can_send = can_send || !!t.can_send;
        can_inbox = can_inbox || !!t.can_inbox;
      }
      return { identity, can_send, can_inbox, signal_counter: this.noteCounters.get(identity), created: null };
    });
  }
  // Wake hooks (wake.js), same semantics as the Durable Object.
  async wakeHookGet(address) {
    const h = this.hooks.get(address);
    return h ? { ...h } : null;
  }
  async wakeHookPut(hook) {
    this.hooks.set(hook.address, { ...hook });
  }
  async wakeHookDelete(address) {
    return this.hooks.delete(address);
  }
  async wakeHooks() {
    return [...this.hooks.values()].sort((a, b) => a.address.localeCompare(b.address)).map((h) => ({ ...h }));
  }
  async wakeClaim(address, now, max) {
    this.wakes = this.wakes.filter((w) => w.ts > now - 3600_000);
    if (this.wakes.filter((w) => w.address === address).length >= max) return false;
    this.wakes.push({ address, ts: now });
    return true;
  }
  // ChatGPT event subscriptions (events.js), same semantics as the Durable Object.
  async eventSubGet(id) {
    const s = this.subs.get(id);
    return s ? JSON.parse(JSON.stringify(s)) : null;
  }
  async eventSubPut(sub, max) {
    for (const [k, s] of this.subs) if (s.expires <= Date.now()) this.subs.delete(k);
    if (!this.subs.has(sub.id) && this.subs.size >= max) return false;
    this.subs.set(sub.id, JSON.parse(JSON.stringify(sub)));
    return true;
  }
  async eventSubDelete(id) {
    return this.subs.delete(id);
  }
  async eventSubsLive(channel, now) {
    for (const [k, s] of this.subs) if (s.expires <= now) this.subs.delete(k);
    return [...this.subs.values()].filter((s) => s.channel === channel).map((s) => JSON.parse(JSON.stringify(s)));
  }
  async oauthClientAdd(id, meta, now = Math.floor(Date.now() / 1000)) {
    for (const [k, c] of this.clients) {
      if (c.used === null ? c.created < now - 86400 : c.used < now - OAUTH_CLIENT_IDLE) this.clients.delete(k);
    }
    if (this.clients.size >= OAUTH_MAX_CLIENTS) {
      let oldest = null;
      for (const [k, c] of this.clients) {
        if (c.used === null && c.created < now - OAUTH_PENDING_GRACE && (!oldest || c.created < this.clients.get(oldest).created)) oldest = k;
      }
      if (!oldest) return false;
      this.clients.delete(oldest);
    }
    this.clients.set(id, { meta: JSON.parse(JSON.stringify(meta)), created: now, used: null });
    return true;
  }
  async oauthRegisterAllowed(source, now = Math.floor(Date.now() / 1000)) {
    this.registrations = this.registrations.filter((r) => r.ts >= now - OAUTH_PENDING_GRACE);
    if (this.registrations.filter((r) => r.source === source).length >= OAUTH_REG_PER_SOURCE) return false;
    this.registrations.push({ source, ts: now });
    return true;
  }
  async oauthClientGet(id, touch = false) {
    const c = this.clients.get(id);
    if (!c) return null;
    if (touch) c.used = Math.floor(Date.now() / 1000);
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

/**
 * The instruction work-item lifecycle (design §4.6, #14372), shared by
 * the fleet and notes surfaces. A received instruction is a work item:
 * the receiving agent records that it acted on it in-session (`acted`),
 * queued it as a named fleet-queue task (`queued`, with the task id),
 * and later that the work is finished (`done`). States move only
 * forward — `received` -> `acted` | `queued` -> `done` — and re-marking
 * the current state is a no-op, so a retry never fails. Rows store null
 * until first marked; null reads as `received`. Only the receiving
 * agent may move its own copy: the store ops scope every update to the
 * caller's own mailbox/inbox, so another agent's row is simply not
 * found. Connector-channel instructions carry no stored state in this
 * build-out; the pipeline derives `received`/`acted` from read state.
 */
export const WORK_STATES = ["received", "acted", "queued", "done"];
export const WORK_SETTABLE_STATES = ["acted", "queued", "done"];

/** The lifecycle state of a stored row: unmarked rows are at `received`. */
export function workStateOf(row) {
  return row?.work_state ?? "received";
}

/**
 * Validates one work mark `{id, state, task_id?}` (the fleet verb body
 * and the notes verb body share the shape); throws InputError.
 * `queued` must name the queue task the instruction became.
 */
export function normalizeWorkMark(data) {
  if (!data || typeof data !== "object" || Array.isArray(data)) throw new InputError("body must be a JSON object");
  if (!Number.isInteger(data.id) || data.id < 1) throw new InputError("id must be a positive integer");
  if (!WORK_SETTABLE_STATES.includes(data.state)) {
    throw new InputError(`state must be one of ${WORK_SETTABLE_STATES.join(", ")}`);
  }
  const task_id = data.task_id === undefined || data.task_id === null ? null : data.task_id;
  if (task_id !== null && (!Number.isInteger(task_id) || task_id < 1)) {
    throw new InputError("task_id must be a positive integer");
  }
  if (data.state === "queued" && task_id === null) {
    throw new InputError("state queued needs task_id: the fleet queue task the instruction became");
  }
  return { id: data.id, state: data.state, task_id };
}

/**
 * The forward-only rule: null when `next` may follow `current`, else
 * the refusal text. The graph is exactly the design's: from `received`
 * an instruction is acted on or queued; from either it completes to
 * `done`. Sideways steps (acted <-> queued), skipping the middle
 * (received -> done) and every backward step are refused; re-marking
 * the current state is a no-op, so a retry never fails.
 */
export function workTransitionError(current, next) {
  const cur = current ?? "received";
  if (next === cur) return null;
  const forward =
    (cur === "received" && (next === "acted" || next === "queued")) ||
    ((cur === "acted" || cur === "queued") && next === "done");
  return forward
    ? null
    : `work_state moves only forward (received -> acted | queued -> done); ${cur} -> ${next} is not a step forward`;
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
