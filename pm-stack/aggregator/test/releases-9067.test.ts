import { describe, expect, test } from "bun:test";
import type { ProjectPoolItem } from "../src/model.ts";
import { isProjectPoolItem, parseProjectItems, PROJECT_QUERY } from "../src/projects.ts";
import {
  buildReleaseSnapshots,
  classifyMilestoneItem,
  computeMilestoneSnapshot,
  milestoneKind,
} from "../src/releases.ts";

// #9067: pm-stack release milestones — ingestion, close-out accounting,
// /api/releases.json. Oracle for the accounting: every counted item is
// an issue in exactly one category (epic > design > issue), and
// closed + carriedOver == total in every category.

function poolItem(partial: Partial<ProjectPoolItem>): ProjectPoolItem {
  return {
    id: "PVTI_x",
    url: "https://github.com/orgs/developing-today/projects/4?pane=issue&itemId=1",
    projectUrl: "https://github.com/orgs/developing-today/projects/4",
    ref: "developing-today/code#1",
    repo: "developing-today/code",
    number: 1,
    issueUrl: "https://github.com/developing-today/code/issues/1",
    title: "item",
    state: "open",
    contentKind: "issue",
    status: null,
    area: null,
    horizon: null,
    kind: null,
    ...partial,
  };
}

describe("collector milestone ingestion (#9067)", () => {
  test("PROJECT_QUERY fetches milestone { title dueOn closedAt } per item", () => {
    expect(PROJECT_QUERY).toContain("milestone { title dueOn closedAt }");
  });

  test("parseProjectItems carries the milestone onto the pool item", () => {
    const page = {
      data: {
        node: {
          items: {
            nodes: [
              {
                id: "PVTI_1",
                databaseId: 111,
                type: "ISSUE",
                content: {
                  number: 42,
                  title: "ship it",
                  state: "OPEN",
                  url: "https://github.com/developing-today/code/issues/42",
                  repository: { nameWithOwner: "developing-today/code" },
                  updatedAt: "2026-11-17T00:00:00Z",
                  author: { login: "dezren39" },
                  labels: { nodes: [{ name: "design/id" }] },
                  milestone: {
                    title: "ID platform — release 2026-12-01",
                    dueOn: "2026-12-01T00:00:00Z",
                    closedAt: null,
                  },
                  parent: null,
                  blockedBy: { nodes: [] },
                  blocking: { nodes: [] },
                },
                fieldValues: { nodes: [] },
              },
            ],
          },
        },
      },
    };
    const items = parseProjectItems(page);
    expect(items).toHaveLength(1);
    expect(items[0].milestone).toEqual({
      title: "ID platform — release 2026-12-01",
      dueOn: "2026-12-01T00:00:00Z",
      closedAt: null,
    });
  });

  test("isProjectPoolItem validates a present milestone and stays absence-friendly", () => {
    const withMilestone = poolItem({ milestone: { title: "Atlas program", dueOn: null, closedAt: null } });
    expect(isProjectPoolItem(withMilestone)).toBe(true);
    expect(isProjectPoolItem(poolItem({}))).toBe(true);
    const malformed = poolItem({
      milestone: { title: 7, dueOn: null, closedAt: null } as unknown as ProjectPoolItem["milestone"],
    });
    expect(isProjectPoolItem(malformed)).toBe(false);
  });
});

describe("close-out accounting (#9067)", () => {
  test("classification is disjoint: epic first, then design, then issue", () => {
    expect(classifyMilestoneItem({ state: "open", kind: "Epic", labels: ["design/x"] })).toBe("epic");
    expect(classifyMilestoneItem({ state: "open", labels: ["epic"] })).toBe("epic");
    // epic/<name> marks membership in an epic, not epic-ness.
    expect(classifyMilestoneItem({ state: "open", labels: ["epic/pm-stack"] })).toBe("issue");
    expect(classifyMilestoneItem({ state: "open", labels: ["design/quota-plane"] })).toBe("design");
    expect(classifyMilestoneItem({ state: "open", labels: ["kind/design"] })).toBe("design");
    expect(classifyMilestoneItem({ state: "open", labels: ["kind/bug"] })).toBe("issue");
  });

  test("snapshot splits closed vs carried-over per category and names carried-over epics", () => {
    const milestone = {
      title: "ID platform — release 2026-12-01",
      dueOn: "2026-12-01T00:00:00Z",
      closedAt: "2026-12-01T00:00:00Z",
    };
    const snap = computeMilestoneSnapshot(milestone, [
      { ref: "developing-today/code#1", state: "closed", labels: [] },
      { ref: "developing-today/code#2", state: "open", labels: [] },
      { ref: "developing-today/code#3", state: "closed", labels: ["design/id"] },
      { ref: "developing-today/code#4", state: "open", kind: "Epic", labels: [] },
      { ref: "developing-today/code#5", state: "closed", kind: "Epic", labels: [] },
      { ref: "developing-today/code#6", state: "open", contentKind: "pr", labels: [] },
    ]);
    expect(snap.kind).toBe("release");
    expect(snap.releaseDate).toBe("2026-12-01T00:00:00Z");
    expect(snap.counts.issues).toEqual({ closed: 1, carriedOver: 1, total: 2 });
    expect(snap.counts.designs).toEqual({ closed: 1, carriedOver: 0, total: 1 });
    expect(snap.counts.epics).toEqual({ closed: 1, carriedOver: 1, total: 2 });
    expect(snap.counts.total).toEqual({ closed: 3, carriedOver: 2, total: 5 });
    expect(snap.carriedOverEpics).toEqual(["developing-today/code#4"]);
    expect(snap.descriptionLine).toContain("Closed with the release on 2026-12-01");
    expect(snap.descriptionLine).toContain("2 issues, 1 designs, 2 epics");
  });

  test("program milestones are rolling: no release date, counts-only accounting", () => {
    expect(milestoneKind({ title: "Atlas program", dueOn: null, closedAt: null })).toBe("program");
    const snap = computeMilestoneSnapshot({ title: "Atlas program", dueOn: null, closedAt: null }, [
      { state: "open", labels: [] },
    ]);
    expect(snap.kind).toBe("program");
    expect(snap.releaseDate).toBeNull();
    expect(snap.descriptionLine).toContain("retroactive accounting");
  });

  test("buildReleaseSnapshots groups the pool and unions known milestones (retroactive pass)", () => {
    const program = { title: "Atlas program", dueOn: null, closedAt: null };
    const snaps = buildReleaseSnapshots(
      [
        poolItem({ ref: "developing-today/code#10", milestone: program, state: "closed" }),
        poolItem({ ref: "developing-today/code#11", milestone: program, state: "open", labels: ["design/atlas"] }),
        poolItem({ ref: "developing-today/code#12" }),
      ],
      [program, { title: "Storage program", dueOn: null, closedAt: null }],
    );
    expect(snaps.map((s) => s.milestone.title)).toEqual(["Atlas program", "Storage program"]);
    expect(snaps[0].counts.total).toEqual({ closed: 1, carriedOver: 1, total: 2 });
    expect(snaps[0].counts.designs.total).toBe(1);
    expect(snaps[1].counts.total).toEqual({ closed: 0, carriedOver: 0, total: 0 });
  });
});
