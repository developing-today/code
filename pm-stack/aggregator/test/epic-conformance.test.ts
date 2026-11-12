import { describe, expect, test } from "bun:test";
import { epicConformanceFindings, isEpicCandidate, type EpicIssue } from "../scripts/check-epic-conformance.ts";

describe("epic intake conformance", () => {
  const templated: EpicIssue = {
    number: 1,
    title: "epic(example): a coherent program",
    labels: ["epic", "kind/feature", "app/pm-stack"],
    projectItems: [{ project: { number: 4 } }],
    body: `## Owner\n\nOwner: @maintainer\n\n## Definition of done\n\nAll outcomes have verifiable acceptance.`,
  };

  test("accepts a templated epic with labels, Project #4 membership, owner, and done condition", () => {
    expect(isEpicCandidate(templated)).toBe(true);
    expect(epicConformanceFindings(templated)).toEqual([]);
  });

  test("reports current gaps on #1047 even after its epic label was repaired", () => {
    const repairedLabelButIncomplete: EpicIssue = {
      number: 1047,
      title: "epic(usage-hub): SaaS/cloud services completion",
      labels: ["epic", "kind/feature", "app/pm-stack"],
      projectItems: [],
      body: "## Done when\n\nEvery native sub-issue closes and the hub is live.",
    };
    expect(epicConformanceFindings(repairedLabelButIncomplete, { canReadProjects: true })).toEqual([
      "not attached to org Project #4 (set Area, Horizon, and Kind=Epic there)",
      "missing named Owner: line",
    ]);
    expect(epicConformanceFindings(repairedLabelButIncomplete, { canReadProjects: false })).toEqual([
      "missing named Owner: line",
    ]);
  });

  test("does not classify an ordinary feature as an epic from incidental body text", () => {
    expect(isEpicCandidate({ title: "feat(pm-stack): epic templates", labels: ["kind/feature"] })).toBe(false);
  });
});
