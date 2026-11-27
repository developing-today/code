// Phase-1 new-issue intake for #1480: poll the four repos for issues
// created since a persisted watermark, so the hourly orchestration pass
// can assess, label, and comment on every new issue in the same pass.
//
// This module holds the pure logic (watermark guards, issue selection,
// item shaping). The CLI that talks to GitHub lives in
// scripts/gh-intake-poll.ts. Assessment itself stays agent-driven in the
// pass — this harness guarantees no new issue is silently skipped and the
// watermark advances only after a successful pass.

/** The four repos the intake pass polls (#1480). */
export const INTAKE_REPOS = [
  "developing-today/code",
  "developing-today/opencode",
  "developing-today/openchamber",
  "dezren39/nix",
] as const;

export type IntakeRepo = (typeof INTAKE_REPOS)[number];

/** Persisted intake state: a single ISO watermark. ISO-8601 UTC. */
export interface IntakeState {
  watermark: string;
}

/** A raw issue object as returned by `gh api repos/{repo}/issues`. */
export interface RawIssue {
  number?: unknown;
  title?: unknown;
  created_at?: unknown;
  html_url?: unknown;
  labels?: unknown;
  body?: unknown;
  pull_request?: unknown;
}

/** One new issue for the pass to assess. */
export interface IntakeItem {
  ref: string;
  repo: string;
  number: number;
  title: string;
  createdAt: string;
  url: string;
  labels: string[];
  bodyExcerpt: string;
}

export function isValidIsoTimestamp(ts: unknown): ts is string {
  if (typeof ts !== "string" || !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/.test(ts)) {
    return false;
  }
  const ms = Date.parse(ts);
  return Number.isFinite(ms);
}

/** Parse persisted state text. Unknown shape or bad watermark → error, never a silent epoch. */
export function loadState(text: string): IntakeState {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    throw new Error("intake state is not valid JSON");
  }
  const watermark = (parsed as { watermark?: unknown })?.watermark;
  if (!isValidIsoTimestamp(watermark)) {
    throw new Error("intake state has no valid ISO watermark");
  }
  return { watermark };
}

/** Serialize state. */
export function saveState(state: IntakeState): string {
  return JSON.stringify(state, null, 2) + "\n";
}

/**
 * Advance the watermark forward only (#1480: the watermark advances only
 * after a successful pass, and never backward — a backward move would
 * re-intake or, worse, a clobbered state would silently drop intake).
 */
export function advanceWatermark(current: IntakeState, to: string): IntakeState {
  if (!isValidIsoTimestamp(to)) throw new Error(`refusing to advance intake watermark to invalid timestamp: ${to}`);
  if (Date.parse(to) <= Date.parse(current.watermark)) {
    throw new Error(
      `refusing to move intake watermark backward or sideways (current ${current.watermark}, requested ${to})`,
    );
  }
  return { watermark: to };
}

/** True for real issues; the issues endpoint also returns PRs, which intake skips. */
export function isIntakeIssue(obj: RawIssue): boolean {
  if (obj == null || typeof obj !== "object") return false;
  if ((obj as { pull_request?: unknown }).pull_request != null) return false;
  return typeof obj.number === "number" && isValidIsoTimestamp(obj.created_at);
}

function labelNames(labels: unknown): string[] {
  if (!Array.isArray(labels)) return [];
  const out: string[] = [];
  for (const l of labels) {
    if (typeof l === "string") out.push(l);
    else if (l != null && typeof l === "object" && typeof (l as { name?: unknown }).name === "string") {
      out.push((l as { name: string }).name);
    }
  }
  return out;
}

/** Shape one raw issue into the item the pass assesses. */
export function shapeIntakeItem(repo: string, obj: RawIssue): IntakeItem {
  const number = obj.number as number;
  const createdAt = obj.created_at as string;
  const body = typeof obj.body === "string" ? obj.body : "";
  return {
    ref: `${repo}#${number}`,
    repo,
    number,
    title: typeof obj.title === "string" ? obj.title : "",
    createdAt,
    url: typeof obj.html_url === "string" ? obj.html_url : `https://github.com/${repo}/issues/${number}`,
    labels: labelNames(obj.labels),
    bodyExcerpt: body.slice(0, 500),
  };
}

/**
 * Select issues created strictly after the watermark, oldest first (the
 * pass assesses in creation order). Input order does not matter.
 */
export function selectNewIssues(repo: string, objs: RawIssue[], watermark: string): IntakeItem[] {
  const mark = Date.parse(watermark);
  const items: IntakeItem[] = [];
  for (const obj of objs) {
    if (!isIntakeIssue(obj)) continue;
    const created = Date.parse(obj.created_at as string);
    if (created > mark) items.push(shapeIntakeItem(repo, obj));
  }
  items.sort((a, b) => (a.createdAt < b.createdAt ? -1 : a.createdAt > b.createdAt ? 1 : a.number - b.number));
  return items;
}
