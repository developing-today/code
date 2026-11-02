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
  private loadPromise: Promise<void> | null = null;
  private writeChain: Promise<void> = Promise.resolve();
  private writeSeq = 0;

  constructor(private readonly file: string) {}

  // The load is memoized as a single promise (#1509): the old
  // loaded-flag version let concurrent first callers proceed before
  // the read finished, and the read's completion then reset the
  // in-memory array, wiping records those callers had just pushed.
  async load(): Promise<void> {
    if (!this.loadPromise) this.loadPromise = this.doLoad();
    return this.loadPromise;
  }

  private async doLoad(): Promise<void> {
    // #1452: a missing file is the only silent empty start ("no store
    // yet"). An unparseable or wrongly-shaped file is NOT an empty
    // store: it is the append-only audit history, so quarantine the
    // original bytes to <file>.corrupt-<timestamp>, start empty, and
    // log loudly — the next persist then writes a fresh valid file and
    // never touches the quarantined copy.
    let raw: string;
    try {
      raw = await readFile(this.file, "utf8");
    } catch (err) {
      if ((err as { code?: unknown })?.code === "ENOENT") {
        this.comments = []; // no store yet — the first write creates it
        return;
      }
      // #2980: a non-ENOENT read failure (EACCES, a transient I/O error,
      // a directory in place of the file, ...) is NOT "no store yet":
      // the on-disk audit history may be intact but unread. Handle it
      // like the corrupt case below — quarantine the original bytes
      // before starting empty, and log loudly — so the next persist
      // cannot silently overwrite history the read never saw.
      this.comments = [];
      const quarantined = `${this.file}.corrupt-${new Date().toISOString().replace(/[:.]/g, "-")}`;
      try {
        await rename(this.file, quarantined);
        console.error(
          `[comments] WARNING: comments store ${this.file} could not be read (${err instanceof Error ? err.message : String(err)}); ` +
            `quarantined the original bytes to ${quarantined} and started empty. ` +
            `The quarantined file holds the prior audit history — do not delete it.`,
        );
      } catch (renameErr) {
        console.error(
          `[comments] WARNING: comments store ${this.file} could not be read (${err instanceof Error ? err.message : String(err)}) ` +
            `and could not be quarantined (${renameErr instanceof Error ? renameErr.message : String(renameErr)}); starting empty — ` +
            `the next persist will overwrite it, so copy it aside now to preserve the audit history.`,
        );
      }
      return;
    }
    try {
      const parsed: unknown = JSON.parse(raw);
      if (typeof parsed !== "object" || parsed === null || !Array.isArray((parsed as CommentsFile).comments)) {
        throw new Error("store file does not have the expected { comments: [...] } shape");
      }
      this.comments = (parsed as CommentsFile).comments.filter(
        (c) => typeof c === "object" && c !== null && typeof (c as DashComment).id === "string",
      );
    } catch (err) {
      this.comments = [];
      const quarantined = `${this.file}.corrupt-${new Date().toISOString().replace(/[:.]/g, "-")}`;
      try {
        await rename(this.file, quarantined);
        console.error(
          `[comments] WARNING: comments store ${this.file} is unparseable (${err instanceof Error ? err.message : String(err)}); ` +
            `quarantined the original bytes to ${quarantined} and started empty. ` +
            `The quarantined file holds the prior audit history — do not delete it.`,
        );
      } catch (renameErr) {
        console.error(
          `[comments] WARNING: comments store ${this.file} is unparseable (${err instanceof Error ? err.message : String(err)}) ` +
            `and could not be quarantined (${renameErr instanceof Error ? renameErr.message : String(renameErr)}); starting empty — ` +
            `the next persist will overwrite it, so copy it aside now to preserve the audit history.`,
        );
      }
    }
  }

  // Persists are serialized through a per-store promise chain and each
  // write lands on a unique temp path (#1509): concurrent adds used to
  // share one pid-only temp path, so the first rename won and the rest
  // threw ENOENT while their records stayed in memory - fulfilled calls
  // diverged from what was on disk. The snapshot is taken when the
  // queued write actually runs, so the last write always carries the
  // full current state; a failed write still rejects its own caller but
  // does not poison the chain for later writes.
  private persist(): Promise<void> {
    const run = this.writeChain.then(async () => {
      const data: CommentsFile = { version: 1, comments: this.comments };
      await mkdir(dirname(this.file), { recursive: true });
      const tmp = `${this.file}.tmp-${process.pid}-${++this.writeSeq}`;
      await writeFile(tmp, JSON.stringify(data, null, 2));
      await rename(tmp, this.file);
    });
    this.writeChain = run.catch(() => {});
    return run;
  }

  async all(): Promise<DashComment[]> {
    await this.load();
    return [...this.comments].sort((a, b) => a.createdAt.localeCompare(b.createdAt));
  }

  async forObject(objectRef: string): Promise<DashComment[]> {
    return (await this.all()).filter((c) => c.objectRef === objectRef);
  }

  /** Comments grouped by object ref, oldest first — the per-card
   *  comment boxes (#1446) render from this. */
  async byObject(): Promise<Map<string, DashComment[]>> {
    const out = new Map<string, DashComment[]>();
    for (const c of await this.all()) out.set(c.objectRef, [...(out.get(c.objectRef) ?? []), c]);
    return out;
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
      ...(input.kind === "github" ? { githubRef: input.githubRef, githubState: "pending" as const } : {}),
    };
    this.comments.push(comment);
    await this.persist();
    return comment;
  }

  /** Kind/state-aware counts per object ref, for 💬 badges (#1210):
   *  badges distinguish notes from actionable comments instead of one
   *  conflated total. */
  async countsDetailed(): Promise<Map<string, CommentKindCounts>> {
    const out = new Map<string, CommentKindCounts>();
    for (const [ref, list] of await this.byObject()) out.set(ref, commentKindCounts(list));
    return out;
  }

  /** Pending triage/posting counts (#1145), for the health view. */
  async pendingCounts(): Promise<PendingCommentCounts> {
    return pendingCommentCounts(await this.all());
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
    // A result report carries exactly one outcome (#1686): a url
    // (posted) or an error (failed). A report with neither — or
    // both — states no outcome and is refused with the state
    // unchanged (the endpoint maps the throw to a 400): the old
    // code mapped any url-less report to failed, so an empty {}
    // wrote a pending comment off unposted, and flipped a posted
    // comment back to failed with its githubUrl still set.
    if (!result.url && !result.error) {
      throw new Error("github result needs a url or an error");
    }
    if (result.url && result.error) {
      throw new Error("github result cannot carry both a url and an error");
    }
    if (result.url) {
      if (c.githubState === "posted") {
        // Posted is terminal. A repeat of the same url is an
        // idempotent no-op; a different url is a conflicting
        // report — the first posted result stands. The no-op still
        // persists first: the original report may have flipped the
        // in-memory state and then failed its write, and
        // acknowledging success here without flushing would leave
        // the on-disk comment pending — a restart would post it
        // again and duplicate the GitHub comment (#1686).
        if (c.githubUrl === result.url) {
          await this.persist();
          return c;
        }
        throw new Error("github comment already posted with a different url");
      }
      // From pending — or from failed, when a retry actually
      // landed the comment: failed is not terminal against
      // success.
      c.githubState = "posted";
      c.githubUrl = result.url;
      delete c.githubError;
    } else {
      // An error report never moves a posted comment backwards.
      if (c.githubState === "posted") {
        throw new Error("github comment already posted; a later failure cannot un-post it");
      }
      if (c.githubState === "failed") {
        // Repeat failure: idempotent — but persist first, for the
        // same reason as the posted repeat above (#1686). The stored
        // last error still refreshes (#1210): "last error" means the
        // newest report, not the first failure.
        c.githubError = result.error;
        await this.persist();
        return c;
      }
      c.githubState = "failed";
      c.githubError = result.error;
    }
    await this.persist();
    return c;
  }
}

/** Pending-work counts for the dashboard comment loops (#1145): a
 *  message comment is pending until the orchestrator triages it
 *  (triageState "new"); a github comment is pending until a fleet
 *  poster lands it (githubState "pending"). The health view publishes
 *  these so an unworked queue is visible instead of silent. */
export interface PendingCommentCounts {
  pendingMessages: number;
  pendingGithub: number;
  total: number;
}

export function pendingCommentCounts(comments: DashComment[]): PendingCommentCounts {
  let pendingMessages = 0;
  let pendingGithub = 0;
  for (const c of comments) {
    if (c.kind === "message" && c.triageState === "new") pendingMessages++;
    if (c.kind === "github" && c.githubState === "pending") pendingGithub++;
  }
  return { pendingMessages, pendingGithub, total: comments.length };
}

/** Whether a comment still waits on a loop (#1210): a message
 *  awaiting triage, or a github comment pending or failed to post.
 *  Notes and terminal states (triaged / posted) are never queued. */
export function isQueuedComment(c: DashComment): boolean {
  if (c.kind === "message") return c.triageState === "new";
  if (c.kind === "github") return c.githubState === "pending" || c.githubState === "failed";
  return false;
}

/** The comments queue (#1210): every queued comment, oldest first. */
export function queuedComments(comments: DashComment[]): DashComment[] {
  return comments.filter(isQueuedComment).sort((a, b) => a.createdAt.localeCompare(b.createdAt));
}

/** Kind/state-aware counts over a comment list (#1210). */
export interface CommentKindCounts {
  total: number;
  notes: number;
  messages: number;
  messagesNew: number;
  github: number;
  githubPending: number;
  githubFailed: number;
  /** Messages awaiting triage + github comments pending/failed. */
  actionable: number;
}

export function commentKindCounts(comments: DashComment[]): CommentKindCounts {
  const out: CommentKindCounts = {
    total: 0,
    notes: 0,
    messages: 0,
    messagesNew: 0,
    github: 0,
    githubPending: 0,
    githubFailed: 0,
    actionable: 0,
  };
  for (const c of comments) {
    out.total++;
    if (c.kind === "note") {
      out.notes++;
    } else if (c.kind === "message") {
      out.messages++;
      if (c.triageState === "new") {
        out.messagesNew++;
        out.actionable++;
      }
    } else if (c.kind === "github") {
      out.github++;
      if (c.githubState === "pending") {
        out.githubPending++;
        out.actionable++;
      } else if (c.githubState === "failed") {
        out.githubFailed++;
        out.actionable++;
      }
    }
  }
  return out;
}

/** The URL a posted GitHub comment lives at, for rendering. */
export function githubCommentUrl(c: DashComment): string | null {
  if (c.githubUrl) return c.githubUrl;
  if (c.githubRef) return refUrl(c.githubRef);
  return null;
}
