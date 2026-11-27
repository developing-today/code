/**
 * C2 drain signals (#9570 design record, D2 + D4; slice C2-S1).
 *
 * A per-kind queue is a cursor over the `events` log: its depth is
 * the count of rows of that kind past the `drain:<kind>` watermark
 * in `agent_watermarks`, and its oldest age reads the smallest
 * `created_at` in that range (both over the existing
 * `(kind, created_at)` index). When a queue crosses its threshold
 * the Worker sets `drain-due:<kind>` to the current time — unless
 * the key is already set, so a standing signal is never moved. The
 * Muse host's detector reads the `drain-due:*` keys and wakes the
 * drain (C2-S2); the drain clears a signal only after advancing
 * its cursor, so a failed drain leaves the signal set.
 *
 * Signals are raised two ways (D4): on append, after a derived row
 * lands (count leg); and on the Worker's 5-minute age cron (age
 * leg — it cannot fire on an append, and it runs on the Worker so
 * the signal is raised even while the Muse host is down).
 *
 * Reads and watermark writes go direct to the fleet-queue store
 * through the Turso client, like the existing `dirty:<repo>`
 * watermark writes in sink.ts; the broker is the append path only.
 */
import { Turso, iArg, tArg } from "./turso";
import type { SinkEnv } from "./sink";
import { QUEUE_KINDS, QUEUE_THRESHOLDS } from "./thresholds";

function queue(env: SinkEnv): Turso {
  return new Turso(env.TURSO_QUEUE_URL, env.TURSO_QUEUE_TOKEN);
}

/** The drain cursor for a kind: the highest event id the drain has
 * handled. Stored as the plain integer in `drain:<kind>`; an absent
 * or unreadable value means 0 (nothing drained yet). */
export async function readCursor(q: Turso, kind: string): Promise<number> {
  const raw = await q.queryText("SELECT value FROM agent_watermarks WHERE key = ?", [tArg(`drain:${kind}`)]);
  const n = raw === null ? NaN : Number.parseInt(raw, 10);
  return Number.isFinite(n) && n > 0 ? n : 0;
}

/** Depth of a kind's queue: rows of that kind past the cursor. */
export async function queueDepth(q: Turso, kind: string, cursor: number): Promise<number> {
  const raw = await q.queryText("SELECT COUNT(*) FROM events WHERE kind = ? AND id > ?", [
    tArg(kind),
    iArg(cursor),
  ]);
  const n = raw === null ? NaN : Number.parseInt(raw, 10);
  return Number.isFinite(n) && n > 0 ? n : 0;
}

/** Oldest queued row's created_at for a kind, or null when empty. */
export async function queueOldest(q: Turso, kind: string, cursor: number): Promise<string | null> {
  return q.queryText("SELECT MIN(created_at) FROM events WHERE kind = ? AND id > ?", [tArg(kind), iArg(cursor)]);
}

/**
 * Set `drain-due:<kind>` to now, unless it is already set. Returns
 * true when this call set it. A standing signal is left untouched:
 * its timestamp is when the kind first became due.
 */
export async function setDrainDue(q: Turso, kind: string, nowIso: string): Promise<boolean> {
  const existing = await q.queryText("SELECT value FROM agent_watermarks WHERE key = ?", [
    tArg(`drain-due:${kind}`),
  ]);
  if (existing !== null) return false;
  await q.execute(
    `INSERT INTO agent_watermarks (key, value, updated_at) VALUES (?, ?, ?)
     ON CONFLICT(key) DO NOTHING`,
    [tArg(`drain-due:${kind}`), tArg(nowIso), tArg(nowIso)],
  );
  return true;
}

export interface AppendSignal {
  kind: string;
  depth: number;
  signaled: boolean;
}

/** Count leg (D4.1): after a derived row of `kind` lands, set its
 * drain-due signal when the queue depth reaches the count
 * threshold. */
export async function signalOnAppend(env: SinkEnv, kind: string, now: Date): Promise<AppendSignal> {
  const threshold = QUEUE_THRESHOLDS[kind];
  if (!threshold) return { kind, depth: 0, signaled: false };
  const q = queue(env);
  const depth = await queueDepth(q, kind, await readCursor(q, kind));
  const signaled = depth >= threshold.count ? await setDrainDue(q, kind, now.toISOString()) : false;
  return { kind, depth, signaled };
}

export interface AgeSignal {
  kind: string;
  oldest: string | null;
  ageMinutes: number | null;
  signaled: boolean;
}

/** Age leg (D4.2): one pass over every queued kind, setting
 * drain-due where the oldest queued item has reached the kind's
 * age threshold. A kind that errors is logged and skipped — one
 * bad kind never blocks the others. Run by the age cron. */
export async function ageSweep(env: SinkEnv, now: Date): Promise<AgeSignal[]> {
  const q = queue(env);
  const out: AgeSignal[] = [];
  for (const kind of QUEUE_KINDS) {
    try {
      const oldest = await queueOldest(q, kind, await readCursor(q, kind));
      const ageMinutes =
        oldest === null ? null : Math.floor((now.getTime() - Date.parse(oldest)) / 60000);
      const signaled =
        ageMinutes !== null && ageMinutes >= QUEUE_THRESHOLDS[kind].ageMinutes
          ? await setDrainDue(q, kind, now.toISOString())
          : false;
      out.push({ kind, oldest, ageMinutes, signaled });
    } catch (e) {
      console.error(`github-ci drain age sweep failed for ${kind}: ${e instanceof Error ? e.message : String(e)}`);
      out.push({ kind, oldest: null, ageMinutes: null, signaled: false });
    }
  }
  return out;
}
