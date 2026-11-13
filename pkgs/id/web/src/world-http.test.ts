import { describe, expect, it } from "vitest";
import {
  advanceHostViewCursor,
  HOST_VIEW_MAX_PAGES,
  parseWorldHostViewPage,
  parseWorldInviteResponse,
} from "./world-http";

// Oracle: the server contracts in pkgs/id/src/web/world_ws.rs —
// HostViewResponse (all fields always present; snapshot/view/surface/
// next_after nullable) and the invite handler's { capability,
// display_name } with a non-empty string capability. The malformed cases
// are the ones issue #7833 probed live against the unguarded page.

const validSnapshot = {
  world_id: "arena",
  current_sequence: 7,
  active_module_hash: null,
  participants: [{ id: 1, display_name: "ann" }],
};

const validPage = {
  snapshot: validSnapshot,
  view: "hello",
  surface: null,
  records_sequence: 42,
  records: { greeting: "hi" },
  next_after: null,
};

describe("parseWorldHostViewPage", () => {
  it("accepts a well-formed page (control)", () => {
    expect(parseWorldHostViewPage(validPage)).toEqual(validPage);
  });

  it("accepts a later page with null snapshot/view/surface and a string next_after", () => {
    const page = { ...validPage, snapshot: null, view: null, next_after: "greeting" };
    expect(parseWorldHostViewPage(page)).toEqual(page);
  });

  it.each([null, {}, [], "abc", 42])("rejects a non-object or empty body: %j", (payload) => {
    expect(() => parseWorldHostViewPage(payload)).toThrow(/^Host view unavailable/);
  });

  it("rejects records that are not a plain object (the per-character render case)", () => {
    expect(() => parseWorldHostViewPage({ ...validPage, records: "abc" })).toThrow(/^Host view unavailable/);
    expect(() => parseWorldHostViewPage({ ...validPage, records: null })).toThrow(/^Host view unavailable/);
    expect(() => parseWorldHostViewPage({ ...validPage, records: [] })).toThrow(/^Host view unavailable/);
  });

  it("rejects a non-number records_sequence", () => {
    expect(() => parseWorldHostViewPage({ ...validPage, records_sequence: "42" })).toThrow(/^Host view unavailable/);
    expect(() => parseWorldHostViewPage({ ...validPage, records_sequence: Number.NaN })).toThrow(
      /^Host view unavailable/,
    );
  });

  it("rejects a wrong-typed next_after", () => {
    expect(() => parseWorldHostViewPage({ ...validPage, next_after: 3 })).toThrow(/^Host view unavailable/);
  });

  it("rejects a snapshot that fails the snapshot guard", () => {
    expect(() => parseWorldHostViewPage({ ...validPage, snapshot: { world_id: "arena" } })).toThrow(
      /^Host view unavailable/,
    );
    expect(() =>
      parseWorldHostViewPage({ ...validPage, snapshot: { ...validSnapshot, participants: [{ id: "1" }] } }),
    ).toThrow(/^Host view unavailable/);
  });
});

describe("parseWorldInviteResponse", () => {
  it("accepts a well-formed invite (control)", () => {
    expect(parseWorldInviteResponse({ capability: "cap-abc-123", display_name: "guest" })).toEqual({
      capability: "cap-abc-123",
      display_name: "guest",
    });
  });

  it.each([null, {}, [], "abc", 42])("rejects a non-object or empty body: %j", (payload) => {
    expect(() => parseWorldInviteResponse(payload)).toThrow(/^Invite failed/);
  });

  it("rejects a missing, empty, or non-string capability (the persisted-'undefined' case)", () => {
    expect(() => parseWorldInviteResponse({ display_name: "guest" })).toThrow(/^Invite failed/);
    expect(() => parseWorldInviteResponse({ capability: "" })).toThrow(/^Invite failed/);
    expect(() => parseWorldInviteResponse({ capability: 123 })).toThrow(/^Invite failed/);
    expect(() => parseWorldInviteResponse({ capability: null })).toThrow(/^Invite failed/);
  });

  it("falls back to a null display_name when it is absent or wrong-typed", () => {
    expect(parseWorldInviteResponse({ capability: "cap-abc-123" })).toEqual({
      capability: "cap-abc-123",
      display_name: null,
    });
    expect(parseWorldInviteResponse({ capability: "cap-abc-123", display_name: 7 })).toEqual({
      capability: "cap-abc-123",
      display_name: null,
    });
  });
});

describe("advanceHostViewCursor (#9043)", () => {
  it("ends paging on a null cursor (control)", () => {
    expect(advanceHostViewCursor({ previous: null, next: null, page: 1 })).toBeNull();
    expect(advanceHostViewCursor({ previous: "a", next: null, page: 2 })).toBeNull();
  });

  it("passes an advancing cursor through (control)", () => {
    expect(advanceHostViewCursor({ previous: null, next: "a", page: 1 })).toBe("a");
    expect(advanceHostViewCursor({ previous: "a", next: "b", page: 2 })).toBe("b");
  });

  it("refuses a cursor that repeats the one just requested", () => {
    expect(() => advanceHostViewCursor({ previous: "a", next: "a", page: 2 })).toThrow(
      /^Host view unavailable: records cursor did not advance/,
    );
  });

  it("refuses to page past the named bound even with advancing cursors", () => {
    expect(() => advanceHostViewCursor({ previous: "a", next: "b", page: HOST_VIEW_MAX_PAGES })).toThrow(
      /^Host view unavailable: records paging exceeded/,
    );
    expect(advanceHostViewCursor({ previous: "a", next: "b", page: HOST_VIEW_MAX_PAGES - 1 })).toBe("b");
  });
});
