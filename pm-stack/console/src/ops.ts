// What the console can do. Every operation is one of the verbs the
// fleet already has: the queue through `fleet queue` and its store,
// messages through `fleet bridge`, GitHub through `fleet gh`, and
// services through `systemctl --user`. Nothing here builds a shell
// command line: each call is an argument vector, and text a person
// typed travels on stdin or as a single argument.
import type { Config } from "./config.ts";

export interface RunResult {
  code: number;
  stdout: string;
  stderr: string;
}
export type Run = (argv: string[], opts?: { stdin?: string; timeoutMs?: number }) => Promise<RunResult>;
export type Fetch = (
  url: string,
  init: { method: string; headers: Record<string, string>; body: string },
) => Promise<{ ok: boolean; status: number; json(): Promise<unknown> }>;

export class OpError extends Error {
  constructor(
    public status: 400 | 404 | 409 | 502,
    message: string,
  ) {
    super(message);
  }
}

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
export const QUEUE_STATUSES = ["queued", "claimed", "blocked", "parked"] as const;
const UNIT_NAME = /^[A-Za-z0-9@._-]+\.(service|timer)$/;
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

export class Ops {
  private queueToken: string | null = null;

  constructor(
    private cfg: Config,
    private run: Run = realRun,
    private fetchImpl: Fetch = fetch as unknown as Fetch,
  ) {}

  private async must(argv: string[], opts?: { stdin?: string; timeoutMs?: number }): Promise<string> {
    const r = await this.run(argv, opts);
    if (r.code !== 0) {
      const detail = (r.stderr || r.stdout).trim().split("\n").slice(-3).join(" ").slice(0, 400);
      throw new OpError(502, `${argv[0]} ${argv[1] ?? ""} failed (exit ${r.code}): ${detail}`);
    }
    return r.stdout;
  }

  private async gh(args: string[], body?: unknown): Promise<unknown> {
    const argv = [this.cfg.fleetBin, "gh", "api", ...args];
    const out = await this.must(
      body === undefined ? argv : [...argv, "--input", "-"],
      body === undefined ? undefined : { stdin: JSON.stringify(body) },
    );
    return out.trim() === "" ? null : JSON.parse(out);
  }

  // ---- queue -----------------------------------------------------------

  private async sql(statement: string, args: (string | number)[]): Promise<Record<string, string | number | null>[]> {
    if (!/^\s*select\b/i.test(statement)) throw new Error("the console only reads the queue store");
    if (this.queueToken === null) {
      this.queueToken = (await this.must([this.cfg.fleetBin, "secrets", "get", this.cfg.queueTokenName])).trim();
    }
    const body = {
      requests: [
        {
          type: "execute",
          stmt: {
            sql: statement,
            args: args.map((a) =>
              typeof a === "number" ? { type: "integer", value: String(a) } : { type: "text", value: a },
            ),
          },
        },
        { type: "close" },
      ],
    };
    const res = await this.fetchImpl(`${this.cfg.queueDbUrl}/v2/pipeline`, {
      method: "POST",
      headers: { Authorization: `Bearer ${this.queueToken}`, "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
    if (!res.ok) throw new OpError(502, `queue store answered http ${res.status}`);
    const first = (
      (await res.json()) as {
        results?: {
          type?: string;
          response?: { result?: { cols: { name: string }[]; rows: { value?: string | number | null }[][] } };
        }[];
      }
    ).results?.[0];
    const result = first?.type === "ok" ? first.response?.result : undefined;
    if (!result) throw new OpError(502, "queue store refused the query");
    return result.rows.map((row) => Object.fromEntries(result.cols.map((c, i) => [c.name, row[i]?.value ?? null])));
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
      "SELECT id, status, priority, title, dedupe_key, substr(updated_at,1,16) AS updated, substr(coalesce(json_extract(progress,'$.reason'), json_extract(progress,'$.blocker'), ready_condition, ''),1,300) AS why FROM tasks WHERE queue = ? AND status = ? AND title LIKE ? ESCAPE '\\' ORDER BY (priority IS NULL), priority ASC, created_at ASC, id ASC LIMIT ?",
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
    const found = (await this.gh(["-X", "GET", "search/issues", "-f", `q=${q}`, "-f", "per_page=100"])) as {
      items: {
        number: number;
        title: string;
        html_url: string;
        updated_at: string;
        comments: number;
        labels: { name: string }[];
      }[];
    };
    return found.items
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
  }

  async setRank(issue: unknown, rank: unknown) {
    const n = id(issue, "issue");
    if (rank !== "none" && !RANKS.includes(rank as (typeof RANKS)[number]))
      throw new OpError(400, `rank: one of ${RANKS.join(", ")}, none`);
    const current = (
      (await this.gh([`repos/${this.cfg.repo}/issues/${n}`])) as { labels: { name: string }[] }
    ).labels.map((l) => l.name);
    for (const label of RANKS) {
      if (label !== rank && current.includes(label)) {
        await this.gh(["-X", "DELETE", `repos/${this.cfg.repo}/issues/${n}/labels/${encodeURIComponent(label)}`]);
      }
    }
    if (rank !== "none" && !current.includes(rank as string)) {
      await this.gh(["-X", "POST", `repos/${this.cfg.repo}/issues/${n}/labels`], { labels: [rank] });
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

  /** File an idea as an issue, ranked so the intake ladder reaches it. */
  async fileIdea(title: unknown, body: unknown, rank: unknown) {
    if (rank !== "none" && !RANKS.includes(rank as (typeof RANKS)[number]))
      throw new OpError(400, `rank: one of ${RANKS.join(", ")}, none`);
    const made = (await this.gh(["-X", "POST", `repos/${this.cfg.repo}/issues`], {
      title: text(title, "title", 200),
      body: typeof body === "string" ? body.slice(0, 60_000) : "",
      labels: rank === "none" ? [] : [rank],
    })) as { number: number; html_url: string };
    return { issue: made.number, url: made.html_url };
  }

  // ---- agents ------------------------------------------------------------

  async agents() {
    return (JSON.parse(await this.must([this.cfg.fleetBin, "bridge", "agents"])) as { agents: unknown[] }).agents;
  }

  async messages(limit: unknown) {
    const n = limit === undefined || limit === null || limit === "" ? 30 : Math.min(id(limit, "limit"), 200);
    return (
      JSON.parse(await this.must([this.cfg.fleetBin, "bridge", "log", "--limit", String(n), "--order", "desc"])) as {
        messages: unknown[];
      }
    ).messages;
  }

  /** Send a message or a command to agents, groups, or everyone. */
  async send(to: unknown, body: unknown, command: unknown) {
    if (!Array.isArray(to) || to.length === 0 || to.length > 50) throw new OpError(400, "to: one to fifty targets");
    const targets = to.map((t) => {
      if (typeof t !== "string" || !/^(all|(agent:|group:)?[A-Za-z0-9][A-Za-z0-9._-]{0,63})$/.test(t))
        throw new OpError(400, `to: not a target: ${JSON.stringify(t).slice(0, 80)}`);
      return t;
    });
    const argv = [this.cfg.fleetBin, "bridge", "send", "--to", targets.join(",")];
    if (command === true) argv.push("--command");
    // The text goes on stdin, so nothing in it is read as an option.
    argv.push("-");
    await this.must(argv, { stdin: text(body, "text", 16_000) });
    return { to: targets, command: command === true };
  }

  // ---- services ----------------------------------------------------------

  async services(): Promise<Unit[]> {
    const out = await this.must([
      "systemctl",
      "--user",
      "list-units",
      "--all",
      "--plain",
      "--no-legend",
      "--type=service",
      "--type=timer",
      ...this.cfg.unitPatterns,
    ]);
    return parseUnits(out).filter((u) => manageable(u.unit, this.cfg.unitPatterns));
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
      head: { ref: string };
    }[];
    return list.map((p) => ({
      number: p.number,
      title: p.title,
      draft: p.draft,
      url: p.html_url,
      created: p.created_at.slice(0, 16),
      author: p.user.login,
      branch: p.head.ref,
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

  async mergePr(number: unknown) {
    const n = id(number, "pull request");
    const done = (await this.gh([
      "-X",
      "PUT",
      `repos/${this.cfg.repo}/pulls/${n}/merge`,
      "-f",
      "merge_method=merge",
    ])) as { merged: boolean; sha: string };
    return { number: n, merged: done.merged, sha: done.sha };
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
