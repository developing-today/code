// What became of each action that was sent under a name.
//
// A device that is offline keeps what the owner did and sends it when
// it is connected again; the edge relays an action to this host and,
// when no answer comes back, holds it and offers it again. Either way
// the same action can arrive twice. Each comes under a name (a UUID
// given by whoever first sent it), and this is where it is written
// down that a name was started and how it ended, so that the second
// arrival is answered from the record and not done again.
import type { Store } from "./store.ts";

/** How an action sent under a name came out: what it answered with
 * when it worked (if that is small enough to keep), or why it did not
 * and the status that went with it. */
export interface Outcome {
  ok: boolean;
  error?: string;
  status?: number;
  result?: unknown;
}
type Fate = "started" | Outcome;

/** How many names are remembered. A name older than that many actions
 * is taken for new again. */
const KEPT = 300;

export class Named {
  /** The record, read once. Everyone who asks while it is being read
   * gets the same one: two readers with a record each would each write
   * theirs back without what the other had written down. */
  private fates: Promise<Map<string, Fate>> | null = null;
  /** Names being done right now, in this process. */
  private running = new Set<string>();
  /** The write under way, if any: the next one waits for it. */
  private writing: Promise<void> = Promise.resolve();

  constructor(private store: Store) {}

  private all(): Promise<Map<string, Fate>> {
    this.fates ??= (async () => {
      try {
        return new Map(Object.entries(JSON.parse((await this.store.read()) ?? "{}") as Record<string, Fate>));
      } catch {
        return new Map<string, Fate>();
      }
    })();
    return this.fates;
  }

  /** Write down what is known of a name. */
  async record(id: string, fate: Fate): Promise<void> {
    const fates = await this.all();
    fates.delete(id);
    fates.set(id, fate);
    while (fates.size > KEPT) fates.delete(fates.keys().next().value as string);
    await this.save();
  }

  /** One write at a time, each of the record as it stands when its turn
   * comes: an earlier write that was slow cannot land over a later one
   * and take a name out of the record. */
  private save(): Promise<void> {
    const mine = this.writing.then(async () => {
      await this.store.write(JSON.stringify(Object.fromEntries(await this.all())));
    });
    // A write that failed is its own caller's to hear of, not the next one's.
    this.writing = mine.catch(() => {});
    return mine;
  }

  /** What is written down for a name, if anything. */
  async get(id: string): Promise<Fate | undefined> {
    return (await this.all()).get(id);
  }

  isRunning(id: string): boolean {
    return this.running.has(id);
  }

  /** Take a name for doing, now. False when it is already being done
   * in this process. There is no waiting in this, so of two arrivals
   * under one name only one can take it, whatever each has yet to
   * write down. Whoever takes it gives it back with release() or end(). */
  take(id: string): boolean {
    if (this.running.has(id)) return false;
    this.running.add(id);
    return true;
  }

  /** Give back a name that was taken and, in the end, not done under
   * (it was refused before it began). */
  release(id: string): void {
    this.running.delete(id);
  }

  /** An action is about to be done under this name. Written down
   * first, so that if its answer is lost and it is offered again, it is
   * not done twice. */
  async begin(id: string): Promise<void> {
    this.running.add(id);
    await this.record(id, "started");
  }

  /** How it came out. */
  async end(id: string, fate: Outcome): Promise<void> {
    try {
      await this.record(id, fate);
    } finally {
      this.running.delete(id);
    }
  }

  /** What is written down for a name, whoever holds it now: nothing;
   * how it came out; or ("lost") that it was started and this process
   * stopped before it was known how it ended. For whoever has just
   * taken the name: read after taking, this cannot miss what another
   * wrote down before giving the name back. */
  async known(id: string): Promise<{ state: "lost" } | { state: "done"; fate: Outcome } | null> {
    const fate = await this.get(id);
    if (fate === undefined) return null;
    return fate === "started" ? { state: "lost" } : { state: "done", fate };
  }

  /** What is known of an action sent under this name: that it is being
   * done now, or what is written down for it. */
  async seen(id: string): Promise<{ state: "running" | "lost" } | { state: "done"; fate: Outcome } | null> {
    if (this.running.has(id)) return { state: "running" };
    return this.known(id);
  }
}
