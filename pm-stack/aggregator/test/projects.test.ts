import { describe, expect, test } from "bun:test";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import type { OrchestrationSection } from "../src/model.ts";
import { derivedProjectState, parseProjectItems, poolItemToGhItem, projectDisagreement } from "../src/projects.ts";
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
