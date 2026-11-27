import { describe, expect, it } from "vitest";
import { authenticateMcpAgent, parseAgentTokenConfig } from "../src/mcp_auth";

const OPENCLAW = "openclaw-token-1234567890abcdef";
const MUSE = "muse-lane-token-1234567890abcdef";

describe("Atlas MCP agent authentication", () => {
  it("parses a non-empty map of stable identities to distinct bearer secrets", () => {
    expect(parseAgentTokenConfig(JSON.stringify({ openclaw: OPENCLAW, "muse-lane": MUSE }))).toEqual({
      openclaw: OPENCLAW,
      "muse-lane": MUSE,
    });
  });

  it("rejects malformed maps, unsafe identities, short tokens, and duplicate credentials", () => {
    for (const raw of [
      "not-json",
      "[]",
      "{}",
      JSON.stringify({ "bad identity": OPENCLAW }),
      '{"__proto__":"openclaw-token-1234567890abcdef"}',
      JSON.stringify({ openclaw: "short" }),
      JSON.stringify({ openclaw: OPENCLAW, muse: OPENCLAW }),
    ]) {
      expect(parseAgentTokenConfig(raw)).toBeNull();
    }
  });

  it("derives the caller identity from its token and disables the shared fallback when configured", async () => {
    const config = JSON.stringify({ openclaw: OPENCLAW, "muse-lane": MUSE });
    expect(await authenticateMcpAgent(`Bearer ${OPENCLAW}`, "shared-gate-token-123456", config)).toBe("openclaw");
    expect(await authenticateMcpAgent(`Bearer ${MUSE}`, "shared-gate-token-123456", config)).toBe("muse-lane");
    expect(
      await authenticateMcpAgent("Bearer shared-gate-token-123456", "shared-gate-token-123456", config),
    ).toBeNull();
    expect(await authenticateMcpAgent("Bearer forged-token-123456789", "shared-gate-token-123456", config)).toBeNull();
  });

  it("uses the compatibility identity only when no per-agent map is configured", async () => {
    expect(await authenticateMcpAgent("Bearer shared-gate-token-123456", "shared-gate-token-123456")).toBe(
      "hosted-per-user",
    );
    expect(
      await authenticateMcpAgent("Bearer shared-gate-token-123456", "shared-gate-token-123456", "bad-json"),
    ).toBeNull();
  });
});
