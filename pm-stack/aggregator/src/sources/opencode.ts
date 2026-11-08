// Shared mapper: OpenCode v2 server signals -> SessionFeedEntry rows
// (#852). Used by the aggregator itself for the laptop's managed server
// (live, at request time) and by scripts/collect-fleet-feed.ts for the
// sandbox server (which binds its own loopback and cannot be reached
// from the laptop).
//
// Wire shapes are the v2 API's, probed live 2026-10-25 against both
// servers: GET /api/session?limit=N -> { data: Session[] , cursor },
// GET /api/session/active -> { data: { [sessionID]: unknown } },
// GET /api/session/{id}/message?limit=N -> { data: Message[] } (newest
// first), GET /api/permission/request -> { data: PermissionRequest[] },
// GET /api/session/{id}/form -> { data: Form[] }.
import type { SessionFeedEntry, SessionSource } from "../model.ts";
import { GENERATED_AT_FUTURE_SKEW_MS } from "../feed.ts";

export interface OpenCodeTarget {
  baseUrl: string;
  /** Basic-auth password for user "opencode" (by NAME from the caller). */
  password: string;
}

interface V2Session {
  id: string;
  title?: string;
  model?: { id?: string; providerID?: string };
  tokens?: { input?: number; output?: number; reasoning?: number };
  outcome?: string | null;
  /** Present on sub-sessions: the spawning session's id (probed live). */
  parentID?: string | null;
  /** Present on sub-sessions: the subagent name (e.g. "explore"). */
  agent?: string;
  time?: { created?: number; updated?: number; idle?: number };
  location?: { directory?: string };
  subpath?: string;
}

interface V2Part {
  type?: string;
  state?: {
    status?: string;
    input?: { command?: string; [k: string]: unknown };
    error?: unknown;
  };
}

interface V2Message {
  id: string;
  type?: string;
  time?: { created?: number };
  finish?: string | null;
  error?: { type?: string; message?: string } | null;
  content?: V2Part[];
}

interface V2PermissionRequest {
  sessionID?: string;
  sessionId?: string;
  title?: string;
  tool?: string;
  description?: string;
  [k: string]: unknown;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function records(value: unknown): Record<string, unknown>[] {
  return Array.isArray(value) ? value.filter(isRecord) : [];
}

function isV2Session(value: unknown): value is V2Session {
  return isRecord(value) && typeof value.id === "string" && value.id.trim().length > 0;
}

function nonEmptyString(value: unknown): string | undefined {
  return typeof value === "string" && value.trim().length > 0 ? value : undefined;
}

function firstString(...values: unknown[]): string | undefined {
  for (const value of values) {
    const text = nonEmptyString(value);
    if (text !== undefined) return text;
  }
  return undefined;
}

function finiteNumber(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

async function getJson<T>(target: OpenCodeTarget, path: string, timeoutMs = 10_000): Promise<T | null> {
  try {
    const res = await fetch(`${target.baseUrl}${path}`, {
      headers: {
        Authorization: `Basic ${Buffer.from(`opencode:${target.password}`).toString("base64")}`,
      },
      signal: AbortSignal.timeout(timeoutMs),
    });
    if (!res.ok) return null;
    return (await res.json()) as T;
  } catch {
    return null;
  }
}

// JS Dates cover +/-8.64e15 ms; a server-supplied stamp outside
// that range makes toISOString() throw RangeError, and one bad
// session used to drop every session from its server (#2286).
// One shared predicate gates both iso() and the freshness
// inclusion below: finite, positive, in Date range.
const MAX_DATE_MS = 8.64e15;
const validMs = (ms: number | undefined): boolean =>
  typeof ms === "number" && Number.isFinite(ms) && ms > 0 && ms <= MAX_DATE_MS;

const iso = (ms: number | undefined): string | null => (validMs(ms) ? new Date(ms as number).toISOString() : null);

// A stamp more than the aggregator-wide skew tolerance ahead of now is
// "future" (#4180): it earns no freshness inclusion and maps to a null
// lastActivityAt, the same treatment as an invalid stamp, so a
// clock-skewed server cannot pin the attention feed or poison
// lastActivityAt-sorted consumers. Within-skew stamps keep their value.
const beyondSkew = (ms: number | undefined, now: number): boolean =>
  typeof ms === "number" && Number.isFinite(ms) && ms - now > GENERATED_AT_FUTURE_SKEW_MS;

function errText(e: unknown): string | null {
  if (!e) return null;
  if (typeof e === "string") return e;
  if (typeof e === "object") {
    const o = e as { message?: unknown; type?: unknown };
    if (typeof o.message === "string") return o.message;
    if (typeof o.type === "string") return o.type;
  }
  return String(e);
}

/** Sessions updated within this window are in the attention feed. */
export const ATTENTION_WINDOW_MS = 12 * 3_600_000;

export interface MappedSessions {
  entries: SessionFeedEntry[];
  /** Cumulative fresh tokens (input+output+reasoning) per session. */
  tokensBySession: Map<string, number>;
  /** Session ids the server currently reports active. */
  activeIds: Set<string>;
}

/**
 * Map one server's sessions to feed entries. Inclusion (per #890):
 * sessions in the active set, sessions with a pending permission
 * request, and sessions updated within 12 h — but a stamp more than
 * the skew tolerance ahead of now earns no freshness inclusion (#4180).
 */
export async function mapServerSessions(
  target: OpenCodeTarget,
  now = Date.now(),
  opts: { messageLimit?: number; maxDetailed?: number; source?: SessionSource } = {},
): Promise<MappedSessions | null> {
  const list = await getJson<unknown>(target, "/api/session?limit=100");
  if (!isRecord(list) || !Array.isArray(list.data)) return null;
  const sessions = list.data.filter(isV2Session);

  const activeResp = await getJson<unknown>(target, "/api/session/active");
  const activeData = isRecord(activeResp) && isRecord(activeResp.data) ? activeResp.data : {};
  const activeIds = new Set(Object.keys(activeData));

  const permResp = await getJson<unknown>(target, "/api/permission/request");
  const pendingBySession = new Map<string, V2PermissionRequest>();
  const permissionRows = isRecord(permResp) ? records(permResp.data) : [];
  for (const row of permissionRows) {
    const sid = firstString(row.sessionID, row.sessionId);
    if (sid && !pendingBySession.has(sid)) pendingBySession.set(sid, row as V2PermissionRequest);
  }

  const tokensBySession = new Map<string, number>();
  for (const s of sessions) {
    const t = isRecord(s.tokens) ? s.tokens : {};
    tokensBySession.set(s.id, finiteNumber(t.input) + finiteNumber(t.output) + finiteNumber(t.reasoning));
  }

  const included = sessions
    .filter(
      (s) =>
        activeIds.has(s.id) ||
        pendingBySession.has(s.id) ||
        (validMs(s.time?.updated) &&
          now - (s.time?.updated as number) <= ATTENTION_WINDOW_MS &&
          !beyondSkew(s.time?.updated, now)),
    )
    .sort((a, b) => (b.time?.updated ?? 0) - (a.time?.updated ?? 0))
    .slice(0, opts.maxDetailed ?? 60);

  const entries: SessionFeedEntry[] = [];
  for (const s of included) {
    entries.push(
      await mapOne(target, s, activeIds, pendingBySession.get(s.id), opts.messageLimit ?? 40, opts.source, now),
    );
  }
  return { entries, tokensBySession, activeIds };
}

async function mapOne(
  target: OpenCodeTarget,
  s: V2Session,
  activeIds: Set<string>,
  pending: V2PermissionRequest | undefined,
  messageLimit: number,
  source: SessionSource | undefined,
  now: number,
): Promise<SessionFeedEntry> {
  const entry: SessionFeedEntry = {
    sessionId: s.id,
    source,
    parentId: s.parentID ?? null,
    agent: s.agent,
    // A session the server holds active is mid-run; any stored outcome
    // belongs to an earlier run of it and must not mark it finished.
    outcome: activeIds.has(s.id) ? null : (s.outcome ?? null),
    createdAt: iso(s.time?.created) ?? undefined,
    title: s.title,
    directory: s.location?.directory,
    model: s.model?.id,
    provider: s.model?.providerID,
    lastActivityAt: beyondSkew(s.time?.updated, now) ? null : iso(s.time?.updated),
    activeToolCalls: 0,
  };

  if (pending) {
    const what = firstString(pending.title, pending.description, pending.tool) ?? "tool call";
    entry.awaitingQuestion = `Permission pending: ${what}`;
  }

  const forms = await getJson<unknown>(target, `/api/session/${s.id}/form`);
  const formRows = isRecord(forms) ? records(forms.data) : [];
  const openForm = formRows[0];
  if (!entry.awaitingQuestion && openForm) {
    const question = records(openForm.questions)[0];
    const field = records(openForm.fields)[0];
    const text = firstString(question?.question, field?.label, openForm.title, openForm.name);
    entry.awaitingQuestion = `Form open: ${text ?? "session form awaiting input"}`;
  }

  const msgs = await getJson<unknown>(target, `/api/session/${s.id}/message?limit=${messageLimit}`);
  const messageRows = isRecord(msgs) ? records(msgs.data) : [];
  const data = messageRows.map((message) => ({
    ...message,
    content: records(message.content),
  })) as unknown as V2Message[];
  entry.messageCount = data.length;

  // Newest first. The "last turn" runs from the head back to (but not
  // including) the newest user message.
  let lastTurnError: string | null = null;
  const errorRun: string[] = [];
  let sawUserBoundary = false;
  for (const m of data) {
    if (m.type === "user") {
      if (errorRun.length > 0 || lastTurnError) break;
      sawUserBoundary = true;
      continue;
    }
    const msgError = errText(m.error);
    if (m.type === "idle" && msgError) {
      if (!lastTurnError) lastTurnError = msgError;
      errorRun.push(msgError);
      continue;
    }
    if (m.type === "assistant") {
      const toolErr = (m.content ?? []).find((p) => p.type === "tool" && p.state?.status === "error");
      const toolErrText = toolErr ? errText(toolErr.state?.error) : null;
      if (m.finish === "error" || toolErrText) {
        const text = msgError ?? toolErrText ?? "turn failed (finish=error)";
        if (!lastTurnError) lastTurnError = text;
        errorRun.push(text);
        continue;
      }
    }
    if (!sawUserBoundary && (m.type === "assistant" || m.type === "idle")) {
      // A clean turn boundary: errors older than this are not the last turn.
      if (lastTurnError) break;
    }
  }
  if (lastTurnError) entry.lastTurnError = lastTurnError;
  // Consecutive identical errors at the head of the run (retry loops).
  if (errorRun.length > 1 && errorRun[0]) {
    let n = 0;
    for (const e of errorRun) {
      if (e === errorRun[0]) n++;
      else break;
    }
    if (n > 1) entry.repeatedErrorCount = n;
  }

  // Tool activity from the newest assistant message.
  const newestAssistant = data.find((m) => m.type === "assistant");
  let running = 0;
  if (newestAssistant?.content) {
    for (const p of newestAssistant.content) {
      if (p.type !== "tool") continue;
      const status = p.state?.status;
      if (status === "running" || status === "pending") {
        running++;
        const cmd = p.state?.input?.command;
        if (!entry.longRunningCall && typeof cmd === "string") {
          entry.longRunningCall = {
            command: cmd,
            startedAt: iso(newestAssistant.time?.created) ?? new Date(0).toISOString(),
          };
        }
      }
    }
  }
  // A session the server holds active is mid-turn even when the newest
  // message snapshot shows no running part (e.g. the model is streaming).
  entry.activeToolCalls = activeIds.has(s.id) ? Math.max(1, running) : running;

  return entry;
}

/** Per-pool live burn attribution from a session list (collector use). */
export function poolBurn(
  sessions: { id: string; provider?: string; model?: string }[],
  tokensBySession: Map<string, number>,
  activeIds: Set<string>,
): Map<string, { session: string; model: string; tokens: number }[]> {
  const byProvider = new Map<string, { session: string; model: string; tokens: number }[]>();
  for (const s of sessions) {
    if (!activeIds.has(s.id) || !s.provider) continue;
    const tokens = tokensBySession.get(s.id) ?? 0;
    if (tokens <= 0) continue;
    const list = byProvider.get(s.provider) ?? [];
    list.push({ session: s.id, model: s.model ?? "?", tokens });
    byProvider.set(s.provider, list);
  }
  return byProvider;
}
