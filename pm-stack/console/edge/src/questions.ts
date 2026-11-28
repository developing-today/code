// Questions discovery at the edge (#13491, slice 2): the decision
// sets' activity, read from the decision-events Worker itself.
//
// Decision sets live with the aggregator on the laptop, and their
// events already go to the decision-events Worker. That Worker now
// enumerates what it holds (GET /sets, landed by PR #14616) and says
// each set's fanned-out state (GET /sets/<setId>/state). Reading both
// here means the Questions screen can show which sets exist and what
// has happened in them while the laptop is away — no machine report
// is in this path.
//
// What this is not: the questions themselves. Their text, options and
// recommendations still live in the aggregator's decisions file, and
// recording an answer still goes through the laptop's endpoint. The
// discovery this returns says so plainly (`definitions`), so the
// screen never presents a set's activity as its questions.

/** Where the decision-events Worker is, and the shared secret it
 * takes: the same secret it accepts on its reads (a Bearer token, or
 * the X-Webhook-Secret header). The value is never logged or returned. */
export interface DecisionEventsSource {
  url: string;
  secret: string;
}

/** One set as the Worker's enumeration names it: its activity, newest
 * first in the list the Worker returns. */
export interface QuestionSetListed {
  setId: string;
  events: number;
  firstTs: string | null;
  lastTs: string | null;
  lastEventId: string | null;
  lastType: string | null;
}

/** One set's state as its Durable Object and D1 together say it:
 * how many events were fanned out, the latest of them, and how many
 * events D1 holds for the set. */
export interface QuestionSetState {
  fannedOut: number | null;
  lastEventId: string | null;
  lastType: string | null;
  lastTs: string | null;
  updatedAt: string | null;
  subscribers: number | null;
  d1Events: number | null;
}

/** One discovered set: the enumeration's row, with its state read
 * back, or the reason the state could not be read. */
export interface QuestionSetDiscovery {
  setId: string;
  events: number;
  firstTs: string | null;
  lastTs: string | null;
  lastEventId: string | null;
  lastType: string | null;
  state: QuestionSetState | null;
  stateError?: string;
}

/** The Questions discovery section the edge reads itself: the sets
 * the Worker holds, in the order it named them (newest activity
 * first), each with its state. `definitions` says what this section
 * cannot give: the questions' text and the answer write path still
 * need the laptop aggregator. */
export interface QuestionsDiscovery {
  source: "decision-events";
  laptopRequired: false;
  definitions: "unavailable";
  note: string;
  sets: QuestionSetDiscovery[];
}

export const QUESTIONS_DISCOVERY_NOTE =
  "These are the decision sets the decision-events Worker holds, read by Cloudflare itself: " +
  "a set's questions and answering them still need the laptop, whose report brings them when it is back.";

const str = (value: unknown): string | null => (typeof value === "string" && value !== "" ? value : null);
const num = (value: unknown): number | null => (typeof value === "number" && Number.isFinite(value) ? value : null);

/** A set row as the Worker lists it, or null when what came back is
 * not one: a row that cannot be read is left out rather than shown
 * half-made, and a list that is not a list fails the whole read. */
function listedSet(raw: unknown): QuestionSetListed | null {
  const row = raw as Record<string, unknown> | null;
  if (!row || typeof row !== "object") return null;
  if (typeof row.setId !== "string" || row.setId === "") return null;
  const events = num(row.events);
  if (events === null) return null;
  return {
    setId: row.setId,
    events,
    firstTs: str(row.firstTs),
    lastTs: str(row.lastTs),
    lastEventId: str(row.lastEventId),
    lastType: str(row.lastType),
  };
}

/** A set's state as the Worker answers GET /sets/<setId>/state: the
 * Durable Object's own state under `object`, merged there with D1's
 * count as `d1Events`. Anything missing reads as null, never as 0. */
function setState(raw: unknown): QuestionSetState {
  const body = (raw ?? {}) as Record<string, unknown>;
  const object = (body.object ?? {}) as Record<string, unknown>;
  return {
    fannedOut: num(object.fannedOut),
    lastEventId: str(object.lastEventId),
    lastType: str(object.lastType),
    lastTs: str(object.lastTs),
    updatedAt: str(object.updatedAt),
    subscribers: num(object.subscribers),
    d1Events: num(body.d1Events),
  };
}

type FetchLike = (
  url: string,
  init?: { headers?: Record<string, string> },
) => Promise<{
  ok: boolean;
  status: number;
  json: () => Promise<unknown>;
}>;

/** Read the Questions discovery from the decision-events Worker:
 * list the sets it holds, then read each set's state. The list is
 * the read that matters — if it fails, the whole read fails and the
 * caller keeps its last good copy. One set's state failing leaves
 * that set listed, with the reason beside it: discovery does not
 * hide a set because its state could not be read. */
export async function readQuestionSets(
  source: DecisionEventsSource,
  fetchImpl: FetchLike,
): Promise<QuestionsDiscovery> {
  const base = source.url.replace(/\/+$/, "");
  const headers = { Authorization: `Bearer ${source.secret}` };
  const listRes = await fetchImpl(`${base}/sets`, { headers });
  if (!listRes.ok) {
    throw new Error(`the decision-events Worker answered ${listRes.status} when asked for its sets`);
  }
  const listBody = (await listRes.json().catch(() => null)) as { sets?: unknown } | null;
  if (!listBody || !Array.isArray(listBody.sets)) {
    throw new Error("the decision-events Worker did not answer with a list of sets");
  }
  const listed = listBody.sets.map(listedSet).filter((s): s is QuestionSetListed => s !== null);
  const sets: QuestionSetDiscovery[] = [];
  for (const set of listed) {
    try {
      const res = await fetchImpl(`${base}/sets/${encodeURIComponent(set.setId)}/state`, { headers });
      if (!res.ok) throw new Error(`the decision-events Worker answered ${res.status} for this set's state`);
      sets.push({ ...set, state: setState(await res.json().catch(() => null)) });
    } catch (error) {
      sets.push({ ...set, state: null, stateError: (error as Error).message.slice(0, 200) });
    }
  }
  return {
    source: "decision-events",
    laptopRequired: false,
    definitions: "unavailable",
    note: QUESTIONS_DISCOVERY_NOTE,
    sets,
  };
}
