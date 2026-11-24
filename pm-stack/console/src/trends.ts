// Trends: what was opened, merged, closed and finished, day by day.
//
// Two sources, both read by whoever serves the app (Cloudflare, or a
// machine): GitHub for issues and pull requests, the queue store for
// tasks. Neither needs the other, and neither needs a machine.
//
// GitHub has no "how many merged each day" to ask for, so the history
// is read once and kept. GitHub's list of issues takes a time and
// answers with everything changed at or after it, oldest change first:
// read from ninety days back, a page at a time, each page's last change
// is where the next page starts. That is every issue and pull request
// touched since, each seen at least once in its present state, however
// they change while being read. Once the list has been read to its end
// it is kept up by asking from the same place again: only what changed
// since comes back. What is kept is small: per day, the numbers of what
// was opened and closed, and for a merged pull request what its title
// says it is.
//
// The queue store is asked each time: one read of the finishing events
// by day, which it has an index for.
import { OpError } from "./errors.ts";
import type { Ops } from "./ops.ts";

/** Where the history is kept: texts under names. Cloudflare's own
 * storage there; memory on a machine (which reads it again when it
 * starts, and is not what the app is served from). */
export interface Shelf {
  get(key: string): Promise<string | null>;
  put(key: string, text: string): Promise<void>;
}

export function memoryShelf(): Shelf & { keys: () => string[] } {
  const kept = new Map<string, string>();
  return {
    get: async (key) => kept.get(key) ?? null,
    put: async (key, text) => void kept.set(key, text),
    keys: () => [...kept.keys()].sort(),
  };
}

/** How far back the history goes. */
export const HISTORY_DAYS = 90;
/** GitHub's largest page. */
const PAGE = 100;
/** How many pages one read of the screen waits for. The rest is read
 * in the background (Cloudflare's own clock) or by the next read. */
export const PAGES_A_READ = 3;
/** History read to its end is asked about again after this long. */
export const FRESH_FOR_MS = 5 * 60_000;
/** The windows the screen offers, in days. */
export const WINDOWS = [7, 14, 30, 90] as const;
const TITLE_CHARS = 110;
/** How many of a project's latest features are listed. */
const FEATURES_LISTED = 12;
/** How many kinds of task get a series of their own. */
const TASK_KINDS = 5;

/** What a merged pull request is, by the first word of its title. */
export const KINDS = ["feat", "fix", "docs", "test", "other"] as const;
export type Kind = (typeof KINDS)[number];
const CONVENTIONAL: Record<string, Kind> = {
  feat: "feat",
  feature: "feat",
  fix: "fix",
  bugfix: "fix",
  hotfix: "fix",
  docs: "docs",
  doc: "docs",
  test: "test",
  tests: "test",
  chore: "other",
  refactor: "other",
  perf: "other",
  ci: "other",
  build: "other",
  style: "other",
  revert: "other",
};

/** The projects a pull request is counted under, by its title's scope.
 * The first four are the capabilities the plan is kept by; a scope none
 * of these claims is "other". The order is the order they are shown
 * and coloured in, and it does not change with what is in the window. */
export const PROJECTS: { id: string; title: string; scope: RegExp }[] = [
  { id: "id", title: "id", scope: /^(id|id[-/].+|pkgs\/id.*)$/ },
  {
    id: "fleet",
    title: "Fleet",
    scope: /^(fleet|fleet[-/].+|queue|queue[-/].+|claimant|laptop-agent|opencode.*|lane|lanes|babysitter|route.*)$/,
  },
  { id: "atlas", title: "Atlas", scope: /^atlas([-/].+)?$/ },
  { id: "pm-stack", title: "PM stack", scope: /^(pm|pm[-/].+|aggregator|console|android|dashboard)$/ },
  { id: "mcpx", title: "mcpx", scope: /^mcpx([-/].+)?$/ },
  { id: "other", title: "Everything else", scope: /^/ },
];

/** A title in parts: `feat(pm-stack): the words` is a feature of
 * pm-stack; `id-pg: the words` names its scope and not its kind; a
 * title that is only words is neither. Two kinds joined (`test+docs`)
 * is the first. */
export function titleParts(title: string): { kind: Kind; scope: string; words: string } {
  const text = String(title ?? "").trim();
  const typed = /^([A-Za-z]+)(?:\+[A-Za-z]+)*(?:\(([^)]*)\))?!?:\s+(.*)$/s.exec(text);
  if (typed && Object.hasOwn(CONVENTIONAL, typed[1].toLowerCase())) {
    return {
      kind: CONVENTIONAL[typed[1].toLowerCase()],
      scope: (typed[2] ?? "").trim().toLowerCase(),
      words: typed[3],
    };
  }
  const scoped = /^([A-Za-z0-9][A-Za-z0-9._/-]{0,39}):\s+(.*)$/s.exec(text);
  if (scoped) return { kind: "other", scope: scoped[1].toLowerCase(), words: scoped[2] };
  return { kind: "other", scope: "", words: text };
}

/** The project a scope belongs to. */
export function projectOf(scope: string): string {
  const s = scope.trim().toLowerCase();
  if (s === "") return "other";
  return (PROJECTS.find((p) => p.scope.test(s)) ?? PROJECTS[PROJECTS.length - 1]).id;
}

/** A merged pull request as kept: what it is, whose it is, how long it
 * was open in minutes, the time of day it merged, and its words (kept
 * for a feature only: those are the ones listed). */
type Merged = [kind: Kind, project: string, minutes: number, at: string, words?: string];
/** A day as kept: the numbers of the issues and pull requests opened
 * and closed on it (a number is in a list once, however often its issue
 * is read), and the pull requests merged on it. */
export interface Day {
  io: number[];
  ic: number[];
  bo: number[];
  bc: number[];
  po: number[];
  px: number[];
  pm: Record<string, Merged>;
}
const emptyDay = (): Day => ({ io: [], ic: [], bo: [], bc: [], po: [], px: [], pm: {} });

interface Meta {
  /** The first day the history covers. */
  from: string;
  /** Everything changed before this has been read. */
  cursor: string;
  /** The list has been read to its end at least once. */
  whole: boolean;
  /** When it was last read to its end. */
  readAt: string | null;
}

/** An issue or a pull request, as GitHub's list of issues gives it. */
interface Item {
  number: number;
  title?: string;
  created_at?: string;
  updated_at?: string;
  closed_at?: string | null;
  state?: string;
  labels?: ({ name?: string } | string)[];
  pull_request?: { merged_at?: string | null } | null;
}

const dayOf = (iso: string) => iso.slice(0, 10);
const add = (list: number[], n: number) => {
  if (!list.includes(n)) list.push(n);
};
const isBug = (item: Item) =>
  (item.labels ?? []).some((l) => ["kind/bug", "bug"].includes(typeof l === "string" ? l : (l.name ?? "")));

/** One item put into the days it belongs to. `day` gives the day to
 * write on. Reading the same item again changes nothing; reading it
 * after it changed adds what is new. An issue closed, opened again and
 * closed on a later day stays counted on the first as well: what was
 * true of a day is not taken back. */
export function fold(item: Item, from: string, day: (date: string) => Day): void {
  const n = Number(item.number);
  if (!Number.isInteger(n) || n <= 0) return;
  const pr = item.pull_request !== undefined && item.pull_request !== null;
  const created = item.created_at ?? "";
  if (created && dayOf(created) >= from) {
    add(pr ? day(dayOf(created)).po : day(dayOf(created)).io, n);
    if (!pr && isBug(item)) add(day(dayOf(created)).bo, n);
  }
  const merged = pr ? (item.pull_request?.merged_at ?? null) : null;
  if (merged) {
    if (dayOf(merged) < from) return;
    const parts = titleParts(item.title ?? "");
    const minutes = created ? Math.max(0, Math.round((Date.parse(merged) - Date.parse(created)) / 60_000)) : 0;
    const record: Merged = [parts.kind, projectOf(parts.scope), minutes, merged.slice(11, 16)];
    if (parts.kind === "feat") record.push(parts.words.slice(0, TITLE_CHARS));
    day(dayOf(merged)).pm[String(n)] = record;
    return;
  }
  const closed = item.state === "closed" ? (item.closed_at ?? null) : null;
  if (!closed || dayOf(closed) < from) return;
  if (pr) add(day(dayOf(closed)).px, n);
  else {
    add(day(dayOf(closed)).ic, n);
    if (isBug(item)) add(day(dayOf(closed)).bc, n);
  }
}

/** The middle of some numbers, or null of none. */
export function median(values: number[]): number | null {
  if (values.length === 0) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
}

/** The days of a window ending today, oldest first, as UTC dates. */
export function daysBack(today: string, count: number): string[] {
  const end = Date.parse(`${today}T00:00:00Z`);
  return Array.from({ length: count }, (_, i) =>
    new Date(end - (count - 1 - i) * 86_400_000).toISOString().slice(0, 10),
  );
}

const P = "CASE WHEN json_valid(t.payload) THEN t.payload ELSE '{}' END";
/** Tasks finished, by the day of their last finishing and their kind:
 * each task once, and only while it still stands as done. The same
 * events, by the same index, as the Done screen counts (src/work.ts). */
export const TASKS_BY_DAY_SQL = `SELECT day, kind, count(*) AS n FROM (
  SELECT t.id AS id, substr(max(e.created_at),1,10) AS day, coalesce(json_extract(${P},'$.kind'),'') AS kind
  FROM events e CROSS JOIN tasks t ON t.id = e.task_id
  WHERE e.kind IN ('completed','staleness_autocomplete') AND e.created_at >= ? AND t.queue = ? AND t.status = 'done'
  GROUP BY t.id
) GROUP BY day, kind ORDER BY day ASC, n DESC, kind ASC`;

export interface TrendsDoc {
  /** The window asked for, in days; `days` covers twice that (or all
   * there is), so the window can be set against the one before it. */
  window: number;
  days: string[];
  today: string;
  history: { from: string; whole: boolean; readTo: string; readAt: string | null };
  projects: { id: string; title: string }[];
  merged: {
    all: number[];
    kinds: Record<Kind, number[]>;
    projects: Record<string, number[]>;
    /** The middle time from opening to merging, in minutes; null on a day nothing merged. */
    minutes: (number | null)[];
  };
  features: {
    projects: Record<string, number[]>;
    latest: Record<string, { number: number; title: string; at: string; url: string }[]>;
  };
  issues: { opened: number[]; closed: number[] };
  bugs: { opened: number[]; closed: number[] };
  prs: { opened: number[]; closed: number[] };
  tasks: { all: number[]; kinds: { kind: string; days: number[] }[]; since: string | null } | { error: string };
  /** Something could not be read just now: what is shown is what was kept. */
  partial?: boolean;
  why?: string;
}

export class Trends {
  private reading: Promise<Meta> | null = null;

  constructor(
    private ops: Ops,
    private shelf: Shelf,
    private clock: () => Date = () => new Date(),
  ) {}

  private get repo(): string {
    return this.ops.cfg.repo;
  }

  private async meta(): Promise<Meta> {
    const text = await this.shelf.get("meta");
    if (text) {
      try {
        const m = JSON.parse(text) as Meta;
        if (typeof m.from === "string" && typeof m.cursor === "string") {
          return { from: m.from, cursor: m.cursor, whole: m.whole === true, readAt: m.readAt ?? null };
        }
      } catch {
        // Not what was written: start again.
      }
    }
    const from = new Date(this.clock().getTime() - HISTORY_DAYS * 86_400_000).toISOString().slice(0, 10);
    return { from, cursor: `${from}T00:00:00Z`, whole: false, readAt: null };
  }

  private async day(date: string): Promise<Day> {
    const text = await this.shelf.get(`day:${date}`);
    if (!text) return emptyDay();
    try {
      return { ...emptyDay(), ...(JSON.parse(text) as Partial<Day>) };
    } catch {
      return emptyDay();
    }
  }

  /** When the history next wants reading on: now (0) while it has not
   * been read to its end, otherwise `every` after it last was. */
  async dueAt(every = FRESH_FOR_MS): Promise<number> {
    const m = await this.meta();
    return m.whole && m.readAt !== null ? Date.parse(m.readAt) + every : 0;
  }

  /** Whether the history wants reading on: it has not been read to its
   * end, or that was a while ago. */
  async due(staleMs = FRESH_FOR_MS): Promise<boolean> {
    return this.clock().getTime() >= (await this.dueAt(staleMs));
  }

  /** Read on from where the last read stopped, at most `pages` pages.
   * One at a time: a second caller waits for the first and gets what it
   * got. A page that could not be read stops it where it is, with
   * everything before that kept. */
  advance(pages = PAGES_A_READ): Promise<Meta> {
    if (!this.reading) {
      this.reading = this.readOn(pages).finally(() => {
        this.reading = null;
      });
    }
    return this.reading;
  }

  private async readOn(pages: number): Promise<Meta> {
    const meta = await this.meta();
    const open = new Map<string, Day>();
    const touched = new Set<string>();
    const save = async () => {
      for (const date of touched) await this.shelf.put(`day:${date}`, JSON.stringify(open.get(date)));
      touched.clear();
      await this.shelf.put("meta", JSON.stringify(meta));
    };
    try {
      for (let page = 0; page < pages; page++) {
        const path = `repos/${this.repo}/issues?state=all&sort=updated&direction=asc&since=${meta.cursor}&per_page=${PAGE}`;
        const items = (await this.ops.gh([path])) as Item[] | null;
        if (!Array.isArray(items)) throw new OpError(502, "GitHub's list of issues was not a list");
        // The days this page writes on, read first.
        const dates = new Set<string>();
        for (const item of items) fold(item, meta.from, (date) => (dates.add(date), emptyDay()));
        for (const date of dates) if (!open.has(date)) open.set(date, await this.day(date));
        for (const item of items) {
          fold(item, meta.from, (date) => {
            touched.add(date);
            return open.get(date) as Day;
          });
        }
        const last = items.length > 0 ? (items[items.length - 1].updated_at ?? "") : "";
        if (items.length < PAGE) {
          // The end of the list: everything changed up to now is read.
          if (last > meta.cursor) meta.cursor = last;
          meta.whole = true;
          meta.readAt = this.clock().toISOString();
          break;
        }
        // A whole page that ends where it began (a hundred things
        // changed in one second) would be read for ever: step past it.
        meta.cursor =
          last > meta.cursor ? last : new Date(Date.parse(meta.cursor) + 1000).toISOString().replace(".000Z", "Z");
        await save();
      }
    } finally {
      // What was read before a failure is kept.
      await save();
    }
    return meta;
  }

  /** The document the Trends screen draws: `window` days, set against
   * the same number before them. */
  async read(asked: string | null, fresh = false): Promise<TrendsDoc> {
    const window = (WINDOWS as readonly number[]).includes(Number(asked)) ? Number(asked) : 30;
    let why: string | undefined;
    if (fresh || (await this.due())) {
      try {
        await this.advance();
      } catch (error) {
        why = `GitHub's history could not be read on: ${(error as Error).message}`;
      }
    }
    const meta = await this.meta();
    const today = this.clock().toISOString().slice(0, 10);
    const days = daysBack(today, Math.min(window * 2, HISTORY_DAYS)).filter((d) => d >= meta.from);
    const kept = await Promise.all(days.map((d) => this.day(d)));
    const zeros = () => days.map(() => 0);
    const kinds = Object.fromEntries(KINDS.map((k) => [k, zeros()])) as Record<Kind, number[]>;
    const projects = Object.fromEntries(PROJECTS.map((p) => [p.id, zeros()]));
    const featured = Object.fromEntries(PROJECTS.map((p) => [p.id, zeros()]));
    const latest: TrendsDoc["features"]["latest"] = Object.fromEntries(PROJECTS.map((p) => [p.id, []]));
    const minutes: (number | null)[] = [];
    const inWindow = days.length - Math.min(window, days.length);
    kept.forEach((day, i) => {
      const merged = Object.entries(day.pm);
      minutes.push(median(merged.map(([, m]) => m[2])));
      for (const [number, [kind, project, , at, words]] of merged) {
        kinds[KINDS.includes(kind) ? kind : "other"][i]++;
        const p = Object.hasOwn(projects, project) ? project : "other";
        projects[p][i]++;
        if (kind !== "feat") continue;
        featured[p][i]++;
        if (i >= inWindow) {
          latest[p].push({
            number: Number(number),
            title: words ?? "",
            at: `${days[i]}T${at}:00Z`,
            url: `https://github.com/${this.repo}/pull/${number}`,
          });
        }
      }
    });
    for (const list of Object.values(latest)) {
      list.sort((a, b) => (a.at < b.at ? 1 : a.at > b.at ? -1 : b.number - a.number));
      list.length = Math.min(list.length, FEATURES_LISTED);
    }
    let tasks: TrendsDoc["tasks"];
    try {
      const rows = (await this.ops.sql(TASKS_BY_DAY_SQL, [`${days[0] ?? today}T00:00:00Z`, this.ops.cfg.queue])) as {
        day: string;
        kind: string;
        n: number;
      }[];
      const at = new Map(days.map((d, i) => [d, i]));
      const totals = new Map<string, number>();
      for (const r of rows) totals.set(r.kind, (totals.get(r.kind) ?? 0) + Number(r.n));
      const named = [...totals.entries()]
        .filter(([kind]) => kind !== "")
        .sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1))
        .slice(0, TASK_KINDS)
        .map(([kind]) => kind);
      const series = new Map<string, number[]>([...named, "other"].map((kind) => [kind, zeros()]));
      const all = zeros();
      for (const r of rows) {
        const i = at.get(r.day);
        if (i === undefined) continue;
        all[i] += Number(r.n);
        (series.get(named.includes(r.kind) ? r.kind : "other") as number[])[i] += Number(r.n);
      }
      tasks = {
        all,
        kinds: [...series.entries()]
          .filter(([, list]) => list.some((n) => n > 0))
          .map(([kind, list]) => ({ kind, days: list })),
        since: rows.length > 0 ? rows[0].day : null,
      };
    } catch (error) {
      tasks = { error: (error as Error).message };
      why = why ?? `The queue's finished tasks could not be read: ${(error as Error).message}`;
    }
    const count = (pick: (day: Day) => number[]) => kept.map((day) => pick(day).length);
    return {
      window,
      days,
      today,
      history: { from: meta.from, whole: meta.whole, readTo: meta.cursor, readAt: meta.readAt },
      projects: PROJECTS.map(({ id, title }) => ({ id, title })),
      merged: { all: kept.map((day) => Object.keys(day.pm).length), kinds, projects, minutes },
      features: { projects: featured, latest },
      issues: { opened: count((d) => d.io), closed: count((d) => d.ic) },
      bugs: { opened: count((d) => d.bo), closed: count((d) => d.bc) },
      prs: { opened: count((d) => d.po), closed: count((d) => d.px) },
      tasks,
      ...(why === undefined ? {} : { partial: true, why }),
    };
  }
}
