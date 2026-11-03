// The cross-stack event log (#1739). One append-only chronological
// stream of what happened across the pm-stack: dashboard action
// outcomes, orchestration publishes / tier changes, and session
// registry outcomes — the facts the actions store, the orchestration
// section, and the session registry each held separately, with no
// single stream a human (or /today) could read.
//
// Storage follows the views/comments store discipline (#906, #1276):
// one JSON file in the aggregator's data directory
// (PM_AGG_EVENTS_FILE); writes are atomic (unique temp file + rename)
// and serialized through a per-store promise chain; a missing file is
// the only silent empty start — an unparseable or wrongly-shaped file
// is quarantined to <file>.corrupt-<timestamp> with a loud log, never
// silently overwritten.
//
// Retention is bounded (#1739 acceptance): records older than 30 days
// or beyond the newest 5000 are dropped on append and on load,
// whichever bound hits first.
//
// Writers are fire-and-forget (see emitEvent): an event write failure
// never blocks or fails the underlying action, publish, or merge.
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname } from "node:path";

export interface StreamEvent {
  /** ISO timestamp of the event. */
  at: string;
  /** Who did it: drew / orchestrator / keeper / a job id / a session. */
  actor: string;
  /** action-applied | action-rejected | tier-change |
   *  orchestration-published | session-outcome | comment | ... */
  kind: string;
  /** The item / session / goal the event concerns. */
  ref: string;
  /** One-line human-readable summary. */
  summary: string;
}

export type NewStreamEvent = Omit<StreamEvent, "at"> & { at?: string };

export interface EventsFile {
  version: 1;
  events: StreamEvent[];
}

export const EVENT_RETENTION_DAYS = 30;
export const EVENT_RETENTION_MAX = 5000;

const FIELD = /^[\w ./#:@-]+$/;

export function validateStreamEvent(e: unknown): { ok: true; value: StreamEvent } | { ok: false; error: string } {
  if (typeof e !== "object" || e === null) return { ok: false, error: "event must be an object" };
  const v = e as Record<string, unknown>;
  if (typeof v.at !== "string" || !Number.isFinite(Date.parse(v.at))) {
    return { ok: false, error: "event.at must be an ISO timestamp string" };
  }
  // Identity fields align with their upstream contracts (#4489 Codex
  // re-review): the action-result route accepts any nonblank byRun
  // (stored as appliedByRun, sliced to 200) - punctuation such as
  // parentheses included - and an action objectRef is any
  // whitespace-free string of at most 300 chars (isObjectRef in
  // comments.ts). kind is an internal constant, so it keeps the
  // narrow charset.
  if (typeof v.actor !== "string" || v.actor.trim() === "" || v.actor.trim().length > 200) {
    return { ok: false, error: "event.actor must be a non-empty string of at most 200 chars" };
  }
  if (typeof v.kind !== "string" || v.kind.trim() === "" || v.kind.length > 200 || !FIELD.test(v.kind)) {
    return { ok: false, error: "event.kind must be a non-empty string" };
  }
  if (typeof v.ref !== "string" || v.ref.trim() === "" || v.ref.trim().length > 300 || /\s/.test(v.ref.trim())) {
    return { ok: false, error: "event.ref must be a non-empty ref string of at most 300 chars" };
  }
  if (typeof v.summary !== "string" || v.summary.trim() === "" || v.summary.length > 500) {
    return { ok: false, error: "event.summary must be a non-empty string of at most 500 chars" };
  }
  return {
    ok: true,
    value: {
      at: v.at,
      actor: (v.actor as string).trim(),
      kind: (v.kind as string).trim(),
      ref: (v.ref as string).trim(),
      summary: v.summary.trim(),
    },
  };
}

function isStreamEvent(value: unknown): value is StreamEvent {
  return validateStreamEvent(value).ok;
}

/** Drop records past the retention bounds (#1739): older than
 *  EVENT_RETENTION_DAYS, or beyond the newest EVENT_RETENTION_MAX.
 *  Input and output are chronological (oldest first). */
export function pruneEvents(events: readonly StreamEvent[], now = new Date()): StreamEvent[] {
  const cutoff = now.getTime() - EVENT_RETENTION_DAYS * 24 * 60 * 60 * 1000;
  const fresh = events
    .map((event, index) => ({ event, index }))
    .filter(({ event }) => Date.parse(event.at) >= cutoff)
    .sort((a, b) => Date.parse(a.event.at) - Date.parse(b.event.at) || a.index - b.index)
    .map(({ event }) => event);
  return fresh.length > EVENT_RETENTION_MAX ? fresh.slice(fresh.length - EVENT_RETENTION_MAX) : fresh;
}

export interface EventFilter {
  kind?: string;
  ref?: string;
  actor?: string;
}

/** The read surface: newest first, exact-match filters on
 *  kind / ref / actor (case-insensitive, like the views matcher). */
export function filterEvents(events: readonly StreamEvent[], filter: EventFilter = {}): StreamEvent[] {
  const eq = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();
  // Newest first by PARSED time (#4489 Codex re-review): accepted
  // timestamps may carry offsets or omit fractional seconds, where a
  // string comparison misorders them; equal instants keep the later
  // arrival first, matching append order.
  return [...events]
    .map((event, index) => ({ event, index }))
    .sort((a, b) => Date.parse(b.event.at) - Date.parse(a.event.at) || b.index - a.index)
    .map(({ event }) => event)
    .filter(
      (e) =>
        (!filter.kind || eq(e.kind, filter.kind)) &&
        (!filter.ref || eq(e.ref, filter.ref)) &&
        (!filter.actor || eq(e.actor, filter.actor)),
    );
}

/**
 * The fire-and-forget writer every emission point uses (#1739
 * acceptance: writing an event never blocks the underlying action).
 * Never throws: a failed append is logged and reported as `false`;
 * the caller's action / publish / merge proceeds regardless.
 */
export async function emitEvent(
  store: { record: (event: NewStreamEvent) => Promise<StreamEvent> },
  event: NewStreamEvent,
): Promise<boolean> {
  try {
    await store.record(event);
    return true;
  } catch (err) {
    console.error(
      `[events] WARNING: event append failed (${err instanceof Error ? err.message : String(err)}); ` +
        `the underlying action proceeds — event lost: ${event.kind} ${event.ref}`,
    );
    return false;
  }
}

// -- the store ------------------------------------------------------------

export class EventsStore {
  private events: StreamEvent[] = [];
  private loadPromise: Promise<void> | null = null;
  private writeChain: Promise<void> = Promise.resolve();
  private writeSeq = 0;

  constructor(private readonly file: string) {}

  async load(): Promise<void> {
    if (!this.loadPromise) this.loadPromise = this.doLoad();
    return this.loadPromise;
  }

  private async doLoad(): Promise<void> {
    let raw: string;
    try {
      raw = await readFile(this.file, "utf8");
    } catch (err) {
      if ((err as { code?: unknown })?.code === "ENOENT") {
        this.events = [];
        return;
      }
      this.events = [];
      await this.quarantine(`could not be read (${err instanceof Error ? err.message : String(err)})`);
      return;
    }
    try {
      const parsed: unknown = JSON.parse(raw);
      if (typeof parsed !== "object" || parsed === null || !Array.isArray((parsed as EventsFile).events)) {
        throw new Error("store file does not have the expected { events: [...] } shape");
      }
      const rows = (parsed as { events: unknown[] }).events;
      const malformedIndex = rows.findIndex((event) => !isStreamEvent(event));
      if (malformedIndex !== -1) {
        throw new Error(`event row ${malformedIndex} does not have the expected StreamEvent shape`);
      }
      this.events = pruneEvents(rows.filter(isStreamEvent));
    } catch (err) {
      this.events = [];
      await this.quarantine(`is unparseable (${err instanceof Error ? err.message : String(err)})`);
    }
  }

  private async quarantine(why: string): Promise<void> {
    const quarantined = `${this.file}.corrupt-${new Date().toISOString().replace(/[:.]/g, "-")}`;
    try {
      await rename(this.file, quarantined);
      console.error(
        `[events] WARNING: events store ${this.file} ${why}; quarantined the original bytes to ${quarantined} and started empty. ` +
          `The quarantined file holds the event history — do not delete it.`,
      );
    } catch (renameErr) {
      console.error(
        `[events] WARNING: events store ${this.file} ${why} and could not be quarantined ` +
          `(${renameErr instanceof Error ? renameErr.message : String(renameErr)}); starting empty — ` +
          `the next persist will overwrite it, so copy it aside now to preserve the event history.`,
      );
    }
  }

  private persist(): Promise<void> {
    const run = this.writeChain.then(async () => {
      const data: EventsFile = { version: 1, events: this.events };
      await mkdir(dirname(this.file), { recursive: true });
      const tmp = `${this.file}.tmp-${process.pid}-${++this.writeSeq}`;
      await writeFile(tmp, JSON.stringify(data, null, 2));
      await rename(tmp, this.file);
    });
    this.writeChain = run.catch(() => {});
    return run;
  }

  /** All retained events, newest first, with optional filters. */
  async list(filter: EventFilter = {}): Promise<StreamEvent[]> {
    await this.load();
    return filterEvents(this.events, filter);
  }

  /** Durable identity check used to avoid replaying an orchestration run
   *  after process restart. */
  async has(kind: string, ref: string): Promise<boolean> {
    await this.load();
    return this.events.some((event) => event.kind === kind && event.ref === ref);
  }

  /** Append one event (retention enforced on append). */
  async record(input: NewStreamEvent): Promise<StreamEvent> {
    await this.load();
    const parsed = validateStreamEvent({ ...input, at: input.at ?? new Date().toISOString() });
    if (!parsed.ok) throw new Error(parsed.error);
    this.events = pruneEvents([...this.events, parsed.value]);
    await this.persist();
    return parsed.value;
  }
}
