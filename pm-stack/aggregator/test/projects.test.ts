import { describe, expect, test } from "bun:test";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import type { OrchestrationSection } from "../src/model.ts";
import { derivedProjectState, parseProjectItems, poolItemToGhItem, projectDisagreement, resolveEpicRefs } from "../src/projects.ts";
import { renderProjects } from "../src/render.ts";
import { liveSource } from "../src/sources/live.ts";

// A fixture GraphQL Projects v2 response, shaped exactly like the
// live read of org Project #4 (one Todo Task, one closed item, one
// draft-ish entry with no linked issue that must be skipped).
const GQL = {
  data: {
    node: {
      title: "Developing Today",
      number: 4,
      url: "https://github.com/orgs/developing-today/projects/4",
      items: {
        totalCount: 3,
        pageInfo: { hasNextPage: false, endCursor: null },
        nodes: [
          {
            id: "PVTI_1",
            databaseId: 111,
            type: "ISSUE",
            content: {
              number: 508,
              title: "EPIC: Basement cluster",
              state: "OPEN",
              url: "https://github.com/dezren39/nix/issues/508",
              repository: { nameWithOwner: "dezren39/nix" },
              updatedAt: "2026-10-20T12:05:58Z",
              author: { login: "dezren39" },
            },
            fieldValues: {
              nodes: [
                {},
                { name: "In Progress", field: { name: "Status" } },
                { name: "lab-cluster", field: { name: "Area" } },
                { name: "Quarter", field: { name: "Horizon" } },
                { name: "Epic", field: { name: "Kind" } },
              ],
            },
          },
          {
            id: "PVTI_2",
            databaseId: 222,
            type: "ISSUE",
            content: {
              number: 910,
              title: "Stage 5",
              state: "CLOSED",
              url: "https://github.com/developing-today/code/issues/910",
              repository: { nameWithOwner: "developing-today/code" },
              updatedAt: "2026-10-25T00:00:00Z",
            },
            fieldValues: { nodes: [{ name: "Done", field: { name: "Status" } }] },
          },
          {
            id: "PVTI_3",
            databaseId: 333,
            type: "DRAFT_ISSUE",
            content: { title: "unlinked draft" },
            fieldValues: { nodes: [] },
          },
        ],
      },
    },
  },
};

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
};

describe("collector projects parsing (GraphQL -> feed section)", () => {
  test("parses items with fields, ref, and both URLs; skips unlinked drafts", () => {
    const pool = parseProjectItems(GQL);
    expect(pool.length).toBe(2);
    const epic = pool[0];
    expect(epic.ref).toBe("dezren39/nix#508");
    expect(epic.state).toBe("open");
    expect(epic.status).toBe("In Progress");
    expect(epic.area).toBe("lab-cluster");
    expect(epic.horizon).toBe("Quarter");
    expect(epic.kind).toBe("Epic");
    expect(epic.issueUrl).toBe("https://github.com/dezren39/nix/issues/508");
    expect(epic.url).toBe("https://github.com/orgs/developing-today/projects/4?pane=issue&itemId=111");
    expect(pool[1].state).toBe("closed");
  });

  test("derived state + disagreement", () => {
    const pool = parseProjectItems(GQL);
    // Open, no orchestration presence -> Todo, which disagrees with In Progress.
    expect(derivedProjectState(pool[0], null)).toBe("Todo");
    expect(projectDisagreement(pool[0], null)).toEqual({ project: "In Progress", derived: "Todo" });
    // Closed -> Done, agrees with Project Done.
    expect(derivedProjectState(pool[1], null)).toBe("Done");
    expect(projectDisagreement(pool[1], null)).toBeNull();
    // Orchestration lined-up presence -> In Progress, agreeing.
    const section = {
      tiers: { needsDrew: [], needsOrchestrator: [] },
      goals: [
        {
          linedUp: [
            { title: "x", ref: "dezren39/nix#508", status: "in-progress", horizon: "today", horizonWhy: "test" },
          ],
        },
      ],
      dependencies: [],
    } as unknown as OrchestrationSection;
    expect(derivedProjectState(pool[0], section)).toBe("In Progress");
    expect(projectDisagreement(pool[0], section)).toBeNull();
  });
});

describe("items-from-pool resolution", () => {
  test("pool entry builds a GhItem carrying the Project fields", () => {
    const item = poolItemToGhItem(parseProjectItems(GQL)[0]);
    expect(item.ref).toBe("dezren39/nix#508");
    expect(item.kind).toBe("issue");
    expect(item.state).toBe("open");
    expect(item.projectStatus).toBe("In Progress");
    expect(item.projectKind).toBe("Epic");
  });

  test("liveSource.items serves pool refs from the feed and labels provenance live", async () => {
    const dir = await mkdtemp(join(tmpdir(), "pm-projects-"));
    const feedFile = join(dir, "fleet-feed.json");
    const pool = parseProjectItems(GQL);
    await writeFile(
      feedFile,
      JSON.stringify({
        version: 1,
        generatedAt: new Date().toISOString(),
        quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
        attention: [],
        projects: pool,
      }),
    );
    const cfg = loadConfig({
      PM_AGG_SOURCE: "live",
      PM_AGG_FEED_FILE: feedFile,
      PM_PROJECTOR_URL: "off",
    });
    const src = liveSource(cfg);
    const items = await src.items(["dezren39/nix#508", "developing-today/code#910"]);
    expect(items.get("dezren39/nix#508")?.title).toBe("EPIC: Basement cluster");
    expect(items.get("developing-today/code#910")?.state).toBe("closed");
    expect(src.provenance().items).toContain("live (project pool");
    const served = await src.projects();
    expect(served.length).toBe(2);
  });
});

// Fixture for the #2399 graph-edges contract: an epic, a child task
// with parent + blocked-by + blocks edges and effort/mode labels, a
// grandchild (two-level chain to the epic), an orphan whose parent is
// off-board, a PR (edges are issue-only), and an edgeless labeled
// issue (sized but ungraphed). Drafts still drop.
const GQL_EDGES = {
  data: {
    node: {
      title: "Developing Today",
      number: 4,
      url: "https://github.com/orgs/developing-today/projects/4",
      items: {
        totalCount: 6,
        pageInfo: { hasNextPage: false, endCursor: null },
        nodes: [
          {
            id: "PVTI_10",
            databaseId: 1010,
            type: "ISSUE",
            content: {
              number: 2401,
              title: "EPIC: edges test",
              state: "OPEN",
              url: "https://github.com/developing-today/code/issues/2401",
              repository: { nameWithOwner: "developing-today/code" },
              updatedAt: "2026-10-29T00:00:00Z",
              author: { login: "dezren39" },
              labels: { nodes: [{ name: "effort/l" }] },
              parent: null,
              blockedBy: { nodes: [] },
              blocking: { nodes: [] },
            },
            fieldValues: { nodes: [{ name: "Epic", field: { name: "Kind" } }] },
          },
          {
            id: "PVTI_11",
            databaseId: 1011,
            type: "ISSUE",
            content: {
              number: 2402,
              title: "child task",
              state: "OPEN",
              url: "https://github.com/developing-today/code/issues/2402",
              repository: { nameWithOwner: "developing-today/code" },
              updatedAt: "2026-10-29T00:00:00Z",
              author: { login: "dezren39" },
              labels: { nodes: [{ name: "effort/s" }, { name: "mode/async" }] },
              parent: {
                number: 2401,
                url: "https://github.com/developing-today/code/issues/2401",
                repository: { nameWithOwner: "developing-today/code" },
              },
              blockedBy: {
                nodes: [
                  {
                    number: 910,
                    url: "https://github.com/developing-today/code/issues/910",
                    repository: { nameWithOwner: "developing-today/code" },
                  },
                  { number: null, repository: { nameWithOwner: "developing-today/code" } },
                ],
              },
              blocking: {
                nodes: [
                  {
                    number: 2403,
                    url: "https://github.com/developing-today/code/issues/2403",
                    repository: { nameWithOwner: "developing-today/code" },
                  },
                ],
              },
            },
            fieldValues: { nodes: [{ name: "Task", field: { name: "Kind" } }] },
          },
          {
            id: "PVTI_12",
            databaseId: 1012,
            type: "ISSUE",
            content: {
              number: 2403,
              title: "grandchild task",
              state: "OPEN",
              url: "https://github.com/developing-today/code/issues/2403",
              repository: { nameWithOwner: "developing-today/code" },
              updatedAt: "2026-10-29T00:00:00Z",
              parent: {
                number: 2402,
                url: "https://github.com/developing-today/code/issues/2402",
                repository: { nameWithOwner: "developing-today/code" },
              },
            },
            fieldValues: { nodes: [{ name: "Task", field: { name: "Kind" } }] },
          },
          {
            id: "PVTI_13",
            databaseId: 1013,
            type: "ISSUE",
            content: {
              number: 2405,
              title: "orphan task",
              state: "OPEN",
              url: "https://github.com/developing-today/code/issues/2405",
              repository: { nameWithOwner: "developing-today/code" },
              updatedAt: "2026-10-29T00:00:00Z",
              parent: {
                number: 1,
                url: "https://github.com/developing-today/opencode/issues/1",
                repository: { nameWithOwner: "developing-today/opencode" },
              },
            },
            fieldValues: { nodes: [{ name: "Task", field: { name: "Kind" } }] },
          },
          {
            id: "PVTI_14",
            databaseId: 1014,
            type: "PULL_REQUEST",
            content: {
              number: 2406,
              title: "a pr",
              state: "OPEN",
              url: "https://github.com/developing-today/code/pull/2406",
              repository: { nameWithOwner: "developing-today/code" },
              updatedAt: "2026-10-29T00:00:00Z",
              author: { login: "dezren39" },
              isDraft: false,
            },
            fieldValues: { nodes: [] },
          },
          {
            id: "PVTI_15",
            databaseId: 1015,
            type: "ISSUE",
            content: {
              number: 2407,
              title: "sized but ungraphed",
              state: "OPEN",
              url: "https://github.com/developing-today/code/issues/2407",
              repository: { nameWithOwner: "developing-today/code" },
              updatedAt: "2026-10-29T00:00:00Z",
              labels: { nodes: [{ name: "effort/m" }] },
            },
            fieldValues: { nodes: [{ name: "Task", field: { name: "Kind" } }] },
          },
        ],
      },
    },
  },
};

describe("collector graph edges (#2399 contract)", () => {
  test("parses parent, blocked-by, and blocks refs; drops incomplete edge nodes", () => {
    const pool = parseProjectItems(GQL_EDGES);
    expect(pool.length).toBe(6);
    const child = pool.find((p) => p.ref === "developing-today/code#2402")!;
    expect(child.parentRef).toBe("developing-today/code#2401");
    // The numberless blocked-by node drops; the good one stays.
    expect(child.blockedBy).toEqual(["developing-today/code#910"]);
    expect(child.blocks).toEqual(["developing-today/code#2403"]);
    expect(child.labels).toEqual(["effort/s", "mode/async"]);
  });

  test("epic, PR, and edgeless shapes carry no edges from the parse", () => {
    const pool = parseProjectItems(GQL_EDGES);
    const epic = pool.find((p) => p.ref === "developing-today/code#2401")!;
    expect(epic.parentRef).toBeUndefined();
    expect(epic.blockedBy).toBeUndefined();
    expect(epic.blocks).toBeUndefined();
    const pr = pool.find((p) => p.ref === "developing-today/code#2406")!;
    expect(pr.contentKind).toBe("pr");
    expect(pr.parentRef).toBeUndefined();
    expect(pr.blockedBy).toBeUndefined();
    expect(pr.blocks).toBeUndefined();
    const plain = pool.find((p) => p.ref === "developing-today/code#2407")!;
    expect(plain.parentRef).toBeUndefined();
    expect(plain.labels).toEqual(["effort/m"]);
  });

  test("resolveEpicRefs names the nearest epic ancestor, chains included", () => {
    const resolved = resolveEpicRefs(parseProjectItems(GQL_EDGES));
    const byRef = new Map(resolved.map((p) => [p.ref, p] as const));
    // Direct child of the epic.
    expect(byRef.get("developing-today/code#2402")?.epicRef).toBe("developing-today/code#2401");
    // Grandchild walks the two-level chain to the same epic.
    expect(byRef.get("developing-today/code#2403")?.epicRef).toBe("developing-today/code#2401");
    // The epic itself, the off-board orphan, the PR, and the plain
    // issue carry no epic membership.
    expect(byRef.get("developing-today/code#2401")?.epicRef).toBeUndefined();
    expect(byRef.get("developing-today/code#2405")?.epicRef).toBeUndefined();
    expect(byRef.get("developing-today/code#2406")?.epicRef).toBeUndefined();
    expect(byRef.get("developing-today/code#2407")?.epicRef).toBeUndefined();
    // The orphan keeps its (off-board) parent ref: absence of an epic
    // is not absence of the edge.
    expect(byRef.get("developing-today/code#2405")?.parentRef).toBe("developing-today/opencode#1");
  });

  test("resolveEpicRefs is cycle-safe and never invents membership", () => {
    const loop = resolveEpicRefs([
      {
        id: "A",
        url: "u",
        projectUrl: "p",
        ref: "o/r#1",
        repo: "o/r",
        number: 1,
        issueUrl: "i",
        title: "a",
        state: "open",
        contentKind: "issue",
        status: null,
        area: null,
        horizon: null,
        kind: "Task",
        parentRef: "o/r#2",
      },
      {
        id: "B",
        url: "u",
        projectUrl: "p",
        ref: "o/r#2",
        repo: "o/r",
        number: 2,
        issueUrl: "i",
        title: "b",
        state: "open",
        contentKind: "issue",
        status: null,
        area: null,
        horizon: null,
        kind: "Task",
        parentRef: "o/r#1",
      },
    ]);
    expect(loop[0]?.epicRef).toBeUndefined();
    expect(loop[1]?.epicRef).toBeUndefined();
  });
});

describe("renderProjects", () => {
  test("renders cards, both links, filter, and the disagreement badge", () => {
    const pool = parseProjectItems(GQL);
    const html = renderProjects(pool, PROV, {});
    expect(html).toContain("EPIC: Basement cluster");
    expect(html).toContain("dezren39/nix#508");
    expect(html).toContain("itemId=111");
    expect(html).toContain("⚠ project: In Progress · derived: Todo");
    expect(html).toContain("/projects?area=lab-cluster");
    const filtered = renderProjects(pool, PROV, { area: "lab-cluster" });
    expect(filtered).toContain("EPIC: Basement cluster");
    expect(filtered).not.toContain("Stage 5");
  });

  test("shows the (unset) bucket for Area-unset items and filters to it (#1143)", () => {
    const pool = parseProjectItems(GQL);
    // The fixture's Stage 5 item carries no Area; the EPIC carries lab-cluster.
    const html = renderProjects(pool, PROV, {});
    expect(html).toContain("(unset) (1)");
    expect(html).toContain(`/projects?area=${encodeURIComponent("(unset)")}`);
    expect(html).toContain("carry no Area");
    const unsetOnly = renderProjects(pool, PROV, { area: "(unset)" });
    expect(unsetOnly).toContain("Stage 5");
    expect(unsetOnly).not.toContain("EPIC: Basement cluster");
    // No unset items -> no empty unset bucket or banner.
    const noUnset = renderProjects([pool[0]], PROV, {});
    expect(noUnset).not.toContain("(unset)");
  });
});
