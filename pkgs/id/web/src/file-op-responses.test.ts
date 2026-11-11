import { describe, expect, it } from "vitest";
import {
  isCopyFileResponse,
  isNewFileResponse,
  isRenameFileResponse,
  isSaveFileResponse,
} from "./file-op-responses";

// Regression tests for #7837: the /api/save, /api/new, /api/rename and
// /api/copy handlers in main.ts shape-trusted their 200 responses via
// bare `as` casts. These guards are the validation the handlers now
// run before any field is used; every wrong-shape body below used to
// be treated as success (dataset.hash = "undefined", navigation to
// /edit/undefined, "renamed!" / "copied!" button text).

const wrongShapes: unknown[] = [{}, null, "abc", [], 42, undefined];

describe("isSaveFileResponse", () => {
  it("accepts a valid save response", () => {
    expect(isSaveFileResponse({ hash: "abc123", name: "notes.md", archive_name: null })).toBe(true);
    expect(
      isSaveFileResponse({ hash: "abc123", name: "notes.md", archive_name: "notes-1.md" }),
    ).toBe(true);
  });

  it("rejects wrong-shape bodies", () => {
    for (const body of wrongShapes) {
      expect(isSaveFileResponse(body)).toBe(false);
    }
  });

  it("rejects a numeric hash (silent coercion pre-fix)", () => {
    expect(isSaveFileResponse({ hash: 123, name: "notes.md", archive_name: null })).toBe(false);
  });

  it("rejects empty or missing fields", () => {
    expect(isSaveFileResponse({ hash: "", name: "notes.md", archive_name: null })).toBe(false);
    expect(isSaveFileResponse({ hash: "abc123", name: "", archive_name: null })).toBe(false);
    expect(isSaveFileResponse({ hash: "abc123", name: "notes.md" })).toBe(false);
    expect(isSaveFileResponse({ hash: "abc123", name: "notes.md", archive_name: 7 })).toBe(false);
  });
});

describe("isNewFileResponse", () => {
  it("accepts a valid create response", () => {
    expect(isNewFileResponse({ hash: "abc123", name: "new.md" })).toBe(true);
  });

  it("rejects wrong-shape bodies (navigated to /edit/undefined pre-fix)", () => {
    for (const body of wrongShapes) {
      expect(isNewFileResponse(body)).toBe(false);
    }
    expect(isNewFileResponse({ hash: 123, name: "new.md" })).toBe(false);
    expect(isNewFileResponse({ name: "new.md" })).toBe(false);
  });
});

describe("isRenameFileResponse", () => {
  it("accepts a valid rename response", () => {
    expect(
      isRenameFileResponse({
        name: "renamed.md",
        hash: "abc123",
        archived_original: null,
        archived_replaced: "old.md",
      }),
    ).toBe(true);
  });

  it("rejects wrong-shape bodies (fake 'renamed!' pre-fix)", () => {
    for (const body of wrongShapes) {
      expect(isRenameFileResponse(body)).toBe(false);
    }
    expect(
      isRenameFileResponse({
        name: "renamed.md",
        hash: 123,
        archived_original: null,
        archived_replaced: null,
      }),
    ).toBe(false);
  });

  it("rejects non-string-or-null archive fields", () => {
    expect(
      isRenameFileResponse({
        name: "renamed.md",
        hash: "abc123",
        archived_original: 7,
        archived_replaced: null,
      }),
    ).toBe(false);
    expect(isRenameFileResponse({ name: "renamed.md", hash: "abc123" })).toBe(false);
  });
});

describe("isCopyFileResponse", () => {
  it("accepts a valid copy response", () => {
    expect(isCopyFileResponse({ name: "copy.md", hash: "abc123" })).toBe(true);
  });

  it("rejects wrong-shape bodies (fake 'copied!' pre-fix)", () => {
    for (const body of wrongShapes) {
      expect(isCopyFileResponse(body)).toBe(false);
    }
    expect(isCopyFileResponse({ name: "copy.md", hash: 123 })).toBe(false);
    expect(isCopyFileResponse({ hash: "abc123" })).toBe(false);
  });
});
