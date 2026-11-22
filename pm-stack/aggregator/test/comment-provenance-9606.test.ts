import { describe, expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { commentProvenance, CommentsStore } from "../src/comments.ts";

// #9606 (#1253 split 4): every stored comment says whether its author was
// derived from a verified Access identity or is the caller's own claim,
// and records written before the marker existed never read as verified.

const AUTHOR = { kind: "drew", id: "drew", label: "Drew" } as const;
const NEW = { objectRef: "gh:developing-today/code#9606", kind: "note", author: { ...AUTHOR }, body: "hi" } as const;

function withDir(run: (dir: string) => Promise<void>): Promise<void> {
  const dir = mkdtempSync(join(tmpdir(), "agg-provenance-9606-"));
  return run(dir).finally(() => rmSync(dir, { recursive: true, force: true }));
}

describe("comment provenance (#9606)", () => {
  test("a missing or unknown marker reads as asserted-legacy, never verified", () => {
    expect(commentProvenance({})).toBe("asserted-legacy");
    expect(commentProvenance({ provenance: "asserted-legacy" })).toBe("asserted-legacy");
    expect(commentProvenance({ provenance: "verified" })).toBe("verified");
  });

  test("add stamps asserted-legacy by default and verified only when told, and both persist", () =>
    withDir(async (dir) => {
      const file = join(dir, "comments.json");
      const store = new CommentsStore(file);
      const asserted = await store.add({ ...NEW });
      const verified = await store.add({ ...NEW }, { provenance: "verified" });
      expect(asserted.provenance).toBe("asserted-legacy");
      expect(verified.provenance).toBe("verified");
      const reloaded = await new CommentsStore(file).all();
      expect(reloaded.find((c) => c.id === asserted.id)?.provenance).toBe("asserted-legacy");
      expect(reloaded.find((c) => c.id === verified.id)?.provenance).toBe("verified");
    }));

  test("pre-marker records are migrated once without changing author strings", () =>
    withDir(async (dir) => {
      const file = join(dir, "comments.json");
      const legacy = {
        id: "dc-legacy",
        objectRef: "gh:developing-today/code#941",
        kind: "message",
        author: { ...AUTHOR, label: "  Drew 👋  " },
        authorVerified: false,
        body: "written before provenance existed",
        createdAt: "2026-11-01T00:00:00.000Z",
        triageState: "new",
      };
      writeFileSync(file, JSON.stringify({ version: 1, comments: [legacy] }));
      const [loaded] = await new CommentsStore(file).all();
      expect(loaded?.id).toBe("dc-legacy");
      expect(loaded?.provenance).toBe("asserted-legacy");
      expect(loaded?.author.label).toBe("  Drew 👋  ");
      expect(commentProvenance(loaded!)).toBe("asserted-legacy");
      const migratedBytes = readFileSync(file, "utf8");
      expect(JSON.parse(migratedBytes).comments[0].author.label).toBe("  Drew 👋  ");
      await new CommentsStore(file).all();
      expect(readFileSync(file, "utf8")).toBe(migratedBytes);
    }));

  test("a record claiming an unknown provenance is not loaded as a valid comment", () =>
    withDir(async (dir) => {
      const file = join(dir, "comments.json");
      const forged = {
        id: "dc-forged",
        objectRef: "gh:developing-today/code#941",
        kind: "note",
        author: { ...AUTHOR },
        provenance: "trusted",
        body: "forged marker",
        createdAt: "2026-11-01T00:00:00.000Z",
      };
      writeFileSync(file, JSON.stringify({ version: 1, comments: [forged] }));
      const all = await new CommentsStore(file).all();
      expect(all.some((c) => c.id === "dc-forged")).toBe(false);
    }));
});
