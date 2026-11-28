// Project description + latest status, derived from the repo record
// (#1108 remainder): a project's /program/<id> page renders its
// description from its roadmap file (planning/roadmaps/<slug>.md in
// developing-today/code) and its latest status from the newest dated
// status record (doc/<UTC>_status_<slug>/ in the same repo), read from
// the aggregator host's code checkout at request time — instead of the
// hand-maintained copies the project registry used to carry (the
// lab-cluster intro paragraph and its pinned 2026-10-27 statusReport
// link were the seed case: both duplicated text that lives in the
// roadmap and the doc/ record, and both could drift silently).
//
// Honesty rules (the repo-roadmap / phase-mapping discipline):
//   - A snapshot exists only when the checkout could be read at all;
//     a source that cannot read it omits the snapshot and the page
//     falls back to the registry text, exactly like the other
//     optional Source snapshots.
//   - Inside a snapshot, each half is independently nullable: no
//     prose description in the roadmap, or no status record for the
//     slug, renders as an explicit absence — never as the stale
//     hand-maintained copy and never as invented text.
//   - "Newest" is the record's own UTC timestamp (the directory
//     name's prefix), never file mtime: the name is the record's
//     identity, and it sorts the same way in every checkout.
import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";

/** Where a project's derived doc lives: its roadmap file in a repo.
 * Derived from the project config's homeDoc by projectDocTarget in
 * src/project-config.ts — only planning/roadmaps/*.md home docs in
 * developing-today/code qualify (the checkout this aggregator reads). */
export interface ProjectDocTarget {
  repo: string;
  /** Repo-relative roadmap path ("planning/roadmaps/basement-cluster.md"). */
  roadmapPath: string;
  /** Roadmap basename without .md; also the status-record slug. */
  slug: string;
}

export interface ProjectLatestStatus {
  /** The record's UTC timestamp, from its directory name. */
  timestamp: string;
  /** Directory name under doc/ ("2026-10-27T03-14-08Z_status_basement-cluster"). */
  dir: string;
  /** Repo-relative path of the record directory. */
  path: string;
  /** Blob URL of the record's markdown file when it could be read,
   * else the tree URL of the record directory. */
  url: string;
  /** The record's own first heading, when its markdown could be read. */
  title: string | null;
}

export interface ProjectDocSnapshot {
  slug: string;
  roadmapPath: string;
  /** Blob URL of the roadmap file the description was derived from. */
  roadmapUrl: string;
  /** Plain-text description from the roadmap's prose, or null when
   * the file carries none before its first section heading. */
  description: string | null;
  /** Newest doc/<UTC>_status_<slug>/ record, or null when none exists. */
  latestStatus: ProjectLatestStatus | null;
  /** When this snapshot was read from the checkout (ISO-8601). */
  fetchedAt: string;
}

const blobUrl = (repo: string, path: string): string => `https://github.com/${repo}/blob/main/${path}`;
const treeUrl = (repo: string, path: string): string => `https://github.com/${repo}/tree/main/${path}`;

/** Plain-text form of a markdown fragment: links and images collapse
 * to their text, emphasis/code markers drop. Mirrors the repo-roadmap
 * source's cellText, extended for prose paragraphs. */
export function markdownPlainText(fragment: string): string {
  return fragment
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/[*_`]/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

/** The roadmap's description: the prose paragraphs between its H1 and
 * its first ## section, in order. Table rows, lists, quotes and
 * headings are not prose and never leak in. A trailing "Latest
 * status: …" pointer is metadata, not description — it is cut from
 * whichever paragraph carries it (the status half below derives the
 * same fact from doc/ itself). Returns null when no prose remains. */
export function roadmapDescription(content: string): string | null {
  const paragraphs: string[] = [];
  let current: string[] = [];
  let seenH1 = false;
  const flush = () => {
    if (!current.length) return;
    const raw = current.join(" ");
    current = [];
    const cut = raw.search(/\bLatest status:/i);
    const text = markdownPlainText(cut === -1 ? raw : raw.slice(0, cut));
    if (text) paragraphs.push(text);
  };
  for (const line of content.split(/\r?\n/)) {
    if (/^#\s+/.test(line)) {
      seenH1 = true;
      continue;
    }
    if (/^##\s+/.test(line)) break;
    if (!seenH1) continue;
    const trimmed = line.trim();
    if (!trimmed) {
      flush();
      continue;
    }
    if (trimmed.startsWith("|") || trimmed.startsWith(">") || /^[-*+]\s/.test(trimmed) || /^\d+\.\s/.test(trimmed)) {
      flush();
      continue;
    }
    current.push(trimmed);
  }
  flush();
  return paragraphs.length ? paragraphs.join("\n\n") : null;
}

const STATUS_DIR_RE = /^(\d{4})-(\d{2})-(\d{2})T(\d{2})-(\d{2})-(\d{2})Z_status_([a-z0-9][a-z0-9-]*)$/;

/** Parse a doc/ entry name as a status record: its UTC timestamp and
 * project slug, or null when the name is not a status record (doc/
 * carries designs, features and references under the same timestamp
 * prefix — only the _status_ infix marks a status record) or the
 * timestamp is not a real UTC instant. The record convention writes
 * the time dash-separated (03-14-08Z, filename-safe), which
 * Date.parse rejects outright — so the components are range-checked
 * here and the returned timestamp keeps the record's own spelling.
 * Every field is fixed-width, so valid timestamps order correctly
 * as plain strings. */
export function parseStatusRecordName(name: string): { timestamp: string; slug: string } | null {
  const m = STATUS_DIR_RE.exec(name);
  if (!m) return null;
  const [, year, month, day, hour, minute, second, slug] = m;
  const parts = [month, day, hour, minute, second].map(Number);
  if (parts[0] < 1 || parts[0] > 12 || parts[1] < 1 || parts[1] > 31 || parts[2] > 23 || parts[3] > 59 || parts[4] > 59)
    return null;
  // Round-trip through a colon-form ISO string to catch impossible
  // dates (e.g. Feb 31) that pass the per-field ranges.
  const iso = `${year}-${month}-${day}T${hour}:${minute}:${second}Z`;
  if (Number.isNaN(Date.parse(iso))) return null;
  return { timestamp: `${year}-${month}-${day}T${hour}-${minute}-${second}Z`, slug };
}

/** The newest status record for a slug among doc/ entry names, by the
 * records' own timestamps; null when the slug has none. */
export function selectLatestStatusRecord(names: string[], slug: string): { timestamp: string; dir: string } | null {
  let best: { timestamp: string; dir: string } | null = null;
  for (const name of names) {
    const parsed = parseStatusRecordName(name);
    if (!parsed || parsed.slug !== slug) continue;
    if (!best || parsed.timestamp > best.timestamp) {
      best = { timestamp: parsed.timestamp, dir: name };
    }
  }
  return best;
}

/** A status record's title: its first heading, else its first
 * non-empty line, as plain text; null for an empty record. */
export function statusRecordTitle(content: string): string | null {
  for (const line of content.split(/\r?\n/)) {
    const heading = /^#\s+(.+?)\s*$/.exec(line);
    if (heading) return markdownPlainText(heading[1]) || null;
    if (line.trim()) return markdownPlainText(line) || null;
  }
  return null;
}

/** Assemble a snapshot from already-read pieces (pure; the reader
 * below supplies them). statusContent is the newest record's markdown
 * when it could be read — its presence also decides blob vs tree URL. */
export function buildProjectDocSnapshot(
  target: ProjectDocTarget,
  roadmapContent: string | null,
  docNames: string[],
  statusContent: string | null,
  fetchedAt: string,
): ProjectDocSnapshot {
  const latest = selectLatestStatusRecord(docNames, target.slug);
  return {
    slug: target.slug,
    roadmapPath: target.roadmapPath,
    roadmapUrl: blobUrl(target.repo, target.roadmapPath),
    description: roadmapContent === null ? null : roadmapDescription(roadmapContent),
    latestStatus: latest
      ? {
          timestamp: latest.timestamp,
          dir: latest.dir,
          path: `doc/${latest.dir}`,
          url:
            statusContent === null
              ? treeUrl(target.repo, `doc/${latest.dir}`)
              : blobUrl(target.repo, `doc/${latest.dir}/${latest.dir}.md`),
          title: statusContent === null ? null : statusRecordTitle(statusContent),
        }
      : null,
    fetchedAt,
  };
}

/** Read a project's derived doc from the aggregator host's code
 * checkout. Never throws. Returns null only when the checkout could
 * not be read at all (neither the roadmap file nor the doc/ listing
 * resolved) — the page then falls back to the registry text instead
 * of claiming an absence the source never verified. A roadmap file
 * that is simply absent, with doc/ readable, is a real snapshot with
 * a null description: the absence is the verified fact. */
export async function loadProjectDoc(
  codeCheckout: string,
  target: ProjectDocTarget,
): Promise<ProjectDocSnapshot | null> {
  let roadmapContent: string | null = null;
  let roadmapFailed = false;
  try {
    roadmapContent = await readFile(join(codeCheckout, target.roadmapPath), "utf8");
  } catch {
    roadmapFailed = true;
  }
  let docNames: string[] = [];
  let docFailed = false;
  try {
    docNames = await readdir(join(codeCheckout, "doc"));
  } catch {
    docFailed = true;
  }
  if (roadmapFailed && docFailed) return null;
  const latest = docFailed ? null : selectLatestStatusRecord(docNames, target.slug);
  let statusContent: string | null = null;
  if (latest) {
    try {
      statusContent = await readFile(join(codeCheckout, "doc", latest.dir, `${latest.dir}.md`), "utf8");
    } catch {
      statusContent = null; // the directory is the record; its file may differ — tree URL fallback
    }
  }
  return buildProjectDocSnapshot(
    target,
    roadmapContent,
    docFailed ? [] : docNames,
    statusContent,
    new Date().toISOString(),
  );
}
