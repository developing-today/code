// Regression tests for the /api/tags response parser (#7841).
// Oracle: the acceptance shapes from issue #7841 — both documented
// control shapes parse; every malformed shape fails closed (null)
// instead of being rendered, coerced, or treated as "no tags".
import { describe, expect, it } from "vitest";
import { parseTagsResponse } from "./tags-response";

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
    expect(parseTagsResponse([{ key: "project", value: "alpha" }])).toEqual([
      { key: "project", value: "alpha" },
    ]);
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
