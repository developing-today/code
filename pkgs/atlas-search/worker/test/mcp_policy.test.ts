import { describe, expect, it } from "vitest";
import { ceilingForAgent, parseAgentPolicy } from "../src/mcp_policy";
import { CEILING } from "../src/space";

describe("Atlas MCP per-agent ceiling policy (#8625)", () => {
  it("parses a map of stable identities to ceilings at or below the surface pin", () => {
    expect(parseAgentPolicy(JSON.stringify({ openclaw: { ceiling: "shareable" } }))).toEqual({
      openclaw: { ceiling: "shareable" },
    });
    expect(parseAgentPolicy(JSON.stringify({ "muse-lane": { ceiling: "personal" } }))).toEqual({
      "muse-lane": { ceiling: "personal" },
    });
    expect(parseAgentPolicy("{}")).toEqual({});
  });

  it("fails closed on malformed policy: non-objects, bad identities, unknown or above-pin ceilings", () => {
    for (const raw of [
      "not-json",
      "[]",
      "3",
      '"x"',
      '{"openclaw":"shareable"}',
      '{"openclaw":null}',
      '{"openclaw":[]}',
      '{"openclaw":{}}',
      '{"openclaw":{"ceiling":"everyone"}}',
      '{"openclaw":{"ceiling":"public"}}',
      '{"openclaw":{"ceiling":"local-only"}}',
      '{"openclaw":{"ceiling":"secret-bearing"}}',
      '{"bad identity":{"ceiling":"shareable"}}',
      '{"__proto__":{"ceiling":"shareable"}}',
    ]) {
      expect(parseAgentPolicy(raw), raw).toBeNull();
    }
  });

  it("resolves an agent's ceiling, defaulting to the surface pin", () => {
    const table = parseAgentPolicy(JSON.stringify({ openclaw: { ceiling: "shareable" } }))!;
    expect(ceilingForAgent(table, "openclaw")).toBe("shareable");
    // Unknown agents search under the pin — never above it.
    expect(ceilingForAgent(table, "muse-lane")).toBe(CEILING);
    expect(ceilingForAgent(table, "muse-lane")).toBe("personal");
  });
});
