// What is worth interrupting the owner for, and only once each.
//
// The state documents say how things stand; this compares them with what
// he has already been told and sends the difference: the fleet answered
// one of his prompts, something new needs his hands, new questions
// arrived, a unit failed. The first pass only takes stock, so turning
// notifications on never sets off a flood about things already there.
import type { Docs } from "./docs.ts";
import type { Class, Push } from "./push.ts";
import type { Store } from "./store.ts";

export interface Seen {
  primed: boolean;
  /** issue number -> time of the last fleet reply already announced */
  asks: Record<string, string>;
  hands: string[];
  questions: number;
  failed: string[];
  /** Units seen failed and not seen well since: when it was first seen,
   * and whether the owner has been told. Absent in a state written
   * before units that run on a timer were told apart. */
  failing?: Record<string, { since: number; told: boolean }>;
}
export const NOTHING_SEEN: Seen = { primed: false, asks: {}, hands: [], questions: 0, failed: [] };

/** A unit that runs on a timer is only worth a message once it has kept
 * failing this long: one bad run that the next run mends is not. */
export const SUSTAINED_MS = 5 * 60_000;

export interface Message {
  cls: Class;
  title: string;
  body: string;
  url: string;
  tag: string;
}

interface State {
  asks?: { issue: number; replies: { you: boolean; at: string; text: string }[] }[] | null;
  census?: { hands: { ref: string; title: string; action?: string }[] } | null;
  sessions?: { waiting: { id: string; title: string }[] } | null;
  questionCount?: { total: number } | null;
  services?: { unit: string; active: string; description?: string }[] | null;
}

const clip = (s: string, n: number) => (s.length > n ? `${s.slice(0, n - 1)}…` : s);

/** The messages owed for `now`, and what will have been seen once they
 * are sent. A section that could not be read (null) changes nothing. */
export function changes(seen: Seen, now: State, at: number = Date.now()): { messages: Message[]; seen: Seen } {
  const next: Seen = {
    primed: true,
    asks: { ...seen.asks },
    hands: seen.hands,
    questions: seen.questions,
    failed: seen.failed,
    ...(seen.failing ? { failing: seen.failing } : {}),
  };
  const messages: Message[] = [];
  for (const ask of now.asks ?? []) {
    const last = ask.replies[ask.replies.length - 1];
    if (!last || last.you || next.asks[ask.issue] === last.at) continue;
    next.asks[ask.issue] = last.at;
    messages.push({
      cls: "replies",
      title: `The fleet replied on #${ask.issue}`,
      body: clip(last.text, 180),
      url: "/console/#ask",
      tag: `ask-${ask.issue}`,
    });
  }
  if (now.census || now.sessions) {
    const hands = [
      ...(now.census?.hands ?? []).map((i) => ({ key: i.ref, text: i.action || i.title })),
      ...(now.sessions?.waiting ?? []).map((w) => ({ key: `session:${w.id}`, text: w.title })),
    ];
    for (const item of hands) {
      if (seen.hands.includes(item.key)) continue;
      messages.push({
        cls: "needs",
        title: "Something needs you",
        body: clip(item.text, 180),
        url: "/console/#needs/hands",
        tag: `hands-${item.key}`,
      });
    }
    next.hands = hands.map((i) => i.key);
  }
  if (now.questionCount) {
    const total = now.questionCount.total;
    if (total > seen.questions) {
      const more = total - seen.questions;
      messages.push({
        cls: "questions",
        title: more === 1 ? "1 new question for you" : `${more} new questions for you`,
        body: `${total} open in all.`,
        url: "/console/#needs/questions",
        tag: "questions",
      });
    }
    next.questions = total;
  }
  if (now.services) {
    // A service with a timer of the same name runs now and then: between
    // runs it shows as failed if its last run failed, and for a moment
    // during a run it shows as starting. Neither moment alone says much.
    const timers = new Set(
      now.services.filter((u) => u.unit.endsWith(".timer")).map((u) => u.unit.slice(0, -".timer".length)),
    );
    const onTimer = (unit: string) => unit.endsWith(".service") && timers.has(unit.slice(0, -".service".length));
    // Units the owner was told of before this was kept are not told again.
    const failing: Record<string, { since: number; told: boolean }> = {
      ...(seen.failing ?? Object.fromEntries(seen.failed.map((unit) => [unit, { since: at, told: true }]))),
    };
    const listed = new Set<string>();
    for (const u of now.services) {
      listed.add(u.unit);
      const before = failing[u.unit];
      if (u.active === "failed") {
        const entry = { ...(before ?? { since: at, told: false }) };
        const sustained = at - entry.since >= SUSTAINED_MS;
        if (!entry.told && (!onTimer(u.unit) || sustained)) {
          entry.told = true;
          messages.push(
            onTimer(u.unit)
              ? {
                  cls: "trouble",
                  title: "A unit keeps failing",
                  body: clip(
                    `${u.unit}, every run for ${Math.round((at - entry.since) / 60_000)} minutes${u.description ? `. ${u.description}` : ""}`,
                    180,
                  ),
                  url: "/console/#more/services",
                  tag: `unit-${u.unit}`,
                }
              : {
                  cls: "trouble",
                  title: "A unit failed",
                  body: u.unit,
                  url: "/console/#more/services",
                  tag: `unit-${u.unit}`,
                },
          );
        }
        failing[u.unit] = entry;
      } else if (u.active === "activating" || u.active === "deactivating" || u.active === "reloading") {
        // In the middle of a run: not failed, and not yet well either.
      } else if (before) {
        // Seen well again. One that was reported as failing run after
        // run gets its ending said too.
        if (before.told && onTimer(u.unit)) {
          messages.push({
            cls: "trouble",
            title: "A unit is working again",
            body: u.unit,
            url: "/console/#more/services",
            tag: `unit-${u.unit}`,
          });
        }
        delete failing[u.unit];
      }
    }
    for (const unit of Object.keys(failing)) if (!listed.has(unit)) delete failing[unit];
    next.failing = failing;
    next.failed = Object.keys(failing);
  }
  return { messages: seen.primed ? messages : [], seen: next };
}

/** Sections the notifier reads, whether or not anyone is looking. */
const WATCHED = ["asks", "census", "sessions", "questionCount", "services"];

export class Notifier {
  private seen: Seen | null = null;
  private timer: ReturnType<typeof setInterval> | null = null;
  private busy = false;

  constructor(
    private docs: Docs,
    private push: Push,
    private store: Store,
    private now: () => number = Date.now,
  ) {}

  private async load(): Promise<Seen> {
    if (this.seen) return this.seen;
    try {
      this.seen = { ...NOTHING_SEEN, ...(JSON.parse((await this.store.read()) ?? "") as Seen) };
    } catch {
      this.seen = NOTHING_SEEN;
    }
    return this.seen;
  }

  /** One pass. Does nothing while no device is subscribed. Sections in
   * `skip` were just read from their live source and are compared as
   * they are, not reloaded from the mirror first. */
  async tick(skip: string[] = []): Promise<number> {
    if (this.busy) return 0;
    this.busy = true;
    try {
      if ((await this.push.status()).devices === 0) return 0;
      await this.docs.touch(WATCHED.filter((n) => !skip.includes(n)));
      const now = Object.fromEntries(WATCHED.map((n) => [n, this.docs.good(n)])) as State;
      const { messages, seen } = changes(await this.load(), now, this.now());
      for (const m of messages) await this.push.send(m.cls, m);
      // Written down only when it differs: most passes find nothing new.
      const before = JSON.stringify(this.seen);
      this.seen = seen;
      if (JSON.stringify(seen) !== before) await this.store.write(JSON.stringify(seen));
      return messages.length;
    } finally {
      this.busy = false;
    }
  }

  start(everyMs = 60_000): void {
    this.timer ??= setInterval(
      () => void this.tick().catch((e) => console.error(`pm-console: notify: ${e.message}`)),
      everyMs,
    );
  }
}
