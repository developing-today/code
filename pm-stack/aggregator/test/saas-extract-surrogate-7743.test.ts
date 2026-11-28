// #7743: saas-extract.ts capped six extracted text fields with raw slices
// (reads/conflicts 400, PAYG notes 300/220, usageNote 600, and a
// since-removed export-overlay metric note 600) — an astral character straddling any cap was split, leaving a
// lone high surrogate in the SaasService field that is serialized into the
// feed/board JSON (the #3178 class; #7694/#7711 fixed the same defect for
// neighbouring modules via capUnits, and #7741's audit of the remaining
// slices missed this module).
import { describe, expect, test } from "bun:test";
import { extractSaasServices } from "../src/saas-extract.ts";

const EMOJI = "\u{1F600}"; // 😀 — one astral character, two UTF-16 units
const loneHighAtEnd = (s: string) => {
  const c = s.charCodeAt(s.length - 1);
  return c >= 0xd800 && c <= 0xdbff;
};
const roundTrips = (s: string) => Buffer.from(s, "utf8").toString("utf8") === s;

// Bitwarden section lines, each placing the emoji pair at a known offset:
// reads line after "- " strip = "2026-11-01 " (11) + filler, cap 400
const readStraddle = "- 2026-11-01 " + "a".repeat(388) + EMOJI + "zzzz";
const readExactFit = "- 2026-11-01 " + "a".repeat(387) + EMOJI;
// conflicts line after strip = "conflict: " (10) + filler, cap 400
const conflictStraddle = "- conflict: " + "a".repeat(389) + EMOJI + "zzzz";
// PAYG estimate note after the label strip = "low $1 / high $2 per month; " (28) + filler, cap 300
const paygStraddle = "- **PAYG estimate:** low $1 / high $2 per month; " + "a".repeat(271) + EMOJI + "zzzz";
// usageNote after the bullet prefix strip = filler, cap 600
const usageStraddle = "- **Usage / dates:** " + "a".repeat(599) + EMOJI + "zzzz";
// not-applicable note = "not applicable — " (17) + filler, cap 220
const paygNaStraddle = "- **PAYG estimate:** not applicable — " + "a".repeat(202) + EMOJI + "zzzz.";

function bitwarden(lines: string[]) {
  const doc = ["## Summary (b)", "", "### Bitwarden", "", ...lines, ""].join("\n");
  return extractSaasServices(doc).find((s) => s.id === "bitwarden")!;
}

describe("#7743 extractSaasServices caps are surrogate-pair-safe", () => {
  test("dated-read text straddling the 400 cap is dropped whole", () => {
    const svc = bitwarden([readStraddle]);
    const text = svc.reads[0].text;
    expect(text.length).toBe(399);
    expect(loneHighAtEnd(text)).toBe(false);
    expect(roundTrips(text)).toBe(true);
  });

  test("dated-read pair ending exactly at the 400 cap stays intact (control)", () => {
    const svc = bitwarden([readExactFit]);
    const text = svc.reads[0].text;
    expect(text.length).toBe(400);
    expect(text.endsWith(EMOJI)).toBe(true);
    expect(roundTrips(text)).toBe(true);
  });

  test("conflict line straddling the 400 cap is dropped whole", () => {
    const svc = bitwarden([conflictStraddle]);
    const line = svc.conflicts[0];
    expect(line.length).toBe(399);
    expect(loneHighAtEnd(line)).toBe(false);
    expect(roundTrips(line)).toBe(true);
  });

  test("PAYG estimate note straddling the 300 cap is dropped whole", () => {
    const svc = bitwarden([paygStraddle]);
    const note = svc.paygEstimate!.note!;
    expect(note.length).toBe(299);
    expect(loneHighAtEnd(note)).toBe(false);
    expect(roundTrips(note)).toBe(true);
  });

  test("PAYG not-applicable note straddling the 220 cap is dropped whole", () => {
    const svc = bitwarden([paygNaStraddle]);
    const note = svc.paygNote!;
    expect(note.length).toBe(219);
    expect(loneHighAtEnd(note)).toBe(false);
    expect(roundTrips(note)).toBe(true);
  });

  test("usageNote straddling the 600 cap is dropped whole", () => {
    const svc = bitwarden([usageStraddle]);
    const note = svc.usageNote!;
    expect(note.length).toBe(599);
    expect(loneHighAtEnd(note)).toBe(false);
    expect(roundTrips(note)).toBe(true);
  });
});
