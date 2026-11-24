// #8570 Phase 3 acceptance: killing the projection module's intake must
// degrade board link data visibly (provenance chips say so) without taking
// the board down. The board enriches Kaneo cards with registry links from
// the projector; when that read fails the board must still serve the Kaneo
// cards while its provenance says the links are unavailable — never claim
// "+ projector links" for links it did not fetch.
//
// Run: bun test test/board-link-degradation-8570.test.ts
import { describe, expect, test } from "bun:test";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { liveSource } from "../src/sources/live.ts";
import { projectorLinks } from "../src/sources/live.ts";

const FEED = {
  version: 1,
  generatedAt: new Date().toISOString(),
  quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
  attention: [],
};

// Port 1 is never bound: reads against it fail with ECONNREFUSED through
// the real fetch, exercising the genuine fail-soft path (not a stubbed ok:false).
const DEAD_PROJECTOR = "http://127.0.0.1:1";

async function driveBoardWithDeadProjector() {
  const dir = await mkdtemp(join(tmpdir(), "pm-link-degraded-"));
  const feedFile = join(dir, "fleet-feed.json");
  await writeFile(feedFile, JSON.stringify(FEED));
  const projectsFile = join(dir, "kaneo-projects.json");
  await writeFile(
    projectsFile,
    JSON.stringify({ projects: [{ slug: "code", projectId: "p1", repository: "example-org/example-repo" }] }),
  );
  const realFetch = globalThis.fetch;
  globalThis.fetch = (async (input: unknown) => {
    const url = String(input);
    if (url.startsWith("http://127.0.0.1:1")) return realFetch(url);
    const json = (body: unknown, headers: Record<string, string> = {}) =>
      new Response(JSON.stringify(body), { status: 200, headers: { "Content-Type": "application/json", ...headers } });
    if (url.includes("/api/auth/sign-in/email")) return json({}, { "set-cookie": "session=test; Path=/" });
    if (url.includes("/api/workspace")) return json([{ id: "ws1" }]);
    if (url.includes("/api/project?")) return json([{ id: "p1", slug: "code", name: "Code" }]);
    if (url.includes("/api/task/tasks/"))
      return json({
        data: {
          columns: [
            {
              id: "in-progress",
              name: "In Progress",
              tasks: [
                {
                  id: "t1",
                  number: 7,
                  title: "Work with links",
                  status: "in-progress",
                  externalLinks: [
                    {
                      resourceType: "issue",
                      externalId: "42",
                      url: "https://github.com/example-org/example-repo/issues/42",
                    },
                  ],
                },
              ],
            },
          ],
        },
        pagination: { totalPages: 1 },
      });
    return new Response("not found", { status: 404 });
  }) as typeof fetch;
  try {
    const cfg = loadConfig({
      ...Bun.env,
      PM_AGG_SOURCE: "live",
      PM_AGG_FEED_FILE: feedFile,
      PM_PROJECTOR_URL: DEAD_PROJECTOR,
      PM_KANEO_PROJECTS_FILE: projectsFile,
      KANEO_URL: "http://kaneo.test",
      KANEO_ADMIN_EMAIL: "test@example.com",
      KANEO_ADMIN_PASSWORD: "test-password",
    });
    const src = liveSource(cfg);
    const board = await src.board();
    return { board, provenance: src.provenance().board };
  } finally {
    globalThis.fetch = realFetch;
  }
}

describe("board link degradation when the projector is unreachable (#8570)", () => {
  test("board still serves the live Kaneo cards with degraded link provenance", async () => {
    const { board, provenance } = await driveBoardWithDeadProjector();
    const kaneo = board.filter((b) => b.source === "kaneo");
    // The board is up: the dead projector took no card down with it.
    expect(kaneo.map((b) => b.title)).toEqual(["Work with links"]);
    // ... but the provenance chip is honest about the missing links.
    expect(provenance).toContain("live (Kaneo");
    expect(provenance).toContain("projector links unavailable");
    expect(provenance).not.toContain("+ projector links;");
  });

  test("projectorLinks reports a failed read as null, not an empty link list", async () => {
    // A null read (registry unreachable) must stay distinguishable from
    // a reachable registry that simply holds no links for the subject.
    expect(await projectorLinks(DEAD_PROJECTOR, "kaneo:board-item")).toBeNull();
  });

  test("projectorLinks still resolves a reachable registry's links", async () => {
    const realFetch = globalThis.fetch;
    globalThis.fetch = (async () =>
      new Response(JSON.stringify({ links: [{ ref: "example-org/example-repo#42", relation: "relates" }] }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      })) as unknown as typeof fetch;
    try {
      expect(await projectorLinks("http://projector", "kaneo:board-item")).toEqual([
        { ref: "example-org/example-repo#42", relation: "relates" },
      ]);
    } finally {
      globalThis.fetch = realFetch;
    }
  });
});
