// The projector: turn GitHub webhook events (and reconciliation payloads) into
// registry + read-model rows. Pure transformation + store writes. No network,
// no writes back to GitHub. The store is the only side effect.
import type {
  upsertItem,
  upsertComment,
  markCommentDeleted,
  deleteItem,
  recordPush,
  ItemRow,
  CommentRow,
  HotEventInput,
} from "./store";

// A thin store interface keeps the projector testable against an in-memory
// fake, independent of libSQL.
export interface ProjectorStore {
  upsertItem(i: Omit<ItemRow, "fetched_at">): Promise<void>;
  upsertComment(c: Omit<CommentRow, "fetched_at">): Promise<void>;
  markCommentDeleted(c: Omit<CommentRow, "fetched_at">): Promise<void>;
  deleteItem(ref: string, sourceUpdatedAt?: string): Promise<void>;
  /**
   * Optional deletion-marker read (#6782). Optional so lightweight test
   * stores keep working; the real libSQL store always provides it, and
   * its upsertItem guard absorbs stale replays even without this check.
   */
  itemDeletion?(ref: string): Promise<{ source_updated_at: string } | null>;
  recordPush(p: {
    deliveryId: string;
    repo: string;
    ref: string | null;
    headSha: string | null;
    beforeSha: string | null;
    pusher: string | null;
  }): Promise<void>;
  /**
   * Optional hot-list append (#5204). Optional so lightweight test stores
   * that only exercise the item/comment projection keep working; the real
   * libSQL store always provides it.
   */
  appendHotEvents?(events: HotEventInput[]): Promise<number>;
}

export interface ProjectResult {
  refs: string[];
  wrote: boolean;
}

const iso = (v: unknown): string | null => (typeof v === "string" && v ? v : null);

const jstr = (v: unknown): string => {
  try {
    return JSON.stringify(v ?? []);
  } catch {
    return "[]";
  }
};

function itemRef(repo: string, number: number): string {
  return `${repo}#${number}`;
}

function fromIssueOrPr(repo: string, obj: Record<string, any>, kind: "issue" | "pr"): Omit<ItemRow, "fetched_at"> {
  const number = obj.number as number;
  const createdAt = iso(obj.created_at) ?? iso(obj.updated_at) ?? new Date().toISOString();
  const updatedAt = iso(obj.updated_at) ?? createdAt;
  const labels = Array.isArray(obj.labels)
    ? obj.labels.map((l: any) => (typeof l === "string" ? l : l?.name)).filter(Boolean)
    : [];
  const assignees = Array.isArray(obj.assignees) ? obj.assignees.map((a: any) => a?.login).filter(Boolean) : [];
  // Merge fields (draft / merged / merged_at) exist only on PR-shaped
  // payloads. Issue-shaped payloads (the `issue` object of issues and
  // issue_comment events, even when it describes a PR) carry none of them:
  // emit null so the store preserves the stored values instead of
  // resetting them to defaults (#4158).
  const carriesMergeFields = "merged" in obj || "merged_at" in obj || "draft" in obj;
  // The pulls LIST endpoint carries merged_at but no merged boolean
  // (only the single-PR get and webhook payloads carry both), so derive
  // merged from merged_at when the boolean is absent (#4528, salvaged
  // from the duplicate PR #4687; fixes the Codex P1 on PR #4597).
  const merged = !carriesMergeFields ? null : obj.merged != null ? (obj.merged ? 1 : 0) : obj.merged_at != null ? 1 : 0;
  return {
    ref: itemRef(repo, number),
    repo,
    number,
    kind,
    url: obj.html_url ?? "",
    api_url: obj.url ?? "",
    title: obj.title ?? "",
    state: obj.state === "closed" ? "closed" : "open",
    state_reason: iso(obj.state_reason),
    draft: carriesMergeFields ? (obj.draft ? 1 : 0) : null,
    merged,
    author: iso(obj.user?.login),
    labels: jstr(labels),
    assignees: jstr(assignees),
    created_at: createdAt,
    updated_at: updatedAt,
    closed_at: iso(obj.closed_at),
    merged_at: carriesMergeFields ? iso(obj.merged_at) : null,
    source_updated_at: updatedAt,
  };
}

function commentCommon(
  c: Record<string, any>,
  repo: string,
  ref: string,
  kind: CommentRow["kind"],
): Omit<CommentRow, "fetched_at"> | null {
  // GitHub review objects use submitted_at as their source timestamp. In
  // particular, webhook review payloads do not carry created_at/updated_at;
  // falling back to delivery time poisons the store's monotonic upsert guard.
  const createdAt = kind === "review"
    ? iso(c.submitted_at)
    : (iso(c.created_at) ?? iso(c.updated_at));
  if (!createdAt) return null;
  const updatedAt = iso(c.updated_at);
  return {
    id: String(c.id),
    ref,
    kind,
    author: iso(c.user?.login),
    body: typeof c.body === "string" ? c.body : null,
    created_at: createdAt,
    updated_at: updatedAt,
    url: iso(c.html_url),
    path: null,
    line: null,
    diff_hunk: null,
    review_state: null,
    submitted_at: kind === "review" ? iso(c.submitted_at) : null,
    commit_id: null,
    deleted: 0,
    source_updated_at: kind === "review" ? createdAt : (updatedAt ?? createdAt),
  };
}

/** The shared canonical row constructor for webhook and reconcile reviews. */
export function reviewCommentRow(
  review: Record<string, any>,
  repo: string,
  ref: string,
): Omit<CommentRow, "fetched_at"> | null {
  const row = commentCommon(review, repo, ref, "review");
  if (!row) return null;
  row.updated_at = null;
  row.review_state = iso(review.state)?.toLowerCase() ?? null;
  row.commit_id = iso(review.commit_id);
  return row;
}

/** Canonical repo name "owner/repo" from an event's repository object. */
export function repoOf(payload: Record<string, any>): string | null {
  const repo = payload?.repository?.full_name;
  return typeof repo === "string" && repo ? repo : null;
}

/** GitHub webhook payloads describe their PR in several shapes. Resolve it. */
export function prFromPayload(payload: Record<string, any>): Record<string, any> | null {
  for (const key of ["pull_request", "issue", "review", "comment"]) {
    const obj = payload?.[key];
    if (obj && (obj.html_url?.includes("/pull/") || key === "pull_request" || key === "review")) {
      return obj;
    }
  }
  return null;
}

/** An issue event may carry a PR (issues events also fire for PRs). */
function issueIsPr(issue: Record<string, any>): boolean {
  return Boolean(issue?.pull_request) || Boolean(issue?.html_url?.includes("/pull/"));
}

export { fromIssueOrPr, issueIsPr, itemRef };

/**
 * Apply a single GitHub webhook event to the store. Returns the item refs
 * touched and whether anything was written. `repos` is the allow-list of
 * owner/repo names (empty = accept all).
 */
export async function applyEvent(
  store: ProjectorStore,
  event: string,
  payload: Record<string, any>,
  repos: string[] = [],
  deliveryId = "",
): Promise<ProjectResult> {
  const repo = repoOf(payload);
  if (!repo) return { refs: [], wrote: false };
  if (repos.length > 0 && !repos.includes(repo)) {
    return { refs: [], wrote: false };
  }
  const refs: string[] = [];
  let wrote = false;
  const addRef = (r: string | null) => {
    if (r && !refs.includes(r)) refs.push(r);
  };

  // Hot list (#5204): one normalized ledger entry per handled sub-event.
  // The event_id is stable per (delivery, sub-event): a redelivery of the
  // same x-github-delivery derives the same ids and appends nothing twice,
  // while distinct deliveries stay distinct rows. With no delivery id
  // (direct applyEvent calls), the source timestamp keeps the id stable.
  const hotEvents: HotEventInput[] = [];
  const senderLogin = iso(payload.sender?.login);
  const hot = (sub: string, fields: Partial<HotEventInput>) => {
    hotEvents.push({
      eventId: deliveryId ? `${deliveryId}:${sub}` : `${event}:${sub}:${fields.sourceTimestamp ?? ""}`,
      deliveryId: deliveryId || null,
      repo,
      event,
      action: iso(payload.action) ?? (event === "push" ? "push" : null),
      ref: null,
      actor: senderLogin,
      sourceUrl: null,
      sourceTimestamp: null,
      ...fields,
    });
  };

  // A deletion marker at or after the incoming item's source timestamp
  // absorbs the whole sub-event (#6782): no item resurrection, and no
  // comment/review rows projected for an item that no longer exists.
  const deletionBlocks = async (ref: string, sourceUpdatedAt: string): Promise<boolean> => {
    if (!store.itemDeletion) return false;
    const marker = await store.itemDeletion(ref);
    return marker !== null && marker.source_updated_at >= sourceUpdatedAt;
  };

  const applyIssue = async (issue: Record<string, any>) => {
    const kind = issueIsPr(issue) ? "pr" : "issue";
    const row = fromIssueOrPr(repo, issue, kind);
    if (await deletionBlocks(row.ref, row.source_updated_at)) return;
    await store.upsertItem(row);
    addRef(itemRef(repo, issue.number));
    hot(`${kind}:${issue.id ?? issue.number}`, {
      ref: itemRef(repo, issue.number),
      actor: iso(issue.user?.login) ?? senderLogin,
      sourceUrl: iso(issue.html_url),
      sourceTimestamp: iso(issue.updated_at) ?? iso(issue.created_at),
    });
    wrote = true;
  };

  const applyComment = async (comment: Record<string, any>) => {
    const issue = payload.issue ?? payload.pull_request;
    if (!issue) return;
    const itemRow = fromIssueOrPr(repo, issue, issueIsPr(issue) ? "pr" : "issue");
    if (await deletionBlocks(itemRow.ref, itemRow.source_updated_at)) {
      // The parent item stays absent, but a comment *deletion* still
      // records its tombstone (Codex P2 on PR #6792): reconcile writes
      // comments through a direct upsertComment that consults comment
      // tombstones, not item markers — skipping the tombstone here
      // would let an in-flight reconcile resurrect the deleted comment.
      if (event === "issue_comment" && payload.action === "deleted") {
        const row = commentCommon(comment, repo, itemRef(repo, issue.number), "issue_comment");
        if (row) {
          await store.markCommentDeleted(row);
          wrote = true;
        }
      }
      return;
    }
    await store.upsertItem(itemRow);
    addRef(itemRef(repo, issue.number));
    const row = commentCommon(comment, repo, itemRef(repo, issue.number), "issue_comment");
    if (row) {
      if (event === "issue_comment" && payload.action === "deleted") {
        await store.markCommentDeleted(row);
      } else {
        await store.upsertComment(row);
      }
    }
    hot(`issue_comment:${comment.id}`, {
      ref: itemRef(repo, issue.number),
      actor: iso(comment.user?.login) ?? senderLogin,
      sourceUrl: iso(comment.html_url),
      sourceTimestamp: iso(comment.updated_at) ?? iso(comment.created_at),
    });
    wrote = true;
  };

  switch (event) {
    case "issues": {
      const issue = payload.issue;
      if (issue && payload.action === "deleted") {
        // A deleted issue must leave the read model, not be re-upserted
        // by its own deletion event (#4210).
        const ref = itemRef(repo, issue.number);
        await store.deleteItem(ref, iso(issue.updated_at) ?? iso(issue.created_at) ?? undefined);
        addRef(ref);
        hot(`issue:${issue.id ?? issue.number}`, {
          ref,
          actor: senderLogin,
          sourceUrl: iso(issue.html_url),
          sourceTimestamp: iso(issue.updated_at) ?? iso(issue.created_at),
        });
        wrote = true;
      } else if (issue) await applyIssue(issue);
      break;
    }
    case "pull_request": {
      const pr = payload.pull_request;
      if (pr) {
        const row = fromIssueOrPr(repo, pr, "pr");
        if (await deletionBlocks(row.ref, row.source_updated_at)) break;
        await store.upsertItem(row);
        addRef(itemRef(repo, pr.number));
        hot(`pr:${pr.id ?? pr.number}`, {
          ref: itemRef(repo, pr.number),
          actor: iso(pr.user?.login) ?? senderLogin,
          sourceUrl: iso(pr.html_url),
          sourceTimestamp: iso(pr.updated_at) ?? iso(pr.created_at),
        });
        wrote = true;
      }
      break;
    }
    case "issue_comment": {
      if (payload.comment) await applyComment(payload.comment);
      break;
    }
    case "pull_request_review": {
      const review = payload.review;
      const pr = payload.pull_request;
      if (review && pr) {
        const itemRow = fromIssueOrPr(repo, pr, "pr");
        if (await deletionBlocks(itemRow.ref, itemRow.source_updated_at)) break;
        await store.upsertItem(itemRow);
        addRef(itemRef(repo, pr.number));
        const ref = itemRef(repo, pr.number);
        const reviewRow = reviewCommentRow(review, repo, ref);
        if (reviewRow) await store.upsertComment(reviewRow);
        hot(`review:${review.id}`, {
          ref,
          actor: iso(review.user?.login) ?? senderLogin,
          sourceUrl: iso(review.html_url),
          sourceTimestamp: iso(review.submitted_at) ?? iso(review.updated_at) ?? iso(review.created_at),
        });
        wrote = true;
      }
      break;
    }
    case "pull_request_review_comment": {
      const comment = payload.comment;
      const pr = payload.pull_request;
      if (comment && pr) {
        const itemRow = fromIssueOrPr(repo, pr, "pr");
        const ref = itemRef(repo, pr.number);
        const base = commentCommon(comment, repo, ref, "review_comment");
        if (!base) break;
        base.path = iso(comment.path);
        base.line = typeof comment.line === "number" ? comment.line : null;
        base.diff_hunk = iso(comment.diff_hunk);
        if (await deletionBlocks(itemRow.ref, itemRow.source_updated_at)) {
          // Same tombstone preservation as applyComment (Codex P2 on
          // PR #6792): suppress the parent-item and live-comment
          // writes, but a review-comment deletion keeps its tombstone.
          if (payload.action === "deleted") {
            await store.markCommentDeleted(base);
            wrote = true;
          }
          break;
        }
        await store.upsertItem(itemRow);
        addRef(itemRef(repo, pr.number));
        if (payload.action === "deleted") {
          await store.markCommentDeleted(base);
        } else {
          await store.upsertComment(base);
        }
        hot(`review_comment:${comment.id}`, {
          ref,
          actor: iso(comment.user?.login) ?? senderLogin,
          sourceUrl: iso(comment.html_url),
          sourceTimestamp: iso(comment.updated_at) ?? iso(comment.created_at),
        });
        wrote = true;
      }
      break;
    }
    case "push": {
      await store.recordPush({
        deliveryId,
        repo,
        ref: iso(payload.ref),
        headSha: iso(payload.after),
        beforeSha: iso(payload.before),
        pusher: iso(payload.pusher?.name),
      });
      const commits: Record<string, any>[] = Array.isArray(payload.commits) ? payload.commits : [];
      if (commits.length === 0) {
        hot(`push:${iso(payload.after) ?? "unknown"}`, {
          ref: iso(payload.ref),
          actor: iso(payload.pusher?.name) ?? senderLogin,
          sourceUrl: iso(payload.head_commit?.url),
          sourceTimestamp: iso(payload.head_commit?.timestamp),
        });
      }
      for (const commit of commits) {
        hot(`commit:${commit.id}`, {
          ref: iso(payload.ref),
          actor: iso(commit.author?.username) ?? iso(payload.pusher?.name) ?? senderLogin,
          sourceUrl: iso(commit.url),
          sourceTimestamp: iso(commit.timestamp),
        });
      }
      wrote = true;
      break;
    }
    default:
      break;
  }
  if (hotEvents.length > 0 && store.appendHotEvents) {
    await store.appendHotEvents(hotEvents);
  }
  return { refs, wrote };
}
