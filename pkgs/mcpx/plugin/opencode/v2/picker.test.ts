/**
 * The v2 daemon picker (#1858 SET-REMAINDER item (3), under #2841),
 * against stub deps: a fixed candidate list, a recording assert, and a
 * recording applyChoice standing in for the ctx.storage-backed one in
 * tools.ts. The oracle is state, not text: a selection counts only
 * when applyChoice ran with the picked target, and every failure path
 * — bad index, no permission domain, a rejected assert (the user
 * cancelling the picker) — must leave applyChoice uncalled, which is
 * what "the selected daemon did not change" means here.
 *
 *   bun test plugin/opencode/v2/picker.test.ts
 */
import { describe, expect, test } from "bun:test";
import { targetKey, type Candidate, type Target } from "../../core/daemon.ts";
import { buildPickerOptions, renderPickerMenu, runPickerSelection, type PickerScope } from "./picker.ts";

const a: Target = { kind: "socket", path: "/tmp/a.sock" };
const b: Target = { kind: "socket", path: "/tmp/b.sock" };

const candidate = (target: Target, over: Partial<Candidate> = {}): Candidate => ({
  target,
  source: "info-file",
  configPath: `/cfg/${target.kind === "socket" ? target.path.split("/").pop() : "url"}.json`,
  ...over,
});

const two = [candidate(a, { servers: 3, version: "1.2.3" }), candidate(b, { sessions: 2 })];

const harness = (over: Record<string, unknown> = {}) => {
  const asserted: unknown[] = [];
  const applied: { target: Target; scope: PickerScope }[] = [];
  const deps = {
    sessionID: "s1",
    candidates: two,
    current: a,
    index: 1,
    scope: "indefinite" as PickerScope,
    assertPermission: async (input: never) => void asserted.push(input),
    applyChoice: async (target: Target, scope: PickerScope) => {
      applied.push({ target, scope });
      return "Remembered indefinitely for this directory.";
    },
    ...over,
  };
  return { deps, asserted, applied };
};

describe("picker menu (the DialogSelect equivalent)", () => {
  test("options carry index, label, config path and the target key", () => {
    const options = buildPickerOptions(two, a);
    expect(options.length).toBe(2);
    expect(options[1]).toEqual(expect.objectContaining({ index: 1, value: targetKey(b), current: false }));
    expect(options[1]!.description).toContain("/cfg/b.sock.json");
    expect(options[0]!.description).toContain("3 servers");
    expect(options[0]!.description).toContain("1.2.3");
  });

  test("the daemon in use is the marked current option", () => {
    const options = buildPickerOptions(two, b);
    expect(options.map((o) => o.current)).toEqual([false, true]);
    const menu = renderPickerMenu(two, b);
    expect(menu).toContain("* [1]");
    expect(menu).toContain("mcpx_daemon_pick");
  });

  test("no current daemon marks nothing; no candidates says so", () => {
    expect(buildPickerOptions(two).every((o) => !o.current)).toBe(true);
    expect(renderPickerMenu([])).toContain("No mcpx daemon is running");
  });
});

describe("picker selection (permission-gated, storage-backed apply)", () => {
  test("a confirmed selection asserts then applies, and the apply persists the picked target", async () => {
    const { deps, asserted, applied } = harness();
    const out = await runPickerSelection(deps);
    expect(out.applied).toBe(true);
    if (!out.applied) throw new Error("unreachable");
    expect(out.candidate.target).toEqual(b);
    expect(out.text).toContain("Now using");
    // The apply the tool performs is the storage-backed one; the
    // recorded call is the persistence oracle for this core.
    expect(applied).toEqual([{ target: b, scope: "indefinite" }]);
    expect(asserted).toEqual([
      expect.objectContaining({
        sessionID: "s1",
        action: "mcpx_daemon_select",
        resources: [targetKey(b)],
        save: [targetKey(b)],
        metadata: expect.objectContaining({ remember: "indefinite", picker: true }),
      }),
    ]);
  });

  test("cancellation applies nothing and reports the choice unchanged", async () => {
    const { deps, applied } = harness({
      assertPermission: async () => {
        throw new Error("user rejected");
      },
    });
    const out = await runPickerSelection(deps);
    expect(out.applied).toBe(false);
    if (out.applied) throw new Error("unreachable");
    expect(out.reason).toBe("cancelled");
    expect(out.text).toContain("not changed");
    expect(out.text).toContain(targetKey(a));
    expect(applied).toEqual([]);
  });

  test("an invalid index applies nothing and never asserts", async () => {
    const { deps, asserted, applied } = harness({ index: 7 });
    const out = await runPickerSelection(deps);
    expect(out.applied).toBe(false);
    if (out.applied) throw new Error("unreachable");
    expect(out.reason).toBe("invalid_index");
    expect(out.text).toContain("0..1");
    expect(asserted).toEqual([]);
    expect(applied).toEqual([]);
  });

  test("with no permission domain the picker refuses, exactly like daemon_select", async () => {
    const { deps, applied } = harness({ assertPermission: undefined });
    const out = await runPickerSelection(deps);
    expect(out.applied).toBe(false);
    if (out.applied) throw new Error("unreachable");
    expect(out.reason).toBe("no_permission");
    expect(applied).toEqual([]);
  });

  test("no candidates is a picker outcome, not an error", async () => {
    const { deps, applied } = harness({ candidates: [] });
    const out = await runPickerSelection(deps);
    expect(out.applied).toBe(false);
    if (out.applied) throw new Error("unreachable");
    expect(out.reason).toBe("no_candidates");
    expect(applied).toEqual([]);
  });

  test("a picker error from apply propagates without a false success", async () => {
    const { deps } = harness({
      applyChoice: async () => {
        throw new Error("storage unavailable");
      },
    });
    await expect(runPickerSelection(deps)).rejects.toThrow("storage unavailable");
  });
});
