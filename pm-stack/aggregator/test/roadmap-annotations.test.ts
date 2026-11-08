import { describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
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

  test("quarantines a non-ENOENT read failure and loads the restored store without a rejected-promise latch", async () => {
    const dir = mkdtempSync(join(tmpdir(), "roadmap-annotations-unreadable-"));
    const file = join(dir, "annotations.json");
    mkdirSync(file); // readFile on a directory fails with EISDIR.
    const errors: string[] = [];
    const origError = console.error;
    console.error = (...args: unknown[]) => {
      errors.push(args.map(String).join(" "));
    };

    const store = new RoadmapAnnotationsStore(file);
    try {
      expect(await store.all()).toEqual([]);
    } finally {
      console.error = origError;
    }

    expect(errors.some((message) => message.includes("could not be read"))).toBe(true);
    const quarantined = readdirSync(dir).filter((name) => name.startsWith("annotations.json.corrupt-"));
    expect(quarantined).toHaveLength(1);
    expect(statSync(join(dir, quarantined[0])).isDirectory()).toBe(true);
    expect(existsSync(file)).toBe(false);

    const restored = {
      id: "ra-restored",
      stableRef: "grist:doc/Roadmap/restored",
      kind: "decision",
      date: "2026-11-02",
      author,
      body: "Restored history",
      createdAt: "2026-11-02T12:00:00.000Z",
    };
    writeFileSync(file, JSON.stringify({ version: 1, annotations: [restored], aliases: {} }));

    // The original instance successfully starts empty after quarantining the
    // unreadable item; it does not retain a rejected load promise.
    expect(await store.all()).toEqual([]);
    expect(await new RoadmapAnnotationsStore(file).all()).toEqual([restored]);
    expect(existsSync(join(dir, quarantined[0]))).toBe(true);
    expect(JSON.parse(readFileSync(file, "utf8")).annotations).toEqual([restored]);
  });
});
