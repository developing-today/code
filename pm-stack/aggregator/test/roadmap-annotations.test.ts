import { describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  RoadmapAnnotationsStore,
  validateNewRoadmapAnnotation,
  type NewRoadmapAnnotation,
} from "../src/roadmap-annotations.ts";

const author = { kind: "drew" as const, id: "drew", label: "Drew" };
const input = (objectRef: string, date: string, body: string): NewRoadmapAnnotation => ({
  objectRef,
  kind: "decision",
  date,
  author,
  body,
});

describe("roadmap annotations (#1284)", () => {
  test("validates typed, dated, attributed notes and rejects invalid calendar dates", () => {
    expect(validateNewRoadmapAnnotation(input("grist:doc/Roadmap/1", "2026-11-01", "slipped"))).toMatchObject({
      ok: true,
    });
    expect(validateNewRoadmapAnnotation(input("grist:doc/Roadmap/1", "2026-02-30", "slipped"))).toMatchObject({
      ok: false,
    });
    expect(
      validateNewRoadmapAnnotation({ ...input("grist:doc/Roadmap/1", "2026-11-01", "slipped"), kind: "note" }),
    ).toMatchObject({
      ok: false,
    });
  });

  test("persists append-only history and resolves a reshuffled row through an alias after restart", async () => {
    const dir = mkdtempSync(join(tmpdir(), "roadmap-annotations-"));
    const file = join(dir, "annotations.json");
    const store = new RoadmapAnnotationsStore(file);
    const stableRef = "grist:doc/Roadmap/old-row";
    const currentRef = "grist:doc/Roadmap/new-row";
    await store.add(input(stableRef, "2026-10-01", "Initial decision"));
    await store.addAlias(currentRef, stableRef);
    await store.add(input(currentRef, "2026-10-03", "Corrected with a follow-up"));

    const restarted = new RoadmapAnnotationsStore(file);
    const history = await restarted.forObject(currentRef);
    expect(history.map((annotation) => annotation.body)).toEqual(["Initial decision", "Corrected with a follow-up"]);
    expect(history[0].stableRef).toBe(stableRef);
    expect(await restarted.aliasEntries()).toEqual({ [currentRef]: stableRef });
  });

  test("a non-ENOENT read failure quarantines + starts empty instead of latching (#4200)", async () => {
    const dir = mkdtempSync(join(tmpdir(), "roadmap-annotations-eisdir-"));
    const file = join(dir, "annotations.json");
    mkdirSync(file); // a directory in place of the store file -> EISDIR on read
    const store = new RoadmapAnnotationsStore(file);
    // The load must not reject (the pre-fix store memoized this
    // rejection and every later read/write rejected for the process
    // lifetime).
    await expect(store.all()).resolves.toEqual([]);
    // The unreadable original was quarantined aside, not deleted.
    expect(readdirSync(dir).some((name) => name.startsWith("annotations.json.corrupt-"))).toBe(true);
    // The same instance serves writes + reads again after the failure.
    await store.add(input("grist:doc/Roadmap/1", "2026-11-02", "Recovered write"));
    expect((await store.forObject("grist:doc/Roadmap/1")).map((a) => a.body)).toEqual(["Recovered write"]);
  });
});
