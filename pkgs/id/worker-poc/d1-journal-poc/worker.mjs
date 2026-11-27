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

// #15538 slice 3: the per-tenant event-store seam. Each tenant's events
// live in that tenant's own D1 database; the binding IS the credential
// boundary, so the Worker never takes a tenant id to switch databases
// inside one binding — it picks the binding the route names.
const TENANT_SCHEMA = [
  `CREATE TABLE IF NOT EXISTS bridge_events (
     author TEXT NOT NULL, seq INTEGER NOT NULL, id TEXT NOT NULL,
     bytes BLOB NOT NULL,
     PRIMARY KEY (author, seq))`,
  `CREATE UNIQUE INDEX IF NOT EXISTS bridge_events_by_id ON bridge_events(id)`,
  `CREATE TABLE IF NOT EXISTS bridge_chains (
     author TEXT PRIMARY KEY, next_seq INTEGER NOT NULL, head TEXT)`,
];

let tenantSchemaReady = new WeakMap();

async function ensureTenantSchema(db) {
  let ready = tenantSchemaReady.get(db);
  if (!ready) {
    ready = (async () => {
      for (const statement of TENANT_SCHEMA) await db.prepare(statement).run();
    })();
    tenantSchemaReady.set(db, ready);
    try {
      await ready;
    } catch (error) {
      tenantSchemaReady.delete(db);
      throw error;
    }
  } else {
    await ready;
  }
}

function tenantDb(env, tenant) {
  if (tenant === "a") return env.TENANT_A_DB;
  if (tenant === "b") return env.TENANT_B_DB;
  throw new HttpError(400, "tenant must be a or b");
}

function hexToBytes(hex) {
  if (typeof hex !== "string" || !/^[0-9a-fA-F]*$/.test(hex) || hex.length % 2 !== 0 || hex.length === 0) {
    throw new HttpError(400, "bytes_hex must be non-empty even-length hex");
  }
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) bytes[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return bytes;
}

function bytesToHex(bytes) {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (request.method === "GET" && url.pathname === "/health") {
      return new Response("ok");
    }

    try {
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

      if (request.method === "POST" && url.pathname === "/tenant-append") {
        const tenant = url.searchParams.get("tenant");
        const db = tenantDb(env, tenant);
        await ensureTenantSchema(db);
        const body = await request.json();
        const { author, seq, id } = body ?? {};
        if (typeof author !== "string" || author.length === 0) throw new HttpError(400, "author is required");
        if (!Number.isSafeInteger(seq) || seq < 0) throw new HttpError(400, "seq must be a non-negative safe integer");
        if (typeof id !== "string" || id.length === 0) throw new HttpError(400, "id is required");
        const bytes = hexToBytes(body.bytes_hex);
        const head = await db.prepare("SELECT next_seq FROM bridge_chains WHERE author = ?").bind(author).first();
        const nextSeq = head ? Number(head.next_seq) : 0;
        if (seq !== nextSeq) throw new HttpError(422, `sequence_gap: author is at ${nextSeq}, append names ${seq}`);
        const payload = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
        await db.batch([
          db.prepare("INSERT INTO bridge_events (author, seq, id, bytes) VALUES (?, ?, ?, ?)").bind(author, seq, id, payload),
          db.prepare("INSERT OR REPLACE INTO bridge_chains (author, next_seq, head) VALUES (?, ?, ?)").bind(author, seq + 1, id),
        ]);
        return jsonResponse({ tenant, author, seq, id, head: id, next_seq: seq + 1 });
      }

      if (request.method === "GET" && url.pathname === "/tenant-event") {
        const tenant = url.searchParams.get("tenant");
        const db = tenantDb(env, tenant);
        await ensureTenantSchema(db);
        const author = url.searchParams.get("author") ?? "";
        const seq = Number(url.searchParams.get("seq") ?? "NaN");
        if (!author || !Number.isSafeInteger(seq) || seq < 0) throw new HttpError(400, "author and seq are required");
        const row = await db
          .prepare("SELECT id, bytes FROM bridge_events WHERE author = ? AND seq = ?")
          .bind(author, seq)
          .first();
        if (!row) return jsonResponse({ error: "not found" }, 404);
        return jsonResponse({ id: row.id, bytes_hex: bytesToHex(blobBytes(row.bytes)) });
      }

      if (request.method === "GET" && url.pathname === "/tenant-head") {
        const tenant = url.searchParams.get("tenant");
        const db = tenantDb(env, tenant);
        await ensureTenantSchema(db);
        const author = url.searchParams.get("author") ?? "";
        if (!author) throw new HttpError(400, "author is required");
        const row = await db.prepare("SELECT next_seq, head FROM bridge_chains WHERE author = ?").bind(author).first();
        if (!row) return jsonResponse({ next_seq: 0, head: null });
        return jsonResponse({ next_seq: Number(row.next_seq), head: row.head ?? null });
      }

      return jsonResponse({ error: "not found" }, 404);
    } catch (error) {
      const status = error instanceof HttpError ? error.status : 500;
      return jsonResponse({ error: error instanceof Error ? error.message : String(error) }, status);
    }
  },
};
