// Repo-file roadmap source (#1738): parse the repos' own roadmap
// records — developing-today/code planning/roadmaps/*.md and
// dezren39/darwin-nix ROADMAP.md — into RoadmapRow records, so /roadmap
// and /api/roadmap.json show them alongside the Grist table rows
// instead of drifting from them (the id program had a full repo record
// and no Grist presence at all).
//
// Both file families carry the same table shape: a "Title" column,
// optional Status / Horizon / Intent / Links columns, grouped under
// ## headings. Code files put the horizon in the heading ("## Now —
// the path to Phase 12") and the area in the file ("# Roadmap: id");
// the nix ROADMAP.md puts the horizon in the table and the area in
// each ## heading. The parser handles both.
//
// Merge rule (acceptance on #1738): a title present in both sources
// collapses to the repo-file row; the Grist duplicate is noted in that
// row's notes (provenance) and its links are unioned in.
//
// Empty sources say why (#1500): every source section reports one of
// ok | missing | parse-empty | unreadable with a reason, instead of
// silently contributing zero rows.

import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { isValidRef, refsInText, type Link, type RoadmapRow } from "../model.ts";
import { validateRoadmapHorizon } from "../roadmap-horizon.ts";
import { normalizeTitle } from "../roadmap-reconcile.ts";

export type RepoRoadmapSectionState = "ok" | "missing" | "parse-empty" | "unreadable";

export interface RepoRoadmapSection {
  /** Human label of the source: "code planning/roadmaps" / "nix ROADMAP.md". */
  source: string;
  state: RepoRoadmapSectionState;
  /** Why the section is empty (or what it read, when ok). */
  reason: string;
  rowCount: number;
}

export interface RepoRoadmaps {
  rows: RoadmapRow[];
  sections: RepoRoadmapSection[];
}

export interface RepoRoadmapFileMeta {
  /** Repo the file lives in ("developing-today/code") — the target for
   * bare #n refs and the blob link's repo. */
  repo: string;
  /** Repo-relative path ("planning/roadmaps/id.md"). */
  path: string;
  /** Area for rows when the file's own H1 does not name one. */
  program?: string;
  /** Nix style: the area is each ## section heading, not the file. */
  areaFromSections?: boolean;
}

type ParsedRow = Omit<RoadmapRow, "id">;

const HORIZON_BUCKETS = ["Now", "Next", "Later", "Ideas", "Ongoing"];

/** GitHub's heading-anchor slug: lowercase, drop punctuation, spaces to hyphens. */
export function githubSlug(heading: string): string {
  return heading
    .toLowerCase()
    .replace(/[^\p{L}\p{N} _-]/gu, "")
    .replace(/ /g, "-");
}

/** Split a markdown table row into cells on unescaped pipes. */
function splitCells(line: string): string[] {
  const parts = line.split(/(?<!\\)\|/);
  // Leading/trailing pipes produce empty edge cells.
  if (parts.length && parts[0].trim() === "") parts.shift();
  if (parts.length && parts[parts.length - 1].trim() === "") parts.pop();
  return parts.map((c) => c.replace(/\\\|/g, "|").trim());
}

/** Plain-text form of a table cell: links collapse to their text. */
function cellText(cell: string): string {
  return cell
    .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/[*_`]/g, "")
    .trim();
}

const SHORTHAND_REPOS = new Map([
  ["nix", "dezren39/darwin-nix"],
  ["code", "developing-today/code"],
  ["opencode", "developing-today-forks/opencode"],
  ["openchamber", "developing-today-forks/openchamber"],
]);

/** Refs named by a roadmap table cell: full GitHub URLs (via
 * refsInText), qualified owner/repo#n tokens, the repos' own shorthand
 * (nix#203, code#1500), and bare #n refs against the file's repo. */
export function roadmapRefsInText(text: string, ownRepo: string): Link[] {
  const refs = new Set<string>(refsInText(text));
  for (const m of text.matchAll(/(?:^|[\s,;(\[`])((?:[\w.-]+)\/(?:[\w.-]+))#(\d+)/g)) {
    refs.add(`${m[1]}#${m[2]}`);
  }
  for (const m of text.matchAll(/(?:^|[\s,;(\[`])(nix|code|opencode|openchamber)#(\d+)/g)) {
    refs.add(`${SHORTHAND_REPOS.get(m[1])}#${m[2]}`);
  }
  for (const m of text.matchAll(/(?:^|[\s,;(\[`])#(\d+)/g)) {
    refs.add(`${ownRepo}#${m[1]}`);
  }
  return [...refs].filter((r) => isValidRef(r)).map((ref) => ({ ref, relation: "tracks" as const }));
}

function horizonOfHeading(heading: string): string | null {
  const first = heading.trim().split(/[\s—–:(-]/)[0];
  return HORIZON_BUCKETS.find((b) => b.toLowerCase() === first.toLowerCase()) ?? null;
}

/** Parse one roadmap markdown file into rows (ids assigned by the reader). */
export function parseRepoRoadmapFile(content: string, meta: RepoRoadmapFileMeta): ParsedRow[] {
  const lines = content.split(/\r?\n/);
  let program = meta.program ?? meta.path.split("/").pop()?.replace(/\.md$/i, "") ?? "";
  let section = "";
  let sectionHorizon: string | null = null;
  const rows: ParsedRow[] = [];
  const subjectCounts = new Map<string, number>();

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const h1 = /^#\s+Roadmap:\s*(.+?)\s*$/.exec(line);
    if (h1) {
      program = h1[1];
      continue;
    }
    const h2 = /^##\s+(.+?)\s*$/.exec(line);
    if (h2) {
      section = h2[1];
      sectionHorizon = horizonOfHeading(section);
      continue;
    }
    if (!line.trimStart().startsWith("|")) continue;
    const header = splitCells(line).map((c) => c.toLowerCase());
    const titleCol = header.indexOf("title");
    if (titleCol === -1) continue; // e.g. the nix "Areas" summary table
    const col = (name: string) => header.indexOf(name);
    const horizonCol = col("horizon");
    const statusCol = col("status");
    const intentCol = col("intent") !== -1 ? col("intent") : col("notes");
    const linksCol = col("links");
    // Consume the separator line plus every following table row.
    let j = i + 1;
    if (j < lines.length && /^\s*\|?[\s:|-]+\|?\s*$/.test(lines[j]) && lines[j].includes("-")) j++;
    for (; j < lines.length && lines[j].trimStart().startsWith("|"); j++) {
      const cells = splitCells(lines[j]);
      const title = cellText(cells[titleCol] ?? "");
      if (!title) continue;
      const horizon = (horizonCol !== -1 ? cellText(cells[horizonCol] ?? "") : "") || sectionHorizon || "Unscheduled";
      const anchor = section ? githubSlug(section) : "";
      const base = `repo-file:${meta.repo}/${meta.path}/${githubSlug(title)}`;
      const seen = (subjectCounts.get(base) ?? 0) + 1;
      subjectCounts.set(base, seen);
      const validation = validateRoadmapHorizon(horizon);
      rows.push({
        subject: seen > 1 ? `${base}-${seen}` : base,
        title,
        area: meta.areaFromSections ? section || program : program,
        status: statusCol !== -1 ? cellText(cells[statusCol] ?? "") : "",
        horizon,
        // "Ideas" is a real bucket in the repo files' own grammar
        // (it earns its own timeline lane); the Grist grammar does not
        // know it, so flagging it "invalid" would lie about the file.
        ...(!validation.valid && horizon !== "Ideas" ? { horizonError: validation.error } : {}),
        source: `repo file (${meta.repo}/${meta.path})`,
        notes: intentCol !== -1 ? cellText(cells[intentCol] ?? "") : "",
        links: roadmapRefsInText(linksCol !== -1 ? (cells[linksCol] ?? "") : lines[j], meta.repo),
        fileUrl: `https://github.com/${meta.repo}/blob/main/${meta.path}${anchor ? `#${anchor}` : ""}`,
      });
    }
    i = j - 1;
  }
  return rows;
}

const errCode = (err: unknown): string =>
  err instanceof Error ? ((err as NodeJS.ErrnoException).code ?? err.message) : String(err);

/** Read both repo roadmap sources from the aggregator host's checkouts.
 * Never throws: every failure lands in the section's state + reason. */
export async function readRepoRoadmaps(checkouts: {
  codeCheckout: string;
  nixCheckout: string;
}): Promise<RepoRoadmaps> {
  const rows: RoadmapRow[] = [];
  const sections: RepoRoadmapSection[] = [];
  let nextId = -1;
  const take = (parsed: ParsedRow[]) => {
    for (const row of parsed) rows.push({ ...row, id: nextId-- });
    return parsed.length;
  };

  // developing-today/code — planning/roadmaps/*.md, one program per file.
  {
    const source = "code planning/roadmaps";
    const dir = join(checkouts.codeCheckout, "planning", "roadmaps");
    let names: string[] | null = null;
    try {
      names = (await readdir(dir)).filter((n) => n.toLowerCase().endsWith(".md")).sort();
    } catch (err) {
      const code = errCode(err);
      sections.push({
        source,
        state: code === "ENOENT" ? "missing" : "unreadable",
        reason:
          code === "ENOENT"
            ? `planning/roadmaps not found under ${checkouts.codeCheckout} (PM_AGG_CODE_CHECKOUT)`
            : `planning/roadmaps under ${checkouts.codeCheckout} could not be listed: ${code}`,
        rowCount: 0,
      });
    }
    if (names) {
      let count = 0;
      const failures: string[] = [];
      for (const name of names) {
        try {
          const content = await readFile(join(dir, name), "utf8");
          count += take(
            parseRepoRoadmapFile(content, {
              repo: "developing-today/code",
              path: `planning/roadmaps/${name}`,
            }),
          );
        } catch (err) {
          failures.push(`${name}: ${errCode(err)}`);
        }
      }
      if (count === 0 && failures.length === names.length && names.length > 0) {
        sections.push({
          source,
          state: "unreadable",
          reason: `no roadmap file could be read (${failures.join("; ")})`,
          rowCount: 0,
        });
      } else if (count === 0) {
        sections.push({
          source,
          state: "parse-empty",
          reason: `${names.length} roadmap file(s) read under planning/roadmaps, but no Title-table rows parsed`,
          rowCount: 0,
        });
      } else {
        sections.push({
          source,
          state: "ok",
          reason: `${names.length} roadmap file(s), ${count} rows${failures.length ? `; unreadable: ${failures.join("; ")}` : ""}`,
          rowCount: count,
        });
      }
    }
  }

  // dezren39/darwin-nix — the single ROADMAP.md, areas per ## section.
  {
    const source = "nix ROADMAP.md";
    const file = join(checkouts.nixCheckout, "ROADMAP.md");
    try {
      const content = await readFile(file, "utf8");
      const count = take(
        parseRepoRoadmapFile(content, {
          repo: "dezren39/darwin-nix",
          path: "ROADMAP.md",
          program: "darwin-nix",
          areaFromSections: true,
        }),
      );
      sections.push(
        count === 0
          ? {
              source,
              state: "parse-empty",
              reason: `ROADMAP.md read under ${checkouts.nixCheckout}, but no Title-table rows parsed`,
              rowCount: 0,
            }
          : { source, state: "ok", reason: `ROADMAP.md, ${count} rows`, rowCount: count },
      );
    } catch (err) {
      const code = errCode(err);
      sections.push({
        source,
        state: code === "ENOENT" ? "missing" : "unreadable",
        reason:
          code === "ENOENT"
            ? `ROADMAP.md not found under ${checkouts.nixCheckout} (PM_AGG_NIX_CHECKOUT)`
            : `ROADMAP.md under ${checkouts.nixCheckout} could not be read: ${code}`,
        rowCount: 0,
      });
    }
  }

  return { rows, sections };
}

/** One-line section summary for the provenance footer: ok sections give
 * their row count; empty sections give their state and reason (#1500). */
export function describeRepoRoadmapSection(section: RepoRoadmapSection): string {
  return section.state === "ok"
    ? `${section.source}: ${section.rowCount} rows`
    : `${section.source}: ${section.state} — ${section.reason}`;
}

export interface RoadmapMerge {
  rows: RoadmapRow[];
  collapsedCount: number;
}

/** Merge repo-file rows with Grist rows: duplicates (same normalized
 * title) collapse in favour of the repo-file row, with the Grist
 * duplicate noted in that row's notes and its links unioned in. */
export function mergeRoadmapRows(gristRows: RoadmapRow[], repoRows: RoadmapRow[]): RoadmapMerge {
  const repoByTitle = new Map<string, RoadmapRow>();
  for (const row of repoRows) {
    const key = normalizeTitle(row.title);
    if (key && !repoByTitle.has(key)) repoByTitle.set(key, row);
  }
  const kept: RoadmapRow[] = [];
  let collapsedCount = 0;
  for (const grist of gristRows) {
    const twin = repoByTitle.get(normalizeTitle(grist.title));
    if (!twin) {
      kept.push(grist);
      continue;
    }
    collapsedCount++;
    const seenLinks = new Set(twin.links.map((l) => `${l.ref}|${l.relation}`));
    for (const link of grist.links) {
      if (!seenLinks.has(`${link.ref}|${link.relation}`)) twin.links.push(link);
    }
    twin.notes = [twin.notes, `Also tracked in the Grist roadmap (${grist.subject}); collapsed in favour of this repo-file row (#1738).`]
      .filter(Boolean)
      .join("\n");
  }
  return { rows: [...repoRows, ...kept], collapsedCount };
}
