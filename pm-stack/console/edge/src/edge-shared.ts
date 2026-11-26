// What the edge's parts share, and the copies of the host's sections:
// the constants and shapes of edge.ts, its small pure helpers about
// what is held, and the mirrors of the host's sections.
// Extracted from edge.ts for the production-line guard (#14547):
// moves only, no behaviour change.
import { type Held, type HeldFor, subjectOf, type SyncSection } from "../../src/edge-proto.ts";
import type { Store } from "../../src/store.ts";

/** A rank set twice ends at the last one: an older held rank never
 * runs when a newer rank for the same issue is done. Anything else
 * held runs first, in the order it was done. */
export const LAST_WINS = new Set(["issue.rank"]);

/** The edge's own storage: small values by key. */
export interface KV {
  get(key: string): Promise<unknown>;
  put(key: string, value: unknown): Promise<void>;
  delete(key: string): Promise<void>;
}

/** How long each section may go unreported before the app says the
 * state is old: the host's own interval for it, and room for a sync. */
export const SECTIONS: Record<string, number> = {
  board: 30_000,
  sessions: 45_000,
  census: 120_000,
  asks: 45_000,
  questionCount: 45_000,
  questions: 45_000,
  prs: 90_000,
  priorities: 180_000,
  done: 120_000,
  agents: 75_000,
  pipeline: 75_000,
  services: 45_000,
  more: 2 * 60 * 60_000,
};
/** Sections the notifier and a phone's widget are built from. */
export const WATCHED = ["board", "asks", "census", "sessions", "questionCount", "services"];
/** One stored value holds at most this many characters of a section. */
export const CHUNK = 600_000;
export const SYNC_LIMIT = 8 * 1024 * 1024;
/** A report's signature: 64 bytes, as unpadded base64url. */
export const SIGNATURE = /^[A-Za-z0-9_-]{86}$/;
/** Reports from one address that may fail their checks in a minute
 * before that address is refused unread. The host's own never fail. */
export const SYNC_REFUSALS = 20;

/** With no report for this long, the laptop is not there to ask. */
export const HOST_QUIET_MS = 60_000;

export const ACT_LIMIT = 256 * 1024;
export const HELD_MAX = 100;
/** After a pass over what is held that could do none of it, how long
 * before another is tried. */
export const DRAIN_AGAIN_MS = 60_000;
/** Said of something begun by an object that was put away before it
 * ended. */
export const RESTARTED = "Cloudflare restarted while doing this; check whether it happened before trying again.";
/** What the owner is told of something held, by who it waits for. */
export const HELD_SAID: Record<HeldFor, string> = {
  either:
    "Held: it could not be done just now. Cloudflare does it as soon as it can again, or the laptop if it is back first.",
  machine: "Held: the laptop cannot be reached. It will be sent when it is back.",
  asked: "Held: the laptop did not answer. It may have got this, and says what became of it when it is back.",
};
export const ACTS_KEPT = 30;
export const AUDIT_KEPT = 300;
/** "Someone is looking" lasts this long after the last read of a document. */
export const WATCHING_MS = 90_000;
/** The host is reported silent after this long without a sync. */
export const SILENT_MS = 10 * 60_000;
/** With nobody looking, what the edge reads itself and tells the owner
 * about is read again this often. */
export const OWN_CLOCK_MS = 60_000;
/** While somebody looks at the Trends screen, the object's own clock
 * reads its history on: this many pages each minute until it has been
 * read to its end, then what changed, this often. */
export const TRENDS_EVERY_MS = 30 * 60_000;
export const TRENDS_PAGES_A_WAKE = 6;
/** It does that for this long after the screen was last asked for. */
export const TRENDS_WATCHED_MS = 24 * 60 * 60_000;
/** While the app is connected, how often this side wakes to read again
 * what has gone stale (each list by its own age), so that what changed
 * is said to the app without the app asking. */
export const LIVE_MS = 20_000;
/** How long that read is waited for before the owner is told what there is. */
export const CLOCK_WAIT_MS = 20_000;
/** A wake that did not get its work done is tried again this soon. */
export const ALARM_RETRY_MS = 60_000;
/** A copy read here that has not changed has its time written down
 * again no more often than this. */
export const OWN_TIME_EVERY_MS = 5 * 60_000;
/** An action or a section this side's token was refused for is left
 * to a machine for this long before this side tries it again. */
export const REFUSED_FOR_MS = 60 * 60_000;
/** The same, when it was turned away for calling too often: that
 * passes by itself, and soon. */
export const LIMITED_FOR_MS = 5 * 60_000;
/** What a section says while this side's token is refused for it and
 * no machine has sent its copy yet. */
export const REFUSED_READ = "Cloudflare was turned away when it read this, and no machine has reported it yet";
/** How many of this side's own calls are remembered, to tell what an
 * action that failed had got done. */
export const OUTWARD_KEPT = 200;
/** How often the self-check may run again. */
export const SELF_CHECK_EVERY_MS = 60 * 60_000;
/** A check in which something could not be read is made again this
 * soon: what stopped it (one request too many, a moment's outage) has
 * usually passed, and a failure should not stand for an hour. */
export const SELF_CHECK_AGAIN_MS = 5 * 60_000;
export const KEY_FETCH_EVERY_MS = 30_000;

export const ACT_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
/** How many actions done here under a name are remembered, for how
 * long, and how much of each answer is kept with it. A device offline
 * for longer than this that sends again is answered as if for the
 * first time; the names are its own, so nobody else can collide. */
export const ACTED_KEPT = 300;
export const ACTED_FOR_MS = 7 * 24 * 60 * 60_000;
export const ACTED_BODY_CHARS = 4000;

/** An action done here under the name the device sent it with. */
export interface Acted {
  at: number;
  kind: string;
  /** "given": handed to a machine under this name. Nothing was done
   * here, and only the machine knows what became of it. */
  state: "running" | "done" | "given";
  status?: number;
  body?: Record<string, unknown>;
  /** For "given": how many times it has set off toward a machine. Word
   * that one asking never arrived says the action reached no machine
   * only when that asking was the only one. (A record from before this
   * was counted has none, and is taken to have set off already.) */
  relays?: number;
}
export const ICONS = /^\/icons\/(icon-192|icon-512|icon-maskable-512|apple-touch-icon)\.png$/;

export interface Mirror {
  at: number;
  hash: string;
  data: unknown;
  /** Read by this side itself, not reported by a machine. */
  own: boolean;
  /** How many stored pieces hold it. */
  chunks: number;
  /** The time the stored record of it says. */
  written: number;
}

/** One stored piece of a section, marked with whose it is. Pieces are
 * written first and the index (`m:<name>`) last: a write stopped
 * between pieces leaves pieces the index never names, and an index
 * naming pieces another section's write left behind never matches,
 * so neither reads as the section. */
export interface MirrorPiece {
  n: string;
  h: string;
  t: string;
}

/** What the stateless part of the Worker must do with a request this
 * part has checked: serve a file of the app, or pass the request to the
 * host. Anything else is an ordinary response. */
export function next(what: "asset" | "relay" | "key", detail: Record<string, unknown>): Response {
  return new Response(JSON.stringify(detail), {
    status: 200,
    headers: { "content-type": "application/json", "x-pm-next": what },
  });
}

export const kvStore = (kv: KV, key: string): Store => ({
  read: async () => ((await kv.get(key)) as string | undefined) ?? null,
  write: (text) => kv.put(key, text),
});

export interface EdgeOptions {
  kv: KV;
  now?: () => number;
  /** Reaches the sign-in issuer's keys and the push services. */
  fetchImpl?: typeof fetch;
  /** What the edge does itself: the switch, the two tokens, and the
   * fetch its own reads and writes use. */
  direct?: {
    enabled: boolean;
    githubToken?: string;
    queueToken?: string;
    /** The PM stack's own bridge credential and the bridge Worker's
     * service binding (#14373): with both, `message.send` and the
     * agents/messages reads are this side's, done through the bridge
     * as service:pm-stack. Either missing, they stay a machine's. */
    bridgeToken?: string;
    bridgeFetch?: unknown;
    fetchImpl?: typeof fetch;
    /** Where GitHub's API and the queue store are, when a local run
     * puts stand-ins in their place. */
    githubApi?: string;
    queueDb?: string;
    /** Which kinds this object does itself: every one it can
     * (DIRECT_ACTIONS) unless said. A test says, to stand for a kind
     * only a machine can do that is about the same thing as one done
     * here: since "done" became this side's no kind of today's is, and
     * the rule for one (what comes after it waits behind it) has to
     * hold for the next that is. */
    kinds?: readonly string[];
  };
  /** The hosted M4 ledger surfaces (#14968, CF-5b): the fleet-platform
   * ledger store, present only when the Worker's LEDGER_PAGES flag
   * reads `on`. With it, this side serves /alerts, /notifications and
   * /comments (and their /api twins) itself from the ledger; without
   * it, those pages stay the host's. The store is the D1 database
   * named fleet-platform since the owner ruling of 2026-11-26 on
   * #14866, or the Turso ledger (address + credential) when the
   * Worker's LEDGER_STORE var does not read `d1` — the rollback. */
  ledgerPages?:
    | { store: "turso"; url: string; token?: string }
    | { store: "d1"; db?: import("./ledger-pages.ts").LedgerD1Database };
  /** Hosted /dns (#14968 slice 2): the #902 register snapshot the
   * collector publishes into the same fleet-platform D1 database,
   * present only when the Worker's DNS_PAGES flag reads `on`. With
   * it, this side serves /dns and /api/dns.json itself from the
   * snapshot row; without it, those pages stay the host's. */
  dnsPages?: { db?: import("./ledger-pages.ts").LedgerD1Database };
  /** C6 quota page (#15018): opt-in. Envelope + existing question sets
   * are supplied by the host; dispatchReset is the existing pool-steward
   * machinery. Absent/off: /console/api/quota answers 503. */
  quota?: {
    enabled: boolean;
    envelope?: unknown;
    questions?: import("../../src/quota-page.ts").ExistingQuestionSet[];
    dispatchReset?: (
      p: import("../../src/quota-page.ts").ResetProposal,
    ) => Promise<{ queueTaskId: string; result: string }>;
  };
  /** The live connections to the app, which the platform holds (they
   * stay open while this object is put away): take a request to be
   * one and give the answer for it, say something to all of them, and
   * how many there are. Absent where there are none to hold (a test
   * that has no need of them): nobody is connected then. */
  sockets?: {
    accept: (req: Request) => Response;
    send: (text: string) => number;
    count: () => number;
  };
  /** Ask to be woken at a time: to read again what the owner is told
   * about, and to notice a machine gone quiet. */
  setAlarm?: (at: number) => Promise<void>;
  /** When this object has asked to be woken, as the platform holds it:
   * null when it has not. For health. */
  getAlarm?: () => Promise<number | null>;
  /** Work that may finish after the answer has gone. */
  later?: (work: Promise<unknown>) => void;
  /** How often the object's own clock wakes to read again what the
   * owner is told about, in milliseconds. A local run shortens it so
   * a smoke run can watch an alarm fire; where this is deployed it
   * is left unset and the minute stands. */
  ownClockMs?: number;
}

/** An action this side tried and its token was refused for, with
 * nothing done: to be done by a machine instead. */
export class HandOver extends Error {}

export function withMachine(h: Held, handed: Set<string>): boolean {
  return handed.has(h.id) || h.unsent !== true;
}

/** What a held action is about, or itself when it stands alone. */
export function heldAbout(h: Held): string {
  return subjectOf(h.kind, h.args) ?? `:${h.id}`;
}

/** A held action put where its time says it goes: oldest first. */
export function insertHeld(held: Held[], item: Held): void {
  const at = held.findIndex((h) => h.at > item.at);
  if (at < 0) held.push(item);
  else held.splice(at, 0, item);
}

/** The copies of the host's sections this side keeps, in memory and in
 * its storage: each read back whole or not at all, its pieces marked
 * with the section and the content they hold. */
export class Mirrors {
  private kept = new Map<string, Mirror | null>();

  constructor(
    private kv: KV,
    private ready: () => Promise<void>,
    private confirmed: () => Record<string, { hash: string; at: number; error: string | null }>,
  ) {}

  async mirror(name: string): Promise<Mirror | null> {
    const cached = this.kept.get(name);
    if (cached !== undefined) return cached;
    await this.ready();
    const meta = (await this.kv.get(`m:${name}`)) as
      | { at: number; hash: string; chunks: number; own?: boolean }
      | undefined;
    let found: Mirror | null = null;
    if (meta) {
      const parts: string[] = [];
      let marked = true;
      for (let i = 0; i < meta.chunks; i++) {
        const kept = (await this.kv.get(`m:${name}:${i}`)) as Partial<MirrorPiece> | string | undefined;
        if (typeof kept === "string") {
          // Written before pieces were marked: still this section's.
          parts.push(kept);
          continue;
        }
        // A piece of another write, or none: what the index names was
        // never finished, or is being replaced. Nothing is kept.
        if (!kept || kept.n !== name || kept.h !== meta.hash || typeof kept.t !== "string") {
          marked = false;
          break;
        }
        parts.push(kept.t);
      }
      try {
        if (!marked) found = null;
        else {
          const own = meta.own === true;
          // As current as the host last said it was, which may be later
          // than when the copy itself was sent. A copy read here is as
          // current as that read, whatever a machine says of its own.
          const told = !own && this.confirmed()[name]?.hash === meta.hash ? this.confirmed()[name].at : 0;
          found = {
            at: Math.max(meta.at, told),
            hash: meta.hash,
            data: JSON.parse(parts.join("")),
            own,
            chunks: meta.chunks,
            written: meta.at,
          };
        }
      } catch {
        found = null; // a copy that cannot be read is no copy: the host is asked for it again
      }
    }
    this.kept.set(name, found);
    return found;
  }

  /** Keep a copy of a section. `own` says this side read it itself. */
  async putMirror(section: SyncSection, own = false): Promise<void> {
    const text = JSON.stringify(section.data);
    const chunks = Math.max(1, Math.ceil(text.length / CHUNK));
    const before = (await this.kv.get(`m:${section.name}`)) as { chunks: number } | undefined;
    // Pieces first, each marked with the section and content they hold,
    // then the index last: see `mirror`.
    for (let i = 0; i < chunks; i++)
      await this.kv.put(`m:${section.name}:${i}`, {
        n: section.name,
        h: section.hash,
        t: text.slice(i * CHUNK, (i + 1) * CHUNK),
      } satisfies MirrorPiece);
    for (let i = chunks; i < (before?.chunks ?? 0); i++) await this.kv.delete(`m:${section.name}:${i}`);
    await this.kv.put(`m:${section.name}`, {
      at: section.at,
      hash: section.hash,
      chunks,
      ...(own ? { own: true } : {}),
    });
    this.kept.set(section.name, {
      at: section.at,
      hash: section.hash,
      data: section.data,
      own,
      chunks,
      written: section.at,
    });
  }
}

/** A list of records that each carry their name (`id`), kept one
 * record per key with a small index of the names in order, instead of
 * one whole-array value rewritten at every change (#14960): a change
 * to one record writes that record and the index, never the whole
 * list. Records are written first, the index after them, and
 * forgotten records deleted last, so a write stopped midway leaves
 * records the index never names — never an index naming a record
 * that is not there (the same rule as the mirrors' pieces). What an
 * older copy kept as one array under the plain name is taken in on
 * first read and kept the new way from then on. */
export class StoredRecords<T extends { id: string }> {
  /** The records as storage holds them, by name, in the JSON they
   * were written with: what a save has to change is read off it. A
   * record's entry is marked stored only once its write has
   * finished, so a save stopped by a failing store is read aright by
   * the next one. */
  private stored = new Map<string, string>();
  /** The index as storage holds it, in the same JSON. */
  private index: string | null = null;
  private loaded = false;

  constructor(
    private kv: KV,
    /** The collection's plain name: its old whole-array key, and the
     * stem of its index key (`<name>:index`). */
    private name: string,
    /** Where one record is kept: `held:<id>`, and the like. */
    private entryKey: (id: string) => string,
  ) {}

  private indexKey(): string {
    return `${this.name}:index`;
  }

  /** The list as storage holds it, oldest first as the index says. */
  async load(): Promise<T[]> {
    this.loaded = true;
    const index = await this.kv.get(this.indexKey());
    if (Array.isArray(index)) {
      const items: T[] = [];
      const ids: string[] = [];
      this.stored.clear();
      for (const id of index) {
        if (typeof id !== "string") continue;
        const value = (await this.kv.get(this.entryKey(id))) as T | undefined;
        // A name whose record is not there is not in the list: the
        // record was forgotten and the forgetting of its name is
        // finished by the next save.
        if (value === undefined || value === null) continue;
        items.push(value);
        ids.push(id);
        this.stored.set(id, JSON.stringify(value));
      }
      this.index = JSON.stringify(ids);
      return items;
    }
    const legacy = await this.kv.get(this.name);
    if (Array.isArray(legacy)) {
      // Kept as one array by an older copy: kept the new way now.
      const items: T[] = [];
      const seen = new Set<string>();
      for (const item of legacy as Partial<T>[]) {
        if (!item || typeof item !== "object" || typeof item.id !== "string" || seen.has(item.id)) continue;
        seen.add(item.id);
        items.push(item as T);
      }
      for (const item of items) {
        await this.kv.put(this.entryKey(item.id), item);
        this.stored.set(item.id, JSON.stringify(item));
      }
      await this.kv.put(
        this.indexKey(),
        items.map((item) => item.id),
      );
      this.index = JSON.stringify(items.map((item) => item.id));
      await this.kv.delete(this.name);
      return items;
    }
    this.stored.clear();
    this.index = JSON.stringify([]);
    return [];
  }

  /** Keep the list as it now stands: only the records that changed
   * are written, and only they and the index. */
  async save(items: T[]): Promise<void> {
    if (!this.loaded) await this.load();
    const now = new Map(items.map((item) => [item.id, JSON.stringify(item)]));
    for (const item of items) {
      const written = now.get(item.id) as string;
      if (this.stored.get(item.id) !== written) {
        await this.kv.put(this.entryKey(item.id), item);
        this.stored.set(item.id, written);
      }
    }
    const index = JSON.stringify(items.map((item) => item.id));
    if (this.index !== index) {
      await this.kv.put(
        this.indexKey(),
        items.map((item) => item.id),
      );
      this.index = index;
    }
    for (const id of [...this.stored.keys()]) {
      if (!now.has(id)) {
        await this.kv.delete(this.entryKey(id));
        this.stored.delete(id);
      }
    }
  }
}
