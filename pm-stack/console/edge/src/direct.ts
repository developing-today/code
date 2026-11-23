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
// What stays a machine's: `fleet queue enqueue` (the fleet binary is
// not here), `fleet bridge`, `systemctl`, and anything that reads the
// dashboard or the decisions file, which live on a machine alone.
// A prompt, a note or a bug is filed at the edge and only its queue
// task is held for a machine, as `prompt.enqueue` or `triage.enqueue`:
// the issue is never filed twice.
import { OpError } from "../../src/errors.ts";
import type { Config } from "../../src/config.ts";
import type { EdgeConfig } from "../../src/edge-proto.ts";
import { Ops, type Fetch, type Run, type RunResult } from "../../src/ops.ts";
import { Plan } from "../../src/plan.ts";
import { Work } from "../../src/work.ts";

/** Sections the edge reads itself. */
export const DIRECT_READ_SECTIONS = ["board", "prs", "priorities", "asks"] as const;
/** Actions the edge runs itself: each is one or a few calls to GitHub
 * or the queue store, and needs nothing a machine has. */
export const DIRECT_ACTIONS = [
  "ask.send",
  "ask.reply",
  "ask.close",
  "issue.comment",
  "need.comment",
  "issue.rank",
  "task.move",
  "pr.merge",
  "deploy.start",
  "intake.send",
] as const;

/** One call this side made of GitHub or the queue store, and how it was
 * answered: `status` 0 is no answer at all. `write` is anything that
 * may have changed something. `limited` is a call turned away for
 * being one too many just now, not for what the token may do. */
export interface Outward {
  to: "github" | "queue";
  write: boolean;
  status: number;
  limited?: boolean;
}

/** Which of the two each section this side reads is read from. */
export const READ_FROM: Record<(typeof DIRECT_READ_SECTIONS)[number], Outward["to"][]> = {
  board: ["queue"],
  prs: ["github"],
  priorities: ["github"],
  asks: ["github", "queue"],
};

export interface DirectSecrets {
  githubToken?: string;
  queueToken?: string;
  /** Told of every call made, as it is answered. */
  seen?: (call: Outward) => void;
  /** Where GitHub's API and the queue store are, when a local run puts
   * stand-ins in their place. Deployed, neither is given: GitHub's own
   * address is used, and the queue store's is the one a machine reports. */
  githubApi?: string;
  queueDb?: string;
}

const GITHUB_API = "https://api.github.com";

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
  let res: Awaited<ReturnType<Fetch>>;
  try {
    res = await fetchImpl(url, {
      method,
      headers: {
        accept: "application/vnd.github+json",
        authorization: `Bearer ${token}`,
        // The Workers runtime supplies no User-Agent of its own, and
        // GitHub requires one: without it every direct call is refused.
        "user-agent": "pm-console-edge",
        "x-github-api-version": "2022-11-28",
        ...(body === undefined ? {} : { "content-type": "application/json" }),
      },
      ...(body === undefined ? {} : { body }),
      signal: AbortSignal.timeout(20_000),
    });
  } catch (error) {
    // No answer at all (out of reach, or too slow) is a failed call like
    // any other, with why: the caller is told, as when the binary fails.
    seen?.({ to: "github", write, status: 0 });
    return { code: 1, stdout: "", stderr: `GitHub could not be reached: ${(error as Error).message}` };
  }
  // The body is read as JSON and written back out, so an empty answer
  // (a 204) stays empty and an error body keeps its own words for the
  // caller.
  const text = await (async () => {
    try {
      return JSON.stringify(await res.json());
    } catch {
      return "";
    }
  })();
  // GitHub turns a caller away for calling too often with the same 403
  // it uses for a token that may not do the thing, and says which in
  // the body; a 429 is always the former.
  const limited = res.status === 429 || (res.status === 403 && /rate limit/i.test(text));
  seen?.({ to: "github", write, status: res.status, ...(limited ? { limited } : {}) });
  if (!res.ok) {
    return { code: 1, stdout: "", stderr: text === "" ? `GitHub answered http ${res.status}` : text };
  }
  return { code: 0, stdout: text, stderr: "" };
}

/** The edge's `run`: GitHub through REST, the queue token from its
 * Worker secret, and everything else refused as a machine's. */
export function edgeRun(config: Config, secrets: DirectSecrets, fetchImpl: Fetch): Run {
  const origin = (secrets.githubApi || GITHUB_API).replace(/\/+$/, "");
  return async (argv, opts) => {
    const [bin, verb, ...rest] = argv;
    if (bin === config.fleetBin && verb === "gh" && rest[0] === "api") {
      return ghApi(rest.slice(1), opts?.stdin, secrets.githubToken, fetchImpl, origin, secrets.seen);
    }
    if (bin === config.fleetBin && verb === "secrets" && rest[0] === "get") {
      const name = rest[1];
      if (name === config.queueTokenName && secrets.queueToken) {
        return { code: 0, stdout: `${secrets.queueToken}\n`, stderr: "" };
      }
      throw new OpError(502, `the edge has no secret named ${name ?? "?"}`);
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
    decisionsFile: "",
    promptLabel: edge.promptLabel,
    needsRepos: edge.needsRepos,
  };
}

export interface DirectOptions extends DirectSecrets {
  fetchImpl?: Fetch;
  now?: () => number;
}

export interface DirectClasses {
  ops: Ops;
  work: Work;
  plan: Plan;
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
  return { ops, work, plan: new Plan(ops, work, clock) };
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
    default:
      throw new Error(`not a section the edge reads itself: ${section}`);
  }
}
