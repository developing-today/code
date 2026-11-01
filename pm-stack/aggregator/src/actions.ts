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
// - kind "preference" — the orchestration pass: set-delegation-default
//   records the standing auto toggle (#1290; Drew's 2026-10-27 decision
//   is the initial record). The aggregator also reads these records to
//   compute the effective default for items the publisher has not yet
//   stamped with a delegation state.
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
import { isRecord } from "./validate.ts";

export type DashActionKind = "triage" | "status-request" | "session" | "preference";

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
  // triage — quick actions + restore (#1449/#1450): "mark-done" is the
  // one-tap "Already done" (the consumer closes the item out; for gh:
  // refs it performs the Done status write). "restore" undoes a
  // suppression — it MUST name the suppressing action in `supersedes`.
  | "mark-done"
  | "restore"
  // status-request (#1154)
  | "request-status"
  // session (#1156)
  | "poke"
  | "stop"
  // preference (#1290)
  | "set-delegation-default"
  // preference — Needs-you card order (#1456)
  | "set-needs-order";

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
    "mark-done",
    "restore",
  ],
  "status-request": ["request-status"],
  session: ["poke", "answer", "stop"],
  preference: ["set-delegation-default", "set-needs-order"],
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
  /** delegate mode: first-pass | auto | user (take it back). The legacy
   *  spelling "drew" is accepted at the boundary and normalized to
   *  "user" — role-based naming (2026-10-27), no new Drew-named ids. */
  mode?: "first-pass" | "auto" | "user";
  /** answer: the published option the user picked (one content verb). */
  optionId?: string;
  /** Canonical survey question answered by an option shortcut. The server
   *  resolves this from the current published item before storing. */
  questionId?: string;
  /** answer: survey answers, keyed by the published question id. A
   *  multi-choice answer carries an array of option ids. */
  answers?: { questionId: string; value: string | string[] }[];
  /** approve-proposal / reject-proposal: the proposal identity the user
   *  reviewed (#1293) — (delegation runId, delegation updatedAt) of the
   *  item at render time. The consumer applies only on an exact match
   *  with the still-live proposal. */
  proposalRunId?: string;
  proposalUpdatedAt?: string;
  /** set-delegation-default: the standing auto toggle value. */
  standingAuto?: boolean;
  /** set-needs-order: the Needs-you card ordering (#1456). "urgency" is
   *  the orchestrator's published order; the rest sort at render. */
  needsOrder?: "urgency" | "newest" | "oldest" | "deadline";
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
  /** Existing answer action this rejected attempt duplicates. */
  duplicateOf?: string;
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

/** The question slots an answer action claims. Option-only and free-text
 *  answers use reserved slots until the item resolver supplies a question id. */
function answerQuestionIds(action: Pick<NewAction, "verb" | "payload">): string[] {
  if (action.verb !== "answer") return [];
  const ids = new Set(action.payload.answers?.map((answer) => answer.questionId) ?? []);
  if (action.payload.optionId) ids.add(action.payload.questionId ?? "$options");
  if (action.payload.text && !ids.size) ids.add("$answer");
  return [...ids];
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
  if (b.kind !== "triage" && b.kind !== "status-request" && b.kind !== "session" && b.kind !== "preference") {
    return { ok: false, error: "kind must be triage | status-request | session | preference" };
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
    if (
      rawPayload.mode !== "first-pass" &&
      rawPayload.mode !== "auto" &&
      rawPayload.mode !== "user" &&
      rawPayload.mode !== "drew"
    ) {
      return { ok: false, error: "payload.mode must be first-pass | auto | user" };
    }
    payload.mode = rawPayload.mode === "drew" ? "user" : rawPayload.mode;
  }
  const optionId = cleanText(rawPayload.optionId, 200);
  if (optionId !== undefined) payload.optionId = optionId;
  const questionId = cleanText(rawPayload.questionId, 200);
  if (questionId !== undefined) payload.questionId = questionId;
  if (rawPayload.answers !== undefined) {
    if (!Array.isArray(rawPayload.answers) || rawPayload.answers.length > 50) {
      return { ok: false, error: "payload.answers must be an array of at most 50 {questionId, value}" };
    }
    const answers: { questionId: string; value: string | string[] }[] = [];
    const seenQuestionIds = new Set<string>();
    for (const entry of rawPayload.answers) {
      if (typeof entry !== "object" || entry === null) {
        return { ok: false, error: "payload.answers entries must be {questionId, value}" };
      }
      const q = cleanText((entry as Record<string, unknown>).questionId, 200);
      const rawValue = (entry as Record<string, unknown>).value;
      let v: string | string[] | undefined;
      if (Array.isArray(rawValue)) {
        if (rawValue.length < 1 || rawValue.length > 10) {
          return { ok: false, error: "payload.answers array values must hold 1–10 choices" };
        }
        const parts = rawValue.map((x) => cleanText(x, 200));
        if (parts.some((x) => x === undefined)) {
          return { ok: false, error: "payload.answers entries need non-empty questionId and value" };
        }
        v = parts as string[];
      } else {
        v = cleanText(rawValue, 2_000);
      }
      if (q === undefined || v === undefined) {
        return { ok: false, error: "payload.answers entries need non-empty questionId and value" };
      }
      if (seenQuestionIds.has(q)) return { ok: false, error: `payload.answers repeats question id "${q}"` };
      seenQuestionIds.add(q);
      answers.push({ questionId: q, value: v });
    }
    if (answers.length) payload.answers = answers;
  }
  const proposalRunId = cleanText(rawPayload.proposalRunId, 80);
  if (proposalRunId !== undefined) payload.proposalRunId = proposalRunId;
  const proposalUpdatedAt = cleanText(rawPayload.proposalUpdatedAt, 64);
  if (proposalUpdatedAt !== undefined) {
    if (Number.isNaN(Date.parse(proposalUpdatedAt))) {
      return { ok: false, error: "payload.proposalUpdatedAt must be an ISO date/time" };
    }
    payload.proposalUpdatedAt = proposalUpdatedAt;
  }
  if (rawPayload.standingAuto !== undefined) {
    if (typeof rawPayload.standingAuto !== "boolean") {
      return { ok: false, error: "payload.standingAuto must be a boolean" };
    }
    payload.standingAuto = rawPayload.standingAuto;
  }
  if (rawPayload.needsOrder !== undefined) {
    if (
      rawPayload.needsOrder !== "urgency" &&
      rawPayload.needsOrder !== "newest" &&
      rawPayload.needsOrder !== "oldest" &&
      rawPayload.needsOrder !== "deadline"
    ) {
      return { ok: false, error: "payload.needsOrder must be urgency | newest | oldest | deadline" };
    }
    payload.needsOrder = rawPayload.needsOrder;
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
    return { ok: false, error: "delegate requires payload.mode (first-pass | auto | user)" };
  }
  if (verb === "set-delegation-default" && payload.standingAuto === undefined) {
    return { ok: false, error: "set-delegation-default requires payload.standingAuto (boolean)" };
  }
  if (verb === "set-needs-order" && payload.needsOrder === undefined) {
    return { ok: false, error: "set-needs-order requires payload.needsOrder" };
  }
  if (verb === "restore" && supersedes === undefined) {
    return { ok: false, error: "restore requires supersedes (the id of the action it undoes)" };
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
  private loadPromise: Promise<void> | null = null;
  private writeChain: Promise<void> = Promise.resolve();
  private writeSeq = 0;

  constructor(private readonly file: string) {}

  // The load is memoized as a single promise (#1509): the old
  // loaded-flag version let concurrent first callers proceed before
  // the read finished, and the read's completion then reset the
  // in-memory array, wiping records those callers had just pushed.
  async load(): Promise<void> {
    if (!this.loadPromise) this.loadPromise = this.doLoad();
    return this.loadPromise;
  }

  private async doLoad(): Promise<void> {
    // #1452: a missing file is the only silent empty start ("no store
    // yet"). An unparseable or wrongly-shaped file is NOT an empty
    // store: it is the append-only audit history, so quarantine the
    // original bytes to <file>.corrupt-<timestamp>, start empty, and
    // log loudly — the next persist then writes a fresh valid file and
    // never touches the quarantined copy.
    let raw: string;
    try {
      raw = await readFile(this.file, "utf8");
    } catch (err) {
      if ((err as { code?: unknown })?.code === "ENOENT") {
        this.actions = []; // no store yet — the first write creates it
        return;
      }
      // #2980: a non-ENOENT read failure (EACCES, a transient I/O error,
      // a directory in place of the file, ...) is NOT "no store yet":
      // the on-disk audit history may be intact but unread. Handle it
      // like the corrupt case below — quarantine the original bytes
      // before starting empty, and log loudly — so the next persist
      // cannot silently overwrite history the read never saw.
      this.actions = [];
      const quarantined = `${this.file}.corrupt-${new Date().toISOString().replace(/[:.]/g, "-")}`;
      try {
        await rename(this.file, quarantined);
        console.error(
          `[actions] WARNING: actions store ${this.file} could not be read (${err instanceof Error ? err.message : String(err)}); ` +
            `quarantined the original bytes to ${quarantined} and started empty. ` +
            `The quarantined file holds the prior audit history — do not delete it.`,
        );
      } catch (renameErr) {
        console.error(
          `[actions] WARNING: actions store ${this.file} could not be read (${err instanceof Error ? err.message : String(err)}) ` +
            `and could not be quarantined (${renameErr instanceof Error ? renameErr.message : String(renameErr)}); starting empty — ` +
            `the next persist will overwrite it, so copy it aside now to preserve the audit history.`,
        );
      }
      return;
    }
    try {
      const parsed: unknown = JSON.parse(raw);
      if (typeof parsed !== "object" || parsed === null || !Array.isArray((parsed as ActionsFile).actions)) {
        throw new Error("store file does not have the expected { actions: [...] } shape");
      }
      this.actions = (parsed as ActionsFile).actions.filter(
        (a) => isRecord(a) && typeof a.id === "string" && isRecord(a.payload),
      );
    } catch (err) {
      this.actions = [];
      const quarantined = `${this.file}.corrupt-${new Date().toISOString().replace(/[:.]/g, "-")}`;
      try {
        await rename(this.file, quarantined);
        console.error(
          `[actions] WARNING: actions store ${this.file} is unparseable (${err instanceof Error ? err.message : String(err)}); ` +
            `quarantined the original bytes to ${quarantined} and started empty. ` +
            `The quarantined file holds the prior audit history — do not delete it.`,
        );
      } catch (renameErr) {
        console.error(
          `[actions] WARNING: actions store ${this.file} is unparseable (${err instanceof Error ? err.message : String(err)}) ` +
            `and could not be quarantined (${renameErr instanceof Error ? renameErr.message : String(renameErr)}); starting empty — ` +
            `the next persist will overwrite it, so copy it aside now to preserve the audit history.`,
        );
      }
    }
  }

  // Serialized persists + unique temp path per write - see
  // CommentsStore.persist (#1509).
  private persist(): Promise<void> {
    const run = this.writeChain.then(async () => {
      const data: ActionsFile = { version: 1, actions: this.actions };
      await mkdir(dirname(this.file), { recursive: true });
      const tmp = `${this.file}.tmp-${process.pid}-${++this.writeSeq}`;
      await writeFile(tmp, JSON.stringify(data, null, 2));
      await rename(tmp, this.file);
    });
    this.writeChain = run.catch(() => {});
    return run;
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
  async pendingCounts(): Promise<{ triage: number; statusRequests: number; session: number; preference: number }> {
    const pending = (await this.all()).filter((a) => a.state === "new");
    return {
      triage: pending.filter((a) => a.kind === "triage").length,
      statusRequests: pending.filter((a) => a.kind === "status-request").length,
      session: pending.filter((a) => a.kind === "session").length,
      preference: pending.filter((a) => a.kind === "preference").length,
    };
  }

  /**
   * Add an action. A repost of an existing clientToken returns the
   * existing record with created:false (submit dedupe). Naming a
   * still-new action in `supersedes` transitions it to "superseded" —
   * undo is a superseding action, never a delete; an already-applied
   * action stays in history and the newer action wins by recency in
   * the effective-state derivation. A distinct answer for an occupied
   * question slot is itself persisted as rejected with `duplicateOf`
   * pointing to the pending/applied answer that owns that slot.
   */
  async add(input: NewAction): Promise<{ action: DashAction; created: boolean; duplicateOf?: string }> {
    await this.load();
    const existing = this.actions.find((a) => a.clientToken === input.clientToken);
    if (existing) return { action: existing, created: false };
    const questionIds = answerQuestionIds(input);
    const duplicate = questionIds.length
      ? this.actions.find(
          (candidate) =>
            candidate.objectRef === input.objectRef &&
            candidate.verb === "answer" &&
            candidate.state !== "rejected" &&
            candidate.state !== "superseded" &&
            answerQuestionIds(candidate).some((questionId) => questionIds.includes(questionId)),
        )
      : undefined;
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
      state: duplicate ? "rejected" : "new",
      ...(input.supersedes ? { supersedes: input.supersedes } : {}),
      ...(duplicate ? { duplicateOf: duplicate.id, resultNote: `Duplicate answer; existing action ${duplicate.id} is ${duplicate.state}.` } : {}),
    };
    this.actions.push(action);
    if (!duplicate && input.supersedes) {
      const old = this.actions.find((a) => a.id === input.supersedes);
      if (old && old.state === "new") old.state = "superseded";
    }
    await this.persist();
    return { action, created: true, ...(duplicate ? { duplicateOf: duplicate.id } : {}) };
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
  "mark-done": "done",
  "request-status": "status",
  poke: "poke",
  stop: "stop",
};

/** The latest non-superseded action of one verb family on one object. */
export function latestInFamily(actions: DashAction[], family: string): DashAction | undefined {
  const inFamily = actions.filter((a) => a.state !== "superseded" && VERB_FAMILIES[a.verb] === family);
  return inFamily[inFamily.length - 1];
}
