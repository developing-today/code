// Stack-health state (#8568, notifications Phase 1): the sentinel
// (scripts/stack-sentinel.ts) reports service transitions through the
// alert throat (POST /api/alerts with a `stackHealth` extension, source
// "stack-sentinel"); the intake writes the latest state per service
// here. The notification engine treats the sentinel as authoritative
// for its service set: when a sentinel record exists for a service, the
// health-page-derived stack-health candidate for that service is
// suppressed (see src/notifications.ts).
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";

/** Service ids the stack sentinel covers — its word overrides /health. */
export const SENTINEL_SERVICE_IDS = [
  "aggregator",
  "projector",
  "kaneo",
  "dispatch",
  "opencode",
  "queue-broker",
] as const;
export type SentinelServiceId = (typeof SENTINEL_SERVICE_IDS)[number];

export type StackHealthState = "down" | "up";

export interface StackHealthRecord {
  serviceId: string;
  name: string;
  state: StackHealthState;
  /** When the service entered this state (ISO), as reported. */
  since: string;
  detail?: string;
  /** When this store last recorded the service (ISO). */
  updatedAt: string;
}

export interface StackHealthInput {
  serviceId: string;
  name: string;
  state: StackHealthState;
  since: string;
  detail?: string;
}

/** Default state path (mirrors src/config.ts stackHealthFile). */
export function defaultStackHealthFile(env: Record<string, string | undefined> = process.env): string {
  const v = env.PM_AGG_STACK_HEALTH_FILE?.trim();
  return v ? v : join(homedir(), ".local", "state", "pm-aggregator", "stack-health.json");
}

/** Validate the intake `stackHealth` extension object (#8568 contract). */
export function validateStackHealth(
  value: unknown,
): { ok: true; value: StackHealthInput } | { ok: false; error: string } {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return { ok: false, error: "stackHealth must be an object" };
  }
  const b = value as Record<string, unknown>;
  const serviceId = typeof b.serviceId === "string" ? b.serviceId.trim() : "";
  if (!serviceId) return { ok: false, error: "stackHealth.serviceId required" };
  const name = typeof b.name === "string" ? b.name.trim() : "";
  if (!name) return { ok: false, error: "stackHealth.name required" };
  if (b.state !== "down" && b.state !== "up") {
    return { ok: false, error: 'stackHealth.state must be "down" | "up"' };
  }
  const since = typeof b.since === "string" ? b.since.trim() : "";
  if (!since) return { ok: false, error: "stackHealth.since required" };
  if (b.detail !== undefined && typeof b.detail !== "string") {
    return { ok: false, error: "stackHealth.detail must be a string" };
  }
  return {
    ok: true,
    value: {
      serviceId,
      name,
      state: b.state,
      since,
      ...(typeof b.detail === "string" ? { detail: b.detail } : {}),
    },
  };
}

interface StackHealthFileShape {
  services: Record<string, StackHealthRecord>;
}

async function readStackHealthFile(file: string): Promise<StackHealthFileShape> {
  try {
    const parsed: unknown = JSON.parse(await readFile(file, "utf8"));
    if (
      parsed &&
      typeof parsed === "object" &&
      (parsed as StackHealthFileShape).services &&
      typeof (parsed as StackHealthFileShape).services === "object"
    ) {
      return parsed as StackHealthFileShape;
    }
    return { services: {} };
  } catch {
    return { services: {} }; // absent/corrupt reads as no sentinel records
  }
}

let stackHealthWriteSeq = 0;
const stackHealthChains = new Map<string, Promise<unknown>>();

function serializeStackHealth<T>(file: string, fn: () => Promise<T>): Promise<T> {
  const prev = stackHealthChains.get(file) ?? Promise.resolve();
  const run = prev.then(fn, fn);
  stackHealthChains.set(
    file,
    run.catch(() => {}),
  );
  return run;
}

/** Record one service's state; returns the previous state (or null). */
export async function setStackHealth(
  file: string,
  input: StackHealthInput,
): Promise<{ previous: StackHealthState | null; record: StackHealthRecord }> {
  return serializeStackHealth(file, async () => {
    const data = await readStackHealthFile(file);
    const previous = data.services[input.serviceId]?.state ?? null;
    const record: StackHealthRecord = {
      serviceId: input.serviceId,
      name: input.name,
      state: input.state,
      since: input.since,
      ...(input.detail !== undefined ? { detail: input.detail } : {}),
      updatedAt: new Date().toISOString(),
    };
    data.services[input.serviceId] = record;
    await mkdir(dirname(file), { recursive: true });
    const tmp = `${file}.tmp-${process.pid}-${++stackHealthWriteSeq}`;
    await writeFile(tmp, JSON.stringify(data, null, 2));
    await rename(tmp, file);
    return { previous, record };
  });
}

/** The full sentinel state map, keyed by service id. */
export async function readStackHealthStates(file: string): Promise<Record<string, StackHealthRecord>> {
  return (await readStackHealthFile(file)).services;
}

/** Services the sentinel currently reports down. */
export async function listStackHealthDown(file: string): Promise<StackHealthRecord[]> {
  const services = await readStackHealthStates(file);
  return Object.values(services)
    .filter((s) => s.state === "down")
    .sort((a, b) => a.serviceId.localeCompare(b.serviceId));
}
