// Message bridge Worker: a remote MCP server Claude can add as a custom
// connector, the same MCP server for the ChatGPT connector (reached
// through its private Site with a token of its own), plus a page where
// Drew sends a message to either or both and sees the history. Every
// message sent or received is stored in one SQLite-backed Durable
// Object, so writes never race; each row names its channel.
//
// Routes and auth: see routes.js and auth.js. The access token is the
// Worker secret ACCESS_TOKEN; it is never in git.
import { DurableObject } from "cloudflare:workers";
import { OAUTH_MAX_CLIENTS, OAUTH_MAX_EVENTS, ackSelector, normalizeChannel, normalizeNew, normalizeQuery } from "./messages.js";
import { route } from "./routes.js";

export class MessageStore extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.sql = ctx.storage.sql;
    this.sql.exec(`CREATE TABLE IF NOT EXISTS messages (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      ts TEXT NOT NULL,
      direction TEXT NOT NULL CHECK (direction IN ('inbound', 'outbound')),
      source TEXT NOT NULL,
      sender TEXT,
      body TEXT NOT NULL,
      read_at TEXT
    )`);
    // Rows from before the ChatGPT connector were all Claude's.
    const cols = this.sql.exec("PRAGMA table_info(messages)").toArray();
    if (!cols.some((c) => c.name === "channel")) {
      this.sql.exec("ALTER TABLE messages ADD COLUMN channel TEXT NOT NULL DEFAULT 'claude'");
    }
    // Rows from before instructions were all plain messages.
    if (!cols.some((c) => c.name === "kind")) {
      this.sql.exec("ALTER TABLE messages ADD COLUMN kind TEXT NOT NULL DEFAULT 'message'");
    }
    this.sql.exec("CREATE INDEX IF NOT EXISTS messages_unread ON messages (direction, read_at)");
    this.sql.exec("CREATE INDEX IF NOT EXISTS messages_channel ON messages (channel, direction, read_at)");
    // OAuth (oauth.js): used authorization codes, and one row per
    // refresh-token family holding the only refresh token still valid.
    this.sql.exec("CREATE TABLE IF NOT EXISTS oauth_claimed (jti TEXT PRIMARY KEY, exp INTEGER NOT NULL)");
    this.sql.exec(`CREATE TABLE IF NOT EXISTS oauth_family (
      fam TEXT PRIMARY KEY,
      jti TEXT NOT NULL,
      exp INTEGER NOT NULL,
      revoked INTEGER NOT NULL DEFAULT 0
    )`);
    // Registered OAuth clients, keyed by a short random id, and the last
    // few sign-in events (no secrets) for diagnosing a failed connect.
    this.sql.exec(`CREATE TABLE IF NOT EXISTS oauth_client (
      id TEXT PRIMARY KEY,
      meta TEXT NOT NULL,
      created INTEGER NOT NULL,
      used INTEGER
    )`);
    this.sql.exec(`CREATE TABLE IF NOT EXISTS oauth_event (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      ts TEXT NOT NULL,
      kind TEXT NOT NULL,
      detail TEXT NOT NULL
    )`);
  }

  /** Stores a new client; false when the table is full. */
  oauthClientAdd(id, meta) {
    const now = Math.floor(Date.now() / 1000);
    // Never-used registrations go after a day, idle ones after 90 days.
    this.sql.exec("DELETE FROM oauth_client WHERE (used IS NULL AND created < ?) OR used < ?", now - 86400, now - 90 * 86400);
    const count = this.sql.exec("SELECT COUNT(*) AS n FROM oauth_client").one().n;
    if (count >= OAUTH_MAX_CLIENTS) return false;
    this.sql.exec("INSERT INTO oauth_client (id, meta, created) VALUES (?, ?, ?)", id, JSON.stringify(meta), now);
    return true;
  }

  /** The client's metadata, or null; touch marks it used. */
  oauthClientGet(id, touch = false) {
    const row = this.sql.exec("SELECT meta FROM oauth_client WHERE id = ?", id).toArray()[0];
    if (!row) return null;
    if (touch) this.sql.exec("UPDATE oauth_client SET used = ? WHERE id = ?", Math.floor(Date.now() / 1000), id);
    return JSON.parse(row.meta);
  }

  oauthLog(kind, detail) {
    this.sql.exec("INSERT INTO oauth_event (ts, kind, detail) VALUES (?, ?, ?)", new Date().toISOString(), kind, JSON.stringify(detail));
    this.sql.exec("DELETE FROM oauth_event WHERE id <= (SELECT MAX(id) FROM oauth_event) - ?", OAUTH_MAX_EVENTS);
  }

  oauthEvents() {
    return this.sql
      .exec("SELECT ts, kind, detail FROM oauth_event ORDER BY id DESC")
      .toArray()
      .map((r) => ({ ts: r.ts, kind: r.kind, ...JSON.parse(r.detail) }));
  }

  /** Marks a code used; false when it already was. */
  oauthClaim(jti, exp) {
    const now = Math.floor(Date.now() / 1000);
    this.sql.exec("DELETE FROM oauth_claimed WHERE exp < ?", now);
    return this.sql.exec("INSERT OR IGNORE INTO oauth_claimed (jti, exp) VALUES (?, ?) RETURNING jti", jti, exp).toArray()
      .length === 1;
  }

  oauthFamilyStart(fam, jti, exp) {
    const now = Math.floor(Date.now() / 1000);
    this.sql.exec("DELETE FROM oauth_family WHERE exp < ?", now);
    this.sql.exec("INSERT OR REPLACE INTO oauth_family (fam, jti, exp, revoked) VALUES (?, ?, ?, 0)", fam, jti, exp);
  }

  /** "ok" after rotating; "replay" revokes the family; "unknown" otherwise. */
  oauthFamilyRotate(fam, jti, next, exp) {
    const row = this.sql.exec("SELECT jti, revoked FROM oauth_family WHERE fam = ?", fam).toArray()[0];
    if (!row || row.revoked) return "unknown";
    if (row.jti !== jti) {
      this.sql.exec("UPDATE oauth_family SET revoked = 1 WHERE fam = ?", fam);
      return "replay";
    }
    this.sql.exec("UPDATE oauth_family SET jti = ?, exp = ? WHERE fam = ?", next, exp, fam);
    return "ok";
  }

  add(msg) {
    const m = normalizeNew(msg);
    const ts = new Date().toISOString();
    return this.sql
      .exec(
        "INSERT INTO messages (ts, direction, source, sender, body, channel, kind) VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING *",
        ts,
        m.direction,
        m.source,
        m.sender,
        m.body,
        m.channel,
        m.kind,
      )
      .one();
  }

  list(query) {
    const q = normalizeQuery(query);
    const where = ["id > ?"];
    const args = [q.sinceId];
    if (q.beforeId !== null) {
      where.push("id < ?");
      args.push(q.beforeId);
    }
    if (q.direction !== "all") {
      where.push("direction = ?");
      args.push(q.direction);
    }
    if (q.unreadOnly) where.push("direction = 'inbound' AND read_at IS NULL");
    if (q.channel !== "all") {
      where.push("channel = ?");
      args.push(q.channel);
    }
    if (q.kind !== "all") {
      where.push("kind = ?");
      args.push(q.kind);
    }
    const order = q.order === "desc" ? "DESC" : "ASC";
    const messages = this.sql
      .exec(`SELECT * FROM messages WHERE ${where.join(" AND ")} ORDER BY id ${order} LIMIT ?`, ...args, q.limit)
      .toArray();
    return { messages, stats: this.stats(q.channel) };
  }

  ack({ up_to_id, ids, channel } = {}) {
    ackSelector({ up_to_id, ids }); // validates
    const c = normalizeChannel(channel);
    const scope = c === "all" ? "" : " AND channel = ?";
    const scopeArgs = c === "all" ? [] : [c];
    const ts = new Date().toISOString();
    // Count with RETURNING: rowsWritten also counts index writes.
    let acked = 0;
    if (up_to_id !== undefined && up_to_id !== null) {
      acked += this.sql.exec(
        `UPDATE messages SET read_at = ? WHERE direction = 'inbound' AND read_at IS NULL AND id <= ?${scope} RETURNING id`,
        ts,
        up_to_id,
        ...scopeArgs,
      ).toArray().length;
    }
    for (const id of Array.isArray(ids) ? ids : []) {
      acked += this.sql.exec(
        `UPDATE messages SET read_at = ? WHERE direction = 'inbound' AND read_at IS NULL AND id = ?${scope} RETURNING id`,
        ts,
        id,
        ...scopeArgs,
      ).toArray().length;
    }
    return { acked, stats: this.stats(c) };
  }

  stats(channel) {
    const c = normalizeChannel(channel);
    return this.sql
      .exec(
        `SELECT COUNT(*) AS total,
          COALESCE(SUM(direction = 'inbound'), 0) AS inbound,
          COALESCE(SUM(direction = 'outbound'), 0) AS outbound,
          COALESCE(SUM(direction = 'inbound' AND read_at IS NULL), 0) AS unread_inbound,
          COALESCE(SUM(kind = 'instruction' AND read_at IS NULL), 0) AS unread_instructions,
          COALESCE(MAX(id), 0) AS last_id
        FROM messages${c === "all" ? "" : " WHERE channel = ?"}`,
        ...(c === "all" ? [] : [c]),
      )
      .one();
  }
}

/**
 * Validates in the Worker before calling the Durable Object, so input
 * errors keep their InputError type (it doesn't survive the RPC hop)
 * and come back as 400s / tool errors instead of 500s.
 */
function storeFor(env) {
  const stub = env.MESSAGES.get(env.MESSAGES.idFromName("default"));
  return {
    add: (msg) => stub.add(normalizeNew(msg)),
    list: (query) => (normalizeQuery(query), stub.list(query)),
    ack: (sel) => (ackSelector(sel ?? {}), normalizeChannel(sel?.channel), stub.ack(sel)),
    stats: (channel) => (normalizeChannel(channel), stub.stats(channel)),
    oauthClaim: (jti, exp) => stub.oauthClaim(jti, exp),
    oauthFamilyStart: (fam, jti, exp) => stub.oauthFamilyStart(fam, jti, exp),
    oauthFamilyRotate: (fam, jti, next, exp) => stub.oauthFamilyRotate(fam, jti, next, exp),
    oauthClientAdd: (id, meta) => stub.oauthClientAdd(id, meta),
    oauthClientGet: (id, touch) => stub.oauthClientGet(id, touch),
    oauthLog: (kind, detail) => stub.oauthLog(kind, detail),
    oauthEvents: () => stub.oauthEvents(),
  };
}

export default {
  async fetch(request, env) {
    return route(request, env, () => storeFor(env));
  },
};
