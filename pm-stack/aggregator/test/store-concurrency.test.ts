import { describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { ActionsStore, type NewAction } from "../src/actions.ts";
import { CommentsStore, type NewComment } from "../src/comments.ts";

// #1509: the JSON stores persisted through a temp file named only by pid
// with no write serialization, so concurrent adds shared one temp path —
// the first rename won, the rest threw ENOENT, and records on disk
// diverged from calls that succeeded (triage repro: 20 concurrent adds
// -> 2 fulfilled, 18 ENOENT, 19 records on disk). Persists are now
// serialized per store with a unique temp path per write; these tests
// pin fulfilled == persisted for both stores.
describe("JSON stores under concurrent adds (#1509)", () => {
  const author = { kind: "drew" as const, id: "drew", label: "Drew" };

  test("20 concurrent comment adds all fulfill and all persist", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-comments-conc-"));
    const file = join(dir, "comments.json");
    const store = new CommentsStore(file);
    const inputs: NewComment[] = Array.from({ length: 20 }, (_, i) => ({
      objectRef: "orch:item-1",
      kind: "note" as const,
      author,
      body: `note ${i}`,
    }));
    const results = await Promise.allSettled(inputs.map((x) => store.add(x)));
    expect(results.filter((r) => r.status === "fulfilled").length).toBe(20);
    const reloaded = new CommentsStore(file);
    expect((await reloaded.all()).length).toBe(20);
  });

  test("20 concurrent action adds all fulfill and all persist", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-actions-conc-"));
    const file = join(dir, "actions.json");
    const store = new ActionsStore(file);
    const inputs: NewAction[] = Array.from({ length: 20 }, (_, i) => ({
      clientToken: `ct-conc-${i}-abcdefgh`,
      kind: "triage" as const,
      verb: "acknowledge",
      objectRef: "orch:item-1",
      payload: {},
      author,
    }));
    const results = await Promise.allSettled(inputs.map((x) => store.add(x)));
    const fulfilled = results.filter((r) => r.status === "fulfilled");
    expect(fulfilled.length).toBe(20);
    expect(fulfilled.every((r) => r.status === "fulfilled" && r.value.created)).toBe(true);
    const reloaded = new ActionsStore(file);
    expect((await reloaded.all()).length).toBe(20);
  });
});
