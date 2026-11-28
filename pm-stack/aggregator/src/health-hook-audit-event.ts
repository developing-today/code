import type { NewAuditEvent } from "./events.ts";
import type { StackHealthInput } from "./stack-health.ts";

/** Map an authenticated health-hook intake into the append-only audit stream. */
export function healthHookAuditEvent(input: {
  title: string;
  source: string;
  stackHealth: StackHealthInput;
}): NewAuditEvent | null {
  if (input.source !== "health-hook") return null;
  const { serviceId, state, since } = input.stackHealth;
  return {
    id: `health-hook/${serviceId}/${state}/${since}`,
    at: since,
    actor: "health-hook",
    kind: "health-hook-incident",
    ref: `health-hook:${serviceId}`,
    summary: input.title,
    attribution: { confidence: "unknown", asserted: false },
    source: { identity: `health-hook/${serviceId}` },
  };
}
