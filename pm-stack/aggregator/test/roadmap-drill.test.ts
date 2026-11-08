import { describe, expect, test } from "bun:test";
import type { ProjectPoolItem, RoadmapRow } from "../src/model.ts";
import { buildRoadmapDrill, renderRoadmap } from "../src/render.ts";
import { buildEpicRollups } from "../src/epics.ts";
import { reconcileRoadmap, unavailableRoadmapReconciliation } from "../src/roadmap-reconcile.ts";

// #1168: /roadmap drill-down — a roadmap row expands via <details> to
// its matched epic, the epic's pool children with states, a done/total
// progress bar, and the row's Grist notes.

const row = (id: number, title: string, refs: string[] = [], notes = ""): RoadmapRow => ({
  id,
  subject: `grist:doc/roadmap/${id}`,
  title,
  area: "id",
  status: "In Progress",
  horizon: "Quarter",
  source: "grist",
  notes,
  links: refs.map((ref) => ({ ref: ref as RoadmapRow["links"][number]["ref"], relation: "tracks" as const })),
});

const poolItem = (
  ref: string,
  title: string,
  opts: { kind?: string | null; state?: ProjectPoolItem["state"]; epicRef?: string; status?: string } = {},
): ProjectPoolItem => ({
  id: `PVTI_${ref}`,
  url: "https://github.com/orgs/developing-today/projects/4",
  projectUrl: "https://github.com/orgs/developing-today/projects/4",
  ref: ref as ProjectPoolItem["ref"],
  repo: ref.split("#")[0],
  number: Number(ref.split("#")[1]),
  issueUrl: `https://github.com/${ref.replace("#", "/issues/")}`,
  title,
  state: opts.state ?? "open",
  contentKind: "issue",
  status: opts.status ?? "Todo",
  area: "id",
  horizon: "Quarter",
  kind: opts.kind === undefined ? "Task" : opts.kind,
  ...(opts.epicRef ? { epicRef: opts.epicRef as ProjectPoolItem["epicRef"] } : {}),
});

const PROV = {
  board: "x",
  roadmap: "x",
  items: "x",
  comments: "x",
  quota: "x",
  usage: "x",
  attention: "x",
  orchestration: "x",
  dns: "x",
  registry: "x",
  aa: "x",
} as Parameters<typeof renderRoadmap>[2];

const render = (rows: RoadmapRow[], pool: ProjectPoolItem[]) => {
  const rec = reconcileRoadmap(rows, pool);
  const drill = buildRoadmapDrill(rows, rec, pool);
  return renderRoadmap(rows, new Map(), PROV, "quarter", undefined, undefined, rec, drill);
};

describe("roadmap drill-down (#1168)", () => {
  test("matched row expands to its epic, children with states, and progress", () => {
    const rows = [row(1, "Identity epic", ["a/b#10"], "watch the token TTL")];
    const pool = [
      poolItem("a/b#10", "Identity epic", { kind: "Epic" }),
      poolItem("a/b#11", "Bound registrations", { state: "closed", epicRef: "a/b#10" }),
      poolItem("a/b#12", "Refresh at capacity", { state: "open", epicRef: "a/b#10" }),
      poolItem("a/b#13", "Unrelated child of another epic", { state: "closed", epicRef: "a/b#99" }),
    ];
    const html = render(rows, pool);
    expect(html).toContain("<details");
    expect(html).toContain("Identity epic</a>");
    expect(html).toContain("Bound registrations");
    expect(html).toContain("Refresh at capacity");
    expect(html).not.toContain("Unrelated child of another epic");
    expect(html).toContain("1/2 child issues done");
    expect(html).toContain('<progress value="1" max="2">');
    expect(html).toContain("watch the token TTL");
  });

  test("unmatched row says 'no epic yet' and shows its notes", () => {
    const rows = [row(2, "Grist only row", [], "a note from Grist")];
    const html = render(rows, []);
    expect(html).toContain("<details");
    expect(html).toContain("No epic yet");
    expect(html).toContain("a note from Grist");
    expect(html).not.toContain("<progress");
  });

  test("epic with zero children says so instead of a 0/0 progress bar", () => {
    const rows = [row(3, "Fresh epic", ["a/b#20"])];
    const pool = [poolItem("a/b#20", "Fresh epic", { kind: "Epic" })];
    const html = render(rows, pool);
    expect(html).toContain("No child issues in the Project pool for this epic yet.");
    expect(html).not.toContain("<progress");
    expect(html).not.toContain("0/0");
  });

  test("no drill index renders no expansion (unavailable reconciliation stays honest)", () => {
    const rows = [row(4, "Any row")];
    const rec = unavailableRoadmapReconciliation("pool read failed");
    const html = renderRoadmap(rows, new Map(), PROV, "quarter", undefined, undefined, rec);
    expect(html).not.toContain("roadmap-drill");
    expect(html).not.toContain("No epic yet");
  });

  test("buildRoadmapDrill keys every row by subject, matched or not", () => {
    const rows = [row(5, "Matched", ["a/b#30"]), row(6, "Unmatched")];
    const pool = [poolItem("a/b#30", "Matched", { kind: "Epic" })];
    const drill = buildRoadmapDrill(rows, reconcileRoadmap(rows, pool), pool);
    expect(drill.get("grist:doc/roadmap/5")?.epic?.ref).toBe("a/b#30");
    expect(drill.get("grist:doc/roadmap/6")?.epic).toBeUndefined();
    expect(drill.get("grist:doc/roadmap/6")?.children).toEqual([]);
  });
  test("status-Done/open child counts as done, matching the epic rollup rule (#3954)", () => {
    const rows = [row(7, "Identity epic", ["a/b#40"])];
    const pool = [
      poolItem("a/b#40", "Identity epic", { kind: "Epic" }),
      poolItem("a/b#41", "Closed child", { state: "closed", epicRef: "a/b#40" }),
      poolItem("a/b#42", "Status-done but still open", { state: "open", status: "Done", epicRef: "a/b#40" }),
      poolItem("a/b#43", "Merged child", { state: "merged", epicRef: "a/b#40" }),
      poolItem("a/b#44", "Open child", { state: "open", epicRef: "a/b#40" }),
    ];
    const html = render(rows, pool);
    expect(html).toContain("3/4 child issues done");
    expect(html).toContain('<progress value="3" max="4">');
    // Cross-surface: the shared rollup rule counts the same pool identically.
    expect(buildEpicRollups(pool).get("a/b#40" as ProjectPoolItem["ref"])).toMatchObject({ total: 4, done: 3 });
  });
});
