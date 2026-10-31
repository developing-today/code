// Request validation helpers: priority mapping, model parsing, and the
// body validators for POST /v1/sessions and POST /v1/tasks. Pure functions —
// unit-tested without a server.

export class ValidationError extends Error {
  status = 400;
}

/** The queue's design priority scale (repo-queue.py PRIORITY_SCALE).
 * Lower claims sooner: 10 critical, 20 high, 30-50 normal, 70-90 low. */
export const PRIORITY_SCALE = [10, 20, 30, 40, 50, 70, 90] as const;

export const PRIORITY_NAMES: Record<string, number> = {
  critical: 10,
  high: 20,
  normal: 40,
  low: 70,
};

const DEFAULT_PRIORITY = 40;

/**
 * Resolve a request priority (number 10-90, a scale name, or absent) to a
 * value on the queue's design scale. Off-scale integers in range snap to
 * the nearest scale value (ties snap down); anything else is a 400.
 */
export function resolvePriority(input: unknown): number {
  if (input === undefined || input === null) return DEFAULT_PRIORITY;
  if (typeof input === "string") {
    // Own-key guard (#2757, Phase 3 of #2792): PRIORITY_NAMES is a plain
    // object, so an unguarded lookup returns inherited members for
    // prototype spellings - 'constructor' yielded the Object constructor
    // function and '__proto__' Object.prototype, both non-numbers that
    // then flowed toward Packet.priority / p_priority. Object.hasOwn
    // restricts the lookup to the four real names; every other string
    // takes the same documented 400 as any unknown name.
    const key = input.trim().toLowerCase();
    if (!Object.hasOwn(PRIORITY_NAMES, key)) {
      throw new ValidationError(
        `priority: unknown name ${JSON.stringify(input)} (use critical|high|normal|low or a number 10-90)`,
      );
    }
    return PRIORITY_NAMES[key];
  }
  if (typeof input === "number" && Number.isInteger(input)) {
    if ((PRIORITY_SCALE as readonly number[]).includes(input)) return input;
    if (input >= 10 && input <= 90) {
      let best: number = PRIORITY_SCALE[0];
      for (const v of PRIORITY_SCALE) {
        if (Math.abs(v - input) < Math.abs(best - input)) best = v;
      }
      return best;
    }
  }
  throw new ValidationError(`priority: must be a name (critical|high|normal|low) or an integer 10-90`);
}

export interface ModelRef {
  providerID: string;
  id: string;
}

/**
 * Parse a model hint: "provider/model" (split on the FIRST slash, so model
 * ids may themselves contain slashes) or an object {providerID, id}
 * ({provider, model} also accepted). Returns null when absent.
 */
export function parseModel(input: unknown): ModelRef | null {
  if (input === undefined || input === null) return null;
  if (typeof input === "string") {
    const t = input.trim();
    const slash = t.indexOf("/");
    if (slash <= 0 || slash === t.length - 1) {
      throw new ValidationError(`model: string form must be "provider/model", got ${JSON.stringify(input)}`);
    }
    return { providerID: t.slice(0, slash), id: t.slice(slash + 1) };
  }
  if (typeof input === "object") {
    const o = input as Record<string, unknown>;
    const providerID = o.providerID ?? o.provider;
    const id = o.id ?? o.model;
    if (typeof providerID === "string" && providerID.trim() && typeof id === "string" && id.trim()) {
      return { providerID: providerID.trim(), id: id.trim() };
    }
  }
  throw new ValidationError(`model: must be "provider/model" or {providerID, id}`);
}

export const TASK_KINDS = ["pr_shepherd", "triage", "small_fix", "ci_investigation"] as const;
export type TaskKind = (typeof TASK_KINDS)[number];

function reqString(body: Record<string, unknown>, field: string): string {
  const v = body[field];
  if (typeof v !== "string" || !v.trim()) {
    throw new ValidationError(`${field}: required non-empty string`);
  }
  return v.trim();
}

function optString(body: Record<string, unknown>, field: string): string | undefined {
  const v = body[field];
  if (v === undefined || v === null) return undefined;
  if (typeof v !== "string") throw new ValidationError(`${field}: must be a string`);
  return v;
}

function optBool(body: Record<string, unknown>, field: string, dflt: boolean): boolean {
  const v = body[field];
  if (v === undefined || v === null) return dflt;
  if (typeof v !== "boolean") throw new ValidationError(`${field}: must be a boolean`);
  return v;
}

export interface SessionRequest {
  repo: string;
  prompt: string;
  title?: string;
  model: ModelRef | null;
  directory?: string;
  priority: number;
  issue: boolean;
  wait: boolean;
}

export function parseSessionRequest(body: unknown): SessionRequest {
  if (typeof body !== "object" || body === null) throw new ValidationError("body: must be a JSON object");
  const b = body as Record<string, unknown>;
  return {
    repo: reqString(b, "repo"),
    prompt: reqString(b, "prompt"),
    title: optString(b, "title"),
    model: parseModel(b.model),
    directory: optString(b, "directory"),
    priority: resolvePriority(b.priority),
    issue: optBool(b, "issue", true),
    wait: optBool(b, "wait", true),
  };
}

/** Session-directory scoping for POST /v1/sessions (issue #2903).
 * OIDC callers (workflows in allowlisted repos) may only run inside the
 * target repo's laptop primary clone: directory omitted (default) or
 * exactly the repo's scoped directory. Anything else is rejected BEFORE
 * any issue/task/session is created. Bearer callers hold the laptop
 * secret, so they keep the free-form directory — a deliberate choice,
 * documented in the README. */
export function resolveSessionDirectory(
  callerType: "oidc" | "bearer",
  repo: string,
  directory: string | undefined,
  repoDirs: Record<string, string>,
): string {
  const scoped = repoDirs[repo];
  if (directory === undefined) return scoped!;
  if (callerType === "bearer") return directory;
  const norm = (d: string) => (d.length > 1 ? d.replace(/\/+$/, "") : d);
  if (scoped === undefined || norm(directory) !== norm(scoped)) {
    throw new ValidationError(
      `directory: OIDC callers may only omit directory or use the repo clone (${scoped ?? "unknown"})`,
    );
  }
  return scoped;
}

export interface TaskRequest {
  repo: string;
  title: string;
  prompt: string;
  kind: TaskKind;
  priority: number;
  model: ModelRef | null;
  issue: boolean;
  issueNumber?: number;
  dedupeKey?: string;
}

/** Optional `issue_number`: when present it must be a positive integer —
 * the existing issue in `repo` the task attaches to instead of creating
 * a new one. */
export function parseIssueNumber(input: unknown): number | undefined {
  if (input === undefined || input === null) return undefined;
  if (typeof input !== "number" || !Number.isInteger(input) || input <= 0) {
    throw new ValidationError(`issue_number: must be a positive integer`);
  }
  return input;
}

export function parseTaskRequest(body: unknown): TaskRequest {
  if (typeof body !== "object" || body === null) throw new ValidationError("body: must be a JSON object");
  const b = body as Record<string, unknown>;
  const promptRaw = b.prompt ?? b.body;
  if (typeof promptRaw !== "string" || !promptRaw.trim()) {
    throw new ValidationError("prompt (or body): required non-empty string");
  }
  let kind: TaskKind = "triage";
  if (b.kind !== undefined && b.kind !== null) {
    if (typeof b.kind !== "string" || !(TASK_KINDS as readonly string[]).includes(b.kind)) {
      throw new ValidationError(`kind: must be one of ${TASK_KINDS.join("|")}`);
    }
    kind = b.kind as TaskKind;
  }
  return {
    repo: reqString(b, "repo"),
    title: reqString(b, "title"),
    prompt: promptRaw.trim(),
    kind,
    priority: resolvePriority(b.priority),
    model: parseModel(b.model),
    issue: optBool(b, "issue", false),
    issueNumber: parseIssueNumber(b.issue_number),
    dedupeKey: optString(b, "dedupe_key"),
  };
}

/** First line of a prompt, trimmed and length-capped — the fallback title. */
export function titleFromPrompt(prompt: string, max = 120): string {
  const first =
    prompt
      .split("\n")
      .map((l) => l.trim())
      .find((l) => l.length > 0) ?? "dispatch request";
  return first.length > max ? first.slice(0, max - 1) + "…" : first;
}
