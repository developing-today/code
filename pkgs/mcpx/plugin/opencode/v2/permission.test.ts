/**
 * The v2 permission gate's evaluate hook (#1858 SET-REMAINDER item (2)).
 *
 *   bun test plugin/opencode/v2/permission.test.ts
 */
import { describe, expect, test } from "bun:test";
import {
  DAEMON_SELECT_ACTION,
  evaluateMcpxPermission,
  wirePermissionGate,
  type PermissionAssertInput,
  type PermissionDomainLike,
  type PermissionEvaluationLike,
} from "./permission.ts";

const event = (
  over: Partial<PermissionEvaluationLike> = {},
): PermissionEvaluationLike => ({
  sessionID: "s1",
  action: DAEMON_SELECT_ACTION,
  resources: ["unix:///tmp/m.sock"],
  effect: "allow",
  ...over,
});

describe("evaluateMcpxPermission", () => {
  test("daemon_select always asks, even under a blanket allow rule", () => {
    const ev = event();
    evaluateMcpxPermission(ev);
    expect(ev.effect).toBe("ask");
  });

  test("a computed deny is left alone", () => {
    const ev = event({ effect: "deny" });
    evaluateMcpxPermission(ev);
    expect(ev.effect).toBe("deny");
  });

  test("other actions pass through unchanged", () => {
    const ev = event({ action: "bash", effect: "allow" });
    evaluateMcpxPermission(ev);
    expect(ev.effect).toBe("allow");
  });
});

describe("wirePermissionGate (fail-closed wiring)", () => {
  const fakeDomain = (
    over: Partial<PermissionDomainLike> = {},
  ): PermissionDomainLike & { hooks: string[]; asserted: PermissionAssertInput[] } => {
    const hooks: string[] = [];
    const asserted: PermissionAssertInput[] = [];
    const base: PermissionDomainLike = {
      hook: async (name) => void hooks.push(name),
      assert: async (input) => void asserted.push(input),
    };
    return { ...base, ...over, hooks, asserted };
  };

  test("successful registration hands out an assert that forwards", async () => {
    const d = fakeDomain();
    const assert = await wirePermissionGate(d);
    expect(d.hooks).toEqual(["evaluate"]);
    expect(assert).toBeDefined();
    await assert!({
      sessionID: "s1",
      action: DAEMON_SELECT_ACTION,
      resources: ["unix:///tmp/m.sock"],
    });
    expect(d.asserted.length).toBe(1);
  });

  test("a rejected registration wires nothing — daemon_select stays refused", async () => {
    const d = fakeDomain({
      hook: async () => {
        throw new Error("host refused the hook");
      },
    });
    expect(await wirePermissionGate(d)).toBeUndefined();
  });

  test("no permission domain wires nothing", async () => {
    expect(await wirePermissionGate(undefined)).toBeUndefined();
  });
});
