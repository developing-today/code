// Projector read path (split from live.ts for the production line limit,
// #12671). live.ts re-exports the public names, so importers are unchanged.

import { getJson } from "./http.ts";
import { type Comment, type GhItem, type Link, type RefKey, isValidRef, parseRef, refUrl } from "../model.ts";

// --- Projector (J's service, pm-stack/projector) -------------------------
// The wire schema is the projector's schema.sql: snake_case fields, `state`
// is only open|closed with `merged` as a separate 0/1 flag, and
// `labels`/`assignees` are JSON array *strings*. Map rows to the read model
// here; nothing else in the app sees the wire shape.

export interface ProjectorItemRow {
  ref: string;
  repo: string;
  number: number;
  kind: "issue" | "pr";
  url: string;
  title: string;
  state: "open" | "closed";
  state_reason?: string | null;
  draft?: number | boolean;
  merged?: number | boolean;
  author?: string | null;
  labels?: string | string[];
  assignees?: string | string[];
  created_at: string;
  updated_at: string;
  closed_at?: string | null;
  merged_at?: string | null;
}

export interface ProjectorCommentRow {
  id: string;
  ref: string;
  kind: Comment["kind"];
  author?: string | null;
  body?: string | null;
  created_at: string;
  url?: string;
  path?: string | null;
  line?: number | null;
  review_state?: string | null;
  commit_id?: string | null;
  deleted?: number | boolean;
}

export interface ProjectorLinkRow {
  subject: string;
  ref: string;
  relation: string;
  note?: string | null;
  created_by?: string | null;
  created_at?: string;
}

const asBool = (v: number | boolean | undefined): boolean => v === 1 || v === true;

export function parseProjectorLabels(labels: string | string[] | undefined): string[] {
  if (Array.isArray(labels)) return labels;
  if (typeof labels !== "string" || labels === "") return [];
  try {
    const parsed: unknown = JSON.parse(labels);
    return Array.isArray(parsed) ? parsed.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
}

export function mapProjectorItem(row: ProjectorItemRow): GhItem {
  return {
    ref: row.ref,
    kind: row.kind,
    url: row.url,
    title: row.title,
    state: asBool(row.merged) ? "merged" : row.state,
    draft: asBool(row.draft),
    labels: parseProjectorLabels(row.labels),
    author: row.author ?? "",
    updatedAt: row.updated_at,
  };
}

export function mapProjectorComment(row: ProjectorCommentRow): Comment {
  return {
    id: String(row.id),
    ref: row.ref,
    kind: row.kind,
    author: row.author ?? "",
    body: row.body ?? "",
    createdAt: row.created_at,
    // #6243 (ref contract rule 4): row.ref is stored projector data; the
    // stored url wins, and the refUrl fallback fires only for a ref the
    // shared rule accepts - an invalid stored ref yields no URL instead
    // of throwing out of the map (cf. the #6211 read gate for links).
    url: row.url ?? (isValidRef(row.ref) ? refUrl(row.ref, row.kind === "issue_comment" ? "issue" : "pr") : ""),
    path: row.path ?? undefined,
    line: row.line ?? undefined,
    reviewState: row.review_state ?? undefined,
    deleted: asBool(row.deleted),
  };
}

/** Union two link lists, deduped by ref+relation (first relation wins). */
export function mergeLinks(existing: Link[], extra: Link[]): Link[] {
  const seen = new Set(existing.map((l) => `${l.ref}|${l.relation}`));
  const out = [...existing];
  for (const l of extra) {
    const key = `${l.ref}|${l.relation}`;
    if (!seen.has(key)) {
      seen.add(key);
      out.push(l);
    }
  }
  return out;
}

const RELATIONS = new Set<string>(["tracks", "implements", "relates"]);
const asRelation = (v: string): Link["relation"] => (RELATIONS.has(v) ? (v as Link["relation"]) : "relates");

/**
 * Refs travel in the query string, and the projector (Bun.serve) rejects
 * request lines past ~16KB with HTTP 431 — at ~30 encoded chars per ref
 * that ceiling is only ~500 refs, which the board prime outgrew on
 * 2026-10-27: the prime's comments/items reads failed over to fixtures
 * while single-item pages (a handful of refs) stayed live. Chunk every
 * multi-ref read so each request stays far below the limit.
 */
const PROJECTOR_REFS_PER_REQUEST = 200;

function refChunks(refs: RefKey[]): RefKey[][] {
  const chunks: RefKey[][] = [];
  for (let i = 0; i < refs.length; i += PROJECTOR_REFS_PER_REQUEST) {
    chunks.push(refs.slice(i, i + PROJECTOR_REFS_PER_REQUEST));
  }
  return chunks;
}

export async function projectorItems(baseUrl: string, refs: RefKey[]): Promise<Map<RefKey, GhItem> | null> {
  if (refs.length === 0) return new Map();
  // Refs contain `#` — they must be percent-encoded or the `#` starts a
  // fragment and the projector never sees the number.
  const out = new Map<RefKey, GhItem>();
  for (const chunk of refChunks(refs)) {
    const r = await getJson(`${baseUrl}/items?refs=${encodeURIComponent(chunk.join(","))}`);
    if (!r.ok) return null;
    const rows = (r.json as { items?: ProjectorItemRow[] }).items ?? [];
    for (const row of rows) out.set(row.ref, mapProjectorItem(row));
  }
  return out;
}

export async function projectorComments(baseUrl: string, refs: RefKey[]): Promise<Comment[] | null> {
  if (refs.length === 0) return [];
  const out: Comment[] = [];
  for (const chunk of refChunks(refs)) {
    const r = await getJson(`${baseUrl}/comments?refs=${encodeURIComponent(chunk.join(","))}`);
    if (!r.ok) return null;
    out.push(
      ...((r.json as { comments?: ProjectorCommentRow[] }).comments ?? [])
        .map(mapProjectorComment)
        .filter((c) => !c.deleted),
    );
  }
  return out;
}

/** The latest non-deleted comment per ref, from a list in any order. */
export function latestPerRef(comments: Comment[]): Comment[] {
  const byRef = new Map<RefKey, Comment>();
  for (const c of comments) {
    if (c.deleted) continue;
    const cur = byRef.get(c.ref);
    if (!cur || cur.createdAt < c.createdAt) byRef.set(c.ref, c);
  }
  return [...byRef.values()];
}

/** TTL for the latest-comments cache below (#9781). */
export const LATEST_COMMENTS_TTL_MS = 60_000;

/**
 * Per-ref stale-while-revalidate cache for the briefs' latest comments
 * (#9781). projectorLatestComments fans out one projector read per ref
 * with an 8 s abort on every fetch, and the board/program/attention
 * handlers call it on EVERY request — so whenever the projector is
 * slow (measured 11.2 s for a /comments read while its 15-minute
 * /reconcile sweep occupies the process), each page assembly pays the
 * full ~8.2 s abort cliff, and /program's assembly crosses its 10 s
 * deadline into a 503. This cache gives the read the same contract the
 * page assemblies already have (#1179/#5082): fresh entries serve
 * directly; expired entries serve the stale comment immediately and
 * refresh single-flight in the background; only a never-seen ref pays
 * a projector read on the request path. A failed fetch never poisons
 * the cache: a failed refresh keeps serving the stale value, and a
 * failed cold read falls back (fixtures, in the live source) without
 * caching the failure, so the next request retries the projector.
 *
 * Truthfulness: a brief's "latest comment" can lag reality by at most
 * the TTL plus one refresh — the same 60 s bound the health probe
 * cache (#1147) already accepts. Only the latest comment per ref is
 * cached, never a comment list, so nothing else can be served stale.
 */
export class LatestCommentsCache {
  private entries = new Map<RefKey, { at: number; comment: Comment | null }>();
  private inFlight = new Set<RefKey>();

  constructor(
    private readonly fetchLatest: (refs: RefKey[]) => Promise<Comment[] | null>,
    private readonly fallback: (refs: RefKey[]) => Promise<Comment[]>,
    private readonly ttlMs: number = LATEST_COMMENTS_TTL_MS,
    private readonly now: () => number = Date.now,
  ) {}

  /** The latest comment for each requested ref that has one, in ref order. */
  async get(refs: RefKey[]): Promise<Comment[]> {
    const at = this.now();
    const byRef = new Map<RefKey, Comment>();
    const missing: RefKey[] = [];
    const stale: RefKey[] = [];
    for (const ref of refs) {
      const hit = this.entries.get(ref);
      if (hit && at - hit.at < this.ttlMs) {
        if (hit.comment) byRef.set(ref, hit.comment);
      } else if (hit) {
        if (hit.comment) byRef.set(ref, hit.comment);
        stale.push(ref);
      } else {
        missing.push(ref);
      }
    }
    if (stale.length) this.refresh(stale);
    if (missing.length) {
      const live = await this.fetchLatest(missing);
      if (live) {
        const fetched = new Map(live.map((c) => [c.ref, c]));
        for (const ref of missing) {
          const comment = fetched.get(ref) ?? null;
          this.store(ref, comment);
          if (comment) byRef.set(ref, comment);
        }
      } else {
        for (const c of await this.fallback(missing)) byRef.set(c.ref, c);
      }
    }
    return refs.flatMap((ref) => {
      const c = byRef.get(ref);
      return c ? [c] : [];
    });
  }

  /** Background re-read behind expired entries; never throws. */
  private refresh(refs: RefKey[]): void {
    const todo = refs.filter((ref) => !this.inFlight.has(ref));
    if (!todo.length) return;
    for (const ref of todo) this.inFlight.add(ref);
    this.fetchLatest(todo).then(
      (live) => {
        if (live) {
          const fetched = new Map(live.map((c) => [c.ref, c]));
          for (const ref of todo) this.store(ref, fetched.get(ref) ?? null);
        }
        // A failed refresh keeps the stale entries; the next expired
        // read retries.
        for (const ref of todo) this.inFlight.delete(ref);
      },
      () => {
        for (const ref of todo) this.inFlight.delete(ref);
      },
    );
  }

  private store(ref: RefKey, comment: Comment | null): void {
    // Soft cap: briefs only ever ask for pool/board/attention refs
    // (hundreds), so this only trims pathological growth.
    if (!this.entries.has(ref) && this.entries.size >= 4096) {
      const oldest = this.entries.keys().next().value;
      if (oldest !== undefined) this.entries.delete(oldest);
    }
    this.entries.set(ref, { at: this.now(), comment });
  }
}

/**
 * The latest comment for each ref, one projector read per ref (#1279).
 * The bulk /comments read applies its row limit across every requested
 * ref (projector default 500), so in a busy batch an item's latest
 * comment can be truncated away entirely; a per-ref limit=1 read cannot
 * be crowded out by other items' comments. Reads run with bounded
 * concurrency against the local projector; any failed read fails the
 * whole call (null), matching projectorComments' fail-soft contract.
 */
export async function projectorLatestComments(baseUrl: string, refs: RefKey[]): Promise<Comment[] | null> {
  if (refs.length === 0) return [];
  const out: Comment[] = [];
  const CONCURRENCY = 16;
  for (let i = 0; i < refs.length; i += CONCURRENCY) {
    const batch = refs.slice(i, i + CONCURRENCY);
    const results = await Promise.all(
      batch.map(async (ref): Promise<Comment[] | null> => {
        const r = await getJson(`${baseUrl}/comments?refs=${encodeURIComponent(ref)}&limit=1`);
        if (!r.ok) return null;
        return ((r.json as { comments?: ProjectorCommentRow[] }).comments ?? [])
          .map(mapProjectorComment)
          .filter((c) => !c.deleted);
      }),
    );
    for (const rows of results) {
      if (rows === null) return null;
      if (rows.length) out.push(latestPerRef(rows)[0]);
    }
  }
  return out;
}

/** Registry links for one subject (kaneo:/grist:/gh:), from GET /links.
 * A failed read returns null (fail-soft, like projectorItems and
 * projectorComments) so callers can tell "registry unreachable" apart
 * from "reachable but no links for this subject" (#8570: the board
 * provenance must say the links are unavailable, not claim them). */
export async function projectorLinks(baseUrl: string, subject: string): Promise<Link[] | null> {
  const r = await getJson(`${baseUrl}/links?subject=${encodeURIComponent(subject)}`);
  if (!r.ok) return null;
  const rows = (r.json as { links?: ProjectorLinkRow[] }).links ?? [];
  // Stored rows bypass the projector's POST /links gate (legacy/seeded
  // data), so the shared ref rule (#4952) is applied here at the read
  // boundary (#6211): a ref parseRef rejects - an unsafe-integer number,
  // or not owner/repo#N at all - must never reach a renderer, where
  // refUrl would throw on it and 500 the page. The drop is logged so a
  // bad registry row stays visible instead of silently vanishing.
  return rows.flatMap((row) => {
    if (!isValidRef(row.ref)) {
      console.error(
        `[registry] WARNING: dropping projector link with invalid ref ${JSON.stringify(row.ref)} for subject ${subject} (#6211)`,
      );
      return [];
    }
    return [{ ref: row.ref, relation: asRelation(row.relation) }];
  });
}
