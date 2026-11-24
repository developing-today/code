// The owner's unified pipeline (design §4.3): one read over the three
// messaging surfaces the store holds — connector messages, fleet
// mailboxes and class/name notes — plus the owner's first read over
// notes and a unified connected-agents list.
//
// These are read projections over the existing tables: no row is
// copied, and nothing here touches a recipient-side projection. The
// anonymity rule stays where it has always lived (messages.js
// forAgent, fleet.js commandForAgent): a recipient never sees who sent
// an instruction or command. Owner surfaces are the opposite — every
// row below carries the real sender, on every surface.
//
// Routes (owner token only, wired in routes.js):
//   GET /api/<token>/pipeline  the normalized feed (this file)
//   GET /api/<token>/notes     owner read over note_message
//   GET /api/<token>/agents    fleet registry + note identities
//
// Normalized row: {surface, id, batch, ts, kind, sender_addr,
// sender_label, receiver, direction, read_at, reply_to, body,
// work_state, work_task_id}.
//   surface     "connector" | "fleet" | "notes"
//   kind        the surface's own kind: connector rows are "message" |
//               "instruction", fleet rows "message" | "command", notes
//               are "note" (notes gain a kind of their own in S4)
//   sender_addr who sent it, as an address: a from_addr
//               ("agent:<name>", "channel:<name>") when the row has
//               one, "user:drew" for what central sent, a notes
//               identity ("muse/archer") on the notes surface
//   receiver    the mirror image: "channel:<name>" for a connector
//               row addressed to that assistant, "agent:<name>" for a
//               fleet mailbox row, the identity for a note, and
//               "user:drew" for rows sent back to the owner
//   batch       the fleet batch id; connector and note rows have no
//               batch today, so null (a batch filter therefore only
//               matches fleet rows)
//   reply_to    the fleet row a reply answers; null elsewhere today
//   work_state  the instruction work-item lifecycle (design §4.6):
//               fleet and notes rows carry the state the receiving
//               agent recorded (unmarked reads as "received");
//               connector instructions derive it from read state
//               (unread "received", read "acted" — they store no
//               lifecycle fields in this build-out); null on rows
//               that are not work items (outbound rows, plain
//               connector messages)
//   work_task_id the fleet-queue task a `queued` instruction became;
//               null unless the receiving agent recorded one
//
// Filters: kind, sender, receiver, surface, batch, unread_only,
// work_state, since_id, before_id, limit, order. since_id/before_id are per-surface
// id cursors (each surface numbers its own rows), applied on every
// surface the query touches. sender/receiver match an address or its
// bare name: "felix", "agent:felix" and — for notes — "muse/felix" all
// name the same sender as far as the owner is concerned.

import { InputError, MAX_LIMIT, WORK_STATES } from "./messages.js";

export const PIPELINE_SURFACES = ["connector", "fleet", "notes"];
export const PIPELINE_KINDS = ["message", "instruction", "command", "note"];
const PIPELINE_DEFAULT_LIMIT = 50;
const OWNER_ADDR = "user:drew";
const AGENT_NAME = /^[a-z0-9][a-z0-9._-]{0,63}$/;

function intFilter(v, name, { min = 0, dflt = null } = {}) {
  if (v === undefined || v === null || v === "") return dflt;
  const n = Number(v);
  if (!Number.isInteger(n) || n < min) throw new InputError(`${name} must be an integer >= ${min}`);
  return n;
}

function boolFilter(v, name) {
  if (v === undefined || v === null || v === "") return false;
  if (v === true || v === "true" || v === "1") return true;
  if (v === false || v === "false" || v === "0") return false;
  throw new InputError(`${name} must be true or false`);
}

function textFilter(v, name, { lower = true } = {}) {
  if (v === undefined || v === null || v === "") return null;
  if (typeof v !== "string") throw new InputError(`${name} must be a string`);
  // Addresses are case-insensitive; a batch id is case-sensitive base64url.
  const s = lower ? v.trim().toLowerCase() : v.trim();
  if (s.length > 200) throw new InputError(`${name} is too long`);
  return s || null;
}

/** Validates a pipeline/notes query; throws InputError. */
export function normalizePipelineQuery(q = {}) {
  const surface = q.surface === undefined || q.surface === null || q.surface === "" ? "all" : String(q.surface).toLowerCase();
  if (surface !== "all" && !PIPELINE_SURFACES.includes(surface)) {
    throw new InputError(`surface must be all or one of ${PIPELINE_SURFACES.join(", ")}`);
  }
  const kind = q.kind === undefined || q.kind === null || q.kind === "" ? "all" : String(q.kind).toLowerCase();
  if (kind !== "all" && !PIPELINE_KINDS.includes(kind)) {
    throw new InputError(`kind must be all or one of ${PIPELINE_KINDS.join(", ")}`);
  }
  const order = q.order === undefined || q.order === null || q.order === "" ? "asc" : String(q.order).toLowerCase();
  if (order !== "asc" && order !== "desc") throw new InputError("order must be asc or desc");
  const work_state = q.work_state === undefined || q.work_state === null || q.work_state === "" ? "all" : String(q.work_state).toLowerCase();
  if (work_state !== "all" && !WORK_STATES.includes(work_state)) {
    throw new InputError(`work_state must be all or one of ${WORK_STATES.join(", ")}`);
  }
  return {
    surface,
    kind,
    work_state,
    sender: textFilter(q.sender, "sender"),
    receiver: textFilter(q.receiver, "receiver"),
    batch: textFilter(q.batch, "batch", { lower: false }),
    unread_only: boolFilter(q.unread_only, "unread_only"),
    since_id: intFilter(q.since_id, "since_id"),
    before_id: intFilter(q.before_id, "before_id", { min: 1 }),
    limit: Math.min(intFilter(q.limit, "limit", { min: 1, dflt: PIPELINE_DEFAULT_LIMIT }), MAX_LIMIT),
    order,
  };
}

/** The bare name of an address: "agent:felix" -> "felix", "muse/felix" -> "felix". */
function bareName(addr) {
  if (typeof addr !== "string") return null;
  const s = addr.toLowerCase();
  if (s.includes(":")) return s.slice(s.indexOf(":") + 1);
  if (s.includes("/")) return s.slice(s.lastIndexOf("/") + 1);
  return s;
}

/**
 * True when a sender/receiver filter names this address (or label).
 * A bare name ("felix") matches every surface's spelling of it
 * ("agent:felix", "muse/felix"); a full notes identity ("muse/felix")
 * is exact — it does not also mean the fleet agent of that name, and
 * an "agent:"/"channel:" filter never bare-matches an identity.
 */
export function addressMatches(filter, addr, label = null) {
  if (!filter) return true;
  const f = filter.toLowerCase();
  for (const cand of [addr, label]) {
    if (typeof cand !== "string" || cand === "") continue;
    const c = cand.toLowerCase();
    if (c === f) return true;
    if (f.includes("/")) continue; // an identity filter is exact only
    if (c.includes("/")) {
      if (!f.includes(":") && bareName(c) === bareName(f)) return true;
      continue;
    }
    if (bareName(c) === f || bareName(f) === c || bareName(c) === bareName(f)) return true;
  }
  return false;
}

// ---- row projections (owner view: the real sender, always) ----

export function projectConnector(m) {
  const inbound = m.direction === "inbound";
  return {
    surface: "connector",
    id: m.id,
    batch: null,
    ts: m.ts,
    kind: m.kind,
    sender_addr: m.from_addr ?? (inbound ? OWNER_ADDR : `channel:${m.channel}`),
    sender_label: m.sender ?? null,
    receiver: inbound ? `channel:${m.channel}` : OWNER_ADDR,
    direction: m.direction,
    read_at: m.read_at ?? null,
    reply_to: null,
    // Connector instructions store no lifecycle fields (§4.6): read
    // state stands in — unread is `received`, read is `acted`.
    work_state: inbound && m.kind === "instruction" ? (m.read_at ? "acted" : "received") : null,
    work_task_id: null,
    body: m.body,
    channel: m.channel,
  };
}

export function projectFleet(m) {
  const inbound = m.direction === "inbound";
  return {
    surface: "fleet",
    id: m.id,
    batch: m.batch ?? null,
    ts: m.ts,
    kind: m.kind,
    sender_addr: inbound ? (m.from_addr ?? OWNER_ADDR) : `agent:${m.agent}`,
    sender_label: m.sender ?? null,
    receiver: inbound ? `agent:${m.agent}` : OWNER_ADDR,
    direction: m.direction,
    read_at: m.read_at ?? null,
    reply_to: m.reply_to ?? null,
    work_state: inbound ? (m.work_state ?? "received") : null,
    work_task_id: inbound ? (m.work_task_id ?? null) : null,
    body: m.body,
    agent: m.agent,
  };
}

export function projectNote(r) {
  return {
    surface: "notes",
    id: r.id,
    batch: null,
    ts: r.ts,
    kind: "note",
    sender_addr: r.sender,
    sender_label: r.sender,
    receiver: r.recipient,
    direction: "inbound",
    read_at: r.ack_at ?? null,
    reply_to: null,
    work_state: r.work_state ?? "received",
    work_task_id: r.work_task_id ?? null,
    body: r.body,
  };
}

// ---- the feed ----

function matches(e, q) {
  if (q.kind !== "all" && e.kind !== q.kind) return false;
  if (q.work_state !== "all" && e.work_state !== q.work_state) return false;
  if (q.batch !== null && e.batch !== q.batch) return false;
  if (q.unread_only && !(e.direction === "inbound" && e.read_at === null)) return false;
  if (!addressMatches(q.sender, e.sender_addr, e.sender_label)) return false;
  if (!addressMatches(q.receiver, e.receiver)) return false;
  return true;
}

function byTime(order) {
  const dir = order === "desc" ? -1 : 1;
  return (a, b) => {
    if (a.ts !== b.ts) return (a.ts < b.ts ? -1 : 1) * dir;
    if (a.surface !== b.surface) return (a.surface < b.surface ? -1 : 1) * dir;
    return (a.id - b.id) * dir;
  };
}

/** A receiver filter that names one fleet agent, for pushing the fleet read down. */
function fleetAgentFilter(receiver) {
  if (!receiver) return null;
  const name = receiver.startsWith("agent:") ? receiver.slice("agent:".length) : receiver;
  return AGENT_NAME.test(name) ? name : null;
}

/** An exact notes identity filter (class/name), the only form pushed into the notes read. */
function identityFilter(v) {
  return v && v.includes("/") ? v : null;
}

/**
 * The unified feed. Each surface contributes its newest `limit` rows
 * under the pushed-down filters (id cursors, kind, unread); the merge
 * then applies the address/batch filters, sorts by ts and takes the
 * page. When an address or batch filter is present the per-surface
 * fetch widens to MAX_LIMIT, since those filters only apply after
 * projection.
 */
export async function pipelineFeed(store, rawQuery = {}) {
  const q = normalizePipelineQuery(rawQuery);
  const fetchLimit = q.sender || q.receiver || q.batch || q.work_state !== "all" ? MAX_LIMIT : q.limit;
  const want = (s) => q.surface === "all" || q.surface === s;
  const kindFor = (...kinds) => q.kind === "all" || kinds.includes(q.kind);
  const entries = [];
  if (want("connector") && kindFor("message", "instruction")) {
    const { messages } = await store.list({
      direction: "all",
      kind: q.kind === "all" ? undefined : q.kind,
      since_id: q.since_id ?? undefined,
      before_id: q.before_id ?? undefined,
      unread_only: q.unread_only,
      limit: fetchLimit,
      order: q.order,
    });
    entries.push(...messages.map(projectConnector));
  }
  if (want("fleet") && kindFor("message", "command") && typeof store.fleet === "function") {
    const out = await store.fleet("messages", {
      agent: fleetAgentFilter(q.receiver),
      direction: "all",
      kind: q.kind === "all" ? "all" : q.kind,
      since_id: q.since_id ?? 0,
      before_id: q.before_id,
      limit: fetchLimit,
      order: q.order,
    });
    entries.push(...out.messages.map(projectFleet));
  }
  if (want("notes") && kindFor("note") && typeof store.noteOwnerList === "function") {
    const rows = await store.noteOwnerList({
      sender: identityFilter(q.sender),
      receiver: identityFilter(q.receiver),
      unread_only: q.unread_only,
      since_id: q.since_id,
      before_id: q.before_id,
      limit: fetchLimit,
      order: q.order,
    });
    entries.push(...rows.map(projectNote));
  }
  const messages = entries.filter((e) => matches(e, q)).sort(byTime(q.order)).slice(0, q.limit);
  return { messages };
}

/**
 * The owner's read over note_message (there was none: the owner could
 * only look through an identity's own inbox). Same filters as the
 * pipeline; rows come back in the table's own shape. A kind filter
 * other than "note" matches nothing — every note is kind "note" until
 * S4 gives notes kinds of their own.
 */
export async function notesOwnerFeed(store, rawQuery = {}) {
  const q = normalizePipelineQuery(rawQuery);
  if ((q.surface !== "all" && q.surface !== "notes") || (q.kind !== "all" && q.kind !== "note")) return { notes: [] };
  const fetchLimit = q.sender || q.receiver ? MAX_LIMIT : q.limit;
  const rows = await store.noteOwnerList({
    sender: identityFilter(q.sender),
    receiver: identityFilter(q.receiver),
    unread_only: q.unread_only,
    since_id: q.since_id,
    before_id: q.before_id,
    limit: fetchLimit,
    order: q.order,
  });
  const notes = rows
    .filter((r) => addressMatches(q.sender, r.sender) && addressMatches(q.receiver, r.recipient))
    .filter((r) => q.work_state === "all" || (r.work_state ?? "received") === q.work_state)
    .slice(0, q.limit)
    .map((r) => ({ id: r.id, ts: r.ts, recipient: r.recipient, sender: r.sender, body: r.body, ack_at: r.ack_at ?? null, work_state: r.work_state ?? null, work_task_id: r.work_task_id ?? null }));
  return { notes };
}

/**
 * The unified connected-agents list: fleet registry rows (with unread
 * counts) and note identities (with their token scopes), one row per
 * surface registration, each marked by surface. Notes identities have
 * no presence — the row says so (presence "none", last_seen null)
 * rather than inventing one; their signal counter stands in as the
 * only activity signal the surface keeps.
 */
export async function unifiedAgents(store) {
  const agents = [];
  if (typeof store.fleet === "function") {
    const { agents: fleetAgents } = await store.fleet("agents", {});
    for (const a of fleetAgents) {
      agents.push({
        surface: "fleet",
        name: a.name,
        groups: a.groups,
        first_seen: a.first_seen,
        last_seen: a.last_seen,
        presence: "last_seen",
        unread_commands: a.unread_commands,
        unread_messages: a.unread_messages,
        unread: (a.unread_commands ?? 0) + (a.unread_messages ?? 0),
        inbound: a.inbound,
        outbound: a.outbound,
      });
    }
  }
  if (typeof store.noteDirectory === "function") {
    for (const d of await store.noteDirectory()) {
      agents.push({
        surface: "notes",
        name: d.identity,
        identity: d.identity,
        scopes: { can_send: d.can_send, can_inbox: d.can_inbox },
        signal_counter: d.signal_counter,
        first_seen: d.created ?? null,
        last_seen: null,
        presence: "none",
      });
    }
  }
  agents.sort((a, b) => a.surface.localeCompare(b.surface) || a.name.localeCompare(b.name));
  return { agents };
}
