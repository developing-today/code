// The state the interface draws, kept ready.
//
// A page never waits on a live fan-out to GitHub, the queue store or the
// dashboard: each source is a section that is refreshed in the
// background and served from memory, with the time it was last read and
// the error if the last read failed. Three documents are built from the
// sections, sized for a phone on a poor link:
//
//   core       what is in progress, what is next, what needs the owner,
//              what he has asked for (small, polled)
//   questions  every open question waiting on him (large, read on demand)
//   ops        pull requests, ranked issues, agents, units, other pages
//
// The same documents can be stored and served somewhere else: nothing in
// them depends on this process.
import { createHash } from "node:crypto";

export type DocName = "core" | "questions" | "ops";
export const DOC_NAMES: DocName[] = ["core", "questions", "ops"];

interface Section {
  /** Refresh when older than this while someone is looking. */
  ttlMs: number;
  load: () => Promise<unknown>;
  at: number;
  /** When it was last read successfully. */
  goodAt: number;
  ok: boolean;
  error: string | null;
  data: unknown;
  loading: Promise<void> | null;
}

/** Data that carries the time it was really read: a copy of a section
 * that somewhere else read (the host reporting to the edge). */
export class Stamped {
  constructor(
    public data: unknown,
    public at: number,
    /** Why the place that read it could not read it again, if it could
     * not: the data is then its last good copy. */
    public error: string | null = null,
  ) {}
}

export interface Act {
  id: string;
  kind: string;
  at: string;
  by: string;
  status: "running" | "done" | "failed";
  note: string;
}

/** Which sections each document is built from. */
const DOCS: Record<DocName, string[]> = {
  core: ["board", "sessions", "census", "asks", "questionCount"],
  questions: ["questions"],
  ops: ["prs", "priorities", "agents", "services", "more"],
};
/** Stop refreshing when nobody has asked for this long. */
const WATCH_MS = 10 * 60_000;
const FIRST_LOAD_MS = 25_000;
const ACTS_KEPT = 30;

export class Docs {
  private sections = new Map<string, Section>();
  private asked = 0;
  private acts: Act[] = [];
  /** Small values that travel in the core document beside its sections. */
  private extras: Record<string, unknown> = {};
  private timer: ReturnType<typeof setInterval> | null = null;

  constructor(
    loaders: Record<string, { ttlMs: number; load: () => Promise<unknown> }>,
    private now: () => number = Date.now,
  ) {
    for (const [name, l] of Object.entries(loaders)) {
      this.sections.set(name, { ...l, at: 0, goodAt: 0, ok: false, error: null, data: null, loading: null });
    }
    for (const names of Object.values(DOCS)) {
      for (const name of names) {
        if (!this.sections.has(name)) throw new Error(`no loader for section ${name}`);
      }
    }
  }

  /** Read one section again. Concurrent callers share the read. A failed
   * read keeps the last good data and records why. */
  refresh(name: string): Promise<void> {
    const s = this.sections.get(name);
    if (!s) return Promise.resolve();
    if (s.loading) return s.loading;
    s.loading = (async () => {
      try {
        const loaded = await s.load();
        const now = this.now();
        // A copy read elsewhere is as old as that read, never newer than now.
        s.data = loaded instanceof Stamped ? loaded.data : loaded;
        s.goodAt = loaded instanceof Stamped ? Math.min(now, loaded.at) : now;
        s.error = loaded instanceof Stamped ? loaded.error : null;
        s.ok = s.error === null;
      } catch (error) {
        s.ok = false;
        s.error = (error as Error).message.slice(0, 300);
      } finally {
        s.at = this.now();
        s.loading = null;
      }
    })();
    return s.loading;
  }

  /** After an action: read the sections it changed, so the next document shows it. */
  async touch(names: string[]): Promise<void> {
    await Promise.all(names.map((n) => this.refresh(n)));
  }

  /** Record a section read by another trusted source, such as the edge
   * reading GitHub or the queue store while the laptop is away. */
  record(name: string, data: unknown, at = this.now()): void {
    const s = this.sections.get(name);
    if (!s) return;
    s.data = data;
    s.at = at;
    s.goodAt = at;
    s.ok = true;
    s.error = null;
  }

  /** Keep the last good value, but say that a fresh read failed. */
  recordFailure(name: string, error: unknown): void {
    const s = this.sections.get(name);
    if (!s) return;
    s.at = this.now();
    s.ok = false;
    s.error = (error as Error).message.slice(0, 300);
  }

  private stale(s: Section): boolean {
    return this.now() - s.at > s.ttlMs;
  }

  /** One pass of the background loop: refresh what is stale, if anyone is looking. */
  tick(): void {
    if (this.now() - this.asked > WATCH_MS) return;
    for (const [name, s] of this.sections) {
      if (this.stale(s)) void this.refresh(name);
    }
  }

  start(everyMs = 5_000): void {
    this.timer ??= setInterval(() => this.tick(), everyMs);
  }

  stop(): void {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
  }

  /** The last good copy of one section, or null if it has never been read. */
  good(name: string): unknown {
    const s = this.sections.get(name);
    return s && s.goodAt > 0 ? s.data : null;
  }

  /** When that data was read from its source, or 0 if it never was. A
   * read that failed since does not move it: the data is still the last
   * good copy, and this is how old that copy is. */
  goodAt(name: string): number {
    return this.sections.get(name)?.goodAt ?? 0;
  }

  /** Sections brought up to date for a caller that asks rarely (a
   * phone's widget): stale ones are read again and waited for, up to
   * `waitMs`. It does not count as someone looking, so it does not keep
   * the background loop running. `missing` names the sections that have
   * never been read; `at` is the oldest good read among the rest. */
  async current(
    names: string[],
    waitMs: number,
  ): Promise<{ data: Record<string, unknown>; at: number; missing: string[] }> {
    const stale = names.filter((n) => {
      const s = this.sections.get(n);
      return s !== undefined && this.stale(s);
    });
    if (stale.length > 0) {
      let timer: ReturnType<typeof setTimeout> | undefined;
      await Promise.race([
        Promise.all(stale.map((n) => this.refresh(n))),
        new Promise((resolve) => {
          timer = setTimeout(resolve, waitMs);
        }),
      ]);
      clearTimeout(timer);
    }
    const read = names.map((n) => this.sections.get(n)?.goodAt ?? 0);
    const good = read.filter((t) => t > 0);
    return {
      data: Object.fromEntries(names.map((n) => [n, this.good(n)])),
      at: good.length > 0 ? Math.min(...good) : 0,
      missing: names.filter((_, i) => read[i] === 0),
    };
  }

  logAct(act: Act): void {
    this.acts = [act, ...this.acts.filter((a) => a.id !== act.id)].slice(0, ACTS_KEPT);
  }

  /** What the owner did lately, newest first; and a way to put it back
   * after a restart of whatever holds this. */
  recentActs(): Act[] {
    return this.acts;
  }

  restoreActs(acts: Act[]): void {
    this.acts = acts.slice(0, ACTS_KEPT);
  }

  /** Set, or with undefined remove, a value carried in the core document. */
  extra(name: string, value: unknown): void {
    if (value === undefined) delete this.extras[name];
    else this.extras[name] = value;
  }

  /** Someone is looking, though not through this process: keep reading. */
  keepAlive(): void {
    this.asked = this.now();
  }

  /** Whether anyone has asked for a document lately. */
  watched(): boolean {
    return this.now() - this.asked <= WATCH_MS;
  }

  /** Every section that has been read, with when and a fingerprint of
   * its content: what is reported to the edge, and how it tells that a
   * section has not changed. */
  snapshot(): { name: string; at: number; hash: string; data: unknown; error: string | null }[] {
    const out: { name: string; at: number; hash: string; data: unknown; error: string | null }[] = [];
    for (const [name, s] of this.sections) {
      if (s.goodAt === 0) continue;
      const hash = createHash("sha1").update(JSON.stringify(s.data)).digest("base64url");
      // The data is the last good copy; `error` says the latest read failed.
      out.push({ name, at: s.goodAt, hash, data: s.data, error: s.ok ? null : s.error });
    }
    return out;
  }

  /** The document, at once. Only a section that has never been read is
   * waited for, and only so long. `asOf` is now when every section has
   * been read within its own interval, and earlier by however far the
   * most overdue section is behind: how stale the document really is. */
  async doc(name: DocName): Promise<{ etag: string; body: string; asOf: number }> {
    this.asked = this.now();
    const names = DOCS[name];
    const section = (n: string) => this.sections.get(n) as Section;
    const never = names.filter((n) => section(n).at === 0);
    if (never.length > 0) {
      let timer: ReturnType<typeof setTimeout> | undefined;
      await Promise.race([
        Promise.all(never.map((n) => this.refresh(n))),
        new Promise((resolve) => {
          timer = setTimeout(resolve, FIRST_LOAD_MS);
        }),
      ]);
      clearTimeout(timer);
    }
    const out: Record<string, unknown> = { v: 1, doc: name };
    const health: Record<string, { ok: boolean; error: string | null }> = {};
    for (const n of names) {
      const s = section(n);
      if (this.stale(s)) void this.refresh(n);
      out[n] = s.data;
      health[n] = { ok: s.ok, error: s.error };
    }
    out.health = health;
    if (name === "core") Object.assign(out, this.extras, { acts: this.acts });
    // The tag covers the content and not the times it was read, so a
    // document that has not changed costs the phone nothing. The read
    // times travel beside it.
    const etag = `"${createHash("sha1").update(JSON.stringify(out)).digest("base64url")}"`;
    const now = this.now();
    const overdue = names
      .map((n) => section(n))
      .filter((s) => s.goodAt > 0)
      .map((s) => Math.max(0, now - s.goodAt - s.ttlMs));
    const asOf = now - Math.max(0, ...overdue);
    out.read = Object.fromEntries(names.map((n) => [n, section(n).at ? new Date(section(n).at).toISOString() : null]));
    return { etag, body: JSON.stringify(out), asOf };
  }
}
