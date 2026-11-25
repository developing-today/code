// What the edge does itself: the console's own classes, with stand-ins
// only where they touch the outside world.
//
// Ops, Work and Plan take a `run` function and a `fetch`. At the edge,
// `run` turns `fleet gh api` argv into GitHub REST calls and
// `fleet secrets get <queue token name>` into the Worker secret, and
// refuses every other fleet verb as needing a machine; `fetch` is the
// Worker's own, which already reaches the queue store. So the board,
// pull requests, ranked issues, prompts, roadmaps, bugs and intake are
// read, and the writes that are one call to GitHub or the queue store
// are run, by the same code a machine runs: nothing here re-implements
// their shapes.
//
// The bridge is the edge's too, when it holds the PM stack's own
// bridge credential (#14373): `run` then turns `fleet bridge` argv
// into calls to the bridge Worker over its service binding,
// presenting that credential, so a message the owner sends from the
// app is delivered by the edge itself — as service:pm-stack, the
// bridge's name for the PM stack — and the agents list, the message
// log and the unified pipeline, directory and credential views
// (#14374) are read there too. Without the credential and the
// binding, `fleet bridge` stays a machine's, exactly as before.
//
// What stays a machine's: `fleet queue enqueue` (the fleet binary is
// not here), `systemctl`, and anything that reads the dashboard or
// the decisions file, which live on a machine alone.
// A prompt, a note or a bug is filed at the edge and only its queue
// task is held for a machine, as `prompt.enqueue` or `triage.enqueue`:
// the issue is never filed twice.
import { OpError } from "../../src/errors.ts";
import type { Config } from "../../src/config.ts";
import type { EdgeConfig } from "../../src/edge-proto.ts";
import { Ops, type Fetch, type Run, type RunResult } from "../../src/ops.ts";
import { Plan } from "../../src/plan.ts";
import { timed } from "../../src/timed.ts";
import { Work } from "../../src/work.ts";
import { memoryShelf, type Shelf, Trends } from "../../src/trends.ts";

/** Sections the edge reads itself. `agents` is read through the
 * bridge, so it is only the edge's while it holds the PM bridge
 * credential (edge.ts `owned` says which of these are its, case by
 * case); the pipeline's unread-instruction count is the bridge's
 * the same way (#14377); the rest are GitHub's and the queue store's. */
export const DIRECT_READ_SECTIONS = ["board", "prs", "priorities", "asks", "census", "done", "agents", "pipeline"] as const;
/** Actions the edge runs itself: each is one or a few calls to GitHub
 * or the queue store, and needs nothing a machine has. */
export const DIRECT_ACTIONS = [
  "ask.send",
  "ask.reply",
  "ask.close",
  "issue.comment",
  "need.comment",
  "hands.done",
  "issue.rank",
  "task.move",
  "pr.merge",
  "deploy.start",
  "intake.send",
] as const;

/** One call this side made of GitHub or the queue store, and how it was
 * answered: `status` 0 is no answer at all, and 304 is GitHub saying
 * the answer this side kept still stands. `write` is anything that may
 * have changed something. `limited` is a call turned away for being
 * one too many just now, not for what the token may do; `until` is
 * when GitHub says it may be asked again, where it says. `unsent` is a
 * call this side did not make, because GitHub had said not to ask
 * before then: it is recorded as the 429 it would have been, and as
 * it was not sent it wrote nothing and GitHub answered nothing. */
export interface Outward {
  to: "github" | "queue" | "bridge";
  write: boolean;
  status: number;
  limited?: boolean;
  until?: number;
  unsent?: boolean;
}

/** Which of them each section this side reads is read from. */
export const READ_FROM: Record<(typeof DIRECT_READ_SECTIONS)[number], Outward["to"][]> = {
  board: ["queue"],
  prs: ["github"],
  priorities: ["github"],
  asks: ["github", "queue"],
  census: ["github"],
  done: ["queue", "github"],
  agents: ["bridge"],
  pipeline: ["bridge"],
};

export interface DirectSecrets {
  githubToken?: string;
  queueToken?: string;
  /** The PM stack's own bridge credential (#14373): minted on the
   * bridge for service:pm-stack, held here as a Worker secret (put
   * there by scripts/expose.py --edge-direct, from the vault name it
   * reads). With `bridgeFetch` it makes `fleet bridge` this side's:
   * sends go out as service:pm-stack, and the agents list and the
   * message log are read with its read scope. Absent either, the
   * bridge stays a machine's. The value is never logged or returned. */
  bridgeToken?: string;
  /** The bridge Worker's fetch: its service binding's, in the Worker.
   * A stand-in in tests. */
  bridgeFetch?: Fetch;
  /** Told of every call made, as it is answered. */
  seen?: (call: Outward) => void;
  /** Where GitHub's API and the queue store are, when a local run puts
   * stand-ins in their place. Deployed, neither is given: GitHub's own
   * address is used, and the queue store's is the one a machine reports. */
  githubApi?: string;
  queueDb?: string;
  /** What this side remembers of GitHub between calls: its answers by
   * address, and when it said to ask again. Given by whoever outlives
   * the classes (the edge's object); without it nothing is remembered. */
  github?: GitHubMemory;
  /** The clock, where a test sets it. */
  now?: () => number;
  /** Where the history behind the Trends screen is kept. Without it,
   * it is kept in memory and read again when the object next starts. */
  shelf?: Shelf;
}

const GITHUB_API = "https://api.github.com";
/** How long GitHub has to answer a call, body and all. */
const GITHUB_ASK_MS = 20_000;

/** One answer of GitHub's, with the tag GitHub gave it. */
export interface KeptAnswer {
  etag: string;
  text: string;
}

/** What is remembered of GitHub between calls.
 *
 * Its answers, each under the address it was asked at. Asked again with
 * the answer's tag, GitHub says "not modified" for one that has not
 * changed, and a call answered that way does not count against the
 * account's allowance for the hour: the agents draw on that allowance
 * too, and a screen left open asks for the same lists every minute.
 * A tag belongs to the token it was given to; after a new token the
 * old tags only get full answers again.
 *
 * And when it said to ask again, after turning a call away for being
 * one too many, with the words it said it in: until then nothing that
 * draws on the same allowance is asked of it at all.
 *
 * None of this can live in the edge object's memory. Cloudflare puts an
 * idle object away within seconds and starts another on the next
 * request, with nothing in memory, and a list is read again a minute
 * or more after it was last read: what was kept in memory was gone
 * every time it was wanted (seen in service on 2026-11-24, the day
 * after it was first built that way). It is kept in a store that
 * outlives the object. */
export interface GitHubMemory {
  get(url: string): Promise<KeptAnswer | undefined>;
  set(url: string, answer: KeptAnswer): Promise<void>;
  delete(url: string): Promise<void>;
  /** Until when GitHub said not to ask, by the allowance a call draws on. */
  quiet(): Promise<Partial<Record<Allowance, Quiet>>>;
  /** GitHub said not to ask before then of what draws on these. A
   * nearer time never shortens a further one. */
  hush(spent: readonly Allowance[], quiet: Quiet): Promise<void>;
  /** The waits are forgotten: the owner said to try again now. */
  free(): Promise<void>;
  /** One more answer of GitHub's, and whether it was "not modified". */
  count(unchanged: boolean): Promise<void>;
  /** For health: how many answers are kept, and GitHub's answers
   * counted since `since`, with how many were "not modified". */
  said(): Promise<{ kept: number; answers: number; unchanged: number; since: number }>;
}

/** What a call draws on. GitHub counts searches apart, a few dozen to
 * the minute, from everything else this side asks, which is counted by
 * the hour: one running out says nothing of the other. */
export type Allowance = "core" | "search";
export const ALLOWANCES: readonly Allowance[] = ["core", "search"];
/** Not to be asked before `until` (milliseconds), GitHub having said `why`. */
export interface Quiet {
  until: number;
  why: string;
}

/** The longest this side stays quiet on GitHub's word: a little over
 * the hour its allowance is counted by. A time further off than that
 * is a header that cannot be right. */
export const QUIET_MOST_MS = 65 * 60_000;

/** Where the memory is kept: values by key, in a store that outlives
 * whatever is using it. The edge's is its object's own storage. */
export interface MemoryStore {
  get(key: string): Promise<unknown>;
  put(key: string, value: unknown): Promise<void>;
  delete(key: string): Promise<void>;
}

/** A store in a map, for where nothing needs to outlive anything (the
 * tests, and a caller given no store of its own). */
export function mapStore(kept: Map<string, unknown> = new Map()): MemoryStore & { kept: Map<string, unknown> } {
  return {
    kept,
    get: async (key) => kept.get(key),
    put: async (key, value) => {
      kept.set(key, structuredClone(value));
    },
    delete: async (key) => {
      kept.delete(key);
    },
  };
}

/** One stored value holds at most this many characters of an answer
 * (the store takes about two million bytes a value, a character can be
 * two of them, and a list of open pull requests can be more than that
 * by itself). */
export const ANSWER_PIECE = 600_000;
/** An answer asked for is marked as still wanted at most this often,
 * so that being read does not cost a write each time. When room is
 * made, what has gone longest unmarked goes first. */
const TOUCH_MS = 6 * 60 * 60_000;

interface Entry {
  /** Which place its pieces are in: `gh:a:<place>:<n>`. */
  id: number;
  etag: string;
  pieces: number;
  size: number;
  at: number;
}
interface Index {
  entries: Record<string, Entry>;
}
/** One piece of an answer, marked with whose it is. */
interface Piece {
  u: string;
  e: string;
  t: string;
}
interface Count {
  answers: number;
  unchanged: number;
  since: number;
}

/** GitHub's answers, its waits and its count, kept in `store`.
 *
 * An answer is kept as pieces in a place of its own, each piece marked
 * with the address and tag it belongs to, under an entry in one index.
 * The pieces are written first and the index last, so what was started
 * and not finished (the object was put away part-way, or the store
 * failed) is never taken for an answer: its pieces do not carry what
 * the index names, and it reads as nothing kept. One change is made at
 * a time.
 *
 * When there are more than `most` answers, or more than `room`
 * characters in them, the one longest unmarked goes. One too big for
 * `one` is not kept. Places are few and used again (a new answer takes
 * the lowest that is free), and whoever writes to a place or empties it
 * clears whatever else is in it: so what a stopped write left behind is
 * unread baggage only until its place is next used, and a write that
 * finishes leaves nothing behind.
 *
 * Nothing here throws at its caller for the store's sake. A store that
 * cannot be written is a memory that forgot. One that cannot be read
 * is not written to either (what it holds is not known, and is not to
 * be written over with less), and is tried again by the next to ask. */
export function githubMemory(
  store: MemoryStore = mapStore(),
  options: { now?: () => number; room?: number; most?: number; one?: number; piece?: number } = {},
): GitHubMemory {
  const now = options.now ?? Date.now;
  const room = options.room ?? 30_000_000;
  const most = options.most ?? 240;
  const one = options.one ?? 6_000_000;
  const pieceSize = Math.max(1, options.piece ?? ANSWER_PIECE);
  /** The most pieces an answer that is kept can have. */
  const longest = Math.max(1, Math.ceil(one / pieceSize));
  const place = (id: number, n: number) => `gh:a:${id}:${n}`;

  interface State {
    index: Index;
    count: Count;
    /** Whether the store could be read. */
    read: boolean;
  }
  /** GitHub's waits, as this object knows them: what the store held
   * when it was read, and what has been said since. Kept here as well
   * as in the store, so that a store in trouble does not have this
   * object asking on while turned away. */
  const waits: Partial<Record<Allowance, Quiet>> = {};
  let loaded: Promise<State> | null = null;
  const load = (): Promise<State> => {
    if (loaded) return loaded;
    const mine = (async (): Promise<State> => {
      let read = true;
      const take = async <T>(key: string, fit: (value: unknown) => T | null, otherwise: T): Promise<T> => {
        try {
          return fit(await store.get(key)) ?? otherwise;
        } catch {
          read = false;
          return otherwise;
        }
      };
      const index = await take<Index>(
        "gh:index",
        (value) => {
          const v = value as Partial<Index> | undefined;
          if (!v || typeof v.entries !== "object" || v.entries === null) return null;
          const entries: Record<string, Entry> = {};
          const places = new Set<number>();
          for (const [url, e] of Object.entries(v.entries)) {
            if (
              e &&
              Number.isInteger(e.id) &&
              !places.has(e.id) &&
              typeof e.etag === "string" &&
              Number.isInteger(e.pieces) &&
              e.pieces >= 1 &&
              // (No more than an answer kept here can have: what is
              // read back says how many pieces to clear.)
              e.pieces <= longest &&
              Number.isInteger(e.size) &&
              e.size >= 0 &&
              e.size <= one &&
              typeof e.at === "number"
            ) {
              places.add(e.id);
              entries[url] = { id: e.id, etag: e.etag, pieces: e.pieces, size: e.size, at: e.at };
            }
          }
          return { entries };
        },
        { entries: {} },
      );
      const stored = await take<Partial<Record<Allowance, Quiet>>>(
        "gh:quiet",
        (value) => {
          const v = value as Record<string, Partial<Quiet>> | undefined;
          if (!v || typeof v !== "object") return null;
          const out: Partial<Record<Allowance, Quiet>> = {};
          for (const allowance of ALLOWANCES) {
            const q = v[allowance];
            if (q && typeof q.until === "number" && typeof q.why === "string") {
              out[allowance] = { until: q.until, why: q.why };
            }
          }
          return out;
        },
        {},
      );
      // A wait read back further off than one could have been set
      // for is not one GitHub gave, and is not kept to at all. (Cut
      // down to the longest instead, it would be cut down again by
      // every object that started, each from its own now, and never
      // end.) If GitHub is still turning this side away, the first
      // call says so, with a time that is its own.
      const latest = now() + QUIET_MOST_MS;
      for (const allowance of ALLOWANCES) {
        const q = stored[allowance];
        if (q && q.until <= latest && (waits[allowance]?.until ?? 0) < q.until) waits[allowance] = q;
      }
      const count = await take<Count>(
        "gh:count",
        (value) => {
          const v = value as Partial<Count> | undefined;
          return v && Number.isInteger(v.answers) && Number.isInteger(v.unchanged) && typeof v.since === "number"
            ? { answers: v.answers as number, unchanged: v.unchanged as number, since: v.since }
            : null;
        },
        { answers: 0, unchanged: 0, since: now() },
      );
      return { index, count, read };
    })();
    loaded = mine;
    void mine.then((state) => {
      if (!state.read && loaded === mine) loaded = null;
    });
    return mine;
  };
  /** The store asked to take something, where it could be read; and a
   * failure of its own let go. */
  const write = async (state: State, work: () => Promise<void>) => {
    if (!state.read) return;
    try {
      await work();
    } catch {
      // A memory that could not be written is one that forgot.
    }
  };
  let turn: Promise<void> = Promise.resolve();
  /** One change at a time: each is several writes, and two at once to
   * the same answer would leave pieces of both. */
  const inTurn = (work: () => Promise<void>): Promise<void> => {
    const mine = turn.then(work);
    turn = mine.catch(() => {});
    return mine;
  };
  /** Every piece in a place from `from` on is removed: the `known`
   * ones, and any a stopped write left after them. From the far end
   * back, so that what is in a place is always its first so many with
   * none missing, which is what finding the far end relies on. */
  const clear = async (id: number, from: number, known: number) => {
    let end = Math.max(from, known);
    while ((await store.get(place(id, end))) !== undefined) end++;
    for (let n = end - 1; n >= from; n--) await store.delete(place(id, n));
  };
  const forget = async (index: Index, url: string) => {
    const entry = index.entries[url];
    if (!entry) return;
    delete index.entries[url];
    await clear(entry.id, 0, entry.pieces);
  };
  const freePlace = (index: Index) => {
    const taken = new Set(Object.values(index.entries).map((e) => e.id));
    let id = 0;
    while (taken.has(id)) id++;
    return id;
  };

  return {
    async get(url) {
      const state = await load();
      const entry = state.index.entries[url];
      if (!entry) return undefined;
      try {
        const parts: string[] = [];
        for (let n = 0; n < entry.pieces; n++) {
          const kept = (await store.get(place(entry.id, n))) as Partial<Piece> | undefined;
          // A piece of another answer, or none: what the index names
          // was never finished, or is being replaced. Nothing is kept.
          if (!kept || kept.u !== url || kept.e !== entry.etag || typeof kept.t !== "string") return undefined;
          parts.push(kept.t);
        }
        const text = parts.join("");
        if (text.length !== entry.size) return undefined;
        if (now() - entry.at > TOUCH_MS) {
          await inTurn(() =>
            write(state, async () => {
              entry.at = now();
              await store.put("gh:index", state.index);
            }),
          );
        }
        return { etag: entry.etag, text };
      } catch {
        return undefined;
      }
    },
    async set(url, answer) {
      const state = await load();
      const { index } = state;
      await inTurn(() =>
        write(state, async () => {
          if (answer.text.length > one) {
            if (!index.entries[url]) return;
            await forget(index, url);
            await store.put("gh:index", index);
            return;
          }
          const before = index.entries[url];
          const id = before?.id ?? freePlace(index);
          const pieces = Math.max(1, Math.ceil(answer.text.length / pieceSize));
          // The pieces first, each marked as this answer's; then what
          // else is in the place; the index last.
          for (let n = 0; n < pieces; n++) {
            const piece: Piece = {
              u: url,
              e: answer.etag,
              t: answer.text.slice(n * pieceSize, (n + 1) * pieceSize),
            };
            await store.put(place(id, n), piece);
          }
          await clear(id, pieces, before?.pieces ?? 0);
          index.entries[url] = { id, etag: answer.etag, pieces, size: answer.text.length, at: now() };
          // Room is made by letting the oldest go, never the one just kept.
          const held = () => Object.values(index.entries).reduce((sum, e) => sum + e.size, 0);
          while (Object.keys(index.entries).length > most || held() > room) {
            const oldest = Object.entries(index.entries)
              .filter(([other]) => other !== url)
              .sort(([, a], [, b]) => a.at - b.at || a.id - b.id)[0];
            if (!oldest) break;
            await forget(index, oldest[0]);
          }
          await store.put("gh:index", index);
        }),
      );
    },
    async delete(url) {
      const state = await load();
      const { index } = state;
      await inTurn(() =>
        write(state, async () => {
          if (!index.entries[url]) return;
          await forget(index, url);
          await store.put("gh:index", index);
        }),
      );
    },
    async quiet() {
      await load();
      return { ...waits };
    },
    async hush(spent, quiet) {
      const state = await load();
      let changed = false;
      for (const allowance of spent) {
        if ((waits[allowance]?.until ?? 0) < quiet.until) {
          waits[allowance] = { until: quiet.until, why: quiet.why };
          changed = true;
        }
      }
      if (changed) await write(state, () => store.put("gh:quiet", waits));
    },
    async free() {
      const state = await load();
      if (Object.keys(waits).length === 0) return;
      for (const allowance of ALLOWANCES) delete waits[allowance];
      await write(state, () => store.put("gh:quiet", waits));
    },
    async count(unchanged) {
      const state = await load();
      state.count.answers++;
      if (unchanged) state.count.unchanged++;
      await write(state, () => store.put("gh:count", state.count));
    },
    async said() {
      const { index, count } = await load();
      return { kept: Object.keys(index.entries).length, ...count };
    },
  };
}

/** When GitHub says a caller it turned away for calling too often may
 * ask again, in milliseconds; undefined where it does not say. It says
 * it one of two ways. `retry-after`, a number of seconds from now: the
 * caller is going too fast, whatever it asks for (`whole`). Or, when
 * `x-ratelimit-remaining` is 0, `x-ratelimit-reset`: the second the
 * allowance the call drew on comes back, and only that one is spent.
 * Never further off than QUIET_MOST_MS, and never in the past. */
export function askAgainAt(
  headers: { get(name: string): string | null } | undefined,
  now: number,
): { until: number; whole: boolean } | undefined {
  const within = (at: number) => Math.min(Math.max(at, now), now + QUIET_MOST_MS);
  const after = headers?.get("retry-after") ?? null;
  if (after !== null && /^\d{1,9}$/.test(after.trim())) {
    return { until: within(now + Number(after.trim()) * 1000), whole: true };
  }
  if (headers?.get("x-ratelimit-remaining")?.trim() === "0") {
    const reset = headers.get("x-ratelimit-reset") ?? "";
    if (/^\d{1,12}$/.test(reset.trim())) return { until: within(Number(reset.trim()) * 1000), whole: false };
  }
  return undefined;
}

/** Which allowance a call draws on, by what follows GitHub's address. */
export const drawsOn = (path: string): Allowance => (/^\/?search\//.test(path) ? "search" : "core");

/** Said at the front of why a call failed, when GitHub said when to
 * ask again: the app reads the time back out of it (whySaid). */
export const askAgainSaid = (until: number) => `GitHub says to ask again at ${new Date(until).toISOString()}: `;

/** One `fleet gh api` call as a GitHub REST call. `argv` is what follows
 * `api`: an optional `-X METHOD`, the path, `-f key=value` pairs, and
 * `--input -` for a JSON body on stdin. A failed call answers the way
 * the fleet binary would: a non-zero code with the body's own words on
 * stderr, so callers that look for them (the prompt label's
 * `already_exists`) still find them. */
async function ghApi(
  argv: string[],
  stdin: string | undefined,
  token: string | undefined,
  fetchImpl: Fetch,
  origin: string,
  seen?: (call: Outward) => void,
  memory?: GitHubMemory,
  now: () => number = Date.now,
): Promise<RunResult> {
  if (!token) throw new OpError(502, "the edge has no GitHub token");
  let method = "GET";
  let path = "";
  const fields: [string, string][] = [];
  let useStdin = false;
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "-X") method = argv[++i] ?? "GET";
    else if (arg === "-f") {
      const pair = argv[++i] ?? "";
      const at = pair.indexOf("=");
      if (at < 0) throw new OpError(502, `fleet gh api: -f needs key=value, got ${pair}`);
      fields.push([pair.slice(0, at), pair.slice(at + 1)]);
    } else if (arg === "--input") {
      if (argv[++i] !== "-") throw new OpError(502, "fleet gh api: --input takes -");
      useStdin = true;
    } else if (path === "") path = arg;
    else throw new OpError(502, `fleet gh api: unexpected argument ${arg}`);
  }
  if (path === "") throw new OpError(502, "fleet gh api: no path");
  let url = `${origin}/${path}`;
  let body: string | undefined;
  if (method === "GET") {
    if (fields.length > 0) {
      const query = new URLSearchParams(fields);
      url += (url.includes("?") ? "&" : "?") + query.toString();
    }
  } else if (useStdin) {
    body = stdin;
  } else if (fields.length > 0) {
    body = JSON.stringify(Object.fromEntries(fields));
  }
  const write = method !== "GET";
  // GitHub said not to ask before a time that has not come: it is not
  // asked. Asking on while turned away is what it warns callers off,
  // and each such call would be one more of the same answer.
  const draws = drawsOn(path);
  const hush = (await memory?.quiet())?.[draws];
  if (hush && hush.until > now()) {
    seen?.({ to: "github", write, status: 429, limited: true, until: hush.until, unsent: true });
    return {
      code: 1,
      stdout: "",
      stderr: `${askAgainSaid(hush.until)}not asked again since GitHub answered: ${hush.why}`,
    };
  }
  // A list asked for before is asked for by its tag.
  // An address with a time in it ("everything changed since…") is asked
  // once and never again in those words: keeping its answer would only
  // push out the answers that are asked for again.
  const moving = /[?&]since=/.test(url);
  const had = write || moving ? undefined : await memory?.get(url);
  let got: { res: Awaited<ReturnType<Fetch>>; text: string; whole: boolean };
  try {
    // The answer and its body, within the time; and no timer left
    // running after (see `timed`).
    got = await timed(GITHUB_ASK_MS, async (signal) => {
      const res = await fetchImpl(url, {
        method,
        headers: {
          accept: "application/vnd.github+json",
          authorization: `Bearer ${token}`,
          // The Workers runtime supplies no User-Agent of its own, and
          // GitHub requires one: without it every direct call is refused.
          "user-agent": "pm-console-edge",
          "x-github-api-version": "2022-11-28",
          ...(body === undefined ? {} : { "content-type": "application/json" }),
          ...(had ? { "if-none-match": had.etag } : {}),
        },
        ...(body === undefined ? {} : { body }),
        signal,
      });
      // "Not modified" has no body, and none is read.
      if (res.status === 304 && had) return { res, text: "", whole: true };
      // The body is read as JSON and written back out, so an empty
      // answer (a 204) stays empty and an error body keeps its own
      // words for the caller.
      try {
        return { res, text: JSON.stringify(await res.json()), whole: true };
      } catch {
        // No body (a 204), or one that could not be read to its end.
        return { res, text: "", whole: false };
      }
    });
  } catch (error) {
    // No answer at all (out of reach, or too slow) is a failed call like
    // any other, with why: the caller is told, as when the binary fails.
    seen?.({ to: "github", write, status: 0 });
    return { code: 1, stdout: "", stderr: `GitHub could not be reached: ${(error as Error).message}` };
  }
  const { res, text, whole } = got;
  // Not modified: the answer kept from before still stands, and this
  // call cost nothing of the hour's allowance.
  if (res.status === 304 && had) {
    await memory?.count(true);
    seen?.({ to: "github", write, status: 304 });
    return { code: 0, stdout: had.text, stderr: "" };
  }
  // GitHub turns a caller away for calling too often with the same 403
  // it uses for a token that may not do the thing, and says which in
  // the body; a 429 is always the former.
  const limited = res.status === 429 || (res.status === 403 && /rate limit/i.test(text));
  const again = limited ? askAgainAt(res.headers, now()) : undefined;
  const until = again?.until;
  if (again && memory) {
    // Going too fast is said of the caller, whatever it asks for; an
    // allowance used up is said of the one the call drew on, which
    // GitHub names (and which is this call's own, where it does not).
    const named = res.headers?.get("x-ratelimit-resource")?.trim();
    const spent: readonly Allowance[] = again.whole
      ? ALLOWANCES
      : [ALLOWANCES.find((allowance) => allowance === named) ?? draws];
    const why = (text === "" ? `GitHub answered http ${res.status}` : text).slice(0, 300);
    await memory.hush(spent, { until: again.until, why });
  }
  await memory?.count(false);
  seen?.({
    to: "github",
    write,
    status: res.status,
    ...(limited ? { limited } : {}),
    ...(until === undefined ? {} : { until }),
  });
  if (!res.ok) {
    // What was kept under this address is no longer known to be what
    // is there, if what is there is gone.
    if (!write && (res.status === 404 || res.status === 410)) await memory?.delete(url);
    const why = text === "" ? `GitHub answered http ${res.status}` : text;
    return { code: 1, stdout: "", stderr: until === undefined ? why : `${askAgainSaid(until)}${why}` };
  }
  if (!write && memory) {
    // Kept only if it arrived whole. A body cut off part-way is an
    // empty answer this once; kept under GitHub's tag it would be the
    // answer every time GitHub said "not modified" after.
    const etag = res.headers?.get("etag") ?? null;
    if (moving) {
      // Not kept (see above).
    } else if (etag && whole) await memory.set(url, { etag, text });
    else await memory.delete(url);
  }
  return { code: 0, stdout: text, stderr: "" };
}

/** Where the bridge is asked, over its service binding: the binding
 * answers for the bridge Worker whatever host the address names, as
 * with the app's own files (worker.ts asks https://assets.internal). */
const BRIDGE_BASE = "https://bridge.internal";
/** How long the bridge has to answer a call, body and all. */
const BRIDGE_ASK_MS = 20_000;
/** The name the PM stack's bridge credential is bound to (#14373):
 * the bridge fixes a credential's sends to its own name, and this
 * side's sends go out under it, as service:pm-stack. */
export const BRIDGE_AGENT = "pm-stack";

/** One `fleet bridge` call as a call to the bridge Worker over its
 * service binding, presenting the PM credential in the path (the
 * bridge's own way: /fleet/<credential>/...). The verbs the console's
 * classes use — send, agents, log, the unified views pipeline,
 * directory and credentials (#14374), and the composer's offer and
 * offers (#14375; a send's --limit and --broaden ride the send verb)
 * — and no others: anything else is refused as a machine's, like
 * every verb this side does not
 * carry. `revoke` is deliberately not among them: the PM credential
 * holds the read scope, and revoking is the owner's act through the
 * owner token on a machine — the action is held for one instead
 * (src/edge-proto.ts). The provisioning verbs stand with it:
 * `enroll-tokens` is read here (state only, #14376), while
 * mint-enroll, mint-credential and revoke-enroll are never translated
 * — a mint's value must reach the owner's screen from the machine
 * that minted it, and a revoke is held for a machine.
 * A failed call answers the way the fleet binary would: a non-zero
 * code with the reason on stderr. The bridge's plain 404 for a wrong
 * token is said plainly here — the credential unknown or revoked —
 * because that is the console-side error the owner acts on (mint a
 * new one; the laptop's own dispatch is untouched by it). */
async function bridgeCall(
  argv: string[],
  opts: { stdin?: string } | undefined,
  secrets: DirectSecrets,
): Promise<RunResult> {
  const token = secrets.bridgeToken;
  const fetchImpl = secrets.bridgeFetch;
  if (!token || !fetchImpl) throw new OpError(502, "needs a machine");
  const [what, ...rest] = argv;
  let method = "GET";
  let path = "";
  let body: string | undefined;
  if (what === "agents") {
    if (rest.length > 0) throw new OpError(502, "fleet bridge agents takes no arguments here");
    path = `/fleet/${token}/agents`;
  } else if (what === "log") {
    const query = new URLSearchParams();
    const names: Record<string, string> = {
      "--agent": "agent",
      "--direction": "direction",
      "--kind": "kind",
      "--since-id": "since_id",
      "--before-id": "before_id",
      "--limit": "limit",
      "--order": "order",
    };
    for (let i = 0; i < rest.length; i += 2) {
      const name = names[rest[i]];
      if (!name || rest[i + 1] === undefined)
        throw new OpError(502, `fleet bridge log: unexpected argument ${rest[i]}`);
      query.set(name, rest[i + 1]);
    }
    const qs = query.toString();
    path = `/fleet/${token}/messages${qs === "" ? "" : `?${qs}`}`;
  } else if (what === "pipeline") {
    // The unified owner feed (#14374): the same flag pairs as `log`,
    // plus the pipeline's own filters and its --unread switch.
    const query = new URLSearchParams();
    const names: Record<string, string> = {
      "--kind": "kind",
      "--sender": "sender",
      "--receiver": "receiver",
      "--surface": "surface",
      "--batch": "batch",
      "--work-state": "work_state",
      "--since-id": "since_id",
      "--before-id": "before_id",
      "--limit": "limit",
      "--order": "order",
    };
    for (let i = 0; i < rest.length; i++) {
      if (rest[i] === "--unread") {
        query.set("unread_only", "true");
        continue;
      }
      const name = names[rest[i]];
      if (!name || rest[i + 1] === undefined)
        throw new OpError(502, `fleet bridge pipeline: unexpected argument ${rest[i]}`);
      query.set(name, rest[++i]);
    }
    const qs = query.toString();
    path = `/fleet/${token}/pipeline${qs === "" ? "" : `?${qs}`}`;
  } else if (what === "directory") {
    if (rest.length > 0) throw new OpError(502, "fleet bridge directory takes no arguments here");
    path = `/fleet/${token}/directory`;
  } else if (what === "credentials") {
    if (rest.length > 0) throw new OpError(502, "fleet bridge credentials takes no arguments here");
    path = `/fleet/${token}/credentials`;
  } else if (what === "enroll-tokens") {
    if (rest.length > 0) throw new OpError(502, "fleet bridge enroll-tokens takes no arguments here");
    path = `/fleet/${token}/enroll-tokens`;
  } else if (what === "wake-hooks") {
    // The wake hooks' list is a read of the same binding (#14544),
    // through the bridge's read-scope view. Setting and removing are
    // owner-token acts, never translated: a set asked at the edge is
    // refused (its token must not be held, edge-proto.ts), a remove
    // is held for a machine.
    if (rest.length > 0) throw new OpError(502, "needs a machine");
    path = `/fleet/${token}/wake-hooks`;
  } else if (what === "send") {
    const to: string[] = [];
    let kind = "message";
    let useStdin = false;
    let limit: number | undefined;
    let broaden: string | undefined;
    for (let i = 0; i < rest.length; i++) {
      const arg = rest[i];
      if (arg === "--to") {
        const list = (rest[++i] ?? "")
          .split(",")
          .map((t) => t.trim())
          .filter(Boolean);
        to.push(...list);
      } else if (arg === "--command") kind = "command";
      else if (arg === "--kind") kind = rest[++i] ?? "message";
      else if (arg === "--limit") {
        const n = Number(rest[++i]);
        if (!Number.isInteger(n) || n < 1) throw new OpError(502, "fleet bridge send: --limit must be a positive whole number");
        limit = n;
      } else if (arg === "--broaden") broaden = rest[++i] ?? "";
      else if (arg === "-") useStdin = true;
      else throw new OpError(502, `fleet bridge send: unexpected argument ${arg}`);
    }
    method = "POST";
    path = `/fleet/${token}/send`;
    if (broaden !== undefined) {
      // A broaden carries no text and no targets (#14375): the bridge
      // re-delivers the original send's body to the matched agents
      // its limit left over, as this credential — the distribution's
      // own sender, so the store allows it.
      if (broaden === "") throw new OpError(502, "fleet bridge send: --broaden needs a batch");
      body = JSON.stringify({ agent: BRIDGE_AGENT, broaden, ...(limit === undefined ? {} : { limit }) });
    } else {
      if (to.length === 0) throw new OpError(502, "fleet bridge send: needs --to");
      if (!useStdin) throw new OpError(502, "fleet bridge send: the text comes on stdin here");
      body = JSON.stringify({ agent: BRIDGE_AGENT, to, body: opts?.stdin ?? "", kind, ...(limit === undefined ? {} : { limit }) });
    }
  } else if (what === "offer") {
    // A claim offer posted as this credential (#14375): the bridge
    // fixes the sender to the credential's own address.
    const to: string[] = [];
    let kind = "message";
    let useStdin = false;
    let slots: number | undefined;
    let ttl: number | undefined;
    for (let i = 0; i < rest.length; i++) {
      const arg = rest[i];
      if (arg === "--to") {
        const list = (rest[++i] ?? "").split(",").map((t) => t.trim()).filter(Boolean);
        to.push(...list);
      } else if (arg === "--command") kind = "command";
      else if (arg === "--kind") kind = rest[++i] ?? "message";
      else if (arg === "--slots") {
        const n = Number(rest[++i]);
        if (!Number.isInteger(n) || n < 1) throw new OpError(502, "fleet bridge offer: --slots must be a positive whole number");
        slots = n;
      } else if (arg === "--ttl") {
        const n = Number(rest[++i]);
        if (!Number.isInteger(n) || n < 1) throw new OpError(502, "fleet bridge offer: --ttl must be a positive whole number");
        ttl = n;
      } else if (arg === "-") useStdin = true;
      else throw new OpError(502, `fleet bridge offer: unexpected argument ${arg}`);
    }
    if (to.length === 0) throw new OpError(502, "fleet bridge offer: needs --to");
    if (slots === undefined) throw new OpError(502, "fleet bridge offer: needs --slots");
    if (!useStdin) throw new OpError(502, "fleet bridge offer: the text comes on stdin here");
    method = "POST";
    path = `/fleet/${token}/offer`;
    body = JSON.stringify({
      agent: BRIDGE_AGENT,
      to,
      body: opts?.stdin ?? "",
      kind,
      slots,
      ...(ttl === undefined ? {} : { ttl_seconds: ttl }),
    });
  } else if (what === "offers") {
    if (rest.length > 0) throw new OpError(502, "fleet bridge offers takes no arguments here");
    path = `/fleet/${token}/offers`;
  } else {
    throw new OpError(502, "needs a machine");
  }
  const write = method !== "GET";
  let res: Awaited<ReturnType<Fetch>>;
  let text = "";
  try {
    await timed(BRIDGE_ASK_MS, async (signal) => {
      res = await fetchImpl(`${BRIDGE_BASE}${path}`, {
        method,
        headers: {
          "user-agent": "pm-console-edge",
          ...(body === undefined ? {} : { "content-type": "application/json" }),
        },
        ...(body === undefined ? {} : { body }),
        signal,
      });
      // The answer is read as JSON and written back out, like GitHub's:
      // the classes parse what the fleet binary would have printed.
      try {
        text = JSON.stringify(await res.json());
      } catch {
        text = "";
      }
    });
  } catch (error) {
    secrets.seen?.({ to: "bridge", write, status: 0 });
    return { code: 1, stdout: "", stderr: `the bridge could not be reached: ${(error as Error).message}` };
  }
  secrets.seen?.({ to: "bridge", write, status: res!.status });
  if (!res!.ok) {
    if (res!.status === 404) {
      return {
        code: 1,
        stdout: "",
        stderr: "the bridge turned the PM credential away (unknown or revoked); nothing was sent",
      };
    }
    const why = text === "" || text === "null" ? `the bridge answered http ${res!.status}` : text;
    return { code: 1, stdout: "", stderr: why };
  }
  return { code: 0, stdout: text, stderr: "" };
}

/** The edge's `run`: GitHub through REST, the queue token from its
 * Worker secret, the bridge through its binding with the PM
 * credential, and everything else refused as a machine's. */
export function edgeRun(config: Config, secrets: DirectSecrets, fetchImpl: Fetch): Run {
  const origin = (secrets.githubApi || GITHUB_API).replace(/\/+$/, "");
  return async (argv, opts) => {
    const [bin, verb, ...rest] = argv;
    if (bin === config.fleetBin && verb === "gh" && rest[0] === "api") {
      return ghApi(
        rest.slice(1),
        opts?.stdin,
        secrets.githubToken,
        fetchImpl,
        origin,
        secrets.seen,
        secrets.github,
        secrets.now,
      );
    }
    if (bin === config.fleetBin && verb === "secrets" && rest[0] === "get") {
      const name = rest[1];
      if (name === config.queueTokenName && secrets.queueToken) {
        return { code: 0, stdout: `${secrets.queueToken}\n`, stderr: "" };
      }
      throw new OpError(502, `the edge has no secret named ${name ?? "?"}`);
    }
    if (bin === config.fleetBin && verb === "bridge") {
      return bridgeCall(rest, opts, secrets);
    }
    throw new OpError(502, "needs a machine");
  };
}

/** The laptop console's config, as the edge knows it: what the host
 * reported, and nothing the edge cannot use left blank. The dashboard
 * is null, so anything that reads it refuses rather than reaching
 * somewhere it cannot. */
export function directConfig(edge: EdgeConfig): Config {
  return {
    port: 0,
    bind: "127.0.0.1",
    owners: edge.owners,
    certNames: edge.certNames,
    issuers: edge.issuers,
    repo: edge.repo,
    queue: edge.queue,
    queueDbUrl: edge.queueDbUrl,
    queueTokenName: edge.queueTokenName,
    fleetBin: "fleet",
    unitPatterns: [],
    protectedUnits: [],
    auditFile: "",
    dashboardUrl: null,
    androidPackage: edge.androidPackage,
    androidFingerprints: edge.androidFingerprints,
    androidApk: "",
    devicesFile: "",
    edgeUrl: null,
    edgeKeyFile: "",
    edgeDoneFile: "",
    machineName: "",
    decisionsFile: "",
    promptLabel: edge.promptLabel,
    needsRepos: edge.needsRepos,
  };
}

export interface DirectOptions extends DirectSecrets {
  fetchImpl?: Fetch;
}

export interface DirectClasses {
  ops: Ops;
  work: Work;
  plan: Plan;
  trends: Trends;
}

/** The console's own classes, wired for the edge. They are made once
 * and kept: what they remember between reads (which prompts' replies
 * have not changed, what was read in the last minute) is what keeps the
 * edge from asking GitHub for the same thing again and again. */
export function directClasses(edge: EdgeConfig, options: DirectOptions): DirectClasses {
  const config = directConfig(edge);
  if (options.queueDb) config.queueDbUrl = options.queueDb.replace(/\/+$/, "");
  const fetchImpl = options.fetchImpl ?? (fetch as unknown as Fetch);
  // The queue store is reached by Ops itself, with the fetch it is
  // given: that one tells of each call too.
  const queueFetch: Fetch = async (url, init) => {
    const write = queueWrites(init.body);
    try {
      const res = await fetchImpl(url, init);
      options.seen?.({ to: "queue", write, status: res.status, ...(res.status === 429 ? { limited: true } : {}) });
      return res;
    } catch (error) {
      options.seen?.({ to: "queue", write, status: 0 });
      throw error;
    }
  };
  const ops = new Ops(config, edgeRun(config, options, fetchImpl), options.seen ? queueFetch : fetchImpl);
  const clock = options.now ? () => new Date((options.now as () => number)()) : undefined;
  const work = new Work(ops, undefined, clock);
  return {
    ops,
    work,
    plan: new Plan(ops, work, clock),
    trends: new Trends(ops, options.shelf ?? memoryShelf(), clock, "leave"),
  };
}

/** Whether a request of the queue store may change something: any
 * statement in it that is not a SELECT. One that cannot be read is
 * taken to. */
export function queueWrites(body: string | undefined): boolean {
  try {
    const requests = (JSON.parse(body ?? "") as { requests?: { stmt?: { sql?: unknown } }[] }).requests;
    if (!Array.isArray(requests)) return true;
    return requests.some((r) => r?.stmt !== undefined && !/^\s*select\b/i.test(String(r.stmt.sql ?? "")));
  } catch {
    return true;
  }
}

/** One section, read by the class that reads it on the laptop. */
export async function directRead(section: string, ops: Ops, work: Work): Promise<unknown> {
  switch (section) {
    case "board":
      return work.board();
    case "prs":
      return ops.prs();
    case "priorities":
      return ops.priorities();
    case "asks":
      return work.asks();
    case "census":
      return work.censusDirect();
    case "done":
      return work.done();
    case "agents":
      return ops.agents();
    case "pipeline":
      return ops.unreadInstructions();
    default:
      throw new Error(`not a section the edge reads itself: ${section}`);
  }
}
