import { describe, expect, test } from "bun:test";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { liveSource } from "../src/sources/live.ts";

// #14397 slice 1: a failed board fetch is an explicit unavailable state,
// not a silent fixture board. Fixtures masquerade as the team's real
// work; an empty board with the failure named in provenance tells the
// dashboard (and its readers) the truth: Kaneo could not be reached.

describe("live board: failed fetch is unavailable, not fixtures (#14397)", () => {
  test("board() returns [] with unavailable provenance when every Kaneo task read fails (503)", async () => {
    const dir = await mkdtemp(join(tmpdir(), "pm-board-unavail-"));
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
    const realFetch = globalThis.fetch;
    globalThis.fetch = (async (input: unknown) => {
      const url = String(input);
      const json = (body: unknown, headers: Record<string, string> = {}) =>
        new Response(JSON.stringify(body), {
          status: 200,
          headers: { "Content-Type": "application/json", ...headers },
        });
      if (url.includes("/api/auth/sign-in/email")) return json({}, { "set-cookie": "session=test; Path=/" });
      if (url.includes("/api/workspace")) return json([{ id: "ws1" }]);
      if (url.includes("/api/project?")) return json([{ id: "p1", slug: "code", name: "Code" }]);
      if (url.includes("/api/task/tasks/")) return new Response("unavailable", { status: 503 });
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
      const board = await src.board();
      // No fixture cards stand in for the unreachable board.
      expect(board).toEqual([]);
      // The provenance names the unavailable state and the cause.
      expect(src.provenance().board).toContain("unavailable");
      expect(src.provenance().board).toContain("kaneo task reads failed for every project");
      expect(src.provenance().board).not.toContain("fixtures");
    } finally {
      globalThis.fetch = realFetch;
    }
  });
});
