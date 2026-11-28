// Phase tracker F-item mapping (#9923, build-order step 3): the
// source of record that maps id-program phases to their F-items and
// each F-item to the GitHub issue (or, for the two items delivered
// without a dedicated issue, the PR) that tracks it. The table lives
// in planning/roadmaps/id.md under "## Phase tracker — F-item
// mapping" and is maintained as plan data; this module parses it and
// derives per-phase progress from live GhItem state.
//
// Coverage, researched and verified against GitHub on 2026-11-14:
// Phases 11 and 13 are the phases whose plans number F-items and
// whose items carry verifiable refs (Phase 11: plan headers for
// F11.1-F11.3, dedicated F-item issues for most of the rest, the
// 2026-10-20 phase-11 status record for F11.6 -> PR #247 and
// F11.14 -> PR #248, #227 for F11.12; Phase 13: issues #783-#790 in
// order). Phase 12's source remains the ordered step list in #1235
// (phase12.ts) — it is already uniform and machine-readable, so it
// is not duplicated here. Phases 0-10 have no F-item numbering in
// any plan (verified by the #9923 archaeology, 2026-11-14), so they
// have no rows: the panel states that gap instead of inventing one.
//
// Oracle (same as derivePhase12Progress): an item is done iff its
// ref is closed (issue) or merged/closed (PR) in LIVE GitHub state —
// completed work is recorded in GitHub, not in the table; the table
// only names the ref. A ref whose live kind differs from the table's
// (an issue number that resolves to a PR, or vice versa) counts as
// unresolved, never as done: a mismatched join must not fabricate
// progress.
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import type { GhItem, RefKey } from "./model.ts";

export const PHASE_MAPPING_REPO = "developing-today/code";
export const PHASE_MAPPING_PATH = "planning/roadmaps/id.md";

/** One row of the mapping table: an F-item and its tracking ref. */
export interface PhaseMappingEntry {
  phase: number;
  /** The F-item id as written in the plans ("F11.6"). */
  item: string;
  kind: "issue" | "pr";
  number: number;
  title: string;
}

export interface PhaseMappingSnapshot {
  entries: PhaseMappingEntry[];
  /** When this snapshot was read from the checkout (ISO-8601). */
  fetchedAt: string;
  /** Repo-relative path the entries were parsed from. */
  sourcePath: string;
}

/** Split a markdown table row into trimmed cells (mirrors the
 * repo-roadmap source's splitter: unescaped pipes, edge pipes dropped). */
function splitCells(line: string): string[] {
  const parts = line.split(/(?<!\\)\|/);
  if (parts.length && parts[0].trim() === "") parts.shift();
  if (parts.length && parts[parts.length - 1].trim() === "") parts.pop();
  return parts.map((c) => c.replace(/\\\|/g, "|").trim());
}

function cellText(cell: string): string {
  return cell
    .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/[*_`]/g, "")
    .trim();
}

/** Parse the F-item mapping table out of planning/roadmaps/id.md.
 * Only the section whose heading names the F-item mapping is read,
 * and only rows shaped like one (numeric phase, F<phase>.<n> item,
 * "#<n>" or "PR #<n>" ref) become entries — the file's other roadmap
 * tables are not mapping data and must never leak in. */
export function parsePhaseMapping(content: string): PhaseMappingEntry[] {
  const entries: PhaseMappingEntry[] = [];
  let inSection = false;
  for (const line of content.split(/\r?\n/)) {
    const heading = /^##\s+(.*)$/.exec(line);
    if (heading) {
      inSection = /f-item mapping/i.test(heading[1]);
      continue;
    }
    if (!inSection || !line.trim().startsWith("|")) continue;
    const cells = splitCells(line).map(cellText);
    if (cells.length < 3) continue;
    const phase = Number(cells[0]);
    if (!Number.isInteger(phase) || phase < 0) continue;
    if (!/^F\d+\.\d+$/.test(cells[1])) continue;
    const ref = /^(PR\s+)?#(\d+)$/.exec(cells[2]);
    if (!ref) continue;
    entries.push({
      phase,
      item: cells[1],
      kind: ref[1] ? "pr" : "issue",
      number: Number(ref[2]),
      title: cells[3] ?? "",
    });
  }
  return entries;
}

/** Read and parse the mapping from the aggregator host's code
 * checkout. Returns null when the file cannot be read or parses to
 * no entries — the panel withholds mapped progress rather than
 * rendering an empty or invented one. */
export async function loadPhaseMapping(codeCheckout: string): Promise<PhaseMappingSnapshot | null> {
  try {
    const content = await readFile(join(codeCheckout, PHASE_MAPPING_PATH), "utf8");
    const entries = parsePhaseMapping(content);
    if (!entries.length) return null;
    return { entries, fetchedAt: new Date().toISOString(), sourcePath: PHASE_MAPPING_PATH };
  } catch {
    return null;
  }
}

/** Per-phase F-item progress counted from the mapping. */
export interface MappedPhaseProgress {
  /** Mapped F-items for the phase. */
  total: number;
  /** Items whose ref is closed/merged in live GitHub state. */
  done: number;
  /** Items whose ref is open in live GitHub state. */
  open: number;
  /** Items the current items source could not resolve (or resolved
   * to the other kind — see the header oracle). */
  unresolved: number;
  /** done / total as a whole percent; null when total is 0. */
  percent: number | null;
}

/** Derive per-phase progress by joining the mapping entries to live
 * GhItem state. Phases with no entries are absent from the result —
 * absence is the honest signal (no source), not a zero. */
export function deriveMappedProgress(
  entries: readonly PhaseMappingEntry[],
  items: ReadonlyMap<RefKey, GhItem>,
): Map<number, MappedPhaseProgress> {
  const byPhase = new Map<number, MappedPhaseProgress>();
  for (const entry of entries) {
    const acc =
      byPhase.get(entry.phase) ?? ({ total: 0, done: 0, open: 0, unresolved: 0, percent: null } as MappedPhaseProgress);
    acc.total++;
    const item = items.get(`${PHASE_MAPPING_REPO}#${entry.number}` as RefKey);
    if (!item || item.kind !== entry.kind) acc.unresolved++;
    else if (item.state === "open") acc.open++;
    else acc.done++; // "closed" or "merged"
    byPhase.set(entry.phase, acc);
  }
  for (const acc of byPhase.values()) {
    acc.percent = acc.total ? Math.round((acc.done / acc.total) * 100) : null;
  }
  return byPhase;
}
