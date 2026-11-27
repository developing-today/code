import { describe, expect, it } from "vitest";
import { deriveQueueRows, mentionsFleetIdentity } from "../src/derive";

const REPO = "developing-today/code";
const DELIVERY = "del-1234";

// Oracles are the C2 design record's kind table (D3): which webhook
// shapes derive which queue rows, and that a derived row shares its
// raw event's delivery id.
describe("deriveQueueRows (C2 D3 kind table)", () => {
  it("issues opened/reopened derive issue_new; other actions derive nothing", () => {
    const opened = deriveQueueRows(
      "issues",
      { action: "opened", issue: { number: 42, title: "Bug" } },
      REPO,
      DELIVERY,
    );
    expect(opened).toHaveLength(1);
    expect(opened[0].kind).toBe("issue_new");
    expect(opened[0].detail).toMatchObject({ repo: REPO, number: 42, delivery_id: DELIVERY });
    expect(
      deriveQueueRows("issues", { action: "reopened", issue: { number: 42 } }, REPO, DELIVERY)[0]?.kind,
    ).toBe("issue_new");
    expect(deriveQueueRows("issues", { action: "closed", issue: { number: 42 } }, REPO, DELIVERY)).toEqual([]);
    expect(deriveQueueRows("issues", { action: "labeled", issue: { number: 42 } }, REPO, DELIVERY)).toEqual([]);
  });

  it("pull_request opened/reopened/ready_for_review derive pr_open", () => {
    for (const action of ["opened", "reopened", "ready_for_review"]) {
      const rows = deriveQueueRows("pull_request", { action, pull_request: { number: 7 } }, REPO, DELIVERY);
      expect(rows.map((r) => r.kind), action).toEqual(["pr_open"]);
      expect(rows[0].detail.delivery_id).toBe(DELIVERY);
    }
    expect(
      deriveQueueRows("pull_request", { action: "synchronize", pull_request: { number: 7 } }, REPO, DELIVERY),
    ).toEqual([]);
  });

  it("issue_comment naming a fleet identity derives mention; other comments derive nothing", () => {
    const rows = deriveQueueRows(
      "issue_comment",
      { action: "created", issue: { number: 9 }, comment: { id: 555, body: "thanks @xaltha, please look" } },
      REPO,
      DELIVERY,
    );
    expect(rows).toHaveLength(1);
    expect(rows[0].kind).toBe("mention");
    expect(rows[0].detail).toMatchObject({ number: 9, comment_id: 555, delivery_id: DELIVERY });
    expect(
      deriveQueueRows(
        "issue_comment",
        { action: "created", issue: { number: 9 }, comment: { id: 556, body: "nice work" } },
        REPO,
        DELIVERY,
      ),
    ).toEqual([]);
  });

  it("pull_request_review: changes_requested derives pr_review; a body naming an identity also derives mention", () => {
    const rows = deriveQueueRows(
      "pull_request_review",
      {
        action: "submitted",
        pull_request: { number: 11 },
        review: { id: 77, state: "changes_requested", body: "dezren39 should see this" },
      },
      REPO,
      DELIVERY,
    );
    expect(rows.map((r) => r.kind)).toEqual(["pr_review", "mention"]);
    expect(rows.every((r) => r.detail.delivery_id === DELIVERY)).toBe(true);
    expect(
      deriveQueueRows(
        "pull_request_review",
        { action: "submitted", pull_request: { number: 11 }, review: { id: 78, state: "approved", body: "lgtm" } },
        REPO,
        DELIVERY,
      ),
    ).toEqual([]);
  });

  it("workflow_run derives ci_failure only for a completed failure on the default branch", () => {
    const payload = (branch: string, conclusion: string, action = "completed") => ({
      action,
      repository: { default_branch: "main" },
      workflow_run: { id: 999, name: "CI", head_branch: branch, conclusion },
    });
    const rows = deriveQueueRows("workflow_run", payload("main", "failure"), REPO, DELIVERY);
    expect(rows).toHaveLength(1);
    expect(rows[0].kind).toBe("ci_failure");
    expect(rows[0].detail).toMatchObject({ run_id: 999, branch: "main", delivery_id: DELIVERY });
    expect(deriveQueueRows("workflow_run", payload("feature-x", "failure"), REPO, DELIVERY)).toEqual([]);
    expect(deriveQueueRows("workflow_run", payload("main", "success"), REPO, DELIVERY)).toEqual([]);
    expect(deriveQueueRows("workflow_run", payload("main", "failure", "requested"), REPO, DELIVERY)).toEqual([]);
  });

  it("log-only kinds derive nothing", () => {
    expect(deriveQueueRows("push", { ref: "refs/heads/main" }, REPO, DELIVERY)).toEqual([]);
    expect(deriveQueueRows("sub_issues", { action: "parent_issue_added" }, REPO, DELIVERY)).toEqual([]);
    expect(deriveQueueRows("ping", {}, REPO, DELIVERY)).toEqual([]);
  });
});

describe("mentionsFleetIdentity", () => {
  it("matches on login boundaries, case-insensitively", () => {
    expect(mentionsFleetIdentity("@Xaltha: ping")).toBe(true);
    expect(mentionsFleetIdentity("cc dezren39")).toBe(true);
    expect(mentionsFleetIdentity("xaltham is not it")).toBe(false);
    expect(mentionsFleetIdentity("")).toBe(false);
    expect(mentionsFleetIdentity(null)).toBe(false);
    expect(mentionsFleetIdentity(42)).toBe(false);
  });
});
