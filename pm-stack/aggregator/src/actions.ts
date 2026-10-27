// The dashboard actions store (#1152/#1154/#1156, coherence design
// Cluster 3). Dashboard-native write actions — the dashboard's second
// write surface after comments (#906 §2), whose pattern this mirrors
// point for point: one JSON file in the aggregator's data directory
// (PM_AGG_ACTIONS_FILE), atomic temp+rename writes, the in-memory copy
// authoritative for the process, strict validation at the POST
// boundary, and authorship via the shared validateAuthor.
//
// The dashboard NEVER writes GitHub, the Project, Kaneo, or a session
// server. It writes only this store; the fleet jobs that already own
// those surfaces apply the actions and report back. One consumer per
// (kind, source) pair — two consumers never poll the same slice, so no
// claim/lease machinery exists:
//
// - kind "triage" — the orchestration pass, in the same run segment
//   where it polls new message comments: acknowledge (the item's next
//   published tier entry carries the ack), snooze (the pass omits the
//   item from the Needs User tier until payload.until; the dashboard
//   renders the suppression in a visible "snoozed by you" strip),
//   escalate / de-escalate (tier override with the reason recorded),
//   answer (the pass delivers payload text/option/answers like its own
//   classifications), delegate (sets the item's delegation mode —
//   execution semantics are #1292's), dismiss (recorded with its
//   reason + the resurface rule, addendum §1.5).
// - kind "status-request" — the orchestration pass again: it is the
//   Project's single writer (#910), so it performs the Status write
//   itself and its run log names "requested via dashboard, action
//   da-…".
// - kind "session" — routed by the action's sessionSource: the Jules
//   keeper (jules-review) consumes source "jules" (answer only —
//   poke/stop render disabled on Jules cards); the orchestration pass
//   consumes the OpenCode sources ("opencode-sandbox",
//   "opencode-laptop") for poke/answer/stop. Sources with no live
//   control channel (claude-code, cron) render no enabled controls.
//
// Consumers poll GET /api/actions.json?state=new (filtered by their
// slice) and report each outcome via POST /api/actions/result
// {id, outcome: "applied"|"rejected", byRun, note}. Anything a
// consumer cannot apply is rejected WITH a note — never silently
// dropped.
//
// Idempotency, three layers (design Cluster 3): (1) clientToken
// dedupe — a repost of the same token returns the existing record;
// (2) consume-once — markResult only transitions out of state "new",
// so a crashed-and-retried consumer cannot double-apply; (3)
// convergent verbs — an item's effective state derives from the latest
// non-superseded action per verb family, and undo is a superseding
// action (supersedes set), never a delete. Records are never deleted:
// the file is the audit log, alongside the applying run's published
// section actions[] entry and the change itself.
//
// Trust model: identical to comments — the aggregator binds loopback +
// tailnet only, so authorship is self-asserted (DashAuthor). #941
// tracks hardening that; it is accepted-risk here by design decision
// (the orchestrator still applies every action and can reject).
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import { isObjectRef, validateAuthor } from "./comments.ts";
import type { DashAuthor, RefKey, SessionSource } from "./model.ts";

export type DashActionKind = "triage" | "status-request" | "session";

export type DashActionVerb =
  // triage (#1152)
  | "acknowledge"
  | "snooze"
  | "escalate"
  | "de-escalate"
  | "answer"
  // triage — needs-user action layer (addendum §4.3; UI lands #1290–#1293)
  | "delegate"
  | "dismiss"
  | "approve-proposal"
  | "reject-proposal"
  // status-request (#1154)
  | "request-status"
  // session (#1156)
  | "poke"
  | "stop";

export const VERBS_BY_KIND: Record<DashActionKind, readonly DashActionVerb[]> = {
  triage: [
    "acknowledge",
    "snooze",
    "escalate",
    "de-escalate",
    "answer",
    "delegate",
    "dismiss",
    "approve-proposal",
    "reject-proposal",
  ],
  "status-request": ["request-status"],
  session: ["poke", "answer", "stop"],
};

export interface DashActionPayload {
  /** snooze horizon (ISO). */
  until?: string;
  /** escalate / de-escalate target tier. */
  tier?: "needs-drew" | "needs-orchestrator";
  /** request-status target (a Project Status value). */
  status?: string;
  /** answer body. */
  text?: string;
  /** Optional on most verbs; REQUIRED for dismiss / reject-proposal. */
  reason?: string;
  /** delegate mode: first-pass | auto | drew (take it back). */
  mode?: "first-pass" | "auto" | "drew";
  /** answer: the published option the user picked (one content verb). */
  optionId?: string;
  /** answer: survey answers, keyed by the published question id. */
  answers?: { questionId: string; value: string }[];
}

export interface DashAction {
  id: string;
  /** Per-form-render dedupe key; unique across the store. */
  clientToken: string;
  kind: DashActionKind;
  verb: DashActionVerb;
  /** orch:<tierItemId> | gh:<owner/repo#N> | session:<registryId>. */
  objectRef: string;
  /** The GitHub item, when the action concerns one. */
  targetRef?: RefKey;
  /** Merged registry identity for session actions. */
  sessionId?: string;
  /** Registry source recorded at render time; routes session actions. */
  sessionSource?: SessionSource | "cron";
  payload: DashActionPayload;
  author: DashAuthor;
  createdAt: string;
  state: "new" | "applied" | "rejected" | "superseded";
  appliedAt?: string;
  /** runId of the consuming orchestration/keeper run. */
  appliedByRun?: string;
  /** What was done, or why rejected. */
  resultNote?: string;
  /** Id of the action this one replaces (undo = supersede). */
  supersedes?: string;
}

export interface ActionsFile {
  version: 1;
  actions: DashAction[];
}

export interface NewAction {
  clientToken: string;
  kind: DashActionKind;
  verb: DashActionVerb;
  objectRef: string;
  targetRef?: RefKey;
  sessionId?: string;
  sessionSource?: SessionSource | "cron";
  payload: DashActionPayload;
  author: DashAuthor;
  supersedes?: string;
}

const GH_REF = /^[\w.-]+\/[\w.-]+#\d+$/;
const SESSION_SOURCES = new Set(["opencode-sandbox", "opencode-laptop", "jules", "claude-code", "cron"]);

function cleanText(v: unknown, max: number): string | undefined {
  if (typeof v !== "string") return undefined;
  const t = v.trim();
  return t === "" ? undefined : t.slice(0, max);
}

/** Validate an untrusted POST body into a NewAction, or explain why not. */
export function validateNewAction(body: unknown): { ok: true; value: NewAction } | { ok: false; error: string } {
  if (typeof body !== "object" || body === null) return { ok: false, error: "body must be a JSON object" };
  const b = body as Record<string, unknown>;
  if (
    typeof b.clientToken !== "string" ||
    b.clientToken.length < 8 ||
    b.clientToken.length > 200 ||
    /\s/.test(b.clientToken)
  ) {
    return { ok: false, error: "clientToken must be an 8–200 char token without whitespace" };
  }
  if (b.kind !== "triage" && b.kind !== "status-request" && b.kind !== "session") {
    return { ok: false, error: "kind must be triage | status-request | session" };
  }
  const kind = b.kind as DashActionKind;
  if (typeof b.verb !== "string" || !VERBS_BY_KIND[kind].includes(b.verb as DashActionVerb)) {
    return { ok: false, error: `verb must be one of ${VERBS_BY_KIND[kind].join(" | ")} for kind ${kind}` };
  }
  const verb = b.verb as DashActionVerb;
  if (!isObjectRef(b.objectRef)) return { ok: false, error: "objectRef must be a non-empty ref string" };
  const author = validateAuthor(b.author);
  if (!author) return { ok: false, error: "author must be {kind: drew|muse|agent, id, label}" };

  const rawPayload = (typeof b.payload === "object" && b.payload !== null ? b.payload : {}) as Record<string, unknown>;
  const payload: DashActionPayload = {};
  const until = cleanText(rawPayload.until, 64);
  if (until !== undefined) {
    if (Number.isNaN(Date.parse(until))) return { ok: false, error: "payload.until must be an ISO date/time" };
    payload.until = until;
  }
  if (rawPayload.tier !== undefined) {
    if (rawPayload.tier !== "needs-drew" && rawPayload.tier !== "needs-orchestrator") {
      return { ok: false, error: "payload.tier must be needs-drew | needs-orchestrator" };
    }
    payload.tier = rawPayload.tier;
  }
  const status = cleanText(rawPayload.status, 60);
  if (status !== undefined) payload.status = status;
  const text = cleanText(rawPayload.text, 20_000);
  if (text !== undefined) payload.text = text;
  const reason = cleanText(rawPayload.reason, 2_000);
  if (reason !== undefined) payload.reason = reason;
  if (rawPayload.mode !== undefined) {
    if (rawPayload.mode !== "first-pass" && rawPayload.mode !== "auto" && rawPayload.mode !== "drew") {
      return { ok: false, error: "payload.mode must be first-pass | auto | drew" };
    }
    payload.mode = rawPayload.mode;
  }
  const optionId = cleanText(rawPayload.optionId, 200);
  if (optionId !== undefined) payload.optionId = optionId;
  if (rawPayload.answers !== undefined) {
    if (!Array.isArray(rawPayload.answers) || rawPayload.answers.length > 50) {
      return { ok: false, error: "payload.answers must be an array of at most 50 {questionId, value}" };
    }
    const answers: { questionId: string; value: string }[] = [];
    for (const entry of rawPayload.answers) {
      if (typeof entry !== "object" || entry === null) {
        return { ok: false, error: "payload.answers entries must be {questionId, value}" };
      }
      const q = cleanText((entry as Record<string, unknown>).questionId, 200);
      const v = cleanText((entry as Record<string, unknown>).value, 2_000);
      if (q === undefined || v === undefined) {
        return { ok: false, error: "payload.answers entries need non-empty questionId and value" };
      }
      answers.push({ questionId: q, value: v });
    }
    if (answers.length) payload.answers = answers;
  }

  let targetRef: RefKey | undefined;
  if (b.targetRef !== undefined) {
    if (typeof b.targetRef !== "string" || !GH_REF.test(b.targetRef)) {
      return { ok: false, error: "targetRef must be owner/repo#N" };
    }
    targetRef = b.targetRef;
  }
  let sessionId: string | undefined;
  if (b.sessionId !== undefined) {
    if (typeof b.sessionId !== "string" || b.sessionId === "" || b.sessionId.length > 300 || /\s/.test(b.sessionId)) {
      return { ok: false, error: "sessionId must be a non-empty id string" };
    }
    sessionId = b.sessionId;
  }
  let sessionSource: NewAction["sessionSource"];
  if (b.sessionSource !== undefined) {
    if (typeof b.sessionSource !== "string" || !SESSION_SOURCES.has(b.sessionSource)) {
      return { ok: false, error: "sessionSource must be a registry source" };
    }
    sessionSource = b.sessionSource as NewAction["sessionSource"];
  }
  const supersedes = cleanText(b.supersedes, 200);

  // Per-verb required payload — the boundary rules the consumers rely on.
  if (verb === "snooze" && payload.until === undefined) {
    return { ok: false, error: "snooze requires payload.until (ISO)" };
  }
  if ((verb === "escalate" || verb === "de-escalate") && payload.tier === undefined) {
    return { ok: false, error: `${verb} requires payload.tier` };
  }
  if (
    verb === "answer" &&
    payload.text === undefined &&
    payload.optionId === undefined &&
    payload.answers === undefined
  ) {
    return { ok: false, error: "answer requires payload.text, payload.optionId, or payload.answers" };
  }
  if (verb === "delegate" && payload.mode === undefined) {
    return { ok: false, error: "delegate requires payload.mode (first-pass | auto | drew)" };
  }
  if ((verb === "dismiss" || verb === "reject-proposal") && payload.reason === undefined) {
    return { ok: false, error: `${verb} requires payload.reason` };
  }
  if (verb === "request-status") {
    if (payload.status === undefined) return { ok: false, error: "request-status requires payload.status" };
    if (!b.objectRef.startsWith("gh:")) {
      return { ok: false, error: "request-status objectRef must be gh:<owner/repo#N>" };
    }
    targetRef ??= b.objectRef.slice(3);
    if (!GH_REF.test(targetRef)) return { ok: false, error: "request-status target must be owner/repo#N" };
  }
  if (kind === "session") {
    if (sessionId === undefined) return { ok: false, error: "session actions require sessionId" };
    if (b.objectRef !== `session:${sessionId}`) {
      return { ok: false, error: "session objectRef must be session:<sessionId>" };
    }
    if (verb === "answer" && payload.text === undefined) {
      return { ok: false, error: "a session answer requires payload.text" };
    }
  }

  return {
    ok: true,
    value: {
      clientToken: b.clientToken,
      kind,
      verb,
      objectRef: b.objectRef,
      ...(targetRef ? { targetRef } : {}),
      ...(sessionId ? { sessionId } : {}),
      ...(sessionSource ? { sessionSource } : {}),
      payload,
      author,
      ...(supersedes ? { supersedes } : {}),
    },
  };
}

let counter = 0;
export function newActionId(now = new Date()): string {
  counter = (counter + 1) % 46656;
  const rand = Math.random().toString(36).slice(2, 7);
  return `da-${now.getTime().toString(36)}-${counter.toString(36)}${rand}`;
}

/** A per-render form token (client-side dedupe key). */
export function newClientToken(): string {
  const rand = Math.random().toString(36).slice(2, 10) + Math.random().toString(36).slice(2, 10);
  return `ct-${Date.now().toString(36)}-${rand}`;
}

export class ActionsStore {
  private actions: DashAction[] = [];
  private loaded = false;

  constructor(private readonly file: string) {}

  async load(): Promise<void> {
    if (this.loaded) return;
    this.loaded = true;
    try {
      const parsed: unknown = JSON.parse(await readFile(this.file, "utf8"));
      if (typeof parsed === "object" && parsed !== null && Array.isArray((parsed as ActionsFile).actions)) {
        this.actions = (parsed as ActionsFile).actions.filter(
          (a) => typeof a === "object" && a !== null && typeof (a as DashAction).id === "string",
        );
      }
    } catch {
      this.actions = []; // no store yet — the first write creates it
    }
  }

  private async persist(): Promise<void> {
    const data: ActionsFile = { version: 1, actions: this.actions };
    await mkdir(dirname(this.file), { recursive: true });
    const tmp = `${this.file}.tmp-${process.pid}`;
    await writeFile(tmp, JSON.stringify(data, null, 2));
    await rename(tmp, this.file);
  }

  async all(): Promise<DashAction[]> {
    await this.load();
    return [...this.actions].sort((a, b) => a.createdAt.localeCompare(b.createdAt));
  }

  async forObject(objectRef: string): Promise<DashAction[]> {
    return (await this.all()).filter((a) => a.objectRef === objectRef);
  }

  /** Actions grouped by object ref, oldest first — the badge/form data. */
  async byObject(): Promise<Map<string, DashAction[]>> {
    const out = new Map<string, DashAction[]>();
    for (const a of await this.all()) out.set(a.objectRef, [...(out.get(a.objectRef) ?? []), a]);
    return out;
  }

  /** Pending (state "new") counts per kind, for /api/actions.json. */
  async pendingCounts(): Promise<{ triage: number; statusRequests: number; session: number }> {
    const pending = (await this.all()).filter((a) => a.state === "new");
    return {
      triage: pending.filter((a) => a.kind === "triage").length,
      statusRequests: pending.filter((a) => a.kind === "status-request").length,
      session: pending.filter((a) => a.kind === "session").length,
    };
  }

  /**
   * Add an action. A repost of an existing clientToken returns the
   * existing record with created:false (submit dedupe). Naming a
   * still-new action in `supersedes` transitions it to "superseded" —
   * undo is a superseding action, never a delete; an already-applied
   * action stays in history and the newer action wins by recency in
   * the effective-state derivation.
   */
  async add(input: NewAction): Promise<{ action: DashAction; created: boolean }> {
    await this.load();
    const existing = this.actions.find((a) => a.clientToken === input.clientToken);
    if (existing) return { action: existing, created: false };
    const action: DashAction = {
      id: newActionId(),
      clientToken: input.clientToken,
      kind: input.kind,
      verb: input.verb,
      objectRef: input.objectRef,
      ...(input.targetRef ? { targetRef: input.targetRef } : {}),
      ...(input.sessionId ? { sessionId: input.sessionId } : {}),
      ...(input.sessionSource ? { sessionSource: input.sessionSource } : {}),
      payload: input.payload,
      author: input.author,
      createdAt: new Date().toISOString(),
      state: "new",
      ...(input.supersedes ? { supersedes: input.supersedes } : {}),
    };
    this.actions.push(action);
    if (input.supersedes) {
      const old = this.actions.find((a) => a.id === input.supersedes);
      if (old && old.state === "new") old.state = "superseded";
    }
    await this.persist();
    return { action, created: true };
  }

  /**
   * Consume-once result transition: only a "new" action transitions.
   * A repeat result for the same id returns the current record
   * unchanged, so a retried consumer report is a safe no-op.
   */
  async markResult(
    id: string,
    result: { outcome: "applied" | "rejected"; byRun: string; note?: string },
  ): Promise<DashAction | null> {
    await this.load();
    const a = this.actions.find((x) => x.id === id);
    if (!a) return null;
    if (a.state !== "new") return a;
    a.state = result.outcome;
    a.appliedAt = new Date().toISOString();
    a.appliedByRun = result.byRun.slice(0, 200);
    if (result.note) a.resultNote = result.note.slice(0, 2_000);
    await this.persist();
    return a;
  }
}

/** Verb families for the convergent effective-state derivation. */
export const VERB_FAMILIES: Partial<Record<DashActionVerb, string>> = {
  acknowledge: "ack",
  snooze: "snooze",
  escalate: "tier",
  "de-escalate": "tier",
  delegate: "delegation",
  dismiss: "dismiss",
  "request-status": "status",
  poke: "poke",
  stop: "stop",
};

/** The latest non-superseded action of one verb family on one object. */
export function latestInFamily(actions: DashAction[], family: string): DashAction | undefined {
  const inFamily = actions.filter((a) => a.state !== "superseded" && VERB_FAMILIES[a.verb] === family);
  return inFamily[inFamily.length - 1];
}
