// Thin TS CLI (#12588; design §3.2) — the hosted fleet caller's
// command line: session carriage, create/query/intake/sync verbs,
// all as authenticated service calls against the fleet-hosted
// doors. This module is the entrypoint the verb modules name
// (src/query-cli.ts, src/sync-cli.ts): it wires their parsers and
// renderers to argv/stdout and to the doors, and adds the verbs
// that had no module yet (session open/resume/close, create, next,
// claim). The node wiring (real fetch, env, files) lives in the
// package-root cli.ts; everything here is injected, so the whole
// command line is testable against the real router in-process.
//
// The CLI holds NO authority (design §3.2, §4.3 layer 1): it
// resolves session context, assembles the caller's body text and
// caller-owned Metadata fields, and sends claims the service may
// verify (repo, branch, host, worktree). It never sends a
// timestamp, a queue field, or a provenance key — every request
// document is built from a fixed field allowlist per verb, and
// test/cli.test.ts pins the create request's key set. Anything
// fleet-owned a caller smuggles into a body block is the service's
// to overwrite wholesale (#12585); the CLI passes the body through
// untouched rather than laundering it.
//
// Token carriage (issue scope; design §4.1, §8 deferral 4): the
// caller presents a #9077 relying-party token it already holds —
// token acquisition flows (PKCE / device) are component H's design
// surface (#9086 Q1), deferred there by the design itself. Storage
// is local to the caller, resolved like the session id: explicit >
// environment (FLEET_HOSTED_TOKEN) > the CLI's stored token file.
//
// Session carriage (design §4.2): explicit --session-id >
// environment > per-workspace stored state, via
// src/session-resolution.ts — including its failure modes (a
// malformed id fails closed; a stored id the registry no longer
// knows is the named stale error, checked against
// POST /v1/sessions/resolve before the id is cited anywhere).
//
// Command line (operator contract, #3352 — the whole line is
// declared here and parsed strictly; anything unaccounted for is a
// usage error, exit non-zero):
//   fleet-hosted session open [--lane <lane>] [--model <model>]
//                             [--lane-start-issue <owner/repo#n>]...
//   fleet-hosted session resume [--session-id <id>]
//   fleet-hosted session close [--session-id <id>]
//   fleet-hosted create issue|pr --repo <owner/name> --title <text>
//                             [--body <text> | --body-file <path>]
//                             [--direct-parent <owner/repo#n>]
//                             [--link <relation>:<status>:<owner/repo#n>]...
//                             [--session-id <id>] [--host <host>]
//                             [--worktree <path>]
//                             [--head <branch>] [--base <branch>]   (pr only)
//   fleet-hosted query [--repo <owner/name>] [--kind issue|pr]
//                             [--direct-parent <owner/repo#n>]
//                             [--session <id>] [--lane <lane>]
//                             [--host <host>] [--queue-id <id>]
//                             [--link-relation <r>] [--link-status <s>]
//                             [--link-target <owner/repo#n>]
//   fleet-hosted next [--worker <id>] [--lane <lane>]
//                             [--queue <name>] [--count <1..5>]
//                             [--session-id <id>]
//   fleet-hosted claim [--worker <id>] [--lane <lane>]
//                             [--queue <name>] [--count <1..5>]
//                             [--task-id <n>] [--session-id <id>]
//   fleet-hosted backfill [--repo <owner/name>]...
//   fleet-hosted backfill runs
//   fleet-hosted divergence [--repo <owner/name>]...
//   fleet-hosted sync project --owner <login> --project <number>
//                             [--add-issue <owner/repo#n>
//                              [--add-status <Status>]
//                              | --set-status <owner/repo#n>=<Status>
//                              | --set-item-status <item-id>=<Status>]...
//   fleet-hosted sync flow --flow <verdict|tend|review>
//                             --repo <owner/name> --number <n>
//                             [--kind issue|pr] --marker <token>
//                             --body <text>
//                             [--add-label <name>]...
//                             [--remove-label <name>]...
// Service URL and token come from the caller's environment / stored
// state (see the package-root cli.ts), never from verb flags.

import { parseBackfillRunsArgs, parseRepoScopeArgs } from "./backfill-cli.ts";
import { FailClosed } from "./errors.ts";
import { parseMetadataBlock, renderBodyWithBlock } from "./metadata.ts";
import { parseQueryCliArgs, renderQueryResponse } from "./query-cli.ts";
import type { MetadataFilter, QueryResponse } from "./query.ts";
import {
  FLEET_SESSION_ENV,
  OPENCODE_SESSION_ENV,
  resolveAndBindSession,
  resolveSessionId,
  SessionResolutionError,
  type SessionStateStore,
} from "./session-resolution.ts";
import { isValidSessionId } from "./sessions.ts";
import type { FlowSyncResponse, ProjectSyncResponse } from "./sync.ts";
import { parseFlowSyncCliArgs, parseProjectSyncCliArgs, renderSyncResponse } from "./sync-cli.ts";

export const TOKEN_ENV = "FLEET_HOSTED_TOKEN";
export const WORKER_ENV = "FLEET_WORKER";

export interface CliDeps {
  /** Service base URL, no trailing slash. */
  serviceUrl: string;
  /** The caller's bearer token, resolved by resolveCallerToken. */
  token: string | null;
  /** Transport: global fetch in production, in-process dispatch in tests. */
  fetch: (req: Request) => Promise<Response>;
  /** Environment lookup by variable name. */
  env: (name: string) => string | null | undefined;
  /** Per-workspace session state (the hosted pwd-file analogue). */
  store: SessionStateStore;
  /** Read a caller-named file (--body-file). */
  readFile: (path: string) => string;
  /** The caller's machine host claim (design §5). */
  hostname: () => string;
  /** The caller's working directory (the §13 pwd / worktree claim). */
  cwd: () => string;
}

export interface CliOutcome {
  exitCode: number;
  stdout: string;
  stderr: string;
}

function nonempty(value: string | null | undefined): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  return trimmed === "" ? null : trimmed;
}

/** Resolve the caller's token: explicit > env > stored token file.
 * Storage is local to the caller (issue scope); acquisition is
 * component H's flow (#9086), not this module's. */
export function resolveCallerToken(sources: {
  explicit?: string | null;
  env: (name: string) => string | null | undefined;
  stored?: string | null;
}): string | null {
  return nonempty(sources.explicit) ?? nonempty(sources.env(TOKEN_ENV)) ?? nonempty(sources.stored ?? null);
}

function usage(message: string): FailClosed {
  return new FailClosed(400, "invalid_cli", message);
}

/** Take the value after flag `flag` at argv[i]; a missing value
 * refuses rather than being silently dropped (the sync-cli idiom). */
function flagValue(argv: string[], i: number, flag: string): string {
  const value = argv[i + 1];
  if (value === undefined || value.startsWith("--")) {
    throw usage(`flag ${flag} requires a value`);
  }
  return value;
}

function renderJson(doc: unknown): string {
  return `${JSON.stringify(doc, null, 2)}\n`;
}

/** The authenticated service call. Plumbing refusals (the router's
 * {error, message} documents) throw as FailClosed with the service's
 * own code; verb outcomes that are documents at HTTP 200 — the
 * queue door's broker envelope above all — return as documents. */
class ServiceClient {
  constructor(private deps: CliDeps) {}

  async post(path: string, body: Record<string, unknown>): Promise<Record<string, unknown>> {
    return this.call("POST", path, body);
  }

  /** The GET twin of post (the backfill run record is the one
   * door document a verb reads rather than writes): same token
   * carriage, same refusal idiom. */
  async get(path: string): Promise<Record<string, unknown>> {
    return this.call("GET", path, undefined);
  }

  private async call(
    method: "GET" | "POST",
    path: string,
    body: Record<string, unknown> | undefined,
  ): Promise<Record<string, unknown>> {
    const token = this.deps.token;
    if (token === null) {
      throw new FailClosed(
        401,
        "missing_token",
        `no caller token: set ${TOKEN_ENV} or store a token in the CLI token file (token storage is local to the caller)`,
      );
    }
    let res: Response;
    try {
      res = await this.deps.fetch(
        new Request(`${this.deps.serviceUrl}${path}`, {
          method,
          headers: {
            authorization: `Bearer ${token}`,
            ...(body !== undefined ? { "content-type": "application/json" } : {}),
          },
          ...(body !== undefined ? { body: JSON.stringify(body) } : {}),
        }),
      );
    } catch (e) {
      throw new FailClosed(
        502,
        "service_unreachable",
        `fleet-hosted service did not answer at ${this.deps.serviceUrl}: ${e instanceof Error ? e.message : String(e)}`,
      );
    }
    let doc: unknown;
    try {
      doc = await res.json();
    } catch {
      throw new FailClosed(
        502,
        "service_unreachable",
        `fleet-hosted service answered HTTP ${res.status} with a non-JSON body`,
      );
    }
    if (typeof doc !== "object" || doc === null || Array.isArray(doc)) {
      throw new FailClosed(
        502,
        "service_unreachable",
        `fleet-hosted service answered HTTP ${res.status} with a non-object body`,
      );
    }
    const record = doc as Record<string, unknown>;
    if (!res.ok) {
      const code = typeof record["error"] === "string" ? record["error"] : "service_error";
      const message = typeof record["message"] === "string" ? record["message"] : `HTTP ${res.status}`;
      throw new FailClosed(res.status, code, message);
    }
    return record;
  }
}

// --- Session carriage -------------------------------------------------

/** Resolve the carried session id (explicit > env > stored) with
 * the resolver's exact semantics. The stored id's liveness is a
 * registry question (session-resolution.ts): it is answered by a
 * resolve call before the resolver trusts the id, and the named
 * stale error — not a silent fallback — is what a dead stored id
 * produces. Returns null when no level supplies a session (an
 * uncited call, which the doors accept where the design does). */
async function carriedSessionId(deps: CliDeps, client: ServiceClient, explicit: string | null): Promise<string | null> {
  const env = deps.env;
  const supplied =
    nonempty(explicit) !== null ||
    nonempty(env(FLEET_SESSION_ENV)) !== null ||
    nonempty(env(OPENCODE_SESSION_ENV)) !== null;
  let storedId: string | null = null;
  let storedLive = true;
  if (!supplied) {
    storedId = nonempty(deps.store.read());
    if (storedId !== null && isValidSessionId(storedId)) {
      try {
        await client.post("/v1/sessions/resolve", { session_id: storedId });
      } catch (e) {
        if (e instanceof FailClosed && (e.code === "session_unknown" || e.code === "session_closed")) {
          storedLive = false;
        } else {
          throw e;
        }
      }
    }
  }
  const resolution = resolveAndBindSession({
    explicit,
    env,
    store: deps.store,
    isLive: (id) => (storedId !== null && id === storedId ? storedLive : true),
  });
  return resolution.kind === "session" ? resolution.sessionId : null;
}

/** The session id a close acts on: explicit > env > stored, with no
 * liveness demand — the registry's own answer (200, or 409 when
 * already closed) is the authority on the session's state. */
function closableSessionId(deps: CliDeps, explicit: string | null): string | null {
  const resolution = resolveSessionId({ explicit, env: deps.env, stored: deps.store.read() }, () => true);
  return resolution.kind === "session" ? resolution.sessionId : null;
}

/** Parse the lone --session-id flag of the session resume/close
 * verbs; anything else on the line refuses. */
function parseSessionFlagOnly(argv: string[], verb: string): string | null {
  let explicit: string | null = null;
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === "--session-id") {
      explicit = flagValue(argv, i, "--session-id");
      i++;
    } else {
      throw usage(`unknown session ${verb} argument: ${argv[i]}`);
    }
  }
  return explicit;
}

// --- Session verbs ----------------------------------------------------

async function verbSessionOpen(deps: CliDeps, client: ServiceClient, argv: string[]): Promise<CliOutcome> {
  const body: Record<string, unknown> = {};
  const laneStartIssues: string[] = [];
  for (let i = 0; i < argv.length; i++) {
    const flag = argv[i];
    switch (flag) {
      case "--lane":
        body["lane"] = flagValue(argv, i, flag);
        i++;
        break;
      case "--model":
        body["model"] = flagValue(argv, i, flag);
        i++;
        break;
      case "--lane-start-issue":
        laneStartIssues.push(flagValue(argv, i, flag));
        i++;
        break;
      default:
        throw usage(`unknown session open flag: ${flag}`);
    }
  }
  if (laneStartIssues.length > 0) body["lane_start_issues"] = laneStartIssues;
  // The service mints the id and owns the binding (§4.2): the
  // request carries only the caller's declarations — lane, model,
  // lane-start issues — never an id, never a tenancy.
  const record = await client.post("/v1/sessions", body);
  const sessionId = record["session_id"];
  if (typeof sessionId === "string") {
    deps.store.write(sessionId);
  }
  return { exitCode: 0, stdout: renderJson(record), stderr: "" };
}

async function verbSessionResume(deps: CliDeps, client: ServiceClient, argv: string[]): Promise<CliOutcome> {
  const explicit = parseSessionFlagOnly(argv, "resume");
  const sessionId = await carriedSessionId(deps, client, explicit);
  if (sessionId === null) {
    throw usage("no session to resume: open one, pass --session-id, or set FLEET_SESSION_ID");
  }
  const record = await client.post("/v1/sessions/resume", { session_id: sessionId });
  return { exitCode: 0, stdout: renderJson(record), stderr: "" };
}

async function verbSessionClose(deps: CliDeps, client: ServiceClient, argv: string[]): Promise<CliOutcome> {
  const explicit = parseSessionFlagOnly(argv, "close");
  const sessionId = closableSessionId(deps, explicit);
  if (sessionId === null) {
    throw usage("no session to close: pass --session-id, set FLEET_SESSION_ID, or open one in this workspace");
  }
  const record = await client.post("/v1/sessions/close", { session_id: sessionId });
  return { exitCode: 0, stdout: renderJson(record), stderr: "" };
}

// --- Create verb ------------------------------------------------------

const ITEM_REF = /^([A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+)#(\d+)$/;

function parseItemRef(text: string, flag: string): { repo: string; number: number } {
  const match = ITEM_REF.exec(text);
  if (!match) {
    throw usage(`${flag} takes <owner/repo#number>, got ${text}`);
  }
  return { repo: match[1], number: Number(match[2]) };
}

interface CreateArgs {
  kind: "issue" | "pr";
  repo: string;
  title: string;
  body: string;
  bodyFile: string | null;
  directParent: { repo: string; number: number } | null;
  links: { target: { repo: string; number: number }; relation: string; status: string }[];
  head: string | null;
  base: string | null;
  host: string | null;
  worktree: string | null;
  sessionId: string | null;
}

function parseCreateArgs(kind: "issue" | "pr", argv: string[]): CreateArgs {
  const args: CreateArgs = {
    kind,
    repo: "",
    title: "",
    body: "",
    bodyFile: null,
    directParent: null,
    links: [],
    head: null,
    base: null,
    host: null,
    worktree: null,
    sessionId: null,
  };
  let bodyFile: string | null = null;
  let bodyText: string | null = null;
  for (let i = 0; i < argv.length; i++) {
    const flag = argv[i];
    switch (flag) {
      case "--repo":
        args.repo = flagValue(argv, i, flag);
        i++;
        break;
      case "--title":
        args.title = flagValue(argv, i, flag);
        i++;
        break;
      case "--body":
        bodyText = flagValue(argv, i, flag);
        i++;
        break;
      case "--body-file":
        bodyFile = flagValue(argv, i, flag);
        i++;
        break;
      case "--direct-parent":
        args.directParent = parseItemRef(flagValue(argv, i, flag), flag);
        i++;
        break;
      case "--link": {
        const value = flagValue(argv, i, flag);
        i++;
        const parts = value.split(":");
        if (parts.length !== 3 || parts[0] === "" || parts[1] === "") {
          throw usage(`--link takes <relation>:<status>:<owner/repo#number>, got ${value}`);
        }
        args.links.push({
          relation: parts[0],
          status: parts[1],
          target: parseItemRef(parts[2], "--link"),
        });
        break;
      }
      case "--head":
        args.head = flagValue(argv, i, flag);
        i++;
        break;
      case "--base":
        args.base = flagValue(argv, i, flag);
        i++;
        break;
      case "--host":
        args.host = flagValue(argv, i, flag);
        i++;
        break;
      case "--worktree":
        args.worktree = flagValue(argv, i, flag);
        i++;
        break;
      case "--session-id":
        args.sessionId = flagValue(argv, i, flag);
        i++;
        break;
      default:
        throw usage(`unknown create flag: ${flag}`);
    }
  }
  if (bodyText !== null && bodyFile !== null) {
    throw usage("create takes --body or --body-file, not both");
  }
  args.body = bodyText ?? "";
  args.bodyFile = bodyFile;
  if (args.repo === "") throw usage("create requires --repo <owner/name>");
  if (args.title === "") throw usage("create requires --title <text>");
  if (kind === "issue" && (args.head !== null || args.base !== null)) {
    throw usage("create issue takes no --head/--base (those are PR claims)");
  }
  return args;
}

/** Assemble the caller's body text with the caller-owned Metadata
 * fields the flags named (design §3.2). A block the body already
 * carries wins for the fields it sets — caller-supplied values are
 * never overwritten, by flags any more than by the service — and a
 * malformed existing block refuses here, as it would at the door.
 * The block's provenance, if any, is passed through untouched: it
 * is the service's to replace, not the CLI's to launder. */
export function assembleCallerBody(args: CreateArgs): string {
  if (args.directParent === null && args.links.length === 0) {
    return args.body;
  }
  const parsed = parseMetadataBlock(args.body);
  const block: Record<string, unknown> = parsed.block ?? { pm_meta: 1 };
  if (args.directParent !== null && block["direct_parent"] === undefined) {
    block["direct_parent"] = args.directParent;
  }
  if (args.links.length > 0 && block["links"] === undefined) {
    block["links"] = args.links;
  }
  return renderBodyWithBlock(parsed, block);
}

/** The create request document — a fixed allowlist of caller-owned
 * fields and verifiable claims (repo, title, body, head, base,
 * host, worktree, the cited session). No timestamps, no queue
 * fields, no provenance: the stamp is the service's (§4.3). */
export function buildCreateRequest(
  args: CreateArgs,
  sessionId: string | null,
  claims: { host: string; worktree: string },
): Record<string, unknown> {
  const request: Record<string, unknown> = {
    repo: args.repo,
    kind: args.kind,
    title: args.title,
    body: assembleCallerBody(args),
    host: args.host ?? claims.host,
    worktree: args.worktree ?? claims.worktree,
  };
  if (sessionId !== null) request["session_id"] = sessionId;
  if (args.head !== null) request["head"] = args.head;
  if (args.base !== null) request["base"] = args.base;
  return request;
}

async function verbCreate(deps: CliDeps, client: ServiceClient, argv: string[]): Promise<CliOutcome> {
  const kind = argv[0];
  if (kind !== "issue" && kind !== "pr") {
    throw usage("create takes a kind: create issue | create pr");
  }
  const args = parseCreateArgs(kind, argv.slice(1));
  if (args.bodyFile !== null) {
    args.body = deps.readFile(args.bodyFile);
  }
  const sessionId = await carriedSessionId(deps, client, args.sessionId);
  const request = buildCreateRequest(args, sessionId, {
    host: deps.hostname(),
    worktree: deps.cwd(),
  });
  const created = await client.post("/v1/stamps", request);
  return { exitCode: 0, stdout: renderJson(created), stderr: "" };
}

// --- Query verb -------------------------------------------------------

/** The parsed filter back into the door's wire shape (the raw
 * snake_case document parseQueryFilter validates): one validation
 * path, so a CLI refusal and a REST refusal are the same refusal. */
export function queryFilterToWire(filter: MetadataFilter): Record<string, unknown> {
  const wire: Record<string, unknown> = {};
  if (filter.repo !== undefined) wire["repo"] = filter.repo;
  if (filter.kind !== undefined) wire["kind"] = filter.kind;
  if (filter.directParent !== undefined) {
    wire["direct_parent"] = `${filter.directParent.repo}#${filter.directParent.number}`;
  }
  if (filter.sessionId !== undefined) wire["session_id"] = filter.sessionId;
  if (filter.lane !== undefined) wire["lane"] = filter.lane;
  if (filter.host !== undefined) wire["host"] = filter.host;
  if (filter.queueId !== undefined) wire["queue_id"] = filter.queueId;
  if (filter.linkRelation !== undefined) wire["link_relation"] = filter.linkRelation;
  if (filter.linkStatus !== undefined) wire["link_status"] = filter.linkStatus;
  if (filter.linkTarget !== undefined) {
    wire["link_target"] = `${filter.linkTarget.repo}#${filter.linkTarget.number}`;
  }
  return wire;
}

async function verbQuery(client: ServiceClient, argv: string[]): Promise<CliOutcome> {
  const filter = parseQueryCliArgs(argv);
  const response = await client.post("/v1/query", queryFilterToWire(filter));
  return { exitCode: 0, stdout: renderQueryResponse(response as unknown as QueryResponse), stderr: "" };
}

// --- Intake verbs (next / claim) --------------------------------------

interface IntakeArgs {
  worker: string | null;
  lane: string | null;
  queue: string | null;
  count: number | null;
  taskId: number | null;
  sessionId: string | null;
}

function parseIntakeArgs(argv: string[], verb: string): IntakeArgs {
  const args: IntakeArgs = { worker: null, lane: null, queue: null, count: null, taskId: null, sessionId: null };
  for (let i = 0; i < argv.length; i++) {
    const flag = argv[i];
    switch (flag) {
      case "--worker":
        args.worker = flagValue(argv, i, flag);
        i++;
        break;
      case "--session-id":
        args.sessionId = flagValue(argv, i, flag);
        i++;
        break;
      case "--lane":
        args.lane = flagValue(argv, i, flag);
        i++;
        break;
      case "--queue":
        args.queue = flagValue(argv, i, flag);
        i++;
        break;
      case "--count": {
        const raw = flagValue(argv, i, flag);
        i++;
        const n = Number(raw);
        if (!Number.isInteger(n) || n < 1 || n > 5) {
          throw usage(`${verb} --count takes an integer in 1..5, got ${raw}`);
        }
        args.count = n;
        break;
      }
      case "--task-id": {
        if (verb !== "claim") throw usage(`${verb} takes no --task-id (claim by id is the claim verb's)`);
        const raw = flagValue(argv, i, flag);
        i++;
        const n = Number(raw);
        if (!Number.isInteger(n) || n < 1) {
          throw usage(`claim --task-id takes a positive integer, got ${raw}`);
        }
        args.taskId = n;
        break;
      }
      default:
        throw usage(`unknown ${verb} flag: ${flag}`);
    }
  }
  return args;
}

function intakeWorker(deps: CliDeps, args: IntakeArgs, verb: string): string {
  const worker = args.worker ?? nonempty(deps.env(WORKER_ENV));
  if (worker === null) {
    throw usage(`${verb} requires --worker <id> (or ${WORKER_ENV} in the environment): a claim names its worker`);
  }
  return worker;
}

function claimRequest(args: IntakeArgs, worker: string, sessionId: string | null): Record<string, unknown> {
  const request: Record<string, unknown> = { worker };
  if (args.lane !== null) request["lane"] = args.lane;
  if (args.queue !== null) request["queue"] = args.queue;
  if (args.count !== null) request["count"] = args.count;
  if (args.taskId !== null) request["task_id"] = args.taskId;
  if (sessionId !== null) request["session_id"] = sessionId;
  return request;
}

type Envelope = Record<string, unknown>;

function envelopeExit(envelope: Envelope): number {
  if (envelope["ok"] === true) return 0;
  return typeof envelope["exit"] === "number" ? envelope["exit"] : 1;
}

function claimedRows(envelope: Envelope): Envelope[] {
  if (Array.isArray(envelope["rows"])) return envelope["rows"] as Envelope[];
  if (typeof envelope["row"] === "object" && envelope["row"] !== null) {
    return [envelope["row"] as Envelope];
  }
  return [];
}

async function verbClaim(deps: CliDeps, client: ServiceClient, argv: string[]): Promise<CliOutcome> {
  const args = parseIntakeArgs(argv, "claim");
  const worker = intakeWorker(deps, args, "claim");
  const sessionId = await carriedSessionId(deps, client, args.sessionId);
  const envelope = await client.post("/v1/queue/claim", claimRequest(args, worker, sessionId));
  return { exitCode: envelopeExit(envelope), stdout: renderJson(envelope), stderr: "" };
}

/** Work intake: the claim-flow composite the queue door names as
 * client-side (claim + lane + bind). The claim names the worker,
 * the claimant lane and the carried session; the lane verb then
 * creates the task's lane ledger row, and the bind verb attaches
 * the carried session to it, so the work the CLI just took is
 * bound to the session that will be stamped for it. A step's
 * refusal stops the composite for that task and is reported in
 * the rendered document, with the step's envelope exit as the
 * process exit. */
async function verbNext(deps: CliDeps, client: ServiceClient, argv: string[]): Promise<CliOutcome> {
  const args = parseIntakeArgs(argv, "next");
  const worker = intakeWorker(deps, args, "next");
  const sessionId = await carriedSessionId(deps, client, args.sessionId);
  const claim = await client.post("/v1/queue/claim", claimRequest(args, worker, sessionId));
  if (claim["ok"] !== true) {
    return { exitCode: envelopeExit(claim), stdout: renderJson(claim), stderr: "" };
  }
  const rows = claimedRows(claim);
  if (rows.length === 0) {
    return { exitCode: 0, stdout: renderJson(claim), stderr: "" };
  }
  const intake: Record<string, unknown>[] = [];
  let exitCode = 0;
  for (const row of rows) {
    const taskId = row["id"];
    const lane = await client.post("/v1/queue/lane", { task_id: taskId, worker });
    let bind: Envelope | null = null;
    if (lane["ok"] === true && sessionId !== null) {
      bind = await client.post("/v1/queue/bind", {
        task_id: taskId,
        worker,
        session_id: sessionId,
      });
    }
    if (lane["ok"] !== true && exitCode === 0) exitCode = envelopeExit(lane);
    if (bind !== null && bind["ok"] !== true && exitCode === 0) exitCode = envelopeExit(bind);
    intake.push({ task_id: taskId, lane, bind });
  }
  return { exitCode, stdout: renderJson({ claim, intake }), stderr: "" };
}

// --- Backfill verbs (#14312) -------------------------------------------

/** The scope body the operator routes accept: named repos, or an
 * empty document for the deployment's configured scope (the
 * routes' own fallback — the CLI asserts no scope of its own). */
function scopeBody(repos: string[]): Record<string, unknown> {
  return repos.length > 0 ? { repos } : {};
}

async function verbBackfill(client: ServiceClient, argv: string[]): Promise<CliOutcome> {
  if (argv[0] === "runs") {
    parseBackfillRunsArgs(argv.slice(1));
    const doc = await client.get("/v1/ingestion/backfill/runs");
    return { exitCode: 0, stdout: renderJson(doc), stderr: "" };
  }
  const repos = parseRepoScopeArgs(argv, "backfill");
  const record = await client.post("/v1/ingestion/backfill", scopeBody(repos));
  return { exitCode: 0, stdout: renderJson(record), stderr: "" };
}

async function verbDivergence(client: ServiceClient, argv: string[]): Promise<CliOutcome> {
  const repos = parseRepoScopeArgs(argv, "divergence");
  const report = await client.post("/v1/ingestion/divergence", scopeBody(repos));
  return { exitCode: 0, stdout: renderJson(report), stderr: "" };
}

// --- Sync verbs ---------------------------------------------------------

async function verbSync(client: ServiceClient, argv: string[]): Promise<CliOutcome> {
  const kind = argv[0];
  if (kind === "project") {
    const request = parseProjectSyncCliArgs(argv.slice(1));
    const response = await client.post("/v1/sync/project", request as unknown as Record<string, unknown>);
    return { exitCode: 0, stdout: renderSyncResponse(response as unknown as ProjectSyncResponse), stderr: "" };
  }
  if (kind === "flow") {
    const request = parseFlowSyncCliArgs(argv.slice(1));
    const response = await client.post("/v1/sync/queue-github", request as unknown as Record<string, unknown>);
    return { exitCode: 0, stdout: renderSyncResponse(response as unknown as FlowSyncResponse), stderr: "" };
  }
  throw usage("sync takes a kind: sync project | sync flow");
}

// --- Entry --------------------------------------------------------------

const USAGE = [
  "usage: fleet-hosted <verb> ...",
  "  session open|resume|close — carriage over the session registry",
  "  create issue|pr — stamped create through the service",
  "  query — Metadata query (JSON first)",
  "  next | claim — work intake through the queue door",
  "  backfill [runs] — recorded Metadata backfill through the ingestion door",
  "  divergence — body-vs-DB divergence report through the ingestion door",
  "  sync project|flow — sync verbs through the sync door",
  "the full command line is declared in src/cli.ts (#3352)",
].join("\n");

/** Run one invocation. Refusals render as the package's error
 * document ({error, message}) on stderr with a non-zero exit;
 * results render JSON-first on stdout (design §3.2). */
export async function runCli(argv: string[], deps: CliDeps): Promise<CliOutcome> {
  const client = new ServiceClient(deps);
  try {
    const [verb, ...rest] = argv;
    switch (verb) {
      case "session": {
        const [sub, ...subRest] = rest;
        if (sub === "open") return await verbSessionOpen(deps, client, subRest);
        if (sub === "resume") return await verbSessionResume(deps, client, subRest);
        if (sub === "close") return await verbSessionClose(deps, client, subRest);
        throw usage("session takes a verb: session open | session resume | session close");
      }
      case "create":
        return await verbCreate(deps, client, rest);
      case "query":
        return await verbQuery(client, rest);
      case "next":
        return await verbNext(deps, client, rest);
      case "claim":
        return await verbClaim(deps, client, rest);
      case "backfill":
        return await verbBackfill(client, rest);
      case "divergence":
        return await verbDivergence(client, rest);
      case "sync":
        return await verbSync(client, rest);
      case undefined:
        throw usage(USAGE);
      default:
        throw usage(`unknown verb: ${verb}\n${USAGE}`);
    }
  } catch (e) {
    if (e instanceof SessionResolutionError) {
      return {
        exitCode: 1,
        stdout: "",
        stderr: renderJson({ error: e.code, message: e.message }),
      };
    }
    if (e instanceof FailClosed) {
      return {
        exitCode: 1,
        stdout: "",
        stderr: renderJson({ error: e.code, message: e.message }),
      };
    }
    throw e;
  }
}
