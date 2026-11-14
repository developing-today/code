/**
 * `QueryCache` Durable Object (#6788): one instance per distinct
 * query key. Holds the query, its embedding in the pinned
 * Snowflake space, and the cached fan-out result — retained
 * indefinitely (no TTL/eviction in v1, by Drew's order).
 *
 * The whole check-and-fill runs inside the DO's fetch handler, so
 * concurrent misses for the same query serialize on the DO's
 * single thread: the fan-out executes once, later requests are
 * hits.
 *
 * State machine per entry:
 *   (absent) --embed--> embedded --search--> complete
 * The embedding is stored BEFORE the search runs: if the search
 * fails, the entry stays `embedded`, the error propagates, and
 * the next request retries the search without re-embedding.
 * Nothing is ever deleted; a `complete` entry is served verbatim
 * forever.
 */
import { embedQuery, searchService, type ServiceEnv } from "./service";
import { EMBED_DIMS, EMBED_MODEL } from "./space";

interface EntryRow {
  [key: string]: SqlStorageValue;
  query: string;
  k: number;
  embedding_json: string | null;
  result_json: string | null;
  created_at: string;
  searched_at: string | null;
  embed_ms: number | null;
  search_ms: number | null;
  hit_count: number;
}

export interface QueryOutcome {
  outcome: "hit" | "miss";
  result: Record<string, unknown>;
  meta: {
    created_at: string;
    searched_at: string;
    hit_count: number;
    embed_ms: number | null;
    search_ms: number | null;
    dims: number;
    model: string;
  };
}

export class QueryCache {
  private state: DurableObjectState;
  private env: ServiceEnv;

  constructor(state: DurableObjectState, env: ServiceEnv) {
    this.state = state;
    this.env = env;
    state.storage.sql.exec(
      `CREATE TABLE IF NOT EXISTS entry (
         id INTEGER PRIMARY KEY CHECK (id = 1),
         query TEXT NOT NULL,
         k INTEGER NOT NULL,
         embedding_json TEXT,
         result_json TEXT,
         created_at TEXT NOT NULL,
         searched_at TEXT,
         embed_ms INTEGER,
         search_ms INTEGER,
         hit_count INTEGER NOT NULL DEFAULT 0
       )`,
    );
  }

  private row(): EntryRow | undefined {
    const rows = [...this.state.storage.sql.exec<EntryRow>("SELECT * FROM entry WHERE id = 1")];
    return rows[0];
  }

  async fetch(req: Request): Promise<Response> {
    const url = new URL(req.url);
    try {
      if (req.method === "POST" && url.pathname === "/query") {
        const body = (await req.json()) as { query: string; k: number };
        return Response.json(await this.query(body.query, body.k));
      }
      if (req.method === "GET" && url.pathname === "/inspect") {
        return Response.json(this.inspect());
      }
      return Response.json({ error: { code: "not_found", message: "no route" } }, { status: 404 });
    } catch (e) {
      const status = (e as { status?: number }).status ?? 500;
      return Response.json({ error: { code: "query_failed", message: String((e as Error).message ?? e) } }, { status });
    }
  }

  private async query(query: string, k: number): Promise<QueryOutcome> {
    let row = this.row();
    if (row?.result_json) {
      const stored = row.result_json;
      this.state.storage.sql.exec("UPDATE entry SET hit_count = hit_count + 1 WHERE id = 1");
      row = this.row()!;
      return {
        outcome: "hit",
        result: JSON.parse(stored),
        meta: this.meta(row),
      };
    }

    if (!row) {
      const now = new Date().toISOString();
      this.state.storage.sql.exec("INSERT INTO entry (id, query, k, created_at) VALUES (1, ?, ?, ?)", query, k, now);
      row = this.row()!;
    }

    // Embed first, and persist it before any searching: the DO
    // is born holding the query's embedding (Drew's spec), even
    // if the fan-out below then fails.
    if (!row.embedding_json) {
      const { embed, ms } = await embedQuery(this.env, query);
      this.state.storage.sql.exec(
        "UPDATE entry SET embedding_json = ?, embed_ms = ? WHERE id = 1",
        JSON.stringify(embed.vector),
        ms,
      );
      row = this.row()!;
    }

    const { result, ms } = await searchService(this.env, query, k);
    const searchedAt = new Date().toISOString();
    this.state.storage.sql.exec(
      "UPDATE entry SET result_json = ?, searched_at = ?, search_ms = ? WHERE id = 1",
      JSON.stringify(result),
      searchedAt,
      ms,
    );
    row = this.row()!;
    return { outcome: "miss", result, meta: this.meta(row) };
  }

  private meta(row: EntryRow): QueryOutcome["meta"] {
    return {
      created_at: row.created_at,
      searched_at: row.searched_at ?? row.created_at,
      hit_count: row.hit_count,
      embed_ms: row.embed_ms,
      search_ms: row.search_ms,
      dims: EMBED_DIMS,
      model: EMBED_MODEL,
    };
  }

  /** Readback surface for proofs/audits: metadata only, no result body. */
  private inspect(): Record<string, unknown> {
    const row = this.row();
    if (!row) return { exists: false };
    let embeddingOk = false;
    let embeddingDims = 0;
    if (row.embedding_json) {
      try {
        const v = JSON.parse(row.embedding_json) as number[];
        embeddingDims = v.length;
        embeddingOk = v.length === EMBED_DIMS && v.every((x) => Number.isFinite(x));
      } catch {
        embeddingOk = false;
      }
    }
    return {
      exists: true,
      query: row.query,
      k: row.k,
      has_embedding: row.embedding_json !== null,
      embedding_dims: embeddingDims,
      embedding_valid: embeddingOk,
      has_result: row.result_json !== null,
      result_bytes: row.result_json ? new TextEncoder().encode(row.result_json).length : 0,
      created_at: row.created_at,
      searched_at: row.searched_at,
      embed_ms: row.embed_ms,
      search_ms: row.search_ms,
      hit_count: row.hit_count,
    };
  }
}
