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
        s.data = await s.load();
        s.ok = true;
        s.error = null;
        s.goodAt = this.now();
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

  logAct(act: Act): void {
    this.acts = [act, ...this.acts.filter((a) => a.id !== act.id)].slice(0, ACTS_KEPT);
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
    if (name === "core") out.acts = this.acts;
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
