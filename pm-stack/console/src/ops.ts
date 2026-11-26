// What the console can do. Every operation is one of the verbs the
// fleet already has: the queue through `fleet queue` and its store,
// messages through `fleet bridge`, GitHub through `fleet gh`, and
// services through `systemctl --user`. Nothing here builds a shell
// command line: each call is an argument vector, and text a person
// typed travels on stdin or as a single argument.
//
// The edge Worker imports this module and runs Ops with stand-ins for
// `run` and `fetch`, so nothing this module needs at load time may come
// from the filesystem: node:fs is imported lazily inside enqueue(), the
// one method that touches the disk, and the expose script's path is
// resolved inside expose().
import type { Config } from "./config.ts";
import { OpError } from "./errors.ts";
import { plain } from "./plain.ts";
import { WORKERS_FILE, workersList } from "./platform.ts";
import { STORES_FILE, storesList } from "./stores.ts";
import { heldTokens, jwtExpiry, type TokenExpiry, tokenExpiry } from "./tokens.ts";
import { TURSO_ADMIN_WORKFLOW, tursoAdminInputs, tursoAdminRequest, tursoAdminRun } from "./turso-admin.ts";
import { timed, unread } from "./timed.ts";

export interface RunResult {
  code: number;
  stdout: string;
  stderr: string;
}
export type Run = (argv: string[], opts?: { stdin?: string; timeoutMs?: number }) => Promise<RunResult>;
export type Fetch = (
  url: string,
  init: { method: string; headers: Record<string, string>; body?: string; signal?: AbortSignal },
) => Promise<{
  ok: boolean;
  status: number;
  json(): Promise<unknown>;
  /** What the answer says of itself, where the caller reads it (the
   * edge does, for GitHub's tag on an answer and when to ask again). */
  headers?: { get(name: string): string | null };
}>;
export type Row = Record<string, string | number | null>;
/** How much of a task's reason the Queue screen is given. One that is
 * this long was cut here, and the screen says so. */
export const QUEUE_WHY_CHARS = 300;

/** A value as the queue's store sends it. A whole number comes as text
 * marked as an integer (the wire carries 64 bits, and a JSON number
 * does not): it is read as the number it is, where a number can hold
 * it exactly. Anything else is passed on as it came. */
function fromStore(cell: { type?: string; value?: string | number | null } | undefined): string | number | null {
  const value = cell?.value ?? null;
  if (cell?.type === "integer" && typeof value === "string" && /^-?[0-9]+$/.test(value)) {
    const n = Number(value);
    if (Number.isSafeInteger(n)) return n;
  }
  return value;
}
/** How long the queue store has to answer one asking, body and all. */
export const QUEUE_ASK_MS = 20_000;
/** Queue priorities, most urgent first (the queue's design scale). */
export const PRIORITY_SCALE = [10, 20, 30, 40, 50, 70, 90] as const;

export { OpError };

export const realRun: Run = async (argv, opts = {}) => {
  const proc = Bun.spawn(argv, {
    stdin: opts.stdin === undefined ? "ignore" : new Blob([opts.stdin]),
    stdout: "pipe",
    stderr: "pipe",
  });
  const timer = setTimeout(() => proc.kill(), opts.timeoutMs ?? 30_000);
  try {
    const [stdout, stderr, code] = await Promise.all([
      new Response(proc.stdout).text(),
      new Response(proc.stderr).text(),
      proc.exited,
    ]);
    return { code, stdout, stderr };
  } finally {
    clearTimeout(timer);
  }
};

/** Rank labels, most urgent first: the order the intake ladder reads. */
export const RANKS = ["expedite", "priority/critical", "now", "priority/high", "next"] as const;
export type Rank = (typeof RANKS)[number] | "none";
/** Pages of ranked issues read before giving up (100 issues a page). */
const PRIORITY_PAGES = 10;
export const QUEUE_STATUSES = ["queued", "claimed", "blocked", "parked"] as const;
const UNIT_NAME = /^[A-Za-z0-9@._-]+\.(service|timer)$/;
/** The workflows that put the site and the app out, by what they put out. */
const DEPLOYS = { site: "pm-console-edge-deploy.yml", app: "pm-console-android.yml" } as const;
const UNIT_ACTIONS = ["start", "stop", "restart"] as const;
export type UnitAction = (typeof UNIT_ACTIONS)[number];

function globToRegExp(glob: string): RegExp {
  return new RegExp("^" + glob.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, "[A-Za-z0-9@._-]*") + "$");
}

/** A unit the console may touch: a plain unit name matching a pattern. */
export function manageable(unit: string, patterns: string[]): boolean {
  return UNIT_NAME.test(unit) && patterns.some((p) => globToRegExp(p).test(unit));
}

export interface Unit {
  unit: string;
  load: string;
  active: string;
  sub: string;
  description: string;
}

/** Parse `systemctl list-units --plain --no-legend` output. */
export function parseUnits(text: string): Unit[] {
  const units: Unit[] = [];
  for (const line of text.split("\n")) {
    const m = /^\s*(?:●\s+)?(\S+)\s+(\S+)\s+(\S+)\s+(\S+)\s*(.*)$/.exec(line);
    if (!m || !UNIT_NAME.test(m[1])) continue;
    units.push({ unit: m[1], load: m[2], active: m[3], sub: m[4], description: m[5].trim() });
  }
  return units;
}

/** Queue priority for owner-filed work, on the queue's design scale
 * (10, 20, 30, ...): lower is sooner and 30 is the bulk band. */
export const RANK_PRIORITY: Record<Rank, number> = {
  expedite: 10,
  "priority/critical": 10,
  now: 20,
  "priority/high": 20,
  next: 30,
  none: 30,
};

/** The triage task for an issue the owner filed from the console. Same
 * shape as the coverage reconciliation's, so the two never disagree. */
export function triagePacket(
  repo: string,
  queue: string,
  number: number,
  title: string,
  rank: Rank,
  checkedAt: string,
) {
  const url = `https://github.com/${repo}/issues/${number}`;
  return {
    title: `Triage ${repo}#${number}: ${title}`.slice(0, 180),
    body: `Filed by the owner from the PM console: ${url}`,
    priority: RANK_PRIORITY[rank],
    dedupe_key: `repo-health:v1:triage:${repo}:issue:${number}`,
    payload: {
      schema: "repo-task/v1",
      kind: "triage",
      repo,
      queue,
      issue: { number, url, title, state_checked_at: checkedAt },
      problem:
        "The owner filed this from the PM console. It is new work with no task yet: triage it and either do it or return it to the queue sized for real work.",
      evidence: [`Filed from the PM console at ${checkedAt}, rank ${rank}`, url],
      acceptance: [
        "Issue triaged per the standing triage doctrine: annotate, validate, and either fix it in a small edit (merge with a closing keyword) or return it to the queue for real work",
      ],
      tests: ["Triage verdict recorded on the issue; if a small fix, the merged PR carries a closing keyword"],
      scope: { include: [`Triage of ${repo}#${number}`], exclude: ["Secrets, credentials or purchases"] },
      risk: { level: "low", flags: ["owner-filed"] },
      priority_reason: `Owner-filed from the console with rank ${rank}: priority ${RANK_PRIORITY[rank]}.`,
      timebox_minutes: 45,
      max_attempts: 1000,
      context_refs: [url],
      dedupe: {
        checked_at: checkedAt,
        basis: "The issue was created in the same pass as this task, so no other task can hold it.",
      },
    },
  };
}

/** Parse `systemctl list-unit-files --plain --no-legend` output:
 * unit, state, and sometimes a preset. Templates (name@.service) are
 * skipped: they cannot be started without an instance name. */
export function parseUnitFiles(text: string): { unit: string; state: string }[] {
  const files: { unit: string; state: string }[] = [];
  for (const line of text.split("\n")) {
    const m = /^\s*(\S+)\s+(\S+)/.exec(line);
    if (!m || !UNIT_NAME.test(m[1]) || m[1].includes("@.")) continue;
    files.push({ unit: m[1], state: m[2] });
  }
  return files;
}

function text(value: unknown, name: string, max: number, min = 1): string {
  if (typeof value !== "string") throw new OpError(400, `${name}: expected text`);
  const trimmed = value.trim();
  if (trimmed.length < min) throw new OpError(400, `${name}: required`);
  if (trimmed.length > max) throw new OpError(400, `${name}: longer than ${max} characters`);
  return trimmed;
}

function id(value: unknown, name: string): number {
  const s = String(value);
  if (!/^[1-9][0-9]{0,9}$/.test(s)) throw new OpError(400, `${name}: not a positive number`);
  return Number(s);
}

/** The bridge's target grammar (#14375): `all`, a bare name or glob, or
 * `agent:`/`group:` before either — the segment characters are the
 * bridge's own (names plus `*`), so a pattern the bridge would refuse
 * is refused here first, before anything runs. */
function bridgeTargets(to: unknown): string[] {
  if (!Array.isArray(to) || to.length === 0 || to.length > 50) throw new OpError(400, "to: one to fifty targets");
  return to.map((t) => {
    if (typeof t !== "string" || !/^(all|(agent:|group:)?[A-Za-z0-9*][A-Za-z0-9._*-]{0,63})$/.test(t))
      throw new OpError(400, `to: not a target: ${JSON.stringify(t).slice(0, 80)}`);
    return t;
  });
}

/** An optional whole-number count (a send limit, an offer's ttl):
 * absent is null; present must be a positive number within the cap. */
function optionalCount(value: unknown, name: string, max: number): number | null {
  if (value === undefined || value === null || value === "") return null;
  const n = id(value, name);
  if (n > max) throw new OpError(400, `${name}: at most ${max}`);
  return n;
}

/** A send's answer, as the composer shows it: the bridge's match
 * report, field for field. An answer without the report's numbers is
 * not a report at all — the send's outcome is unknown, and saying so
 * beats inventing counts. */
function matchReport(out: Record<string, unknown> | null, to: string[], command: boolean) {
  const num = (v: unknown) => (typeof v === "number" && Number.isFinite(v) ? v : null);
  const matched = num(out?.matched);
  const delivered = num(out?.delivered);
  if (!out || matched === null || delivered === null) {
    throw new OpError(502, "the bridge did not say who that reached");
  }
  return {
    to,
    command,
    batch: typeof out.batch === "string" ? out.batch : null,
    matched,
    delivered,
    recipients: Array.isArray(out.recipients) ? out.recipients.filter((r): r is string => typeof r === "string") : [],
    remaining: num(out.remaining) ?? Math.max(0, matched - delivered),
    unregistered: Array.isArray(out.unregistered) ? out.unregistered : [],
  };
}

/** How long what a token said of its expiry is kept before the token
 * is fetched again: long enough that the notifier's minute does not
 * fetch every secret every minute, short enough that a rotation shows
 * the same hour. */
export const TOKEN_EXPIRY_KEPT_MS = 10 * 60_000;

/** Where the repository-described Cloudflare configuration was last
 * successfully applied from: the apply workflow's own record. */
export interface ApplyProvenance {
  /** The workflow that applied it, and who set it going when GitHub
   * says so: the source the configuration was applied from. */
  source: string;
  /** The commit the applying run had checked out, in full. */
  commit: string;
  /** When that run finished, as GitHub recorded it (ISO 8601). */
  at: string;
  /** The run's own page, for the owner to look at. */
  url: string;
}

/** The latest successful apply in a workflow-runs answer, or null when
 * there is none. Only a dispatched run can apply at all — pushes and
 * pull requests of pm-cloudflare-config.yml only validate — and only a
 * run that finished successfully applied anything: a failed or
 * still-running one is never presented as an apply. A run whose record
 * is malformed is passed over, not trusted. Nothing here dispatches a
 * workflow or reads a secret: it only reads the runs GitHub already
 * recorded. */
export function latestApply(got: unknown): ApplyProvenance | null {
  const runs = (got as { workflow_runs?: unknown } | null)?.workflow_runs;
  if (!Array.isArray(runs)) return null;
  let best: (ApplyProvenance & { done: number }) | null = null;
  for (const raw of runs) {
    const run = raw as {
      event?: unknown;
      status?: unknown;
      conclusion?: unknown;
      head_branch?: unknown;
      head_sha?: unknown;
      updated_at?: unknown;
      html_url?: unknown;
      name?: unknown;
      triggering_actor?: { login?: unknown } | null;
      actor?: { login?: unknown } | null;
    } | null;
    if (!run || typeof run !== "object") continue;
    if (run.event !== "workflow_dispatch" || run.head_branch !== "main") continue;
    if (run.status !== "completed" || run.conclusion !== "success") continue;
    if (typeof run.head_sha !== "string" || !/^[0-9a-f]{40}$/.test(run.head_sha)) continue;
    if (typeof run.html_url !== "string" || !run.html_url.startsWith("https://")) continue;
    if (typeof run.updated_at !== "string") continue;
    const done = Date.parse(run.updated_at);
    if (!Number.isFinite(done)) continue;
    const workflow = typeof run.name === "string" && run.name ? run.name : "pm-console-cloudflare config";
    const actor = run.triggering_actor?.login ?? run.actor?.login;
    const source = typeof actor === "string" && actor ? `${workflow}, dispatched by ${actor}` : workflow;
    if (!best || done > best.done) best = { source, commit: run.head_sha, at: run.updated_at, url: run.html_url, done };
  }
  return best ? { source: best.source, commit: best.commit, at: best.at, url: best.url } : null;
}

export class Ops {
  private queueToken: string | null = null;
  /** What each token said of when it runs out, by its name, and when
   * that was read. Never the token itself. */
  private expiries = new Map<string, { at: number; expiresAt: number | null | undefined }>();

  /** How the queue store and the dashboard are reached. Kept as a plain
   * caller: these classes also run at the edge, where the platform's
   * fetch may not be called as a method (see `plain`). */
  readonly fetchImpl: Fetch;

  constructor(
    readonly cfg: Config,
    readonly run: Run = realRun,
    fetchImpl: Fetch = fetch as unknown as Fetch,
  ) {
    this.fetchImpl = plain(fetchImpl);
  }

  async must(argv: string[], opts?: { stdin?: string; timeoutMs?: number }): Promise<string> {
    const r = await this.run(argv, opts);
    if (r.code !== 0) {
      const detail = (r.stderr || r.stdout).trim().split("\n").slice(-3).join(" ").slice(0, 400);
      throw new OpError(502, `${argv[0]} ${argv[1] ?? ""} failed (exit ${r.code}): ${detail}`);
    }
    return r.stdout;
  }

  async gh(args: string[], body?: unknown): Promise<unknown> {
    const argv = [this.cfg.fleetBin, "gh", "api", ...args];
    const out = await this.must(
      body === undefined ? argv : [...argv, "--input", "-"],
      body === undefined ? undefined : { stdin: JSON.stringify(body) },
    );
    return out.trim() === "" ? null : JSON.parse(out);
  }

  private labels = new Set<string>();
  /** Make sure a label this service files under exists, with what it
   * means written on it. Asked once per label while this runs. */
  async ensureLabel(name: string, color: string, description: string) {
    if (this.labels.has(name)) return;
    const made = await this.run(
      [this.cfg.fleetBin, "gh", "api", "-X", "POST", `repos/${this.cfg.repo}/labels`, "--input", "-"],
      { stdin: JSON.stringify({ name, color, description }) },
    );
    // Exit 0: created. "already_exists": it was there. Anything else is a real failure.
    if (made.code !== 0 && !/already_exists/.test(made.stdout + made.stderr)) {
      throw new OpError(502, `could not prepare the ${name} label`);
    }
    this.labels.add(name);
  }

  // ---- queue -----------------------------------------------------------

  /** One request to the queue store carrying `statements` in order. */
  private async pipeline(statements: { sql: string; args: (string | number)[] }[]) {
    const body = {
      requests: [
        ...statements.map((st) => ({
          type: "execute",
          stmt: {
            sql: st.sql,
            args: st.args.map((a) =>
              typeof a === "number" ? { type: "integer", value: String(a) } : { type: "text", value: a },
            ),
          },
        })),
        { type: "close" },
      ],
    };
    type Answer = {
      results?: {
        type?: string;
        response?: {
          result?: {
            cols: { name: string }[];
            rows: { type?: string; value?: string | number | null }[][];
            affected_row_count?: number;
          };
        };
      }[];
    };
    /** One asking: the answer and its body, within the time. A queue
     * store that never answers would otherwise hold the read for as
     * long as it liked. (The token is fetched before the time starts:
     * on a machine that is the fleet binary's work, not the store's.) */
    const ask = async (): Promise<{ status: number; ok: boolean; body: Answer | null }> => {
      if (this.queueToken === null) {
        this.queueToken = (await this.must([this.cfg.fleetBin, "secrets", "get", this.cfg.queueTokenName])).trim();
      }
      const bearer = this.queueToken;
      try {
        return await timed(QUEUE_ASK_MS, async (signal) => {
          const res = await this.fetchImpl(`${this.cfg.queueDbUrl}/v2/pipeline`, {
            method: "POST",
            headers: { Authorization: `Bearer ${bearer}`, "Content-Type": "application/json" },
            body: JSON.stringify(body),
            signal,
          });
          if (res.ok) return { status: res.status, ok: true, body: (await res.json()) as Answer };
          unread(res);
          return { status: res.status, ok: false, body: null };
        });
      } catch (error) {
        if ((error as Error).name === "TimeoutError") {
          throw new OpError(502, `the queue store did not answer within ${QUEUE_ASK_MS / 1000} seconds`);
        }
        throw error;
      }
    };
    let answered = await ask();
    if (answered.status === 401 || answered.status === 403) {
      // The token was rotated: fetch it again, once.
      this.queueToken = null;
      answered = await ask();
    }
    if (!answered.ok) throw new OpError(502, `queue store answered http ${answered.status}`);
    const results = answered.body?.results ?? [];
    return statements.map((_, i) => {
      const result = results[i]?.type === "ok" ? results[i].response?.result : undefined;
      if (!result) throw new OpError(502, "queue store refused the query");
      return result;
    });
  }

  /** Several reads in one round trip. Reads only: anything else throws. */
  async reads(statements: { sql: string; args: (string | number)[] }[]): Promise<Row[][]> {
    for (const st of statements) {
      if (!/^\s*select\b/i.test(st.sql)) throw new Error("the console only reads the queue store");
    }
    return (await this.pipeline(statements)).map((result) =>
      result.rows.map((row) => Object.fromEntries(result.cols.map((c, i) => [c.name, fromStore(row[i])]))),
    );
  }

  async sql(statement: string, args: (string | number)[]): Promise<Row[]> {
    return (await this.reads([{ sql: statement, args }]))[0];
  }

  /**
   * Move a queued task along the priority scale. The one write this
   * service makes to the queue store, because the queue has no verb for
   * it: one column, on one row, and only while the task is still queued.
   */
  async setTaskPriority(task: unknown, priority: unknown) {
    const t = id(task, "task");
    if (!PRIORITY_SCALE.includes(priority as (typeof PRIORITY_SCALE)[number])) {
      throw new OpError(400, `priority: one of ${PRIORITY_SCALE.join(", ")}`);
    }
    const [result] = await this.pipeline([
      {
        sql: "UPDATE tasks SET priority = ? WHERE id = ? AND queue = ? AND status = 'queued'",
        args: [priority as number, t, this.cfg.queue],
      },
    ]);
    if (!result.affected_row_count) throw new OpError(409, `task ${t} is not queued, so its place cannot change`);
    return { task: t, priority };
  }

  async queueCounts(): Promise<Record<string, number>> {
    const rows = await this.sql(
      "SELECT status, count(*) AS n FROM tasks WHERE queue = ? AND status NOT IN ('done','cancelled','failed') GROUP BY status",
      [this.cfg.queue],
    );
    return Object.fromEntries(rows.map((r) => [String(r.status), Number(r.n)]));
  }

  async queueList(status: unknown, search: unknown, limit: unknown) {
    if (!QUEUE_STATUSES.includes(status as (typeof QUEUE_STATUSES)[number]))
      throw new OpError(400, `status: one of ${QUEUE_STATUSES.join(", ")}`);
    const n = limit === undefined || limit === null || limit === "" ? 50 : id(limit, "limit");
    const q = typeof search === "string" ? search.trim().slice(0, 100) : "";
    const like = `%${q.replace(/[\\%_]/g, "\\$&")}%`;
    return this.sql(
      `SELECT id, status, priority, title, dedupe_key, substr(updated_at,1,16) AS updated, substr(coalesce(json_extract(progress,'$.reason'), json_extract(progress,'$.blocker'), ready_condition, ''),1,${QUEUE_WHY_CHARS}) AS why FROM tasks WHERE queue = ? AND status = ? AND title LIKE ? ESCAPE '\\' ORDER BY (priority IS NULL), priority ASC, created_at ASC, id ASC LIMIT ?`,
      [this.cfg.queue, status as string, like, Math.min(n, 200)],
    );
  }

  async queueAct(action: unknown, task: unknown, reason: unknown) {
    if (action !== "unblock" && action !== "cancel") throw new OpError(400, "action: unblock or cancel");
    const taskId = id(task, "task");
    const why = text(reason, "reason", 500);
    await this.must([this.cfg.fleetBin, "queue", action, String(taskId), "--worker", "pm-console", "--reason", why]);
    return { task: taskId, action };
  }

  // ---- priorities and ideas ---------------------------------------------

  async priorities() {
    const q = `repo:${this.cfg.repo} is:issue is:open label:${RANKS.map((r) => `"${r}"`).join(",")}`;
    type Item = {
      number: number;
      title: string;
      html_url: string;
      updated_at: string;
      comments: number;
      labels: { name: string }[];
    };
    const items: Item[] = [];
    let truncated = false;
    // Search order is not rank order, so every page is read before sorting.
    for (let page = 1; ; page++) {
      const found = (await this.gh([
        "-X",
        "GET",
        "search/issues",
        "-f",
        `q=${q}`,
        "-f",
        "per_page=100",
        "-f",
        `page=${page}`,
      ])) as { items: Item[] };
      items.push(...found.items);
      if (found.items.length < 100) break;
      if (page === PRIORITY_PAGES) {
        truncated = true;
        break;
      }
    }
    const issues = items
      .map((i) => {
        const labels = i.labels.map((l) => l.name);
        const rank = RANKS.findIndex((r) => labels.includes(r));
        return {
          number: i.number,
          title: i.title,
          url: i.html_url,
          updated: i.updated_at.slice(0, 16),
          comments: i.comments,
          labels,
          rank: RANKS[rank] ?? "none",
          order: rank < 0 ? RANKS.length : rank,
        };
      })
      .sort((a, b) => a.order - b.order || a.number - b.number);
    return { issues, truncated };
  }

  async setRank(issue: unknown, rank: unknown) {
    const n = id(issue, "issue");
    if (rank !== "none" && !RANKS.includes(rank as (typeof RANKS)[number])) {
      throw new OpError(400, `rank: one of ${RANKS.join(", ")}, none`);
    }
    const current = (
      (await this.gh([`repos/${this.cfg.repo}/issues/${n}`])) as { labels: { name: string }[] }
    ).labels.map((l) => l.name);
    // Add first: if a later call fails the issue keeps a rank, and stays
    // on the list it would otherwise vanish from.
    if (rank !== "none" && !current.includes(rank as string)) {
      await this.gh(["-X", "POST", `repos/${this.cfg.repo}/issues/${n}/labels`], { labels: [rank] });
    }
    for (const label of RANKS) {
      if (label !== rank && current.includes(label)) {
        await this.gh(["-X", "DELETE", `repos/${this.cfg.repo}/issues/${n}/labels/${encodeURIComponent(label)}`]);
      }
    }
    return { issue: n, rank };
  }

  async comment(issue: unknown, body: unknown) {
    const n = id(issue, "issue");
    const made = (await this.gh(["-X", "POST", `repos/${this.cfg.repo}/issues/${n}/comments`], {
      body: text(body, "comment", 20_000),
    })) as { html_url: string };
    return { issue: n, url: made.html_url };
  }

  /** Hand one packet to the queue. `fleet queue enqueue` reads a packet
   * file, not stdin, so the packet lives in a private file for the call. */
  async enqueue(packet: unknown): Promise<RunResult> {
    // The filesystem is the laptop's only: this import is lazy so the
    // edge Worker, which never calls this, can import this module.
    const { mkdtempSync, rmSync, writeFileSync } = await import("node:fs");
    const { tmpdir } = await import("node:os");
    const { join } = await import("node:path");
    const dir = mkdtempSync(join(tmpdir(), "pm-console-"));
    const file = join(dir, "packet.json");
    try {
      writeFileSync(file, JSON.stringify(packet), { mode: 0o600 });
      return await this.run([this.cfg.fleetBin, "queue", "enqueue", file, "--queue", this.cfg.queue]);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  }

  /**
   * File an idea as an issue and enqueue its triage task in the same
   * pass (doc/agents/one-queue-claiming-work.md): an issue with no queue
   * task is invisible to every worker. If the enqueue fails the issue
   * still exists, the result says it is not queued, and the coverage
   * reconciliation picks it up.
   */
  async fileIdea(title: unknown, body: unknown, rank: unknown) {
    if (rank !== "none" && !RANKS.includes(rank as (typeof RANKS)[number])) {
      throw new OpError(400, `rank: one of ${RANKS.join(", ")}, none`);
    }
    const made = (await this.gh(["-X", "POST", `repos/${this.cfg.repo}/issues`], {
      title: text(title, "title", 200),
      body: typeof body === "string" ? body.slice(0, 60_000) : "",
      labels: rank === "none" ? [] : [rank],
    })) as { number: number; html_url: string; title: string };
    const packet = triagePacket(
      this.cfg.repo,
      this.cfg.queue,
      made.number,
      made.title,
      rank as Rank,
      new Date().toISOString(),
    );
    const queued = await this.enqueue(packet);
    return {
      issue: made.number,
      url: made.html_url,
      queued: queued.code === 0,
      ...(queued.code === 0
        ? {}
        : { note: `filed, not queued: ${(queued.stderr || queued.stdout).trim().split("\n").pop()?.slice(0, 200)}` }),
    };
  }

  // ---- agents ------------------------------------------------------------

  async agents() {
    return (JSON.parse(await this.must([this.cfg.fleetBin, "bridge", "agents"])) as { agents: unknown[] }).agents;
  }

  async messages(limit: unknown) {
    const n = limit === undefined || limit === null || limit === "" ? 30 : Math.min(id(limit, "limit"), 200);
    const log = (
      JSON.parse(await this.must([this.cfg.fleetBin, "bridge", "log", "--limit", String(n), "--order", "desc"])) as {
        messages?: unknown;
      } | null
    )?.messages;
    // A log with nothing in it is an empty list, never no answer at all.
    return Array.isArray(log) ? log : [];
  }

  /**
   * The owner's unified pipeline (#14374, design §4.9): one feed over
   * connector messages, fleet mailboxes and notes, read through
   * `fleet bridge pipeline`. The owner's view is never sender-stripped
   * — an instruction shows who really sent it here; anonymity is what
   * agents see, never this console. Filters are the bridge's own
   * (kind / sender / receiver / surface / batch / unread / work
   * state); each value is validated here and travels as its own
   * argument. (Named for the bridge's feed, not this class's own
   * queue-store pipeline above.)
   */
  async pipelineFeed(query: URLSearchParams) {
    const argv = [this.cfg.fleetBin, "bridge", "pipeline", "--order", "desc"];
    const one = (param: string, flag: string, name: string, ok: RegExp) => {
      const v = query.get(param);
      if (v === null || v === "") return;
      const value = text(v, name, 200);
      if (!ok.test(value)) throw new OpError(400, `${name}: not a ${name}: ${JSON.stringify(value).slice(0, 80)}`);
      argv.push(flag, value);
    };
    one("kind", "--kind", "kind", /^(message|instruction|command|note)$/);
    one("surface", "--surface", "surface", /^(connector|fleet|notes)$/);
    one("sender", "--sender", "sender", /^[A-Za-z0-9][A-Za-z0-9@:._/-]{0,127}$/);
    one("receiver", "--receiver", "receiver", /^[A-Za-z0-9][A-Za-z0-9@:._/-]{0,127}$/);
    one("batch", "--batch", "batch", /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/);
    one("work_state", "--work-state", "work state", /^[a-z][a-z0-9_-]{0,63}$/);
    if (query.get("unread") === "1" || query.get("unread_only") === "true") argv.push("--unread");
    const limit = query.get("limit");
    argv.push("--limit", String(limit === null || limit === "" ? 100 : Math.min(id(limit, "limit"), 200)));
    const rows = (JSON.parse(await this.must(argv)) as { messages?: unknown } | null)?.messages;
    return Array.isArray(rows) ? rows : [];
  }

  /**
   * The one count the phone's widget adds to its summary (#14377,
   * design §4.10): how many instructions in the owner's pipeline are
   * unread — connector instructions and fleet commands under the
   * pipeline's own unread rule (inbound, not yet read), counted from
   * the same feed the Messages screen shows, so the widget's number
   * is the screen's. The feed is read at its own largest page: a
   * backlog past the page counts as the page, as it does on the
   * screen. This is the `pipeline` document section's whole data;
   * the phone's token stays read-only and summary-scoped — it is
   * handed this count by the summary, never the feed.
   */
  async unreadInstructions() {
    const rows = await this.pipelineFeed(new URLSearchParams({ unread: "1", limit: "200" }));
    const instructions = rows.filter((r) => {
      const kind = (r as { kind?: unknown } | null)?.kind;
      return kind === "instruction" || kind === "command";
    }).length;
    return { unreadInstructions: instructions };
  }

  /** Per-agent credential state on the bridge (#14374): active or
   * revoked, the scopes the live credentials carry. State only — the
   * bridge never returns a hash, and neither does this. */
  async credentials() {
    const rows = (
      JSON.parse(await this.must([this.cfg.fleetBin, "bridge", "credentials"])) as { credentials?: unknown } | null
    )?.credentials;
    return Array.isArray(rows) ? (rows as Record<string, unknown>[]) : [];
  }

  /**
   * The unified agents list for the Agents screen (#14374, design
   * §4.9): the bridge's directory (`fleet bridge directory` — fleet
   * registry rows and notes identities, each marked by surface) with
   * each row's credential state merged in. A fleet row's state comes
   * from the credential view under the agent's own name; a notes
   * row's from its identity's token scopes — an identity whose tokens
   * were revoked keeps its directory place with both scopes off,
   * which is the revoked state the screen shows. Notes identities
   * have no presence: the bridge says `presence: "none"` and this
   * merge invents nothing.
   */
  async directory() {
    const [dir, creds] = await Promise.all([
      this.must([this.cfg.fleetBin, "bridge", "directory"]).then(
        (out) => (JSON.parse(out) as { agents?: unknown } | null)?.agents,
      ),
      this.credentials().catch(() => [] as Record<string, unknown>[]),
    ]);
    const rows = Array.isArray(dir) ? (dir as Record<string, unknown>[]) : [];
    const byAgent = new Map(creds.map((c) => [String(c.agent), c]));
    return rows.map((row) => {
      if (row.surface === "fleet") {
        const c = byAgent.get(String(row.name));
        return {
          ...row,
          credential: c
            ? {
                state: c.state,
                scopes: Array.isArray(c.scopes) ? c.scopes : [],
                credentials: typeof c.credentials === "number" ? c.credentials : 0,
                created: c.created ?? null,
                revoked_at: c.revoked_at ?? null,
              }
            : null,
        };
      }
      const scopes = (row.scopes ?? {}) as { can_send?: unknown; can_inbox?: unknown };
      const live = scopes.can_send === true || scopes.can_inbox === true;
      return {
        ...row,
        credential: {
          state: live ? "active" : "revoked",
          scopes: [...(scopes.can_send === true ? ["send"] : []), ...(scopes.can_inbox === true ? ["inbox"] : [])],
          credentials: null,
          created: row.first_seen ?? null,
          revoked_at: null,
        },
      };
    });
  }

  /**
   * Revoke an agent's bridge credentials from the Agents screen
   * (#14374): the owner's kill switch, per surface — a fleet agent's
   * credentials, or a notes identity's tokens. Revoking is the
   * owner's act: on the laptop it runs here; at the edge the action
   * is held for a machine (edge-proto.ts), never performed by the
   * edge's read-scope credential.
   */
  async revokeCredential(surface: unknown, name: unknown) {
    const s = text(surface, "surface", 20);
    if (s !== "fleet" && s !== "notes") throw new OpError(400, `surface: not a surface: ${JSON.stringify(s)}`);
    const n = text(name, "name", 200);
    if (!/^[A-Za-z0-9][A-Za-z0-9@._/-]{0,127}$/.test(n)) {
      throw new OpError(400, `name: not an agent name: ${JSON.stringify(n).slice(0, 80)}`);
    }
    const argv =
      s === "fleet"
        ? [this.cfg.fleetBin, "bridge", "revoke", "--agent", n]
        : [this.cfg.fleetBin, "bridge", "revoke", "--identity", n];
    const out = JSON.parse(await this.must(argv)) as { revoked?: unknown };
    return { surface: s, name: n, revoked: typeof out.revoked === "number" ? out.revoked : 0 };
  }

  /** The minted bootstrap tokens' state for the Agents screen
   * (#14376): pending, used, expired or revoked, each with the times
   * that closed it. State only — the bridge stores hashes and returns
   * neither a hash nor a value here; a token's value existed once, in
   * its mint answer. */
  async enrollTokens() {
    const rows = (
      JSON.parse(await this.must([this.cfg.fleetBin, "bridge", "enroll-tokens"])) as {
        enroll_tokens?: unknown;
      } | null
    )?.enroll_tokens;
    return Array.isArray(rows) ? (rows as Record<string, unknown>[]) : [];
  }

  /**
   * The wake hooks for the Agents screen (#14544): one per address
   * (channel:claude, channel:chatgpt or agent:<name>), read through
   * `fleet bridge wake-hooks`. What the bridge shows of a hook —
   * address, kind, host, whether it holds a token — never the token
   * or the URL's path.
   */
  async wakeHooks() {
    const rows = (
      JSON.parse(await this.must([this.cfg.fleetBin, "bridge", "wake-hooks"])) as { hooks?: unknown } | null
    )?.hooks;
    return Array.isArray(rows) ? (rows as Record<string, unknown>[]) : [];
  }

  /**
   * Set one wake hook from the Agents screen (#14544), through the
   * wake-hooks API the retired bridge page used. The hook travels as
   * JSON on the CLI's stdin, never as an argument: its token is a
   * value, and values do not travel as arguments (mintEnrollToken).
   * The address and URL are checked here first, in the console's own
   * terms, before anything runs; the bridge checks them again.
   */
  async setWakeHook(input: { address?: unknown; url?: unknown; token?: unknown }) {
    const address = text(input.address, "address", 200).trim().toLowerCase();
    if (!/^(channel:(claude|chatgpt)|agent:[a-z0-9][a-z0-9._-]{0,63})$/.test(address)) {
      throw new OpError(400, `address: not a wake address: ${JSON.stringify(address).slice(0, 80)}`);
    }
    const url = text(input.url, "url", 2048).trim();
    let parsed: URL;
    try {
      parsed = new URL(url);
    } catch {
      throw new OpError(400, "url: not a URL");
    }
    if (parsed.protocol !== "https:" || parsed.username || parsed.password || parsed.hash) {
      throw new OpError(400, "url: must be https, with no user name, password or #fragment");
    }
    const hook: Record<string, unknown> = { address, url: parsed.href };
    if (input.token !== undefined && input.token !== null && input.token !== "") {
      hook.token = text(input.token, "token", 512);
    }
    const out = JSON.parse(
      await this.must([this.cfg.fleetBin, "bridge", "wake-hooks", "--set", "-"], { stdin: JSON.stringify(hook) }),
    ) as { hook?: unknown };
    return { hook: out.hook ?? null };
  }

  /**
   * Remove one address's wake hook from the Agents screen (#14544):
   * the same API with no URL in it. No value travels in it, and done
   * late it is still the right act, so at the edge it is held for a
   * machine like agent.revoke-enroll (edge-proto.ts).
   */
  async removeWakeHook(address: unknown) {
    const a = text(address, "address", 200).trim().toLowerCase();
    if (!/^(channel:(claude|chatgpt)|agent:[a-z0-9][a-z0-9._-]{0,63})$/.test(a)) {
      throw new OpError(400, `address: not a wake address: ${JSON.stringify(a).slice(0, 80)}`);
    }
    const out = JSON.parse(
      await this.must([this.cfg.fleetBin, "bridge", "wake-hooks", "--remove", "--address", a]),
    ) as { removed?: unknown };
    return { address: a, removed: out.removed === true };
  }

  /**
   * Mint a bootstrap enroll token from the Agents screen (#14376,
   * design §4.9), bound at mint to the new agent's name, kind, groups
   * and class notes identity, for the TTL the owner picked. The
   * bridge's answer carries the token value, and it passes straight
   * through to the caller — the one screen that shows it, once.
   * Nothing keeps it: this class holds no state but the queue token,
   * the action's audit record names only the binding (actions.ts),
   * and the value never travels as an argument — the CLI builds the
   * mint from these flags, and the value comes back on stdout alone.
   * The binding is checked here first, in the console's own terms,
   * before anything runs.
   */
  async mintEnrollToken(input: {
    agent?: unknown;
    kind?: unknown;
    groups?: unknown;
    notesIdentity?: unknown;
    ttlSeconds?: unknown;
  }) {
    const name = (value: unknown, what: string, max = 128) => {
      const n = text(value, what, max);
      if (!/^[A-Za-z0-9][A-Za-z0-9@._/-]{0,127}$/.test(n)) {
        throw new OpError(400, `${what}: not a name: ${JSON.stringify(n).slice(0, 80)}`);
      }
      return n;
    };
    const agent = name(input.agent, "agent");
    const argv = [this.cfg.fleetBin, "bridge", "mint-enroll", "--agent", agent];
    if (input.kind !== undefined && input.kind !== null && input.kind !== "") {
      argv.push("--kind", name(input.kind, "kind", 64));
    }
    if (input.groups !== undefined && input.groups !== null && input.groups !== "") {
      const groups = (Array.isArray(input.groups) ? input.groups : String(input.groups).split(","))
        .map((g) => (typeof g === "string" ? g.trim() : g))
        .filter((g) => g !== "");
      if (groups.length > 16) throw new OpError(400, "groups: at most sixteen");
      for (const g of groups) {
        if (typeof g !== "string" || !/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(g)) {
          throw new OpError(400, `groups: not a group name: ${JSON.stringify(g).slice(0, 80)}`);
        }
      }
      if (groups.length > 0) argv.push("--groups", groups.join(","));
    }
    if (input.notesIdentity !== undefined && input.notesIdentity !== null && input.notesIdentity !== "") {
      argv.push("--notes-identity", name(input.notesIdentity, "notes identity"));
    }
    if (input.ttlSeconds !== undefined && input.ttlSeconds !== null && input.ttlSeconds !== "") {
      const ttl = id(input.ttlSeconds, "ttl");
      if (ttl < 60 || ttl > 604_800) throw new OpError(400, "ttl: between a minute and a week, in seconds");
      argv.push("--ttl", String(ttl));
    }
    const out = JSON.parse(await this.must(argv)) as {
      agent?: unknown;
      kind?: unknown;
      groups?: unknown;
      notes_identity?: unknown;
      expires_at?: unknown;
      bootstrap_token?: unknown;
    } | null;
    if (typeof out?.bootstrap_token !== "string" || out.bootstrap_token === "") {
      throw new OpError(502, "the bridge's mint answer carried no token");
    }
    return {
      agent: typeof out.agent === "string" ? out.agent : agent,
      kind: typeof out.kind === "string" ? out.kind : "agent",
      groups: Array.isArray(out.groups) ? out.groups.filter((g) => typeof g === "string") : [],
      notes_identity: typeof out.notes_identity === "string" ? out.notes_identity : null,
      expires_at: typeof out.expires_at === "string" ? out.expires_at : null,
      bootstrap_token: out.bootstrap_token,
    };
  }

  /**
   * Take back an agent's unused bootstrap tokens from the Agents
   * screen (#14376): every still-pending token minted for the agent.
   * By name, never by value — the list shows no hash, and a token
   * value has no business travelling to undo its own minting. Used
   * tokens are not this revoke's (they became credentials; revoke
   * those above), and expired ones are already dead.
   */
  async revokeEnrollTokens(agent: unknown) {
    const n = text(agent, "agent", 200);
    if (!/^[A-Za-z0-9][A-Za-z0-9@._/-]{0,127}$/.test(n)) {
      throw new OpError(400, `agent: not an agent name: ${JSON.stringify(n).slice(0, 80)}`);
    }
    const out = JSON.parse(await this.must([this.cfg.fleetBin, "bridge", "revoke-enroll", "--agent", n])) as {
      revoked?: unknown;
    };
    return { agent: n, revoked: typeof out.revoked === "number" ? out.revoked : 0 };
  }

  /**
   * Mint a per-agent credential directly from the Agents screen
   * (#14376): for an agent that is already enrolled, or a service
   * credential with --addr. Like the enroll mint, the value passes
   * straight through to the one screen that shows it, once, and is
   * kept nowhere. A service address must name the agent itself — the
   * bridge binds a service credential to its own name, and the check
   * is made here first as everywhere else.
   */
  async mintCredential(input: { agent?: unknown; scopes?: unknown; addr?: unknown }) {
    const agent = text(input.agent, "agent", 200);
    if (!/^[A-Za-z0-9][A-Za-z0-9@._/-]{0,127}$/.test(agent)) {
      throw new OpError(400, `agent: not an agent name: ${JSON.stringify(agent).slice(0, 80)}`);
    }
    const argv = [this.cfg.fleetBin, "bridge", "mint-credential", "--agent", agent];
    if (input.scopes !== undefined && input.scopes !== null && input.scopes !== "") {
      const scopes = (Array.isArray(input.scopes) ? input.scopes : String(input.scopes).split(","))
        .map((s) => (typeof s === "string" ? s.trim() : s))
        .filter((s) => s !== "");
      if (scopes.length === 0 || scopes.some((s) => s !== "inbox" && s !== "send" && s !== "read")) {
        throw new OpError(400, "scopes: only inbox, send and read exist");
      }
      argv.push("--scopes", scopes.join(","));
    }
    if (input.addr !== undefined && input.addr !== null && input.addr !== "") {
      const addr = text(input.addr, "addr", 200);
      if (addr !== `service:${agent}`) {
        throw new OpError(400, "addr: a service credential is bound to its own name (service:<agent>)");
      }
      argv.push("--addr", addr);
    }
    const out = JSON.parse(await this.must(argv)) as {
      agent?: unknown;
      scopes?: unknown;
      addr?: unknown;
      credential?: unknown;
    } | null;
    if (typeof out?.credential !== "string" || out.credential === "") {
      throw new OpError(502, "the bridge's mint answer carried no credential");
    }
    return {
      agent: typeof out.agent === "string" ? out.agent : agent,
      scopes: Array.isArray(out.scopes) ? out.scopes.filter((s) => typeof s === "string") : [],
      addr: typeof out.addr === "string" ? out.addr : null,
      credential: out.credential,
    };
  }

  /** Send a message or a command to agents, groups, or everyone.
   * Targets are the bridge's whole grammar (#14375, design §4.1): a
   * name, a glob (agent:f*), a group, or all. `limit` caps how many
   * of the matched agents receive it. The answer is the bridge's own
   * match report — matched, delivered, the recipients by name, and
   * how many matched agents remain — never a console-side recount. */
  async send(to: unknown, body: unknown, command: unknown, limit?: unknown) {
    const targets = bridgeTargets(to);
    const argv = [this.cfg.fleetBin, "bridge", "send", "--to", targets.join(",")];
    if (command === true) argv.push("--command");
    const n = optionalCount(limit, "limit", 1000);
    if (n !== null) argv.push("--limit", String(n));
    // The text goes on stdin, so nothing in it is read as an option.
    argv.push("-");
    const out = JSON.parse(await this.must(argv, { stdin: text(body, "text", 16_000) })) as Record<
      string,
      unknown
    > | null;
    return matchReport(out, targets, command === true);
  }

  /** Broaden a pattern send (#14375, design §4.1): deliver its
   * original text to the matched agents its limit left over. One
   * bridge call with the send's batch — a follow-up send, never a
   * fresh fan-out, so nobody is delivered to twice. The answer is
   * the same match report, updated: delivered counts this call,
   * remaining what is still left. */
  async broaden(batch: unknown, limit?: unknown) {
    const b = text(batch, "batch", 128);
    if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(b)) {
      throw new OpError(400, `batch: not a batch id: ${JSON.stringify(b).slice(0, 80)}`);
    }
    const argv = [this.cfg.fleetBin, "bridge", "send", "--broaden", b];
    const n = optionalCount(limit, "limit", 1000);
    if (n !== null) argv.push("--limit", String(n));
    const out = JSON.parse(await this.must(argv)) as Record<string, unknown> | null;
    return matchReport(out, [], false);
  }

  /** Post the text as a claim offer instead of delivering it
   * (#14375, design §4.2): the next `slots` eligible agents to claim
   * it are delivered a row on the offer's batch, and their replies
   * thread onto that batch. The answer is the bridge's offer row. */
  async offer(to: unknown, body: unknown, command: unknown, slots: unknown, ttlSeconds?: unknown) {
    const targets = bridgeTargets(to);
    const argv = [
      this.cfg.fleetBin,
      "bridge",
      "offer",
      "--to",
      targets.join(","),
      "--slots",
      String(id(slots, "slots")),
    ];
    if (command === true) argv.push("--command");
    const ttl = optionalCount(ttlSeconds, "ttl", 7 * 24 * 3600);
    if (ttl !== null) argv.push("--ttl", String(ttl));
    argv.push("-");
    const out = JSON.parse(await this.must(argv, { stdin: text(body, "text", 16_000) })) as { offer?: unknown } | null;
    if (!out || typeof out !== "object" || !out.offer || typeof out.offer !== "object") {
      throw new OpError(502, "the bridge did not say what became of the offer");
    }
    return out.offer as Record<string, unknown>;
  }

  /** Every offer with its claims (#14375): the composer's offer rows
   * read through `fleet bridge offers` — who claimed, when, and the
   * row each claim delivered. Claimants' replies are pipeline rows
   * on the offer's batch, read with the feed. */
  async offers() {
    const out = JSON.parse(await this.must([this.cfg.fleetBin, "bridge", "offers"])) as { offers?: unknown } | null;
    return Array.isArray(out?.offers) ? out.offers : [];
  }

  // ---- sign-in ------------------------------------------------------------

  /** The script that keeps the public address as declared. It is the one
   * place that talks to the sign-in gate, so sign-in is read and changed
   * through it. */
  private async expose(args: string[], stdin?: string) {
    // Resolved here, not at load: the edge imports this module and never
    // calls this, and import.meta.dir is the laptop's.
    const { join } = await import("node:path");
    const script = join(import.meta.dir, "..", "scripts", "expose.py");
    return this.run(["python3", script, ...args], { stdin, timeoutMs: 120_000 });
  }

  /** Which sign-in methods are on, and what Google has to be told to
   * let this gate use it. */
  async signin() {
    const r = await this.expose(["--status"]);
    let status: { google?: unknown; pin?: unknown; team?: unknown } | null = null;
    try {
      status = JSON.parse(r.stdout.trim().split("\n").pop() ?? "");
    } catch {
      status = null;
    }
    if (r.code !== 0 || !status || typeof status.team !== "string" || !/^[a-z0-9.-]+$/.test(status.team)) {
      throw new OpError(502, "could not read the sign-in settings from Cloudflare");
    }
    return {
      google: status.google === true,
      pin: status.pin === true,
      // What goes into the Google client: where sign-ins come from, and
      // where Google sends them back.
      origin: `https://${status.team}`,
      redirect: `https://${status.team}/cdn-cgi/access/callback`,
    };
  }

  /** Turn Google sign-in on with an OAuth client the owner made. The two
   * values go to the script on its standard input and from there to
   * Cloudflare; they are not kept here, logged, or returned. */
  async googleSignin(clientId: unknown, clientSecret: unknown) {
    const id = text(clientId, "client ID", 200);
    const value = text(clientSecret, "client secret", 200);
    if (!/^[0-9]+-[0-9a-z]+\.apps\.googleusercontent\.com$/.test(id)) {
      throw new OpError(400, "client ID: one from Google ends in .apps.googleusercontent.com");
    }
    if (!/^[\x21-\x7e]{8,200}$/.test(value)) throw new OpError(400, "client secret: not a client secret");
    const r = await this.expose(["--google-stdin"], `${id}\n${value}\n`);
    // What the script prints is never one of the two values, but nothing
    // is passed on that could hold one.
    const said = `${r.stdout}\n${r.stderr}`
      .split("\n")
      .map((line) => line.trim())
      .filter((line) => line && !line.includes(value) && !line.includes(id));
    if (r.code !== 0) {
      throw new OpError(
        502,
        `Google sign-in was not turned on: ${(said.pop() ?? "the setup script failed").slice(0, 300)}`,
      );
    }
    return { google: true, changed: said.filter((line) => line.startsWith("changed: ")).map((line) => line.slice(9)) };
  }

  /** Persist the desired direct-mode state in the repository, then ask
   * Actions to apply it. Token rotation is explicit because Worker
   * secret values cannot be read back to determine whether they changed. */
  async edgeDirect(on: unknown, refreshTokens: unknown = false) {
    if (typeof on !== "boolean") throw new OpError(400, "on: true or false");
    if (typeof refreshTokens !== "boolean") throw new OpError(400, "refreshTokens: true or false");
    if (refreshTokens && !on) throw new OpError(400, "refreshTokens requires direct mode on");
    const file = `repos/${this.cfg.repo}/contents/infra/pm-cloudflare/state.json`;
    const current = (await this.gh([file])) as { sha?: unknown; content?: unknown } | null;
    if (typeof current?.sha !== "string" || typeof current.content !== "string") {
      throw new OpError(502, "could not read the Cloudflare repository state");
    }
    let state: Record<string, unknown>;
    try {
      const binary = atob(current.content.replace(/\s/g, ""));
      state = JSON.parse(new TextDecoder().decode(Uint8Array.from(binary, (c) => c.charCodeAt(0)))) as Record<
        string,
        unknown
      >;
    } catch {
      throw new OpError(502, "the Cloudflare repository state was not valid JSON");
    }
    const worker = state.worker as Record<string, unknown> | undefined;
    if (!worker || (worker.direct !== "on" && worker.direct !== "off")) {
      throw new OpError(502, "the Cloudflare repository state has no valid worker.direct setting");
    }
    worker.direct = on ? "on" : "off";
    const bytes = new TextEncoder().encode(`${JSON.stringify(state, null, 2)}\n`);
    let binary = "";
    for (const byte of bytes) binary += String.fromCharCode(byte);
    await this.gh(["-X", "PUT", file], {
      message: `pm-console: ${on ? "enable" : "disable"} edge direct mode`,
      sha: current.sha,
      content: btoa(binary),
      branch: "main",
    });
    await this.gh(["-X", "POST", `repos/${this.cfg.repo}/actions/workflows/pm-cloudflare-config.yml/dispatches`], {
      ref: "main",
      inputs: { mode: "apply", refresh_tokens: String(refreshTokens) },
    });
    return { on, started: "pm-cloudflare-config.yml" };
  }

  /** Where the repository-described Cloudflare configuration was last
   * successfully applied from, per the apply workflow's runs on main.
   * A read only: it dispatches nothing, changes no Cloudflare state,
   * and returns no secret value — only the run's source, commit,
   * completion time and page. `apply` is null when no successful apply
   * is on record, which the screen says as provenance unavailable. */
  async cloudflareApply() {
    const got = await this.gh([
      `repos/${this.cfg.repo}/actions/workflows/pm-cloudflare-config.yml/runs?branch=main&event=workflow_dispatch&per_page=10`,
    ]);
    return { apply: latestApply(got) };
  }

  // ---- services ----------------------------------------------------------

  async services(): Promise<Unit[]> {
    const select = ["--plain", "--no-legend", "--type=service", "--type=timer", ...this.cfg.unitPatterns];
    const [loaded, files] = await Promise.all([
      this.must(["systemctl", "--user", "list-units", "--all", ...select]),
      // A stopped unit nothing refers to leaves memory and drops off
      // list-units; without this it could never be started from here.
      this.run(["systemctl", "--user", "list-unit-files", ...select]),
    ]);
    const units = parseUnits(loaded);
    const seen = new Set(units.map((u) => u.unit));
    for (const f of files.code === 0 ? parseUnitFiles(files.stdout) : []) {
      if (!seen.has(f.unit))
        units.push({
          unit: f.unit,
          load: "not-loaded",
          active: "inactive",
          sub: "dead",
          description: `not loaded (${f.state})`,
        });
    }
    return units.filter((u) => manageable(u.unit, this.cfg.unitPatterns)).sort((a, b) => a.unit.localeCompare(b.unit));
  }

  async serviceAct(unit: unknown, action: unknown) {
    if (typeof unit !== "string" || !manageable(unit, this.cfg.unitPatterns))
      throw new OpError(404, "not a unit this console manages");
    if (!UNIT_ACTIONS.includes(action as UnitAction))
      throw new OpError(400, `action: one of ${UNIT_ACTIONS.join(", ")}`);
    if (action === "stop" && this.cfg.protectedUnits.includes(unit)) {
      throw new OpError(409, `${unit} carries this interface: it can be restarted from here, not stopped`);
    }
    await this.must(["systemctl", "--user", action as string, unit], { timeoutMs: 60_000 });
    return { unit, action };
  }

  async serviceLogs(unit: unknown, lines: unknown) {
    if (typeof unit !== "string" || !manageable(unit, this.cfg.unitPatterns))
      throw new OpError(404, "not a unit this console manages");
    const n = lines === undefined || lines === null || lines === "" ? 80 : Math.min(id(lines, "lines"), 400);
    return {
      unit,
      log: await this.must(["journalctl", "--user", "-u", unit, "-n", String(n), "--no-pager", "-o", "short-iso"]),
    };
  }

  // ---- pull requests -----------------------------------------------------

  async prs() {
    const list = (await this.gh([`repos/${this.cfg.repo}/pulls?state=open&per_page=60`])) as {
      number: number;
      title: string;
      draft: boolean;
      html_url: string;
      created_at: string;
      user: { login: string };
      head: { ref: string; sha?: string };
    }[];
    return list.map((p) => ({
      number: p.number,
      title: p.title,
      draft: p.draft,
      url: p.html_url,
      created: p.created_at.slice(0, 16),
      author: p.user.login,
      branch: p.head.ref,
      // The commit it stood at when this was read: a merge asked for
      // from this list is a merge of this, and of nothing pushed since.
      ...(typeof p.head.sha === "string" ? { head: p.head.sha } : {}),
    }));
  }

  async pr(number: unknown) {
    const n = id(number, "pull request");
    const p = (await this.gh([`repos/${this.cfg.repo}/pulls/${n}`])) as Record<string, unknown>;
    return {
      number: n,
      title: p.title,
      state: p.state,
      draft: p.draft,
      merged: p.merged,
      mergeable: p.mergeable,
      mergeable_state: p.mergeable_state,
      additions: p.additions,
      deletions: p.deletions,
      changed_files: p.changed_files,
      review_comments: p.review_comments,
      comments: p.comments,
      url: p.html_url,
      body: typeof p.body === "string" ? p.body.slice(0, 4000) : "",
    };
  }

  /** Merge a pull request. With `head`, the commit the owner was shown,
   * GitHub merges only if that is still where it stands: a merge asked
   * for a while ago (kept on a device that was offline) must not take
   * in what was pushed since. */
  async mergePr(number: unknown, head?: unknown) {
    const n = id(number, "pull request");
    if (head !== undefined && head !== null && !(typeof head === "string" && /^[0-9a-f]{40}$/.test(head))) {
      throw new OpError(400, "head: a commit's full name");
    }
    const done = (await this.gh([
      "-X",
      "PUT",
      `repos/${this.cfg.repo}/pulls/${n}/merge`,
      "-f",
      "merge_method=merge",
      ...(typeof head === "string" ? ["-f", `sha=${head}`] : []),
    ])) as { merged: boolean; sha: string };
    return { number: n, merged: done.merged, sha: done.sha };
  }

  // ---- deploys and releases ---------------------------------------------

  /** What GitHub's own record says of the site's deploys and the app's
   * builds: the last few runs of each on main, newest first. One that
   * could not be read says so by itself; the other still shows. */
  /** Every Worker the repository holds, as its own list says
   * (pm-stack/platform/workers.json, read from the repository so that
   * a merge is all it takes to change it), and for each one deployed
   * from Actions the latest run of that deploy on main. A run that
   * could not be read says so by itself; the list still shows. */
  async platform() {
    const got = (await this.gh([`repos/${this.cfg.repo}/contents/${WORKERS_FILE}`])) as { content?: string } | null;
    if (typeof got?.content !== "string") throw new OpError(502, "the list of Workers could not be read");
    let parsed: unknown;
    try {
      // GitHub wraps the encoded file in lines; decoding passes over them.
      parsed = JSON.parse(new TextDecoder().decode(Uint8Array.from(atob(got.content), (c) => c.charCodeAt(0))));
    } catch {
      throw new OpError(502, "the list of Workers is not valid JSON");
    }
    const list = workersList(parsed);
    const flows = [...new Set(list.workers.map((w) => w.workflow).filter((f): f is string => f !== null))];
    const last = async (workflow: string) => {
      const found = (await this.gh([
        `repos/${this.cfg.repo}/actions/workflows/${workflow}/runs?branch=main&per_page=1`,
      ])) as {
        workflow_runs?: {
          head_sha: string;
          status: string;
          conclusion: string | null;
          created_at: string;
          html_url: string;
        }[];
      } | null;
      const run = found?.workflow_runs?.[0];
      return run
        ? {
            commit: run.head_sha.slice(0, 12),
            state: run.status === "completed" ? (run.conclusion ?? "unknown") : run.status,
            at: run.created_at,
            url: run.html_url,
          }
        : null;
    };
    const runs = new Map(
      await Promise.all(
        flows.map(async (flow) => [flow, await last(flow).catch((e: Error) => ({ error: e.message }))] as const),
      ),
    );
    return {
      checked: list.checked,
      malformed: list.malformed,
      workers: list.workers.map((w) => ({ ...w, run: w.workflow ? (runs.get(w.workflow) ?? null) : null })),
    };
  }

  /** When each store token the platform holds runs out (#14255): the
   * tokens the repository's list of stores names
   * (pm-stack/platform/stores.json), each fetched by name the way the
   * queue token is and read for its `exp` claim. Only the name and the
   * date leave here; a token that could not be fetched says only that. */
  async tokenExpiries(now: number = Date.now()): Promise<{ checkedAt: string; tokens: TokenExpiry[] }> {
    const got = (await this.gh([`repos/${this.cfg.repo}/contents/${STORES_FILE}`])) as { content?: string } | null;
    if (typeof got?.content !== "string") throw new OpError(502, "the list of stores could not be read");
    let parsed: unknown;
    try {
      parsed = JSON.parse(new TextDecoder().decode(Uint8Array.from(atob(got.content), (c) => c.charCodeAt(0))));
    } catch {
      throw new OpError(502, "the list of stores is not valid JSON");
    }
    const held = heldTokens(storesList(parsed).stores);
    const tokens = await Promise.all(
      held.map(async (token) => {
        const kept = this.expiries.get(token.name);
        if (kept && now - kept.at < TOKEN_EXPIRY_KEPT_MS) return tokenExpiry(token, kept.expiresAt, now);
        let expiresAt: number | null | undefined;
        try {
          const value =
            token.name === this.cfg.queueTokenName && this.queueToken !== null
              ? this.queueToken
              : (await this.must([this.cfg.fleetBin, "secrets", "get", token.name])).trim();
          expiresAt = jwtExpiry(value);
        } catch {
          // Why it could not be fetched is not said: what the fleet
          // binary prints about a secret stays on the machine.
          expiresAt = undefined;
        }
        this.expiries.set(token.name, { at: now, expiresAt });
        return tokenExpiry(token, expiresAt, now);
      }),
    );
    return { checkedAt: new Date(now).toISOString(), tokens };
  }

  async releases() {
    const runs = async (workflow: string) => {
      const got = (await this.gh([
        `repos/${this.cfg.repo}/actions/workflows/${workflow}/runs?branch=main&per_page=5`,
      ])) as {
        workflow_runs?: {
          id: number;
          head_sha: string;
          status: string;
          conclusion: string | null;
          event: string;
          created_at: string;
          html_url: string;
        }[];
      } | null;
      return (got?.workflow_runs ?? []).map((r) => ({
        id: r.id,
        commit: r.head_sha.slice(0, 12),
        // Once it is over, how it ended; before that, where it is.
        state: r.status === "completed" ? (r.conclusion ?? "unknown") : r.status,
        byHand: r.event === "workflow_dispatch",
        at: r.created_at,
        url: r.html_url,
      }));
    };
    const part = async <T>(p: Promise<T>): Promise<T | { error: string }> =>
      p.catch((e: Error) => ({ error: e.message }));
    const [site, app] = await Promise.all([part(runs(DEPLOYS.site)), part(runs(DEPLOYS.app))]);
    return { site, app };
  }

  /** Start the site's deploy, or the app's build, again from main: the
   * same workflow a merge starts. `scratch` asks the site's deploy to
   * first stand a throwaway copy up from nothing and remove it. */
  async deploy(what: unknown, scratch: unknown) {
    if (what !== "site" && what !== "app") throw new OpError(400, "what: site or app");
    const fromNothing = what === "site" && scratch === true;
    await this.gh(["-X", "POST", `repos/${this.cfg.repo}/actions/workflows/${DEPLOYS[what]}/dispatches`], {
      ref: "main",
      // GitHub takes a workflow's inputs as text.
      ...(fromNothing ? { inputs: { scratch: "true" } } : {}),
    });
    return { started: what, scratch: fromNothing };
  }

  /** Start a Turso management operation (#14257): create a database or
   * mint a token for one. The operation runs in the pm-turso-admin
   * workflow, which alone holds the organisation's token (by secret
   * name, in the runner only); this console never sees a token value —
   * it dispatches names and choices, like deploy() above. */
  async tursoAdmin(args: Record<string, unknown>) {
    const req = tursoAdminRequest(args);
    await this.gh(["-X", "POST", `repos/${this.cfg.repo}/actions/workflows/${TURSO_ADMIN_WORKFLOW}/dispatches`], {
      ref: "main",
      inputs: tursoAdminInputs(req),
    });
    return { started: req.operation, database: req.database };
  }

  /** The last pm-turso-admin run, for the Stores section: its outcome
   * as GitHub records it, with a failure kept a failure (#14257). */
  async tursoAdminLastRun() {
    const found = (await this.gh([
      `repos/${this.cfg.repo}/actions/workflows/${TURSO_ADMIN_WORKFLOW}/runs?per_page=1`,
    ])) as {
      workflow_runs?: { status: string; conclusion: string | null; created_at: string; html_url: string }[];
    } | null;
    const run = found?.workflow_runs?.[0];
    return run ? tursoAdminRun(run) : null;
  }

  // ---- one screen --------------------------------------------------------

  async overview() {
    const part = async <T>(p: Promise<T>): Promise<T | { error: string }> =>
      p.catch((e: Error) => ({ error: e.message }));
    const [queue, units, prs, agents] = await Promise.all([
      part(this.queueCounts()),
      part(this.services()),
      part(this.prs()),
      part(this.agents()),
    ]);
    return {
      queue,
      services: Array.isArray(units)
        ? {
            total: units.length,
            failed: units.filter((u) => u.active === "failed").map((u) => u.unit),
            inactive: units.filter((u) => u.unit.endsWith(".service") && u.active === "inactive").length,
          }
        : units,
      prs: Array.isArray(prs) ? { open: prs.length, ready: prs.filter((p) => !p.draft).length } : prs,
      agents: Array.isArray(agents) ? { registered: agents.length } : agents,
    };
  }
}
