// #8570 Phase 3: projector read seam. The live source must read
// projections through an injectable ProjectorReader — the seam the
// in-process projector module will plug — instead of reaching
// PM_PROJECTOR_URL HTTP directly. An injected reader serves the board,
// items and comments with the same provenance contracts as the HTTP
// client, while the HTTP factory keeps the exact fail-soft behaviour
// (null = read failed) the degradation test pins down.
//
// Run: bun test test/projector-reader-8570.test.ts
import { describe, expect, test } from "bun:test";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { liveSource } from "../src/sources/live.ts";
import {
  createHttpProjectorReader,
  type ProjectorReader,
} from "../src/sources/projector.ts";

const FEED = {
  version: 1,
  generatedAt: new Date().toISOString(),
  quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
  attention: [],
};

// Port 1 is never bound: reads against it fail with ECONNREFUSED through
// the real fetch, exercising the genuine fail-soft path (not a stubbed ok:false).
const DEAD_PROJECTOR = "http://127.0.0.1:1";

const REGISTRY_LINK = { ref: "example-org/example-repo#42", relation: "relates" as const };

/** In-memory stand-in for the future in-process projector module. */
function fakeReader(): ProjectorReader {
  return {
    items: async (refs) =>
      new Map(
        refs.map((ref) => [
          ref,
          {
            ref,
            kind: "issue" as const,
            url: `https://github.com/${ref.replace("#", "/issues/")}`,
            title: "Reader-served item",
            state: "open" as const,
            draft: false,
            labels: [],
            author: "reader",
            updatedAt: new Date().toISOString(),
          },
        ]),
      ),
    comments: async (refs) =>
      refs.map((ref) => ({
        id: `${ref}:c1`,
        ref,
        kind: "issue_comment" as const,
        author: "reader",
        body: "reader-served comment",
        createdAt: new Date().toISOString(),
        url: `https://github.com/${ref.replace("#", "/issues/")}#c1`,
      })),
    latestComments: async (refs) =>
      refs.map((ref) => ({
        id: `${ref}:c1`,
        ref,
        kind: "issue_comment" as const,
        author: "reader",
        body: "reader-served comment",
        createdAt: new Date().toISOString(),
        url: `https://github.com/${ref.replace("#", "/issues/")}#c1`,
      })),
    links: async () => [REGISTRY_LINK],
  };
}

async function driveBoardWithReader() {
  const dir = await mkdtemp(join(tmpdir(), "pm-reader-seam-"));
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
    // PM_PROJECTOR_URL=off: no HTTP projector is configured, so any
    // registry data on the board must have come through the injected
    // reader — HTTP cannot be the source.
    const cfg = loadConfig({
      ...Bun.env,
      PM_AGG_SOURCE: "live",
      PM_AGG_FEED_FILE: feedFile,
      PM_PROJECTOR_URL: "off",
      PM_KANEO_PROJECTS_FILE: projectsFile,
      KANEO_URL: "http://kaneo.test",
      KANEO_ADMIN_EMAIL: "test@example.com",
      KANEO_ADMIN_PASSWORD: "test-password",
    });
    const src = liveSource(cfg, { projector: fakeReader() });
    const board = await src.board();
    return { src, board, provenance: src.provenance().board };
  } finally {
    globalThis.fetch = realFetch;
  }
}

describe("projector reader seam (#8570)", () => {
  test("an injected reader enriches the board with registry links and honest provenance", async () => {
    const { board, provenance } = await driveBoardWithReader();
    const kaneo = board.filter((b) => b.source === "kaneo");
    expect(kaneo.map((b) => b.title)).toEqual(["Work with links"]);
    const card = kaneo[0];
    expect(card.links.map((l) => l.ref)).toContain(REGISTRY_LINK.ref);
    expect(provenance).toContain("+ projector links");
    expect(provenance).not.toContain("unavailable");
  });

  test("an injected reader resolves items outside the pool as projector-served", async () => {
    const dir = await mkdtemp(join(tmpdir(), "pm-reader-seam-"));
    const feedFile = join(dir, "fleet-feed.json");
    await writeFile(feedFile, JSON.stringify(FEED));
    const cfg = loadConfig({ ...Bun.env, PM_AGG_SOURCE: "live", PM_AGG_FEED_FILE: feedFile, PM_PROJECTOR_URL: "off" });
    const src = liveSource(cfg, { projector: fakeReader() });
    const items = await src.items(["example-org/example-repo#42"]);
    expect(items.get("example-org/example-repo#42")?.resolution).toBe("projector");
    expect(src.provenance().items).toBe("live (projector)");
  });

  test("the HTTP factory keeps the fail-soft contract: a dead projector reads as null", async () => {
    const reader = createHttpProjectorReader(DEAD_PROJECTOR);
    await expect(reader.links("kaneo:board-item")).resolves.toBeNull();
    await expect(reader.items(["example-org/example-repo#42"])).resolves.toBeNull();
    await expect(reader.comments(["example-org/example-repo#42"])).resolves.toBeNull();
    await expect(reader.latestComments(["example-org/example-repo#42"])).resolves.toBeNull();
  });
});
