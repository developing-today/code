// #7740: the event log's writers capped actor/summary with raw UTF-16
// slices at exactly the lengths validateStreamEvent enforces (200/500).
// An astral character straddling a writer's cap was split, the validator
// (length-only) passed it, and a lone high surrogate was persisted into
// the append-only, hash-chained event log. The five writer sites now use
// pair-safe capUnits (live.ts, server.ts), and validateStreamEvent drops
// a lone trailing surrogate at the boundary so no present or future
// writer can persist one. Oracle: the stored value never ends in a lone
// high surrogate and always round-trips through UTF-8 unchanged.
import { describe, expect, test } from "bun:test";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { EventsStore, validateStreamEvent } from "../src/events.ts";

const EMOJI = "\u{1F600}"; // 😀 — one astral character, two UTF-16 units
const loneHighAtEnd = (s: string) => {
  const c = s.charCodeAt(s.length - 1);
  return c >= 0xd800 && c <= 0xdbff;
};
const roundTrips = (s: string) => Buffer.from(s, "utf8").toString("utf8") === s;
const at = new Date().toISOString();
const base = { at, actor: "drew", kind: "action-applied", ref: "gh:developing-today/code#7740", summary: "did a thing" };
// The writer shapes from the issue: raw slice at the cap, pair straddling.
const writerSummary = ("x".repeat(499) + EMOJI + "y".repeat(50)).slice(0, 500);
const writerActor = ("r".repeat(199) + EMOJI + "z".repeat(10)).slice(0, 200);

describe("#7740 validateStreamEvent boundary guard is surrogate-pair-safe", () => {
  test("precondition: the writer shapes really do end in a lone high surrogate", () => {
    expect(writerSummary.length).toBe(500);
    expect(loneHighAtEnd(writerSummary)).toBe(true);
    expect(writerActor.length).toBe(200);
    expect(loneHighAtEnd(writerActor)).toBe(true);
  });

  test("summary straddling the 500 cap: dangling half dropped, value validates", () => {
    const r = validateStreamEvent({ ...base, summary: writerSummary });
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.value.summary.length).toBe(499);
    expect(loneHighAtEnd(r.value.summary)).toBe(false);
    expect(roundTrips(r.value.summary)).toBe(true);
  });

  test("actor straddling the 200 cap: dangling half dropped, value validates", () => {
    const r = validateStreamEvent({ ...base, actor: writerActor });
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.value.actor.length).toBe(199);
    expect(loneHighAtEnd(r.value.actor)).toBe(false);
    expect(roundTrips(r.value.actor)).toBe(true);
  });

  test("pair fully inside each cap stays intact (exact-fit controls)", () => {
    const summary = "x".repeat(498) + EMOJI;
    const actor = "r".repeat(198) + EMOJI;
    const r = validateStreamEvent({ ...base, summary, actor });
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.value.summary).toBe(summary);
    expect(r.value.summary.length).toBe(500);
    expect(r.value.actor).toBe(actor);
    expect(r.value.actor.length).toBe(200);
  });

  test("under-cap values are unchanged", () => {
    const r = validateStreamEvent({ ...base, summary: "plain summary 😀 ok", actor: "run-😀" });
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.value.summary).toBe("plain summary 😀 ok");
    expect(r.value.actor).toBe("run-😀");
  });
});

describe("#7740 EventsStore never persists a lone surrogate", () => {
  test("recording writer-shaped events persists clean values and a verifiable chain", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-events-surrogate-7740-"));
    const file = join(dir, "events.json");
    const store = new EventsStore(file);
    const rec = await store.record({ ...base, actor: writerActor, summary: writerSummary });
    expect(loneHighAtEnd(rec.summary)).toBe(false);
    expect(loneHighAtEnd(rec.actor)).toBe(false);
    expect(roundTrips(rec.summary)).toBe(true);
    expect(roundTrips(rec.actor)).toBe(true);
    const raw = readFileSync(file, "utf8");
    expect(raw.includes("\\ud83d")).toBe(false);
    const health = await store.health();
    expect(health.integrity.ok).toBe(true);
  });
});
