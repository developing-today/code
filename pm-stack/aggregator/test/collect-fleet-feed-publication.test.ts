import { createHash } from "node:crypto";
import { open, mkdtemp, readFile, readdir, rm, rename, unlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { FreshnessStore } from "../src/freshness.ts";
import {
  publishFleetFeed,
  publishFleetFeedWithInvalidation,
  type FleetFeedPublicationFileOps,
} from "../scripts/collect-fleet-feed.ts";

describe("publishFleetFeed", () => {
  let directory: string;

  beforeEach(async () => {
    directory = await mkdtemp(join(tmpdir(), "fleet-feed-publication-"));
  });

  afterEach(async () => {
    await rm(directory, { recursive: true, force: true });
  });

  test("replaces the published file atomically and returns content metadata", async () => {
    const outputPath = join(directory, "fleet-feed.json");
    await writeFile(outputPath, "previous bytes", "utf8");
    const contents = "new feed bytes";

    const publication = await publishFleetFeed(outputPath, contents);

    expect(await readFile(outputPath, "utf8")).toBe(contents);
    expect(publication).toEqual({
      path: outputPath,
      sha256: createHash("sha256").update(contents, "utf8").digest("hex"),
      sizeBytes: Buffer.byteLength(contents, "utf8"),
    });
    expect(await readdir(directory)).toEqual(["fleet-feed.json"]);
  });

  test("submits a payload-free invalidation only after the complete feed is visible", async () => {
    const outputPath = join(directory, "fleet-feed.json");
    await writeFile(outputPath, "previous bytes", "utf8");
    const contents = "complete new feed";
    const generatedAt = "2026-11-04T21:50:00.000Z";
    const notices: unknown[] = [];

    const publication = await publishFleetFeedWithInvalidation(outputPath, contents, generatedAt, async (notice) => {
      expect(await readFile(outputPath, "utf8")).toBe(contents);
      notices.push(notice);
    });

    expect(publication.sha256).toBe(createHash("sha256").update(contents, "utf8").digest("hex"));
    expect(notices).toEqual([
      {
        eventId: `fleet-feed:${generatedAt}:${publication.sha256}`,
        source: "fleet-feed",
        scope: ["feed"],
        occurredAt: generatedAt,
        reason: "source-event",
      },
    ]);
  });

  test("retries a feed notice with the same generation/hash identity exactly once", async () => {
    const outputPath = join(directory, "fleet-feed.json");
    const store = new FreshnessStore(join(directory, "freshness.json"));
    const contents = "same complete feed";
    const generatedAt = "2026-11-04T21:50:00.000Z";
    const outcomes: Array<{ duplicate: boolean; pending: boolean }> = [];
    const notify = async (notice: Parameters<typeof store.publish>[0]) => {
      const result = await store.publish(notice);
      outcomes.push({ duplicate: result.duplicate, pending: result.pending });
    };

    await publishFleetFeedWithInvalidation(outputPath, contents, generatedAt, notify);
    await publishFleetFeedWithInvalidation(outputPath, contents, generatedAt, notify);

    expect(outcomes).toEqual([
      { duplicate: false, pending: false },
      { duplicate: true, pending: false },
    ]);
    const replay = await store.listAfter(0);
    expect(replay.events).toHaveLength(1);
    expect(replay.events[0]?.eventId).toContain(`fleet-feed:${generatedAt}:`);
  });

  test("a write failure preserves the previous bytes and removes the partial temp file", async () => {
    const outputPath = join(directory, "fleet-feed.json");
    await writeFile(outputPath, "previous bytes", "utf8");
    const failure = new Error("simulated write failure");
    const fileOps: FleetFeedPublicationFileOps = {
      async openTemp(path) {
        const file = await open(path, "wx");
        return {
          async writeFile(_contents: string) {
            await file.writeFile("partial", { encoding: "utf8" });
            throw failure;
          },
          close: () => file.close(),
        };
      },
      rename,
      unlink,
    };

    await expect(publishFleetFeed(outputPath, "new bytes", fileOps)).rejects.toBe(failure);

    expect(await readFile(outputPath, "utf8")).toBe("previous bytes");
    expect(await readdir(directory)).toEqual(["fleet-feed.json"]);
  });

  test("a rename failure preserves the previous bytes and removes the completed temp file", async () => {
    const outputPath = join(directory, "fleet-feed.json");
    await writeFile(outputPath, "previous bytes", "utf8");
    const failure = new Error("simulated rename failure");
    const fileOps: FleetFeedPublicationFileOps = {
      async openTemp(path) {
        const file = await open(path, "wx");
        return {
          writeFile: (contents: string) => file.writeFile(contents, { encoding: "utf8" }),
          close: () => file.close(),
        };
      },
      async rename() {
        throw failure;
      },
      unlink,
    };

    await expect(publishFleetFeed(outputPath, "new bytes", fileOps)).rejects.toBe(failure);

    expect(await readFile(outputPath, "utf8")).toBe("previous bytes");
    expect(await readdir(directory)).toEqual(["fleet-feed.json"]);
  });
});
