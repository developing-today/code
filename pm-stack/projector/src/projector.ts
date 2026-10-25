// The projector: turn GitHub webhook events (and reconciliation payloads) into
// registry + read-model rows. Pure transformation + store writes. No network,
// no writes back to GitHub. The store is the only side effect.
import type { upsertItem, upsertComment, markCommentDeleted, recordPush, ItemRow, CommentRow } from "./store";

// A thin store interface keeps the projector testable against an in-memory
// fake, independent of libSQL.
export interface ProjectorStore {
  upsertItem(i: Omit<ItemRow, "fetched_at">): Promise<void>;
  upsertComment(c: Omit<CommentRow, "fetched_at">): Promise<void>;
  markCommentDeleted(id: string): Promise<void>;
  recordPush(p: {
    deliveryId: string;
    repo: string;
    ref: string | null;
    headSha: string | null;
    beforeSha: string | null;
    pusher: string | null;
  }): Promise<void>;
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
    draft: obj.draft ? 1 : 0,
    merged: obj.merged ? 1 : 0,
    author: iso(obj.user?.login),
    labels: jstr(labels),
    assignees: jstr(assignees),
    created_at: createdAt,
    updated_at: updatedAt,
    closed_at: iso(obj.closed_at),
    merged_at: iso(obj.merged_at),
    source_updated_at: updatedAt,
  };
}

function commentCommon(
  c: Record<string, any>,
  repo: string,
  ref: string,
  kind: CommentRow["kind"],
): Omit<CommentRow, "fetched_at"> {
  const createdAt = iso(c.created_at) ?? iso(c.updated_at) ?? new Date().toISOString();
  return {
    id: String(c.id),
    ref,
    kind,
    author: iso(c.user?.login),
    body: typeof c.body === "string" ? c.body : null,
    created_at: createdAt,
    updated_at: iso(c.updated_at),
    url: iso(c.html_url),
    path: null,
    line: null,
    diff_hunk: null,
    review_state: null,
    submitted_at: null,
    commit_id: null,
    deleted: 0,
    source_updated_at: createdAt,
  };
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

  const applyIssue = async (issue: Record<string, any>) => {
    const kind = issueIsPr(issue) ? "pr" : "issue";
    await store.upsertItem(fromIssueOrPr(repo, issue, kind));
    addRef(itemRef(repo, issue.number));
    wrote = true;
  };

  const applyComment = async (comment: Record<string, any>) => {
    const issue = payload.issue ?? payload.pull_request;
    if (!issue) return;
    await store.upsertItem(fromIssueOrPr(repo, issue, issueIsPr(issue) ? "pr" : "issue"));
    addRef(itemRef(repo, issue.number));
    if (event === "issue_comment" && payload.action === "deleted") {
      await store.markCommentDeleted(String(comment.id));
    } else {
      await store.upsertComment(commentCommon(comment, repo, itemRef(repo, issue.number), "issue_comment"));
    }
    wrote = true;
  };

  switch (event) {
    case "issues": {
      const issue = payload.issue;
      if (issue) await applyIssue(issue);
      break;
    }
    case "pull_request": {
      const pr = payload.pull_request;
      if (pr) {
        await store.upsertItem(fromIssueOrPr(repo, pr, "pr"));
        addRef(itemRef(repo, pr.number));
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
        await store.upsertItem(fromIssueOrPr(repo, pr, "pr"));
        addRef(itemRef(repo, pr.number));
        const ref = itemRef(repo, pr.number);
        const base = commentCommon(review, repo, ref, "review");
        base.review_state = iso(review.state);
        base.submitted_at = iso(review.submitted_at) ?? base.created_at;
        base.commit_id = iso(review.commit_id);
        await store.upsertComment(base);
        wrote = true;
      }
      break;
    }
    case "pull_request_review_comment": {
      const comment = payload.comment;
      const pr = payload.pull_request;
      if (comment && pr) {
        await store.upsertItem(fromIssueOrPr(repo, pr, "pr"));
        addRef(itemRef(repo, pr.number));
        const ref = itemRef(repo, pr.number);
        const base = commentCommon(comment, repo, ref, "review_comment");
        base.path = iso(comment.path);
        base.line = typeof comment.line === "number" ? comment.line : null;
        base.diff_hunk = iso(comment.diff_hunk);
        if (payload.action === "deleted") {
          await store.markCommentDeleted(String(comment.id));
        } else {
          await store.upsertComment(base);
        }
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
      wrote = true;
      break;
    }
    default:
      break;
  }
  return { refs, wrote };
}
