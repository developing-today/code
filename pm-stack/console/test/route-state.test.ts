// Per-surface route state for the PM front door (#14944, CF-4).
//
// Oracles: the design's own rules (doc/2026-11-24T06-06-29Z_design_pm-
// cloudflare-migration §5.1 + the 2026-11-24 amendment) — a flip is
// visible and clears a fall; reads fall automatically and the fall is
// a fact in the state; writes never auto-fall; state that does not
// read as route state is not route state.
import { describe, expect, test } from "bun:test";
import {
  flipRoute,
  healthRoutes,
  mayAutoFall,
  recordHostedSuccess,
  recordReadFailure,
  routeState,
  servingTier,
} from "../edge/src/route-state.ts";

const AT = Date.parse("2026-11-26T12:00:00Z");

describe("route state reading", () => {
  test("what storage holds is read back; what is not route state is dropped, not guessed", () => {
    expect(routeState(null)).toEqual({});
    expect(routeState("hosted")).toEqual({});
    expect(routeState([])).toEqual({});
    expect(
      routeState({
        board: { tier: "hosted", degraded: true, fellAt: AT },
        pages: { tier: "tunnel" },
        bogus: { tier: "sideways" },
        fallenTunnel: { tier: "tunnel", degraded: true, fellAt: AT },
        noTime: { tier: "hosted", degraded: true, fellAt: "soon" },
      }),
    ).toEqual({
      board: { tier: "hosted", degraded: true, fellAt: AT },
      // A tunnel surface cannot be fallen: the fall is hosted → tunnel.
      fallenTunnel: { tier: "tunnel", degraded: false, fellAt: null },
      pages: { tier: "tunnel", degraded: false, fellAt: null },
    });
  });
});

describe("flips", () => {
  test("a flip declares the tier, clears any fall, and does not change the state it was given", () => {
    const before = routeState({ board: { tier: "hosted", degraded: true, fellAt: AT } });
    const after = flipRoute(before, "board", "tunnel");
    expect(after.board).toEqual({ tier: "tunnel", degraded: false, fellAt: null });
    expect(before.board.degraded).toBe(true);
    expect(flipRoute(after, "board", "hosted").board).toEqual({ tier: "hosted", degraded: false, fellAt: null });
  });

  test("a flip that is not a surface and a tier is refused", () => {
    expect(() => flipRoute({}, "has space", "hosted")).toThrow();
    expect(() => flipRoute({}, "board", "sideways" as never)).toThrow();
    expect(() => flipRoute({}, undefined as never, "hosted")).toThrow();
  });
});

describe("automatic fall (reads only)", () => {
  test("a hosted read failure falls to the tunnel, visibly, once", () => {
    const state = flipRoute({}, "board", "hosted");
    const first = recordReadFailure(state, "board", "GET", AT);
    expect(first.fell).toBe(true);
    expect(first.state.board).toEqual({ tier: "hosted", degraded: true, fellAt: AT });
    expect(servingTier(first.state.board)).toBe("tunnel");
    // Already fallen: a second failure is not a second fall.
    const second = recordReadFailure(first.state, "board", "GET", AT + 1_000);
    expect(second.fell).toBe(false);
    expect(second.state.board.fellAt).toBe(AT);
  });

  test("writes never auto-fall, and neither do tunnel or unknown surfaces", () => {
    const hosted = flipRoute({}, "board", "hosted");
    for (const method of ["POST", "PUT", "PATCH", "DELETE"]) {
      const result = recordReadFailure(hosted, "board", method, AT);
      expect(result.fell).toBe(false);
      expect(result.state.board.degraded).toBe(false);
    }
    expect(mayAutoFall("GET")).toBe(true);
    expect(mayAutoFall("HEAD")).toBe(true);
    expect(mayAutoFall("POST")).toBe(false);
    const tunnel = flipRoute({}, "board", "tunnel");
    expect(recordReadFailure(tunnel, "board", "GET", AT).fell).toBe(false);
    expect(recordReadFailure({}, "board", "GET", AT).fell).toBe(false);
    expect(servingTier(undefined)).toBeNull();
  });

  test("a hosted success stands a fallen surface back up", () => {
    const fallen = recordReadFailure(flipRoute({}, "board", "hosted"), "board", "GET", AT).state;
    const back = recordHostedSuccess(fallen, "board");
    expect(back.recovered).toBe(true);
    expect(back.state.board).toEqual({ tier: "hosted", degraded: false, fellAt: null });
    expect(recordHostedSuccess(back.state, "board").recovered).toBe(false);
  });
});

describe("health view", () => {
  test("health says declared tier, serving tier and fall per surface, by name in order", () => {
    const state = recordReadFailure(
      flipRoute(flipRoute({}, "pages", "tunnel"), "board", "hosted"),
      "board",
      "GET",
      AT,
    ).state;
    expect(healthRoutes(state)).toEqual({
      board: { tier: "hosted", degraded: true, fellAt: AT, serving: "tunnel" },
      pages: { tier: "tunnel", degraded: false, fellAt: null, serving: "tunnel" },
    });
    expect(Object.keys(healthRoutes(state))).toEqual(["board", "pages"]);
    expect(healthRoutes({})).toEqual({});
  });
});
