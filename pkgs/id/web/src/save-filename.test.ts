import { describe, expect, it } from "vitest";
import { resolveSaveFilename } from "./save-filename";

// The dataset literals below mirror exactly what the server emits on
// #editor-container (pkgs/id/src/web/templates.rs): data-doc-id is the RAW
// filename (html-escaped only); data-filename is urlencoding::encode(name),
// which percent-encodes "%" itself as %25 and " " as %20.
describe("resolveSaveFilename", () => {
  it("resolves a plain filename", () => {
    expect(resolveSaveFilename({ filename: "notes.md" })).toBe("notes.md");
  });

  it("saves a file whose name contains a valid %XX sequence under its own exact name (#1494)", () => {
    // Opened file: "a%20b.md". The old saveFile decoded dataset.docId
    // ("a%20b.md") and posted "a b.md" — a different file's name.
    const dataset = { docId: "a%20b.md", filename: "a%2520b.md" };
    expect(resolveSaveFilename(dataset)).toBe("a%20b.md");
  });

  it("saves a file whose name contains a bare % without throwing (#1494)", () => {
    // Opened file: "100%.md". decodeURIComponent on the raw docId threw
    // URIError on every save attempt, so the file could never be saved.
    const dataset = { docId: "100%.md", filename: "100%25.md" };
    expect(resolveSaveFilename(dataset)).toBe("100%.md");
  });

  it("decodes url-encoded characters from dataset.filename", () => {
    // Opened file: "my notes.md" — data-filename carries the encoded form.
    expect(resolveSaveFilename({ filename: "my%20notes.md" })).toBe("my notes.md");
  });

  it("ignores dataset.docId entirely", () => {
    // Even a docId that would decode to another name must not win.
    const dataset = { docId: "other%20file.md", filename: "mine.md" };
    expect(resolveSaveFilename(dataset)).toBe("mine.md");
  });

  it("returns null when the filename field is missing or empty", () => {
    expect(resolveSaveFilename({})).toBe(null);
    expect(resolveSaveFilename({ filename: "" })).toBe(null);
    expect(resolveSaveFilename({ filename: undefined })).toBe(null);
  });
});
