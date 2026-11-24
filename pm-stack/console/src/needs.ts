// What waits on the owner, by GitHub's labels.
//
// Two labels say that something waits on him: `user/review` and
// `user/escalated`. The laptop's dashboard censuses them
// (pm-stack/aggregator/src/needs-drew.ts) and the console has always
// read the dashboard's answer. This module is what the list is made of
// whoever reads it: how an item's body is read for a reason and for the
// mark of something only a person's hands can do (FDR-0013), and the
// three lists the app shows. The dashboard's two readers are copied
// here, because that module needs a machine's files and cannot be part
// of the Worker; a test holds each copy to the original.
//
// It also reads the list from GitHub directly, for a side that has no
// dashboard to ask: each repository's own list of open issues carrying
// a label (which has the pull requests too). That list is asked for
// like any other: by its tag, so that one that has not changed costs
// nothing; and never through the search, which is counted by the
// minute, lags behind, and takes issues and pull requests in separate
// queries.

export const NEEDS_LABELS = ["user/review", "user/escalated"] as const;
export type NeedsLabel = (typeof NEEDS_LABELS)[number];

/** What only a person's hands can do (FDR-0013). */
export const HANDS_KINDS = ["physical", "interactive-login", "payment", "signature"] as const;
export type HandsKind = (typeof HANDS_KINDS)[number];

export interface NeedsHands {
  kind: HandsKind;
  /** The one action, when the mark states it. */
  action: string | null;
}

/** One open issue or pull request that carries one of the labels. */
export interface NeedsItem {
  /** "owner/repo#123". */
  ref: string;
  kind: "issue" | "pr";
  title: string;
  url: string;
  /** Which of the two labels it carries (one at least). */
  needsLabels: string[];
  /** A reason its body states, or null: nothing is made up for one that states none. */
  reason: string | null;
  updatedAt: string;
  hands?: NeedsHands | null;
}

/** The three lists the app shows, and whether they are the whole of it. */
export interface Census {
  /** When the dashboard censused the list. Null from a reader that
   * asks GitHub itself: the section's own time says when it was read,
   * and a time in here would make a list that has not changed look
   * new with every read. */
  at: string | null;
  /** Every repository's list of each label was read to its end. */
  complete: boolean;
  /** What could not be read, when not everything was: a repository, or
   * a repository and the label asked for. Absent from a reader that
   * does not say (the dashboard says only that something is missing). */
  unread?: string[];
  hands: { ref: string; title: string; url: string; pr: boolean; updated: string; kind?: string; action: string }[];
  reviews: { ref: string; title: string; url: string; pr: boolean; updated: string }[];
  other: { ref: string; title: string; url: string; pr: boolean; updated: string; why: string }[];
}

function clip(value: unknown, max: number): string {
  const s = typeof value === "string" ? value : "";
  return s.length > max ? `${s.slice(0, max - 1)}…` : s;
}

/** A time to the second, as the app reads one: cut there, with nothing
 * put in the place of what was cut. (Shortened like a title, with a
 * mark at the cut, it is no longer a time, and the app showed nothing
 * for it.) */
function toTheSecond(value: unknown): string {
  return typeof value === "string" ? value.slice(0, 19) : "";
}

/** At most `max` UTF-16 units, and never half of a pair at the cut. */
function capUnits(text: string, max: number): string {
  if (text.length <= max) return text;
  let end = max;
  const before = text.charCodeAt(end - 1);
  const after = text.charCodeAt(end);
  if (before >= 0xd800 && before <= 0xdbff && after >= 0xdc00 && after <= 0xdfff) end--;
  return text.slice(0, end);
}

/** A reason the body states outright: an "Escalated: …" or "Reason: …"
 * line. Anything else is null. */
export function needsReason(body: unknown): string | null {
  if (typeof body !== "string") return null;
  const m = /(?:^|\n)\s*(?:escalated(?:\s+to\s+drew)?|reason)\s*[:\-—]\s*(.+)/i.exec(body);
  const reason = m?.[1]?.trim();
  return reason ? capUnits(reason, 240) : null;
}

/** The hands mark a body carries, if any:
 * `<!-- hands:v1 kind=interactive-login action="one sentence" -->`.
 * A mark with a kind that is not one of the four is not a hands mark. */
export function needsHands(body: unknown): NeedsHands | null {
  if (typeof body !== "string") return null;
  const m = /<!--\s*hands:v1\s+kind=([a-z-]+)(?:\s+action="([^"]*)")?\s*-->/.exec(body);
  if (!m) return null;
  const kind = (HANDS_KINDS as readonly string[]).includes(m[1]) ? (m[1] as HandsKind) : null;
  if (!kind) return null;
  const action = m[2]?.trim();
  return { kind, action: action ? capUnits(action, 240) : null };
}

/** The three lists, from the items in the order they are to be shown.
 * A hands mark wins; then a review that was asked for; everything else
 * was put on the owner by an agent with no stated action, and is the
 * fleet's to decide (it is listed so that nothing is hidden). */
export function censusOf(items: NeedsItem[], at: string | null, complete: boolean, unread?: string[]): Census {
  const base = (i: NeedsItem) => ({
    ref: i.ref,
    title: clip(i.title, 200),
    url: i.url,
    pr: i.kind === "pr",
    updated: toTheSecond(i.updatedAt),
  });
  return {
    at,
    complete,
    ...(unread === undefined ? {} : { unread }),
    hands: items
      .filter((i) => i.hands?.kind)
      .map((i) => ({ ...base(i), kind: i.hands?.kind, action: clip(i.hands?.action, 400) })),
    reviews: items.filter((i) => !i.hands?.kind && i.needsLabels.includes("user/review")).map(base),
    other: items
      .filter((i) => !i.hands?.kind && !i.needsLabels.includes("user/review"))
      .map((i) => ({ ...base(i), why: clip(i.reason, 300) })),
  };
}

/** How many of a repository's issues one page of its list holds, and
 * how many pages of one list are read before it is said to be cut
 * short. (A thousand open items under one label is not a list anyone
 * is working from.) */
export const NEEDS_PER_PAGE = 100;
export const NEEDS_PAGES = 10;

/** The address of one page of a repository's open issues and pull
 * requests carrying a label, as GitHub's REST API has it. */
export function needsListPath(repo: string, label: NeedsLabel, page: number): string {
  return `repos/${repo}/issues?state=open&labels=${encodeURIComponent(label)}&per_page=${NEEDS_PER_PAGE}&page=${page}`;
}

function labelNames(raw: unknown): string[] {
  if (!Array.isArray(raw)) return [];
  const out: string[] = [];
  for (const l of raw) {
    if (typeof l === "string") out.push(l);
    else if (l && typeof l === "object" && typeof (l as { name?: unknown }).name === "string") {
      out.push((l as { name: string }).name);
    }
  }
  return out;
}

/** The items one page of a repository's list holds. Only what is open
 * and has a number is taken: the list was asked for open issues, and a
 * page that says otherwise is not believed. */
export function needsItems(repo: string, page: unknown): (NeedsItem & { number: number })[] {
  if (!Array.isArray(page)) return [];
  const out: (NeedsItem & { number: number })[] = [];
  for (const raw of page as Record<string, unknown>[]) {
    if (!raw || typeof raw !== "object") continue;
    if (raw.state !== "open" || typeof raw.number !== "number" || !Number.isSafeInteger(raw.number)) continue;
    const names = labelNames(raw.labels);
    out.push({
      ref: `${repo}#${raw.number}`,
      number: raw.number,
      kind: raw.pull_request ? "pr" : "issue",
      title: typeof raw.title === "string" ? raw.title : `${repo}#${raw.number}`,
      url: typeof raw.html_url === "string" ? raw.html_url : `https://github.com/${repo}/issues/${raw.number}`,
      needsLabels: names.filter((n) => (NEEDS_LABELS as readonly string[]).includes(n)),
      reason: needsReason(raw.body),
      updatedAt: typeof raw.updated_at === "string" ? raw.updated_at : "",
      hands: needsHands(raw.body),
    });
  }
  return out;
}

/** The HTTP status in what a failed call to GitHub said: as the fleet
 * binary's `gh` words it ("… (HTTP 404)"), as GitHub's own body does
 * ("status":"404"), or as the edge says of an answer with no body.
 * Null when it names none: no answer at all, or a call not made. */
export function githubStatus(message: string): number | null {
  const m =
    /\(HTTP (\d{3})\)/.exec(message) ??
    /"status":\s*"?(\d{3})"?/.exec(message) ??
    /GitHub answered http (\d{3})/.exec(message);
  if (m) return Number(m[1]);
  return /"message":\s*"Not Found"/.test(message) ? 404 : null;
}

/** A failure that is one repository's own: it is not there for this
 * caller (404), keeps no issues (410), or this caller may not read it
 * (a 403 that is not GitHub saying it was called too often). The other
 * repositories are read all the same. Any other failure (called too
 * often, out of reach, GitHub's own trouble) would be no different for
 * the next list, and asking on is what GitHub warns callers off. */
function repositoryOnly(message: string): boolean {
  const status = githubStatus(message);
  if (status === 404 || status === 410) return true;
  return status === 403 && !/rate limit/i.test(message);
}

/** Read the whole list from GitHub. `get` answers one path of the REST
 * API with its parsed body, and throws when it cannot. A repository
 * that cannot be read by this caller is named in `unread` and the rest
 * is still shown. Any other failure ends the read where it is and is
 * thrown; so is there being no repository that could be read (an empty
 * list would say that nothing waits). */
export async function readCensus(repos: readonly string[], get: (path: string) => Promise<unknown>): Promise<Census> {
  const byRef = new Map<string, NeedsItem & { number: number; repo: string }>();
  /** Each list that was not read to its end: not at all, or cut short. */
  const missed: { repo: string; label: NeedsLabel; cut: boolean }[] = [];
  let asked = 0;
  let why = "";
  for (const repo of repos) {
    for (const label of NEEDS_LABELS) {
      asked++;
      try {
        for (let page = 1; ; page++) {
          const raw = await get(needsListPath(repo, label, page));
          if (!Array.isArray(raw)) throw new Error("GitHub's answer was not a list");
          for (const item of needsItems(repo, raw)) {
            // One carrying both labels is in both lists, and once here:
            // its own labels say which it carries.
            byRef.set(item.ref, { ...item, repo });
          }
          if (raw.length < NEEDS_PER_PAGE) break;
          if (page >= NEEDS_PAGES) {
            missed.push({ repo, label, cut: true });
            break;
          }
        }
      } catch (error) {
        const said = (error as Error).message;
        if (!repositoryOnly(said)) throw error;
        missed.push({ repo, label, cut: false });
        if (why === "") why = said;
      }
    }
  }
  if (asked > 0 && missed.filter((m) => !m.cut).length === asked) {
    throw new Error(why.slice(0, 300) || "what needs you could not be read");
  }
  // In the order the repositories are named, and by number within one.
  const order = new Map(repos.map((repo, at) => [repo, at]));
  const items = [...byRef.values()].sort(
    (a, b) => (order.get(a.repo) ?? 0) - (order.get(b.repo) ?? 0) || a.number - b.number,
  );
  return censusOf(items, null, missed.length === 0, unreadSaid(missed));
}

/** What was not read, by name: a repository none of whose lists were
 * read is said once; otherwise the repository and the label; and a list
 * cut short says so. */
function unreadSaid(missed: { repo: string; label: NeedsLabel; cut: boolean }[]): string[] {
  const out: string[] = [];
  for (const repo of [...new Set(missed.map((m) => m.repo))]) {
    const lists = missed.filter((m) => m.repo === repo);
    if (lists.length === NEEDS_LABELS.length && lists.every((m) => !m.cut)) {
      out.push(repo);
      continue;
    }
    for (const m of lists) {
      out.push(m.cut ? `${repo} (${m.label}: only the first ${NEEDS_PAGES * NEEDS_PER_PAGE})` : `${repo} (${m.label})`);
    }
  }
  return out;
}
