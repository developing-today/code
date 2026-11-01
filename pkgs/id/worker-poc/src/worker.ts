// Worker isolate host for plaza_* worlds (#2271 PoC).
//
// One Durable Object per world. The DO's SQLite storage is the journal and
// the source of truth: every applied event is journaled with the resulting
// snapshot, so a forced cold start (fresh isolate / evicted DO instance)
// restores from the latest journaled snapshot via plaza_restore and state
// evolution continues identically. The guest module itself is imported as a
// compiled Wasm binding (COUNTER_WASM) — in the PoC that is the prebuilt,
// unmodified pkgs/id/examples/roc-counter/counter.wasm.
//
// Routes (on the DO, forwarded by the worker fetch handler):
//   POST /step  {participant: number, event: string}
//               -> {step, view, records, snapshotHash}
//   GET  /state -> {step, view, records, snapshotHash}
//   POST /cold-start -> drops the in-memory guest; the next request lazily
//               restores from the journal (this is the forced cold start).
import { PlazaGuest } from "./guest.ts";
import { loadHostedModule, type HostedManifest } from "./manifest.ts";
// @ts-expect-error CompiledWasm import: wrangler supplies a WebAssembly.Module.
import COUNTER_WASM from "../../examples/roc-counter/counter.wasm";

interface SqlStorage {
  exec(query: string, ...params: unknown[]): { toArray(): Record<string, unknown>[] };
}
interface DOState {
  storage: { sql: SqlStorage };
  blockConcurrencyWhile<T>(fn: () => Promise<T>): Promise<T>;
}
interface Env {
  WORLD: { idFromName(name: string): unknown; get(id: unknown): { fetch(req: Request): Promise<Response> } };
}

const MANIFEST: HostedManifest = { name: "roc-counter", portability: "portable" };
const SEED = 0;
const enc = new TextEncoder();
const dec = new TextDecoder();

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytes as BufferSource);
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

export class WorldObject {
  private sql: SqlStorage;
  private guest: PlazaGuest | null = null;
  private step = 0;

  constructor(state: DOState) {
    this.sql = state.storage.sql;
    this.sql.exec(
      `CREATE TABLE IF NOT EXISTS journal (
         step INTEGER PRIMARY KEY,
         participant INTEGER NOT NULL,
         event TEXT NOT NULL,
         snapshot BLOB NOT NULL,
         view TEXT NOT NULL,
         records TEXT,
         snapshot_hash TEXT NOT NULL
       )`,
    );
  }

  // Lazy (re)instantiation: fresh init from the seed, or plaza_restore from
  // the latest journaled snapshot when the journal is non-empty. This is the
  // cold-start path — it runs identically after a real eviction.
  private async ensureGuest(): Promise<PlazaGuest> {
    if (this.guest) return this.guest;
    const rows = this.sql
      .exec(`SELECT step, snapshot FROM journal ORDER BY step DESC LIMIT 1`)
      .toArray();
    const guest = await loadHostedModule(MANIFEST, COUNTER_WASM as WebAssembly.Module, SEED);
    if (rows.length > 0) {
      guest.restore(rows[0].snapshot as Uint8Array);
      this.step = rows[0].step as number;
    }
    this.guest = guest;
    return guest;
  }

  async fetch(req: Request): Promise<Response> {
    const url = new URL(req.url);
    if (req.method === "POST" && url.pathname === "/cold-start") {
      this.guest = null; // force the next request through ensureGuest()'s restore path
      return Response.json({ cold: true, journaledStep: this.step });
    }
    const guest = await this.ensureGuest();
    if (req.method === "POST" && url.pathname === "/step") {
      const { participant, event } = (await req.json()) as { participant: number; event: string };
      guest.update(participant, enc.encode(event));
      const view = guest.view();
      const records = guest.records();
      const snapshot = guest.snapshot();
      if (!snapshot) throw new Error("guest exports no snapshot; journaling requires the snapshot/restore pair");
      const snapshotHash = await sha256Hex(snapshot);
      this.step += 1;
      this.sql.exec(
        `INSERT INTO journal (step, participant, event, snapshot, view, records, snapshot_hash) VALUES (?, ?, ?, ?, ?, ?, ?)`,
        this.step, participant, event, snapshot, dec.decode(view),
        records ? dec.decode(records) : null, snapshotHash,
      );
      return Response.json({
        step: this.step,
        view: dec.decode(view),
        records: records ? dec.decode(records) : null,
        snapshotHash,
      });
    }
    if (req.method === "GET" && url.pathname === "/state") {
      const view = guest.view();
      const records = guest.records();
      const snapshot = guest.snapshot();
      return Response.json({
        step: this.step,
        view: dec.decode(view),
        records: records ? dec.decode(records) : null,
        snapshotHash: snapshot ? await sha256Hex(snapshot) : null,
      });
    }
    return new Response("not found", { status: 404 });
  }
}

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    const url = new URL(req.url);
    const name = url.searchParams.get("world") ?? "default";
    const stub = env.WORLD.get(env.WORLD.idFromName(name));
    return stub.fetch(req);
  },
};
