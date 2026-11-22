// What the edge can do itself while the laptop is away: the laptop
// console's own classes, with stand-ins only where they touch the
// outside world.
//
// Ops and Work take a `run` function and a `fetch`. At the edge, `run`
// turns `fleet gh api` argv into GitHub REST calls and
// `fleet secrets get <queue token name>` into the Worker secret, and
// refuses every other fleet verb as needing the laptop; `fetch` is the
// Worker's own, which already reaches the queue store. So board, pull
// requests, ranked issues and prompts are read, and the one-call writes
// are run, by the same code the laptop runs — nothing here re-implements
// their shapes.
//
// What stays the laptop's: `fleet queue enqueue` (the fleet binary is
// not here), `fleet bridge`, `systemctl`, and anything that reads the
// dashboard or the decisions file, which live on the laptop alone.
// A new prompt is filed at the edge and only its queue task is held for
// the laptop, as `prompt.enqueue`: the issue is never filed twice.
import { OpError } from "../../src/errors.ts";
import type { Config } from "../../src/config.ts";
import type { EdgeConfig } from "../../src/edge-proto.ts";
import { Ops, type Fetch, type Run, type RunResult } from "../../src/ops.ts";
import { Work } from "../../src/work.ts";

/** Sections the edge reads itself once the laptop has gone quiet. */
export const DIRECT_READ_SECTIONS = ["board", "prs", "priorities", "asks"] as const;
/** Actions the edge runs itself once the laptop has gone quiet. */
export const DIRECT_ACTIONS = [
  "ask.send",
  "ask.reply",
  "ask.close",
  "issue.comment",
  "need.comment",
  "issue.rank",
  "task.move",
] as const;

export interface DirectSecrets {
  githubToken?: string;
  queueToken?: string;
}

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
  let url = `https://api.github.com/${path}`;
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
  const res = await fetchImpl(url, {
    method,
    headers: {
      accept: "application/vnd.github+json",
      authorization: `Bearer ${token}`,
      "x-github-api-version": "2022-11-28",
      ...(body === undefined ? {} : { "content-type": "application/json" }),
    },
    ...(body === undefined ? {} : { body }),
    signal: AbortSignal.timeout(20_000),
  });
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
  if (!res.ok) {
    return { code: 1, stdout: "", stderr: text === "" ? `GitHub answered http ${res.status}` : text };
  }
  return { code: 0, stdout: text, stderr: "" };
}

/** The edge's `run`: GitHub through REST, the queue token from its
 * Worker secret, and everything else refused as the laptop's. */
export function edgeRun(config: Config, secrets: DirectSecrets, fetchImpl: Fetch): Run {
  return async (argv, opts) => {
    const [bin, verb, ...rest] = argv;
    if (bin === config.fleetBin && verb === "gh" && rest[0] === "api") {
      return ghApi(rest.slice(1), opts?.stdin, secrets.githubToken, fetchImpl);
    }
    if (bin === config.fleetBin && verb === "secrets" && rest[0] === "get") {
      const name = rest[1];
      if (name === config.queueTokenName && secrets.queueToken) {
        return { code: 0, stdout: `${secrets.queueToken}\n`, stderr: "" };
      }
      throw new OpError(502, `the edge has no secret named ${name ?? "?"}`);
    }
    throw new OpError(502, "needs the laptop");
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

export interface DirectOptions {
  githubToken?: string;
  queueToken?: string;
  fetchImpl?: Fetch;
}

/** The laptop's own classes, wired for the edge. */
export function directClasses(edge: EdgeConfig, options: DirectOptions): { ops: Ops; work: Work } {
  const config = directConfig(edge);
  const fetchImpl = options.fetchImpl ?? (fetch as unknown as Fetch);
  const ops = new Ops(config, edgeRun(config, options, fetchImpl), fetchImpl);
  return { ops, work: new Work(ops) };
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
