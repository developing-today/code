// Fleet mailboxes: the fleet binary (`fleet bridge ...`) and the agents
// that run it (Muse lanes and any other fleet agent) talk to central
// through the bridge.
//
// Every agent has a mailbox named after it. Central (the owner: Drew's
// page, the JSON API, or `fleet bridge send` on the laptop, all with the
// owner token) sends messages and commands to one agent, a group of
// agents or every agent; agents read their own mailbox, mark what they
// handled, and post replies and reports back. Rows live in their own
// tables in the same Durable Object, so nothing here touches the
// Claude/ChatGPT message rows.
//
// Routes (area "fleet" on the agent side, "fleet" under the owner API):
//
//   POST /fleet/<fleet token>/hello    {agent, groups?}: register or
//                                      refresh the agent and its groups
//   GET  /fleet/<fleet token>/inbox    ?agent=&groups=&kind=&since_id=
//                                      &unread_only=&mark_read=&limit=: the
//                                      agent's inbound rows, oldest first
//   POST /fleet/<fleet token>/ack      {agent, up_to_id | ids}: mark read
//   POST /fleet/<fleet token>/work     {agent, id, state, task_id?}:
//                                      record the work-item lifecycle of
//                                      one of the agent's own inbound
//                                      rows (design §4.6): state is
//                                      acted | queued | done, moving
//                                      only forward from received;
//                                      queued names its queue task id
//   POST /fleet/<fleet token>/send     {agent, body, to?, kind?, reply_to?}:
//                                      a message from the agent. With no
//                                      to it goes to central (or, with
//                                      reply_to, back to whoever sent that
//                                      row); with to, it goes to any
//                                      address relay.js knows: drew,
//                                      claude, chatgpt, connectors,
//                                      agent:<n>, group:<g>, agents, all.
//                                      reply_to marks that row read.
//   GET  /api/<owner>/fleet/agents     registered agents, groups, last seen,
//                                      unread counts
//   GET  /api/<owner>/fleet/messages   ?agent=&direction=&kind=&since_id=
//                                      &before_id=&limit=&order=
//   POST /api/<owner>/fleet/send       {body, to, kind?, sender?}: one
//                                      inbound row per recipient
//
// Offers: claim-style distribution (design §4.2, #14371). A fan-out
// send cannot express "the next task goes to the next N agents who
// pick it up", so a sender can instead post an offer — stored once in
// fleet_offer, never fanned out:
//
//   POST /api/<owner>/fleet/offer      {body, to, kind?, slots, sender?,
//                                      ttl_seconds?}: post an offer
//   GET  /api/<owner>/fleet/offers     every offer with its claims
//                                      (agent, time, delivered row)
//   POST /fleet/<fleet token>/offer    {agent, body, to, kind?, slots,
//                                      ttl_seconds?}: an agent posts one
//   GET  /fleet/<fleet token>/offers   ?agent=&groups=: the open offers
//                                      this agent may claim (the inbox
//                                      response carries the same list in
//                                      its "offers" section)
//   POST /fleet/<fleet token>/claim    {agent, offer_id}: claim one
//
// Eligibility is the offer's address (a name, glob, group or all)
// matched against the registry at claim time. A claim is one atomic
// store op: while slots remain it delivers a normal mailbox row to the
// claimer — on the offer's batch, so the claimer's reply threads onto
// the batch and the owner reads claims and replies together (the
// offer list above, and the pipeline filtered by batch) — and records
// the claim in fleet_offer_claim. Past the last slot a claim gets a
// definite "filled" answer and no row; an expired offer (ttl_seconds)
// answers "expired", a second claim by the same agent "already
// claimed", and an agent outside the address can neither see nor
// claim the offer. A command offer reaches its claimers with no
// sender, like every command (commandForAgent); the owner still sees
// who posted it.
//
// Per-agent credentials and bootstrap enrollment (#14368, design §4.5):
// the routes above also answer to a per-agent credential in place of
// the fleet token (/fleet/<credential>/...). A credential is a random
// value whose SHA-256 hash alone is stored (fleet_credential, the
// notes-token model); it acts only as its own agent — hello/inbox/ack
// for its own name (scope "inbox"), send with from_addr fixed to
// agent:<its name> (scope "send") — and every refusal is the usual
// plain 404. The owner mints single-use, expiring bootstrap tokens
// bound at mint time to a name, kind and groups (enroll_token, hash
// only):
//
//   POST /api/<owner>/enroll-tokens    {agent, kind?, groups?,
//                                      notes_identity?, ttl_seconds?}:
//                                      mints a bootstrap token; the
//                                      value is returned once, here
//   POST /api/<owner>/enroll-tokens/revoke  {bootstrap_token | token_hash}:
//                                      revokes an unused bootstrap token
//   POST /enroll/exchange              {bootstrap_token, agent}: burns
//                                      the token, registers the mailbox
//                                      (and the notes identity, when
//                                      minted with one) and returns the
//                                      agent's fleet credential and
//                                      notes token, once
//   POST /api/<owner>/fleet/credentials/revoke  {agent}: revokes every
//                                      live fleet credential of an agent
//   POST /api/<owner>/notes-tokens/revoke  {identity}: revokes an
//                                      identity's notes token(s)
//
// Service credentials (#14373, design §4.8): the owner can also mint a
// credential directly, with the scopes it names:
//
//   POST /api/<owner>/fleet/credentials/mint  {agent, scopes?, addr?}:
//                                      mints a credential outside the
//                                      enroll flow; the value is
//                                      returned once, here
//
// A credential minted with an `addr` sends as that address instead of
// agent:<its name>. The one addr form is service:<name>, and the name
// must be the credential's own: it is how a platform service that is
// not an agent — the PM stack, from_addr service:pm-stack — holds a
// scoped credential of its own (send + the pipeline read below)
// instead of relaying through a machine's shared token. Provenance
// tells such rows apart from owner-sent (from_addr null) and
// agent-sent (agent:<name>) ones in the owner's log; a command still
// reaches its agent with no sender, as every command does.
//
// A third scope, "read", opens the owner's views to the credential
// that holds it, at the agent-side address:
//
//   GET  /fleet/<credential>/agents    the registered agents (the
//                                      ownerRoute "agents" store op)
//   GET  /fleet/<credential>/messages  the fleet message log (the
//                                      ownerRoute "messages" store op)
//   GET  /fleet/<credential>/pipeline  the unified pipeline feed
//                                      (pipeline.js, #14374)
//   GET  /fleet/<credential>/directory the unified agents list —
//                                      fleet registry + note
//                                      identities (pipeline.js)
//   GET  /fleet/<credential>/credentials  per-agent credential state
//                                      (active / revoked; never a
//                                      hash), as the owner's own
//                                      GET /api/<owner>/fleet/credentials
//
// Enroll-minted agent credentials never carry "read" (the exchange
// mints CREDENTIAL_SCOPES only): the pipeline stays the owner's and
// its services', not every agent's.
//
// `to` names recipients as "agent:<name>" (or a bare name), "group:<name>"
// (every registered agent in that group) or "all" (every registered
// agent); a list or a comma-separated string mixes them. An agent name
// containing "*" is a glob over registered agents (agent:f*), resolved
// in name order; an optional `limit: N` on a send delivers to only the
// first N matches, and the send returns a match report {matched,
// delivered, recipients, remaining}. Every send records a distribution
// (bridge_distribution: batch, sender, pattern, limit, matched and
// delivered sets); a follow-up send with `broaden: <batch>` and an
// optional new limit or pattern delivers the original body to exactly
// the matched-but-undelivered remainder. The notes surface (notes.js)
// follows the same matching rule per identity segment. kind is
// "message" (default) or "command". Central, a connector or another
// agent (relay.js) can send either; a row another assistant or agent
// sent carries its address in from_addr ("channel:chatgpt",
// "agent:muse-3"), and from_addr is null when central sent it. A command
// reaches the agent's inbox without its sender (commandForAgent), so the
// agent treats every command as Drew's; Drew's log still shows it.
//
// The fleet token is derived from ACCESS_TOKEN (HMAC-SHA256 over a fixed
// label, base64url), so it needs no Worker secret of its own and rotating
// the access token rotates it too. It can't open the page, the owner API
// or either MCP connector. FLEET_TOKEN, when set as a Worker secret of at
// least 32 characters distinct from the other two, replaces the derived
// one. Any holder of the fleet token can read any agent's mailbox and send
// as any agent: an agent's name is what it says it is, like a queue
// claim's worker name, so from_addr "agent:<n>" is a claim at the fleet
// token's trust level, not a proof. Because receivers treat every
// instruction as Drew's, the fleet token can send what reads as Drew's
// word to any connector or agent.

import { MIN_TOKEN_LENGTH, matchTokenPath, safeEqual, tokenConfigured } from "./auth.js";
import { GLOB_SEGMENT, globMatch, isGlob } from "./glob.js";
import { InputError, MAX_BODY, MAX_SENDER, normalizeWorkMark, workTransitionError } from "./messages.js";
import { normalizeNoteIdentity, noteTokenHash } from "./notes.js";
import { pipelineFeed, unifiedAgents } from "./pipeline.js";
import { relay, replyAddress } from "./relay.js";

export const FLEET_KINDS = ["message", "command"];
export const FLEET_DIRECTIONS = ["inbound", "outbound"];
export const MAX_RECIPIENTS = 200;
export const OFFER_MAX_TTL_SECONDS = 30 * 24 * 3600;
export const MAX_GROUPS = 16;
export const FLEET_DEFAULT_LIMIT = 50;
export const FLEET_MAX_LIMIT = 200;
const NAME = /^[a-z0-9][a-z0-9._-]{0,63}$/;
const FLEET_TOKEN_LABEL = "message-bridge/fleet-token/v1";

const b64url = (bytes) => {
  let s = "";
  for (const b of bytes) s += String.fromCharCode(b);
  return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
};

/** The fleet token: FLEET_TOKEN when set and distinct, else derived from ACCESS_TOKEN. */
export async function fleetToken(env) {
  if (!tokenConfigured(env)) return null;
  const own = env.FLEET_TOKEN;
  if (
    typeof own === "string" &&
    own.length >= MIN_TOKEN_LENGTH &&
    !safeEqual(own, env.ACCESS_TOKEN) &&
    !safeEqual(own, env.CHATGPT_TOKEN ?? "")
  ) {
    return own;
  }
  return deriveFleetToken(env.ACCESS_TOKEN);
}

/** HMAC-SHA256(ACCESS_TOKEN, label), base64url: 43 characters. */
export async function deriveFleetToken(accessToken) {
  const enc = new TextEncoder();
  const key = await crypto.subtle.importKey("raw", enc.encode(accessToken), { name: "HMAC", hash: "SHA-256" }, false, [
    "sign",
  ]);
  return b64url(new Uint8Array(await crypto.subtle.sign("HMAC", key, enc.encode(FLEET_TOKEN_LABEL))));
}

// ---- validation (runs in the Worker, so InputError keeps its type) ----

export function normalizeName(raw, what = "agent") {
  if (typeof raw !== "string") throw new InputError(`${what} must be a name`);
  const n = raw.trim().toLowerCase();
  if (!NAME.test(n)) throw new InputError(`${what} must be 1-64 of a-z 0-9 . _ - starting with a letter or digit`);
  return n;
}

export function normalizeGroups(raw) {
  if (raw === undefined || raw === null || raw === "") return null;
  const list = Array.isArray(raw) ? raw : String(raw).split(",");
  const out = [];
  for (const g of list) {
    if (typeof g === "string" && g.trim() === "") continue;
    const n = normalizeName(g, "group");
    if (!out.includes(n)) out.push(n);
  }
  if (out.length > MAX_GROUPS) throw new InputError(`at most ${MAX_GROUPS} groups`);
  return out.sort();
}

export function normalizeFleetKind(kind, { filter = false } = {}) {
  if (kind === undefined || kind === null || kind === "") return filter ? "all" : "message";
  if (filter && kind === "all") return "all";
  if (!FLEET_KINDS.includes(kind)) {
    throw new InputError(`kind must be ${filter ? "all or " : ""}one of ${FLEET_KINDS.join(", ")}`);
  }
  return kind;
}

function normalizeBody(body) {
  if (typeof body !== "string" || body.trim() === "") throw new InputError("body must be a non-empty string");
  if (body.length > MAX_BODY) throw new InputError(`body is longer than ${MAX_BODY} characters`);
  return body;
}

function normalizeSender(sender) {
  if (sender === undefined || sender === null || sender === "") return null;
  if (typeof sender !== "string") throw new InputError("sender must be a string");
  return sender.trim().slice(0, MAX_SENDER) || null;
}

function intParam(v, name, { min = 0, dflt = null } = {}) {
  if (v === undefined || v === null || v === "") return dflt;
  const n = Number(v);
  if (!Number.isInteger(n) || n < min) throw new InputError(`${name} must be an integer >= ${min}`);
  return n;
}

function boolParam(v, dflt) {
  if (v === undefined || v === null || v === "") return dflt;
  if (v === true || v === "true" || v === "1") return true;
  if (v === false || v === "false" || v === "0") return false;
  throw new InputError("expected true or false");
}

/** Validates an agent glob ("f*", "muse-*"): name characters plus "*". */
export function normalizePattern(raw) {
  if (typeof raw !== "string") throw new InputError("agent pattern must be a string");
  const p = raw.trim().toLowerCase();
  if (!GLOB_SEGMENT.test(p)) {
    throw new InputError("agent pattern must be 1-64 of a-z 0-9 . _ - and * starting with a letter, digit or *");
  }
  return p;
}

/** An agent target: an exact name, or a glob when it contains "*". */
function normalizeAgentTarget(raw) {
  return isGlob(raw) ? normalizePattern(raw) : normalizeName(raw, "agent");
}

/** The raw `to` as one string, for the distribution record's pattern column. */
function rawPattern(to) {
  if (typeof to === "string") return to;
  if (Array.isArray(to)) return to.join(",");
  return null;
}

function normalizeLimit(v) {
  const n = intParam(v, "limit", { min: 1 });
  if (n !== null && n > MAX_RECIPIENTS) throw new InputError(`limit must be at most ${MAX_RECIPIENTS}`);
  return n;
}

const BATCH = /^[A-Za-z0-9_-]{8,40}$/;

function normalizeBatch(v, what = "batch") {
  if (typeof v !== "string" || !BATCH.test(v)) throw new InputError(`${what} must be a batch id`);
  return v;
}

/**
 * Parses `to` into { agents: [names and globs], groups: [names], all: bool }.
 * "agent:x" or "x" is an agent (a glob when x contains "*"), "group:g"
 * a group, "all" every agent. Globs resolve against the registry at
 * send time (fleetOp "send"), in name order.
 */
export function parseTargets(to) {
  if (to === undefined || to === null || to === "") throw new InputError("to must name at least one recipient");
  const list = Array.isArray(to) ? to : String(to).split(",");
  const agents = [];
  const groups = [];
  let all = false;
  for (const raw of list) {
    if (typeof raw !== "string") throw new InputError("each recipient must be a string");
    const t = raw.trim().toLowerCase();
    if (t === "") continue;
    if (t === "all" || t === "agents" || t === "group:all") {
      all = true;
    } else if (t.startsWith("group:")) {
      const g = normalizeName(t.slice(6), "group");
      if (!groups.includes(g)) groups.push(g);
    } else {
      const a = normalizeAgentTarget(t.startsWith("agent:") ? t.slice(6) : t);
      if (!agents.includes(a)) agents.push(a);
    }
  }
  if (!all && !agents.length && !groups.length) throw new InputError("to must name at least one recipient");
  if (agents.length + groups.length > MAX_RECIPIENTS) throw new InputError(`at most ${MAX_RECIPIENTS} recipients`);
  return { agents, groups, all };
}

const FROM_ADDR = /^(channel:(claude|chatgpt)|(agent|service):[a-z0-9][a-z0-9._-]{0,63})$/;

function normalizeFromAddr(v) {
  if (v === undefined || v === null) return null;
  if (typeof v !== "string" || !FROM_ADDR.test(v)) {
    throw new InputError("from_addr must be channel:<name>, agent:<name> or service:<name>");
  }
  return v;
}

function ackArgs(data) {
  const hasUpTo = data?.up_to_id !== undefined && data?.up_to_id !== null;
  const ids = Array.isArray(data?.ids) ? data.ids : [];
  if (!hasUpTo && !ids.length) throw new InputError("give up_to_id or a non-empty ids list");
  if (hasUpTo && (!Number.isInteger(data.up_to_id) || data.up_to_id < 0)) {
    throw new InputError("up_to_id must be a non-negative integer");
  }
  if (!ids.every((i) => Number.isInteger(i) && i > 0)) throw new InputError("ids must be positive integers");
  if (ids.length > 1000) throw new InputError("at most 1000 ids at once");
  return { up_to_id: hasUpTo ? data.up_to_id : null, ids };
}

/** Validates one store operation's arguments; returns the normalized ones. */
export function normalizeFleetOp(op, a = {}) {
  switch (op) {
    case "hello":
      return { agent: normalizeName(a.agent), groups: normalizeGroups(a.groups) };
    case "inbox":
      return {
        agent: normalizeName(a.agent),
        groups: normalizeGroups(a.groups),
        kind: normalizeFleetKind(a.kind, { filter: true }),
        since_id: intParam(a.since_id, "since_id", { dflt: 0 }),
        unread_only: boolParam(a.unread_only, true),
        mark_read: boolParam(a.mark_read, false),
        limit: Math.min(intParam(a.limit, "limit", { min: 1, dflt: FLEET_DEFAULT_LIMIT }), FLEET_MAX_LIMIT),
      };
    case "ack":
      return { agent: normalizeName(a.agent), ...ackArgs(a) };
    case "work":
      return { agent: normalizeName(a.agent), ...normalizeWorkMark(a) };
    case "post":
      return {
        agent: normalizeName(a.agent),
        body: normalizeBody(a.body),
        reply_to: intParam(a.reply_to, "reply_to", { min: 1 }),
      };
    case "send":
      return {
        body: normalizeBody(a.body),
        targets: parseTargets(a.to),
        kind: normalizeFleetKind(a.kind),
        sender: normalizeSender(a.sender),
        batch: typeof a.batch === "string" && BATCH.test(a.batch) ? a.batch : null,
        from_addr: normalizeFromAddr(a.from_addr),
        // relay.js: the sending agent never gets its own message, and
        // "all" reaching no agent yet isn't an error when it also
        // reached a connector.
        exclude: a.exclude === undefined || a.exclude === null ? null : normalizeName(a.exclude),
        lenient: a.lenient === true,
        // Pattern addressing (§4.1): an optional cap on deliveries and
        // the raw address text, recorded on the distribution.
        limit: normalizeLimit(a.limit),
        pattern: rawPattern(a.to),
      };
    case "broaden":
      return {
        batch: normalizeBatch(a.batch, "broaden"),
        targets: a.to === undefined || a.to === null || a.to === "" ? null : parseTargets(a.to),
        pattern: rawPattern(a.to),
        limit: normalizeLimit(a.limit),
        from_addr: normalizeFromAddr(a.from_addr),
        exclude: a.exclude === undefined || a.exclude === null ? null : normalizeName(a.exclude),
      };
    case "offer": {
      const slots = intParam(a.slots, "slots", { min: 1 });
      if (slots === null) throw new InputError("slots must be an integer >= 1");
      if (slots > MAX_RECIPIENTS) throw new InputError(`slots must be at most ${MAX_RECIPIENTS}`);
      const ttl = intParam(a.ttl_seconds, "ttl_seconds", { min: 1 });
      if (ttl !== null && ttl > OFFER_MAX_TTL_SECONDS) {
        throw new InputError(`ttl_seconds must be at most ${OFFER_MAX_TTL_SECONDS}`);
      }
      return {
        body: normalizeBody(a.body),
        targets: parseTargets(a.to),
        kind: normalizeFleetKind(a.kind),
        sender: normalizeSender(a.sender),
        from_addr: normalizeFromAddr(a.from_addr),
        pattern: rawPattern(a.to),
        slots,
        ttl_seconds: ttl,
      };
    }
    case "offers":
      return { agent: normalizeName(a.agent), groups: normalizeGroups(a.groups) };
    case "claim": {
      const offer_id = intParam(a.offer_id, "offer_id", { min: 1 });
      if (offer_id === null) throw new InputError("offer_id must be a positive integer");
      return { agent: normalizeName(a.agent), offer_id };
    }
    case "offer-list":
      return {};
    case "origin":
      return { agent: normalizeName(a.agent), id: intParam(a.id, "id", { min: 1 }) };
    case "agents":
      return {};
    case "messages": {
      const direction = a.direction ?? "all";
      if (direction !== "all" && !FLEET_DIRECTIONS.includes(direction)) {
        throw new InputError("direction must be inbound, outbound or all");
      }
      const order = a.order ?? "asc";
      if (order !== "asc" && order !== "desc") throw new InputError("order must be asc or desc");
      return {
        agent: a.agent === undefined || a.agent === null || a.agent === "" ? null : normalizeName(a.agent),
        direction,
        kind: normalizeFleetKind(a.kind, { filter: true }),
        since_id: intParam(a.since_id, "since_id", { dflt: 0 }),
        before_id: intParam(a.before_id, "before_id", { min: 1 }),
        limit: Math.min(intParam(a.limit, "limit", { min: 1, dflt: FLEET_DEFAULT_LIMIT }), FLEET_MAX_LIMIT),
        order,
      };
    }
    default:
      throw new InputError(`unknown fleet operation ${op}`);
  }
}

// ---- storage (runs inside the Durable Object; also over node:sqlite in tests) ----

/** Creates the fleet tables; safe to run on every start. */
export function fleetSchema(sql) {
  sql.exec(`CREATE TABLE IF NOT EXISTS fleet_agent (
    name TEXT PRIMARY KEY,
    groups TEXT NOT NULL DEFAULT '[]',
    first_seen TEXT NOT NULL,
    last_seen TEXT NOT NULL
  )`);
  sql.exec(`CREATE TABLE IF NOT EXISTS fleet_message (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts TEXT NOT NULL,
    agent TEXT NOT NULL,
    direction TEXT NOT NULL CHECK (direction IN ('inbound', 'outbound')),
    kind TEXT NOT NULL CHECK (kind IN ('message', 'command')),
    sender TEXT,
    body TEXT NOT NULL,
    batch TEXT,
    reply_to INTEGER,
    read_at TEXT
  )`);
  // Rows from before connectors and agents could message agents all came from central.
  const cols = sql.exec("PRAGMA table_info(fleet_message)").toArray();
  if (!cols.some((c) => c.name === "from_addr")) sql.exec("ALTER TABLE fleet_message ADD COLUMN from_addr TEXT");
  // The instruction work-item lifecycle (#14372, design §4.6): null
  // until the receiving agent marks the row; null reads as "received".
  if (!cols.some((c) => c.name === "work_state")) sql.exec("ALTER TABLE fleet_message ADD COLUMN work_state TEXT");
  if (!cols.some((c) => c.name === "work_task_id")) sql.exec("ALTER TABLE fleet_message ADD COLUMN work_task_id INTEGER");
  sql.exec("CREATE INDEX IF NOT EXISTS fleet_message_mailbox ON fleet_message (agent, direction, read_at)");
  // Per-agent credentials (#14368): only the SHA-256 hash of a credential
  // is stored, never the value. scopes is a JSON array drawn from
  // CREDENTIAL_SCOPES; revoked_at is the per-agent kill switch.
  sql.exec(`CREATE TABLE IF NOT EXISTS fleet_credential (
    token_hash TEXT PRIMARY KEY,
    agent TEXT NOT NULL,
    scopes TEXT NOT NULL DEFAULT '[]',
    created TEXT NOT NULL,
    revoked_at TEXT
  )`);
  // Service credentials (#14373): the address a credential sends as,
  // when it is not agent:<its name> (service:<name>, its own name).
  const credCols = sql.exec("PRAGMA table_info(fleet_credential)").toArray();
  if (!credCols.some((c) => c.name === "addr")) sql.exec("ALTER TABLE fleet_credential ADD COLUMN addr TEXT");
  // Bootstrap enroll tokens (#14368): hash only, bound at mint time to
  // the agent name, kind and groups; single-use (used_at), expiring
  // (expires_at), revocable while unused (revoked_at).
  sql.exec(`CREATE TABLE IF NOT EXISTS enroll_token (
    token_hash TEXT PRIMARY KEY,
    agent TEXT NOT NULL,
    kind TEXT NOT NULL,
    groups TEXT NOT NULL DEFAULT '[]',
    notes_identity TEXT,
    expires_at TEXT NOT NULL,
    used_at TEXT,
    revoked_at TEXT,
    created TEXT NOT NULL
  )`);
  // One record per pattern send (design §4.1): what the address
  // resolved to and who actually got a row, so a later broaden can
  // deliver to exactly the matched-but-undelivered remainder. The
  // notes surface writes its distributions here too (same store).
  sql.exec(`CREATE TABLE IF NOT EXISTS bridge_distribution (
    batch TEXT PRIMARY KEY,
    sender_addr TEXT,
    pattern TEXT,
    limit_n INTEGER,
    matched TEXT NOT NULL,
    delivered TEXT NOT NULL,
    ts TEXT NOT NULL
  )`);
  // Offer pool (design §4.2): an offer is stored once, not fanned out;
  // a claim (fleet_offer_claim, one row per claimer) is what delivers
  // a mailbox row. state is 'open' until the last slot is claimed
  // ('filled'); expiry is derived from expires_at at read time.
  sql.exec(`CREATE TABLE IF NOT EXISTS fleet_offer (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    batch TEXT NOT NULL,
    body TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('message', 'command')),
    pattern TEXT,
    slots INTEGER NOT NULL,
    sender TEXT,
    sender_addr TEXT,
    ts TEXT NOT NULL,
    expires_at TEXT,
    state TEXT NOT NULL DEFAULT 'open' CHECK (state IN ('open', 'filled'))
  )`);
  sql.exec(`CREATE TABLE IF NOT EXISTS fleet_offer_claim (
    offer_id INTEGER NOT NULL,
    agent TEXT NOT NULL,
    claimed_at TEXT NOT NULL,
    message_id INTEGER,
    PRIMARY KEY (offer_id, agent)
  )`);
}

/** Records (or replaces) one send's distribution; matched/delivered are name lists. */
export function recordDistribution(sql, { batch, sender_addr, pattern, limit_n, matched, delivered, ts }) {
  sql.exec(
    `INSERT OR REPLACE INTO bridge_distribution (batch, sender_addr, pattern, limit_n, matched, delivered, ts)
     VALUES (?, ?, ?, ?, ?, ?, ?)`,
    batch,
    sender_addr ?? null,
    pattern ?? null,
    limit_n ?? null,
    JSON.stringify(matched),
    JSON.stringify(delivered),
    ts,
  );
}

/** One distribution record by batch, or null. */
export function getDistribution(sql, batch) {
  const r = sql.exec("SELECT * FROM bridge_distribution WHERE batch = ?", batch).toArray()[0];
  if (!r) return null;
  return { ...r, matched: JSON.parse(r.matched), delivered: JSON.parse(r.delivered) };
}

// ---- offers (design §4.2) ----

/** One offer's claims, oldest first: [{agent, claimed_at, message_id}]. */
function offerClaims(sql, offer_id) {
  return sql
    .exec("SELECT agent, claimed_at, message_id FROM fleet_offer_claim WHERE offer_id = ? ORDER BY claimed_at, agent", offer_id)
    .toArray()
    .map((r) => ({ ...r }));
}

/** An offer's state as of now: the stored state, or "expired" past its expiry. */
function offerState(offer, claimed, now) {
  if (offer.state === "filled" || claimed >= offer.slots) return "filled";
  if (offer.expires_at && offer.expires_at <= now) return "expired";
  return "open";
}

/** The owner's view of one offer: real sender, claims named. */
function offerForOwner(sql, offer, now) {
  const claims = offerClaims(sql, offer.id);
  return {
    ...offer,
    state: offerState(offer, claims.length, now),
    claimed: claims.length,
    remaining: offer.slots - claims.length,
    claims,
  };
}

/**
 * An eligible agent's view of one offer. A command offer names no
 * sender here either (the anonymity rule, §4.3): the claimer treats
 * it as Drew's, exactly like a delivered command.
 */
function offerForAgent(offer, claimed, now) {
  const command = offer.kind === "command";
  return {
    id: offer.id,
    batch: offer.batch,
    body: offer.body,
    kind: offer.kind,
    slots: offer.slots,
    claimed,
    remaining: offer.slots - claimed,
    ts: offer.ts,
    expires_at: offer.expires_at,
    state: offerState(offer, claimed, now),
    sender: command ? null : offer.sender,
    from_addr: command ? null : offer.sender_addr,
  };
}

/**
 * Whether the offer's address names this agent, matched against the
 * registry row at claim time (design §4.2): a glob over its name, one
 * of its groups, an exact name, or all. The poster never claims its
 * own offer.
 */
function offerEligible(offer, agent) {
  if (offer.sender_addr === `agent:${agent.name}`) return false;
  let targets;
  try {
    targets = parseTargets(offer.pattern);
  } catch {
    return false;
  }
  if (targets.all) return true;
  if (targets.groups.some((g) => agent.groups.includes(g))) return true;
  return targets.agents.some((t) => (isGlob(t) ? globMatch(t, agent.name) : t === agent.name));
}

/** The open offers this agent may still claim, oldest first, in agent projection. */
function openOffersFor(sql, agent, now) {
  const me = agentRow(sql.exec("SELECT * FROM fleet_agent WHERE name = ?", agent).toArray()[0]);
  if (!me) return [];
  const out = [];
  for (const offer of sql.exec("SELECT * FROM fleet_offer WHERE state = 'open' ORDER BY id ASC").toArray()) {
    const claims = offerClaims(sql, offer.id);
    if (claims.some((c) => c.agent === agent)) continue;
    if (offerState(offer, claims.length, now) !== "open") continue;
    if (!offerEligible(offer, me)) continue;
    out.push(offerForAgent(offer, claims.length, now));
  }
  return out;
}

const rows = (cursor) => cursor.toArray().map((r) => ({ ...r }));

function agentRow(r) {
  return r ? { name: r.name, groups: JSON.parse(r.groups), first_seen: r.first_seen, last_seen: r.last_seen } : null;
}

function touch(sql, agent, groups, ts) {
  const existing = sql.exec("SELECT * FROM fleet_agent WHERE name = ?", agent).toArray()[0];
  if (!existing) {
    sql.exec(
      "INSERT INTO fleet_agent (name, groups, first_seen, last_seen) VALUES (?, ?, ?, ?)",
      agent,
      JSON.stringify(groups ?? []),
      ts,
      ts,
    );
  } else if (groups) {
    sql.exec("UPDATE fleet_agent SET groups = ?, last_seen = ? WHERE name = ?", JSON.stringify(groups), ts, agent);
  } else {
    sql.exec("UPDATE fleet_agent SET last_seen = ? WHERE name = ?", ts, agent);
  }
  return agentRow(sql.exec("SELECT * FROM fleet_agent WHERE name = ?", agent).toArray()[0]);
}

function mailboxStats(sql, agent) {
  const s = sql
    .exec(
      `SELECT
        COALESCE(SUM(direction = 'inbound' AND read_at IS NULL AND kind = 'command'), 0) AS unread_commands,
        COALESCE(SUM(direction = 'inbound' AND read_at IS NULL AND kind = 'message'), 0) AS unread_messages,
        COALESCE(SUM(direction = 'inbound'), 0) AS inbound,
        COALESCE(SUM(direction = 'outbound'), 0) AS outbound,
        COALESCE(MAX(id), 0) AS last_id
      FROM fleet_message WHERE agent = ?`,
      agent,
    )
    .toArray()[0];
  return { ...s };
}

export function randomBatch() {
  const b = new Uint8Array(12);
  crypto.getRandomValues(b);
  return b64url(b);
}

/**
 * Resolves parsed targets against the registry, in name order.
 * Returns { matched: [names] } or { error }. Exact names resolve to
 * themselves (registered or not, as fleet sends have always done);
 * globs, groups and all resolve over registered agents, the sender
 * excluded. A glob, group or all matching nobody is an error, except
 * a lenient all (relay.js: it also reached a connector).
 */
function resolveTargets(registered, targets, { exclude = null, lenient = false } = {}) {
  const others = registered.filter((r) => r.name !== exclude);
  const matched = new Set();
  for (const t of targets.agents) {
    if (isGlob(t)) {
      const hits = others.filter((r) => globMatch(t, r.name));
      if (!hits.length) return { error: `no registered agent matches agent:${t}` };
      for (const r of hits) matched.add(r.name);
    } else {
      if (exclude && t === exclude) return { error: "an agent can't send to itself" };
      matched.add(t);
    }
  }
  if (targets.all) {
    if (!others.length && !lenient) return { error: "no agents have registered yet" };
    for (const r of others) matched.add(r.name);
  }
  for (const g of targets.groups) {
    const members = others.filter((r) => r.groups.includes(g));
    if (!members.length) return { error: `no registered agent is in group ${g}` };
    for (const r of members) matched.add(r.name);
  }
  return { matched: [...matched].sort() };
}

/**
 * One store operation over normalized arguments. State-dependent
 * refusals come back as { error } (an InputError doesn't survive the
 * Durable Object's RPC hop); fleetCall turns them into InputErrors.
 */
export function fleetOp(sql, op, a, now = new Date().toISOString()) {
  switch (op) {
    case "hello": {
      const agent = touch(sql, a.agent, a.groups, now);
      return { agent, stats: mailboxStats(sql, a.agent) };
    }
    case "inbox": {
      const agent = touch(sql, a.agent, a.groups, now);
      const where = ["agent = ?", "direction = 'inbound'", "id > ?"];
      const args = [a.agent, a.since_id];
      if (a.unread_only) where.push("read_at IS NULL");
      if (a.kind !== "all") {
        where.push("kind = ?");
        args.push(a.kind);
      }
      const messages = rows(
        sql.exec(`SELECT * FROM fleet_message WHERE ${where.join(" AND ")} ORDER BY id ASC LIMIT ?`, ...args, a.limit),
      );
      const next = messages.length ? messages[messages.length - 1].id : a.since_id;
      if (a.mark_read) {
        for (const m of messages) {
          if (m.read_at === null) {
            sql.exec("UPDATE fleet_message SET read_at = ? WHERE id = ? AND read_at IS NULL", now, m.id);
            m.read_at = now;
          }
        }
      }
      return { agent, messages, next_since_id: next, stats: mailboxStats(sql, a.agent), offers: openOffersFor(sql, a.agent, now) };
    }
    case "ack": {
      let acked = 0;
      if (a.up_to_id !== null) {
        acked += sql
          .exec(
            "UPDATE fleet_message SET read_at = ? WHERE agent = ? AND direction = 'inbound' AND read_at IS NULL AND id <= ? RETURNING id",
            now,
            a.agent,
            a.up_to_id,
          )
          .toArray().length;
      }
      for (const id of a.ids) {
        acked += sql
          .exec(
            "UPDATE fleet_message SET read_at = ? WHERE agent = ? AND direction = 'inbound' AND read_at IS NULL AND id = ? RETURNING id",
            now,
            a.agent,
            id,
          )
          .toArray().length;
      }
      touch(sql, a.agent, null, now);
      return { acked, stats: mailboxStats(sql, a.agent) };
    }
    case "work": {
      // Only the receiving agent's own inbound rows can be marked: the
      // lookup is scoped to this mailbox, so another agent's row (or a
      // row this agent sent) is simply not found. Transitions are
      // forward-only (messages.js workTransitionError); a queued mark
      // already named its task id at validation time, and a later mark
      // without one keeps the recorded id.
      const row = sql
        .exec(
          "SELECT id, work_state, work_task_id FROM fleet_message WHERE id = ? AND agent = ? AND direction = 'inbound'",
          a.id,
          a.agent,
        )
        .toArray()[0];
      if (!row) return { error: `message ${a.id} is not in ${a.agent}'s mailbox` };
      const err = workTransitionError(row.work_state, a.state);
      if (err) return { error: err };
      sql.exec(
        "UPDATE fleet_message SET work_state = ?, work_task_id = ? WHERE id = ?",
        a.state,
        a.task_id ?? row.work_task_id ?? null,
        a.id,
      );
      touch(sql, a.agent, null, now);
      const message = { ...sql.exec("SELECT * FROM fleet_message WHERE id = ?", a.id).toArray()[0] };
      return { message, stats: mailboxStats(sql, a.agent) };
    }
    case "post": {
      let batch = null;
      if (a.reply_to !== null) {
        const orig = sql
          .exec("SELECT * FROM fleet_message WHERE id = ? AND agent = ? AND direction = 'inbound'", a.reply_to, a.agent)
          .toArray()[0];
        if (!orig) return { error: `reply_to ${a.reply_to} is not a message to ${a.agent}` };
        batch = orig.batch;
        sql.exec("UPDATE fleet_message SET read_at = ? WHERE id = ? AND read_at IS NULL", now, a.reply_to);
      }
      touch(sql, a.agent, null, now);
      const message = {
        ...sql
          .exec(
            `INSERT INTO fleet_message (ts, agent, direction, kind, sender, body, batch, reply_to)
             VALUES (?, ?, 'outbound', 'message', ?, ?, ?, ?) RETURNING *`,
            now,
            a.agent,
            a.agent,
            a.body,
            batch,
            a.reply_to,
          )
          .toArray()[0],
      };
      return { message, stats: mailboxStats(sql, a.agent) };
    }
    case "send": {
      const registered = rows(sql.exec("SELECT * FROM fleet_agent ORDER BY name")).map(agentRow);
      const resolved = resolveTargets(registered, a.targets, { exclude: a.exclude, lenient: a.lenient });
      if (resolved.error) return { error: resolved.error };
      const matched = resolved.matched;
      if (!a.limit && matched.length > MAX_RECIPIENTS) return { error: `that reaches more than ${MAX_RECIPIENTS} agents` };
      const recipients = a.limit ? matched.slice(0, a.limit) : matched;
      const known = new Set(registered.map((r) => r.name));
      const batch = a.batch ?? randomBatch();
      const messages = [];
      for (const agent of recipients) {
        messages.push({
          ...sql
            .exec(
              `INSERT INTO fleet_message (ts, agent, direction, kind, sender, body, batch, from_addr)
               VALUES (?, ?, 'inbound', ?, ?, ?, ?, ?) RETURNING *`,
              now,
              agent,
              a.kind,
              a.sender,
              a.body,
              batch,
              a.from_addr,
            )
            .toArray()[0],
        });
      }
      recordDistribution(sql, {
        batch,
        sender_addr: a.from_addr,
        pattern: a.pattern,
        limit_n: a.limit,
        matched,
        delivered: recipients,
        ts: now,
      });
      return {
        batch,
        messages,
        unregistered: recipients.filter((n) => !known.has(n)),
        matched: matched.length,
        delivered: recipients.length,
        recipients,
        remaining: matched.length - recipients.length,
      };
    }
    case "broaden": {
      const dist = getDistribution(sql, a.batch);
      if (!dist) return { error: `no distribution recorded for batch ${a.batch}` };
      // Broadening re-sends under the original sender's identity, so
      // only that sender may broaden — or central (from_addr null,
      // the owner token), who may broaden any distribution.
      if (a.from_addr !== null && dist.sender_addr !== null && a.from_addr !== dist.sender_addr) {
        return { error: "only the original sender can broaden this distribution" };
      }
      const sample = sql
        .exec("SELECT * FROM fleet_message WHERE batch = ? AND direction = 'inbound' ORDER BY id ASC LIMIT 1", a.batch)
        .toArray()[0];
      if (!sample) return { error: `batch ${a.batch} has no fleet messages to broaden` };
      let matched = dist.matched;
      if (a.targets) {
        const registered = rows(sql.exec("SELECT * FROM fleet_agent ORDER BY name")).map(agentRow);
        const resolved = resolveTargets(registered, a.targets, { exclude: a.exclude });
        if (resolved.error) return { error: resolved.error };
        matched = [...new Set([...dist.matched, ...resolved.matched])].sort();
      }
      const deliveredSet = new Set(dist.delivered);
      const pending = matched.filter((n) => !deliveredSet.has(n));
      const recipients = a.limit ? pending.slice(0, a.limit) : pending;
      const messages = [];
      for (const agent of recipients) {
        messages.push({
          ...sql
            .exec(
              `INSERT INTO fleet_message (ts, agent, direction, kind, sender, body, batch, from_addr)
               VALUES (?, ?, 'inbound', ?, ?, ?, ?, ?) RETURNING *`,
              now,
              agent,
              sample.kind,
              sample.sender,
              sample.body,
              a.batch,
              sample.from_addr,
            )
            .toArray()[0],
        });
        deliveredSet.add(agent);
      }
      const delivered = [...deliveredSet].sort();
      recordDistribution(sql, {
        batch: a.batch,
        sender_addr: dist.sender_addr,
        pattern: a.pattern ?? dist.pattern,
        limit_n: a.limit ?? dist.limit_n,
        matched,
        delivered,
        ts: now,
      });
      const known = new Set(rows(sql.exec("SELECT name FROM fleet_agent")).map((r) => r.name));
      return {
        batch: a.batch,
        messages,
        unregistered: recipients.filter((n) => !known.has(n)),
        matched: matched.length,
        delivered: recipients.length,
        recipients,
        remaining: matched.length - delivered.length,
      };
    }
    case "offer": {
      const batch = randomBatch();
      const expires_at = a.ttl_seconds ? new Date(Date.parse(now) + a.ttl_seconds * 1000).toISOString() : null;
      const offer = {
        ...sql
          .exec(
            `INSERT INTO fleet_offer (batch, body, kind, pattern, slots, sender, sender_addr, ts, expires_at, state)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'open') RETURNING *`,
            batch,
            a.body,
            a.kind,
            a.pattern,
            a.slots,
            a.sender,
            a.from_addr,
            now,
            expires_at,
          )
          .toArray()[0],
      };
      return { offer: offerForOwner(sql, offer, now) };
    }
    case "offers": {
      touch(sql, a.agent, a.groups, now);
      return { offers: openOffersFor(sql, a.agent, now) };
    }
    case "claim": {
      touch(sql, a.agent, null, now);
      const offer = sql.exec("SELECT * FROM fleet_offer WHERE id = ?", a.offer_id).toArray()[0];
      if (!offer) return { error: `offer ${a.offer_id} does not exist` };
      const me = agentRow(sql.exec("SELECT * FROM fleet_agent WHERE name = ?", a.agent).toArray()[0]);
      if (!offerEligible(offer, me)) return { error: `${a.agent} is not eligible for offer ${offer.id}` };
      const claims = offerClaims(sql, offer.id);
      if (claims.some((c) => c.agent === a.agent)) return { error: `${a.agent} has already claimed offer ${offer.id}` };
      if (offer.expires_at && offer.expires_at <= now) return { error: `offer ${offer.id} has expired` };
      if (offer.state !== "open" || claims.length >= offer.slots) return { error: `offer ${offer.id} is filled` };
      // The claim itself is one conditional insert: it lands only
      // while a slot remains on an open offer and this agent has not
      // claimed — so racing claims cannot oversubscribe the slots,
      // whatever order the store runs them in.
      const landed = sql
        .exec(
          `INSERT INTO fleet_offer_claim (offer_id, agent, claimed_at, message_id)
           SELECT ?, ?, ?, NULL
           WHERE (SELECT COUNT(*) FROM fleet_offer_claim WHERE offer_id = ?) < (SELECT slots FROM fleet_offer WHERE id = ? AND state = 'open')
             AND NOT EXISTS (SELECT 1 FROM fleet_offer_claim WHERE offer_id = ? AND agent = ?)
           RETURNING offer_id`,
          offer.id,
          a.agent,
          now,
          offer.id,
          offer.id,
          offer.id,
          a.agent,
        )
        .toArray();
      if (!landed.length) {
        const again = offerClaims(sql, offer.id);
        if (again.some((c) => c.agent === a.agent)) return { error: `${a.agent} has already claimed offer ${offer.id}` };
        return { error: `offer ${offer.id} is filled` };
      }
      const message = {
        ...sql
          .exec(
            `INSERT INTO fleet_message (ts, agent, direction, kind, sender, body, batch, from_addr)
             VALUES (?, ?, 'inbound', ?, ?, ?, ?, ?) RETURNING *`,
            now,
            a.agent,
            offer.kind,
            offer.sender,
            offer.body,
            offer.batch,
            offer.sender_addr,
          )
          .toArray()[0],
      };
      sql.exec("UPDATE fleet_offer_claim SET message_id = ? WHERE offer_id = ? AND agent = ?", message.id, offer.id, a.agent);
      const claimed = offerClaims(sql, offer.id).length;
      if (claimed >= offer.slots) sql.exec("UPDATE fleet_offer SET state = 'filled' WHERE id = ?", offer.id);
      return {
        offer: offerForAgent(offer, claimed, now),
        message,
        claimed,
        remaining: offer.slots - claimed,
        stats: mailboxStats(sql, a.agent),
      };
    }
    case "offer-list": {
      const offers = rows(sql.exec("SELECT * FROM fleet_offer ORDER BY id ASC")).map((r) => offerForOwner(sql, r, now));
      return { offers };
    }
    case "origin": {
      const orig = sql
        .exec("SELECT id, from_addr, batch FROM fleet_message WHERE id = ? AND agent = ? AND direction = 'inbound'", a.id, a.agent)
        .toArray()[0];
      if (!orig) return { error: `reply_to ${a.id} is not a message to ${a.agent}` };
      return { id: orig.id, from_addr: orig.from_addr ?? null, batch: orig.batch ?? null };
    }
    case "agents": {
      const agents = rows(sql.exec("SELECT * FROM fleet_agent ORDER BY name"))
        .map(agentRow)
        .map((r) => ({ ...r, ...mailboxStats(sql, r.name) }));
      return { agents };
    }
    case "messages": {
      const where = ["id > ?"];
      const args = [a.since_id];
      if (a.before_id !== null) {
        where.push("id < ?");
        args.push(a.before_id);
      }
      if (a.agent) {
        where.push("agent = ?");
        args.push(a.agent);
      }
      if (a.direction !== "all") {
        where.push("direction = ?");
        args.push(a.direction);
      }
      if (a.kind !== "all") {
        where.push("kind = ?");
        args.push(a.kind);
      }
      const order = a.order === "desc" ? "DESC" : "ASC";
      const messages = rows(
        sql.exec(`SELECT * FROM fleet_message WHERE ${where.join(" AND ")} ORDER BY id ${order} LIMIT ?`, ...args, a.limit),
      );
      return { messages };
    }
    default:
      return { error: `unknown fleet operation ${op}` };
  }
}

/** Worker side: validate, call the store's fleet RPC, turn { error } into InputError. */
export async function fleetCall(stub, op, args) {
  const normalized = normalizeFleetOp(op, args);
  const out = await stub.fleet(op, normalized);
  if (out && typeof out.error === "string") throw new InputError(out.error);
  return out;
}

// ---- per-agent credentials and bootstrap enrollment (#14368) ----

export const CREDENTIAL_SCOPES = ["inbox", "send"];
/** Every scope a credential can hold. "read" (the owner's pipeline
 * views, #14373) is minted directly by the owner only; the enroll
 * exchange mints CREDENTIAL_SCOPES and never this. */
export const MINT_SCOPES = ["inbox", "send", "read"];
export const ENROLL_DEFAULT_TTL_SECONDS = 24 * 3600;
export const ENROLL_MIN_TTL_SECONDS = 60;
export const ENROLL_MAX_TTL_SECONDS = 7 * 24 * 3600;

/** A fresh credential value: 32 random bytes, base64url (43 chars). Only its hash is stored. */
export function randomTokenValue() {
  const b = new Uint8Array(32);
  crypto.getRandomValues(b);
  return b64url(b);
}

// Storage for credentials and enroll tokens (Durable Object sql; the
// test MemoryStore mirrors these semantics). Hash-only throughout: no
// function here ever sees a token value, only its SHA-256 hex.

export function fleetCredentialAuthSql(sql, token_hash) {
  const r = sql
    .exec("SELECT agent, scopes, addr FROM fleet_credential WHERE token_hash = ? AND revoked_at IS NULL", token_hash)
    .toArray()[0];
  return r ? { agent: r.agent, scopes: JSON.parse(r.scopes), addr: r.addr ?? null } : null;
}

export function fleetCredentialPutSql(sql, { token_hash, agent, scopes, created, addr = null }) {
  sql.exec(
    "INSERT INTO fleet_credential (token_hash, agent, scopes, created, addr) VALUES (?, ?, ?, ?, ?)",
    token_hash,
    agent,
    JSON.stringify(scopes),
    created,
    addr,
  );
  return { agent, scopes: [...scopes], addr };
}

const SERVICE_ADDR = /^service:([a-z0-9][a-z0-9._-]{0,63})$/;

/**
 * Validates a direct credential mint body (the owner's service
 * credentials, #14373). The addr, when given, is service:<name> and
 * the name is the credential's own agent name: a service speaks as
 * itself, never as another principal. Default scopes follow the form:
 * a service gets send + read (it has no mailbox of its own to read),
 * an agent minted directly gets the enroll pair.
 */
export function normalizeMintCredential(data) {
  if (!data || typeof data !== "object" || Array.isArray(data)) throw new InputError("body must be a JSON object");
  const agent = normalizeName(data.agent);
  let addr = null;
  if (data.addr !== undefined && data.addr !== null && data.addr !== "") {
    const m = typeof data.addr === "string" ? SERVICE_ADDR.exec(data.addr) : null;
    if (!m) throw new InputError("addr must be service:<name>");
    if (m[1] !== agent) throw new InputError("addr must name the credential's own agent (service:<agent>)");
    addr = data.addr;
  }
  let scopes;
  if (data.scopes === undefined || data.scopes === null) {
    scopes = addr ? ["send", "read"] : [...CREDENTIAL_SCOPES];
  } else {
    if (!Array.isArray(data.scopes) || data.scopes.length === 0) {
      throw new InputError(`scopes must be a non-empty list drawn from ${MINT_SCOPES.join(", ")}`);
    }
    scopes = [];
    for (const s of data.scopes) {
      if (!MINT_SCOPES.includes(s)) throw new InputError(`scopes must be drawn from ${MINT_SCOPES.join(", ")}`);
      if (!scopes.includes(s)) scopes.push(s);
    }
  }
  return { agent, addr, scopes };
}

/**
 * Mints a credential directly for the owner, outside the enroll flow:
 * how a platform service (the PM stack) gets its own. The value exists
 * only in this return value; the store receives its SHA-256 hash.
 */
export async function mintFleetCredential(store, data, now = new Date().toISOString()) {
  const m = normalizeMintCredential(data);
  const value = randomTokenValue();
  await store.fleetCredentialPut({
    token_hash: await noteTokenHash(value),
    agent: m.agent,
    scopes: m.scopes,
    addr: m.addr,
    created: now,
  });
  return { agent: m.agent, addr: m.addr, scopes: m.scopes, credential: value };
}

/**
 * Folds credential rows (hashes already stripped by the caller) into
 * one state row per agent: whether the agent's credential stands
 * active (at least one live credential) or revoked, the scopes its
 * live credentials carry between them, the address a live service
 * credential sends as, and when the state last changed (the first
 * mint, or the final revocation). An agent part-revoked and part-live
 * is active, and says no revocation time: a superseded credential's
 * end is not the agent's.
 */
export function foldCredentialState(rows) {
  const byAgent = new Map();
  for (const r of rows) {
    let a = byAgent.get(r.agent);
    if (!a) {
      a = {
        agent: r.agent,
        state: "revoked",
        scopes: [],
        addr: null,
        credentials: 0,
        live: 0,
        created: r.created,
        revoked_at: null,
      };
      byAgent.set(r.agent, a);
    }
    a.credentials++;
    if (r.created < a.created) a.created = r.created;
    if (r.revoked_at) {
      if (!a.revoked_at || r.revoked_at > a.revoked_at) a.revoked_at = r.revoked_at;
    } else {
      a.live++;
      for (const s of JSON.parse(r.scopes)) if (!a.scopes.includes(s)) a.scopes.push(s);
      if (r.addr) a.addr = r.addr;
    }
  }
  const out = [...byAgent.values()];
  for (const a of out) {
    if (a.live > 0) {
      a.state = "active";
      a.revoked_at = null;
    }
  }
  return out.sort((x, y) => (x.agent < y.agent ? -1 : x.agent > y.agent ? 1 : 0));
}

/**
 * Per-agent credential state for the owner's console (design §4.9):
 * the fold above over every credential row. Only hashes are excluded
 * — they never leave the store; this is state, not key material.
 */
export function fleetCredentialStateSql(sql) {
  const rows = sql
    .exec("SELECT agent, scopes, addr, created, revoked_at FROM fleet_credential ORDER BY agent, created")
    .toArray();
  return foldCredentialState(rows);
}

/** Revokes every live credential of the agent; returns how many. */
export function fleetCredentialRevokeSql(sql, agent, now) {
  return sql
    .exec(
      "UPDATE fleet_credential SET revoked_at = ? WHERE agent = ? AND revoked_at IS NULL RETURNING token_hash",
      now,
      agent,
    )
    .toArray().length;
}

export function enrollTokenMintSql(sql, row) {
  sql.exec(
    `INSERT INTO enroll_token (token_hash, agent, kind, groups, notes_identity, expires_at, created)
     VALUES (?, ?, ?, ?, ?, ?, ?)`,
    row.token_hash,
    row.agent,
    row.kind,
    JSON.stringify(row.groups),
    row.notes_identity,
    row.expires_at,
    row.created,
  );
  return {
    agent: row.agent,
    kind: row.kind,
    groups: [...row.groups],
    notes_identity: row.notes_identity,
    expires_at: row.expires_at,
    created: row.created,
  };
}

/** Every minted enroll token's state for the owner's list (fleet.js,
 * #14376): every column but the hash. The hash names a token only to
 * the revoke route; the list never carries it, and no value was ever
 * stored to show. */
export function enrollTokenListSql(sql) {
  return sql
    .exec(
      `SELECT agent, kind, groups, notes_identity, created, expires_at, used_at, revoked_at
         FROM enroll_token ORDER BY created DESC, agent`,
    )
    .toArray()
    .map((r) => ({
      agent: r.agent,
      kind: r.kind,
      groups: JSON.parse(r.groups),
      notes_identity: r.notes_identity,
      created: r.created,
      expires_at: r.expires_at,
      used_at: r.used_at,
      revoked_at: r.revoked_at,
    }));
}

/** The state a minted enroll token stands in, derived the same way
 * over either store's rows (#14376): revoked by the owner, used by the
 * exchange that burned it, expired by the clock, else pending. The
 * closing times ride along, so the console can say when each token
 * closed and by what: the exchange (used_at), the owner's revoke
 * (revoked_at), or the lapse of its TTL (expires_at). */
export function foldEnrollTokens(rows, now) {
  return rows.map((r) => ({
    agent: r.agent,
    kind: r.kind,
    groups: r.groups ?? [],
    notes_identity: r.notes_identity ?? null,
    created: r.created,
    expires_at: r.expires_at,
    used_at: r.used_at ?? null,
    revoked_at: r.revoked_at ?? null,
    state: r.revoked_at ? "revoked" : r.used_at ? "used" : r.expires_at <= now ? "expired" : "pending",
  }));
}

export function enrollTokenGetSql(sql, token_hash) {
  const r = sql.exec("SELECT * FROM enroll_token WHERE token_hash = ?", token_hash).toArray()[0];
  return r
    ? {
        agent: r.agent,
        kind: r.kind,
        groups: JSON.parse(r.groups),
        notes_identity: r.notes_identity,
        expires_at: r.expires_at,
        used_at: r.used_at,
        revoked_at: r.revoked_at,
        created: r.created,
      }
    : null;
}

/**
 * Burns a bootstrap token: one UPDATE that only lands when the token
 * exists, names this agent, and is unused, unrevoked and unexpired —
 * so a token can never be claimed twice. Returns the bound mint record.
 */
export function enrollTokenClaimSql(sql, token_hash, agent, now) {
  const claimed = sql
    .exec(
      `UPDATE enroll_token SET used_at = ?
       WHERE token_hash = ? AND agent = ? AND used_at IS NULL AND revoked_at IS NULL AND expires_at > ?
       RETURNING agent, kind, groups, notes_identity`,
      now,
      token_hash,
      agent,
      now,
    )
    .toArray();
  if (!claimed.length) return null;
  const r = claimed[0];
  return { agent: r.agent, kind: r.kind, groups: JSON.parse(r.groups), notes_identity: r.notes_identity };
}

/** Revokes an unused bootstrap token; false when it was used, revoked or unknown. */
export function enrollTokenRevokeSql(sql, token_hash, now) {
  return (
    sql
      .exec(
        "UPDATE enroll_token SET revoked_at = ? WHERE token_hash = ? AND used_at IS NULL AND revoked_at IS NULL RETURNING token_hash",
        now,
        token_hash,
      )
      .toArray().length > 0
  );
}

/** Revokes every still-pending bootstrap token minted for one agent
 * (#14376); returns how many. The console's revoke works by name: the
 * token list never shows a hash, so the owner's "take back what I
 * minted for this agent" cannot name one. Used tokens are untouched
 * (they became credentials — revoking those is the credential
 * revoke), and expired ones are already dead and keep their state. */
export function enrollTokenRevokeAgentSql(sql, agent, now) {
  return sql
    .exec(
      "UPDATE enroll_token SET revoked_at = ? WHERE agent = ? AND used_at IS NULL AND revoked_at IS NULL AND expires_at > ? RETURNING token_hash",
      now,
      agent,
      now,
    )
    .toArray().length;
}

/** Validates a bootstrap mint body; the token is bound to what this returns. */
export function normalizeEnrollMint(data) {
  if (!data || typeof data !== "object" || Array.isArray(data)) throw new InputError("body must be a JSON object");
  const ttl = data.ttl_seconds === undefined || data.ttl_seconds === null ? ENROLL_DEFAULT_TTL_SECONDS : data.ttl_seconds;
  if (!Number.isInteger(ttl) || ttl < ENROLL_MIN_TTL_SECONDS || ttl > ENROLL_MAX_TTL_SECONDS) {
    throw new InputError(`ttl_seconds must be an integer between ${ENROLL_MIN_TTL_SECONDS} and ${ENROLL_MAX_TTL_SECONDS}`);
  }
  return {
    agent: normalizeName(data.agent),
    kind:
      data.kind === undefined || data.kind === null || data.kind === "" ? "agent" : normalizeName(data.kind, "kind"),
    groups: normalizeGroups(data.groups) ?? [],
    notes_identity:
      data.notes_identity === undefined || data.notes_identity === null || data.notes_identity === ""
        ? null
        : normalizeNoteIdentity(data.notes_identity),
    ttl_seconds: ttl,
  };
}

/**
 * Mints a bootstrap token for the owner. The value exists only in this
 * return value; the store receives its SHA-256 hash.
 */
export async function mintEnrollToken(store, data, nowMs = Date.now()) {
  const m = normalizeEnrollMint(data);
  const value = randomTokenValue();
  const created = new Date(nowMs).toISOString();
  const expires_at = new Date(nowMs + m.ttl_seconds * 1000).toISOString();
  await store.enrollTokenMint({
    token_hash: await noteTokenHash(value),
    agent: m.agent,
    kind: m.kind,
    groups: m.groups,
    notes_identity: m.notes_identity,
    expires_at,
    created,
  });
  return {
    agent: m.agent,
    kind: m.kind,
    groups: m.groups,
    notes_identity: m.notes_identity,
    expires_at,
    bootstrap_token: value,
  };
}

/**
 * The owner's list of minted bootstrap tokens (#14376): state only —
 * pending, used, expired or revoked, with the times that closed each
 * one. No hash and no value: the value was shown once at mint and
 * exists nowhere on the bridge to show again.
 */
export async function listEnrollTokens(store, now = new Date().toISOString()) {
  return { enroll_tokens: foldEnrollTokens(await store.enrollTokenList(), now) };
}

/**
 * Exchanges a bootstrap token for the agent's working credentials.
 * Every failure — wrong, used, expired or revoked token, or a name
 * that isn't the name the token was minted for — returns null, and the
 * route answers the plain 404 every wrong token gets on this Worker.
 * On success the credential values are returned here, once; the store
 * keeps only their hashes. The token is burned before any credential
 * is minted, so a failure mid-exchange costs a re-mint, never a
 * second use.
 */
export async function exchangeEnrollToken(store, data, now = new Date().toISOString()) {
  if (!data || typeof data !== "object" || Array.isArray(data)) return null;
  if (typeof data.bootstrap_token !== "string" || data.bootstrap_token.length < 32 || data.bootstrap_token.length > 512) {
    return null;
  }
  let agent;
  try {
    agent = normalizeName(data.agent);
  } catch {
    return null;
  }
  const claimed = await store.enrollTokenClaim(await noteTokenHash(data.bootstrap_token), agent, now);
  if (!claimed) return null;
  const fleetValue = randomTokenValue();
  await store.fleetCredentialPut({
    token_hash: await noteTokenHash(fleetValue),
    agent,
    scopes: [...CREDENTIAL_SCOPES],
    created: now,
  });
  await store.fleet("hello", { agent, groups: claimed.groups });
  let notesToken = null;
  if (claimed.notes_identity) {
    notesToken = randomTokenValue();
    await store.noteRegister({
      identity: claimed.notes_identity,
      token_hash: await noteTokenHash(notesToken),
      can_send: true,
      can_inbox: true,
    });
  }
  return {
    agent,
    kind: claimed.kind,
    groups: claimed.groups,
    notes_identity: claimed.notes_identity,
    fleet_token: fleetValue,
    notes_token: notesToken,
  };
}

/** The hash a revoke call names: a bootstrap value to hash, or a stored hash itself. */
async function enrollRevokeHash(data) {
  if (!data || typeof data !== "object" || Array.isArray(data)) throw new InputError("body must be a JSON object");
  if (typeof data.bootstrap_token === "string" && data.bootstrap_token.length >= 32 && data.bootstrap_token.length <= 512) {
    return noteTokenHash(data.bootstrap_token);
  }
  if (typeof data.token_hash === "string" && /^[0-9a-f]{64}$/.test(data.token_hash)) return data.token_hash;
  throw new InputError("give bootstrap_token or token_hash");
}

// ---- HTTP ----

const json = (data, status = 200) =>
  new Response(JSON.stringify(data), {
    status,
    headers: {
      "content-type": "application/json",
      "cache-control": "no-store",
      "referrer-policy": "no-referrer",
      "x-robots-tag": "noindex, nofollow",
      "x-content-type-options": "nosniff",
    },
  });

const notFound = () => new Response("Not found\n", { status: 404, headers: { "content-type": "text/plain" } });
const notAllowed = (allow) => new Response("Method Not Allowed", { status: 405, headers: { allow } });

async function readJson(request) {
  const text = await request.text();
  if (text.length > 64 * 1024) throw new InputError("request too large");
  let data;
  try {
    data = JSON.parse(text);
  } catch {
    throw new InputError("body must be JSON");
  }
  if (!data || typeof data !== "object" || Array.isArray(data)) throw new InputError("body must be a JSON object");
  return data;
}

const query = (url) => Object.fromEntries(url.searchParams.entries());

/**
 * An agent's send: to central as before, or (with to, or reply_to a row
 * another assistant or agent sent) through relay.js to any address.
 * `fromAddr` is the sender address the credential fixes (a service
 * credential's service:<name>, #14373); without one the sender is
 * agent:<the agent>, as it has always been.
 */
async function agentSend(store, d, delivered, fromAddr = null) {
  if (d.broaden !== undefined && d.broaden !== null && (d.to === undefined || d.to === null || d.to === "")) {
    // Broadening one of the sender's own earlier distributions: the
    // store re-delivers its body to the matched-but-undelivered rest.
    // from_addr is the credential's fixed address when there is one
    // (agent:<name>, or service:<name> for a service credential),
    // exactly as the relay path below passes it: the store lets only
    // the distribution's original sender broaden it.
    const agent = normalizeName(d.agent);
    const out = await store.fleet("broaden", {
      batch: d.broaden,
      limit: d.limit,
      exclude: agent,
      from_addr: fromAddr ?? `agent:${agent}`,
    });
    delivered({ messages: [], fleet: out.messages });
    return out;
  }
  if (d.to === undefined || d.to === null || d.to === "") {
    let origin = null;
    if (d.reply_to !== undefined && d.reply_to !== null) {
      origin = await store.fleet("origin", { agent: d.agent, id: d.reply_to });
    }
    if (!origin?.from_addr) {
      // To central: always a message, whatever kind says (as before to existed).
      return await store.fleet("post", { agent: d.agent, body: d.body, reply_to: d.reply_to });
    }
    d = { ...d, to: replyAddress(origin.from_addr) };
  }
  const agent = normalizeName(d.agent);
  if (d.reply_to !== undefined && d.reply_to !== null) {
    await store.fleet("origin", { agent, id: d.reply_to }); // must be a row to this agent
  }
  const out = await relay(store, {
    from: fromAddr ?? `agent:${agent}`,
    to: d.to,
    body: d.body,
    kind: d.kind,
    sender: d.sender ?? agent,
    limit: d.limit,
    broaden: d.broaden,
  });
  if (d.reply_to !== undefined && d.reply_to !== null) await store.fleet("ack", { agent, ids: [d.reply_to] });
  delivered(out);
  return out;
}

/**
 * What an agent sees of a row in its own inbox. A command names no
 * sender (no label, from_addr null), whoever passed it on, so the agent
 * treats it as Drew's; the bridge still knows the origin and reply_to
 * still answers it. Drew's log shows the sender.
 */
export function commandForAgent(m) {
  if (m?.kind !== "command") return m;
  return { ...m, sender: null, from_addr: null };
}

/** The credential's own name when raw names it (normalized), else null. */
function credOwnName(raw, cred) {
  try {
    return normalizeName(raw) === cred.agent ? cred.agent : null;
  } catch {
    return null;
  }
}

/**
 * Agent routes. With the shared fleet token (cred null) any agent name
 * may be named, as before. With a per-agent credential the request may
 * act only as that agent, within its scopes; anything else is the
 * plain 404 a wrong token gets, so a credential learns nothing about
 * other agents' mailboxes.
 */
async function agentRoute(request, store, what, url, delivered, cred = null) {
  const scoped = (scope) => !cred || cred.scopes.includes(scope);
  if (what === "hello") {
    if (request.method !== "POST") return notAllowed("POST");
    const d = await readJson(request);
    if (cred && (!scoped("inbox") || credOwnName(d.agent, cred) === null)) return notFound();
    return json(await store.fleet("hello", d));
  }
  if (what === "inbox") {
    if (request.method !== "GET") return notAllowed("GET");
    if (cred && (!scoped("inbox") || credOwnName(url.searchParams.get("agent"), cred) === null)) return notFound();
    const out = await store.fleet("inbox", query(url));
    return json({ ...out, messages: out.messages.map(commandForAgent) });
  }
  if (what === "ack") {
    if (request.method !== "POST") return notAllowed("POST");
    const d = await readJson(request);
    if (cred && (!scoped("inbox") || credOwnName(d.agent, cred) === null)) return notFound();
    return json(await store.fleet("ack", d));
  }
  if (what === "work") {
    if (request.method !== "POST") return notAllowed("POST");
    const d = await readJson(request);
    if (cred && (!scoped("inbox") || credOwnName(d.agent, cred) === null)) return notFound();
    try {
      return json(await store.fleet("work", d));
    } catch (e) {
      // With a credential every work refusal is the plain 404, like
      // every other credential refusal: the caller learns nothing
      // about rows that are not its own.
      if (cred && e instanceof InputError) return notFound();
      throw e;
    }
  }
  if (what === "send") {
    if (request.method !== "POST") return notAllowed("POST");
    const d = await readJson(request);
    if (cred) {
      if (!scoped("send") || credOwnName(d.agent, cred) === null) return notFound();
      // from_addr is fixed by the relay to the credential's address
      // (agent:<this name>, or service:<name> for a service credential,
      // #14373); the sender label follows it, whatever the body claims.
      d.sender = cred.addr ?? cred.agent;
      return json(await agentSend(store, d, delivered, cred.addr ?? `agent:${cred.agent}`), 201);
    }
    return json(await agentSend(store, d, delivered), 201);
  }
  if (what === "offers") {
    if (request.method !== "GET") return notAllowed("GET");
    if (cred && (!scoped("inbox") || credOwnName(url.searchParams.get("agent"), cred) === null)) return notFound();
    return json(await store.fleet("offers", query(url)));
  }
  if (what === "offer") {
    if (request.method !== "POST") return notAllowed("POST");
    const d = await readJson(request);
    let fromAddr = null;
    if (cred) {
      if (!scoped("send") || credOwnName(d.agent, cred) === null) return notFound();
      // from_addr is fixed to the credential's address (agent:<this
      // name>, or service:<name> for a service credential, #14373);
      // the sender label follows it, whatever the body claims.
      d.sender = cred.addr ?? cred.agent;
      fromAddr = cred.addr ?? `agent:${cred.agent}`;
    }
    const agent = normalizeName(d.agent);
    const out = await store.fleet("offer", {
      body: d.body,
      to: d.to,
      kind: d.kind,
      slots: d.slots,
      ttl_seconds: d.ttl_seconds,
      sender: d.sender ?? agent,
      from_addr: fromAddr ?? `agent:${agent}`,
    });
    return json(out, 201);
  }
  if (what === "claim") {
    if (request.method !== "POST") return notAllowed("POST");
    const d = await readJson(request);
    if (cred && (!scoped("inbox") || credOwnName(d.agent, cred) === null)) return notFound();
    const out = await store.fleet("claim", d);
    // The delivered row is projected like any inbox row: a claimed
    // command names no sender.
    return json({ ...out, message: commandForAgent(out.message) });
  }
  // The owner's views, for a credential holding the "read" scope (a
  // service credential, #14373): the same store ops the owner routes
  // answer, nothing more. #14374 adds the unified projections the
  // console's Messages and Agents surfaces read — the pipeline feed,
  // the unified agents list and per-agent credential state — the very
  // functions the owner's /api routes answer with. #14376 adds the
  // minted enroll tokens' state the same way. All are reads; no
  // credential value or hash is in any of them, and revoking stays an
  // owner-token route: a service credential can see that a credential
  // stands revoked, never revoke one. Enroll-minted agent credentials
  // never hold the scope, and the shared fleet token does not open
  // these.
  if (
    what === "agents" ||
    what === "messages" ||
    what === "pipeline" ||
    what === "directory" ||
    what === "credentials" ||
    what === "enroll-tokens"
  ) {
    if (request.method !== "GET") return notAllowed("GET");
    if (!cred || !scoped("read")) return notFound();
    if (what === "agents") return json(await store.fleet("agents", {}));
    if (what === "messages") return json(await store.fleet("messages", query(url)));
    if (what === "pipeline") return json(await pipelineFeed(store, Object.fromEntries(url.searchParams.entries())));
    if (what === "directory") return json(await unifiedAgents(store));
    if (what === "enroll-tokens") return json(await listEnrollTokens(store));
    return json({ credentials: await store.fleetCredentialState() });
  }
  return notFound();
}

async function ownerRoute(request, store, what, url, delivered) {
  if (what === "agents") {
    if (request.method !== "GET") return notAllowed("GET");
    return json(await store.fleet("agents", {}));
  }
  if (what === "messages") {
    if (request.method !== "GET") return notAllowed("GET");
    return json(await store.fleet("messages", query(url)));
  }
  if (what === "send") {
    if (request.method !== "POST") return notAllowed("POST");
    const d = await readJson(request);
    if (d.broaden !== undefined && d.broaden !== null) {
      const out = await store.fleet("broaden", { batch: d.broaden, to: d.to, limit: d.limit });
      delivered({ messages: [], fleet: out.messages });
      return json(out, 201);
    }
    const out = await store.fleet("send", { body: d.body, to: d.to, kind: d.kind, sender: d.sender, limit: d.limit });
    delivered({ messages: [], fleet: out.messages });
    return json(out, 201);
  }
  if (what === "offer") {
    if (request.method !== "POST") return notAllowed("POST");
    const d = await readJson(request);
    // An offer delivers nothing yet, so there is nobody to wake.
    const out = await store.fleet("offer", {
      body: d.body,
      to: d.to,
      kind: d.kind,
      sender: d.sender,
      slots: d.slots,
      ttl_seconds: d.ttl_seconds,
    });
    return json(out, 201);
  }
  if (what === "offers") {
    if (request.method !== "GET") return notAllowed("GET");
    return json(await store.fleet("offer-list", {}));
  }
  return notFound();
}

/**
 * Handles /fleet/<fleet token or agent credential>/<what>,
 * /api/<owner>/fleet/<what>, the owner's credential and enroll-token
 * routes, and POST /enroll/exchange; returns null for any other path
 * so the caller routes it as before. A wrong token — shared, credential
 * or bootstrap — is a plain 404, like everywhere else.
 */
export async function handleFleet(request, env, url, getStore, deliveredFor = () => () => {}) {
  const parts = url.pathname.split("/").filter(Boolean);
  try {
    if (parts[0] === "enroll") {
      if (parts.length !== 2 || parts[1] !== "exchange") return notFound();
      if (request.method !== "POST") return notAllowed("POST");
      let data = null;
      try {
        data = await readJson(request);
      } catch {
        return notFound();
      }
      const out = await exchangeEnrollToken(getStore(), data);
      return out ? json(out, 201) : notFound();
    }
    if (parts[0] === "fleet") {
      if (parts.length !== 3) return notFound();
      const store = getStore();
      const expected = await fleetToken(env);
      if (expected && safeEqual(parts[1], expected)) {
        return await agentRoute(request, store, parts[2], url, deliveredFor(store));
      }
      // A per-agent credential: looked up by hash, bound to one agent.
      if (parts[1].length < MIN_TOKEN_LENGTH || parts[1].length > 512) return notFound();
      const cred = await store.fleetCredentialAuth(await noteTokenHash(parts[1]));
      if (!cred) return notFound();
      return await agentRoute(request, store, parts[2], url, deliveredFor(store), cred);
    }
    if (parts[0] === "api") {
      const m = matchTokenPath(url.pathname, env);
      if (!m || m.role !== "owner") return null; // not ours: the caller 404s
      const store = getStore();
      if (parts[2] === "fleet") {
        if (parts.length === 4 && parts[3] === "credentials") {
          if (request.method !== "GET") return notAllowed("GET");
          return json({ credentials: await store.fleetCredentialState() });
        }
        if (parts.length === 4) return await ownerRoute(request, store, parts[3], url, deliveredFor(store));
        if (parts.length === 5 && parts[3] === "credentials" && parts[4] === "revoke") {
          if (request.method !== "POST") return notAllowed("POST");
          const agent = normalizeName((await readJson(request)).agent);
          return json({ agent, revoked: await store.fleetCredentialRevoke(agent) });
        }
        if (parts.length === 5 && parts[3] === "credentials" && parts[4] === "mint") {
          if (request.method !== "POST") return notAllowed("POST");
          return json(await mintFleetCredential(store, await readJson(request)), 201);
        }
        return notFound();
      }
      if (parts[2] === "enroll-tokens") {
        if (parts.length === 3) {
          if (request.method === "GET") return json(await listEnrollTokens(store));
          if (request.method !== "POST") return notAllowed("POST");
          return json(await mintEnrollToken(store, await readJson(request)), 201);
        }
        if (parts.length === 4 && parts[3] === "revoke") {
          if (request.method !== "POST") return notAllowed("POST");
          const data = await readJson(request);
          // By name (#14376): revoke the agent's outstanding tokens.
          // The hash forms below name one token exactly.
          if (typeof data.agent === "string" && data.agent !== "") {
            const agent = normalizeName(data.agent);
            return json({ agent, revoked: await store.enrollTokenRevokeAgent(agent) });
          }
          const hash = await enrollRevokeHash(data);
          return json({ revoked: await store.enrollTokenRevoke(hash) });
        }
        return notFound();
      }
      if (parts[2] === "notes-tokens" && parts.length === 4 && parts[3] === "revoke") {
        if (request.method !== "POST") return notAllowed("POST");
        const identity = normalizeNoteIdentity((await readJson(request)).identity);
        return json({ identity, revoked: await store.noteRevoke(identity) });
      }
      if (parts[2] === "enroll-tokens" || parts[2] === "notes-tokens") return notFound();
      return null; // another /api area: the caller routes it
    }
  } catch (err) {
    if (err instanceof InputError) return json({ error: err.message }, 400);
    throw err;
  }
  return null;
}
