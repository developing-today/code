import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { FreshnessStore } from "../src/freshness.ts";
import { createFreshnessIpcHandler } from "../src/freshness-ipc.ts";

describe("loopback freshness publisher handler", () => {
  let directory: string;
  let handle: (request: Request) => Promise<Response>;

  beforeEach(async () => {
    directory = await mkdtemp(join(tmpdir(), "freshness-ipc-"));
    handle = createFreshnessIpcHandler(new FreshnessStore(join(directory, "freshness.json")));
  });

  afterEach(async () => rm(directory, { recursive: true, force: true }));

  test("accepts only a bounded POST and returns the durable journal result", async () => {
    const input = {
      eventId: "github:delivery-1",
      source: "github",
      scope: ["issue:developing-today/code#3977"],
      occurredAt: "2026-11-04T21:00:00.000Z",
      reason: "source-event",
    };
    const response = await handle(
      new Request("http://127.0.0.1:4871/publish", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(input),
      }),
    );

    expect(response.status).toBe(202);
    expect(await response.json()).toMatchObject({
      duplicate: false,
      pending: false,
      notice: { schemaVersion: 1, sequence: 1, ...input },
    });
    expect(await readFile(join(directory, "freshness.json"), "utf8")).not.toContain("github payload");
  });

  test("rejects other routes, methods, and oversized request bodies", async () => {
    expect((await handle(new Request("http://127.0.0.1/not-publish", { method: "POST", body: "{}" }))).status).toBe(404);
    expect((await handle(new Request("http://127.0.0.1/publish"))).status).toBe(405);
    expect(
      (
        await handle(
          new Request("http://127.0.0.1/publish", {
            method: "POST",
            body: "x".repeat(16 * 1024 + 1),
          }),
        )
      ).status,
    ).toBe(413);
  });
});
