/**
 * The v2 permission gate's evaluate hook (#1858 SET-REMAINDER item (2)).
 *
 *   bun test plugin/opencode/v2/permission.test.ts
 */
import { describe, expect, test } from "bun:test";
import {
  DAEMON_SELECT_ACTION,
  evaluateMcpxPermission,
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
