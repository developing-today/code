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
import { OAUTH_CLIENT_IDLE, OAUTH_MAX_CLIENTS, OAUTH_MAX_EVENTS, OAUTH_PENDING_GRACE, OAUTH_REG_PER_SOURCE, ackSelector, normalizeChannel, normalizeNew, normalizeQuery, noteForInbox } from "./messages.js";
import {
  enrollTokenClaimSql,
  enrollTokenGetSql,
  enrollTokenMintSql,
  enrollTokenRevokeSql,
  fleetCall,
  fleetCredentialAuthSql,
  fleetCredentialPutSql,
  fleetCredentialRevokeSql,
  fleetCredentialStateSql,
  fleetOp,
  fleetSchema,
  getDistribution,
  recordDistribution,
} from "./fleet.js";
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
    // Rows from before connectors and agents could message each other all came from Drew.
    if (!cols.some((c) => c.name === "from_addr")) {
      this.sql.exec("ALTER TABLE messages ADD COLUMN from_addr TEXT");
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
    this.sql.exec("CREATE TABLE IF NOT EXISTS oauth_reg (source TEXT NOT NULL, ts INTEGER NOT NULL)");
    // ChatGPT event subscriptions (events.js): one row each, the
    // subscription (callback URL and signing secret included) as JSON.
    this.sql.exec(`CREATE TABLE IF NOT EXISTS event_sub (
      id TEXT PRIMARY KEY,
      channel TEXT NOT NULL,
      expires INTEGER NOT NULL,
      data TEXT NOT NULL
    )`);
    this.sql.exec(`CREATE TABLE IF NOT EXISTS oauth_event (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      ts TEXT NOT NULL,
      kind TEXT NOT NULL,
      detail TEXT NOT NULL
    )`);
    // Wake hooks (wake.js): one per address, and the last hour's wakes
    // for the per-address cap.
    this.sql.exec("CREATE TABLE IF NOT EXISTS wake_hook (address TEXT PRIMARY KEY, data TEXT NOT NULL)");
    this.sql.exec("CREATE TABLE IF NOT EXISTS wake_log (address TEXT NOT NULL, ts INTEGER NOT NULL)");
    // Fleet mailboxes (fleet.js): agents and their messages, own tables.
    fleetSchema(this.sql);
    // Class/name notes (notes.js): registered identities, token hashes
    // (never token values), per-identity inboxes and the delivery
    // counter behind the public wake signal.
    this.sql.exec(`CREATE TABLE IF NOT EXISTS note_identity (
      identity TEXT PRIMARY KEY,
      signal_counter INTEGER NOT NULL DEFAULT 0,
      created TEXT NOT NULL
    )`);
    this.sql.exec(`CREATE TABLE IF NOT EXISTS note_token (
      token_hash TEXT PRIMARY KEY,
      identity TEXT NOT NULL,
      can_send INTEGER NOT NULL DEFAULT 1,
      can_inbox INTEGER NOT NULL DEFAULT 1
    )`);
    this.sql.exec(`CREATE TABLE IF NOT EXISTS note_message (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      ts TEXT NOT NULL,
      recipient TEXT NOT NULL,
      sender TEXT NOT NULL,
      body TEXT NOT NULL,
      ack_at TEXT,
      kind TEXT NOT NULL DEFAULT 'note',
      reply_to INTEGER,
      batch TEXT
    )`);
    // Rows from before notes had kinds and threading (#14370, design
    // §4.4) and before sends carried their distribution batch (design
    // §4.1): the same ALTER-on-start migration fleet_message uses.
    const noteCols = this.sql.exec("PRAGMA table_info(note_message)").toArray();
    if (!noteCols.some((c) => c.name === "kind")) {
      this.sql.exec("ALTER TABLE note_message ADD COLUMN kind TEXT NOT NULL DEFAULT 'note'");
    }
    if (!noteCols.some((c) => c.name === "reply_to")) {
      this.sql.exec("ALTER TABLE note_message ADD COLUMN reply_to INTEGER");
    }
    if (!noteCols.some((c) => c.name === "batch")) {
      this.sql.exec("ALTER TABLE note_message ADD COLUMN batch TEXT");
    }
    this.sql.exec("CREATE INDEX IF NOT EXISTS note_message_inbox ON note_message (recipient, ack_at, id)");
  }

  /** One fleet mailbox operation over arguments fleetCall validated. */
  fleet(op, args) {
    return fleetOp(this.sql, op, args);
  }

  // Per-agent credentials and bootstrap enroll tokens (#14368): the
  // store only ever sees SHA-256 hashes, never a token value.

  fleetCredentialAuth(token_hash) {
    return fleetCredentialAuthSql(this.sql, token_hash);
  }

  fleetCredentialPut(rec) {
    return fleetCredentialPutSql(this.sql, rec);
  }

  fleetCredentialRevoke(agent) {
    return fleetCredentialRevokeSql(this.sql, agent, new Date().toISOString());
  }

  fleetCredentialState() {
    return fleetCredentialStateSql(this.sql);
  }

  enrollTokenMint(row) {
    return enrollTokenMintSql(this.sql, row);
  }

  enrollTokenGet(token_hash) {
    return enrollTokenGetSql(this.sql, token_hash);
  }

  enrollTokenClaim(token_hash, agent, now) {
    return enrollTokenClaimSql(this.sql, token_hash, agent, now);
  }

  enrollTokenRevoke(token_hash) {
    return enrollTokenRevokeSql(this.sql, token_hash, new Date().toISOString());
  }

  /** Deletes an identity's notes token(s); returns how many were revoked. */
  noteRevoke(identity) {
    return this.sql.exec("DELETE FROM note_token WHERE identity = ? RETURNING token_hash", identity).toArray().length;
  }

  /** Stores a new client; false when the table is full of clients worth keeping. */
  oauthClientAdd(id, meta) {
    const now = Math.floor(Date.now() / 1000);
    // Never-used registrations go after a day; used ones outlive their refresh tokens.
    this.sql.exec("DELETE FROM oauth_client WHERE (used IS NULL AND created < ?) OR used < ?", now - 86400, now - OAUTH_CLIENT_IDLE);
    const count = this.sql.exec("SELECT COUNT(*) AS n FROM oauth_client").one().n;
    if (count >= OAUTH_MAX_CLIENTS) {
      // Full: the oldest never-used registration past its grace makes room.
      const evicted = this.sql
        .exec(
          "DELETE FROM oauth_client WHERE id = (SELECT id FROM oauth_client WHERE used IS NULL AND created < ? ORDER BY created LIMIT 1) RETURNING id",
          now - OAUTH_PENDING_GRACE,
        )
        .toArray();
      if (evicted.length === 0) return false;
    }
    this.sql.exec("INSERT INTO oauth_client (id, meta, created) VALUES (?, ?, ?)", id, JSON.stringify(meta), now);
    return true;
  }

  /** Counts a registration from source (a hashed address); false past the per-source limit. */
  oauthRegisterAllowed(source) {
    const now = Math.floor(Date.now() / 1000);
    this.sql.exec("DELETE FROM oauth_reg WHERE ts < ?", now - OAUTH_PENDING_GRACE);
    const n = this.sql.exec("SELECT COUNT(*) AS n FROM oauth_reg WHERE source = ?", source).one().n;
    if (n >= OAUTH_REG_PER_SOURCE) return false;
    this.sql.exec("INSERT INTO oauth_reg (source, ts) VALUES (?, ?)", source, now);
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

  wakeHookGet(address) {
    const row = this.sql.exec("SELECT data FROM wake_hook WHERE address = ?", address).toArray()[0];
    return row ? JSON.parse(row.data) : null;
  }

  wakeHookPut(hook) {
    this.sql.exec("INSERT OR REPLACE INTO wake_hook (address, data) VALUES (?, ?)", hook.address, JSON.stringify(hook));
  }

  wakeHookDelete(address) {
    return this.sql.exec("DELETE FROM wake_hook WHERE address = ? RETURNING address", address).toArray().length > 0;
  }

  wakeHooks() {
    return this.sql.exec("SELECT data FROM wake_hook ORDER BY address").toArray().map((r) => JSON.parse(r.data));
  }

  /** Records a wake for address; false when it already had max in the last hour. */
  wakeClaim(address, now, max) {
    this.sql.exec("DELETE FROM wake_log WHERE ts <= ?", now - 3600_000);
    if (this.sql.exec("SELECT COUNT(*) AS n FROM wake_log WHERE address = ?", address).one().n >= max) return false;
    this.sql.exec("INSERT INTO wake_log (address, ts) VALUES (?, ?)", address, now);
    return true;
  }

  noteRegister({ identity, token_hash, can_send, can_inbox }) {
    this.sql.exec(
      "INSERT OR IGNORE INTO note_identity (identity, created) VALUES (?, ?)",
      identity,
      new Date().toISOString(),
    );
    this.sql.exec(
      "INSERT OR REPLACE INTO note_token (token_hash, identity, can_send, can_inbox) VALUES (?, ?, ?, ?)",
      token_hash,
      identity,
      can_send ? 1 : 0,
      can_inbox ? 1 : 0,
    );
    return { identity, can_send, can_inbox };
  }

  noteAuth(token_hash) {
    const row = this.sql
      .exec("SELECT identity, can_send, can_inbox FROM note_token WHERE token_hash = ?", token_hash)
      .toArray()[0];
    return row ? { identity: row.identity, can_send: !!row.can_send, can_inbox: !!row.can_inbox } : null;
  }

  noteIdentities() {
    return this.sql.exec("SELECT identity FROM note_identity ORDER BY identity").toArray().map((r) => r.identity);
  }

  noteDeliver({ from, recipients, note, kind = "note", reply_to = null, batch = null, pattern = null, limit_n = null, matched = null }) {
    const ts = new Date().toISOString();
    const notes = [];
    for (const recipient of recipients) {
      const known = this.sql.exec("SELECT 1 FROM note_identity WHERE identity = ?", recipient).toArray().length > 0;
      if (!known) continue;
      const row = this.sql
        .exec(
          "INSERT INTO note_message (ts, recipient, sender, body, kind, reply_to, batch) VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING id, ts, recipient, sender, body, kind, reply_to, batch",
          ts,
          recipient,
          from,
          note,
          kind,
          reply_to,
          batch,
        )
        .one();
      this.sql.exec("UPDATE note_identity SET signal_counter = signal_counter + 1 WHERE identity = ?", recipient);
      notes.push({
        id: row.id,
        recipient: row.recipient,
        from: row.sender,
        note: row.body,
        ts: row.ts,
        kind: row.kind,
        reply_to: row.reply_to,
        batch: row.batch,
      });
    }
    if (batch) {
      recordDistribution(this.sql, {
        batch,
        sender_addr: from,
        pattern,
        limit_n,
        matched: matched ?? notes.map((n) => n.recipient),
        delivered: notes.map((n) => n.recipient),
        ts,
      });
    }
    return { delivered: notes.length, recipients: notes.map((n) => n.recipient), notes };
  }

  /**
   * The sender of a note in `identity`'s inbox, or null. Reply routing
   * (design §4.4) reads it store-side: a reply answers the real sender
   * without the replier ever seeing who it was.
   */
  noteOrigin(identity, id) {
    const row = this.sql
      .exec("SELECT sender FROM note_message WHERE id = ? AND recipient = ?", id, identity)
      .toArray()[0];
    return row ? row.sender : null;
  }

  /**
   * Broadening a notes distribution: only the original sender, only
   * the matched-but-undelivered remainder (widened by extra_matched,
   * which the caller resolved from the call's new address). State
   * refusals come back as { error }, like fleet ops.
   */
  noteBroaden({ from, batch, extra_matched = [], pattern = null, limit = null }) {
    const dist = getDistribution(this.sql, batch);
    if (!dist) return { error: `no distribution recorded for batch ${batch}` };
    if (dist.sender_addr !== from) return { error: "only the original sender can broaden this distribution" };
    const sample = this.sql
      .exec("SELECT sender, body FROM note_message WHERE batch = ? ORDER BY id ASC LIMIT 1", batch)
      .toArray()[0];
    if (!sample) return { error: `batch ${batch} has no notes to broaden` };
    const matched = [...new Set([...dist.matched, ...extra_matched])].sort();
    const deliveredSet = new Set(dist.delivered);
    const pending = matched.filter((n) => !deliveredSet.has(n));
    const recipients = limit ? pending.slice(0, limit) : pending;
    const ts = new Date().toISOString();
    const notes = [];
    for (const recipient of recipients) {
      const known = this.sql.exec("SELECT 1 FROM note_identity WHERE identity = ?", recipient).toArray().length > 0;
      if (!known) continue;
      const row = this.sql
        .exec(
          "INSERT INTO note_message (ts, recipient, sender, body, batch) VALUES (?, ?, ?, ?, ?) RETURNING id, ts, recipient, sender, body, batch",
          ts,
          recipient,
          sample.sender,
          sample.body,
          batch,
        )
        .one();
      this.sql.exec("UPDATE note_identity SET signal_counter = signal_counter + 1 WHERE identity = ?", recipient);
      deliveredSet.add(recipient);
      notes.push({ id: row.id, recipient: row.recipient, from: row.sender, note: row.body, ts: row.ts, batch: row.batch });
    }
    const delivered = [...deliveredSet].sort();
    recordDistribution(this.sql, {
      batch,
      sender_addr: dist.sender_addr,
      pattern: pattern ?? dist.pattern,
      limit_n: limit ?? dist.limit_n,
      matched,
      delivered,
      ts,
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

  noteInbox(identity) {
    return this.sql
      .exec(
        "SELECT id, sender, body AS note, ts, kind, reply_to FROM note_message WHERE recipient = ? AND ack_at IS NULL ORDER BY id ASC",
        identity,
      )
      .toArray()
      .map((r) => noteForInbox(r));
  }

  noteAck(identity, { ids, up_to_id }) {
    const ts = new Date().toISOString();
    let acked = 0;
    if (up_to_id !== null) {
      acked += this.sql
        .exec(
          "UPDATE note_message SET ack_at = ? WHERE recipient = ? AND ack_at IS NULL AND id <= ? RETURNING id",
          ts,
          identity,
          up_to_id,
        )
        .toArray().length;
    }
    for (const id of ids) {
      acked += this.sql
        .exec(
          "UPDATE note_message SET ack_at = ? WHERE recipient = ? AND ack_at IS NULL AND id = ? RETURNING id",
          ts,
          identity,
          id,
        )
        .toArray().length;
    }
    return { acked };
  }

  noteSignal(identity) {
    const row = this.sql
      .exec("SELECT signal_counter FROM note_identity WHERE identity = ?", identity)
      .toArray()[0];
    return row ? { registered: true, counter: row.signal_counter } : { registered: false, counter: 0 };
  }

  /** Owner read over note_message (pipeline.js); sender/receiver are exact identities. */
  noteOwnerList(q = {}) {
    const where = ["id > ?"];
    const args = [q.since_id ?? 0];
    if (q.before_id !== undefined && q.before_id !== null) {
      where.push("id < ?");
      args.push(q.before_id);
    }
    if (q.unread_only) where.push("ack_at IS NULL");
    if (q.sender) {
      where.push("sender = ?");
      args.push(q.sender);
    }
    if (q.receiver) {
      where.push("recipient = ?");
      args.push(q.receiver);
    }
    const order = q.order === "desc" ? "DESC" : "ASC";
    return this.sql
      .exec(
        `SELECT id, ts, recipient, sender, body, ack_at FROM note_message WHERE ${where.join(" AND ")} ORDER BY id ${order} LIMIT ?`,
        ...args,
        q.limit ?? 50,
      )
      .toArray()
      .map((r) => ({ ...r }));
  }

  /** Owner directory of note identities (pipeline.js): scopes folded over each identity's tokens. */
  noteDirectory() {
    return this.sql
      .exec(
        `SELECT i.identity AS identity, i.signal_counter AS signal_counter, i.created AS created,
           COALESCE(MAX(t.can_send), 0) AS can_send, COALESCE(MAX(t.can_inbox), 0) AS can_inbox
         FROM note_identity i LEFT JOIN note_token t ON t.identity = i.identity
         GROUP BY i.identity ORDER BY i.identity`,
      )
      .toArray()
      .map((r) => ({
        identity: r.identity,
        can_send: !!r.can_send,
        can_inbox: !!r.can_inbox,
        signal_counter: r.signal_counter,
        created: r.created,
      }));
  }

  eventSubGet(id) {
    const row = this.sql.exec("SELECT data FROM event_sub WHERE id = ?", id).toArray()[0];
    return row ? JSON.parse(row.data) : null;
  }

  /** Stores or replaces a subscription; false when a new one would pass max live ones. */
  eventSubPut(sub, max) {
    this.sql.exec("DELETE FROM event_sub WHERE expires <= ?", Date.now());
    const exists = this.sql.exec("SELECT 1 FROM event_sub WHERE id = ?", sub.id).toArray().length > 0;
    if (!exists && this.sql.exec("SELECT COUNT(*) AS n FROM event_sub").one().n >= max) return false;
    this.sql.exec(
      "INSERT OR REPLACE INTO event_sub (id, channel, expires, data) VALUES (?, ?, ?, ?)",
      sub.id,
      sub.channel,
      sub.expires,
      JSON.stringify(sub),
    );
    return true;
  }

  eventSubDelete(id) {
    return this.sql.exec("DELETE FROM event_sub WHERE id = ? RETURNING id", id).toArray().length > 0;
  }

  eventSubsLive(channel, now) {
    this.sql.exec("DELETE FROM event_sub WHERE expires <= ?", now);
    return this.sql
      .exec("SELECT data FROM event_sub WHERE channel = ? ORDER BY id", channel)
      .toArray()
      .map((r) => JSON.parse(r.data));
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
        "INSERT INTO messages (ts, direction, source, sender, body, channel, kind, from_addr) VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING *",
        ts,
        m.direction,
        m.source,
        m.sender,
        m.body,
        m.channel,
        m.kind,
        m.from_addr,
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
    oauthRegisterAllowed: (source) => stub.oauthRegisterAllowed(source),
    oauthClientGet: (id, touch) => stub.oauthClientGet(id, touch),
    oauthLog: (kind, detail) => stub.oauthLog(kind, detail),
    oauthEvents: () => stub.oauthEvents(),
    eventSubGet: (id) => stub.eventSubGet(id),
    eventSubPut: (sub, max) => stub.eventSubPut(sub, max),
    eventSubDelete: (id) => stub.eventSubDelete(id),
    eventSubsLive: (channel, now) => stub.eventSubsLive(channel, now),
    wakeHookGet: (address) => stub.wakeHookGet(address),
    wakeHookPut: (hook) => stub.wakeHookPut(hook),
    wakeHookDelete: (address) => stub.wakeHookDelete(address),
    wakeHooks: () => stub.wakeHooks(),
    wakeClaim: (address, now, max) => stub.wakeClaim(address, now, max),
    noteRegister: (reg) => stub.noteRegister(reg),
    noteRevoke: (identity) => stub.noteRevoke(identity),
    noteAuth: (token_hash) => stub.noteAuth(token_hash),
    noteIdentities: () => stub.noteIdentities(),
    noteDeliver: (delivery) => stub.noteDeliver(delivery),
    noteBroaden: (broaden) => stub.noteBroaden(broaden),
    noteOrigin: (identity, id) => stub.noteOrigin(identity, id),
    noteInbox: (identity) => stub.noteInbox(identity),
    noteAck: (identity, sel) => stub.noteAck(identity, sel),
    noteSignal: (identity) => stub.noteSignal(identity),
    // Owner reads (pipeline.js): the query is validated in the Worker
    // by normalizePipelineQuery before these are called, like fleet.
    noteOwnerList: (q) => stub.noteOwnerList(q),
    noteDirectory: () => stub.noteDirectory(),
    fleet: (op, args) => fleetCall(stub, op, args),
    fleetCredentialAuth: (token_hash) => stub.fleetCredentialAuth(token_hash),
    fleetCredentialPut: (rec) => stub.fleetCredentialPut(rec),
    fleetCredentialRevoke: (agent) => stub.fleetCredentialRevoke(agent),
    fleetCredentialState: () => stub.fleetCredentialState(),
    enrollTokenMint: (row) => stub.enrollTokenMint(row),
    enrollTokenGet: (token_hash) => stub.enrollTokenGet(token_hash),
    enrollTokenClaim: (token_hash, agent, now) => stub.enrollTokenClaim(token_hash, agent, now),
    enrollTokenRevoke: (token_hash) => stub.enrollTokenRevoke(token_hash),
  };
}

export default {
  async fetch(request, env, ctx) {
    return route(request, env, () => storeFor(env), (p) => ctx.waitUntil(p));
  },
};
