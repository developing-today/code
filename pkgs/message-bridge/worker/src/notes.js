// Class/name notes for the message bridge.
//
// This is deliberately a signal + mailbox, not another messaging
// platform. An identity is `class/name` (`muse/archer`). A sender token
// maps, by SHA-256 hash in the store, to one identity and may send to a
// single identity or to a wildcard address (`muse/*`, `*/archer`,
// `*/*`); one stored row lands in each matching registered identity's
// inbox. The owner access token is only the bootstrap administrator for
// registering an identity/token pair.
//
// The public side is `GET /<class>/<name>`: a bare UUID in the same
// shape for every non-reserved two-segment path. A registered path's
// signal counter changes when a note is delivered. Every path also
// rotates with a deterministic per-path window (10-40 minutes), so a
// changed UUID does not prove that a note arrived, and an unchanged
// UUID does not prove that none did beyond the current window. The
// response carries no content and no registration marker.

import { safeEqual, tokenConfigured } from "./auth.js";
import { randomBatch } from "./fleet.js";
import { GLOB_SEGMENT, globMatch, isGlob } from "./glob.js";
import { InputError, MAX_BODY, normalizeWorkMark } from "./messages.js";

export const NOTE_SEGMENT = /^[a-z0-9][a-z0-9._-]{0,63}$/;
const RESERVED_SIGNAL_CLASSES = new Set([
  ".well-known",
  "api",
  "authorize",
  "enroll",
  "fleet",
  "healthz",
  "mcp",
  "notes",
  "p",
  "register",
  "token",
]);

const PRIVATE_HEADERS = {
  "cache-control": "no-store",
  "referrer-policy": "no-referrer",
  "x-robots-tag": "noindex, nofollow",
  "x-content-type-options": "nosniff",
};

const json = (data, status = 200) =>
  new Response(JSON.stringify(data), {
    status,
    headers: { "content-type": "application/json", ...PRIVATE_HEADERS },
  });

const notFound = () => new Response("Not found\n", { status: 404, headers: { "content-type": "text/plain" } });

function segment(value, what) {
  if (typeof value !== "string") throw new InputError(`${what} must be a string`);
  const s = value.trim().toLowerCase();
  if (!NOTE_SEGMENT.test(s)) {
    throw new InputError(`${what} must be 1-64 of a-z 0-9 . _ - starting with a letter or digit`);
  }
  return s;
}

/** Validates `class/name`; returns it lower-cased. */
export function normalizeNoteIdentity(value) {
  if (typeof value !== "string") throw new InputError("identity must be class/name");
  const parts = value.trim().toLowerCase().split("/");
  if (parts.length !== 2) throw new InputError("identity must be class/name");
  return `${segment(parts[0], "identity class")}/${segment(parts[1], "identity name")}`;
}

/**
 * Validates a recipient address: an identity whose segments are exact
 * names, `*`, or globs (`f*`) — the same matching rule as fleet
 * addressing (design §4.1), applied per segment.
 */
export function normalizeNoteAddress(value) {
  if (typeof value !== "string") throw new InputError("to must be class/name");
  const parts = value.trim().toLowerCase().split("/");
  if (parts.length !== 2) throw new InputError("to must be class/name, with * allowed in either segment");
  const parse = (raw, what) => {
    if (raw === "*") return "*";
    if (isGlob(raw)) {
      if (!GLOB_SEGMENT.test(raw)) throw new InputError(`${what} must be 1-64 of a-z 0-9 . _ - and *`);
      return raw;
    }
    return segment(raw, what);
  };
  return `${parse(parts[0], "address class")}/${parse(parts[1], "address name")}`;
}

/** True when a wildcard or glob address names an identity. */
export function noteAddressMatches(address, identity) {
  const [ac, an] = address.split("/");
  const [ic, iname] = identity.split("/");
  return globMatch(ac, ic) && globMatch(an, iname);
}

function boolField(value, dflt, what) {
  if (value === undefined || value === null) return dflt;
  if (typeof value !== "boolean") throw new InputError(`${what} must be true or false`);
  return value;
}

/** Validates a registration body; the caller hashes `token` before storage. */
export function normalizeNoteRegister(data) {
  if (!data || typeof data !== "object" || Array.isArray(data)) throw new InputError("body must be a JSON object");
  if (typeof data.token !== "string" || data.token.length < 32 || data.token.length > 512) {
    throw new InputError("token must be a string of 32-512 characters");
  }
  return {
    identity: normalizeNoteIdentity(data.identity),
    token: data.token,
    can_send: boolField(data.can_send, true, "can_send"),
    can_inbox: boolField(data.can_inbox, true, "can_inbox"),
  };
}

/** The note kinds (design §4.4): a plain note is input; an instruction is a work item. */
export const NOTE_KINDS = ["note", "instruction"];

const NOTE_BATCH = /^[A-Za-z0-9_-]{8,40}$/;
export const NOTE_MAX_LIMIT = 200;

function noteLimit(value) {
  if (value === undefined || value === null) return null;
  if (!Number.isInteger(value) || value < 1) throw new InputError("limit must be a positive integer");
  if (value > NOTE_MAX_LIMIT) throw new InputError(`limit must be at most ${NOTE_MAX_LIMIT}`);
  return value;
}

/**
 * Validates a send body: `{to, note, kind?, reply_to?, limit?}`, or a
 * broaden follow-up `{broaden, to?, limit?}` (no note: the original
 * body is re-delivered to the distribution's matched-but-undelivered
 * remainder). `to` may be omitted only when `reply_to` names a note
 * in the sender's inbox — the reply then routes store-side to that
 * note's real sender, whom the replier may never have seen (an
 * instruction's sender is projected away in the inbox).
 */
export function normalizeNoteSend(data) {
  if (!data || typeof data !== "object" || Array.isArray(data)) throw new InputError("body must be a JSON object");
  const limit = noteLimit(data.limit);
  if (data.broaden !== undefined && data.broaden !== null) {
    if (typeof data.broaden !== "string" || !NOTE_BATCH.test(data.broaden)) throw new InputError("broaden must be a batch id");
    return {
      broaden: data.broaden,
      to: data.to === undefined || data.to === null || data.to === "" ? null : normalizeNoteAddress(data.to),
      note: null,
      limit,
    };
  }
  if (typeof data.note !== "string" || data.note.trim() === "") throw new InputError("note must be a non-empty string");
  if (data.note.length > MAX_BODY) throw new InputError(`note is longer than ${MAX_BODY} characters`);
  const kind = data.kind === undefined || data.kind === null || data.kind === "" ? "note" : data.kind;
  if (!NOTE_KINDS.includes(kind)) throw new InputError(`kind must be one of ${NOTE_KINDS.join(", ")}`);
  let reply_to = null;
  if (data.reply_to !== undefined && data.reply_to !== null) {
    if (!Number.isInteger(data.reply_to) || data.reply_to < 1) throw new InputError("reply_to must be a positive integer");
    reply_to = data.reply_to;
  }
  const to =
    data.to === undefined || data.to === null || data.to === ""
      ? null
      : normalizeNoteAddress(data.to);
  if (to === null && reply_to === null) throw new InputError("give to, or reply_to naming a note in your inbox");
  return { broaden: null, to, note: data.note, kind, reply_to, limit };
}

/** Validates an ack body: `{id}`, `{ids}` or `{up_to_id}`. */
export function normalizeNoteAck(data) {
  if (!data || typeof data !== "object" || Array.isArray(data)) throw new InputError("body must be a JSON object");
  const ids = [];
  if (data.id !== undefined && data.id !== null) ids.push(data.id);
  if (Array.isArray(data.ids)) ids.push(...data.ids);
  const upTo = data.up_to_id === undefined || data.up_to_id === null ? null : data.up_to_id;
  if (!ids.length && upTo === null) throw new InputError("give id, ids or up_to_id");
  if (!ids.every((i) => Number.isInteger(i) && i > 0)) throw new InputError("ids must be positive integers");
  if (ids.length > 1000) throw new InputError("at most 1000 ids at once");
  if (upTo !== null && (!Number.isInteger(upTo) || upTo < 0)) throw new InputError("up_to_id must be a non-negative integer");
  return { ids: [...new Set(ids)], up_to_id: upTo };
}

/** SHA-256 hex of a bearer token; only this hash is stored or compared. */
export async function noteTokenHash(token) {
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(token)));
  return Array.from(digest, (b) => b.toString(16).padStart(2, "0")).join("");
}

function hash32(text, seed = 0x811c9dc5) {
  let h = seed >>> 0;
  for (let i = 0; i < text.length; i++) {
    h ^= text.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

function splitmix32(seed) {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x9e3779b9) >>> 0;
    let z = state;
    z = Math.imul(z ^ (z >>> 16), 0x85ebca6b);
    z = Math.imul(z ^ (z >>> 13), 0xc2b2ae35);
    return (z ^ (z >>> 16)) >>> 0;
  };
}

/** The deterministic decoy window for one path: 10-40 minutes. */
export function noteWindowSeconds(identity) {
  return 600 + (hash32(`note-signal-window:${identity}`) % 1800);
}

/**
 * The public UUID for an identity at a moment. Inputs are the path, the
 * current decoy bucket and the registered path's delivery counter (0
 * when unregistered or when no note has arrived). A delivery increments
 * the counter and therefore changes the UUID inside the same window;
 * the bucket changes it on the path's own jittered schedule.
 */
export function signalUuid(identity, counter = 0, now = Date.now()) {
  const bucket = Math.floor(now / (noteWindowSeconds(identity) * 1000));
  const next = splitmix32(hash32(`${identity}\0${bucket}\0${counter}`));
  const bytes = new Uint8Array(16);
  for (let i = 0; i < 4; i++) {
    const n = next();
    bytes[i * 4] = n >>> 24;
    bytes[i * 4 + 1] = n >>> 16;
    bytes[i * 4 + 2] = n >>> 8;
    bytes[i * 4 + 3] = n;
  }
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

function bearerToken(request) {
  const h = request.headers.get("authorization") ?? "";
  const m = /^Bearer\s+(.+)$/i.exec(h);
  return m ? m[1].trim() : null;
}

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

/**
 * Handles the authenticated note API:
 *   POST /notes/register  owner access token as Bearer; {identity, token, can_send?, can_inbox?}
 *   POST /notes/send      identity token; {to, note, kind?, reply_to?, limit?}
 *                         or {broaden, to?, limit?} — kind "instruction"
 *                         lands sender-free in the recipient's inbox
 *                         (the store keeps the sender); reply_to answers
 *                         a note in the sender's own inbox, routed
 *                         store-side to its real sender, and may
 *                         replace `to`
 *   GET  /notes/inbox     identity token; pending notes for that identity
 *   POST /notes/ack       identity token; {id | ids | up_to_id}
 *   POST /notes/work      identity token; {id, state, task_id?}: the
 *                         work-item lifecycle of one of the identity's
 *                         own notes (design §4.6) — state acted |
 *                         queued | done, forward only from received
 * Returns null when the path is not this API's.
 */
export async function handleNotes(request, env, url, getStore) {
  const parts = url.pathname.split("/").filter(Boolean);
  if (parts[0] !== "notes") return null;
  if (parts.length !== 2) return notFound();
  const what = parts[1];
  try {
    const store = getStore();
    if (what === "register") {
      if (request.method !== "POST") return new Response("Method Not Allowed", { status: 405, headers: { allow: "POST" } });
      const admin = bearerToken(request);
      if (!tokenConfigured(env) || !admin || !safeEqual(admin, env.ACCESS_TOKEN)) return notFound();
      const data = normalizeNoteRegister(await readJson(request));
      await store.noteRegister({
        identity: data.identity,
        token_hash: await noteTokenHash(data.token),
        can_send: data.can_send,
        can_inbox: data.can_inbox,
      });
      return json({ identity: data.identity, can_send: data.can_send, can_inbox: data.can_inbox }, 201);
    }

    const token = bearerToken(request);
    if (!token || token.length < 32 || token.length > 512) return notFound();
    const auth = await store.noteAuth(await noteTokenHash(token));
    if (!auth) return notFound();

    if (what === "send") {
      if (request.method !== "POST") return new Response("Method Not Allowed", { status: 405, headers: { allow: "POST" } });
      if (!auth.can_send) return json({ error: "this token cannot send notes" }, 403);
      const data = normalizeNoteSend(await readJson(request));
      if (data.broaden) {
        const extra_matched = data.to
          ? (await store.noteIdentities()).filter((identity) => noteAddressMatches(data.to, identity))
          : [];
        const out = await store.noteBroaden({
          from: auth.identity,
          batch: data.broaden,
          extra_matched,
          pattern: data.to,
          limit: data.limit,
        });
        if (out && typeof out.error === "string") throw new InputError(out.error);
        return json(out, 201);
      }
      if (data.reply_to !== null) {
        // A reply answers the original note's sender, resolved
        // store-side; the replier names only the note's id, never
        // the sender (who may be projected away in their inbox).
        const origin = await store.noteOrigin(auth.identity, data.reply_to);
        if (origin === null) throw new InputError(`reply_to ${data.reply_to} is not a note in your inbox`);
        const recipients = [origin];
        return json(
          await store.noteDeliver({ from: auth.identity, recipients, note: data.note, kind: data.kind, reply_to: data.reply_to }),
          201,
        );
      }
      // Identities come back in name order, so a limit takes the first
      // N matches deterministically; the response is the match report.
      const matched = (await store.noteIdentities()).filter((identity) => noteAddressMatches(data.to, identity));
      if (!matched.length) throw new InputError(`no registered identity matches ${data.to}`);
      const recipients = data.limit ? matched.slice(0, data.limit) : matched;
      const batch = randomBatch();
      const out = await store.noteDeliver({
        from: auth.identity,
        recipients,
        note: data.note,
        kind: data.kind,
        reply_to: data.reply_to,
        batch,
        pattern: data.to,
        limit_n: data.limit,
        matched,
      });
      return json({ ...out, batch, matched: matched.length, remaining: matched.length - recipients.length }, 201);
    }
    if (what === "inbox") {
      if (request.method !== "GET") return new Response("Method Not Allowed", { status: 405, headers: { allow: "GET" } });
      if (!auth.can_inbox) return json({ error: "this token has no inbox" }, 403);
      return json({ identity: auth.identity, notes: await store.noteInbox(auth.identity) });
    }
    if (what === "ack") {
      if (request.method !== "POST") return new Response("Method Not Allowed", { status: 405, headers: { allow: "POST" } });
      if (!auth.can_inbox) return json({ error: "this token has no inbox" }, 403);
      return json(await store.noteAck(auth.identity, normalizeNoteAck(await readJson(request))));
    }
    if (what === "work") {
      if (request.method !== "POST") return new Response("Method Not Allowed", { status: 405, headers: { allow: "POST" } });
      if (!auth.can_inbox) return json({ error: "this token has no inbox" }, 403);
      const out = await store.noteWork(auth.identity, normalizeWorkMark(await readJson(request)));
      if (out && typeof out.error === "string") throw new InputError(out.error);
      return json(out);
    }
    return notFound();
  } catch (err) {
    if (err instanceof InputError) return json({ error: err.message }, 400);
    throw err;
  }
}

/**
 * Handles the public signal for a non-reserved two-segment path. Every
 * such path goes through the same store lookup and response shape,
 * whether or not the identity is registered. Returns null when this is
 * not a signal path, so operational routes keep their own handling.
 */
export async function handleSignal(request, url, getStore) {
  if (request.method !== "GET" && request.method !== "HEAD") return null;
  const parts = url.pathname.split("/").filter(Boolean);
  if (parts.length !== 2) return null;
  let cls, name;
  try {
    cls = decodeURIComponent(parts[0]).toLowerCase();
    name = decodeURIComponent(parts[1]).toLowerCase();
  } catch {
    return null;
  }
  if (!cls || !name || RESERVED_SIGNAL_CLASSES.has(cls)) return null;
  const identity = `${cls}/${name}`;
  const state = await getStore().noteSignal(identity);
  const uuid = signalUuid(identity, state?.counter ?? 0);
  return new Response(request.method === "HEAD" ? null : `${uuid}\n`, {
    headers: { "content-type": "text/plain", ...PRIVATE_HEADERS },
  });
}
