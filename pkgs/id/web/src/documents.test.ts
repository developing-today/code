import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  DocumentsRefusal,
  MAX_DOCUMENT_PAGES,
  collectDocuments,
  documentFrame,
  documentsFrame,
  documentsRefusal,
  parseDocumentRead,
  parseDocumentsPage,
  readDocument,
} from "./documents";
import { checkClientFrame, checkServerFrame, parseShellContract } from "./shell-contract";

/**
 * Contract tests for the documents/document web client (#14923).
 *
 * Oracle: the declared shell contract (shell-contract.v1.json) for frame
 * shapes, and the session layer's serving rules in
 * world_session/documents.rs + session_core.rs for behavior — pages are
 * exclusive-`next_after` snapshots, prefix filters server-side, reads
 * return { name, size, text }, and refusals ride the error frame with
 * the DocumentError strings or "join denied".
 */

const here = dirname(fileURLToPath(import.meta.url));
const contract = parseShellContract(
  JSON.parse(readFileSync(resolve(here, "../../contract/shell-contract.v1.json"), "utf8")),
);
const cap = "capability-token";

describe("documents client frames against the shell contract", () => {
  it("builds contract-valid documents frames for first and subsequent pages", () => {
    expect(documentsFrame(cap)).toEqual({ type: "documents", capability: cap });
    expect(checkClientFrame(contract, documentsFrame(cap), "joined").ok).toBe(true);
    expect(checkClientFrame(contract, documentsFrame(cap, { after: "a.md" }), "joined").ok).toBe(true);
    expect(checkClientFrame(contract, documentsFrame(cap, { after: "a.md" }), "opening").ok).toBe(true);
  });

  it("carries the prefix filter on the documents frame", () => {
    const frame = documentsFrame(cap, { prefix: "notes/" });
    expect(frame).toEqual({ type: "documents", capability: cap, prefix: "notes/" });
    expect(checkClientFrame(contract, frame, "joined").ok).toBe(true);
  });

  it("builds a contract-valid document read frame", () => {
    expect(documentFrame(cap, "a.md")).toEqual({ type: "document", capability: cap, name: "a.md" });
    expect(checkClientFrame(contract, documentFrame(cap, "a.md"), "joined").ok).toBe(true);
  });

  it("accepts contract-valid server replies and rejects wrong shapes", () => {
    expect(checkServerFrame(contract, { type: "documents", names: ["a.md"], next_after: null }).ok).toBe(true);
    expect(checkServerFrame(contract, { type: "document", name: "a.md", size: 3, text: "abc" }).ok).toBe(true);
    expect(parseDocumentsPage({ type: "documents", names: ["a.md", 7], next_after: null })).toBeNull();
    expect(parseDocumentsPage({ type: "documents", names: "a.md", next_after: null })).toBeNull();
    expect(parseDocumentsPage({ type: "documents", names: [], next_after: 9 })).toBeNull();
    expect(parseDocumentRead({ type: "document", name: "a.md", size: "3", text: "abc" })).toBeNull();
    expect(parseDocumentRead({ type: "document", name: "a.md", size: 3 })).toBeNull();
  });
});

describe("collectDocuments pagination", () => {
  it("returns the first page when next_after is null", async () => {
    const seen: Array<string | null> = [];
    const names = await collectDocuments(async (after) => {
      seen.push(after);
      return { type: "documents", names: ["a.md", "b.txt"], next_after: null };
    });
    expect(names).toEqual(["a.md", "b.txt"]);
    expect(seen).toEqual([null]);
  });

  it("follows next_after as the exclusive after cursor for subsequent pages", async () => {
    const pages = new Map<string | null, unknown>([
      [null, { type: "documents", names: ["a.md"], next_after: "a.md" }],
      ["a.md", { type: "documents", names: ["b.md"], next_after: "b.md" }],
      ["b.md", { type: "documents", names: ["c.md"], next_after: null }],
    ]);
    const names = await collectDocuments(async (after) => pages.get(after));
    expect(names).toEqual(["a.md", "b.md", "c.md"]);
  });

  it("sends the prefix on every page request the panel builds", async () => {
    // The panel builds each page frame with the same prefix; a reply
    // that ignored it would surface names outside the prefix — the
    // collector itself is prefix-agnostic, so assert the frame shape
    // the panel reuses per page instead.
    for (const after of [null, "a.md"]) {
      expect(documentsFrame(cap, { prefix: "re", after })).toEqual({
        type: "documents",
        capability: cap,
        prefix: "re",
        after: after ?? undefined,
      });
    }
    const names = await collectDocuments(async (after) =>
      after === null
        ? { type: "documents", names: ["re-a.md"], next_after: "re-a.md" }
        : { type: "documents", names: ["re-b.md"], next_after: null },
    );
    expect(names.every((name) => name.startsWith("re"))).toBe(true);
  });

  it("fails closed on a malformed page and on a non-advancing cursor", async () => {
    await expect(collectDocuments(async () => ({ type: "documents", names: [1] }))).rejects.toThrow(
      "malformed documents frame",
    );
    await expect(
      collectDocuments(async () => ({ type: "documents", names: [], next_after: "x" })),
    ).rejects.toThrow("cursor did not advance");
  });

  it("does not request beyond the maximum number of pages", async () => {
    let requests = 0;
    await expect(
      collectDocuments(async () => {
        requests += 1;
        return { type: "documents", names: [], next_after: `cursor-${requests}` };
      }),
    ).rejects.toThrow(`listing exceeded ${MAX_DOCUMENT_PAGES} pages`);
    expect(requests).toBe(MAX_DOCUMENT_PAGES);
  });
});

describe("document reads and refusals", () => {
  it("reads a document's name, size and read-only text", async () => {
    const read = await readDocument(
      async (name) => ({ type: "document", name, size: 5, text: "hello" }),
      "a.md",
    );
    expect(read).toEqual({ name: "a.md", size: 5, text: "hello" });
    expect(parseDocumentRead({ type: "document", name: "a.md", size: 5, text: "hello" })).toEqual(read);
  });

  it.each([
    ["join denial", "join denied", "joinDenied"],
    ["a missing document", "document not found", "missing"],
    ["a non-viewable name", "document name must be a public .txt, .md, or .markdown tag", "invalidName"],
    ["an oversized document", "document exceeds the 64 KiB SSH viewing limit", "tooLarge"],
    ["a binary document", "document is not valid UTF-8 text", "binary"],
    ["an unavailable document", "document unavailable", "unavailable"],
  ] as const)("surfaces %s as a refusal, never as empty data", async (_label, message, kind) => {
    const errorFrame = { type: "error", message };
    expect(documentsRefusal(errorFrame)).toBeInstanceOf(DocumentsRefusal);
    expect(documentsRefusal(errorFrame)?.kind).toBe(kind);
    await expect(collectDocuments(async () => errorFrame)).rejects.toMatchObject({ kind, message });
    await expect(readDocument(async () => errorFrame, "a.md")).rejects.toMatchObject({ kind, message });
  });

  it("treats a non-error frame as no refusal and a malformed error frame as a refusal", () => {
    expect(documentsRefusal({ type: "documents", names: [], next_after: null })).toBeNull();
    expect(documentsRefusal({ type: "error", message: 7 })).toBeInstanceOf(DocumentsRefusal);
    expect(documentsRefusal(null)).toBeNull();
  });
});
