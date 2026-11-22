import { describe, expect, test } from "bun:test";
import { mergeRecords, migrateRegistryProvenance } from "../src/registry.ts";
import { applyOwnerConfirmedNameLinks } from "../src/identity-migration.ts";
import { dashCommentHtml, sessionRecordLine } from "../src/render.ts";
import type { SessionRecord } from "../src/model.ts";

const LEGACY: SessionRecord = {
  id: "session-legacy",
  source: "opencode-sandbox",
  label: "A title with exact spacing  ",
  agentId: "opencode:main",
  agentLabel: "claimed label",
  parentId: null,
  status: "idle",
  taskRefs: [],
};

describe("registry provenance migration (#9607)", () => {
  test("marks pre-migration rows asserted-legacy without rewriting identity strings", () => {
    const first = migrateRegistryProvenance([LEGACY]);
    expect(first.changed).toBe(true);
    expect(first.records[0]).toMatchObject({
      provenance: "asserted-legacy",
      label: LEGACY.label,
      agentId: LEGACY.agentId,
      agentLabel: LEGACY.agentLabel,
    });

    const second = migrateRegistryProvenance(first.records);
    expect(second.changed).toBe(false);
    expect(second.records[0]).toBe(first.records[0]);
  });

  test("does not downgrade explicitly verified rows", () => {
    const verified = { ...LEGACY, provenance: "verified" as const };
    const result = migrateRegistryProvenance([verified]);
    expect(result.changed).toBe(false);
    expect(result.records[0]).toBe(verified);
  });

  test("a pre-marker refresh cannot downgrade a verified stored row", () => {
    const verified = { ...LEGACY, provenance: "verified" as const };
    const [merged] = mergeRecords([verified], [LEGACY]);
    expect(merged?.provenance).toBe("verified");
    expect(merged?.label).toBe(LEGACY.label);
  });

  test("renders asserted provenance visibly on comments and registry records", () => {
    const comment = {
      id: "dc-legacy",
      objectRef: "gh:developing-today/code#9607",
      kind: "note" as const,
      author: { kind: "drew" as const, id: "drew", label: "Drew" },
      body: "legacy claim",
      createdAt: "2026-11-01T00:00:00.000Z",
    };
    expect(dashCommentHtml(comment)).toContain("asserted-legacy — author is a claim");
    expect(sessionRecordLine(LEGACY)).toContain("asserted-legacy — identity is a claim");
  });

  test("links only exact owner-confirmed author tuples and leaves provenance and labels alone", () => {
    const comment = {
      id: "dc-legacy",
      objectRef: "gh:developing-today/code#9607",
      kind: "note" as const,
      author: { kind: "drew" as const, id: "drew", label: "Drew " },
      provenance: "asserted-legacy" as const,
      body: "legacy claim",
      createdAt: "2026-11-01T00:00:00.000Z",
    };
    const confirmation = {
      author: comment.author,
      identity: { source: LEGACY.source, id: LEGACY.id },
    };
    const absent = applyOwnerConfirmedNameLinks([comment], [LEGACY], []);
    expect(absent.linked).toBe(0);
    expect(absent.comments[0]).toBe(comment);

    const linked = applyOwnerConfirmedNameLinks([comment], [LEGACY], [confirmation]);
    expect(linked.linked).toBe(1);
    expect(linked.comments[0]).toMatchObject({
      author: comment.author,
      provenance: "asserted-legacy",
      linkedIdentity: { source: LEGACY.source, id: LEGACY.id },
    });
    expect(dashCommentHtml(linked.comments[0]!)).toContain('title="Owner-confirmed display link"');
  });

  test("ambiguous confirmations and missing registry targets do not create links", () => {
    const comment = {
      id: "dc-legacy",
      objectRef: "gh:developing-today/code#9607",
      kind: "note" as const,
      author: { kind: "drew" as const, id: "drew", label: "Drew" },
      body: "legacy claim",
      createdAt: "2026-11-01T00:00:00.000Z",
    };
    const first = { author: comment.author, identity: { source: LEGACY.source, id: LEGACY.id } };
    const conflicting = { author: comment.author, identity: { source: LEGACY.source, id: "second-session" } };
    const secondRecord = { ...LEGACY, id: "second-session" };
    expect(applyOwnerConfirmedNameLinks([comment], [LEGACY, secondRecord], [first, conflicting]).linked).toBe(0);
    expect(applyOwnerConfirmedNameLinks([comment], [LEGACY], [conflicting]).linked).toBe(0);
  });
});
