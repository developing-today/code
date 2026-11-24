// What the owner looks at first: the work in progress, what comes next,
// what is waiting on him, and what he has asked for.
//
// Reads come from the queue store, the dashboard's own data and GitHub.
// Every write is something the fleet already understands: an answer to a
// decision set through the dashboard's own endpoint, an issue and a
// queue task, a comment, a label.
import { type Census, censusOf, type NeedsItem, readCensus } from "./needs.ts";
import { OpError, type Ops, PRIORITY_SCALE, type Row, type RunResult } from "./ops.ts";
import { timed } from "./timed.ts";

const P = "CASE WHEN json_valid(payload) THEN payload ELSE '{}' END";
const G = "CASE WHEN json_valid(progress) THEN progress ELSE '{}' END";
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
const ASK_NOTE =
  "This is an instruction from the owner, in his own words, sent from the PM console. Work out what he wants and do it with the fleet's own verbs; if it is a question, answer it here. Say on this issue what you did, with links, and close it when nothing is left.";
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

/** The task that carries out an owner prompt. Top band: the owner asked.
 * It takes the issue's triage key, the one key the queue has for "someone
 * must look at this issue", so an issue never has two tasks: a reply on a
 * prompt whose task is still live joins it, and one whose task has
 * finished gets a new one. */
export function promptPacket(repo: string, queue: string, number: number, text: string, at: string) {
  const url = `https://github.com/${repo}/issues/${number}`;
  const first = text.split("\n")[0].trim();
  return {
    title: `Owner prompt ${repo}#${number}: ${first}`.slice(0, 180),
    body: `Instruction from the owner, sent from the PM console: ${url}`,
    priority: 10,
    dedupe_key: `repo-health:v1:triage:${repo}:issue:${number}`,
    payload: {
      schema: "repo-task/v1",
      kind: "triage",
      repo,
      queue,
      issue: { number, url, title: `Owner: ${first}`.slice(0, 120), state_checked_at: at },
      problem:
        "The owner wrote this instruction in his own words from the PM console. It is not a report to classify: it is something he wants done, or a question he wants answered.",
      evidence: [`Sent from the PM console at ${at}`, url],
      acceptance: [
        "What the owner asked for is done with the fleet's own verbs, or answered if it is a question",
        "A comment on the issue says in plain words what was done, with links, or what could not be done and why",
        "The issue is closed when nothing is left; work that outlasts this task is filed, enqueued and linked from the issue",
      ],
      tests: ["The issue carries the report comment and is closed, or carries the reason it is still open"],
      scope: {
        include: [`Carry out the owner's instruction on ${repo}#${number}`],
        exclude: ["Secrets, credentials or purchases"],
      },
      risk: { level: "low", flags: ["owner-prompt"] },
      priority_reason: "Owner prompt from the console: top band (10).",
      timebox_minutes: 45,
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
      {
        sql: `SELECT substr(coalesce(json_extract(${G},'$.reason'), json_extract(${G},'$.blocker'), ready_condition, blocked_deps, 'no reason recorded'),1,140) AS why, count(*) AS n FROM tasks WHERE queue = ? AND status = 'blocked' GROUP BY 1 ORDER BY n DESC LIMIT 6`,
        args: [q],
      },
    ]);
    return {
      counts: Object.fromEntries(counts.map((r) => [String(r.status), Number(r.n)])),
      now: now.map(this.task),
      next: next.map(this.task),
      blockedWhy: why,
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
  async ask(
    args: Record<string, unknown>,
    enqueue: (packet: unknown) => Promise<RunResult> = (p) => this.ops.enqueue(p),
  ) {
    const text = words(args.text, "text", 8000);
    await this.ensureLabel();
    const first = text.split("\n")[0].trim();
    const made = (await this.ops.gh(["-X", "POST", `repos/${this.repo}/issues`], {
      title: `Owner: ${first}`.slice(0, 120),
      body: `${text}\n\n---\n${ASK_NOTE}\n${OWNER_PROMPT_MARK}`,
      labels: [this.ops.cfg.promptLabel, "expedite"],
    })) as { number: number; html_url: string };
    const queued = await enqueue(
      promptPacket(this.repo, this.ops.cfg.queue, made.number, text, this.clock().toISOString()),
    );
    return {
      issue: made.number,
      url: made.html_url,
      queued: queued.code === 0,
      ...(queued.code === 0
        ? {}
        : { note: `filed, not queued: ${(queued.stderr || queued.stdout).trim().split("\n").pop()?.slice(0, 200)}` }),
    };
  }

  /** The owner answers back on one of his prompts; the fleet is woken for it.
   * `enqueue` is a stand-in point, as in ask(): the edge posts the reply
   * itself and holds the enqueue for the laptop's return. */
  async askReply(
    args: Record<string, unknown>,
    enqueue: (packet: unknown) => Promise<RunResult> = (p) => this.ops.enqueue(p),
  ) {
    const issue = Number(args.issue);
    if (!Number.isSafeInteger(issue) || issue <= 0) throw new OpError(400, "issue: a positive whole number");
    const text = words(args.text, "text", 8000);
    const current = (await this.ops.gh([`repos/${this.repo}/issues/${issue}`])) as {
      state: string;
      labels: { name: string }[];
    };
    if (!current.labels.some((l) => l.name === this.ops.cfg.promptLabel)) {
      throw new OpError(400, "that issue is not one of the owner's prompts");
    }
    await this.ops.gh(["-X", "POST", `repos/${this.repo}/issues/${issue}/comments`], {
      body: `${text}\n\n${OWNER_REPLY_MARK}`,
    });
    if (current.state === "closed") {
      await this.ops.gh(["-X", "PATCH", `repos/${this.repo}/issues/${issue}`], { state: "open" });
    }
    const queued = await enqueue(promptPacket(this.repo, this.ops.cfg.queue, issue, text, this.clock().toISOString()));
    return { issue, queued: queued.code === 0 };
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
