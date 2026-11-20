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
import { ackSelector, normalizeChannel, normalizeNew, normalizeQuery } from "./messages.js";
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
    this.sql.exec("CREATE INDEX IF NOT EXISTS messages_unread ON messages (direction, read_at)");
    this.sql.exec("CREATE INDEX IF NOT EXISTS messages_channel ON messages (channel, direction, read_at)");
  }

  add(msg) {
    const m = normalizeNew(msg);
    const ts = new Date().toISOString();
    return this.sql
      .exec(
        "INSERT INTO messages (ts, direction, source, sender, body, channel) VALUES (?, ?, ?, ?, ?, ?) RETURNING *",
        ts,
        m.direction,
        m.source,
        m.sender,
        m.body,
        m.channel,
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
  };
}

export default {
  async fetch(request, env) {
    return route(request, env, () => storeFor(env));
  },
};
