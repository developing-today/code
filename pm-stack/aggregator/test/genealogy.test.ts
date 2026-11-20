import { describe, expect, test } from "bun:test";
import { deriveGenealogy, enrichRoadmapRows } from "../src/genealogy.ts";
import { historicalParentsFromMetadata } from "../src/projects.ts";
import type { ProjectPoolItem, RefKey, RoadmapRow } from "../src/model.ts";

const ref = (n: number) => `developing-today/code#${n}` as RefKey;
function item(n: number, partial: Partial<ProjectPoolItem> = {}): ProjectPoolItem {
  return {
    id: `PVTI_${n}`,
    url: "https://example.test/board",
    projectUrl: "https://example.test/board",
    ref: ref(n),
    repo: "developing-today/code",
    number: n,
    issueUrl: `https://github.com/developing-today/code/issues/${n}`,
    title: `Item ${n}`,
    state: "open",
    contentKind: "issue",
    status: null,
    area: null,
    horizon: null,
    kind: null,
    ...partial,
  };
}

describe("genealogy read model", () => {
  test("derives the verified parent, epic, roadmap, priority, release and blocker chain", () => {
    const pool = [
      item(1, {
        title: "Overall priority",
        kind: "Roadmap item",
        labels: ["kind/roadmap-item"],
        milestone: { title: "Release 1", dueOn: "2026-12-01", closedAt: null },
      }),
      item(2, {
        title: "Project roadmap",
        kind: "Roadmap item",
        labels: ["kind/roadmap-item"],
        parentRef: ref(1),
        blockedBy: [ref(9)],
      }),
      item(3, { title: "Design", kind: "Design", labels: ["kind/design", "design/crate-split"], parentRef: ref(2) }),
      item(4, { title: "Epic", kind: "Epic", labels: ["epic/crate-split"], parentRef: ref(3) }),
      item(5, { title: "Task", parentRef: ref(4), labels: ["design/crate-split"] }),
    ];
    const result = deriveGenealogy(pool[4]!, pool);
    expect(result.ancestors.map(({ ref: r, relation }) => [r, relation])).toEqual([
      [ref(4), "parent"],
      [ref(3), "parent"],
      [ref(2), "parent"],
      [ref(1), "parent"],
      [ref(4), "primary-parent"],
      [ref(3), "spawned-by"],
      [ref(2), "roadmap"],
      [ref(1), "priority"],
    ]);
    expect(result.release).toEqual({ title: "Release 1", dueOn: "2026-12-01", ref: ref(1) });
    expect(result.afterRefs).toEqual([ref(9)]);
    expect(result.sentence).toContain("Working on Task as part of epic Epic");
    expect(result.sentence).toContain("release Release 1 due 2026-12-01");
  });

  test("renders chain breaks honestly and enriches roadmap rows with known overlay refs", () => {
    const lone = item(7, { title: "Orphan" });
    const result = deriveGenealogy(lone, [lone]);
    expect(result.sentence).toContain("no epic yet");
    expect(result.sentence).toContain("no roadmap item yet");
    const row = {
      id: 1,
      subject: "grist:x/y/1",
      title: "Roadmap",
      area: "ID",
      status: "Now",
      horizon: "Now",
      source: "test",
      notes: "",
      links: [{ ref: ref(7), relation: "tracks" }],
    } satisfies RoadmapRow;
    expect(enrichRoadmapRows([row], [lone])[0]).toMatchObject({ priority: null, afterRefs: [], releaseRef: null });
  });

  test("reconstructs the design's crate-split worked chain from its real refs", () => {
    const pool = [
      item(2056, { title: "ID program", kind: "Epic", labels: ["epic/id-program"] }),
      item(3195, {
        title: "Crate split design",
        kind: "Design",
        labels: ["kind/design", "design/id-crate-split"],
        parentRef: ref(2056),
      }),
      item(3447, {
        title: "Crate split epic",
        kind: "Epic",
        labels: ["epic/id-crate-split", "design/id-crate-split"],
        parentRef: ref(3195),
      }),
      item(4403, { title: "First crate-split step", parentRef: ref(3447), labels: ["design/id-crate-split"] }),
    ];
    const result = deriveGenealogy(pool[3]!, pool);
    expect(result.ancestors.map((ancestor) => ancestor.ref)).toContain(ref(3195));
    expect(result.ancestors.map((ancestor) => ancestor.ref)).toContain(ref(3447));
    expect(result.ancestors.map((ancestor) => ancestor.ref)).toContain(ref(2056));
    expect(result.sentence).toContain("Crate split epic");
    expect(result.sentence).not.toContain("Formerly part of");
  });

  test("renders a historical split parent only when canonical metadata marks it historical", () => {
    const body = `<!-- pm-metadata:start -->\n\`\`\`json\n{"pm_meta":1,"links":[{"target":{"repo":"developing-today/code","number":99},"relation":"parent","status":"historical","since":"2026-11-02","until":"2026-11-12","reason":"split"},{"target":{"repo":"developing-today/code","number":98},"relation":"parent","status":"removed"}]}\n\`\`\`\n<!-- pm-metadata:end -->`;
    const refs = historicalParentsFromMetadata(body);
    expect(refs).toEqual([ref(99)]);
    const former = item(99, { title: "Former epic" });
    const current = item(100, { title: "Split task", historicalParentRefs: refs });
    expect(deriveGenealogy(current, [current, former]).sentence).toContain(
      "Formerly part of Former epic (developing-today/code#99)",
    );
  });
});
