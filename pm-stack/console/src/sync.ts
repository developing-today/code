// The host's side of its link to the edge.
//
// The app is served from the edge. This reports the host's state to it
// (only the sections that changed), learns from the answer whether
// anyone is looking, and runs the actions the owner took while the host
// could not be reached. See edge-proto.ts for what travels and why it
// can be trusted.
//
// A held action is run at most once. Its id is written down before it
// starts; if the host dies part-way, the action is reported as
// interrupted instead of being run a second time.
import type { Docs } from "./docs.ts";
import {
  type EdgeConfig,
  type Held,
  HOLDABLE,
  keyId,
  newKeyPair,
  type PublicKey,
  type PhoneLinks,
  publicKey,
  type SyncRequest,
  type SyncResponse,
  sign,
} from "./edge-proto.ts";
import type { Store } from "./store.ts";

/** Sections kept fresh for notifications and a phone's widget when
 * nobody has the app open. */
const WATCHED = ["board", "asks", "census", "sessions", "questionCount", "services"];
const FAST_MS = 5_000;
const IDLE_MS = 30_000;
const WATCHED_EVERY_MS = 60_000;
const SYNC_TIMEOUT_MS = 20_000;
const DONE_KEPT = 300;

type Fate = "started" | { ok: boolean; error?: string };

export interface LinkOptions {
  /** The public address the edge answers on, e.g. https://pm.example. */
  url: string;
  /** The host's key pair. Made on first use; the private half stays here. */
  keys: Store;
  /** What became of each held action, so none is run twice. */
  done: Store;
  docs: Docs;
  phones: { links(): Promise<PhoneLinks>; mergeLinks(value: unknown): Promise<void> };
  config: () => Promise<EdgeConfig>;
  /** Run one held action; throws if it could not be done. */
  run: (held: Held) => Promise<void>;
  fetchImpl?: typeof fetch;
  now?: () => number;
  log?: (line: string) => void;
}

export class EdgeLink {
  private pair: { publicJwk: PublicKey; privateJwk: JsonWebKey; kid: string } | null = null;
  private fates: Map<string, Fate> | null = null;
  /** What the edge is known to have, by fingerprint. */
  private sent = new Map<string, string>();
  private want = new Set<string>();
  private acks: { id: string; ok: boolean; error?: string }[] = [];
  private lastAt = 0;
  private watchedAt = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private busy = false;
  /** Actions being done right now, by the name the edge gave them. */
  private running = new Set<string>();
  private now: () => number;
  private fetchImpl: typeof fetch;
  private log: (line: string) => void;
  /** The last answer from the edge, or null while it cannot be reached. */
  last: SyncResponse | null = null;

  constructor(private o: LinkOptions) {
    this.now = o.now ?? Date.now;
    this.fetchImpl = o.fetchImpl ?? fetch;
    this.log = o.log ?? ((line) => console.error(`pm-console: edge: ${line}`));
  }

  private async keys() {
    if (this.pair) return this.pair;
    let stored: { publicJwk?: unknown; privateJwk?: JsonWebKey } | null = null;
    try {
      stored = JSON.parse((await this.o.keys.read()) ?? "null");
    } catch {
      stored = null;
    }
    let pub = publicKey(stored?.publicJwk);
    let priv = stored?.privateJwk;
    if (!pub || !priv) {
      const made = await newKeyPair();
      pub = made.publicJwk;
      priv = made.privateJwk;
      await this.o.keys.write(JSON.stringify(made));
    }
    this.pair = { publicJwk: pub, privateJwk: priv, kid: await keyId(pub) };
    return this.pair;
  }

  /** The public half, for the edge to fetch through the tunnel. */
  async publicKey(): Promise<PublicKey> {
    return (await this.keys()).publicJwk;
  }

  private async fate(): Promise<Map<string, Fate>> {
    if (this.fates) return this.fates;
    try {
      this.fates = new Map(Object.entries(JSON.parse((await this.o.done.read()) ?? "{}") as Record<string, Fate>));
    } catch {
      this.fates = new Map();
    }
    return this.fates;
  }

  private async record(id: string, fate: Fate): Promise<void> {
    const fates = await this.fate();
    fates.delete(id);
    fates.set(id, fate);
    while (fates.size > DONE_KEPT) fates.delete(fates.keys().next().value as string);
    await this.o.done.write(JSON.stringify(Object.fromEntries(fates)));
  }

  /** The edge relayed an action under this name and it is about to be
   * done. Written down first, so that if the answer never reaches the
   * edge and the edge offers the action again, it is not done twice. */
  async begin(id: string): Promise<void> {
    this.running.add(id);
    await this.record(id, "started");
  }

  /** How a relayed action came out. */
  async end(id: string, fate: { ok: boolean; error?: string }): Promise<void> {
    try {
      await this.record(id, fate);
    } finally {
      this.running.delete(id);
    }
  }

  /** A held action from the edge: run it unless it has been seen before. */
  private async take(held: Held): Promise<void> {
    if (this.acks.some((a) => a.id === held.id)) return;
    // Still being done (its answer was slow, not lost): say nothing yet.
    if (this.running.has(held.id)) return;
    const seen = (await this.fate()).get(held.id);
    if (seen === "started") {
      // The host stopped while this was running. Whether it happened is
      // not known, and running it again could do it twice.
      const fate = { ok: false, error: "the host restarted while doing this; check whether it happened" };
      await this.record(held.id, fate);
      this.acks.push({ id: held.id, ...fate });
      return;
    }
    if (seen) {
      this.acks.push({ id: held.id, ...seen }); // the edge missed the first answer
      return;
    }
    let fate: { ok: boolean; error?: string };
    if (!HOLDABLE.has(held.kind)) {
      fate = { ok: false, error: "this is not an action that may be done late" };
    } else {
      await this.record(held.id, "started");
      try {
        await this.o.run(held);
        fate = { ok: true };
      } catch (error) {
        fate = { ok: false, error: (error as Error).message.slice(0, 300) };
      }
    }
    await this.record(held.id, fate);
    this.acks.push({ id: held.id, ...fate });
  }

  /** One report to the edge. Returns its answer, or null if it could not
   * be reached or refused the report; nothing is lost either way. */
  async once(): Promise<SyncResponse | null> {
    if (this.busy) return this.last;
    this.busy = true;
    try {
      const pair = await this.keys();
      const snapshot = this.o.docs.snapshot();
      const sections = snapshot.filter((s) => this.sent.get(s.name) !== s.hash || this.want.has(s.name));
      const acks = [...this.acks];
      const at = Math.max(this.now(), this.lastAt + 1);
      const message: SyncRequest & { acts: unknown } = {
        v: 1,
        at,
        config: await this.o.config(),
        sections,
        have: Object.fromEntries(snapshot.map((s) => [s.name, { hash: s.hash, at: s.at, error: s.error }])),
        acks,
        phones: await this.o.phones.links(),
        acts: this.o.docs.recentActs(),
      };
      const body = new TextEncoder().encode(JSON.stringify(message));
      let res: Response;
      try {
        res = await this.fetchImpl(`${this.o.url}/console/edge/sync`, {
          method: "POST",
          headers: {
            "content-type": "application/json",
            "x-pm-key": pair.kid,
            "x-pm-signature": await sign(pair.privateJwk, body),
          },
          body,
          redirect: "manual",
          signal: AbortSignal.timeout(SYNC_TIMEOUT_MS),
        });
      } catch (error) {
        if (this.last) this.log(`cannot reach the edge: ${(error as Error).message}`);
        this.last = null;
        return null;
      }
      this.lastAt = at;
      const answer = (await res.json().catch(() => null)) as (SyncResponse & { error?: string; last?: number }) | null;
      if (!res.ok || answer?.ok !== true) {
        // "replay" means the edge has seen a later time than this clock
        // gives (it ran backwards): step past it.
        if (res.status === 409 && typeof answer?.last === "number") this.lastAt = Math.max(this.lastAt, answer.last);
        if (this.last) this.log(`the edge refused the report: http ${res.status} ${answer?.error ?? ""}`);
        this.last = null;
        return null;
      }
      if (!this.last) this.log("reporting to the edge");
      for (const s of sections) this.sent.set(s.name, s.hash);
      this.acks = this.acks.filter((a) => !acks.includes(a));
      this.want = new Set(answer.want ?? []);
      this.last = answer;
      await this.o.phones.mergeLinks(answer.phones);
      for (const held of answer.commands ?? []) await this.take(held);
      return answer;
    } finally {
      this.busy = false;
    }
  }

  /** One turn of the loop: keep fresh what someone needs, report, and
   * say how long to wait before the next turn. */
  async turn(): Promise<number> {
    const now = this.now();
    if (this.last?.watching) this.o.docs.keepAlive();
    else if (this.last?.notify && now - this.watchedAt >= WATCHED_EVERY_MS) {
      this.watchedAt = now;
      // Read again what is stale, without waiting for it and without
      // counting as someone looking.
      void this.o.docs.current(WATCHED, 0);
    }
    const answer = await this.once();
    // Something was done or is wanted: say so at once.
    if (this.acks.length > 0 || this.want.size > 0) return 500;
    return answer?.watching || this.o.docs.watched() ? FAST_MS : IDLE_MS;
  }

  start(): void {
    if (this.timer) return;
    const loop = async () => {
      let wait = IDLE_MS;
      try {
        wait = await this.turn();
      } catch (error) {
        this.log((error as Error).message);
      }
      this.timer = setTimeout(loop, wait);
    };
    this.timer = setTimeout(loop, 1_000);
  }

  stop(): void {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
  }
}
