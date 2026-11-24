// The plan: each capability's roadmap, and the one way in for what the
// owner says about it.
//
// A roadmap is read from where it already lives: the curated rows of
// its file in planning/roadmaps/, and the issues under the capability's
// labels. Nothing is copied into a store of this service's own.
//
// A note, a bug and a change to a roadmap item are one thing here, an
// intake item: the owner's words verbatim, what they are about, how
// urgent, and whether an issue or a session should take them. Where it
// goes is decided by one table (ROUTES), not by the screen it came
// from. The source of truth is GitHub, so the Intake screen is read
// back from the issues themselves and is the same wherever this ran.
//
// Everything here reaches GitHub through Ops.gh, so it runs unchanged on
// a machine (the fleet binary's gh) and at the edge (GitHub's REST API).
import { OpError } from "./errors.ts";
import { type Ops, RANK_PRIORITY, RANKS, type Rank, type RunResult, triagePacket } from "./ops.ts";
import type { Work } from "./work.ts";

export const KINDS = ["note", "bug", "change"] as const;
export type Kind = (typeof KINDS)[number];
/** Where an item stands, in the order a roadmap page shows them. */
export const PHASES = ["doing", "now", "next", "later", "built"] as const;
export type Phase = (typeof PHASES)[number];
export const PHASE_TITLE: Record<Phase, string> = {
  doing: "In progress",
  now: "Planned: immediately",
  next: "Planned: short term",
  later: "Planned: long term",
  built: "Built",
};
/** What may be asked of a roadmap item. */
export const CHANGES = ["move", "add", "add-under", "built", "drop", "rewrite"] as const;
export type Change = (typeof CHANGES)[number];

/** Marks an issue as having come in through the app. Read back to list them. */
export const INTAKE_LABEL = "owner/intake";
const HUMAN_LABEL = "source/human";

export interface Capability {
  id: string;
  title: string;
  /** The capability's own label, and the labels of its programmes: an
   * issue under any of them is the capability's. */
  labels: string[];
  /** Its roadmap file, when it has one. */
  file: string | null;
}

/** The major capabilities first, then every other roadmap file. */
export const CAPABILITIES: Capability[] = [
  { id: "id", title: "id", labels: ["app/id", "roadmap/id-program"], file: "planning/roadmaps/id.md" },
  {
    id: "fleet",
    title: "Fleet",
    labels: [
      "app/fleet",
      "epic/fleet-tool",
      "epic/fleet-orchestration",
      "epic/fleet-expansion",
      "roadmap/fleet-health",
    ],
    file: "planning/roadmaps/fleet.md",
  },
  {
    id: "atlas",
    title: "Atlas",
    labels: ["app/atlas", "app/atlas-search", "epic/atlas-program", "epic/atlas"],
    file: "planning/roadmaps/atlas.md",
  },
  {
    id: "pm-stack",
    title: "PM stack",
    labels: ["app/pm-stack", "epic/pm-stack", "epic/pm-stack-coherence", "roadmap/pm-stack"],
    file: "planning/roadmaps/pm-stack.md",
  },
  { id: "mcpx", title: "mcpx", labels: ["app/mcpx"], file: "planning/roadmaps/mcpx.md" },
  {
    id: "infra",
    title: "NixOS and infra",
    labels: ["area/nixos", "area/infra"],
    file: "planning/roadmaps/nixos-and-infra.md",
  },
  { id: "repo-and-ci", title: "Repo and CI", labels: ["area/ci"], file: "planning/roadmaps/repo-and-ci.md" },
  {
    id: "basement-cluster",
    title: "Basement cluster",
    labels: ["roadmap/lab-cluster"],
    file: "planning/roadmaps/basement-cluster.md",
  },
];

/** Where each kind goes. A note about an existing issue is a comment on
 * it; everything else is an issue with these labels (and a person's, the
 * app's and the capability's own) and a task, or, asked for as a session,
 * a prompt that starts one. A note and a change carry no kind of their
 * own: whoever takes it decides what kind of work it turned out to be. */
export const ROUTES: Record<Kind, { labels: string[]; rank: Rank; says: string }> = {
  bug: {
    labels: ["kind/bug"],
    rank: "now",
    says: "A bug the owner reported. Reproduce it, fix it, and say on this issue what was wrong.",
  },
  note: {
    labels: [],
    rank: "next",
    says: "A note from the owner. Decide what it changes (a design, an issue, a priority, a roadmap row), do that, and say here what was done.",
  },
  change: {
    labels: [],
    rank: "next",
    says: "A change the owner asked for to the roadmap. Make it where the roadmap lives (the file, the issues and their labels) and say here what was changed.",
  },
};

const WORKING = ["work/in-progress", "queue/claimed", "queue/in-progress"];
const SOON = ["expedite", "priority/critical", "now"];
const SHORT = ["priority/high", "next"];
const EPICS = ["meta/epic", "kind/tracker"];
const BUILT_DAYS = 14;
/** How long a read is kept. GitHub's search allows thirty a minute, and
 * the fleet's own searches count against the same thirty. */
const KEEP_MS = 60_000;

export interface RoadmapRow {
  title: string;
  status: string;
  intent: string;
  /** Issues the row names. */
  issues: number[];
  phase: Phase;
  /** The file's own heading the row sits under. */
  section: string;
}

const cells = (line: string): string[] =>
  line
    .trim()
    .replace(/^\||\|$/g, "")
    .split(/(?<!\\)\|/)
    .map((c) => c.trim().replace(/\\\|/g, "|"));

/** The curated rows of a roadmap file: every table whose first column
 * is Title, under a heading that starts Now, Next, Later or Done. A
 * row's own status wins over its heading for work under way and work
 * finished. */
export function parseRoadmap(markdown: string): RoadmapRow[] {
  const rows: RoadmapRow[] = [];
  let section = "";
  let under: Phase | null = null;
  let columns: string[] | null = null;
  for (const line of markdown.split("\n")) {
    const heading = /^##\s+(.*)$/.exec(line);
    if (heading) {
      section = heading[1].trim();
      const word = section.toLowerCase();
      under = word.startsWith("now")
        ? "now"
        : word.startsWith("next")
          ? "next"
          : word.startsWith("later")
            ? "later"
            : word.startsWith("done") || word.startsWith("built")
              ? "built"
              : null;
      columns = null;
      continue;
    }
    if (!line.trim().startsWith("|")) {
      columns = null;
      continue;
    }
    const row = cells(line);
    if (columns === null) {
      columns = row.map((c) => c.toLowerCase());
      continue;
    }
    if (row.every((c) => /^:?-+:?$/.test(c))) continue;
    if (under === null || columns[0] !== "title") continue;
    const at = (name: string) => row[columns?.indexOf(name) ?? -1] ?? "";
    const title = at("title");
    if (title === "") continue;
    const status = at("status").toLowerCase();
    const phase: Phase = /^(in[- ]progress|doing|started)$/.test(status)
      ? "doing"
      : /^(done|built|shipped|complete|completed)$/.test(status)
        ? "built"
        : under;
    const issues = [...`${at("links")} ${at("intent")}`.matchAll(/(?<![\w/])#(\d{1,7})\b/g)].map((m) => Number(m[1]));
    rows.push({ title, status, intent: at("intent"), issues: [...new Set(issues)], phase, section });
  }
  return rows;
}

function text(value: unknown, name: string, max: number): string {
  if (typeof value !== "string") throw new OpError(400, `${name}: expected text`);
  const trimmed = value.trim();
  if (trimmed === "") throw new OpError(400, `${name}: required`);
  if (trimmed.length > max) throw new OpError(400, `${name}: longer than ${max} characters`);
  return trimmed;
}

function one<T extends string>(value: unknown, of: readonly T[], name: string): T {
  if (!of.includes(value as T)) throw new OpError(400, `${name}: one of ${of.join(", ")}`);
  return value as T;
}

/** The first line of the owner's words, short enough for a title. */
const firstLine = (words: string, max: number): string => {
  const line = words.split("\n")[0].trim();
  return line.length > max ? `${line.slice(0, max - 1).trimEnd()}…` : line;
};

/** What an intake item writes at the foot of what it files, so that it
 * can be told apart later without trusting a title. */
const MARK = /<!-- pm-intake (\{[^\n]*\}) -->/;
function markOf(body: unknown): { kind?: string; area?: string | null; mode?: string } | null {
  const found = typeof body === "string" ? MARK.exec(body) : null;
  if (!found) return null;
  try {
    const mark = JSON.parse(found[1]);
    return mark !== null && typeof mark === "object" ? mark : null;
  } catch {
    return null;
  }
}

interface Found {
  number: number;
  title: string;
  html_url: string;
  state: string;
  updated_at: string;
  comments: number;
  body?: string | null;
  labels: { name: string }[];
  pull_request?: unknown;
}

export interface PlanIssue {
  number: number;
  title: string;
  url: string;
  state: string;
  updated: string;
  comments: number;
  rank: Rank;
  working: boolean;
  /** The capability whose own label it carries. */
  area: string | null;
  kind: "bug" | "session" | "change" | "note";
  labels: string[];
}

function shown(i: Found): PlanIssue {
  const labels = i.labels.map((l) => l.name);
  const mark = markOf(i.body);
  return {
    number: i.number,
    title: i.title,
    url: i.html_url,
    state: i.state,
    updated: i.updated_at.slice(0, 16),
    comments: i.comments,
    rank: RANKS.find((r) => labels.includes(r)) ?? "none",
    working: labels.some((l) => WORKING.includes(l)),
    area: CAPABILITIES.find((c) => labels.includes(c.labels[0]))?.id ?? null,
    kind:
      labels.includes("kind/bug") || labels.includes("bug")
        ? "bug"
        : labels.includes("owner/prompt")
          ? "session"
          : mark?.kind === "change"
            ? "change"
            : "note",
    labels,
  };
}

/** Where an open issue stands, by its labels: being worked on beats
 * ranked, ranked sooner beats ranked later, and an epic with no rank is
 * the long term. */
export function phaseOf(labels: string[]): Exclude<Phase, "built"> {
  if (labels.some((l) => WORKING.includes(l))) return "doing";
  if (labels.some((l) => SOON.includes(l))) return "now";
  if (labels.some((l) => SHORT.includes(l))) return "next";
  return "later";
}

/** The task for a note the owner left on an issue that already exists.
 * It takes the issue's triage key, like every task about "someone must
 * look at this issue": a note on an issue whose task is still live joins
 * it, and one whose task has finished gets a new one. */
export function notePacket(repo: string, queue: string, number: number, first: string, rank: Rank, at: string) {
  const url = `https://github.com/${repo}/issues/${number}`;
  return {
    title: `Owner's note on ${repo}#${number}: ${first}`.slice(0, 180),
    body: `A note from the owner, left on the issue from the PM app: ${url}`,
    priority: RANK_PRIORITY[rank],
    dedupe_key: `repo-health:v1:triage:${repo}:issue:${number}`,
    payload: {
      schema: "repo-task/v1",
      kind: "triage",
      repo,
      queue,
      issue: { number, url, title: `Owner's note: ${first}`.slice(0, 120), state_checked_at: at },
      problem:
        "The owner left a note on this issue from the PM app, in his own words: the newest comment that ends with a pm-intake mark. It is not a report to classify: it says what he wants changed about this issue or the work under it.",
      evidence: [`Left from the PM app at ${at}, rank ${rank}`, url],
      acceptance: [
        "What the note asks for is done: the issue, its plan, its priority or the work under it is changed to match",
        "A comment on the issue says in plain words what was changed, with links, or what could not be done and why",
      ],
      tests: ["The issue carries the reply comment under the owner's note"],
      scope: {
        include: [`Carry out the owner's note on ${repo}#${number}`],
        exclude: ["Secrets, credentials or purchases"],
      },
      risk: { level: "low", flags: ["owner-note"] },
      priority_reason: `A note from the owner with rank ${rank}: priority ${RANK_PRIORITY[rank]}.`,
      timebox_minutes: 45,
      max_attempts: 1000,
      context_refs: [url],
      dedupe: { checked_at: at, basis: "Filed with the owner's note, in the same pass." },
    },
  };
}

/** The half a machine does when the edge filed the issue itself: the
 * action it holds for one, with arguments the machine checks again and
 * builds the task from. */
export interface Later {
  kind: "triage.enqueue" | "prompt.enqueue";
  args: Record<string, unknown>;
}
export type Enqueue = (packet: unknown, later: Later) => Promise<RunResult>;

export interface IntakeResult {
  issue: number;
  url: string;
  /** A comment was left on an existing issue; no issue was made. */
  commented: boolean;
  queued: boolean;
  note?: string;
}

/** What was read lately, by the Ops it was read through, so that every
 * reader of the same repository shares it however it was wired. */
const kept = new WeakMap<Ops, Map<string, { at: number; value: Promise<unknown> }>>();

export class Plan {
  constructor(
    private ops: Ops,
    private work: Work,
    private clock: () => Date = () => new Date(),
  ) {}

  private get repo(): string {
    return this.ops.cfg.repo;
  }

  /** A read kept for a minute; `fresh` asks again now. A read that
   * failed, in whole or in part, is not kept. */
  private keep<T>(key: string, fresh: boolean, read: () => Promise<T>): Promise<T> {
    let mine = kept.get(this.ops);
    if (!mine) {
      mine = new Map();
      kept.set(this.ops, mine);
    }
    const all = mine;
    const now = this.clock().getTime();
    const had = all.get(key);
    if (!fresh && had && now - had.at < KEEP_MS) return had.value as Promise<T>;
    const value = read();
    const entry = { at: now, value };
    all.set(key, entry);
    const drop = () => {
      if (all.get(key) === entry) all.delete(key);
    };
    value.then((v) => {
      if (v !== null && typeof v === "object" && (v as { partial?: boolean }).partial) drop();
    }, drop);
    return value;
  }

  /** Something changed what these reads show: the next one asks again. */
  forget() {
    kept.get(this.ops)?.clear();
  }

  capabilities() {
    return CAPABILITIES.map((c) => ({ id: c.id, title: c.title, file: c.file }));
  }

  private capability(area: unknown): Capability {
    const found = CAPABILITIES.find((c) => c.id === area);
    if (!found) throw new OpError(400, `area: one of ${CAPABILITIES.map((c) => c.id).join(", ")}`);
    return found;
  }

  private async search(q: string, limit: number): Promise<{ items: PlanIssue[]; total: number }> {
    const found = (await this.ops.gh([
      "-X",
      "GET",
      "search/issues",
      "-f",
      `q=repo:${this.repo} is:issue ${q}`,
      "-f",
      "sort=updated",
      "-f",
      `per_page=${limit}`,
    ])) as { items?: Found[]; total_count?: number } | null;
    const items = (found?.items ?? []).map(shown);
    return { items, total: Math.max(Number(found?.total_count ?? 0), items.length) };
  }

  /** One capability's roadmap: its file's rows, and its issues by where
   * they stand. Three reads: the file, the open issues that are being
   * worked on, ranked or epics, and what closed lately. A part that
   * could not be read says so by itself, and the rest still shows. */
  roadmap(area: unknown, fresh = false) {
    const cap = this.capability(area);
    return this.keep(`roadmap:${cap.id}`, fresh, async () => {
      const any = (labels: string[]) => `label:${labels.map((l) => `"${l}"`).join(",")}`;
      const mine = any(cap.labels);
      const since = new Date(this.clock().getTime() - BUILT_DAYS * 86_400_000).toISOString().slice(0, 10);
      const part = async <T>(p: Promise<T>): Promise<T | { error: string }> =>
        p.catch((e: Error) => ({ error: e.message }));
      const file = async (): Promise<RoadmapRow[]> => {
        if (!cap.file) return [];
        const got = (await this.ops.gh([`repos/${this.repo}/contents/${cap.file}`])) as { content?: string } | null;
        if (typeof got?.content !== "string") throw new Error("the roadmap file could not be read");
        // GitHub wraps the encoded file in lines; decoding passes over them.
        const bytes = Uint8Array.from(atob(got.content), (c) => c.charCodeAt(0));
        return parseRoadmap(new TextDecoder().decode(bytes));
      };
      const [rows, open, built] = await Promise.all([
        part(file()),
        part(this.search(`is:open ${mine} ${any([...WORKING, ...SOON, ...SHORT, ...EPICS])}`, 100)),
        part(this.search(`is:closed ${mine} closed:>=${since}`, 30)),
      ]);
      const failed = (p: unknown): p is { error: string } => p !== null && typeof p === "object" && "error" in p;
      const at = (phase: Phase) => (failed(open) ? open : open.items.filter((i) => phaseOf(i.labels) === phase));
      return {
        area: cap.id,
        title: cap.title,
        file: cap.file,
        labels: cap.labels,
        rows,
        issues: {
          doing: at("doing"),
          now: at("now"),
          next: at("next"),
          later: at("later"),
          built: failed(built) ? built : built.items,
        },
        /** How many there are beyond the ones listed. */
        more: {
          open: failed(open) ? 0 : open.total - open.items.length,
          built: failed(built) ? 0 : built.total - built.items.length,
        },
        builtDays: BUILT_DAYS,
        partial: failed(rows) || failed(open) || failed(built),
      };
    });
  }

  /** Open bugs, soonest rank first, and the ones closed lately. */
  bugs(fresh = false) {
    return this.keep("bugs", fresh, async () => {
      const since = new Date(this.clock().getTime() - BUILT_DAYS * 86_400_000).toISOString().slice(0, 10);
      const order = (r: string) => {
        const at = (RANKS as readonly string[]).indexOf(r);
        return at < 0 ? RANKS.length : at;
      };
      const [open, fixed] = await Promise.all([
        this.search(`is:open label:"kind/bug","bug"`, 100),
        this.search(`is:closed label:"kind/bug","bug" closed:>=${since}`, 30),
      ]);
      return {
        open: open.items.sort((a, b) => order(a.rank) - order(b.rank) || b.number - a.number),
        fixed: fixed.items,
        more: { open: open.total - open.items.length, fixed: fixed.total - fixed.items.length },
        builtDays: BUILT_DAYS,
      };
    });
  }

  /** Everything that came in through the app, newest first, as GitHub has it now. */
  intakeList(fresh = false) {
    return this.keep("intake", fresh, async () => {
      const found = (await this.ops.gh([
        `repos/${this.repo}/issues?labels=${encodeURIComponent(INTAKE_LABEL)}&state=all&sort=updated&direction=desc&per_page=50`,
      ])) as Found[] | null;
      return {
        items: (found ?? []).filter((i) => !i.pull_request).map(shown),
        routes: Object.fromEntries(
          KINDS.map((k) => [k, { labels: [...ROUTES[k].labels, HUMAN_LABEL, INTAKE_LABEL], rank: ROUTES[k].rank }]),
        ),
      };
    });
  }

  /**
   * Take in a note, a bug or a change, in the owner's words.
   *
   * `enqueue` is a stand-in point, as in Work.ask(): the edge files the
   * issue itself and has a machine file the queue task, from `later`.
   */
  async intake(
    args: Record<string, unknown>,
    enqueue: Enqueue = (packet) => this.ops.enqueue(packet),
  ): Promise<IntakeResult> {
    const kind = one(args.kind, KINDS, "kind");
    const words = text(args.text, "text", 8000);
    const cap = args.area === undefined || args.area === null || args.area === "" ? null : this.capability(args.area);
    const mode = args.mode === undefined ? "issue" : one(args.mode, ["issue", "session"] as const, "mode");
    const rank: Rank | "keep" =
      args.rank === undefined || args.rank === "keep"
        ? "keep"
        : args.rank === "none"
          ? "none"
          : one(args.rank, RANKS, "rank");
    if (args.about !== undefined && (args.about === null || typeof args.about !== "object")) {
      throw new OpError(400, "about: expected an object");
    }
    const about = (args.about ?? {}) as { issue?: unknown; row?: unknown; phase?: unknown };
    let aboutIssue: number | null = null;
    if (about.issue !== undefined && about.issue !== null && about.issue !== "") {
      if (!/^[1-9][0-9]{0,9}$/.test(String(about.issue))) throw new OpError(400, "about.issue: not a positive number");
      aboutIssue = Number(about.issue);
    }
    const aboutRow = about.row === undefined || about.row === null ? null : text(about.row, "about.row", 300);
    const aboutPhase =
      about.phase === undefined || about.phase === null ? null : one(about.phase, PHASES, "about.phase");
    if (args.change !== undefined && (args.change === null || typeof args.change !== "object")) {
      throw new OpError(400, "change: expected an object");
    }
    const change = (args.change ?? null) as { action?: unknown; to?: unknown; title?: unknown } | null;
    const asked =
      change === null
        ? null
        : {
            action: one(change.action, CHANGES, "change.action"),
            to: change.to === undefined || change.to === null ? null : one(change.to, PHASES, "change.to"),
            title: change.title === undefined || change.title === null ? null : text(change.title, "change.title", 200),
          };
    if (kind === "change" && asked === null) throw new OpError(400, "change: required for a change");
    if (kind !== "change" && asked !== null) throw new OpError(400, "change: only a change says what to change");
    if (asked?.action === "move" && asked.to === null) throw new OpError(400, "change.to: required to move an item");
    const title =
      args.title === undefined || args.title === null || args.title === "" ? null : text(args.title, "title", 200);
    const at = this.clock().toISOString();

    const what = [
      `Filed from the PM app: a **${kind}**${cap ? ` about **${cap.title}**` : ""}.`,
      aboutIssue !== null ? `About: #${aboutIssue}` : null,
      aboutRow !== null
        ? `About the roadmap row: "${aboutRow}"${aboutPhase ? ` (${PHASE_TITLE[aboutPhase]})` : ""}${cap?.file ? ` in \`${cap.file}\`` : ""}`
        : null,
      asked ? `Asked for: ${changeSaid(asked)}` : null,
      rank !== "keep" && rank !== "none" ? `Rank asked for: \`${rank}\`` : null,
      "",
      `The words above the line are the owner's, verbatim. ${ROUTES[kind].says}`,
    ]
      .filter((l) => l !== null)
      .join("\n");
    const mark = `<!-- pm-intake ${JSON.stringify({
      v: 1,
      id: typeof args.id === "string" ? args.id.slice(0, 64) : null,
      kind,
      area: cap?.id ?? null,
      mode,
    })} -->`;
    const notQueued = (e: Error): RunResult => ({ code: 1, stdout: "", stderr: e.message });
    await this.ops.ensureLabel(
      INTAKE_LABEL,
      "0e8a16",
      "Said by the owner through the PM app: a note, a bug or a change",
    );
    this.forget();

    // A note about an existing issue is left on that issue: the place
    // people already look. A rank asked for with it is set there too.
    if (kind === "note" && aboutIssue !== null && mode === "issue") {
      const made = (await this.ops.gh(["-X", "POST", `repos/${this.repo}/issues/${aboutIssue}/comments`], {
        body: `${words}\n\n---\n${what}\n${mark}`,
      })) as { html_url: string };
      // The comment is the note. What follows helps it be found and
      // acted on; none of it failing takes the note back.
      const trouble: string[] = [];
      await this.ops
        .gh(["-X", "POST", `repos/${this.repo}/issues/${aboutIssue}/labels`], { labels: [INTAKE_LABEL] })
        .catch(() => {});
      if (rank !== "keep") {
        await this.ops
          .setRank(aboutIssue, rank)
          .catch((e: Error) => trouble.push(`its rank was not set: ${e.message}`));
      }
      const first = firstLine(words, 80);
      // He spoke about it: someone looks soon, whatever rank the issue keeps.
      const taskRank: Rank = rank === "keep" ? "now" : rank;
      const queued = await enqueue(notePacket(this.repo, this.ops.cfg.queue, aboutIssue, first, taskRank, at), {
        kind: "triage.enqueue",
        args: { issue: aboutIssue, title: first, rank: taskRank, at, note: true },
      }).catch(notQueued);
      const said = queuedSaid(queued, "left");
      const notes = [...trouble, ...(said.note ? [said.note] : [])];
      return {
        issue: aboutIssue,
        url: made.html_url,
        commented: true,
        queued: said.queued,
        ...(notes.length ? { note: notes.join("; ").slice(0, 300) } : {}),
      };
    }

    const filedRank: Rank = rank === "keep" ? ROUTES[kind].rank : rank;
    const labels = [...ROUTES[kind].labels, HUMAN_LABEL, INTAKE_LABEL, ...(cap ? [cap.labels[0]] : [])];

    // As a session: the owner's prompt flow starts one with the words
    // verbatim, and the routing labels are added to what it filed.
    if (mode === "session") {
      const context = [
        aboutIssue !== null ? `About #${aboutIssue}.` : null,
        aboutRow !== null ? `About the roadmap row "${aboutRow}"${cap ? ` of ${cap.title}` : ""}.` : null,
        asked ? `Asked for: ${changeSaid(asked)}.` : null,
        `(${kind}${cap ? `, ${cap.title}` : ""}; from the PM app)`,
      ]
        .filter((l) => l !== null)
        .join(" ");
      const prompt = `${words}\n\n${context}`;
      const made = await this.work.ask({ text: prompt }, (packet) => {
        const issue = Number((packet as { payload?: { issue?: { number?: unknown } } })?.payload?.issue?.number);
        return enqueue(packet, { kind: "prompt.enqueue", args: { issue, text: prompt, at } });
      });
      await this.ops.gh(["-X", "POST", `repos/${this.repo}/issues/${made.issue}/labels`], { labels }).catch(() => {});
      return {
        issue: made.issue,
        url: made.url,
        commented: false,
        queued: made.queued,
        ...(made.note ? { note: made.note } : {}),
      };
    }

    const heading =
      title ??
      (asked
        ? `${cap ? `${cap.title} roadmap` : "Roadmap"}: ${changeSaid(asked, aboutRow)}`
        : `${kind === "bug" ? "Bug" : "Note"}${cap ? ` (${cap.title})` : ""}: ${firstLine(words, 100)}`);
    const made = (await this.ops.gh(["-X", "POST", `repos/${this.repo}/issues`], {
      title: heading.slice(0, 200),
      body: `${words}\n\n---\n${what}\n${mark}`,
      labels: filedRank === "none" ? labels : [...labels, filedRank],
    })) as { number: number; html_url: string; title: string };
    const queued = await enqueue(triagePacket(this.repo, this.ops.cfg.queue, made.number, made.title, filedRank, at), {
      kind: "triage.enqueue",
      args: { issue: made.number, title: made.title, rank: filedRank, at },
    }).catch(notQueued);
    return { issue: made.number, url: made.html_url, commented: false, ...queuedSaid(queued, "filed") };
  }
}

function queuedSaid(queued: RunResult, done: string): { queued: boolean; note?: string } {
  if (queued.code === 0) return { queued: true };
  return {
    queued: false,
    note: `${done}, not queued: ${(queued.stderr || queued.stdout).trim().split("\n").pop()?.slice(0, 200) ?? ""}`,
  };
}

/** A change asked for, in words: "move to Planned: immediately". */
export function changeSaid(
  asked: { action: Change; to: Phase | null; title: string | null },
  row: string | null = null,
): string {
  const named = row ? ` "${row}"` : "";
  switch (asked.action) {
    case "move":
      return `move${named}${asked.to ? ` to "${PHASE_TITLE[asked.to]}"` : ""}`;
    case "add":
      return `add an item${asked.title ? ` "${asked.title}"` : ""}${asked.to ? ` under "${PHASE_TITLE[asked.to]}"` : ""}`;
    case "add-under":
      return `add a sub-item${asked.title ? ` "${asked.title}"` : ""} under${named || " this item"}`;
    case "built":
      return `mark${named || " this item"} as built`;
    case "drop":
      return `drop${named || " this item"} from the roadmap`;
    case "rewrite":
      return `rewrite${named || " this item"}`;
  }
}

/** Build again, on a machine, the task the edge asked for when it filed
 * the issue itself: the arguments are checked here, never trusted. */
export function laterPacket(repo: string, queue: string, a: Record<string, unknown>) {
  const issue = Number(a.issue);
  if (!Number.isSafeInteger(issue) || issue <= 0) throw new OpError(400, "issue: a positive whole number");
  if (typeof a.title !== "string" || a.title.trim() === "" || a.title.length > 300)
    throw new OpError(400, "title: required, at most 300 characters");
  if (a.rank !== "none" && !(RANKS as readonly unknown[]).includes(a.rank))
    throw new OpError(400, `rank: one of ${RANKS.join(", ")}, none`);
  if (typeof a.at !== "string" || a.at === "") throw new OpError(400, "at: required");
  return a.note === true
    ? notePacket(repo, queue, issue, a.title, a.rank as Rank, a.at)
    : triagePacket(repo, queue, issue, a.title, a.rank as Rank, a.at);
}
