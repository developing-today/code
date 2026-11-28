/**
 * C2 kind derivation (#9570 design record, D3; slice C2-S1).
 *
 * The Worker is the intake of record: beside each raw event it
 * appends to the fleet-queue `events` log, it derives the per-kind
 * queue rows the event implies and appends those too, under the
 * same GitHub delivery id, so the raw history stays intact and a
 * queue (D2) is just the derived rows past its `drain:<kind>`
 * cursor. Pure functions over the webhook payload — the caller
 * (index.ts) appends what comes back.
 *
 * Derived kinds: `issue_new`, `pr_open`, `ci_failure`, `mention`,
 * `pr_review`. Everything else (push, sub_issues, other actions)
 * is log-only and derives nothing.
 */
import { MENTION_IDENTITIES } from "./thresholds";

export interface DerivedRow {
  kind: string;
  detail: Record<string, unknown>;
}

function obj(v: unknown): Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v) ? (v as Record<string, unknown>) : {};
}

function num(v: unknown): number | null {
  const n = Number(v ?? 0);
  return Number.isFinite(n) && n !== 0 ? n : null;
}

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** True when the body names a configured fleet identity on a
 * non-login-character boundary (so "xaltha" matches "@xaltha:" and
 * " xaltha " but not "xaltham"). Case-insensitive. */
export function mentionsFleetIdentity(body: unknown): boolean {
  if (typeof body !== "string" || body.length === 0) return false;
  return MENTION_IDENTITIES.some((id) => new RegExp(`(^|[^a-z0-9-])${escapeRegExp(id)}($|[^a-z0-9-])`, "i").test(body));
}

/**
 * Derive the queue rows for one webhook delivery. `eventType` is
 * the X-GitHub-Event header; `payload` the parsed body; `repoName`
 * the repository full name; `deliveryId` the X-GitHub-Delivery id,
 * carried into every derived row so a derived row and its raw event
 * share a delivery id.
 */
export function deriveQueueRows(
  eventType: string,
  payload: Record<string, unknown>,
  repoName: string,
  deliveryId: string | null,
): DerivedRow[] {
  const action = String(payload.action ?? "");
  const base = { repo: repoName, delivery_id: deliveryId };
  switch (eventType) {
    case "issues": {
      if (action !== "opened" && action !== "reopened") return [];
      const issue = obj(payload.issue);
      return [
        {
          kind: "issue_new",
          detail: { ...base, action, number: num(issue.number), title: (issue.title as string) ?? null },
        },
      ];
    }
    case "pull_request": {
      if (action !== "opened" && action !== "reopened" && action !== "ready_for_review") return [];
      const pr = obj(payload.pull_request);
      return [
        {
          kind: "pr_open",
          detail: { ...base, action, number: num(pr.number), title: (pr.title as string) ?? null },
        },
      ];
    }
    case "issue_comment": {
      const comment = obj(payload.comment);
      if (!mentionsFleetIdentity(comment.body)) return [];
      const issue = obj(payload.issue);
      return [
        {
          kind: "mention",
          detail: { ...base, action, number: num(issue.number), comment_id: num(comment.id) },
        },
      ];
    }
    case "pull_request_review": {
      const review = obj(payload.review);
      const pr = obj(payload.pull_request);
      const rows: DerivedRow[] = [];
      if (action === "submitted" && String(review.state ?? "") === "changes_requested") {
        rows.push({
          kind: "pr_review",
          detail: { ...base, action, number: num(pr.number), review_id: num(review.id) },
        });
      }
      if (mentionsFleetIdentity(review.body)) {
        rows.push({
          kind: "mention",
          detail: { ...base, action, number: num(pr.number), comment_id: num(review.id) },
        });
      }
      return rows;
    }
    case "workflow_run": {
      // ci_failure: a completed run, conclusion failure, on the
      // repo's default branch only (D3). The raw event's home is
      // ci_events (fleet-platform); this derived row is its entry
      // in the event log, linked by run id and delivery id.
      const run = obj(payload.workflow_run);
      const repo = obj(payload.repository);
      if (action !== "completed") return [];
      if (String(run.conclusion ?? "") !== "failure") return [];
      if (typeof repo.default_branch !== "string" || String(run.head_branch ?? "") !== repo.default_branch) return [];
      return [
        {
          kind: "ci_failure",
          detail: {
            ...base,
            action,
            run_id: num(run.id),
            workflow: (run.name as string) ?? null,
            branch: (run.head_branch as string) ?? null,
          },
        },
      ];
    }
    default:
      return [];
  }
}
