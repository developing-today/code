import { describe, expect, test } from "bun:test";
import {
  collectPrimaryParentViolations,
  itemRefKey,
  primaryParentLabels,
  type EpicSlugMap,
  type IngestionItem,
} from "../src/metadata-ingestion.ts";
import { planPrimaryParentLabels } from "../scripts/sync-primary-parent-labels.ts";

// Oracle for these tests: the invariants in design §3 of
// doc/2026-11-12T22-50-00Z_design_pm-board-hierarchy-genealogy/note.md —
// at most one primary-parent/* per issue, and label/native agreement or the
// label naming an ancestor of the native parent; violations are reported,
// never resolved.

const EPICS: EpicSlugMap = new Map([
  ["id-crate-split", { repo: "developing-today/code", number: 3447 }],
  ["id-program", { repo: "developing-today/code", number: 9060 }],
  ["pm-stack", { repo: "developing-today/code", number: 8983 }],
]);

function item(overrides: Partial<IngestionItem> = {}): IngestionItem {
  return {
    repo: "developing-today/code",
    number: 9999,
    kind: "issue",
    title: "A descendant",
    body: "",
    labels: [],
    state: "OPEN",
    author: "someone",
    createdAt: "2026-11-20T00:00:00Z",
    updatedAt: "2026-11-21T00:00:00Z",
    ...overrides,
  };
}

const ref = (number: number): string => itemRefKey({ repo: "developing-today/code", number });

describe("primaryParentLabels", () => {
  test("selects only the primary-parent family", () => {
    expect(primaryParentLabels(["epic/id-program", "primary-parent/id-program", "kind/feature"])).toEqual([
      "primary-parent/id-program",
    ]);
    expect(primaryParentLabels(["epic/id-program"])).toEqual([]);
  });
});

describe("collectPrimaryParentViolations (§3 invariants)", () => {
  test("no labels is not a violation", () => {
    expect(collectPrimaryParentViolations(item(), EPICS)).toEqual([]);
  });

  test("a single label with no native parent is fine", () => {
    expect(collectPrimaryParentViolations(item({ labels: ["primary-parent/id-program"] }), EPICS)).toEqual([]);
  });

  test("a single label agreeing with the native parent is fine", () => {
    const violations = collectPrimaryParentViolations(
      item({
        labels: ["primary-parent/id-program"],
        nativeParent: { repo: "developing-today/code", number: 9060, kind: "sub-issue" },
      }),
      EPICS,
      new Set([ref(9060)]),
    );
    expect(violations).toEqual([]);
  });

  test("a label naming an ancestor of the native parent is allowed (program-level parent)", () => {
    const violations = collectPrimaryParentViolations(
      item({
        labels: ["primary-parent/id-program"],
        // Native parent is #3447 (crate-split); its ancestor chain includes the program epic #9060.
        nativeParent: { repo: "developing-today/code", number: 3447, kind: "sub-issue" },
      }),
      EPICS,
      new Set([ref(3447), ref(9060)]),
    );
    expect(violations).toEqual([]);
  });

  test("a label disagreeing with the native parent and not an ancestor is reported", () => {
    const violations = collectPrimaryParentViolations(
      item({
        labels: ["primary-parent/pm-stack"],
        nativeParent: { repo: "developing-today/code", number: 3447, kind: "sub-issue" },
      }),
      EPICS,
      new Set([ref(3447), ref(9060)]),
    );
    expect(violations.map((violation) => violation.kind)).toEqual(["label-native-disagreement"]);
    expect(violations[0]?.message).toContain("developing-today/code#8983");
    expect(violations[0]?.message).toContain("developing-today/code#3447");
  });

  test("an unreadable ancestor chain never manufactures a disagreement", () => {
    const violations = collectPrimaryParentViolations(
      item({
        labels: ["primary-parent/id-program"],
        nativeParent: { repo: "developing-today/code", number: 3447, kind: "sub-issue" },
      }),
      EPICS,
      undefined,
    );
    expect(violations).toEqual([]);
  });

  test("two labels are a violation even when they name the same epic", () => {
    const violations = collectPrimaryParentViolations(
      item({ labels: ["primary-parent/id-program", "primary-parent/ID-PROGRAM"] }),
      EPICS,
    );
    expect(violations.map((violation) => violation.kind)).toEqual(["multiple-primary-parent-labels"]);
    expect(violations[0]?.message).toContain("at most one is allowed");
  });

  test("two labels naming distinct epics report both targets", () => {
    const violations = collectPrimaryParentViolations(
      item({ labels: ["primary-parent/id-program", "primary-parent/pm-stack"] }),
      EPICS,
    );
    expect(violations.map((violation) => violation.kind)).toEqual(["multiple-primary-parent-labels"]);
    expect(violations[0]?.message).toContain("2 distinct parents");
    expect(violations[0]?.labelParents.map((parent) => parent.number).sort()).toEqual([8983, 9060]);
  });

  test("a label naming no known epic is reported, never silently dropped", () => {
    const violations = collectPrimaryParentViolations(item({ labels: ["primary-parent/ghost-epic"] }), EPICS);
    expect(violations.map((violation) => violation.kind)).toEqual(["unresolved-primary-parent-label"]);
    expect(violations[0]?.labels).toEqual(["primary-parent/ghost-epic"]);
  });

  test("multiplicity and unresolved findings coexist", () => {
    const violations = collectPrimaryParentViolations(
      item({ labels: ["primary-parent/ghost-epic", "primary-parent/id-program"] }),
      EPICS,
    );
    expect(violations.map((violation) => violation.kind).sort()).toEqual([
      "multiple-primary-parent-labels",
      "unresolved-primary-parent-label",
    ]);
  });
});

describe("planPrimaryParentLabels (label family registry)", () => {
  const labels = [
    { name: "epic/id-crate-split", color: "5319e7" },
    { name: "epic/pm-stack", color: "1d76db" },
    { name: "epic/already-there", color: "5319e7" },
    { name: "primary-parent/already-there", color: "5319e7" },
    { name: "kind/feature", color: "a2eeef" },
  ];

  test("plans one label per epic slug that lacks one, reusing the epic colour", () => {
    const { plan, existing } = planPrimaryParentLabels(labels);
    expect(existing).toEqual(["primary-parent/already-there"]);
    expect(plan.map((entry) => entry.name)).toEqual(["primary-parent/id-crate-split", "primary-parent/pm-stack"]);
    expect(plan[1]?.color).toBe("1d76db");
    expect(plan[0]?.description).toContain("id-crate-split");
  });

  test("a slug too long for GitHub's 50-character label limit is skipped, not truncated", () => {
    const long = `epic/${"x".repeat(36)}`;
    const { plan, skippedTooLong } = planPrimaryParentLabels([{ name: long, color: "5319e7" }]);
    expect(plan).toEqual([]);
    expect(skippedTooLong).toEqual([`primary-parent/${"x".repeat(36)}`]);
  });
});
