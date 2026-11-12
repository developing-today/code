import { describe, expect, test } from "bun:test";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { liveSource } from "../src/sources/live.ts";

// #1201: the Kaneo reader must use pm-stack/kaneo-projects.json as the
// authoritative project-repo map. Unknown slugs stay unmapped (repo "")
// instead of guessing `developing-today/<slug>`, and non-GitHub external
// URLs never produce invented repo-based refs — while GitHub-URL links
// and the Kaneo card link keep working.

const FEED = {
  version: 1,
  generatedAt: new Date().toISOString(),
  quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
  attention: [],
};

async function driveBoard(registry: unknown, projects: { id: string; slug: string; name: string }[]) {
  const dir = await mkdtemp(join(tmpdir(), "pm-kaneo-repos-"));
  const feedFile = join(dir, "fleet-feed.json");
  await writeFile(feedFile, JSON.stringify(FEED));
  const projectsFile = join(dir, "kaneo-projects.json");
  await writeFile(projectsFile, JSON.stringify(registry));
  const realFetch = globalThis.fetch;
  globalThis.fetch = (async (input: unknown) => {
    const url = String(input);
    const json = (body: unknown, headers: Record<string, string> = {}) =>
      new Response(JSON.stringify(body), { status: 200, headers: { "Content-Type": "application/json", ...headers } });
    if (url.includes("/api/auth/sign-in/email")) return json({}, { "set-cookie": "session=test; Path=/" });
    if (url.includes("/api/workspace")) return json([{ id: "ws1" }]);
    if (url.includes("/api/project?")) return json(projects);
    if (url.includes("/api/task/tasks/")) {
      const pid = url.split("/api/task/tasks/")[1]!.split("?")[0];
      return json({
        data: {
          columns: [
            {
              id: "in-progress",
              name: "In Progress",
              tasks: [
                {
                  id: `t-${pid}`,
                  number: 7,
                  title: `Work in ${pid}`,
                  status: "in-progress",
                  externalLinks: [
                    { resourceType: "issue", externalId: "42", url: "https://github.com/example-org/example-repo/issues/42" },
                    { resourceType: "issue", externalId: "9", url: "https://linear.app/team/issue/XYZ-9" },
                  ],
                },
              ],
            },
          ],
        },
        pagination: { totalPages: 1 },
      });
    }
    return new Response("not found", { status: 404 });
  }) as typeof fetch;
  try {
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
    return (await liveSource(cfg).board()).filter((b) => b.source === "kaneo");
  } finally {
    globalThis.fetch = realFetch;
  }
}

describe("live board: Kaneo project-repo registry is authoritative (#1201)", () => {
  test("mapped slug uses the registry repo; unknown slug stays unmapped", async () => {
    const board = await driveBoard(
      { projects: [{ slug: "code", projectId: "p1", repository: "example-org/example-repo", installationId: 1 }] },
      [
        { id: "p1", slug: "code", name: "Code" },
        { id: "pX", slug: "mystery", name: "Mystery" },
      ],
    );
    const known = board.find((b) => b.title === "Work in p1");
    const unknown = board.find((b) => b.title === "Work in pX");
    expect(known?.repo).toBe("example-org/example-repo");
    // Unmapped — never a guessed `developing-today/mystery`.
    expect(unknown?.repo).toBe("");
    expect(unknown?.repo).not.toContain("mystery");
  });

  test("GitHub-URL links survive; non-GitHub URLs invent no refs; card link intact", async () => {
    const board = await driveBoard(
      { projects: [{ slug: "code", projectId: "p1", repository: "example-org/example-repo", installationId: 1 }] },
      [{ id: "p1", slug: "code", name: "Code" }],
    );
    const card = board.find((b) => b.title === "Work in p1");
    expect(card).toBeDefined();
    // The GitHub-URL external link keeps its exact ref.
    expect(card?.links.map((l) => l.ref)).toContain("example-org/example-repo#42");
    // The Linear URL produces no invented `example-org/example-repo#9` ref.
    expect(card?.links.map((l) => l.ref)).not.toContain("example-org/example-repo#9");
    expect(card?.links).toHaveLength(1);
    // The Kaneo card deep link is untouched.
    expect(card?.kaneoUrl).toContain("kaneo");
  });

  test("unreadable registry degrades to unmapped, not guessed", async () => {
    const dir = await mkdtemp(join(tmpdir(), "pm-kaneo-repos-"));
    const feedFile = join(dir, "fleet-feed.json");
    await writeFile(feedFile, JSON.stringify(FEED));
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
          data: { columns: [{ id: "in-progress", name: "In Progress", tasks: [{ id: "t1", number: 1, title: "T", status: "in-progress" }] }] },
          pagination: { totalPages: 1 },
        });
      return new Response("not found", { status: 404 });
    }) as typeof fetch;
    try {
      const cfg = loadConfig({
        ...Bun.env,
        PM_AGG_SOURCE: "live",
        PM_AGG_FEED_FILE: feedFile,
        PM_PROJECTOR_URL: "off",
        PM_KANEO_PROJECTS_FILE: join(dir, "does-not-exist.json"),
        KANEO_URL: "http://kaneo.test",
        KANEO_ADMIN_EMAIL: "test@example.com",
        KANEO_ADMIN_PASSWORD: "test-password",
      });
      const board = (await liveSource(cfg).board()).filter((b) => b.source === "kaneo");
      expect(board[0]?.repo).toBe("");
    } finally {
      globalThis.fetch = realFetch;
    }
  });
});
