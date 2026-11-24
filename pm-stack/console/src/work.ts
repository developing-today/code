// What the owner looks at first: the work in progress, what comes next,
// what is waiting on him, and what he has asked for.
//
// Reads come from the queue store, the dashboard's own data and GitHub.
// Every write is something the fleet already understands: an answer to a
// decision set through the dashboard's own endpoint, an issue and a
// queue task, a comment, a label.
import { type Census, censusOf, type NeedsItem, readCensus } from "./needs.ts";
import {
  category,
  claimPriority,
  DEFAULT_OPTIONS,
  fileTask,
  type FilingOptions,
  filingOptions,
  IMPORTANCES,
  NeedsBroker,
  ownerBlock,
  ownerLines,
  URGENCIES,
} from "./filing.ts";
import { OpError, type Ops, PRIORITY_SCALE, type Row } from "./ops.ts";
import { timed } from "./timed.ts";

const P = "CASE WHEN json_valid(payload) THEN payload ELSE '{}' END";
const G = "CASE WHEN json_valid(progress) THEN progress ELSE '{}' END";
/** A task's condition for being ready, where it is JSON; `{}` where it
 * is words, or nothing. */
const R = "CASE WHEN json_valid(ready_condition) THEN ready_condition ELSE '{}' END";
/** How many tasks of a queue are blocked, by the kind of reason: one
 * written on the task (each is its own text, so the texts are not
 * counted: the commonest six once said fifteen of 272), a time waited
 * for and whether it has passed, other tasks waited on, or nothing
 * recorded. Every blocked task is counted once, in the order the
 * reason shown for it is chosen (reason, blocker, condition, tasks). */
export const BLOCKED_KINDS_SQL = `SELECT CASE
  WHEN coalesce(json_extract(${G},'$.reason'), json_extract(${G},'$.blocker')) IS NOT NULL THEN 'reason'
  WHEN json_extract(${R},'$.kind') = 'time' THEN
    CASE WHEN substr(json_extract(${R},'$.until'),1,19) <= strftime('%Y-%m-%dT%H:%M:%S','now') THEN 'time-past' ELSE 'time' END
  WHEN coalesce(ready_condition,'') <> '' THEN 'reason'
  WHEN coalesce(blocked_deps,'') NOT IN ('','[]') THEN 'tasks'
  ELSE 'none' END AS kind, count(*) AS n
  FROM tasks WHERE queue = ? AND status = 'blocked' GROUP BY 1 ORDER BY n DESC`;
const RES = "CASE WHEN json_valid(result) THEN result ELSE '{}' END";
/** How many of each kind of finished thing are shown. */
const DONE_SHOWN = 40;
/** The events that say a task was finished: by whoever did it, or by
 * the queue's own sweep finding it already done. */
const FINISHED = "('completed','staleness_autocomplete')";
const DONE_FROM = `FROM events e CROSS JOIN tasks t ON t.id = e.task_id
  WHERE e.kind IN ${FINISHED} AND e.created_at >= ? AND t.queue = ? AND t.status = 'done'`;
/** Tasks finished since a time, newest first: each once, at its last
 * finishing, and only while it still stands as done (one taken up again
 * is not). They are found by the events that say so, which the store
 * keeps in order of time; the tasks themselves it does not, and there
 * are tens of thousands of them. (CROSS JOIN is SQLite's way of being
 * told to start from the events.) */
export const DONE_TASKS_SQL = `SELECT t.id AS id, t.title AS title, substr(max(e.created_at),1,19) AS at,
  t.lane_id AS lane, t.claim_model AS model,
  json_extract(${P},'$.kind') AS kind, json_extract(${P},'$.issue.number') AS issue,
  coalesce(json_extract(${RES},'$.disposition.pulls[0].number'), json_extract(${RES},'$.pr'),
    json_extract(${P},'$.pr.number')) AS pr,
  json_extract(${RES},'$.disposition.verdict') AS verdict
  ${DONE_FROM}
  GROUP BY t.id ORDER BY max(e.created_at) DESC, t.id DESC LIMIT ${DONE_SHOWN}`;
/** How many tasks were finished since a time, each counted once. */
export const DONE_COUNT_SQL = `SELECT count(DISTINCT t.id) AS n ${DONE_FROM}`;
/** The same tasks by their kind, the most first: what the day went on. */
export const DONE_KINDS_SQL = `SELECT coalesce(json_extract(${P},'$.kind'),'') AS kind, count(DISTINCT t.id) AS n
  ${DONE_FROM}
  GROUP BY 1 ORDER BY n DESC, kind ASC LIMIT 12`;
/** One page of what GitHub lists as closed, most lately touched first. */
const CLOSED_PAGE = 100;
const DAY_MS = 24 * 3_600_000;
/** How far back "lately" goes. */
const DONE_SINCE_MS = 7 * DAY_MS;
/** A time as the queue and GitHub both write it. */
const stamp = (ms: number) => new Date(ms).toISOString().replace(/\.\d+Z$/, "Z");
/** A pull request's number, from a number or from its address. */
function pullNumber(value: unknown): number | null {
  if (typeof value === "number") return Number.isSafeInteger(value) && value > 0 ? value : null;
  if (typeof value !== "string") return null;
  const found = /(?:\/pull\/|^#?)([1-9][0-9]{0,8})$/.exec(value.trim());
  return found ? Number(found[1]) : null;
}

const TASK_COLS = `id, title, priority, status, attempts,
  substr(coalesce(claimed_at,''),1,19) AS claimed, substr(coalesce(lease_until,''),1,19) AS lease,
  substr(created_at,1,19) AS created, worker_id AS worker, lane_id AS lane, claim_model AS model,
  json_extract(${P},'$.kind') AS kind, json_extract(${P},'$.issue.number') AS issue,
  json_extract(${P},'$.pr.number') AS pr,
  substr(coalesce(json_extract(${G},'$.next'), json_extract(${G},'$.last_checkpoint.progress'),
    json_extract(${G},'$.last_checkpoint.stage'), ''),1,400) AS step,
  substr(coalesce(json_extract(${P},'$.acceptance[0]'),''),1,300) AS goal`;

const OWNER_PROMPT_MARK = "<!-- owner-prompt:v1 -->";
const OWNER_REPLY_MARK = "<!-- owner-reply:v1 -->";
/** What the owner says he is sending: something to do, a question to
 * answer, or how he wants work done from now on. */
export const ASK_INTENTS = ["task", "question", "directive"] as const;
export type AskIntent = (typeof ASK_INTENTS)[number];
/** How each is titled on its issue, which is also how it is told apart later. */
const ASK_TITLE: Record<AskIntent, string> = {
  task: "Owner: ",
  question: "Owner asks: ",
  directive: "Owner directs: ",
};
/** What the issue tells whoever picks it up. */
const ASK_NOTE: Record<AskIntent, string> = {
  task: "This is an instruction from the owner, in his own words, sent from the PM console. Work out what he wants and do it with the fleet's own verbs; if it is a question, answer it here. Say on this issue what you did, with links, and close it when nothing is left.",
  question:
    "This is a question from the owner, in his own words, sent from the PM console. Answer it here, in plain words, with what the answer rests on: links, numbers, the commit or run you read. Change nothing to answer it: no code, no configuration, no other issue opened or closed. If answering shows something that should be done, say so in the answer and leave it to him to ask for it. Leave this issue open for him to read; he closes it.",
  directive:
    "This is a standing instruction from the owner, in his own words, sent from the PM console. It is not one piece of work: it is how he wants work done from now on. Record it where every agent reads it (AGENTS.md, or the document it points to for the area) by pull request, bring work already queued or in flight in line with it, and say on this issue where it now stands, with links. Close it when that is done.",
};
/** What kind a prompt is, by how its issue is titled. One from before
 * there were kinds, or retitled by hand, is something to do. */
export function askIntentOf(title: unknown): AskIntent {
  const t = typeof title === "string" ? title : "";
  if (t.startsWith(ASK_TITLE.question)) return "question";
  if (t.startsWith(ASK_TITLE.directive)) return "directive";
  return "task";
}
function askIntent(value: unknown): AskIntent {
  if (value === undefined) return "task";
  if (!ASK_INTENTS.includes(value as AskIntent)) throw new OpError(400, `intent: one of ${ASK_INTENTS.join(", ")}`);
  return value as AskIntent;
}
const SET_ID = /^[A-Za-z0-9._:-]{1,160}$/;
const REF = /^([A-Za-z0-9._-]+\/[A-Za-z0-9._-]+)#([1-9][0-9]{0,8})$/;
const LEANS = ["none", "skeptical", "approve"] as const;
const SUBMIT_BATCH = 200;
/** Replies shown in a prompt's thread, and how many pages back are read to find them. */
const REPLIES_SHOWN = 6;
const REPLY_PAGES = 3;

function clip(value: unknown, max: number): string {
  const s = typeof value === "string" ? value : "";
  return s.length > max ? `${s.slice(0, max - 1)}…` : s;
}

function words(value: unknown, name: string, max: number): string {
  if (typeof value !== "string" || value.trim() === "") throw new OpError(400, `${name}: required`);
  if (value.length > max) throw new OpError(400, `${name}: at most ${max} characters`);
  return value.trim();
}

export interface DecisionOption {
  id: string;
  label: string;
  tags?: string[];
}
interface DecisionQuestion {
  qid: string;
  question: string;
  context?: string;
  description?: string;
  mode?: string;
  order?: number;
  recommendedOptionIds?: string[];
  options: DecisionOption[];
  resolution?: unknown;
}
interface DecisionItem {
  ledgerId: string;
  title: string;
  order?: number;
  group?: string;
  links?: string[];
  questions: DecisionQuestion[];
}
interface DecisionSet {
  setId: string;
  title: string;
  token: string;
  createdAt: string;
  items: DecisionItem[];
}
export interface Submission {
  ledgerId: string;
  qid: string;
  kind: "answered" | "deferred";
  optionId?: string;
  optionIds?: string[];
  freeText?: string;
  comment?: string;
  lean?: (typeof LEANS)[number];
}

/** How the task for each kind of prompt differs: what the problem is,
 * what counts as finished, what is in and out of its bounds, and how
 * long it gets. */
const ASK_TASK: Record<
  AskIntent,
  {
    named: string;
    problem: string;
    acceptance: string[];
    tests: string[];
    include: string;
    exclude: string[];
    minutes: number;
  }
> = {
  task: {
    named: "prompt",
    problem:
      "The owner wrote this instruction in his own words from the PM console. It is not a report to classify: it is something he wants done, or a question he wants answered.",
    acceptance: [
      "What the owner asked for is done with the fleet's own verbs, or answered if it is a question",
      "A comment on the issue says in plain words what was done, with links, or what could not be done and why",
      "The issue is closed when nothing is left; work that outlasts this task is filed, enqueued and linked from the issue",
    ],
    tests: ["The issue carries the report comment and is closed, or carries the reason it is still open"],
    include: "Carry out the owner's instruction on",
    exclude: ["Secrets, credentials or purchases"],
    minutes: 45,
  },
  question: {
    named: "question",
    problem:
      "The owner asked this question in his own words from the PM console. He wants an answer, not a change: nothing is to be done about it but to answer it.",
    acceptance: [
      "The question is answered in a comment on the issue, in plain words, with what the answer rests on: links, numbers, the commit or run that was read",
      "Nothing was changed to answer it: no code, no configuration, no other issue opened or closed",
      "The issue is left open for the owner to read; he closes it",
    ],
    tests: ["The issue carries the answer, with what it rests on, and is still open"],
    include: "Answer the owner's question on",
    exclude: ["Changing code, configuration or other issues", "Secrets, credentials or purchases"],
    minutes: 30,
  },
  directive: {
    named: "directive",
    problem:
      "The owner gave this standing instruction in his own words from the PM console. It is not one piece of work: it is how he wants work done from now on.",
    acceptance: [
      "The instruction is recorded where every agent reads it (AGENTS.md, or the document it points to for the area), by a pull request that is merged",
      "Work already queued or in flight that it bears on is brought in line with it: re-ranked, commented on or parked, with links",
      "A comment on the issue says where the instruction now stands and what was changed because of it; the issue is closed when that is done",
    ],
    tests: ["The instruction is in the document agents read, on main, and the issue links to where"],
    include: "Record and apply the owner's standing instruction on",
    exclude: ["Secrets, credentials or purchases"],
    minutes: 60,
  },
};

/** The task that carries out an owner prompt. Top band: the owner asked.
 * It takes the issue's triage key, the one key the queue has for "someone
 * must look at this issue", so an issue never has two tasks: a reply on a
 * prompt whose task is still live joins it, and one whose task has
 * finished gets a new one. What kind of prompt it is changes what the
 * task asks for, not where it stands in the queue. */
/** What is said of a filing: a note on one that is in, and on one that
 * is not, why. */
function noteOf(q: Queued): { note?: string } {
  if (q.queued) return q.note ? { note: q.note } : {};
  return { note: `filed, not queued: ${q.note || "the queue did not take it"}` };
}

/** How a prompt's task went: in the queue (and which task), or not yet;
 * `note` says what else there is to know, or why it is not in. */
export interface Queued {
  queued: boolean;
  task?: number;
  status?: string;
  priority?: number | null;
  note?: string | null;
}

export function promptPacket(
  repo: string,
  queue: string,
  number: number,
  text: string,
  at: string,
  intent: AskIntent = "task",
  options: FilingOptions = DEFAULT_OPTIONS,
  /** The labels the issue was given, carried so the queue need not
   * look them up (and so the priority is the one chosen). */
  labels: string[] | null = null,
  /** A reply on a prompt whose task may be done: it is filed again,
   * on purpose, rather than taken as the same work. */
  reply = false,
) {
  const url = `https://github.com/${repo}/issues/${number}`;
  const first = text.split("\n")[0].trim();
  const as = ASK_TASK[intent];
  const priority = claimPriority(options.urgency, options.importance);
  const u = URGENCIES.find((x) => x[0] === options.urgency)?.[1];
  const i = IMPORTANCES.find((x) => x[0] === options.importance)?.[1];
  return {
    title: `Owner ${as.named} ${repo}#${number}: ${first}`.slice(0, 180),
    body: `Instruction from the owner, sent from the PM console: ${url}`,
    priority,
    dedupe_key: `repo-health:v1:triage:${repo}:issue:${number}`,
    payload: {
      schema: "repo-task/v1",
      kind: "triage",
      repo,
      queue,
      issue: { number, url, title: `${ASK_TITLE[intent]}${first}`.slice(0, 120), state_checked_at: at },
      ...(labels ? { labels } : {}),
      owner: ownerBlock(options, priority),
      ...(reply ? { dedupe_override: `the owner replied at ${at}` } : {}),
      problem: as.problem,
      evidence: [`Sent from the PM console at ${at}`, url],
      acceptance: as.acceptance,
      tests: as.tests,
      scope: {
        include: [`${as.include} ${repo}#${number}`],
        exclude: as.exclude,
      },
      risk: { level: "low", flags: ["owner-prompt"] },
      priority_reason: `Owner prompt from the console: urgency ${u}, importance ${i} (${priority}).`,
      timebox_minutes: as.minutes,
      max_attempts: 1000,
      context_refs: [url],
      dedupe: { checked_at: at, basis: "Created with its issue, or with the owner's reply, in the same pass." },
    },
  };
}

export class Work {
  private replies = new Map<number, { updated: string; replies: { you: boolean; at: string; text: string }[] }>();

  constructor(
    private ops: Ops,
    // The decisions file lives on the laptop: the import is lazy so the
    // edge Worker, which never reads it, can import this module.
    private read: (path: string) => Promise<string> = async (path) =>
      (await import("node:fs/promises")).readFile(path, "utf8"),
    private clock: () => Date = () => new Date(),
  ) {}

  private get repo() {
    return this.ops.cfg.repo;
  }

  private task = (r: Row): Row & { url: string | null } => ({
    ...r,
    url: r.issue ? `https://github.com/${this.repo}/issues/${r.issue}` : null,
  });

  /** In progress and next, in the order the queue will hand work out. */
  async board() {
    const q = this.ops.cfg.queue;
    const [counts, now, next, why] = await this.ops.reads([
      {
        sql: "SELECT status, count(*) AS n FROM tasks WHERE queue = ? AND status IN ('queued','claimed','blocked','parked') GROUP BY status",
        args: [q],
      },
      {
        sql: `SELECT ${TASK_COLS} FROM tasks WHERE queue = ? AND status = 'claimed' ORDER BY claimed_at DESC, id DESC LIMIT 60`,
        args: [q],
      },
      {
        sql: `SELECT ${TASK_COLS} FROM tasks WHERE queue = ? AND status = 'queued' ORDER BY (priority IS NULL), priority ASC, created_at ASC, id ASC LIMIT 60`,
        args: [q],
      },
      { sql: BLOCKED_KINDS_SQL, args: [q] },
    ]);
    return {
      counts: Object.fromEntries(counts.map((r) => [String(r.status), Number(r.n)])),
      now: now.map(this.task),
      next: next.map(this.task),
      blockedKinds: why.map((r) => ({ kind: String(r.kind), n: Number(r.n) })),
    };
  }

  /** What was finished lately: tasks the queue says are done, pull
   * requests merged and issues closed. One round trip to the queue
   * store and one question of GitHub, at an address that is the same
   * from one asking to the next, so an answer that has not changed
   * costs nothing to fetch again. `day` is how many in the last day;
   * `more` says GitHub's page was full of things touched within the
   * day, so there may be more than were counted. */
  async done() {
    const now = this.clock().getTime();
    const day = stamp(now - DAY_MS);
    const since = stamp(now - DONE_SINCE_MS);
    const [finished, counted, kinds] = await this.ops.reads([
      { sql: DONE_TASKS_SQL, args: [since, this.ops.cfg.queue] },
      { sql: DONE_COUNT_SQL, args: [day, this.ops.cfg.queue] },
      { sql: DONE_KINDS_SQL, args: [day, this.ops.cfg.queue] },
    ]);
    const listed = await this.ops.gh([
      `repos/${this.repo}/issues?state=closed&sort=updated&direction=desc&per_page=${CLOSED_PAGE}`,
    ]);
    const page = (Array.isArray(listed) ? listed : []) as (Record<string, unknown> | null)[];
    const merged: { number: number; title: string; url: string; at: string; by: string }[] = [];
    const closed: { number: number; title: string; url: string; at: string; reason: string | null }[] = [];
    for (const item of page) {
      if (item === null || typeof item !== "object") continue;
      const { number, title, html_url: url } = item;
      if (!Number.isSafeInteger(number) || typeof title !== "string" || typeof url !== "string") continue;
      const base = { number: number as number, title: clip(title, 200), url: clip(url, 300) };
      const pull = item.pull_request as { merged_at?: unknown } | null | undefined;
      if (pull && typeof pull === "object") {
        // Closed without merging is not something done.
        if (typeof pull.merged_at !== "string") continue;
        merged.push({
          ...base,
          at: pull.merged_at.slice(0, 19),
          by: clip((item.user as { login?: unknown } | null)?.login, 60),
        });
      } else if (typeof item.closed_at === "string") {
        closed.push({
          ...base,
          at: item.closed_at.slice(0, 19),
          reason: typeof item.state_reason === "string" ? clip(item.state_reason, 40) : null,
        });
      }
    }
    // The page is in the order things were last touched. Full, with its
    // last touched within the day, it may be followed by more closed today.
    const last = page.at(-1);
    const more = page.length >= CLOSED_PAGE && typeof last?.updated_at === "string" && last.updated_at >= day;
    const lately = <T extends { at: string; number: number }>(list: T[]) => {
      const within = list
        .filter((x) => x.at >= since.slice(0, 19))
        .sort((a, b) => (a.at < b.at ? 1 : a.at > b.at ? -1 : b.number - a.number));
      return { day: within.filter((x) => x.at >= day.slice(0, 19)).length, more, latest: within.slice(0, DONE_SHOWN) };
    };
    const whole = (value: unknown) => (Number.isSafeInteger(value) && (value as number) > 0 ? (value as number) : null);
    const said = (value: unknown, max: number) => (typeof value === "string" && value !== "" ? clip(value, max) : null);
    return {
      tasks: {
        day: Number(counted[0]?.n) || 0,
        // By kind, for the same day: what it went on.
        kinds: kinds.map((r) => ({ kind: said(r.kind, 40), n: Number(r.n) || 0 })),
        latest: finished.map((r) => ({
          id: Number(r.id),
          title: clip(r.title, 200),
          at: clip(r.at, 19),
          lane: said(r.lane, 60),
          model: said(r.model, 60),
          kind: said(r.kind, 40),
          issue: whole(r.issue),
          pr: pullNumber(r.pr),
          verdict: said(r.verdict, 40),
          url: whole(r.issue) ? `https://github.com/${this.repo}/issues/${whole(r.issue)}` : null,
          prUrl: pullNumber(r.pr) ? `https://github.com/${this.repo}/pull/${pullNumber(r.pr)}` : null,
        })),
      },
      merged: lately(merged),
      closed: lately(closed),
    };
  }

  private async dash(path: string, body?: unknown): Promise<unknown> {
    const base = this.ops.cfg.dashboardUrl;
    if (base === null) throw new OpError(502, "no dashboard is configured");
    const { res, data } = await timed(20_000, async (signal) => {
      const res = await this.ops.fetchImpl(`${base}${path}`, {
        method: body === undefined ? "GET" : "POST",
        headers: { accept: "application/json", ...(body === undefined ? {} : { "content-type": "application/json" }) },
        ...(body === undefined ? {} : { body: JSON.stringify(body) }),
        signal,
      });
      return { res, data: await res.json().catch(() => null) };
    });
    if (!res.ok) {
      const why = (data as { error?: string } | null)?.error ?? `http ${res.status}`;
      throw new OpError(
        res.status === 400 || res.status === 403 || res.status === 404 ? 400 : 502,
        `dashboard: ${why}`,
      );
    }
    return data;
  }

  /** Agent sessions that are moving, and what the dashboard's own tiers
   * say is waiting on the owner. */
  async sessions() {
    const all = (await this.dash("/api/attention.json")) as Record<string, unknown>[];
    const moving = all
      .filter((e) => e.tier === "in-motion")
      .map((e) => ({
        id: clip(e.sessionId, 80),
        title: clip(e.title, 160),
        agent: clip(e.agent, 60),
        model: clip(e.model, 60),
        where: clip(e.directory, 200).split("/").slice(-2).join("/"),
        last: clip(e.lastActivityAt, 30),
        tools: Number(e.activeToolCalls) || 0,
        messages: Number(e.messageCount) || 0,
      }))
      .sort((a, b) => b.last.localeCompare(a.last));
    const waiting = all
      .filter((e) => e.tier === "needs-drew")
      .map((e) => ({
        id: clip(e.id, 120),
        title: clip(e.title, 300),
        ref: clip(e.ref, 120),
        why: clip(e.drewReason, 300),
      }));
    return { moving, waiting };
  }

  /** Issues and pull requests labelled as waiting on the owner, as the
   * dashboard has censused them. */
  async census(): Promise<Census> {
    const data = (await this.dash("/api/needs-drew.json")) as {
      fetchedAt?: string;
      complete?: boolean;
      items?: (Omit<NeedsItem, "kind"> & { kind: string })[];
    };
    const items = (data.items ?? []).map((i) => ({
      ...i,
      kind: i.kind === "pr" ? ("pr" as const) : ("issue" as const),
    }));
    return censusOf(items, data.fetchedAt ?? null, data.complete !== false);
  }

  /** The same list, read from GitHub by a side that has no dashboard to
   * ask (see needs.ts). */
  async censusDirect(): Promise<Census> {
    try {
      return await readCensus(this.ops.cfg.needsRepos, (path) => this.ops.gh([path]));
    } catch (error) {
      if (error instanceof OpError) throw error;
      throw new OpError(502, `what needs you could not be read: ${(error as Error).message}`);
    }
  }

  private async sets(): Promise<DecisionSet[]> {
    let raw: { sets?: DecisionSet[] };
    try {
      raw = JSON.parse(await this.read(this.ops.cfg.decisionsFile));
    } catch (error) {
      throw new OpError(502, `the questions store could not be read: ${(error as Error).message.slice(0, 120)}`);
    }
    return Array.isArray(raw.sets) ? raw.sets : [];
  }

  /** Every open question, newest set first. The per-set tokens stay here. */
  async questions() {
    const sets = (await this.sets()).sort((a, b) => String(b.createdAt).localeCompare(String(a.createdAt)));
    const open = [];
    const byOrder = (a: { order?: number }, b: { order?: number }) => (a.order ?? 0) - (b.order ?? 0);
    for (const set of sets) {
      for (const item of [...(set.items ?? [])].sort(byOrder)) {
        for (const q of [...(item.questions ?? [])].sort(byOrder)) {
          if (q.resolution) continue;
          open.push({
            set: set.setId,
            setTitle: clip(set.title, 120),
            // To the second, cut and not shortened with a mark: it is a time.
            created: typeof set.createdAt === "string" ? set.createdAt.slice(0, 19) : "",
            ledger: item.ledgerId,
            item: clip(item.title, 200),
            links: (item.links ?? []).slice(0, 3),
            qid: q.qid,
            q: clip(q.question, 600),
            // The list carries a line of background; the rest is read
            // when a question is opened, so 360 questions fit a phone.
            context: clip(q.context, 280),
            more: (q.context ?? "").length > 280 || Boolean(q.description),
            multi: q.mode === "multi",
            recommended: q.recommendedOptionIds ?? [],
            options: (q.options ?? []).map((o) => ({ id: o.id, label: clip(o.label, 240), tags: o.tags ?? [] })),
          });
        }
      }
    }
    return { total: open.length, sets: new Set(open.map((q) => q.set)).size, open };
  }

  /** The full background of one question, read when it is opened. */
  async question(setId: unknown, ledger: unknown, qid: unknown) {
    const set = (await this.sets()).find((s) => s.setId === setId);
    const q = set?.items?.find((i) => i.ledgerId === ledger)?.questions?.find((x) => x.qid === qid);
    if (!q) throw new OpError(404, "that question is not in the store");
    return { context: clip(q.context, 8000), detail: clip(q.description, 8000) };
  }

  /** Answer or hand back questions through the dashboard's own endpoint,
   * so everything downstream of an answer happens as it always has. */
  async submit(setId: unknown, submissions: Submission[]) {
    if (typeof setId !== "string" || !SET_ID.test(setId)) throw new OpError(400, "set: not a decision set id");
    if (submissions.length === 0) throw new OpError(400, "nothing to submit");
    const set = (await this.sets()).find((s) => s.setId === setId);
    if (!set) throw new OpError(404, "that decision set is not in the store");
    const out = (await this.dash(`/api/decisions/${encodeURIComponent(setId)}/submit`, {
      token: set.token,
      submissions,
    })) as { outcomes?: { qid: string; applied: boolean; reason?: string }[] };
    const outcomes = out.outcomes ?? [];
    return {
      set: setId,
      applied: outcomes.filter((o) => o.applied).length,
      refused: outcomes.filter((o) => !o.applied).map((o) => ({ qid: o.qid, why: o.reason ?? "not applied" })),
    };
  }

  async answer(args: Record<string, unknown>) {
    const sub: Submission = {
      ledgerId: words(args.ledger, "ledger", 300),
      qid: words(args.qid, "qid", 300),
      kind: "answered",
    };
    if (typeof args.option === "string" && args.option !== "") sub.optionId = words(args.option, "option", 200);
    if (Array.isArray(args.options) && args.options.length > 0) {
      sub.optionIds = args.options.slice(0, 20).map((o) => words(o, "options", 200));
    }
    if (typeof args.text === "string" && args.text.trim() !== "") sub.freeText = words(args.text, "text", 4000);
    if (typeof args.comment === "string" && args.comment.trim() !== "")
      sub.comment = words(args.comment, "comment", 2000);
    if (!sub.optionId && !sub.optionIds && !sub.freeText) throw new OpError(400, "choose an option or write an answer");
    return this.submit(args.set, [sub]);
  }

  /** "Decide for me": hand the question back with a lean. */
  async defer(args: Record<string, unknown>) {
    const lean = args.lean === undefined ? "approve" : args.lean;
    if (!LEANS.includes(lean as (typeof LEANS)[number])) throw new OpError(400, `lean: one of ${LEANS.join(", ")}`);
    return this.submit(args.set, [
      {
        ledgerId: words(args.ledger, "ledger", 300),
        qid: words(args.qid, "qid", 300),
        kind: "deferred",
        lean: lean as (typeof LEANS)[number],
      },
    ]);
  }

  /** Take the recommended option on every open question of one set that has one. */
  async acceptRecommended(setId: unknown) {
    const set = (await this.sets()).find((s) => s.setId === setId);
    if (!set) throw new OpError(404, "that decision set is not in the store");
    const subs: Submission[] = [];
    for (const item of set.items ?? []) {
      for (const q of item.questions ?? []) {
        const rec = q.recommendedOptionIds ?? [];
        if (q.resolution || rec.length === 0) continue;
        subs.push(
          q.mode === "multi"
            ? { ledgerId: item.ledgerId, qid: q.qid, kind: "answered", optionIds: rec }
            : { ledgerId: item.ledgerId, qid: q.qid, kind: "answered", optionId: rec[0] },
        );
      }
    }
    if (subs.length === 0) throw new OpError(409, "no open question in that set has a recommendation");
    // The dashboard takes at most 256 answers in one request.
    const total = { set: String(setId), applied: 0, refused: [] as { qid: string; why: string }[] };
    for (let at = 0; at < subs.length; at += SUBMIT_BATCH) {
      const part = await this.submit(setId, subs.slice(at, at + SUBMIT_BATCH));
      total.applied += part.applied;
      total.refused.push(...part.refused);
    }
    return total;
  }

  private ref(value: unknown): { repo: string; number: number } {
    const m = typeof value === "string" ? REF.exec(value) : null;
    if (!m || !this.ops.cfg.needsRepos.includes(m[1])) throw new OpError(400, "ref: not an item on the Needs me list");
    return { repo: m[1], number: Number(m[2]) };
  }

  /** The owner did the one thing only he could: take the item off his
   * list and say so. The label goes first. Taking a label off twice is
   * the same as once (GitHub says it is not there, which is what was
   * wanted), and saying "done" twice is two comments: so a failure
   * part-way can be tried again from the start, by the owner or by
   * whoever holds this for him, and it is said once. */
  async handsDone(args: Record<string, unknown>) {
    const { repo, number } = this.ref(args.ref);
    const note =
      typeof args.note === "string" && args.note.trim() !== "" ? `\n\n${words(args.note, "note", 4000)}` : "";
    try {
      await this.ops.gh([
        "-X",
        "DELETE",
        `repos/${repo}/issues/${number}/labels/${encodeURIComponent("user/escalated")}`,
      ]);
    } catch (error) {
      // Already off: an agent took it, or this is a second try after
      // the comment failed. GitHub's words for it, and no other 404.
      if (!(error instanceof OpError) || !/Label does not exist/i.test(error.message)) throw error;
    }
    await this.ops.gh(["-X", "POST", `repos/${repo}/issues/${number}/comments`], {
      body: `Done by the owner, from the PM console. The step that needed a person is finished; carry on.${note}\n\n${OWNER_REPLY_MARK}`,
    });
    return { ref: args.ref, done: true };
  }

  async needComment(args: Record<string, unknown>) {
    const { repo, number } = this.ref(args.ref);
    const made = (await this.ops.gh(["-X", "POST", `repos/${repo}/issues/${number}/comments`], {
      body: `${words(args.text, "text", 8000)}\n\n${OWNER_REPLY_MARK}`,
    })) as { html_url: string };
    return { ref: args.ref, url: made.html_url };
  }

  // ---- asks: what the owner has told the fleet to do, in his words ------

  /** Take a list of prompts read before as what is known of each
   * thread, so that a prompt which has not changed since is not fetched
   * again. For a holder that was restarted and kept its last list (the
   * edge): without this every restart reads every thread anew. Anything
   * that is not such a list is passed over. */
  recall(asks: unknown): void {
    if (!Array.isArray(asks)) return;
    for (const ask of asks as { issue?: unknown; updated?: unknown; replies?: unknown }[]) {
      if (!ask || !Number.isSafeInteger(ask.issue) || typeof ask.updated !== "string" || !Array.isArray(ask.replies)) {
        continue;
      }
      const replies = (ask.replies as { you?: unknown; at?: unknown; text?: unknown }[]).filter(
        (r) => r && typeof r.you === "boolean" && typeof r.at === "string" && typeof r.text === "string",
      ) as { you: boolean; at: string; text: string }[];
      if (replies.length !== ask.replies.length) continue;
      if (!this.replies.has(ask.issue as number)) {
        this.replies.set(ask.issue as number, { updated: ask.updated, replies });
      }
    }
  }

  async asks() {
    const label = this.ops.cfg.promptLabel;
    // The issue list, not the search index: a prompt sent a second ago
    // must already be here, and the index runs a minute behind.
    const listed = (await this.ops.gh([
      `repos/${this.repo}/issues?labels=${encodeURIComponent(label)}&state=all&sort=updated&direction=desc&per_page=20`,
    ])) as {
      number: number;
      title: string;
      body: string | null;
      html_url: string;
      state: string;
      created_at: string;
      updated_at: string;
      comments: number;
      pull_request?: unknown;
    }[];
    const found = { items: listed.filter((i) => !i.pull_request) };
    // The newest task for each prompt's issue, by the key the task was filed under.
    const keys = found.items.map((i) => `repo-health:v1:triage:${this.repo}:issue:${i.number}`);
    const tasks = keys.length
      ? await this.ops.sql(
          `SELECT dedupe_key, status, worker_id AS worker FROM tasks WHERE queue = ? AND dedupe_key IN (${keys.map(() => "?").join(",")}) ORDER BY id DESC LIMIT 200`,
          [this.ops.cfg.queue, ...keys],
        )
      : [];
    const taskOf = new Map<number, Row>();
    for (const t of tasks) {
      const n = Number(/:issue:(\d+)$/.exec(String(t.dedupe_key))?.[1]);
      if (n && !taskOf.has(n)) taskOf.set(n, t);
    }
    const out = [];
    for (const i of found.items) {
      let thread = this.replies.get(i.number);
      if (i.comments > 0 && thread?.updated !== i.updated_at.slice(0, 19)) {
        // The newest comments are on the last page. Bots that only link the
        // issue elsewhere (the board sync) are not the fleet answering.
        // Read back from the last page until the replies shown are in
        // hand: a page boundary, or a tail of bot comments, must not hide
        // the answer.
        type Comment = { body: string | null; created_at: string; user?: { type?: string } | null };
        const last = Math.max(1, Math.ceil(i.comments / 100));
        let comments: Comment[] = [];
        for (let page = last; page >= 1 && last - page < REPLY_PAGES && comments.length < REPLIES_SHOWN; page--) {
          const got = (await this.ops.gh([
            `repos/${this.repo}/issues/${i.number}/comments?per_page=100&page=${page}`,
          ])) as Comment[];
          comments = [...got.filter((c) => c.user?.type !== "Bot"), ...comments];
        }
        thread = {
          updated: i.updated_at.slice(0, 19),
          replies: comments.slice(-REPLIES_SHOWN).map((c) => ({
            you: (c.body ?? "").includes(OWNER_REPLY_MARK),
            at: c.created_at.slice(0, 19),
            text: clip((c.body ?? "").replace(OWNER_REPLY_MARK, "").trim(), 1500),
          })),
        };
        this.replies.set(i.number, thread);
      }
      const replies = i.comments > 0 ? (thread?.replies ?? []) : [];
      const task = taskOf.get(i.number);
      const answered = replies.length > 0 && !replies[replies.length - 1].you;
      out.push({
        issue: i.number,
        url: i.html_url,
        intent: askIntentOf(i.title),
        text: clip((i.body ?? "").split("\n---\n")[0].trim() || i.title, 2000),
        created: i.created_at.slice(0, 19),
        updated: i.updated_at.slice(0, 19),
        // done: closed. working: an agent holds the task. answered: the last word is the fleet's.
        status:
          i.state === "closed"
            ? "done"
            : task?.status === "claimed"
              ? "working"
              : answered
                ? "answered"
                : task?.status === "queued"
                  ? "queued"
                  : "waiting",
        worker: task?.status === "claimed" ? task.worker : null,
        replies,
      });
    }
    return out;
  }

  private async ensureLabel() {
    await this.ops.ensureLabel(
      this.ops.cfg.promptLabel,
      "5319e7",
      "An instruction from the owner, in his words (PM console)",
    );
  }

  /** Turn the owner's words into an issue and a top-band task, in one pass.
   * `enqueue` is a stand-in point: the edge files the issue itself and
   * holds the enqueue for the laptop's return instead of running the
   * fleet binary, which is not there. */
  /**
   * File a prompt's task where this side can: straight into the queue's
   * store (src/filing.ts), and through the fleet binary where only the
   * broker can say what filing it means, or the store did not answer.
   * The binary files no blockers; that is said.
   */
  async file(packet: unknown, options: FilingOptions): Promise<Queued> {
    try {
      const filed = await fileTask(this.ops, packet as Record<string, unknown>, this.clock(), options.blockers);
      return {
        queued: true,
        task: filed.task,
        status: filed.status,
        priority: filed.priority,
        note: filed.filed ? null : filed.said,
      };
    } catch (error) {
      if (error instanceof OpError && error.status === 400) throw error;
      const via = await this.ops.enqueue(packet);
      if (via.code !== 0)
        return { queued: false, note: (via.stderr || via.stdout).trim().split("\n").pop()?.slice(0, 200) ?? "" };
      const why = error instanceof NeedsBroker ? error.message : "the queue's store did not answer";
      return {
        queued: true,
        note: options.blockers.length ? `queued by the broker (${why}), not waiting for what you named` : null,
      };
    }
  }

  /**
   * The owner sends a prompt: something to do, a question, or a standing
   * instruction, with how soon and how much it matters (now and critical
   * unless he says otherwise), when it is wanted by, what it waits for,
   * a note and anything else as names and values. It is filed as an
   * issue and its task put in the queue. `enqueue` is a stand-in point:
   * Cloudflare files the task itself and hands it to a machine only
   * when it must.
   */
  async ask(
    args: Record<string, unknown>,
    enqueue: (packet: unknown, intent: AskIntent, options: FilingOptions) => Promise<Queued> = (p, _i, o) =>
      this.file(p, o),
  ) {
    const text = words(args.text, "text", 8000);
    const intent = askIntent(args.intent);
    const options = filingOptions(args, this.repo);
    await this.ensureLabel();
    const first = text.split("\n")[0].trim();
    const priority = claimPriority(options.urgency, options.importance);
    // The queue's own label for the chosen priority: whatever ranks
    // issues by their labels reads it the same way as the task.
    const labels = [this.ops.cfg.promptLabel, `priority/${category(priority)}`];
    const lines = ownerLines(options, this.repo);
    const made = (await this.ops.gh(["-X", "POST", `repos/${this.repo}/issues`], {
      title: `${ASK_TITLE[intent]}${first}`.slice(0, 120),
      body: `${text}\n\n---\n${lines.length ? `${lines.join("\n")}\n\n` : ""}${ASK_NOTE[intent]}\n${OWNER_PROMPT_MARK}`,
      labels,
    })) as { number: number; html_url: string };
    const queued = await enqueue(
      promptPacket(
        this.repo,
        this.ops.cfg.queue,
        made.number,
        text,
        this.clock().toISOString(),
        intent,
        options,
        labels,
      ),
      intent,
      options,
    );
    return {
      issue: made.number,
      url: made.html_url,
      priority,
      queued: queued.queued,
      ...(queued.task ? { task: queued.task, status: queued.status } : {}),
      ...noteOf(queued),
    };
  }

  /** The owner answers back on one of his prompts; the fleet is woken for
   * it. Its task is filed again even when the last one is done: an answer
   * after the work is finished is more work (a done task would otherwise
   * be taken as the same work, and nothing filed). `enqueue` is a
   * stand-in point, as in ask(). */
  async askReply(
    args: Record<string, unknown>,
    enqueue: (packet: unknown, intent: AskIntent, options: FilingOptions) => Promise<Queued> = (p, _i, o) =>
      this.file(p, o),
  ) {
    const issue = Number(args.issue);
    if (!Number.isSafeInteger(issue) || issue <= 0) throw new OpError(400, "issue: a positive whole number");
    const text = words(args.text, "text", 8000);
    const options = filingOptions(args, this.repo);
    const current = (await this.ops.gh([`repos/${this.repo}/issues/${issue}`])) as {
      title?: string;
      state: string;
      labels: { name: string }[];
    };
    if (!current.labels.some((l) => l.name === this.ops.cfg.promptLabel)) {
      throw new OpError(400, "that issue is not one of the owner's prompts");
    }
    // What he says next on a question is still about a question.
    const intent = askIntentOf(current.title);
    await this.ops.gh(["-X", "POST", `repos/${this.repo}/issues/${issue}/comments`], {
      body: `${text}\n\n${OWNER_REPLY_MARK}`,
    });
    if (current.state === "closed") {
      await this.ops.gh(["-X", "PATCH", `repos/${this.repo}/issues/${issue}`], { state: "open" });
    }
    const labels = current.labels.map((l) => l.name);
    const queued = await enqueue(
      promptPacket(
        this.repo,
        this.ops.cfg.queue,
        issue,
        text,
        this.clock().toISOString(),
        intent,
        options,
        labels,
        true,
      ),
      intent,
      options,
    );
    return {
      issue,
      queued: queued.queued,
      ...(queued.task ? { task: queued.task, status: queued.status } : {}),
      ...noteOf(queued),
    };
  }

  /** Nothing is left on a prompt: close it. */
  async askClose(args: Record<string, unknown>) {
    const issue = Number(args.issue);
    if (!Number.isSafeInteger(issue) || issue <= 0) throw new OpError(400, "issue: a positive whole number");
    const current = (await this.ops.gh([`repos/${this.repo}/issues/${issue}`])) as { labels: { name: string }[] };
    if (!current.labels.some((l) => l.name === this.ops.cfg.promptLabel)) {
      throw new OpError(400, "that issue is not one of the owner's prompts");
    }
    await this.ops.gh(["-X", "PATCH", `repos/${this.repo}/issues/${issue}`], {
      state: "closed",
      state_reason: "completed",
    });
    return { issue, closed: true };
  }

  /** Move a queued task: to the top band, or one step sooner or later. */
  async moveTask(args: Record<string, unknown>) {
    const task = Number(args.task);
    if (!Number.isSafeInteger(task) || task <= 0) throw new OpError(400, "task: a positive whole number");
    if (args.to === "first") return this.ops.setTaskPriority(task, PRIORITY_SCALE[0]);
    if (args.to !== "sooner" && args.to !== "later") throw new OpError(400, "to: first, sooner or later");
    const [row] = await this.ops.sql("SELECT priority FROM tasks WHERE id = ? AND queue = ? AND status = 'queued'", [
      task,
      this.ops.cfg.queue,
    ]);
    if (!row) throw new OpError(409, `task ${task} is not queued, so its place cannot change`);
    const scale = PRIORITY_SCALE as readonly number[];
    const current = Number(row.priority ?? 30);
    // The nearest step on the scale in the direction asked.
    const target =
      args.to === "sooner" ? [...scale].reverse().find((p) => p < current) : scale.find((p) => p > current);
    if (target === undefined) {
      throw new OpError(
        409,
        `task ${task} is already ${args.to === "sooner" ? "in the top band" : "in the last band"}`,
      );
    }
    return this.ops.setTaskPriority(task, target);
  }
}
