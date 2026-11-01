import { describe, expect, test } from "bun:test";
import type { BoardItem, GhItem, OrchestrationSection, Provenance } from "../src/model.ts";
import { renderBoard } from "../src/render.ts";
import { inFlightBoardItems, titleRefs } from "../src/sources/live.ts";

// The board's in-flight lane: synthetic BoardItems built in live.ts
// board() from the fleet feed's orchestration section (#899), so the
// board shows what is being worked on today instead of presenting
// Kaneo's done / in-review completion record as WIP.

const GENERATED = "2026-10-26T16:15:23Z";

function section(
  linedUp: OrchestrationSection["goals"][number]["linedUp"],
  goal = "Keep the four repos healthy",
): OrchestrationSection {
  return {
    version: 1,
    generatedAt: GENERATED,
    runId: `orch-${GENERATED}`,
    provenance: { sources: ["test section"] },
    tiers: { needsDrew: [], needsOrchestrator: [] },
    dependencies: [],
    goals: [
      {
        goalId: null,
        goal,
        momentum: "moving",
        summary: "",
        linedUp,
        progressedSinceLastRun: [],
        stalled: [],
        gaps: [],
      },
    ],
    actions: [],
  };
}

const task = (
  title: string,
  status: "in-progress" | "queued" | "blocked" | "done-recently",
  extra: Partial<OrchestrationSection["goals"][number]["linedUp"][number]> = {},
) => ({ title, status, horizon: "today" as const, horizonWhy: "test", ...extra });

const kaneoCard = (title: string, refs: string[]): BoardItem => ({
  id: `kaneo:code/${title}`,
  title,
  repo: "developing-today/code",
  source: "kaneo",
  status: "done",
  links: refs.map((ref) => ({ ref, relation: "tracks" as const })),
});

describe("titleRefs", () => {
  test("attributes shorthand refs to the nearest repo alias", () => {
    expect(titleRefs("code #944 canonical mark nesting fix - PR #985 opened, task 1091 parked on CI")).toEqual([
      "developing-today/code#944",
      "developing-today/code#985",
    ]);
    expect(titleRefs("nix #889 queued; nix #848 bump task 1127 re-queued")).toEqual([
      "dezren39/nix#889",
      "dezren39/nix#848",
    ]);
    expect(titleRefs("Jules PR #927 for code #811 - still OPEN draft")).toEqual([
      "developing-today/code#927",
      "developing-today/code#811",
    ]);
  });

  test("keeps explicit refs and leaves alias-less numbers unlinked", () => {
    expect(titleRefs("see developing-today/code#958 and dezren39/nix#501")).toEqual([
      "developing-today/code#958",
      "dezren39/nix#501",
    ]);
    expect(titleRefs("queue task 1116 revived, items 3/4/5 picked")).toEqual([]);
    expect(titleRefs("Daily morning readout")).toEqual([]);
  });
});

describe("inFlightBoardItems", () => {
  test("keeps in-progress / queued / blocked, drops done-recently", () => {
    const out = inFlightBoardItems(
      section([
        task("code #944 fix - PR #985 opened", "in-progress"),
        task("code #968 - queue task 1116 revived", "queued"),
        task("Plaid/Citi sign-in", "blocked", {
          blockedBy: [{ task: "Drew's Plaid/Citi sign-in", status: "stalled" }],
        }),
        task("code #983 fix - done via PR #986 merged", "done-recently"),
      ]),
      [],
    );
    expect(out.map((i) => i.orchStatus)).toEqual(["in-progress", "queued", "blocked"]);
    expect(out.every((i) => i.source === "orchestrator" && i.status === "in-flight")).toBe(true);
    expect(out.every((i) => i.updatedAt === GENERATED)).toBe(true);
    expect(out[0].links.map((l) => l.ref)).toEqual(["developing-today/code#944", "developing-today/code#985"]);
    expect(out[0].repo).toBe("developing-today/code");
    expect(out[2].blockedBy).toEqual(["Drew's Plaid/Citi sign-in (stalled)"]);
    expect(out[2].links).toEqual([]);
    expect(out[2].goal).toBe("Keep the four repos healthy");
  });

  test("an explicit linedUp ref is linked even when the title carries none", () => {
    const out = inFlightBoardItems(
      section([task("mcpx slice work", "in-progress", { ref: "developing-today/code#958" })]),
      [],
    );
    expect(out).toHaveLength(1);
    expect(out[0].links).toEqual([{ ref: "developing-today/code#958", relation: "tracks" }]);
  });

  test("dedupes matched refs and ref-less title matches against Kaneo cards", () => {
    const existing = [
      kaneoCard("Some finished card", ["developing-today/code#944"]),
      kaneoCard("Daily morning readout", []),
    ];
    const out = inFlightBoardItems(
      section([
        task("code #944 fix - PR #985 opened", "in-progress"), // ref already on a Kaneo card
        task("code #944 fully represented", "queued"), // its only ref is already on a Kaneo card
        task("Daily morning readout", "queued"), // title already on a Kaneo card
        task("code #968 - queue task 1116 revived", "queued"), // genuinely new
      ]),
      existing,
    );
    expect(out.map((i) => i.title)).toEqual([
      "code #944 fix - PR #985 opened",
      "code #968 - queue task 1116 revived",
    ]);
    expect(out[0].links.map((link) => link.ref)).toEqual(["developing-today/code#985"]);
  });

  test("removes only refs already represented on Kaneo cards", () => {
    const out = inFlightBoardItems(
      section([task("code #944 and openchamber #492 implementation", "in-progress")]),
      [kaneoCard("Some finished card", ["developing-today/code#944"])],
    );

    expect(out).toHaveLength(1);
    expect(out[0].links.map((link) => link.ref)).toEqual(["developing-today/openchamber#492"]);
    expect(out[0].repo).toBe("developing-today/openchamber");
  });

  test("does not let a title-only collision hide a task with distinct refs", () => {
    const out = inFlightBoardItems(
      section([task("Shared orchestrator title", "in-progress", { ref: "developing-today/code#944" })]),
      [kaneoCard("Shared orchestrator title", ["developing-today/code#901"])],
    );

    expect(out).toHaveLength(1);
    expect(out[0].links.map((link) => link.ref)).toEqual(["developing-today/code#944"]);
  });

  test("dedupes repeats inside the section", () => {
    const s = section([
      task("code #968 - queue task 1116 revived", "queued"),
      task("code #968 - queue task 1116 revived", "queued"),
    ]);
    s.goals.push({
      ...s.goals[0],
      goal: "Another goal",
      linedUp: [task("code #968 - queue task 1116 revived", "in-progress")],
    });
    expect(inFlightBoardItems(s, [])).toHaveLength(1);
  });

  // Real-shape sample: titles verbatim from the orchestration section
  // published 2026-10-26T16:15:23Z (no linedUp task carries a `ref`
  // field in that section — refs come from the titles).
  test("builds the lane from the real 2026-10-26 section shape", () => {
    const out = inFlightBoardItems(
      section([
        task("code #983 R2-secret-in-argv fix - done via PR #986 merged, issue CLOSED", "done-recently"),
        task("code #944 canonical mark nesting fix - PR #985 opened, task 1091 parked on CI", "in-progress"),
        task("code PRs #979 (#932) / #980 (#934) - parked on CI", "in-progress"),
        task("code #968 - queue task 1116 revived to queued after monitor correction", "queued"),
        task(
          "nix #889 queued (park condition cleared - PR #887 merged); nix #848 bump task 1127 re-queued after lease expiry (labels truthful, no repair needed)",
          "queued",
        ),
        task(
          "openchamber #492 - task 1124 claim-evaporation repaired by monitor, labels reset to queued with handoff note",
          "queued",
        ),
      ]),
      [],
    );
    expect(out.map((i) => i.title)).not.toContain(
      "code #983 R2-secret-in-argv fix - done via PR #986 merged, issue CLOSED",
    );
    const by944 = out.find((i) => i.title.includes("#944"));
    expect(by944?.links.map((l) => l.ref)).toContain("developing-today/code#985");
    expect(out.find((i) => i.title.includes("#889"))?.repo).toBe("dezren39/nix");
    expect(out.find((i) => i.title.includes("#492"))?.links.map((l) => l.ref)).toEqual([
      "developing-today/openchamber#492",
    ]);
  });
});

describe("renderBoard in-flight lane", () => {
  const provenance = {
    board: "live (Kaneo)",
    roadmap: "fixtures",
    items: "fixtures",
    comments: "fixtures",
    quota: "fixtures",
    usage: "fixtures",
    attention: "fixtures",
    orchestration: "fixtures",
    dns: "fixtures",
    registry: "fixtures",
    aa: "fixtures",
  } as Provenance;

  test("lane renders first, visibly sourced and dated, with direct GitHub links", () => {
    const lane = inFlightBoardItems(section([task("code #944 fix - PR #985 opened", "in-progress")]), []);
    const board: BoardItem[] = [...lane, kaneoCard("Finished thing", ["developing-today/code#983"])];
    const html = renderBoard(board, new Map(), provenance);
    expect(html).toContain("orchestrator · in flight");
    expect(html).toContain(GENERATED.slice(0, 10)); // the lane header carries the section's date
    expect(html).toContain('href="https://github.com/developing-today/code/issues/944"');
    expect(html).toContain("completion history");
    expect(html.indexOf("in flight — orchestrator")).toBeLessThan(html.indexOf("Done recently ("));
  });

  test("partially matched tasks render only their remaining GitHub links", () => {
    const existing = [kaneoCard("Some finished card", ["developing-today/code#944"])] as const;
    const lane = inFlightBoardItems(
      section([
        task("code #944 and openchamber #492 implementation", "in-progress"),
        task("code #944 fully represented", "queued"),
      ]),
      [...existing],
    );
    const items = new Map([
      [
        "developing-today/code#944",
        {
          ref: "developing-today/code#944",
          kind: "issue",
          url: "https://github.com/developing-today/code/issues/944",
          title: "Some finished card",
          state: "closed",
          labels: [],
          author: "test",
          updatedAt: GENERATED,
        } as GhItem,
      ],
    ]);
    const html = renderBoard([...lane, ...existing], items, provenance);

    expect(html).toContain('href="https://github.com/developing-today/openchamber/issues/492"');
    expect(html.match(/href="https:\/\/github.com\/developing-today\/openchamber\/issues\/492"/g)).toHaveLength(1);
    expect(html.match(/href="https:\/\/github.com\/developing-today\/code\/issues\/944"/g)).toHaveLength(1);
  });
});
