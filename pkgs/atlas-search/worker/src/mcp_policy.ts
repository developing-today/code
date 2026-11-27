/**
 * Per-agent query ceilings for the MCP surface (#8625 slice:
 * ceilings; corpus scoping is the named follow-on).
 *
 * AGENT_POLICY is an optional JSON object mapping stable agent ids
 * (the same ids AGENT_TOKENS in mcp_auth.ts authenticates) to a
 * per-agent ceiling:
 *
 *   {"openclaw": {"ceiling": "shareable"}, "muse-lane": {"ceiling": "personal"}}
 *
 * The ceiling names the most sensitive tier the agent may receive.
 * Only ceilings at or below this surface's pin (space.ts CEILING,
 * `personal`) are accepted: a policy naming anything higher is
 * malformed and fails closed, because the service would clamp it
 * back to the pin and the verification in service.ts would then
 * refuse every answer. An agent with no entry defaults to the pin.
 *
 * Corpus scoping is deliberately NOT part of this table. There is
 * no corpus dimension on indexed chunks and no corpus selector in
 * the service's branch queries yet, so an allowlist here would be
 * unenforced intent — worse than no allowlist. Scoping lands with
 * the backend corpus-dimension slice, which will extend this table.
 */
import { CEILING, type Ceiling } from "./space";

export interface AgentPolicy {
  ceiling: Ceiling;
}

export type AgentPolicyTable = Record<string, AgentPolicy>;

/** Ceilings this surface may serve, in increasing sensitivity. */
const POLICY_CEILINGS: readonly Ceiling[] = ["shareable", "personal"];

const AGENT_ID = /^[A-Za-z0-9._:-]{1,128}$/;

/** Parse and validate the policy table; malformed config fails closed. */
export function parseAgentPolicy(raw: string): AgentPolicyTable | null {
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    return null;
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) return null;
  const table: AgentPolicyTable = {};
  for (const [agentId, policy] of Object.entries(value as Record<string, unknown>)) {
    if (!AGENT_ID.test(agentId) || agentId === "__proto__") return null;
    if (typeof policy !== "object" || policy === null || Array.isArray(policy)) return null;
    const { ceiling } = policy as { ceiling?: unknown };
    if (typeof ceiling !== "string" || !(POLICY_CEILINGS as readonly string[]).includes(ceiling)) return null;
    table[agentId] = { ceiling: ceiling as Ceiling };
  }
  return table;
}

/**
 * The ceiling one query runs under: the agent's policy ceiling, or
 * the surface pin when the agent has no entry. The ceiling travels
 * with the request (key material, DO body, service call) so the
 * branch queries enforce it and the cache can never serve a result
 * computed above it.
 */
export function ceilingForAgent(table: AgentPolicyTable, agentId: string): Ceiling {
  return table[agentId]?.ceiling ?? CEILING;
}
