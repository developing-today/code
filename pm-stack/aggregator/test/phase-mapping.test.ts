// #9923 step 3: tests for the Phase tracker F-item mapping — the
// parser for the source-of-record table in planning/roadmaps/id.md
// and the per-phase progress derivation against live GhItem state.
// Oracle: an item is done iff its mapped ref is closed (issue) or
// merged/closed (PR) in live state; a kind mismatch is unresolved,
// never done.
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
  PHASE_MAPPING_REPO,
  deriveMappedProgress,
  parsePhaseMapping,
  type PhaseMappingEntry,
} from "../src/phase-mapping.ts";
import type { GhItem } from "../src/model.ts";

const FIXTURE = `# Roadmap: id

## Now — the path to Phase 12

| Title | Status | Intent | Links |
| --- | --- | --- | --- |
| Validate the delivered work | in-progress | Audits. | #1075 |

## Phase tracker — F-item mapping (source of record for #9923)

| Phase | Item | Ref | Title |
| --- | --- | --- | --- |
| 11 | F11.1 | #238 | Relay kit location |
| 11 | F11.6 | PR #247 | Per-call fuel metrics and budgets |
| 13 | F13.4 | #786 | Presentation capability tiers |

## Later — Phase 14

| Topic | Core |
| --- | --- |
| Hosted worlds | #6276 |
`;

const item = (number: number, state: GhItem["state"], kind: GhItem["kind"] = "issue"): GhItem => ({
  ref: `${PHASE_MAPPING_REPO}#${number}`,
  kind,
  url: `https://github.com/${PHASE_MAPPING_REPO}/issues/${number}`,
  title: `live title ${number}`,
  state,
  labels: [],
  author: "dezren39",
  updatedAt: "2026-11-14T00:00:00Z",
});

describe("parsePhaseMapping (#9923)", () => {
  test("reads only the mapping section's F-item rows", () => {
    const entries = parsePhaseMapping(FIXTURE);
    expect(entries).toEqual([
      { phase: 11, item: "F11.1", kind: "issue", number: 238, title: "Relay kit location" },
      { phase: 11, item: "F11.6", kind: "pr", number: 247, title: "Per-call fuel metrics and budgets" },
      { phase: 13, item: "F13.4", kind: "issue", number: 786, title: "Presentation capability tiers" },
    ]);
  });

  test("no mapping section parses to no entries", () => {
    expect(parsePhaseMapping("# Roadmap: id\n\n| Title | Links |\n| --- | --- |\n| X | #1 |\n")).toEqual([]);
    expect(parsePhaseMapping("")).toEqual([]);
  });

  test("the real planning/roadmaps/id.md carries the researched mapping", () => {
    const content = readFileSync(join(import.meta.dir, "../../../planning/roadmaps/id.md"), "utf8");
    const entries = parsePhaseMapping(content);
    const byItem = new Map(entries.map((e) => [e.item, e]));
    // Phase 11: all 18 items, including the two PR-delivered ones.
    for (let n = 1; n <= 18; n++) expect(byItem.has(`F11.${n}`)).toBe(true);
    expect(byItem.get("F11.1")).toMatchObject({ kind: "issue", number: 238 });
    expect(byItem.get("F11.6")).toMatchObject({ kind: "pr", number: 247 });
    expect(byItem.get("F11.12")).toMatchObject({ kind: "issue", number: 227 });
    expect(byItem.get("F11.14")).toMatchObject({ kind: "pr", number: 248 });
    // Phase 13: all 8 items, #783-#790 in order.
    for (let n = 1; n <= 8; n++) expect(byItem.get(`F13.${n}`)?.number).toBe(782 + n);
    // Phase 12 stays sourced from #1235, never duplicated here.
    expect(entries.some((e) => e.phase === 12)).toBe(false);
  });
});

describe("deriveMappedProgress (#9923)", () => {
  const entries: PhaseMappingEntry[] = [
    { phase: 11, item: "F11.1", kind: "issue", number: 238, title: "" },
    { phase: 11, item: "F11.6", kind: "pr", number: 247, title: "" },
    { phase: 11, item: "F11.12", kind: "issue", number: 227, title: "" },
    { phase: 13, item: "F13.4", kind: "issue", number: 786, title: "" },
  ];

  test("closed issue and merged PR count done; open counts open; missing counts unresolved", () => {
    const items = new Map<string, GhItem>([
      [`${PHASE_MAPPING_REPO}#238`, item(238, "closed")],
      [`${PHASE_MAPPING_REPO}#247`, item(247, "merged", "pr")],
      [`${PHASE_MAPPING_REPO}#786`, item(786, "open")],
    ]);
    const progress = deriveMappedProgress(entries, items);
    expect(progress.get(11)).toEqual({ total: 3, done: 2, open: 0, unresolved: 1, percent: 67 });
    expect(progress.get(13)).toEqual({ total: 1, done: 0, open: 1, unresolved: 0, percent: 0 });
    expect(progress.has(12)).toBe(false);
  });

  test("a kind mismatch is unresolved, never done", () => {
    const items = new Map<string, GhItem>([
      // #247 resolved as an issue, but the mapping says PR: no credit.
      [`${PHASE_MAPPING_REPO}#247`, item(247, "closed", "issue")],
    ]);
    const progress = deriveMappedProgress(entries, items);
    expect(progress.get(11)?.done).toBe(0);
    expect(progress.get(11)?.unresolved).toBe(3);
  });
});
