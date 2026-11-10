// #7736: validateNewRoadmapAnnotation (roadmap-annotations.ts) capped the
// body with a raw slice(0, 20_000) — an astral character straddling the cap
// was split, leaving a lone high surrogate in the persisted annotation
// record (the #3178 class; #7731 fixed the shared validateAuthor this
// module imports and the comments body cap, but this module's own body
// cap was not audited).
import { describe, expect, test } from "bun:test";
import { validateNewRoadmapAnnotation } from "../src/roadmap-annotations.ts";

const EMOJI = "\u{1F600}"; // 😀 — one astral character, two UTF-16 units
const loneHighAtEnd = (s: string) => {
  const c = s.charCodeAt(s.length - 1);
  return c >= 0xd800 && c <= 0xdbff;
};
const roundTrips = (s: string) => Buffer.from(s, "utf8").toString("utf8") === s;
const author = { kind: "drew" as const, id: "drew", label: "Drew" };
const ann = (body: string) =>
  validateNewRoadmapAnnotation({ objectRef: "grist:doc/Roadmap/1", kind: "context", date: "2026-11-08", author, body });

describe("#7736 validateNewRoadmapAnnotation body cap is surrogate-pair-safe", () => {
  test("body straddling the 20,000 cap is dropped whole", () => {
    const r = ann("a".repeat(19_999) + EMOJI + "tail");
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.value.body.length).toBe(19_999);
    expect(loneHighAtEnd(r.value.body)).toBe(false);
    expect(roundTrips(r.value.body)).toBe(true);
  });

  test("pair fully inside the 20,000 cap stays intact (exact-fit control)", () => {
    const body = "a".repeat(19_998) + EMOJI;
    const r = ann(body);
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.value.body).toBe(body);
    expect(r.value.body.length).toBe(20_000);
  });

  test("plain over-long ASCII still caps at exactly 20,000", () => {
    const r = ann("a".repeat(25_000));
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.value.body.length).toBe(20_000);
  });

  test("under-cap body with an astral character is unchanged", () => {
    const body = `decision ${EMOJI} recorded`;
    const r = ann(body);
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.value.body).toBe(body);
  });
});
