// The fallback-claim protocol, hosted (#12589): when the queue
// backend is down, an issue's GitHub comments carry the claim —
// marker comments folded in order into a claim state, exactly as
// the native CLI does it (pkgs/fleet/crates/fleet-queue/src/
// fallback.rs, ported). The hosted door performs the protocol as a
// service over its GitHub comment surface; the marker format is
// unchanged, so a claim posted by the native CLI and one posted
// here read identically to every other worker.
//
// Exit-code mapping (the native CLI's contract): success → the
// verb's JSON at HTTP 200; a state refusal (exit 5) → HTTP 409 with
// the same JSON; a usage refusal (exit 2) → HTTP 400; a GitHub
// transport failure (exit 3) → HTTP 502.

import { iso, type JsonObject } from "./queue-validate.ts";
import type { GitHubCommentSurface } from "./github.ts";

export const FALLBACK_DEFAULT_LEASE_MIN = 30;

export interface Marker {
  kind: "claim" | "heartbeat" | "release";
  worker: string;
  outcome: string | null;
  ts: number;
}

const MARKER_PREFIX = "<!-- fleet-claim v1 ";
const MARKER_SUFFIX = " -->";

export function validWorkerId(w: string): boolean {
  return (
    w !== "" &&
    w.length <= 64 &&
    [...w].every((c) => /[A-Za-z0-9._-]/.test(c))
  );
}

function daysInMonth(year: number, month: number): number {
  const leap = (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0;
  return [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][month - 1] ?? 0;
}

function daysFromCivil(y: number, m: number, d: number): number {
  const yAdj = m <= 2 ? y - 1 : y;
  const era = Math.floor((yAdj >= 0 ? yAdj : yAdj - 399) / 400);
  const yoe = yAdj - era * 400;
  const mp = m > 2 ? m - 3 : m + 9;
  const doy = Math.floor((153 * mp + 2) / 5) + d - 1;
  const doe = yoe * 365 + Math.floor(yoe / 4) - Math.floor(yoe / 100) + doy;
  return era * 146097 + doe - 719468;
}

/** The fallback marker timestamp parser (fallback.rs parse_ts):
 * RFC 3339 with a 'T' or ' ' separator, Z or numeric offset,
 * fractional seconds dropped, seconds optional. Field widths are
 * not enforced; the calendar is validated for real. */
export function parseMarkerTs(input: string): number | null {
  const s = input.trim();
  const sepIdx = (() => {
    const t = s.indexOf("T");
    if (t >= 0) return t;
    const t2 = s.indexOf("t");
    if (t2 >= 0) return t2;
    return s.indexOf(" ");
  })();
  if (sepIdx < 0) return null;
  const date = s.slice(0, sepIdx);
  let rest = s.slice(sepIdx + 1);
  const dp = date.split("-");
  if (dp.length !== 3) return null;
  const num = (v: string): number | null =>
    v !== "" && /^[0-9]+$/.test(v) ? Number(v) : null;
  const year = num(dp[0]);
  const month = num(dp[1]);
  const day = num(dp[2]);
  if (year === null || month === null || day === null) return null;
  if (month < 1 || month > 12 || day < 1 || day > daysInMonth(year, month)) return null;
  let offsetS = 0;
  if (rest.endsWith("Z") || rest.endsWith("z")) {
    rest = rest.slice(0, -1);
  } else {
    const idx = Math.max(rest.lastIndexOf("+"), rest.lastIndexOf("-"));
    if (idx > 0) {
      const off = rest.slice(idx + 1);
      const sign = rest[idx] === "+" ? 1 : -1;
      const op = off.split(":");
      if (op.length > 2) return null;
      const oh = num(op[0]);
      const om = op.length > 1 ? num(op[1]) : 0;
      if (oh === null || om === null || oh > 23 || om > 59) return null;
      offsetS = sign * (oh * 3600 + om * 60);
      rest = rest.slice(0, idx);
    } else if (idx === 0) {
      return null;
    }
  }
  const dot = rest.indexOf(".");
  if (dot >= 0) rest = rest.slice(0, dot);
  const tp = rest.split(":");
  if (tp.length < 2 || tp.length > 3) return null;
  const hh = num(tp[0]);
  const mm = num(tp[1]);
  const ss = tp.length > 2 ? num(tp[2]) : 0;
  if (hh === null || mm === null || ss === null) return null;
  if (hh >= 24 || mm >= 60 || ss > 60) return null;
  return daysFromCivil(year, month, day) * 86400 + hh * 3600 + mm * 60 + ss - offsetS;
}

/** The first well-formed claim marker in a comment body, if any. */
export function parseMarker(body: string): Marker | null {
  for (const rawLine of body.split("\n")) {
    const line = rawLine.trim();
    if (!line.startsWith(MARKER_PREFIX) || !line.endsWith(MARKER_SUFFIX)) continue;
    const inner = line.slice(MARKER_PREFIX.length, line.length - MARKER_SUFFIX.length);
    const fields = new Map<string, string>();
    let malformed = false;
    for (const part of inner.split(/\s+/).filter((p) => p !== "")) {
      const eq = part.indexOf("=");
      if (eq < 0) {
        malformed = true;
        break;
      }
      fields.set(part.slice(0, eq), part.slice(eq + 1));
    }
    if (malformed) continue;
    for (const key of fields.keys()) {
      if (!["kind", "worker", "outcome", "ts"].includes(key)) {
        malformed = true;
        break;
      }
    }
    if (malformed) continue;
    const kind = fields.get("kind");
    if (kind !== "claim" && kind !== "heartbeat" && kind !== "release") continue;
    const worker = fields.get("worker");
    if (worker === undefined || !validWorkerId(worker)) continue;
    const tsRaw = fields.get("ts");
    if (tsRaw === undefined) continue;
    const ts = parseMarkerTs(tsRaw);
    if (ts === null) continue;
    const outcome = fields.get("outcome") ?? null;
    if (kind === "release") {
      if (outcome !== "done" && outcome !== "free") continue;
    } else if (outcome !== null) {
      continue;
    }
    return { kind, worker, outcome, ts };
  }
  return null;
}

export type ClaimState =
  | { state: "free" }
  | { state: "released"; worker: string; outcome: string; ts: number }
  | { state: "live" | "stale"; worker: string; lastActivity: number; ageSeconds: number }
  | { state: "contested"; holder: string; challenger: string; ts: number };

export function claimStateClaimable(s: ClaimState): boolean {
  return s.state === "free" || s.state === "released" || s.state === "stale";
}

export function claimStateHolder(s: ClaimState): string | null {
  if (s.state === "live" || s.state === "stale") return s.worker;
  if (s.state === "contested") return s.holder;
  return null;
}

export function claimStateJson(s: ClaimState): JsonObject {
  switch (s.state) {
    case "free":
      return { state: "free", claimable: true };
    case "released":
      return {
        state: "released",
        claimable: true,
        worker: s.worker,
        outcome: s.outcome,
        released_at: iso(s.ts),
      };
    case "live":
    case "stale":
      return {
        state: s.state,
        claimable: claimStateClaimable(s),
        worker: s.worker,
        last_activity: iso(s.lastActivity),
        age_seconds: s.ageSeconds,
      };
    case "contested":
      return {
        state: "contested",
        claimable: false,
        worker: s.holder,
        challenger: s.challenger,
        contested_at: iso(s.ts),
      };
  }
}

/** Fold marker events (in comment order) into the claim state at
 * `now` — fallback.rs fold_claim, transcribed. */
export function foldClaim(events: Marker[], now: number, leaseSeconds: number): ClaimState {
  let holder: { worker: string; lastActivity: number } | null = null;
  let released: { worker: string; outcome: string; ts: number } | null = null;
  let challengers: { worker: string; ts: number }[] = [];
  for (const ev of events) {
    if (ev.kind === "claim") {
      released = null;
      if (holder === null) {
        holder = { worker: ev.worker, lastActivity: ev.ts };
      } else if (holder.worker === ev.worker) {
        holder = { worker: ev.worker, lastActivity: ev.ts };
      } else if (ev.ts - holder.lastActivity > leaseSeconds) {
        holder = { worker: ev.worker, lastActivity: ev.ts };
        challengers = [];
      } else {
        const entry = challengers.find((c) => c.worker === ev.worker);
        if (entry) entry.ts = ev.ts;
        else challengers.push({ worker: ev.worker, ts: ev.ts });
      }
    } else if (ev.kind === "heartbeat") {
      if (holder !== null && holder.worker === ev.worker) {
        holder = { worker: ev.worker, lastActivity: ev.ts };
      }
    } else {
      // release
      if (holder !== null && holder.worker === ev.worker) {
        holder = null;
        challengers = [];
        released = { worker: ev.worker, outcome: ev.outcome ?? "done", ts: ev.ts };
      }
      challengers = challengers.filter((c) => c.worker !== ev.worker);
    }
  }
  if (holder !== null) {
    const contest = challengers.find((c) => now - c.ts <= leaseSeconds);
    if (contest) {
      return { state: "contested", holder: holder.worker, challenger: contest.worker, ts: contest.ts };
    }
    const age = now - holder.lastActivity;
    return age <= leaseSeconds
      ? { state: "live", worker: holder.worker, lastActivity: holder.lastActivity, ageSeconds: age }
      : { state: "stale", worker: holder.worker, lastActivity: holder.lastActivity, ageSeconds: age };
  }
  if (released !== null) {
    return { state: "released", worker: released.worker, outcome: released.outcome, ts: released.ts };
  }
  return { state: "free" };
}

// ---------------------------------------------------------------------------
// The fallback verbs over the comment surface
// ---------------------------------------------------------------------------

export interface FallbackResult {
  status: number;
  body: JsonObject;
}

export interface FallbackCtx {
  surface: GitHubCommentSurface;
  nowEpoch: () => number;
}

function leaseSecondsOf(req: JsonObject): number {
  const min = req["lease_min"];
  const n = typeof min === "number" && Number.isInteger(min) && min >= 1 ? min : FALLBACK_DEFAULT_LEASE_MIN;
  return n * 60;
}

function repoOf(req: JsonObject): string | null {
  const repo = req["repo"];
  if (typeof repo !== "string") return null;
  const parts = repo.split("/");
  if (parts.length !== 2 || parts.some((p) => p === "")) return null;
  return repo;
}

function numberOf(req: JsonObject): number | null {
  const n = req["number"];
  return typeof n === "number" && Number.isInteger(n) && n > 0 ? n : null;
}

function workerOf(req: JsonObject): string | null {
  const w = req["worker"];
  return typeof w === "string" && w !== "" ? w : null;
}

function usageError(message: string): FallbackResult {
  return { status: 400, body: { error: "fallback_usage", message } };
}

function transportError(e: unknown): FallbackResult {
  return {
    status: 502,
    body: {
      error: "github_transport_failed",
      message: e instanceof Error ? e.message : String(e),
    },
  };
}

async function readMarkers(
  ctx: FallbackCtx,
  repo: string,
  number: number,
): Promise<Marker[]> {
  const comments = await ctx.surface.listIssueComments(repo, number);
  const markers: Marker[] = [];
  for (const c of comments) {
    const m = parseMarker(c.body);
    if (m) markers.push(m);
  }
  return markers;
}

function markerLine(kind: string, worker: string, ts: number, outcome?: string): string {
  const outcomePart = outcome ? ` outcome=${outcome}` : "";
  return `<!-- fleet-claim v1 kind=${kind} worker=${worker}${outcomePart} ts=${iso(ts)} -->`;
}

const CLAIMED_LABELS_ADD = ["queue/claimed", "work/assigned"];
const QUEUED_LABELS_REMOVE = ["queue/queued"];

export async function fallbackStatus(
  ctx: FallbackCtx,
  req: JsonObject,
): Promise<FallbackResult> {
  const repo = repoOf(req);
  const number = numberOf(req);
  if (repo === null || number === null) {
    return usageError("fallback status: repo (owner/name) and number (positive integer) are required");
  }
  const lease = leaseSecondsOf(req);
  let markers: Marker[];
  try {
    markers = await readMarkers(ctx, repo, number);
  } catch (e) {
    return transportError(e);
  }
  const state = foldClaim(markers, ctx.nowEpoch(), lease);
  return {
    status: 200,
    body: {
      mode: "fallback",
      repo,
      number,
      url: `https://github.com/${repo}/issues/${number}`,
      lease_seconds: lease,
      claim: claimStateJson(state),
    },
  };
}

export async function fallbackClaim(
  ctx: FallbackCtx,
  req: JsonObject,
): Promise<FallbackResult> {
  const repo = repoOf(req);
  const number = numberOf(req);
  if (repo === null || number === null) {
    return usageError("fallback claim: repo (owner/name) and number (positive integer) are required");
  }
  const worker = workerOf(req);
  if (worker === null) {
    return usageError(
      "fallback claim: worker is required: the claim marker carries your worker id, and two workers sharing an id would share one lease",
    );
  }
  if (!validWorkerId(worker)) {
    return usageError(
      `worker id ${JSON.stringify(worker)} is not a valid marker token (letters, digits, '.', '_', '-', max 64)`,
    );
  }
  const lease = leaseSecondsOf(req);
  const now = ctx.nowEpoch();
  let markers: Marker[];
  try {
    markers = await readMarkers(ctx, repo, number);
  } catch (e) {
    return transportError(e);
  }
  const before = foldClaim(markers, now, lease);
  if (before.state === "live" && before.worker !== worker) {
    return { status: 409, body: { claimed: false, reason: "live_claim" } };
  }
  if (before.state === "contested") {
    return { status: 409, body: { claimed: false, reason: "contested" } };
  }
  const superseded = before.state === "stale" ? before.worker : null;
  const note = typeof req["note"] === "string" && req["note"] !== "" ? req["note"] : null;
  const body =
    `${markerLine("claim", worker, now)}\n\nPicked up by \`${worker}\` — working this now. ` +
    `(Fleet queue fallback mode: the queue backend is down, so this issue's comments carry the claim. ` +
    `The claim stays live while \`${worker}\` posts progress markers; after ${lease / 60} minutes of silence any other worker may pick this up.)` +
    (note ? `\n\n${note}` : "");
  let posted: { url: string };
  try {
    posted = await ctx.surface.postIssueComment(repo, number, body);
  } catch (e) {
    return transportError(e);
  }
  let after: ClaimState;
  try {
    after = foldClaim(await readMarkers(ctx, repo, number), ctx.nowEpoch(), lease);
  } catch (e) {
    return transportError(e);
  }
  if (claimStateHolder(after) !== worker) {
    return {
      status: 409,
      body: { claimed: false, reason: "race_lost", repo, number, worker, comment_url: posted.url },
    };
  }
  const warnings: string[] = [];
  try {
    await ctx.surface.editIssueLabels(repo, number, CLAIMED_LABELS_ADD, QUEUED_LABELS_REMOVE);
  } catch (e) {
    warnings.push(`labels: ${e instanceof Error ? e.message : String(e)}`);
  }
  return {
    status: 200,
    body: {
      claimed: true,
      repo,
      number,
      url: `https://github.com/${repo}/issues/${number}`,
      worker,
      comment_url: posted.url,
      superseded,
      contested_by: after.state === "contested" ? after.challenger : null,
      lease_seconds: lease,
      warnings,
    },
  };
}

export async function fallbackHeartbeat(
  ctx: FallbackCtx,
  req: JsonObject,
): Promise<FallbackResult> {
  const repo = repoOf(req);
  const number = numberOf(req);
  if (repo === null || number === null) {
    return usageError("fallback heartbeat: repo (owner/name) and number (positive integer) are required");
  }
  const worker = workerOf(req);
  if (worker === null) {
    return usageError("fallback heartbeat: worker is required");
  }
  if (!validWorkerId(worker)) {
    return usageError(
      `worker id ${JSON.stringify(worker)} is not a valid marker token (letters, digits, '.', '_', '-', max 64)`,
    );
  }
  const lease = leaseSecondsOf(req);
  const now = ctx.nowEpoch();
  let markers: Marker[];
  try {
    markers = await readMarkers(ctx, repo, number);
  } catch (e) {
    return transportError(e);
  }
  const state = foldClaim(markers, now, lease);
  const holder = claimStateHolder(state);
  if (holder === null) {
    return { status: 409, body: { heartbeat: false, reason: "no_claim" } };
  }
  if (holder !== worker) {
    return { status: 409, body: { heartbeat: false, reason: "held_by_other", holder } };
  }
  const note = typeof req["note"] === "string" && req["note"] !== "" ? req["note"] : "still working";
  const body = `${markerLine("heartbeat", worker, now)}\n\nProgress from \`${worker}\`: ${note}`;
  let posted: { url: string };
  try {
    posted = await ctx.surface.postIssueComment(repo, number, body);
  } catch (e) {
    return transportError(e);
  }
  return {
    status: 200,
    body: { heartbeat: true, repo, number, worker, comment_url: posted.url, lease_seconds: lease },
  };
}

export async function fallbackRelease(
  ctx: FallbackCtx,
  req: JsonObject,
): Promise<FallbackResult> {
  const repo = repoOf(req);
  const number = numberOf(req);
  if (repo === null || number === null) {
    return usageError("fallback release: repo (owner/name) and number (positive integer) are required");
  }
  const worker = workerOf(req);
  if (worker === null) {
    return usageError("fallback release: worker is required");
  }
  if (!validWorkerId(worker)) {
    return usageError(
      `worker id ${JSON.stringify(worker)} is not a valid marker token (letters, digits, '.', '_', '-', max 64)`,
    );
  }
  const outcome = typeof req["outcome"] === "string" ? req["outcome"] : "done";
  if (outcome !== "done" && outcome !== "free") {
    return usageError(`fallback release: outcome must be done|free, got ${JSON.stringify(outcome)}`);
  }
  const summary = typeof req["summary"] === "string" ? req["summary"] : "";
  if (summary.trim() === "") {
    return usageError(
      "fallback release: --summary is required (what landed, with PR links for outcome=done)",
    );
  }
  const lease = leaseSecondsOf(req);
  const now = ctx.nowEpoch();
  let markers: Marker[];
  try {
    markers = await readMarkers(ctx, repo, number);
  } catch (e) {
    return transportError(e);
  }
  const state = foldClaim(markers, now, lease);
  const holder = claimStateHolder(state);
  if (holder === null) {
    return { status: 409, body: { released: false, reason: "no_claim" } };
  }
  if (holder !== worker) {
    return { status: 409, body: { released: false, reason: "held_by_other", holder } };
  }
  const body = `${markerLine("release", worker, now, outcome)}\n\nReleased by \`${worker}\` (outcome: ${outcome}). ${summary}`;
  let posted: { url: string };
  try {
    posted = await ctx.surface.postIssueComment(repo, number, body);
  } catch (e) {
    return transportError(e);
  }
  const warnings: string[] = [];
  try {
    if (outcome === "free") {
      await ctx.surface.editIssueLabels(repo, number, ["queue/queued"], CLAIMED_LABELS_ADD);
    } else {
      await ctx.surface.editIssueLabels(repo, number, [], CLAIMED_LABELS_ADD);
    }
  } catch (e) {
    warnings.push(`labels: ${e instanceof Error ? e.message : String(e)}`);
  }
  return {
    status: 200,
    body: {
      released: true,
      repo,
      number,
      worker,
      outcome,
      comment_url: posted.url,
      warnings,
    },
  };
}
