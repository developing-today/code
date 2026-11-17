// #7731: validateAuthor (comments.ts) capped id/label/sessionId with a raw
// slice(0, 120) and validateNewComment capped the body with a raw
// slice(0, 20_000) — an astral character straddling either cap was split,
// leaving a lone high surrogate in the persisted author/comment record
// (the #3178 class; #7694/#7711 fixed the same defect for actions.ts text
// fields via capUnits, but this shared validator was not audited).
import { describe, expect, test } from "bun:test";
import { validateAuthor, validateNewComment } from "../src/comments.ts";

const EMOJI = "\u{1F600}"; // 😀 — one astral character, two UTF-16 units
const loneHighAtEnd = (s: string) => {
  const c = s.charCodeAt(s.length - 1);
  return c >= 0xd800 && c <= 0xdbff;
};
const roundTrips = (s: string) => Buffer.from(s, "utf8").toString("utf8") === s;

describe("#7731 validateAuthor caps are surrogate-pair-safe", () => {
  test("label straddling the 120 cap is dropped whole", () => {
    const a = validateAuthor({ kind: "agent", id: "a1", label: "x".repeat(119) + EMOJI + "tail" });
    expect(a).not.toBeNull();
    expect(a!.label.length).toBe(119);
    expect(loneHighAtEnd(a!.label)).toBe(false);
    expect(roundTrips(a!.label)).toBe(true);
  });

  test("id straddling the 120 cap is dropped whole", () => {
    const a = validateAuthor({ kind: "drew", id: "y".repeat(119) + EMOJI + "tail", label: "ok" });
    expect(a).not.toBeNull();
    expect(a!.id.length).toBe(119);
    expect(loneHighAtEnd(a!.id)).toBe(false);
    expect(roundTrips(a!.id)).toBe(true);
  });

  test("sessionId straddling the 120 cap is dropped whole", () => {
    const a = validateAuthor({ kind: "agent", id: "a1", label: "ok", sessionId: "s".repeat(119) + EMOJI + "tail" });
    expect(a).not.toBeNull();
    expect(a!.sessionId!.length).toBe(119);
    expect(loneHighAtEnd(a!.sessionId!)).toBe(false);
  });

  test("pair fully inside the 120 cap stays intact (exact-fit control)", () => {
    const label = "x".repeat(118) + EMOJI;
    const a = validateAuthor({ kind: "agent", id: "a1", label });
    expect(a!.label).toBe(label);
    expect(a!.label.length).toBe(120);
  });

  test("plain over-long ASCII still caps at exactly 120", () => {
    const a = validateAuthor({ kind: "agent", id: "a1", label: "x".repeat(200) });
    expect(a!.label.length).toBe(120);
  });
});

describe("#7731 validateNewComment body cap is surrogate-pair-safe", () => {
  const author = { kind: "drew" as const, id: "d", label: "D" };

  test("body straddling the 20,000 cap is dropped whole", () => {
    const r = validateNewComment({ objectRef: "code#1", kind: "note", author, body: "z".repeat(19_999) + EMOJI + "tail" });
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.value.body.length).toBe(19_999);
    expect(loneHighAtEnd(r.value.body)).toBe(false);
    expect(roundTrips(r.value.body)).toBe(true);
  });

  test("pair fully inside the 20,000 cap stays intact (exact-fit control)", () => {
    const body = "z".repeat(19_998) + EMOJI;
    const r = validateNewComment({ objectRef: "code#1", kind: "note", author, body });
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.value.body).toBe(body);
    expect(r.value.body.length).toBe(20_000);
  });
});
