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
import { plain } from "./plain.ts";
import {
  APK_LIMIT,
  type ApkReport,
  type EdgeConfig,
  type Held,
  HOLDABLE,
  keyId,
  newKeyPair,
  type PublicKey,
  type PhoneLinks,
  publicKey,
  REPORT_ROOM,
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
/** After a report that carried the Android build did not get through,
 * the build is left out of the reports that follow for this long. */
const APK_AGAIN_MS = 10 * 60_000;
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
  /** The commit this console is running, if it could be found. */
  version?: string;
  /** The signed Android build on this host, if there is one, and a way
   * to read it when the edge asks for it. */
  apk?: () => Promise<{ sha256: string; size: number; at: number; read: () => Promise<Uint8Array> } | null>;
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
  /** The Android build is not put in a report before this time. */
  private apkNotBefore = 0;
  /** The last report left something out that the next should carry at
   * once: the sections, when the build went alone, or everything but
   * the build, when a report carrying it did not get through. */
  private more = false;
  /** Actions being done right now, by the name the edge gave them. */
  private running = new Set<string>();
  private now: () => number;
  private fetchImpl: typeof fetch;
  private log: (line: string) => void;
  /** The last answer from the edge, or null while it cannot be reached. */
  last: SyncResponse | null = null;

  constructor(private o: LinkOptions) {
    this.now = o.now ?? Date.now;
    this.fetchImpl = plain(o.fetchImpl ?? fetch);
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

  /** The build could not be sent. It is left out of the reports that
   * follow for a while: nothing about the build may stand in the way of
   * an ordinary report, which is how the edge knows the host is there. */
  private holdBuild(why: string): void {
    this.apkNotBefore = this.now() + APK_AGAIN_MS;
    this.log(`${why}; reporting without it for ten minutes`);
  }

  /** One report to the edge. Returns its answer, or null if it could not
   * be reached or refused the report; nothing is lost either way. */
  async once(): Promise<SyncResponse | null> {
    if (this.busy) return this.last;
    this.busy = true;
    try {
      const pair = await this.keys();
      this.more = false;
      // What the edge said it reads itself is not reported to it: not
      // the copy, and not this host's word on how current it is.
      const theirs = new Set(this.last?.owns ?? []);
      const snapshot = this.o.docs.snapshot().filter((s) => !theirs.has(s.name));
      let sections = snapshot.filter((s) => this.sent.get(s.name) !== s.hash || this.want.has(s.name));
      const acks = [...this.acks];
      const at = Math.max(this.now(), this.lastAt + 1);
      // The Android build: always what it is, and the build itself only
      // when the edge has said it lacks this one (and not for a while
      // after it could not be sent).
      const build = (await this.o.apk?.().catch(() => null)) ?? null;
      const apk: ApkReport | null =
        build && build.size <= APK_LIMIT ? { sha256: build.sha256, size: build.size, at: build.at } : null;
      if (apk && build && this.want.has("apk") && this.now() >= this.apkNotBefore) {
        try {
          apk.data = Buffer.from(await build.read()).toString("base64");
        } catch (error) {
          this.holdBuild(`the Android build could not be read (${(error as Error).message})`);
        }
      }
      const config = await this.o.config();
      const have = Object.fromEntries(snapshot.map((s) => [s.name, { hash: s.hash, at: s.at, error: s.error }]));
      const phones = await this.o.phones.links();
      const acts = this.o.docs.recentActs();
      const encoded = (carried: typeof sections) => {
        const message: SyncRequest & { acts: unknown } = {
          v: 1,
          at,
          ...(this.o.version ? { version: this.o.version } : {}),
          ...(apk ? { apk } : {}),
          config,
          sections: carried,
          have,
          acks,
          phones,
          acts,
        };
        return new TextEncoder().encode(JSON.stringify(message));
      };
      let body = encoded(sections);
      // The build and large sections together could pass what the edge
      // takes in one report. Measured on the report as it would be sent,
      // in bytes: when it is past the room there is, the build goes
      // alone; the sections are not marked sent and follow at once.
      if (apk?.data && sections.length > 0 && body.byteLength > REPORT_ROOM) {
        sections = [];
        body = encoded(sections);
        this.more = true;
      }
      // A report that carries the build is large and slow to send. If it
      // does not get through, the same report again would not either, and
      // while that went on the edge would hear nothing at all from this
      // host. So the reports after it go without the build, the first of
      // them at once.
      const lost = () => {
        if (!apk?.data) return;
        this.holdBuild("a report carrying the Android build did not get through");
        this.more = true;
      };
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
        lost();
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
        lost();
        this.last = null;
        return null;
      }
      if (!this.last) this.log("reporting to the edge");
      for (const s of sections) this.sent.set(s.name, s.hash);
      // What the edge reads itself it does not hold from here: should it
      // stop reading it, this host's copy goes in the very next report.
      for (const name of answer.owns ?? []) this.sent.delete(name);
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
    // Something was done, is wanted, or was left out of that report:
    // say so at once.
    // (The Android build is not such a thing: it waits for the next
    // ordinary report, and half a megabyte is never sent in a hurry.)
    if (this.acks.length > 0 || this.more || [...this.want].some((name) => name !== "apk")) return 500;
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
