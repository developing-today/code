// The agent/session identity registry (#906 §3). Pure builders + merge:
// the collector assembles records from the sources it can read (sandbox
// OpenCode sessions, the Jules keeper state, Muse launch
// records), the fleet feed carries them, and the aggregator merges them
// with the laptop's live sessions into a durable store — so a session
// stays in the registry after it expires out of the attention feed.
//
// Loose v1 by design: identity is (source, session id); task linkage is
// whatever refs the session's title/PR/branch or launch record names.
import { type SessionFeedEntry, type SessionRecord, refsInText } from "./model.ts";

/** Agent identity for an OpenCode session entry. */
export function agentIdentityOf(e: SessionFeedEntry): { agentId: string; agentLabel: string } {
  if (e.source === "jules") return { agentId: "jules", agentLabel: "Jules" };
  if (e.source === "claude-code") return { agentId: "claude-code", agentLabel: "Muse" };
  const server = e.source === "opencode-laptop" ? "laptop" : "sandbox";
  if (e.agent) return { agentId: `opencode:${e.agent}`, agentLabel: `OpenCode · ${e.agent} (${server})` };
  return { agentId: "opencode:main", agentLabel: `OpenCode · main (${server})` };
}

/** Task refs a session is loosely linked to: its PR, then its title. */
export function taskRefsOf(e: SessionFeedEntry): string[] {
  const refs = new Set<string>();
  if (e.pr) for (const r of refsInText(e.pr)) refs.add(r);
  if (e.title) for (const r of refsInText(e.title)) refs.add(r);
  if (e.branch) for (const r of refsInText(e.branch)) refs.add(r);
  return [...refs];
}

/** A registry record from one session feed entry. */
export function recordFromSessionEntry(e: SessionFeedEntry, tokensTotal?: number): SessionRecord {
  const { agentId, agentLabel } = agentIdentityOf(e);
  const status = e.outcome
    ? e.outcome
    : (e.activeToolCalls ?? 0) > 0
      ? "in-progress"
      : e.awaitingQuestion
        ? "waiting"
        : "idle";
  return {
    id: e.sessionId,
    source: e.source ?? "opencode-sandbox",
    label: e.title ?? e.sessionId,
    agentId,
    agentLabel,
    parentId: e.parentId ?? null,
    startedAt: e.createdAt,
    lastSeenAt: e.lastActivityAt ?? undefined,
    status,
    model: e.model,
    title: e.title,
    directory: e.directory,
    tokensTotal,
    taskRefs: taskRefsOf(e),
  };
}

/** One line of the Muse launch-record JSONL (appended at spawn). */
export interface ClaudeLaunchRecord {
  id: string;
  label?: string;
  parentId?: string | null;
  startedAt?: string;
  lastSeenAt?: string;
  status?: string;
  model?: string;
  tokensTotal?: number;
  taskRefs?: string[];
  directory?: string;
}

/** A registry record from a Muse launch record. */
export function recordFromLaunch(l: ClaudeLaunchRecord): SessionRecord {
  return {
    id: l.id,
    source: "claude-code",
    label: l.label ?? l.id,
    agentId: "claude-code",
    agentLabel: "Muse",
    parentId: l.parentId ?? null,
    startedAt: l.startedAt,
    lastSeenAt: l.lastSeenAt ?? l.startedAt,
    status: l.status ?? "launched",
    model: l.model,
    title: l.label,
    directory: l.directory,
    tokensTotal: l.tokensTotal,
    taskRefs: l.taskRefs ?? [],
  };
}

/** Parse launch-record JSONL; malformed lines are skipped, never fatal. */
export function parseLaunchRecords(jsonl: string): SessionRecord[] {
  const out: SessionRecord[] = [];
  for (const line of jsonl.split("\n")) {
    const t = line.trim();
    if (!t) continue;
    try {
      const parsed: unknown = JSON.parse(t);
      if (typeof parsed === "object" && parsed !== null && typeof (parsed as ClaudeLaunchRecord).id === "string") {
        // #2376: route through the same hygiene filter as the store/feed
        // paths — a launch record carrying a non-string lastSeenAt or
        // startedAt must not reach mergeRecords' string-only sort.
        const rec = recordFromLaunch(parsed as ClaudeLaunchRecord);
        if (isSessionRecord(rec)) out.push(rec);
      }
    } catch {
      /* a torn final line must not lose the registry */
    }
  }
  return out;
}

/**
 * Merge incoming records into the stored set, keyed by (source, id).
 * Tokens are cumulative, so the max wins; lastSeenAt keeps the newest;
 * every other field takes the incoming value when present. "Present"
 * means the incoming key holds a real value: undefined-valued keys are
 * skipped (a sparse re-read must not wipe what the store already knew,
 * #1560), an incoming parentId of null is treated as absent (a feed
 * entry that does not state a parent is not a root claim), and the
 * label follows the title — when the incoming title is absent the
 * incoming label is just the session-id fallback, so the stored label
 * is preserved.
 */
export function mergeRecords(existing: SessionRecord[], incoming: SessionRecord[]): SessionRecord[] {
  const byKey = new Map<string, SessionRecord>();
  for (const r of existing) byKey.set(`${r.source}:${r.id}`, r);
  for (const r of incoming) {
    const key = `${r.source}:${r.id}`;
    const prev = byKey.get(key);
    if (!prev) {
      byKey.set(key, r);
      continue;
    }
    const present: Partial<SessionRecord> = {};
    for (const [k, v] of Object.entries(r)) {
      if (v !== undefined) (present as Record<string, unknown>)[k] = v;
    }
    byKey.set(key, {
      ...prev,
      ...present,
      parentId: r.parentId ?? prev.parentId,
      label: r.title !== undefined ? r.label : prev.label,
      startedAt: prev.startedAt ?? r.startedAt,
      tokensTotal: Math.max(prev.tokensTotal ?? 0, r.tokensTotal ?? 0) || undefined,
      taskRefs: [...new Set([...(prev.taskRefs ?? []), ...(r.taskRefs ?? [])])],
      lastSeenAt: (prev.lastSeenAt ?? "") >= (r.lastSeenAt ?? "") ? (prev.lastSeenAt ?? r.lastSeenAt) : r.lastSeenAt,
    });
  }
  return [...byKey.values()].sort((a, b) => (b.lastSeenAt ?? "").localeCompare(a.lastSeenAt ?? ""));
}

/** Structural check for one registry record (feed/store hygiene). */
export function isSessionRecord(v: unknown): v is SessionRecord {
  if (typeof v !== "object" || v === null) return false;
  const r = v as Partial<SessionRecord>;
  // #2376: mergeRecords compares lastSeenAt with string-only methods
  // (localeCompare / >=), so a present-but-non-string timestamp passes
  // this filter today and throws TypeError once the sort reaches it.
  // Timestamps must be string-or-absent; a poisoned record is rejected,
  // not coerced (the #2347 load-filter precedent).
  return (
    typeof r.id === "string" &&
    typeof r.source === "string" &&
    typeof r.label === "string" &&
    (r.lastSeenAt === undefined || typeof r.lastSeenAt === "string") &&
    (r.startedAt === undefined || typeof r.startedAt === "string") &&
    // #2408: mergeRecords spreads taskRefs as an array, so a present-but-
    // non-array taskRefs throws TypeError on the duplicate-key merge (and
    // a string silently spreads into single-character refs). The validator
    // owns the rule, per the #2347/#2376 precedent above: taskRefs must be
    // absent or an array of strings; anything else rejects the record.
    (r.taskRefs === undefined || (Array.isArray(r.taskRefs) && r.taskRefs.every((t) => typeof t === "string")))
  );
}

/** Standing fleet identities + one per registry session, for comment authorship. */
export interface AuthorIdentity {
  id: string;
  label: string;
  kind: "drew" | "muse" | "agent";
  sessionId?: string;
}

export const STANDING_IDENTITIES: AuthorIdentity[] = [
  { id: "drew", label: "Drew", kind: "drew" },
  { id: "muse", label: "Muse", kind: "muse" },
  { id: "orchestrator", label: "Orchestrator (orchestration-pass)", kind: "agent" },
  { id: "keeper", label: "Jules keeper (jules-review)", kind: "agent" },
  { id: "stewardship", label: "Stewardship cycle", kind: "agent" },
];

export function identitiesFromRecords(records: SessionRecord[]): AuthorIdentity[] {
  const sessions: AuthorIdentity[] = records.map((r) => ({
    id: r.id,
    label: `${r.agentLabel} · ${r.label}`,
    kind: "agent" as const,
    sessionId: r.id,
  }));
  return [...STANDING_IDENTITIES, ...sessions];
}
