import { describe, expect, test } from "bun:test";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { liveSource } from "../src/sources/live.ts";

// #1197: the Kaneo reader must never present a task's creation time as
// its update time. When Kaneo omits updatedAt, the board item is
// undated (updatedAt absent) so downstream freshness sorts and the
// done policy treat its age as unproven instead of consuming a
// fabricated date; when Kaneo sends a distinct updatedAt, the board
// item carries exactly that value.

describe("live board: Kaneo updatedAt honesty (#1197)", () => {
  test("board() passes through the API update time and never substitutes creation time", async () => {
    const dir = await mkdtemp(join(tmpdir(), "pm-kaneo-updated-"));
    const feedFile = join(dir, "fleet-feed.json");
    await writeFile(
      feedFile,
      JSON.stringify({
        version: 1,
        generatedAt: new Date().toISOString(),
        quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
        attention: [],
      }),
    );
    const created = "2026-10-01T09:00:00.000Z";
    const updated = "2026-10-20T15:30:00.000Z";
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
                  { id: "t-dated", number: 1, title: "Dated work", status: "in-progress", createdAt: created, updatedAt: updated },
                  { id: "t-undated", number: 2, title: "Undated work", status: "in-progress", createdAt: created },
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
        PM_PROJECTOR_URL: "off",
        KANEO_URL: "http://kaneo.test",
        KANEO_ADMIN_EMAIL: "test@example.com",
        KANEO_ADMIN_PASSWORD: "test-password",
      });
      const src = liveSource(cfg);
      const board = (await src.board()).filter((b) => b.source === "kaneo");
      const dated = board.find((b) => b.title === "Dated work");
      const undated = board.find((b) => b.title === "Undated work");
      // The API's own update time, verbatim — not the creation time.
      expect(dated?.updatedAt).toBe(updated);
      expect(dated?.updatedAt).not.toBe(created);
      // No update time from Kaneo means no update time on the board:
      // never the creation time dressed up as freshness.
      expect(undated).toBeDefined();
      expect(undated?.updatedAt).toBeUndefined();
    } finally {
      globalThis.fetch = realFetch;
    }
  });
});
