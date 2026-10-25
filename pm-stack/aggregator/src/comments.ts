// The dashboard comments store (#906 §2). Dashboard-native comments —
// NOT the projector's GitHub comment projection — written on the
// dashboard and keyed by stable object ref (see model.ts DashComment),
// so a thread survives feed republishes. Storage is one JSON file in
// the aggregator's data directory (PM_AGG_COMMENTS_FILE); writes are
// atomic (temp file + rename) and the in-memory copy is authoritative
// for the process.
//
// Trust model: the aggregator binds loopback + tailnet only (Drew,
// 2026-10-25 — no public route), so comment writes are unauthenticated
// like every other fleet-internal surface. Authorship is still never
// anonymous: every comment carries a validated author identity.
//
// Consumption contract (documented on #899):
// - kind "message" comments are the orchestrator's queue input: it
//   polls GET /api/comments.json?kind=message&state=new, includes them
//   in its run report as triaged input, and marks them via
//   POST /api/comments/triage { ids: [...] }.
// - kind "github" comments sit at githubState "pending" until a fleet
//   poster (the orchestrator, or any agent with gh) posts them with
//   `gh issue comment <n> --repo <repo>` / `gh pr comment`, appending
//   the attribution line "— posted by Drew via the PM dashboard", then
//   reports the result: POST /api/comments/github-result { id, url }
//   or { id, error }.
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import type { DashAuthor, DashComment, DashCommentKind, RefKey } from "./model.ts";
import { refUrl } from "./model.ts";

export interface CommentsFile {
  version: 1;
  comments: DashComment[];
}

const GH_REF = /^[\w.-]+\/[\w.-]+#\d+$/;

export function isObjectRef(s: unknown): s is string {
  return typeof s === "string" && s.length > 0 && s.length <= 300 && !/\s/.test(s);
}

export function validateAuthor(a: unknown): DashAuthor | null {
  if (typeof a !== "object" || a === null) return null;
  const { kind, id, label, sessionId } = a as Partial<DashAuthor>;
  if (kind !== "drew" && kind !== "muse" && kind !== "agent") return null;
  if (typeof id !== "string" || id === "" || typeof label !== "string" || label === "") return null;
  return {
    kind,
    id: id.slice(0, 120),
    label: label.slice(0, 120),
    ...(typeof sessionId === "string" && sessionId ? { sessionId: sessionId.slice(0, 120) } : {}),
  };
}

export interface NewComment {
  objectRef: string;
  kind: DashCommentKind;
  author: DashAuthor;
  body: string;
  githubRef?: RefKey;
}

/** Validate an untrusted POST body into a NewComment, or explain why not. */
export function validateNewComment(body: unknown): { ok: true; value: NewComment } | { ok: false; error: string } {
  if (typeof body !== "object" || body === null) return { ok: false, error: "body must be a JSON object" };
  const b = body as Record<string, unknown>;
  if (!isObjectRef(b.objectRef)) return { ok: false, error: "objectRef must be a non-empty ref string" };
  if (b.kind !== "message" && b.kind !== "note" && b.kind !== "github") {
    return { ok: false, error: "kind must be message | note | github" };
  }
  const author = validateAuthor(b.author);
  if (!author) return { ok: false, error: "author must be {kind: drew|muse|agent, id, label}" };
  if (typeof b.body !== "string" || b.body.trim() === "") return { ok: false, error: "body must be non-empty text" };
  if (b.kind === "github") {
    if (typeof b.githubRef !== "string" || !GH_REF.test(b.githubRef)) {
      return { ok: false, error: "kind github requires githubRef as owner/repo#N" };
    }
  }
  return {
    ok: true,
    value: {
      objectRef: b.objectRef,
      kind: b.kind,
      author,
      body: b.body.trim().slice(0, 20_000),
      ...(b.kind === "github" ? { githubRef: b.githubRef as RefKey } : {}),
    },
  };
}

let counter = 0;
export function newCommentId(now = new Date()): string {
  counter = (counter + 1) % 46656;
  const rand = Math.random().toString(36).slice(2, 7);
  return `dc-${now.getTime().toString(36)}-${counter.toString(36)}${rand}`;
}

export class CommentsStore {
  private comments: DashComment[] = [];
  private loaded = false;

  constructor(private readonly file: string) {}

  async load(): Promise<void> {
    if (this.loaded) return;
    this.loaded = true;
    try {
      const parsed: unknown = JSON.parse(await readFile(this.file, "utf8"));
      if (typeof parsed === "object" && parsed !== null && Array.isArray((parsed as CommentsFile).comments)) {
        this.comments = (parsed as CommentsFile).comments.filter(
          (c) => typeof c === "object" && c !== null && typeof (c as DashComment).id === "string",
        );
      }
    } catch {
      this.comments = []; // no store yet — the first write creates it
    }
  }

  private async persist(): Promise<void> {
    const data: CommentsFile = { version: 1, comments: this.comments };
    await mkdir(dirname(this.file), { recursive: true });
    const tmp = `${this.file}.tmp-${process.pid}`;
    await writeFile(tmp, JSON.stringify(data, null, 2));
    await rename(tmp, this.file);
  }

  async all(): Promise<DashComment[]> {
    await this.load();
    return [...this.comments].sort((a, b) => a.createdAt.localeCompare(b.createdAt));
  }

  async forObject(objectRef: string): Promise<DashComment[]> {
    return (await this.all()).filter((c) => c.objectRef === objectRef);
  }

  /** Counts per object ref, for 💬 badges across the views. */
  async counts(): Promise<Map<string, number>> {
    const out = new Map<string, number>();
    for (const c of await this.all()) out.set(c.objectRef, (out.get(c.objectRef) ?? 0) + 1);
    return out;
  }

  async add(input: NewComment): Promise<DashComment> {
    await this.load();
    const comment: DashComment = {
      id: newCommentId(),
      objectRef: input.objectRef,
      kind: input.kind,
      author: input.author,
      body: input.body,
      createdAt: new Date().toISOString(),
      ...(input.kind === "message" ? { triageState: "new" as const } : {}),
      ...(input.kind === "github"
        ? { githubRef: input.githubRef, githubState: "pending" as const }
        : {}),
    };
    this.comments.push(comment);
    await this.persist();
    return comment;
  }

  async markTriaged(ids: string[]): Promise<number> {
    await this.load();
    const wanted = new Set(ids);
    let n = 0;
    for (const c of this.comments) {
      if (wanted.has(c.id) && c.kind === "message" && c.triageState === "new") {
        c.triageState = "triaged";
        n++;
      }
    }
    if (n > 0) await this.persist();
    return n;
  }

  async markGithubResult(id: string, result: { url?: string; error?: string }): Promise<DashComment | null> {
    await this.load();
    const c = this.comments.find((x) => x.id === id);
    if (!c || c.kind !== "github") return null;
    if (result.url) {
      c.githubState = "posted";
      c.githubUrl = result.url;
    } else {
      c.githubState = "failed";
    }
    await this.persist();
    return c;
  }
}

/** The URL a posted GitHub comment lives at, for rendering. */
export function githubCommentUrl(c: DashComment): string | null {
  if (c.githubUrl) return c.githubUrl;
  if (c.githubRef) return refUrl(c.githubRef);
  return null;
}
