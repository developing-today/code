/**
 * The system-prompt paragraph, via session.hook("context") (design doc
 * §2.2). The hook fires per turn, so the append is idempotent: the paragraph
 * carries a marker and is added only when no existing part contains it.
 */
export const CONTEXT_MARKER = "mcpx:session-context";

export const CONTEXT_PARAGRAPH =
  `[${CONTEXT_MARKER}] mcpx leases MCP servers per session through a local ` +
  `daemon; the mcpx_* tools and shell environment already know this session's ` +
  `identity, so never pass a session id to a tool or ask the user for one.`;

export type SystemPart = { type?: string; text?: string } | string;

export type ContextHookInput = {
  sessionID: string;
  system: SystemPart[];
};

/** Append the paragraph once; returns true when it appended. */
export function appendContextOnce(input: ContextHookInput): boolean {
  const already = input.system.some((part) =>
    (typeof part === "string" ? part : (part.text ?? "")).includes(CONTEXT_MARKER),
  );
  if (already) return false;
  input.system.push({ type: "text", text: CONTEXT_PARAGRAPH });
  return true;
}
