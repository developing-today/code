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
  private fates: Map<string, Fate> | null = null;
  /** Names being done right now, in this process. */
  private running = new Set<string>();

  constructor(private store: Store) {}

  private async all(): Promise<Map<string, Fate>> {
    if (this.fates) return this.fates;
    try {
      this.fates = new Map(Object.entries(JSON.parse((await this.store.read()) ?? "{}") as Record<string, Fate>));
    } catch {
      this.fates = new Map();
    }
    return this.fates;
  }

  /** Write down what is known of a name. */
  async record(id: string, fate: Fate): Promise<void> {
    const fates = await this.all();
    fates.delete(id);
    fates.set(id, fate);
    while (fates.size > KEPT) fates.delete(fates.keys().next().value as string);
    await this.store.write(JSON.stringify(Object.fromEntries(fates)));
  }

  /** What is written down for a name, if anything. */
  async get(id: string): Promise<Fate | undefined> {
    return (await this.all()).get(id);
  }

  isRunning(id: string): boolean {
    return this.running.has(id);
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

  /** What is known of an action sent under this name: nothing; that it
   * is being done now; how it came out; or ("lost") that it was started
   * and this process stopped before it was known how it ended. */
  async seen(id: string): Promise<{ state: "running" | "lost" } | { state: "done"; fate: Outcome } | null> {
    if (this.running.has(id)) return { state: "running" };
    const fate = await this.get(id);
    if (fate === undefined) return null;
    return fate === "started" ? { state: "lost" } : { state: "done", fate };
  }
}
