// Regression tests for the /api/tags response parser (#7841).
// Oracle: the acceptance shapes from issue #7841 — both documented
// control shapes parse; every malformed shape fails closed (null)
// instead of being rendered, coerced, or treated as "no tags".
import { describe, expect, it } from "vitest";
import { parseTagsResponse, parseTagWsEvent } from "./tags-response";

describe("parseTagsResponse control shapes", () => {
  it("parses the object form and keeps string and null values", () => {
    expect(
      parseTagsResponse({
        tags: [
          { key: "project", value: "alpha" },
          { key: "created", value: "2026-01-01" },
          { key: "plain", value: null },
        ],
      }),
    ).toEqual([
      { key: "project", value: "alpha" },
      { key: "created", value: "2026-01-01" },
      { key: "plain", value: null },
    ]);
  });

  it("parses the flat array form", () => {
    expect(parseTagsResponse([{ key: "project", value: "alpha" }])).toEqual([{ key: "project", value: "alpha" }]);
  });

  it("parses an empty tag list as an empty array, not a failure", () => {
    expect(parseTagsResponse({ tags: [] })).toEqual([]);
    expect(parseTagsResponse([])).toEqual([]);
  });

  it("normalizes a missing value to null", () => {
    expect(parseTagsResponse([{ key: "plain" }])).toEqual([{ key: "plain", value: null }]);
  });
});

describe("parseTagsResponse fails closed on wrong shapes", () => {
  it.each([
    ["empty object", {}],
    ["null body", null],
    ["bare string", "abc"],
    ["tags is a string", { tags: "abc" }],
    ["tags is null", { tags: null }],
    ["null entry", [null]],
    ["numeric key", [{ key: 42, value: "x" }]],
    ["object value", [{ key: "note", value: { a: 1 } }]],
    ["numeric value", [{ key: "note", value: 7 }]],
    ["bare string entry", ["project"]],
  ])("rejects %s", (_name, raw) => {
    expect(parseTagsResponse(raw)).toBeNull();
  });
});

// Boundary guard for /ws/tags broadcast frames (#7983 item 2).
// Oracle: the TagEvent wire format from book/reference/websockets.md —
// every frame is an object with a string `type`; `subject`/`key`/`value`
// are the only fields this surface reads. Probe matrix: null, array,
// scalar, {}, each relied-on field wrong-typed.
describe("parseTagWsEvent control shapes", () => {
  it("parses a set frame", () => {
    expect(
      parseTagWsEvent({ type: "set", ns: "global", subject: "report.txt", key: "status", value: "draft" }),
    ).toEqual({ type: "set", subject: "report.txt", key: "status", value: "draft" });
  });

  it("normalizes missing fields to null (del_all, transfer)", () => {
    expect(parseTagWsEvent({ type: "del_all", ns: "global", subject: "report-final.txt" })).toEqual({
      type: "del_all",
      subject: "report-final.txt",
      key: null,
      value: null,
    });
    expect(parseTagWsEvent({ type: "transfer", ns: "global", from: "a.txt", to: "b.txt", count: 3 })).toEqual({
      type: "transfer",
      subject: null,
      key: null,
      value: null,
    });
  });
});

describe("parseTagWsEvent fails closed on wrong shapes", () => {
  it.each([
    ["null frame", null],
    ["bare string", "ok"],
    ["bare number", 42],
    ["array frame", []],
    ["empty object", {}],
    ["missing type", { subject: "a.txt" }],
    ["numeric type", { type: 7 }],
    ["numeric subject", { type: "set", subject: 42 }],
    ["object subject", { type: "set", subject: { a: 1 } }],
    ["numeric key", { type: "set", subject: "a.txt", key: 7 }],
    ["object value", { type: "set", subject: "a.txt", key: "k", value: { a: 1 } }],
    ["array value", { type: "set", subject: "a.txt", key: "k", value: ["x"] }],
  ])("rejects %s", (_name, raw) => {
    expect(parseTagWsEvent(raw)).toBeNull();
  });
});
