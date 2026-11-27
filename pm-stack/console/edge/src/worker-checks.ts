// The edge's checks of the platform's Workers (#15793, steps 1–2 of
// #13494).
//
// Whether a Worker is in service used to be written by hand in
// pm-stack/platform/workers.json, from asking each address from
// outside on a given day. The edge could not ask itself: a Worker
// cannot fetch another Worker on the same account by its public
// address (it is answered with error 1042) without a setting that
// also changes how the console reaches the laptop. Service bindings
// are the way around that — no public hop, no setting changed — so
// the edge holds a binding to each Worker it checks, and on a
// schedule (the cron trigger in wrangler.jsonc) the stateless part of
// this Worker asks each bound one, by the binding, for the commit it
// was deployed from: `GET /version`, or its health answer where that
// is what names the commit (the packet's "or includes that commit in
// its existing health answer").
//
// The answer — answering or not, the commit it names, when it was
// asked — is handed to the object (POST /internal/worker-checks),
// which keeps the last set under WORKER_CHECKS_KEY and shows it in
// its health answer. The Platform screen drawing it, and a Worker
// that stops answering raising a notification, are step 3 of #13494,
// a separate slice.
//
// This module is the probe and the stored shape only: no runtime in
// it, so the console tests exercise it with stand-in fetchers.

/** One Worker, as one asking found it. */
export interface WorkerCheck {
  /** The Worker's name, as in pm-stack/platform/workers.json. */
  name: string;
  /** It answered at all: any HTTP answer counts; a binding whose
   * fetch throws (no such Worker, or it is down) does not. */
  answering: boolean;
  /** The commit it named, or null when its answer names none. */
  commit: string | null;
  /** When it was asked, in milliseconds since the epoch. */
  askedAt: number;
  /** The path that answered: "/version", "/health", "self" for the
   * edge's own record (its commit is its VERSION var, asked of no
   * one), or null when nothing answered. */
  via: string | null;
  /** The HTTP status of the answer that settled it, or null. */
  status: number | null;
}

/** One round of asking: every check of the round, and when it ran. */
export interface WorkerCheckSet {
  askedAt: number;
  checks: WorkerCheck[];
}

/** The storage key the object keeps the last set under. */
export const WORKER_CHECKS_KEY = "worker-checks";

/** A service binding's fetch, shaped as the probe takes it: not to
 * be called as a method, like the platform's own. */
export type WorkerFetcher = (request: Request) => Promise<Response>;

/** One Worker to ask: its name and the binding that reaches it. */
export interface WorkerCheckTarget {
  name: string;
  fetcher: WorkerFetcher;
}

/** The commit an answer names: a `commit` field first, then
 * `version` (the field the Workers that stamp a VERSION var answer
 * with). Anything else — no body, not JSON, no such field — names
 * none, and the check records that honestly rather than guessing. */
function commitOf(body: unknown): string | null {
  if (typeof body !== "object" || body === null) return null;
  const record = body as Record<string, unknown>;
  for (const key of ["commit", "version"]) {
    const value = record[key];
    if (typeof value === "string" && value.length > 0) return value;
  }
  return null;
}

/** Ask one Worker. `/version` first; a 404 there means it has no
 * version route, so its health answer is asked instead. A fetch that
 * throws settles it at once: not answering. */
export async function probeWorker(target: WorkerCheckTarget, askedAt: number): Promise<WorkerCheck> {
  const base = { name: target.name, askedAt };
  for (const path of ["/version", "/health"]) {
    let res: Response;
    try {
      res = await target.fetcher(new Request(`https://worker-checks.internal${path}`));
    } catch {
      return { ...base, answering: false, commit: null, via: null, status: null };
    }
    if (path === "/version" && res.status === 404) continue;
    let commit: string | null = null;
    try {
      commit = commitOf(await res.json());
    } catch {
      // Not JSON: it answered, and names no commit.
    }
    return { ...base, answering: true, commit, via: path, status: res.status };
  }
  // Unreachable: the /health pass always returns. Present only so
  // the function's end is explicit.
  return { ...base, answering: false, commit: null, via: null, status: null };
}

/** Ask every target, in parallel; the set is sorted by name so two
 * rounds read alike. */
export async function probeWorkers(targets: WorkerCheckTarget[], askedAt: number): Promise<WorkerCheckSet> {
  const checks = await Promise.all(targets.map((target) => probeWorker(target, askedAt)));
  checks.sort((a, b) => a.name.localeCompare(b.name));
  return { askedAt, checks };
}

/** The edge's own record: it asks itself nothing — its commit is the
 * VERSION var its deploy stamps — but it belongs in the set, so the
 * store names every Worker the edge checks, itself included. */
export function selfCheck(name: string, version: string | undefined, askedAt: number): WorkerCheck {
  return {
    name,
    answering: true,
    commit: version && version.length > 0 ? version : null,
    askedAt,
    via: "self",
    status: null,
  };
}

/** Read a stored set back, or null when what is stored is not one:
 * the store is written by this code, but an object that outlives a
 * change of shape must not hand the health answer a shape it cannot
 * show. */
export function parseWorkerCheckSet(raw: unknown): WorkerCheckSet | null {
  if (typeof raw !== "object" || raw === null) return null;
  const record = raw as Record<string, unknown>;
  if (typeof record.askedAt !== "number" || !Array.isArray(record.checks)) return null;
  const checks: WorkerCheck[] = [];
  for (const item of record.checks) {
    if (typeof item !== "object" || item === null) return null;
    const check = item as Record<string, unknown>;
    if (typeof check.name !== "string" || typeof check.answering !== "boolean") return null;
    if (check.commit !== null && typeof check.commit !== "string") return null;
    if (typeof check.askedAt !== "number") return null;
    if (check.via !== null && typeof check.via !== "string") return null;
    if (check.status !== null && typeof check.status !== "number") return null;
    checks.push({
      name: check.name,
      answering: check.answering,
      commit: check.commit as string | null,
      askedAt: check.askedAt,
      via: check.via as string | null,
      status: check.status as number | null,
    });
  }
  return { askedAt: record.askedAt, checks };
}
