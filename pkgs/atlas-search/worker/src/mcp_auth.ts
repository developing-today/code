/**
 * Authenticated identities for the Atlas MCP surface (#8625).
 * AGENT_TOKENS is an optional secret JSON object mapping stable agent ids
 * to bearer tokens. When configured, only those tokens are accepted on
 * /mcp; the shared GATE_TOKEN fallback is retained only for deployments
 * that have not configured per-agent credentials yet.
 */
const AGENT_ID = /^[A-Za-z0-9._:-]{1,128}$/;
const MIN_TOKEN_LENGTH = 16;

export type AgentTokenConfig = Record<string, string>;

/** Parse and validate the secret allowlist; malformed config fails closed. */
export function parseAgentTokenConfig(raw: string): AgentTokenConfig | null {
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    return null;
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) return null;

  const entries = Object.entries(value as Record<string, unknown>);
  if (entries.length === 0) return null;
  const seenTokens = new Set<string>();
  const config: AgentTokenConfig = {};
  for (const [agentId, token] of entries) {
    if (
      !AGENT_ID.test(agentId) ||
      agentId === "__proto__" ||
      typeof token !== "string" ||
      token.length < MIN_TOKEN_LENGTH
    ) return null;
    if (seenTokens.has(token)) return null;
    seenTokens.add(token);
    config[agentId] = token;
  }
  return config;
}

async function equalSecret(a: string, b: string): Promise<boolean> {
  const encoder = new TextEncoder();
  const [left, right] = await Promise.all([
    crypto.subtle.digest("SHA-256", encoder.encode(a)),
    crypto.subtle.digest("SHA-256", encoder.encode(b)),
  ]);
  const x = new Uint8Array(left);
  const y = new Uint8Array(right);
  let diff = 0;
  for (let i = 0; i < x.length; i++) diff |= x[i] ^ y[i];
  return diff === 0;
}

/**
 * Resolve the MCP caller from credentials, never from a caller-supplied
 * identity header or request field. A configured per-agent allowlist
 * disables the legacy shared-token fallback.
 */
export async function authenticateMcpAgent(
  authorization: string | null,
  gateToken: string,
  agentTokens?: string,
): Promise<string | null> {
  const presented = authorization?.startsWith("Bearer ") ? authorization.slice(7) : "";
  if (!presented) return null;

  if (agentTokens !== undefined) {
    const parsed = parseAgentTokenConfig(agentTokens);
    if (!parsed) return null;
    let matched: string | null = null;
    for (const [agentId, token] of Object.entries(parsed)) {
      if (await equalSecret(presented, token)) matched = agentId;
    }
    return matched;
  }

  return gateToken && (await equalSecret(presented, gateToken)) ? "hosted-per-user" : null;
}
