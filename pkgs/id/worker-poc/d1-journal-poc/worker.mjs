import { D1BridgeStore } from "../../hosted-worker/src/d1-bridge-store.ts";
import { importIdKey } from "../../hosted-worker/src/ed25519.ts";
import { signEvent } from "../../hosted-worker/src/event.ts";

const WORLD_ID = "d1-journal-poc-world";
const JOURNAL_VERSION = 1;

const CREATE_TABLE = `
  CREATE TABLE IF NOT EXISTS journal_entries (
    world_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal > 0),
    entry_bytes BLOB NOT NULL,
    PRIMARY KEY (world_id, ordinal)
  )
`;

let schemaReady;

class HttpError extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}

async function ensureSchema(db) {
  if (!schemaReady) schemaReady = db.prepare(CREATE_TABLE).run();
  try {
    await schemaReady;
  } catch (error) {
    schemaReady = undefined;
    throw error;
  }
}

function jsonResponse(value, status = 200) {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json; charset=utf-8" },
  });
}

function parseEntryBytes(bytes) {
  if (bytes.length < 2 || bytes[bytes.length - 1] !== 0x0a) {
    throw new HttpError(400, "expected one newline-terminated JSONL entry");
  }
  if (bytes.subarray(0, -1).includes(0x0a)) {
    throw new HttpError(400, "expected exactly one JSONL entry per append");
  }

  let entry;
  try {
    const line = new TextDecoder("utf-8", { fatal: true }).decode(bytes.subarray(0, -1));
    entry = JSON.parse(line);
  } catch {
    throw new HttpError(400, "entry must be valid UTF-8 JSON");
  }

  if (!entry || typeof entry !== "object" || Array.isArray(entry)) {
    throw new HttpError(400, "entry must be a JSON object");
  }
  if (entry.entry === "created") {
    if (entry.world_id !== WORLD_ID || entry.version !== JOURNAL_VERSION) {
      throw new HttpError(400, "created entry must name the synthetic world and journal version 1");
    }
  } else if (entry.entry === "caps_granted") {
    if (!Array.isArray(entry.granted) || entry.granted.some((name) => typeof name !== "string")) {
      throw new HttpError(400, "caps_granted entry must contain a string array");
    }
  } else {
    throw new HttpError(400, "POC accepts only created and caps_granted entries");
  }

  return bytes;
}

function sessionFor(db, request) {
  const bookmark = request.headers.get("x-d1-bookmark");
  return bookmark ? db.withSession(bookmark) : db.withSession();
}

async function readRows(session, after) {
  const result = await session
    .prepare(
      `SELECT ordinal, entry_bytes
       FROM journal_entries
       WHERE world_id = ? AND ordinal > ?
       ORDER BY ordinal ASC`,
    )
    .bind(WORLD_ID, after)
    .all();
  return result.results ?? [];
}

function blobBytes(value) {
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  if (ArrayBuffer.isView(value)) {
    return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  }
  // The local Wrangler D1 runtime currently exposes a returned BLOB as a
  // number array; accept it only when every element is a byte.
  if (
    Array.isArray(value) &&
    value.every((byte) => Number.isInteger(byte) && byte >= 0 && byte <= 0xff)
  ) {
    return Uint8Array.from(value);
  }
  throw new Error("D1 returned a non-BLOB journal value");
}

function jsonlResponse(rows, emptyCursor = 0) {
  const chunks = rows.map((row) => blobBytes(row.entry_bytes));
  const byteLength = chunks.reduce((total, chunk) => total + chunk.byteLength, 0);
  const bytes = new Uint8Array(byteLength);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }

  const firstOrdinal = rows.length ? Number(rows[0].ordinal) : 0;
  const lastOrdinal = rows.length ? Number(rows[rows.length - 1].ordinal) : emptyCursor;
  return new Response(bytes, {
    headers: {
      "content-type": "application/x-ndjson; charset=utf-8",
      "x-journal-first-ordinal": String(firstOrdinal),
      "x-journal-last-ordinal": String(lastOrdinal),
      "x-journal-entry-count": String(rows.length),
      // This POC's stream cursor is an ordinal; it is not a D1 bookmark.
      "x-journal-stream-cursor": String(lastOrdinal),
    },
  });
}

function parseAfter(url) {
  const raw = url.searchParams.get("after") ?? "0";
  const after = Number(raw);
  if (!Number.isSafeInteger(after) || after < 0) {
    throw new HttpError(400, "after must be a non-negative safe integer");
  }
  return after;
}

// #15538 slice 3: the hosted bridge event store on per-tenant D1.
// Two tenant databases are two separate D1 bindings. In the hosted
// path the binding comes from the authoritative tenant record; this
// local PoC names it by a fixed test letter, and an unknown letter is
// refused rather than falling back to some other database.
function tenantStore(env, url) {
  const tenant = url.searchParams.get("tenant");
  const binding = tenant === "a" ? env.TENANT_A : tenant === "b" ? env.TENANT_B : undefined;
  if (!binding) throw new HttpError(400, "tenant must be a or b");
  return new D1BridgeStore(binding);
}

// A fixed test-only signing seed: the fixture chain is synthetic and
// signs nothing outside this local harness.
const FIXTURE_SEED = new Uint8Array(32).fill(0x5d);

async function fixtureChain() {
  const key = await importIdKey(FIXTURE_SEED);
  const events = [];
  for (let seq = 0; seq < 2; seq++) {
    events.push(
      await signEvent(key, {
        seq,
        backlink: seq === 0 ? null : events[seq - 1].idBytes,
        kind: "message",
        body: new TextEncoder().encode(`d1 bridge fixture ${seq}`),
      }),
    );
  }
  return {
    author: key.publicHex,
    events: events.map((event) => ({ id: event.id, seq: event.seq, bytes_b64: btoa(String.fromCharCode(...event.bytes)) })),
  };
}

async function bridgeRoute(request, env, url) {
  if (request.method === "GET" && url.pathname === "/bridge/fixture") {
    return jsonResponse(await fixtureChain());
  }
  if (request.method === "POST" && url.pathname === "/bridge/append") {
    const store = tenantStore(env, url);
    const outcome = await store.append(new Uint8Array(await request.arrayBuffer()));
    return jsonResponse(outcome, outcome.kind === "refused" ? 409 : 200);
  }
  if (request.method === "GET" && url.pathname === "/bridge/head") {
    const author = url.searchParams.get("author") ?? "";
    return jsonResponse(await tenantStore(env, url).chainOf(author));
  }
  if (request.method === "GET" && url.pathname === "/bridge/event") {
    const author = url.searchParams.get("author") ?? "";
    const seq = Number(url.searchParams.get("seq"));
    if (!Number.isSafeInteger(seq) || seq < 0) throw new HttpError(400, "seq must be a non-negative safe integer");
    const stored = await tenantStore(env, url).storedAt(author, seq);
    if (!stored) return jsonResponse({ error: "not found" }, 404);
    return new Response(stored.bytes, {
      headers: { "content-type": "application/octet-stream", "x-event-id": stored.id },
    });
  }
  return undefined;
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (request.method === "GET" && url.pathname === "/health") {
      return new Response("ok");
    }

    try {
      if (url.pathname.startsWith("/bridge/")) {
        const response = await bridgeRoute(request, env, url);
        if (response) return response;
      }

      if (request.method === "POST" && url.pathname === "/append") {
        await ensureSchema(env.DB);
        const bytes = parseEntryBytes(new Uint8Array(await request.arrayBuffer()));
        const entryBytes = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
        // #806 assumes one journal writer per world. The INSERT allocates its
        // next ordinal from that world's durable tail and returns it atomically.
        const session = env.DB.withSession("first-primary");
        const inserted = await session
          .prepare(
            `INSERT INTO journal_entries (world_id, ordinal, entry_bytes)
             VALUES (?, (SELECT COALESCE(MAX(ordinal), 0) + 1
                         FROM journal_entries WHERE world_id = ?), ?)`,
          )
          .bind(WORLD_ID, WORLD_ID, entryBytes)
          .run();
        // Re-query the last inserted ordinal under the same session to return it.
        const last = await session
          .prepare(
            `SELECT ordinal FROM journal_entries
             WHERE world_id = ? AND rowid = ?`,
          )
          .bind(WORLD_ID, inserted.meta.last_row_id)
          .first();
        const bookmarkResult = session.getBookmark();
        const bookmark =
          typeof bookmarkResult === "string" ? bookmarkResult : bookmarkResult?.bookmark;
        if (!last || typeof bookmark !== "string" || bookmark.length === 0) {
          throw new Error("D1 append did not return an ordinal and session bookmark");
        }
        return jsonResponse({ world_id: WORLD_ID, ordinal: Number(last.ordinal), bookmark });
      }

      if (request.method === "GET" && url.pathname === "/snapshot") {
        await ensureSchema(env.DB);
        const rows = await sessionFor(env.DB, request)
          .prepare(
            `SELECT ordinal, entry_bytes
             FROM journal_entries
             WHERE world_id = ?
             ORDER BY ordinal ASC`,
          )
          .bind(WORLD_ID)
          .all();
        return jsonlResponse(rows.results ?? []);
      }

      if (request.method === "GET" && url.pathname === "/tail") {
        await ensureSchema(env.DB);
        const after = parseAfter(url);
        const rows = await readRows(sessionFor(env.DB, request), after);
        return jsonlResponse(rows, after);
      }

      return jsonResponse({ error: "not found" }, 404);
    } catch (error) {
      const status = error instanceof HttpError ? error.status : 500;
      return jsonResponse({ error: error instanceof Error ? error.message : String(error) }, status);
    }
  },
};
