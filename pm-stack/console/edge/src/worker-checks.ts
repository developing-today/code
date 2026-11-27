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

/** A service binding as the runtime hands it over: an object with a
 * fetch, not a bare function. */
export interface WorkerBinding {
  fetch(request: Request): Promise<Response>;
}

/** One binding the edge holds for the checks: the Worker it reaches
 * and the binding's name in the edge's environment (and so in
 * wrangler.jsonc's services). */
export interface WorkerBindingSpec {
  name: string;
  binding: string;
}

/** The roster: every Worker pm-stack/platform/workers.json says is
 * in service, except the edge itself, whose record is selfCheck's.
 * The console tests hold this list to workers.json and to
 * wrangler.jsonc, so a Worker that enters service without a binding,
 * or a binding declared for no in-service Worker, fails there. */
export const WORKER_BINDINGS: WorkerBindingSpec[] = [
  { name: "message-bridge", binding: "BRIDGE" },
  { name: "pm-service", binding: "PM_SERVICE" },
  { name: "decision-events", binding: "DECISION_EVENTS" },
  { name: "pm-notify", binding: "PM_NOTIFY" },
  { name: "mcpx", binding: "MCPX" },
  { name: "queue-transport-alert", binding: "QUEUE_TRANSPORT_ALERT" },
  { name: "github-ci", binding: "GITHUB_CI" },
  { name: "voice-line", binding: "VOICE_LINE" },
  { name: "muse-agent-handoff", binding: "MUSE_AGENT_HANDOFF" },
  { name: "desktop-commander", binding: "DESKTOP_COMMANDER" },
  { name: "fleet-hosted", binding: "FLEET_HOSTED" },
  { name: "atlas-search", binding: "ATLAS_SEARCH" },
  { name: "d1-gateway", binding: "D1_GATEWAY" },
  { name: "cold-store-rehydrator", binding: "COLD_STORE_REHYDRATOR" },
  { name: "developing-today-backup", binding: "DEVELOPING_TODAY_BACKUP" },
  { name: "fleet-git-rehost", binding: "FLEET_GIT_REHOST" },
  { name: "pkarr", binding: "PKARR" },
  { name: "lab-ops-hub", binding: "LAB_OPS_HUB" },
  { name: "cf-container-hello", binding: "CF_CONTAINER_HELLO" },
];

/** The round's targets: every roster Worker whose binding the
 * environment actually has, in roster order. A binding that is not
 * configured leaves its Worker out of the round — the deployed edge
 * declares them all; a local run may not. */
export function workerCheckTargets(resolve: (binding: string) => WorkerBinding | undefined): WorkerCheckTarget[] {
  const targets: WorkerCheckTarget[] = [];
  for (const spec of WORKER_BINDINGS) {
    const binding = resolve(spec.binding);
    if (binding === undefined) continue;
    targets.push({ name: spec.name, fetcher: (request: Request) => binding.fetch(request) });
  }
  return targets;
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

/** One scheduled round, without the runtime around it: resolve the
 * configured bindings to targets, ask each, and add the edge's own
 * record. The stateless part's `scheduled` handler runs this and
 * hands the set to the object to keep; the console tests run it
 * with stand-in bindings, which is the scheduled path exercised. */
export async function runCheckRound(
  resolve: (binding: string) => WorkerBinding | undefined,
  edgeName: string,
  version: string | undefined,
  askedAt: number,
): Promise<WorkerCheckSet> {
  const set = await probeWorkers(workerCheckTargets(resolve), askedAt);
  set.checks.push(selfCheck(edgeName, version, askedAt));
  set.checks.sort((a, b) => a.name.localeCompare(b.name));
  return set;
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
