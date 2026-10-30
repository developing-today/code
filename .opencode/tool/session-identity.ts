// OpenCode custom tool: `session-identity` (#906 §3).
//
// Returns the calling session's identity — its stable session ID, its
// parent session ID (for sub-sessions), the agent name it runs as, and
// its directory — as JSON the agent can stamp onto work products,
// dashboard comments, and dispatches without having to remember or
// transcribe anything. The aggregator's identity registry keys on
// exactly these ids.
//
// Install (per host, then restart the OpenCode server):
//   cp session-identity.ts ~/.config/opencode/tool/
// (project-local installs go in <project>/.opencode/tool/ instead).
// The tool is read-only and fail-soft: if the server API is not
// reachable for the parent lookup, parentId comes back null and the
// registry resolves the parent from its own session records instead.
import { tool } from "@opencode-ai/plugin";

export interface SessionIdentity {
  sessionId: string;
  parentId: string | null;
  agent: string | null;
  directory: string | null;
  worktree: string | null;
}

/** The identity payload for one tool invocation (pure; unit-tested). */
export function identityPayload(
  context: { sessionID?: string; agent?: string; directory?: string; worktree?: string },
  parentId: string | null,
): SessionIdentity {
  return {
    sessionId: context.sessionID ?? "unknown",
    parentId,
    agent: context.agent ?? null,
    directory: context.directory ?? null,
    worktree: context.worktree ?? null,
  };
}

/** Best-effort parent lookup against the server's session list. */
async function lookupParentId(sessionId: string): Promise<string | null> {
  try {
    const port = process.env.OPENCODE_PORT;
    const base = process.env.OPENCODE_BASE_URL ?? (port ? `http://127.0.0.1:${port}` : null);
    if (!base) return null;
    const headers: Record<string, string> = {};
    const pw = process.env.OPENCODE_SERVER_PASSWORD;
    if (pw) headers.Authorization = `Basic ${Buffer.from(`opencode:${pw}`).toString("base64")}`;
    const res = await fetch(`${base}/api/session?limit=200`, {
      headers,
      signal: AbortSignal.timeout(3000),
    });
    if (!res.ok) return null;
    const body = (await res.json()) as { data?: { id: string; parentID?: string | null }[] };
    const me = (body.data ?? []).find((s) => s.id === sessionId);
    return me?.parentID ?? null;
  } catch {
    return null;
  }
}

export default tool({
  description:
    "Return this session's identity (session id, parent session id, agent, directory) as JSON, for stamping work products, dashboard comments, and dispatches. Read-only.",
  args: {},
  async execute(_args, context) {
    const parentId = context.sessionID ? await lookupParentId(context.sessionID) : null;
    return JSON.stringify(identityPayload(context, parentId), null, 2);
  },
});
