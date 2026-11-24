// A roadmap change asked for in the app, made by the platform itself:
// the change applied to the roadmap file where it lives, and opened as
// a pull request against that file and no other (#13497, second half).
//
// The ask arrives already checked by Plan.intake: one of the CHANGES
// against a named row of a capability's file, or a new item to add.
// What can be done mechanically is done here — a row added, moved,
// marked built, dropped, or retitled — and what cannot is refused
// with the reason: a sub-item, whose rows have no rows of their own
// in the file's tables, and a rewrite in the owner's words, which is
// prose and not a cell. A refusal is not a failure of the intake: the
// issue and the task stand, as they did before this path existed.
//
// Everything reaches GitHub through Ops.gh, as everywhere in the
// console, so this runs unchanged on a machine (the fleet binary's gh)
// and at the edge (GitHub's REST API). The pull request is made with
// the contents API — one branch, one write to the one file, one pull
// request — so its diff touches only the roadmap file by construction.
import { OpError } from "./errors.ts";
import type { Ops } from "./ops.ts";
import type { Capability, Change, Phase } from "./plan.ts";

/** A change to one roadmap file, in the terms Plan.intake checks. */
export interface RoadmapChange {
  action: Change;
  /** The row it is about, by its title in the file. */
  row?: string | null;
  /** A new item's title (add), or a row's new title (rewrite). */
  title?: string | null;
  /** The phase it goes to (add, move). */
  to?: Phase | null;
  /** A new item's intent: the owner's words. */
  intent?: string | null;
  /** An issue a new item is about, named in its Links. */
  issue?: number | null;
}

/** Why this change cannot be made mechanically, or null when it can. */
export function roadmapChangeRefusal(change: RoadmapChange): string | null {
  switch (change.action) {
    case "add":
      return change.title ? null : "an added item needs a title";
    case "add-under":
      return "a sub-item has no row of its own in the roadmap file's tables";
    case "move":
      return !change.row ? "a move needs the row it moves" : !change.to ? "a move needs where it goes" : null;
    case "built":
      return change.row ? null : "marking built needs the row it is about";
    case "drop":
      return change.row ? null : "a drop needs the row it is about";
    case "rewrite":
      return !change.row
        ? "a rewrite needs the row it is about"
        : !change.title
          ? "only a retitle can be made mechanically; a rewrite in words stays with the issue"
          : null;
  }
}

const cells = (line: string): string[] =>
  line
    .trim()
    .replace(/^\||\|$/g, "")
    .split(/(?<!\\)\|/)
    .map((c) => c.trim().replace(/\\\|/g, "|"));

const cellOut = (value: string): string =>
  value
    .replace(/\|/g, "\\|")
    .replace(/\s*\n\s*/g, " ")
    .trim();

const rowLine = (values: string[]): string => `| ${values.map(cellOut).join(" | ")} |`;

/** The phase a section holds, by its heading, as parseRoadmap reads it. */
const sectionPhase = (heading: string): Phase | null => {
  const word = heading.toLowerCase();
  return word.startsWith("now")
    ? "now"
    : word.startsWith("next")
      ? "next"
      : word.startsWith("later")
        ? "later"
        : word.startsWith("done") || word.startsWith("built")
          ? "built"
          : null;
};

/** The section a phase asked for lives in: work in progress sits under Now. */
const homeOf = (to: Phase): Phase => (to === "doing" ? "now" : to);

const SECTION_WORD: Record<Phase, string> = {
  doing: "Now",
  now: "Now",
  next: "Next",
  later: "Later",
  built: "Done",
};

interface RowAt {
  line: number;
  values: string[];
}
interface TableAt {
  header: number;
  columns: string[];
  rows: RowAt[];
}
interface SectionAt {
  heading: string;
  phase: Phase | null;
  tables: TableAt[];
}

/** The Title tables of a roadmap file, section by section, with the
 * line each row sits on. Mirrors parseRoadmap's reading of the file. */
function scan(lines: string[]): SectionAt[] {
  const sections: SectionAt[] = [];
  let section: SectionAt | null = null;
  let table: TableAt | null = null;
  let inRun = false;
  lines.forEach((line, at) => {
    const heading = /^##\s+(.*)$/.exec(line);
    if (heading) {
      const name = heading[1].trim();
      section = { heading: name, phase: sectionPhase(name), tables: [] };
      sections.push(section);
      table = null;
      inRun = false;
      return;
    }
    if (!line.trim().startsWith("|")) {
      table = null;
      inRun = false;
      return;
    }
    if (!section) return;
    if (!inRun) {
      inRun = true;
      const columns = cells(line).map((c) => c.toLowerCase());
      table = columns[0] === "title" ? { header: at, columns, rows: [] } : null;
      if (table) section.tables.push(table);
      return;
    }
    if (!table) return;
    const values = cells(line);
    if (values.every((c) => /^:?-+:?$/.test(c))) return;
    table.rows.push({ line: at, values });
  });
  return sections;
}

function tableFor(sections: SectionAt[], phase: Phase): TableAt {
  const held = sections.filter((s) => s.phase === phase);
  const withTable = held.find((s) => s.tables.length > 0);
  if (withTable) return withTable.tables[0];
  if (held.length > 0) throw new OpError(400, `the roadmap's ${held[0].heading} section has no table of rows`);
  throw new OpError(400, `the roadmap file has no ${SECTION_WORD[phase]} section`);
}

function oneRow(sections: SectionAt[], title: string): { row: RowAt; table: TableAt } {
  const found: { row: RowAt; table: TableAt }[] = [];
  for (const section of sections)
    for (const table of section.tables)
      for (const row of table.rows)
        if ((row.values[0] ?? "").toLowerCase() === title.trim().toLowerCase()) found.push({ row, table });
  if (found.length === 0) throw new OpError(400, `no roadmap row is titled "${title}"`);
  if (found.length > 1) throw new OpError(400, `more than one roadmap row is titled "${title}"`);
  return found[0];
}

const setCell = (row: RowAt, table: TableAt, name: string, value: string) => {
  const at = table.columns.indexOf(name);
  if (at < 0) throw new OpError(400, `the roadmap table has no ${name} column`);
  while (row.values.length <= at) row.values.push("");
  row.values[at] = value;
};

/** Where a new row goes in a table: after its last row, or after the
 * header's separator when the table is still empty. */
const insertAt = (lines: string[], table: TableAt): number => {
  if (table.rows.length > 0) return table.rows[table.rows.length - 1].line + 1;
  const sep = lines[table.header + 1] ?? "";
  return table.header + (cells(sep).every((c) => /^:?-+:?$/.test(c)) ? 2 : 1);
};

/**
 * Apply one change to a roadmap file's text. Pure: the same reading of
 * the file as parseRoadmap, and nothing outside the one row is touched.
 */
export function applyRoadmapChange(markdown: string, change: RoadmapChange): string {
  const refusal = roadmapChangeRefusal(change);
  if (refusal) throw new OpError(400, refusal);
  const lines = markdown.split("\n");
  const sections = scan(lines);
  switch (change.action) {
    case "add": {
      const to = homeOf(change.to ?? "next");
      const table = tableFor(sections, to);
      const values = table.columns.map((c) =>
        c === "title"
          ? (change.title as string)
          : c === "status"
            ? to === "built"
              ? "shipped"
              : "planned"
            : c === "intent"
              ? (change.intent ?? "")
              : c === "links"
                ? change.issue
                  ? `#${change.issue}`
                  : ""
                : "",
      );
      lines.splice(insertAt(lines, table), 0, rowLine(values));
      break;
    }
    case "move": {
      const { row } = oneRow(sections, change.row as string);
      const text = lines[row.line];
      lines.splice(row.line, 1);
      const table = tableFor(scan(lines), homeOf(change.to as Phase));
      lines.splice(insertAt(lines, table), 0, text);
      break;
    }
    case "built": {
      const { row, table } = oneRow(sections, change.row as string);
      setCell(row, table, "status", "shipped");
      lines[row.line] = rowLine(row.values);
      break;
    }
    case "drop": {
      const { row } = oneRow(sections, change.row as string);
      lines.splice(row.line, 1);
      break;
    }
    case "rewrite": {
      const { row, table } = oneRow(sections, change.row as string);
      setCell(row, table, "title", change.title as string);
      lines[row.line] = rowLine(row.values);
      break;
    }
    case "add-under":
      throw new OpError(400, "a sub-item has no row of its own in the roadmap file's tables");
  }
  const out = lines.join("\n");
  if (out === markdown) throw new OpError(400, "the change leaves the roadmap file as it was");
  return out;
}

const encode64 = (text: string): string => {
  const bytes = new TextEncoder().encode(text);
  let bin = "";
  bytes.forEach((b) => {
    bin += String.fromCharCode(b);
  });
  return btoa(bin);
};

const decode64 = (encoded: string): string => {
  const bytes = Uint8Array.from(atob(encoded), (c) => c.charCodeAt(0));
  return new TextDecoder().decode(bytes);
};

export interface RoadmapPr {
  url: string;
  number: number;
  branch: string;
  file: string;
}

/**
 * Make one roadmap change as a pull request: read the file from main,
 * apply the change, write the file — and only the file — to a fresh
 * branch, and open the pull request against main.
 */
export async function openRoadmapChangePr(
  ops: Ops,
  repo: string,
  cap: Capability,
  change: RoadmapChange,
  opts: { said: string; words: string; issue: number; now: Date },
): Promise<RoadmapPr> {
  const file = cap.file;
  if (!file) throw new OpError(400, `${cap.title} has no roadmap file`);
  const refusal = roadmapChangeRefusal(change);
  if (refusal) throw new OpError(400, refusal);
  const got = (await ops.gh([`repos/${repo}/contents/${file}`])) as { content?: string; sha?: string } | null;
  if (typeof got?.content !== "string" || typeof got?.sha !== "string")
    throw new OpError(502, "the roadmap file could not be read");
  const after = applyRoadmapChange(decode64(got.content), change);
  const base = (await ops.gh([`repos/${repo}/git/ref/heads/main`])) as { object?: { sha?: string } } | null;
  if (typeof base?.object?.sha !== "string") throw new OpError(502, "the repository's main branch could not be read");
  const stamp = opts.now
    .toISOString()
    .replace(/[^0-9]/g, "")
    .slice(0, 14);
  const branch = `roadmap/${cap.id}-${stamp}`;
  await ops.gh(["-X", "POST", `repos/${repo}/git/refs`], { ref: `refs/heads/${branch}`, sha: base.object.sha });
  await ops.gh(["-X", "PUT", `repos/${repo}/contents/${file}`], {
    message: `${cap.title} roadmap: ${opts.said}`,
    content: encode64(after),
    branch,
    sha: got.sha,
  });
  const words = opts.words.length > 1000 ? `${opts.words.slice(0, 1000)}…` : opts.words;
  const made = (await ops.gh(["-X", "POST", `repos/${repo}/pulls`], {
    title: `${cap.title} roadmap: ${opts.said}`.slice(0, 200),
    head: branch,
    base: "main",
    body: [
      `Asked for in the PM app, and made by the platform itself: ${opts.said}.`,
      "",
      `The owner's words: ${words}`,
      "",
      `Filed as #${opts.issue}. This pull request changes only \`${file}\`; merging it puts the change on the roadmap, and the roadmap pages read the file from main.`,
    ].join("\n"),
  })) as { number?: number; html_url?: string } | null;
  if (typeof made?.html_url !== "string" || typeof made?.number !== "number")
    throw new OpError(502, "the pull request could not be opened");
  return { url: made.html_url, number: made.number, branch, file };
}
