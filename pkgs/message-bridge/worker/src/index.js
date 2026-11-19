// Message bridge Worker: a remote MCP server Claude can add as a custom
// connector, plus a page where Drew sends messages and sees the
// history. Every message sent or received is stored in one
// SQLite-backed Durable Object, so writes never race.
//
// Routes and auth: see routes.js and auth.js. The access token is the
// Worker secret ACCESS_TOKEN; it is never in git.
import { DurableObject } from "cloudflare:workers";
import { ackSelector, normalizeNew, normalizeQuery } from "./messages.js";
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
    this.sql.exec("CREATE INDEX IF NOT EXISTS messages_unread ON messages (direction, read_at)");
  }

  add(msg) {
    const m = normalizeNew(msg);
    const ts = new Date().toISOString();
    return this.sql
      .exec(
        "INSERT INTO messages (ts, direction, source, sender, body) VALUES (?, ?, ?, ?, ?) RETURNING *",
        ts,
        m.direction,
        m.source,
        m.sender,
        m.body,
      )
      .one();
  }

  list(query) {
    const q = normalizeQuery(query);
    const where = ["id > ?"];
    const args = [q.sinceId];
    if (q.direction !== "all") {
      where.push("direction = ?");
      args.push(q.direction);
    }
    if (q.unreadOnly) where.push("direction = 'inbound' AND read_at IS NULL");
    const order = q.order === "desc" ? "DESC" : "ASC";
    const messages = this.sql
      .exec(`SELECT * FROM messages WHERE ${where.join(" AND ")} ORDER BY id ${order} LIMIT ?`, ...args, q.limit)
      .toArray();
    return { messages, stats: this.stats() };
  }

  ack({ up_to_id, ids } = {}) {
    ackSelector({ up_to_id, ids }); // validates
    const ts = new Date().toISOString();
    // Count with RETURNING: rowsWritten also counts index writes.
    let acked = 0;
    if (up_to_id !== undefined && up_to_id !== null) {
      acked += this.sql.exec(
        "UPDATE messages SET read_at = ? WHERE direction = 'inbound' AND read_at IS NULL AND id <= ? RETURNING id",
        ts,
        up_to_id,
      ).toArray().length;
    }
    for (const id of Array.isArray(ids) ? ids : []) {
      acked += this.sql.exec(
        "UPDATE messages SET read_at = ? WHERE direction = 'inbound' AND read_at IS NULL AND id = ? RETURNING id",
        ts,
        id,
      ).toArray().length;
    }
    return { acked, stats: this.stats() };
  }

  stats() {
    return this.sql
      .exec(
        `SELECT COUNT(*) AS total,
          COALESCE(SUM(direction = 'inbound'), 0) AS inbound,
          COALESCE(SUM(direction = 'outbound'), 0) AS outbound,
          COALESCE(SUM(direction = 'inbound' AND read_at IS NULL), 0) AS unread_inbound,
          COALESCE(MAX(id), 0) AS last_id
        FROM messages`,
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
    ack: (sel) => (ackSelector(sel), stub.ack(sel)),
    stats: () => stub.stats(),
  };
}

export default {
  async fetch(request, env) {
    return route(request, env, () => storeFor(env));
  },
};
