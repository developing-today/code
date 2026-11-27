// The queue door's verb engine (#12589): the broker protocol of
// the one fleet queue, implemented over the QueueStore interface —
// a faithful port of the native broker's verbs
// (pkgs/fleet/crates/fleet-queue/src/turso_queue + queue_broker.rs
// + disposition.rs + work_identity.rs + label_enrich.rs). The
// hosted door and the native door are peers over the same backend
// contract (design §6): same packet validation, same dedupe and
// work-identity rules, same lane ledger, same agent.events stream,
// same envelopes ({ok:true, ...} / {ok:false, exit, error}).
//
// Two deliberate divergences, both from the design:
//  - Authentication: the hosted door sits behind the worker's
//    authenticate → tenancy → service-scope pipeline and refuses
//    anonymous claims (acceptance 3); the native door keeps its
//    local trust model. Provenance: every event this door writes
//    carries the caller's credential-resolved binding_id in its
//    detail (an additive field; the native facade writes none).
//  - The claim guard's staleness retire (#10626) probes the landing
//    page over HTTP in the native broker; with no probe configured
//    it fails open and the claim proceeds. This door has no probe,
//    so it behaves exactly like a probe-less native broker.
//
// Not ported (other stacks / background machinery, named in the
// PR): the claim-flow composite (a client-side composite of
// claim+lane+bind, all present here), the sweep and
// quarantine-recheck background verbs (dep-recheck IS ported,
// including wake's forced recheck), merge-intent stack verbs,
// creation-outbox verbs, signal-count, coverage.

import {
  asI64,
  isObj,
  iso,
  labelBandPriority,
  parseRfc3339Epoch,
  taskFromPacket,
  validateNotBeforeAt,
  validatePacket,
  type Json,
  type JsonObject,
} from "./queue-validate.ts";
import {
  NO_DEP_PROBE,
  type DepObs,
  type EventRow,
  type LedgerRow,
  type QueueDepProbe,
  type QueueStore,
  type TaskRow,
} from "./queue-store.ts";

export interface QueueEnv {
  /** Service clock, epoch seconds (the native broker's clock). */
  nowEpoch: () => number;
  depProbe: QueueDepProbe;
  /** Credential-resolved tenancy binding; stamped into event
   * detail as claim provenance. Null for the native facade. */
  bindingId: string | null;
}

export function nativeQueueEnv(nowEpoch: () => number): QueueEnv {
  return { nowEpoch, depProbe: NO_DEP_PROBE, bindingId: null };
}

type Req = JsonObject;
type Resp = JsonObject;

export class QueueFail extends Error {
  constructor(
    readonly exit: number,
    message: string,
  ) {
    super(message);
  }
}

function fail(exit: number, message: string): never {
  throw new QueueFail(exit, message);
}

function ok(fields: Record<string, unknown>): Resp {
  return { ok: true, ...fields } as Resp;
}

function nowIso(env: QueueEnv): string {
  return iso(env.nowEpoch());
}

function clone<T extends Json>(v: T): T {
  return JSON.parse(JSON.stringify(v)) as T;
}

function obj(v: Json | undefined): JsonObject {
  return isObj(v) ? clone(v) : {};
}

function str(v: Json | undefined): string | undefined {
  return typeof v === "string" ? v : undefined;
}

function rawStr(v: Json | undefined): string | null {
  return typeof v === "string" ? v : null;
}

function intOrNull(v: Json | undefined): number | null {
  return v === undefined ? null : asI64(v);
}

function intOr(v: Json | undefined, dflt: number): number {
  return intOrNull(v) ?? dflt;
}

function mergeObj(a: JsonObject, b: JsonObject): JsonObject {
  return { ...a, ...b };
}

export const LIVE_STATUSES = ["queued", "claimed", "parked", "handed_off", "blocked"];
export const TERMINAL_STATUSES = ["done", "cancelled", "failed"];
const PLATFORM_FAMILIES = ["openchamber", "muse"];

export function parsePlatform(v: Json | undefined): string | null {
  if (v === undefined || v === null) return null;
  if (typeof v !== "string") {
    throw new Error(`invalid platform ${JSON.stringify(v)} (expected "openchamber" or "muse")`);
  }
  const p = v.trim();
  if (p === "") return null;
  if (PLATFORM_FAMILIES.includes(p)) return p;
  throw new Error(`invalid platform ${JSON.stringify(p)} (expected "openchamber" or "muse")`);
}

// ---------------------------------------------------------------------------
// Events and audit marks
// ---------------------------------------------------------------------------

async function emit(
  store: QueueStore,
  env: QueueEnv,
  worker: string | null,
  taskId: number | null,
  kind: string,
  detail: JsonObject,
): Promise<void> {
  const d = env.bindingId ? { ...detail, binding_id: env.bindingId } : detail;
  await store.appendEvent(worker, taskId, kind, d, nowIso(env));
}

function auditMarks(task: TaskRow): JsonObject {
  const progress = isObj(task.progress) ? task.progress : {};
  return {
    task_id: task.id,
    queue: task.queue,
    pool: task.queue,
    platform: task.platform,
    lane_id: task.lane_id,
    model_provider: task.claim_provider,
    model_id: task.claim_model,
    model_variant: task.claim_variant,
    session_id: typeof progress["session_id"] === "string" ? progress["session_id"] : null,
  };
}

async function emitMarked(
  store: QueueStore,
  env: QueueEnv,
  worker: string | null,
  task: TaskRow,
  kind: string,
  detail: JsonObject,
): Promise<void> {
  await emit(store, env, worker, task.id, kind, mergeObj(auditMarks(task), detail));
}

function progressOf(task: TaskRow): JsonObject {
  return isObj(task.progress) ? task.progress : {};
}

function progressStage(progress: JsonObject): string | null {
  if (typeof progress["stage"] === "string") return progress["stage"];
  const lc = progress["last_checkpoint"];
  if (isObj(lc) && typeof lc["stage"] === "string") return lc["stage"];
  return null;
}

function sessionIdOf(progress: JsonObject): string | null {
  const sid = progress["session_id"];
  return typeof sid === "string" ? sid : null;
}

/** A session mark is present when session_id is neither absent nor
 * null (the native is_none_or(is_null) test — a non-string value
 * still counts as a mark). */
function sessionMarkPresent(progress: JsonObject): boolean {
  return progress["session_id"] !== undefined && progress["session_id"] !== null;
}

function handoffShape(progress: JsonObject): boolean {
  const stage = progressStage(progress);
  return progress["lane"] === "opencode" && (stage === "handed_off" || stage === "launch_failed");
}

function isHandedOff(task: TaskRow): boolean {
  if (task.status !== "handed_off") return false;
  const progress = progressOf(task);
  return progress["lane"] === "opencode" && !sessionMarkPresent(progress);
}

/** clear_binding: strip the session binding from progress,
 * recording where it went. Returns [progress, clearedSessionId]. */
function clearBinding(progress: Json, via: string, env: QueueEnv): [JsonObject, string | null] {
  const p = obj(progress);
  if (typeof p["session_id"] !== "string") return [p, null];
  const old = p["session_id"];
  delete p["session_id"];
  delete p["launched_by"];
  p["session_cleared"] = { session_id: old, cleared_at: nowIso(env), via };
  return [p, old];
}

function clearedSessionId(task: TaskRow): string | null {
  const cleared = progressOf(task)["session_cleared"];
  if (isObj(cleared) && typeof cleared["session_id"] === "string") return cleared["session_id"];
  return null;
}

async function onBindingCleared(
  store: QueueStore,
  env: QueueEnv,
  worker: string | null,
  task: TaskRow,
  sessionId: string,
  via: string,
): Promise<void> {
  const ledger = await store.getLedger(task.id);
  if (ledger && ledger.session_id !== null) {
    await store.updateLedger(task.id, { session_id: null, updated_at: nowIso(env) });
  }
  await emit(store, env, worker, task.id, "session_binding_cleared", { session_id: sessionId, via });
  await emit(store, env, worker, task.id, "session_unbound", { session_id: sessionId, via });
}

async function recordSessionBound(
  store: QueueStore,
  env: QueueEnv,
  worker: string,
  task: TaskRow,
  sessionId: string,
  trigger: string,
): Promise<void> {
  await emitMarked(store, env, worker, task, "session_bound", { session_id: sessionId, trigger });
}

async function recordSessionUnbound(
  store: QueueStore,
  env: QueueEnv,
  worker: string,
  task: TaskRow,
  sessionId: string,
  trigger: string,
): Promise<void> {
  await emitMarked(store, env, worker, task, "session_unbound", { session_id: sessionId, trigger });
}

export interface ClaimedModel {
  provider: string;
  model: string;
  variant: string | null;
  sessionId: string | null;
}

export function parseClaimModel(req: Req): ClaimedModel | null {
  const field = (name: string): string | null => {
    const v = req[name];
    if (typeof v !== "string") return null;
    const t = v.trim();
    return t === "" ? null : t;
  };
  const provider = field("model_provider");
  const model = field("model_id");
  const variant = field("model_variant");
  const sessionId = typeof req["session_id"] === "string" ? req["session_id"] : null;
  if (provider === null && model === null) {
    if (variant !== null) {
      throw new Error("model_variant requires model_provider and model_id");
    }
    return null;
  }
  if (provider === null) {
    throw new Error(
      "model_id requires model_provider (the claim-model stamp names the serving leaf, provider and model together)",
    );
  }
  if (model === null) {
    throw new Error(
      "model_provider requires model_id (the claim-model stamp names the serving leaf, provider and model together)",
    );
  }
  return { provider, model, variant, sessionId };
}

async function stampClaimModel(
  store: QueueStore,
  env: QueueEnv,
  worker: string,
  taskId: number,
  model: ClaimedModel,
  trigger: string,
): Promise<void> {
  await store.updateTask(taskId, {
    claim_provider: model.provider,
    claim_model: model.model,
    claim_variant: model.variant,
    claim_session_id: model.sessionId,
    updated_at: nowIso(env),
  });
  const row = await store.getTask(taskId);
  if (!row) return;
  await emitMarked(store, env, worker, row, "model_stamp", {
    session_id: model.sessionId,
    provider: model.provider,
    model: model.model,
    variant: model.variant,
    trigger,
  });
}

// ---------------------------------------------------------------------------
// The claim guard (#5444) and quarantine
// ---------------------------------------------------------------------------

function payloadMissing(parsed: Json): boolean {
  if (parsed === null || parsed === undefined) return true;
  if (isObj(parsed)) return Object.keys(parsed).length === 0;
  return false;
}

function claimGuardReason(env: QueueEnv, row: TaskRow): string | null {
  if (payloadMissing(row.payload)) {
    return "quarantined by claim path: payload missing/empty (#5444)";
  }
  const [errors] = validatePacket(row as unknown as Json, env.nowEpoch());
  if (errors.length > 0) {
    return `quarantined by claim path: packet failed validation (#5444): ${errors.join("; ")}`;
  }
  return null;
}

async function quarantineTask(store: QueueStore, env: QueueEnv, taskId: number, reason: string): Promise<void> {
  await store.updateTask(taskId, {
    status: "blocked",
    worker_id: null,
    lease_until: null,
    last_error: reason,
    updated_at: nowIso(env),
  });
  await store.deleteLease(taskId);
  await emit(store, env, null, taskId, "blocked", { reason, via: "claim_payload_quarantine" });
}

// ---------------------------------------------------------------------------
// Repo identity, requirements, filters
// ---------------------------------------------------------------------------

function taskPayload(row: { payload: Json }): JsonObject {
  return isObj(row.payload) ? row.payload : {};
}

export function taskRepoIdentity(row: { queue: string; payload: Json }): string | null {
  const payload = taskPayload(row as { payload: Json });
  const repo = payload["repo"];
  if (typeof repo === "string" && repo !== "") return repo;
  let q = row.queue;
  if (q.startsWith("repo-health-")) q = q.slice("repo-health-".length);
  q = q.split("-pilot").join("").split("-shadow").join("");
  return q.includes("/") ? q : null;
}

function taskImplementationRepo(row: TaskRow): string | null {
  const target = taskPayload(row)["implementation_repo"];
  if (typeof target === "string" && target !== "") return target;
  return taskRepoIdentity(row);
}

export interface ClaimerCtx {
  family: string | null;
  lane: string | null;
  worker: string;
  pool: string | null;
  model: ClaimedModel | null;
  remaining: number | null;
}

function taskRequirementsUnmet(row: TaskRow, claimer: ClaimerCtx): string | null {
  const requires = taskPayload(row)["requires"];
  if (!isObj(requires)) return null;
  const platform = requires["platform"];
  if (typeof platform === "string" && claimer.family !== platform) {
    return `task requires platform ${platform}`;
  }
  const lane = requires["lane"];
  if (typeof lane === "string" && claimer.lane !== lane) {
    return `task requires lane ${lane}`;
  }
  const pool = requires["pool"];
  if (typeof pool === "string" && claimer.pool !== pool) {
    return `task requires pool ${pool}`;
  }
  const model = requires["model"];
  if (typeof model === "string") {
    const met =
      claimer.model !== null &&
      (claimer.model.model === model || `${claimer.model.provider}/${claimer.model.model}` === model);
    if (!met) return `task requires model ${model}`;
  }
  return null;
}

function requiredBudget(row: TaskRow): number | null {
  if (row.required_budget_minutes !== null) return row.required_budget_minutes;
  return intOrNull(taskPayload(row)["required_budget_minutes"]);
}

// --- The claim filter language (support.rs parse_filter, ported).

type Tok =
  | { t: "lparen" }
  | { t: "rparen" }
  | { t: "comma" }
  | { t: "word"; v: string }
  | { t: "str"; v: string }
  | { t: "op"; v: "=" | "!=" | "<" | "<=" | ">" | ">=" };

function tokDebug(tok: Tok | undefined): string {
  if (!tok) return "end of input";
  switch (tok.t) {
    case "lparen":
      return "LParen";
    case "rparen":
      return "RParen";
    case "comma":
      return "Comma";
    case "word":
      return `Word(${JSON.stringify(tok.v)})`;
    case "str":
      return `Str(${JSON.stringify(tok.v)})`;
    case "op":
      return tok.v === "="
        ? "Eq"
        : tok.v === "!="
          ? "Ne"
          : tok.v === "<"
            ? "Lt"
            : tok.v === "<="
              ? "Le"
              : tok.v === ">"
                ? "Gt"
                : "Ge";
  }
}

function tokenizeFilter(input: string): Tok[] {
  const toks: Tok[] = [];
  const chars = [...input];
  let i = 0;
  while (i < chars.length) {
    const c = chars[i];
    if (c === " " || c === "\t" || c === "\n" || c === "\r") {
      i++;
      continue;
    }
    if (c === "(") {
      toks.push({ t: "lparen" });
      i++;
      continue;
    }
    if (c === ")") {
      toks.push({ t: "rparen" });
      i++;
      continue;
    }
    if (c === ",") {
      toks.push({ t: "comma" });
      i++;
      continue;
    }
    if (c === "=") {
      toks.push({ t: "op", v: "=" });
      i++;
      continue;
    }
    if (c === "!") {
      if (chars[i + 1] === "=") {
        toks.push({ t: "op", v: "!=" });
        i += 2;
        continue;
      }
      throw new Error("filter: '!' is only valid as part of '!='");
    }
    if (c === "<") {
      if (chars[i + 1] === "=") {
        toks.push({ t: "op", v: "<=" });
        i += 2;
      } else {
        toks.push({ t: "op", v: "<" });
        i++;
      }
      continue;
    }
    if (c === ">") {
      if (chars[i + 1] === "=") {
        toks.push({ t: "op", v: ">=" });
        i += 2;
      } else {
        toks.push({ t: "op", v: ">" });
        i++;
      }
      continue;
    }
    if (c === '"' || c === "'") {
      const quote = c;
      let j = i + 1;
      let value = "";
      let closed = false;
      while (j < chars.length) {
        if (chars[j] === quote) {
          closed = true;
          break;
        }
        value += chars[j];
        j++;
      }
      if (!closed) throw new Error("filter: unterminated quoted value");
      toks.push({ t: "str", v: value });
      i = j + 1;
      continue;
    }
    if (/[A-Za-z0-9_\-./:#@*]/.test(c)) {
      let j = i;
      let value = "";
      while (j < chars.length && /[A-Za-z0-9_\-./:#@*]/.test(chars[j])) {
        value += chars[j];
        j++;
      }
      toks.push({ t: "word", v: value });
      i = j;
      continue;
    }
    throw new Error(`filter: unexpected character ${JSON.stringify(c)}`);
  }
  return toks;
}

type FilterExpr =
  | { type: "or"; l: FilterExpr; r: FilterExpr }
  | { type: "and"; l: FilterExpr; r: FilterExpr }
  | { type: "not"; e: FilterExpr }
  | { type: "cmp"; field: string; op: string; values: string[] };

const FILTER_FIELDS = ["repo", "queue", "pool", "platform", "label", "kind", "priority", "lane", "model"];

class FilterParser {
  private pos = 0;
  constructor(private toks: Tok[]) {}

  private peek(): Tok | undefined {
    return this.toks[this.pos];
  }

  private next(): Tok | undefined {
    return this.toks[this.pos++];
  }

  private peekWord(word: string): boolean {
    const t = this.peek();
    return t?.t === "word" && t.v.toUpperCase() === word;
  }

  parse(): FilterExpr {
    if (this.toks.length === 0) throw new Error("filter: empty expression");
    const expr = this.parseOr();
    if (this.pos < this.toks.length) throw new Error("filter: unexpected trailing input");
    validateFilterExpr(expr);
    return expr;
  }

  private parseOr(): FilterExpr {
    let l = this.parseAnd();
    while (this.peekWord("OR")) {
      this.next();
      const r = this.parseAnd();
      l = { type: "or", l, r };
    }
    return l;
  }

  private parseAnd(): FilterExpr {
    let l = this.parseUnary();
    while (this.peekWord("AND")) {
      this.next();
      const r = this.parseUnary();
      l = { type: "and", l, r };
    }
    return l;
  }

  private parseUnary(): FilterExpr {
    if (this.peekWord("NOT")) {
      this.next();
      return { type: "not", e: this.parseUnary() };
    }
    return this.parsePrimary();
  }

  private parsePrimary(): FilterExpr {
    const t = this.peek();
    if (t?.t === "lparen") {
      this.next();
      const e = this.parseOr();
      if (this.next()?.t !== "rparen") throw new Error("filter: expected ')'");
      return e;
    }
    return this.parseComparison();
  }

  private parseValue(): string {
    const t = this.next();
    if (t?.t === "word" || t?.t === "str") return t.v;
    throw new Error(`filter: expected a value, found ${tokDebug(t)}`);
  }

  private parseComparison(): FilterExpr {
    const fieldTok = this.next();
    if (fieldTok?.t !== "word") {
      throw new Error(`filter: expected a field name, found ${tokDebug(fieldTok)}`);
    }
    const field = fieldTok.v.toLowerCase();
    if (!FILTER_FIELDS.includes(field)) {
      throw new Error(
        `filter: unknown field ${JSON.stringify(fieldTok.v)} (fields: repo, queue, pool, platform, label, kind, priority, lane, model)`,
      );
    }
    let negated = false;
    if (this.peekWord("NOT")) {
      this.next();
      negated = true;
    }
    if (this.peekWord("IN")) {
      this.next();
      if (this.next()?.t !== "lparen") throw new Error("filter: expected '(' after IN");
      const values = [this.parseValue()];
      while (this.peek()?.t === "comma") {
        this.next();
        values.push(this.parseValue());
      }
      if (this.next()?.t !== "rparen") {
        throw new Error("filter: expected ')' after the IN list");
      }
      return { type: "cmp", field, op: negated ? "notin" : "in", values };
    }
    if (negated) {
      throw new Error(`filter: expected an operator after ${field}, found ${tokDebug(this.peek())}`);
    }
    const opTok = this.next();
    if (opTok?.t !== "op") {
      throw new Error(`filter: expected an operator after ${field}, found ${tokDebug(opTok)}`);
    }
    const op =
      opTok.v === "="
        ? "eq"
        : opTok.v === "!="
          ? "ne"
          : opTok.v === "<"
            ? "lt"
            : opTok.v === "<="
              ? "le"
              : opTok.v === ">"
                ? "gt"
                : "ge";
    return { type: "cmp", field, op, values: [this.parseValue()] };
  }
}

function validateFilterExpr(e: FilterExpr): void {
  if (e.type === "or" || e.type === "and") {
    validateFilterExpr(e.l);
    validateFilterExpr(e.r);
    return;
  }
  if (e.type === "not") {
    validateFilterExpr(e.e);
    return;
  }
  if (e.values.length === 0) throw new Error("filter: a comparison needs at least one value");
  if (e.op !== "in" && e.op !== "notin" && e.values.length > 1) {
    throw new Error("filter: only IN takes multiple values");
  }
  if (e.field === "priority") {
    for (const v of e.values) {
      if (!/^[+-]?[0-9]+$/.test(v)) {
        throw new Error(`filter: priority value ${JSON.stringify(v)} is not an integer`);
      }
    }
  } else if (e.op === "lt" || e.op === "le" || e.op === "gt" || e.op === "ge") {
    throw new Error("filter: ordering comparisons are only valid on priority");
  }
}

export function parseFilter(input: string): FilterExpr {
  return new FilterParser(tokenizeFilter(input)).parse();
}

function filterMatches(e: FilterExpr, row: TaskRow): boolean {
  switch (e.type) {
    case "or":
      return filterMatches(e.l, row) || filterMatches(e.r, row);
    case "and":
      return filterMatches(e.l, row) && filterMatches(e.r, row);
    case "not":
      return !filterMatches(e.e, row);
    case "cmp":
      return cmpMatches(e, row);
  }
}

function cmpMatches(e: { field: string; op: string; values: string[] }, row: TaskRow): boolean {
  const payload = taskPayload(row);
  if (e.field === "label") {
    const labels = payload["labels"];
    const list = Array.isArray(labels) ? labels.filter((l): l is string => typeof l === "string") : [];
    const any = e.values.some((v) => list.includes(v));
    return e.op === "eq" || e.op === "in" ? any : e.op === "ne" || e.op === "notin" ? !any : false;
  }
  if (e.field === "priority") {
    const subject = row.priority;
    const nums = e.values.map((v) => Number(v));
    switch (e.op) {
      case "eq":
        return subject !== null && nums.some((n) => n === subject);
      case "ne":
        return subject === null || !nums.some((n) => n === subject);
      case "in":
        return subject !== null && nums.some((n) => n === subject);
      case "notin":
        return subject === null || !nums.some((n) => n === subject);
      case "lt":
        return subject !== null && subject < nums[0];
      case "le":
        return subject !== null && subject <= nums[0];
      case "gt":
        return subject !== null && subject > nums[0];
      case "ge":
        return subject !== null && subject >= nums[0];
      default:
        return false;
    }
  }
  let subject: string | null = null;
  switch (e.field) {
    case "repo":
      subject = taskImplementationRepo(row);
      break;
    case "queue":
    case "pool":
      subject = row.queue;
      break;
    case "platform":
      subject = row.platform;
      break;
    case "kind":
      subject = typeof payload["kind"] === "string" ? payload["kind"] : null;
      break;
    case "lane": {
      const r = payload["requires"];
      subject = isObj(r) && typeof r["lane"] === "string" ? r["lane"] : null;
      break;
    }
    case "model": {
      const r = payload["requires"];
      subject = isObj(r) && typeof r["model"] === "string" ? r["model"] : null;
      break;
    }
  }
  const matches = (v: string): boolean => {
    if (subject === null) return false;
    if (e.field === "repo") return subject === v || subject.endsWith(`/${v}`);
    return subject === v;
  };
  switch (e.op) {
    case "eq":
      return e.values.some(matches);
    case "in":
      return e.values.some(matches);
    case "ne":
      return subject === null || !e.values.some(matches);
    case "notin":
      return subject === null || !e.values.some(matches);
    default:
      return false;
  }
}

// ---------------------------------------------------------------------------
// Claim machinery
// ---------------------------------------------------------------------------

interface ClaimScope {
  queue: string | null;
  filter: FilterExpr | null;
  claimer: ClaimerCtx;
}

function platformLimitInvalid(limit: string | null, reason: string | null): boolean {
  return limit !== null && limit.trim() !== "" && (reason === null || reason.trim() === "");
}

function platformLimitError(limit: string | null, reason: string | null): string | null {
  const limitBlank = limit === null || limit.trim() === "";
  const reasonBlank = reason === null || reason.trim() === "";
  if (limitBlank && !reasonBlank) {
    return "platform_limit_reason requires a platform_limit to limit";
  }
  if (!limitBlank) {
    parsePlatform(limit as unknown as Json); // propagates the parse error
    if (reasonBlank) {
      return `a platform limitation requires a recorded reason (--limit-reason): a task is limited to '${limit}' only for a good reason, recorded on the task`;
    }
  }
  return null;
}

function platformLimitRefusal(limit: string | null, reason: string | null, family: string | null): string | null {
  if (limit === null || limit.trim() === "" || platformLimitInvalid(limit, reason)) {
    return null;
  }
  if (family === limit) return null;
  if (family !== null) {
    return `task is limited to platform '${limit}' (reason: ${reason}); a '${family}' transfer-claim cannot take it`;
  }
  return `task is limited to platform '${limit}' (reason: ${reason}); the transfer-claim must declare its platform family to take it`;
}

async function scopeMatch(
  store: QueueStore,
  row: TaskRow,
  scope: ClaimScope,
  withPlatformLimit: boolean,
): Promise<boolean> {
  if (scope.queue !== null && row.queue !== scope.queue) return false;
  if (scope.filter && !filterMatches(scope.filter, row)) return false;
  if (taskRequirementsUnmet(row, scope.claimer) !== null) return false;
  if (withPlatformLimit) {
    const ledger = await store.getLedger(row.id);
    if (
      ledger &&
      ledger.platform_limit !== null &&
      ledger.platform_limit !== "" &&
      ledger.platform_limit_reason !== null &&
      ledger.platform_limit_reason !== "" &&
      (scope.claimer.family === null || ledger.platform_limit !== scope.claimer.family)
    ) {
      return false;
    }
  }
  return true;
}

function eligibleDefault(row: TaskRow, now: string, claimer: ClaimerCtx): boolean {
  if (row.manual_only) return false;
  const required = requiredBudget(row);
  const remaining = claimer.remaining ?? -1;
  const gate = required === null || remaining >= required;
  const bypass = required !== null && remaining >= required;
  const due = row.not_before === null || row.not_before <= now;
  let live = false;
  if (row.status === "queued" && row.attempts < row.max_attempts && due) {
    live = gate;
  } else if (
    row.status === "parked" &&
    ((row.not_before !== null && row.not_before <= now) || (row.wait_kind === "capacity" && bypass))
  ) {
    live = gate;
  }
  return live || isHandedOff(row);
}

function payloadNonEmpty(row: TaskRow): boolean {
  return !payloadMissing(row.payload);
}

/** expire_leases: claimed rows whose lease lapsed return to the
 * queue (or the hand-off sink), or fail at the attempt cap. */
async function expireLeases(store: QueueStore, env: QueueEnv): Promise<number> {
  const now = nowIso(env);
  let expired = 0;
  const rows = (await store.allTasks()).filter(
    (t) => t.status === "claimed" && t.lease_until !== null && t.lease_until < now,
  );
  for (const row of rows) {
    const toSink = handoffShape(progressOf(row));
    const [progress, cleared] = clearBinding(row.progress, "expire_leases", env);
    const atCap = !toSink && row.attempts >= row.max_attempts;
    const set: Partial<TaskRow> = atCap
      ? {
          status: "failed",
          worker_id: null,
          lease_until: null,
          last_error: `lease expired at attempt cap (${row.attempts}/${row.max_attempts})`,
          progress,
          updated_at: now,
        }
      : toSink
        ? {
            status: "handed_off",
            worker_id: null,
            lease_until: null,
            not_before: null,
            park_kind: null,
            wait_kind: null,
            ready_condition: null,
            progress,
            updated_at: now,
          }
        : {
            status: "queued",
            worker_id: null,
            lease_until: null,
            progress,
            updated_at: now,
          };
    const updated = await store.conditionalUpdateTask(row.id, { status: "claimed", lease_until: row.lease_until }, set);
    if (!updated) continue;
    if (atCap) {
      await emit(store, env, row.worker_id, row.id, "lease_expired_at_cap", {
        via: "expire_leases",
        attempts: row.attempts,
        max_attempts: row.max_attempts,
        cleared_session_id: cleared,
      });
    } else {
      expired += 1;
      await emit(store, env, row.worker_id, row.id, "lease_expired", {
        via: "expire_leases",
        cleared_session_id: cleared,
        return: toSink ? "handed_off" : "queued",
      });
    }
    if (cleared) {
      const fresh = await store.getTask(row.id);
      if (fresh) await onBindingCleared(store, env, row.worker_id, fresh, cleared, "expire_leases");
    }
  }
  await store.deleteExpiredLeases(now);
  return expired;
}

async function quarantineSweep(store: QueueStore, env: QueueEnv, queue: string | null): Promise<void> {
  for (const row of await store.allTasks()) {
    if (queue !== null && row.queue !== queue) continue;
    if (!["queued", "parked", "handed_off"].includes(row.status)) continue;
    if (payloadMissing(row.payload)) {
      await quarantineTask(store, env, row.id, "quarantined by claim path: payload missing/empty (#5444)");
    }
  }
}

interface FinishClaimOpts {
  worker: string;
  lane: string | null;
  claimSession: string | null;
  model: ClaimedModel | null;
  stampTrigger: string;
  detail: JsonObject;
}

/** The shared post-claim flow: unbind the previous session, bind
 * the new one, write the lease, stamp the model, touch the worker,
 * and record the claim on the shared event timeline. */
async function finishClaim(store: QueueStore, env: QueueEnv, row: TaskRow, opts: FinishClaimOpts): Promise<TaskRow> {
  const now = nowIso(env);
  let current = row;
  const [progress, cleared] = clearBinding(current.progress, "claim_repo_task", env);
  let nextProgress = progress;
  if (opts.claimSession) {
    nextProgress = { ...nextProgress, session_id: opts.claimSession, launched_by: opts.worker };
  }
  if (cleared || opts.claimSession) {
    await store.updateTask(current.id, { progress: nextProgress, updated_at: now });
    current = (await store.getTask(current.id)) ?? current;
  }
  await store.upsertLease(
    {
      task_id: current.id,
      worker_id: opts.worker,
      lease_until: current.lease_until ?? "",
      heartbeat_at: now,
    },
    { keepWorker: false },
  );
  if (opts.claimSession) {
    await recordSessionBound(store, env, opts.worker, current, opts.claimSession, opts.stampTrigger);
  }
  if (opts.model) {
    await stampClaimModel(store, env, opts.worker, current.id, opts.model, opts.stampTrigger);
    current = (await store.getTask(current.id)) ?? current;
  }
  await store.touchWorker(opts.worker, true, now);
  if (cleared) {
    await onBindingCleared(store, env, opts.worker, current, cleared, "claim_repo_task");
  }
  await emitMarked(store, env, opts.worker, current, "claimed", {
    ...opts.detail,
    cleared_session_id: cleared,
  });
  return (await store.getTask(current.id)) ?? current;
}

const MAX_CLAIM_RACE_LOSSES = 32;
const MAX_CLAIM_QUARANTINES_PER_CALL = 512;

async function claimPickOnce(
  store: QueueStore,
  env: QueueEnv,
  scope: ClaimScope,
  pickMode: "default" | "handoff",
  leaseSeconds: number,
  claimSession: string | null,
  model: ClaimedModel | null,
): Promise<TaskRow | null> {
  const now = nowIso(env);
  let skipped = 0;
  let quarantined = 0;
  for (;;) {
    if (skipped > MAX_CLAIM_RACE_LOSSES || quarantined > MAX_CLAIM_QUARANTINES_PER_CALL) {
      return null;
    }
    const candidates: TaskRow[] = [];
    for (const t of await store.allTasks()) {
      if (
        !t.manual_only &&
        payloadNonEmpty(t) &&
        (pickMode === "handoff" ? isHandedOff(t) : eligibleDefault(t, now, scope.claimer)) &&
        (await scopeMatch(store, t, scope, true))
      ) {
        candidates.push(t);
      }
    }
    candidates.sort(
      (a, b) =>
        (a.priority === null ? 1 : 0) - (b.priority === null ? 1 : 0) ||
        (a.priority ?? 0) - (b.priority ?? 0) ||
        (a.created_at < b.created_at ? -1 : a.created_at > b.created_at ? 1 : 0) ||
        a.id - b.id,
    );
    const cand = candidates[0];
    if (!cand) return null;
    const leaseUntil = iso(env.nowEpoch() + leaseSeconds);
    const updated = await store.conditionalUpdateTask(
      cand.id,
      {
        status: cand.status,
        worker_id: cand.worker_id,
        attempts: cand.attempts,
        lease_until: cand.lease_until,
      },
      {
        status: "claimed",
        worker_id: scope.claimer.worker,
        claimed_at: now,
        lease_until: leaseUntil,
        attempts: cand.status === "parked" || cand.status === "handed_off" ? cand.attempts : cand.attempts + 1,
        lane_id: scope.claimer.lane,
        platform: scope.claimer.family ?? cand.platform,
        park_kind: null,
        wait_kind: null,
        not_before: null,
        required_budget_minutes: null,
        updated_at: now,
      },
    );
    if (!updated) {
      skipped++;
      continue;
    }
    const row = await store.getTask(cand.id);
    if (!row) return null;
    const guard = claimGuardReason(env, row);
    if (guard) {
      await quarantineTask(store, env, row.id, guard);
      quarantined++;
      continue;
    }
    return await finishClaim(store, env, row, {
      worker: scope.claimer.worker,
      lane: scope.claimer.lane,
      claimSession,
      model,
      stampTrigger: "claim",
      detail: {
        lease_seconds: leaseSeconds,
        via: "claim_repo_task_turso",
        lane_id: scope.claimer.lane,
        platform: scope.claimer.family,
        session_id: claimSession,
        model_provider: model?.provider ?? null,
        model_id: model?.model ?? null,
        model_variant: model?.variant ?? null,
        pick: pickMode === "handoff" ? "handed_off" : "default",
      },
    });
  }
}

function byIdClaimable(row: TaskRow, now: string): boolean {
  if (row.status === "queued") {
    return row.attempts < row.max_attempts && (row.not_before === null || row.not_before <= now);
  }
  if (row.status === "handed_off") return isHandedOff(row);
  return false;
}

function liveClaimHolder(row: TaskRow, now: string): boolean {
  return row.status === "claimed" && row.lease_until !== null && row.lease_until >= now;
}

async function claimById(
  store: QueueStore,
  env: QueueEnv,
  scope: ClaimScope,
  taskId: number,
  leaseSeconds: number,
  claimSession: string | null,
  model: ClaimedModel | null,
): Promise<TaskRow | null> {
  const now = nowIso(env);
  const row = await store.getTask(taskId);
  if (!row) return null;
  if (scope.queue !== null && row.queue !== scope.queue) return null;
  const guard = claimGuardReason(env, row);
  if (guard) {
    await quarantineTask(store, env, row.id, guard);
    return null;
  }
  let ledger = await store.getLedger(taskId);
  if (ledger && platformLimitInvalid(ledger.platform_limit, ledger.platform_limit_reason)) {
    await store.updateLedger(taskId, {
      platform_limit: null,
      platform_limit_reason: null,
      updated_at: now,
    });
    ledger = await store.getLedger(taskId);
  }
  if (
    ledger &&
    platformLimitRefusal(ledger.platform_limit, ledger.platform_limit_reason, scope.claimer.family) !== null
  ) {
    return null;
  }
  if (!byIdClaimable(row, now)) {
    if (liveClaimHolder(row, now)) {
      try {
        await emit(store, env, scope.claimer.worker, row.id, "claim_refused", {
          via: "claim_by_id",
          reason: "already_claimed",
          holder_worker_id: row.worker_id,
          holder_lane_id: row.lane_id,
          holder_lease_until: row.lease_until,
          claimer_lane_id: scope.claimer.lane,
        });
      } catch {
        // The refusal event is best-effort (native: let _ =).
      }
    }
    return null;
  }
  if (taskRequirementsUnmet(row, scope.claimer) !== null) return null;
  const updated = await store.conditionalUpdateTask(
    row.id,
    { status: row.status, worker_id: row.worker_id, attempts: row.attempts, queue: row.queue },
    {
      status: "claimed",
      worker_id: scope.claimer.worker,
      claimed_at: now,
      lease_until: iso(env.nowEpoch() + leaseSeconds),
      attempts: row.status === "parked" || row.status === "handed_off" ? row.attempts : row.attempts + 1,
      lane_id: scope.claimer.lane,
      platform: scope.claimer.family ?? row.platform,
      park_kind: null,
      wait_kind: null,
      not_before: null,
      updated_at: now,
    },
  );
  if (!updated) return null;
  const claimed = await store.getTask(taskId);
  if (!claimed) return null;
  return await finishClaim(store, env, claimed, {
    worker: scope.claimer.worker,
    lane: scope.claimer.lane,
    claimSession,
    model,
    stampTrigger: "claim",
    detail: {
      lease_seconds: leaseSeconds,
      via: "claim_repo_task_turso",
      lane_id: scope.claimer.lane,
      by_id: true,
      task_id: taskId,
      platform: scope.claimer.family,
      model_provider: model?.provider ?? null,
      model_id: model?.model ?? null,
      model_variant: model?.variant ?? null,
    },
  });
}

function transferRefusal(
  row: TaskRow,
  fromSession: string,
  claimedEventSession: string | null,
  ledgerSession: string | null,
  newSession: string | null,
  now: string,
): string | null {
  if (row.status !== "claimed") {
    return `task is ${row.status}, not claimed: only a live claim can be transferred`;
  }
  if (row.lease_until === null || row.lease_until <= now) {
    return "the holder's lease is not live: no live claim to transfer";
  }
  if (fromSession.trim() === "") return "from_session is empty";
  const progress = progressOf(row);
  const marks = [
    typeof progress["session_id"] === "string" ? progress["session_id"] : "",
    typeof progress["from_session"] === "string" ? progress["from_session"] : "",
    claimedEventSession ?? "",
  ];
  if (!marks.some((m) => m !== "" && m === fromSession)) {
    return "from_session does not match the current holder's session mark: only the current holder can give this task away";
  }
  for (const existing of [
    typeof progress["session_id"] === "string" ? progress["session_id"] : "",
    ledgerSession ?? "",
  ]) {
    if (existing !== "" && existing !== fromSession && existing !== (newSession ?? "")) {
      return `task is already bound to session ${existing}: a bound launch is not a transfer`;
    }
  }
  return null;
}

async function claimTransfer(
  store: QueueStore,
  env: QueueEnv,
  scope: ClaimScope,
  taskId: number,
  fromSession: string,
  leaseSeconds: number,
  claimSession: string | null,
  model: ClaimedModel | null,
): Promise<TaskRow | null> {
  const now = nowIso(env);
  const row = await store.getTask(taskId);
  if (!row) return null;
  if (scope.queue !== null && row.queue !== scope.queue) return null;
  const guard = claimGuardReason(env, row);
  if (guard) {
    await quarantineTask(store, env, row.id, guard);
    return null;
  }
  const claimedEvents = (await store.eventsForTask(taskId)).filter((e) => e.kind === "claimed");
  const lastClaimed = claimedEvents[claimedEvents.length - 1];
  const claimedEventSession =
    lastClaimed && isObj(lastClaimed.detail) && typeof lastClaimed.detail["session_id"] === "string"
      ? (lastClaimed.detail["session_id"] as string)
      : null;
  let ledger = await store.getLedger(taskId);
  let limitClearedInvalid = false;
  if (ledger && platformLimitInvalid(ledger.platform_limit, ledger.platform_limit_reason)) {
    await store.updateLedger(taskId, {
      platform_limit: null,
      platform_limit_reason: null,
      updated_at: now,
    });
    limitClearedInvalid = true;
    ledger = await store.getLedger(taskId);
  }
  if (ledger) {
    const refusal = platformLimitRefusal(ledger.platform_limit, ledger.platform_limit_reason, scope.claimer.family);
    if (refusal) fail(4, `claim_repo_task: transfer refused: ${refusal}`);
  }
  const unmet = taskRequirementsUnmet(row, scope.claimer);
  if (unmet) fail(4, `claim_repo_task: transfer refused: ${unmet}`);
  const refusal = transferRefusal(row, fromSession, claimedEventSession, ledger?.session_id ?? null, claimSession, now);
  if (refusal) fail(4, `claim_repo_task: transfer refused: ${refusal}`);
  const oldWorker = row.worker_id;
  const oldLane = row.lane_id;
  const oldProgress = progressOf(row);
  const newLane = scope.claimer.lane ?? oldLane;
  const updated = await store.conditionalUpdateTask(
    row.id,
    { status: "claimed", worker_id: oldWorker },
    {
      worker_id: scope.claimer.worker,
      claimed_at: now,
      lease_until: iso(env.nowEpoch() + leaseSeconds),
      lane_id: newLane,
      platform: scope.claimer.family ?? row.platform,
      park_kind: null,
      wait_kind: null,
      not_before: null,
      updated_at: now,
    },
  );
  if (!updated) {
    fail(4, "claim_repo_task: transfer refused: the claim changed under the transfer");
  }
  let current = (await store.getTask(taskId)) as TaskRow;
  await store.upsertLease(
    {
      task_id: taskId,
      worker_id: scope.claimer.worker,
      lease_until: current.lease_until ?? "",
      heartbeat_at: now,
    },
    { keepWorker: false },
  );
  if (claimSession) {
    await store.updateTask(taskId, {
      progress: { ...progressOf(current), session_id: claimSession, launched_by: scope.claimer.worker },
      updated_at: now,
    });
    if (ledger) {
      await store.updateLedger(taskId, { session_id: claimSession, updated_at: now });
    }
    current = (await store.getTask(taskId)) as TaskRow;
    await recordSessionBound(store, env, scope.claimer.worker, current, claimSession, "transfer");
  }
  if (model) {
    await stampClaimModel(store, env, scope.claimer.worker, taskId, model, "transfer");
    current = (await store.getTask(taskId)) as TaskRow;
  }
  await store.touchWorker(scope.claimer.worker, true, now);
  await emitMarked(store, env, scope.claimer.worker, current, "claimed", {
    lease_seconds: leaseSeconds,
    via: "claim_repo_task_transfer",
    transfer: true,
    by_id: true,
    task_id: taskId,
    from_session: fromSession,
    session_id: claimSession,
    previous_worker: oldWorker,
    previous_lane_id: oldLane,
    previous_session_id: sessionIdOf(oldProgress),
    platform_limit: limitClearedInvalid ? null : (ledger?.platform_limit ?? null),
    platform_limit_reason: limitClearedInvalid ? null : (ledger?.platform_limit_reason ?? null),
    platform_limit_cleared_invalid: limitClearedInvalid,
    lane_id: newLane,
    platform: scope.claimer.family,
    model_provider: model?.provider ?? null,
    model_id: model?.model ?? null,
    model_variant: model?.variant ?? null,
  });
  return await store.getTask(taskId);
}

async function budgetSkipSummary(store: QueueStore, scope: ClaimScope, now: string): Promise<Resp> {
  const remaining = scope.claimer.remaining ?? -1;
  const rows: TaskRow[] = [];
  for (const t of await store.allTasks()) {
    if (!payloadNonEmpty(t)) continue;
    // The native budget query applies queue/filter/requirements
    // scope but not the platform-limit clause.
    if (!(await scopeMatch(store, t, scope, false))) continue;
    const due = t.not_before === null || t.not_before <= now;
    const liveMinusGate =
      (t.status === "queued" && t.attempts < t.max_attempts && due) ||
      (t.status === "parked" && ((t.not_before !== null && t.not_before <= now) || t.wait_kind === "capacity"));
    if (!liveMinusGate) continue;
    const required = requiredBudget(t);
    if (required !== null && required > remaining) rows.push(t);
  }
  let max: number | null = null;
  for (const t of rows) {
    const r = requiredBudget(t);
    if (r !== null && (max === null || r > max)) max = r;
  }
  return { budget_skipped: rows.length, max_required_budget_minutes: max };
}

async function noClaimCounts(store: QueueStore, scope: ClaimScope, now: string): Promise<Resp> {
  const rows: TaskRow[] = [];
  for (const t of await store.allTasks()) {
    if (payloadNonEmpty(t) && (await scopeMatch(store, t, scope, true))) rows.push(t);
  }
  return {
    queued: rows.filter((t) => t.status === "queued").length,
    at_cap_queued: rows.filter((t) => t.status === "queued" && t.attempts >= t.max_attempts).length,
    queued_not_yet_due: rows.filter(
      (t) => t.status === "queued" && t.attempts < t.max_attempts && t.not_before !== null && t.not_before > now,
    ).length,
    parked_due: rows.filter(
      (t) => t.status === "parked" && ((t.not_before !== null && t.not_before <= now) || t.wait_kind === "capacity"),
    ).length,
  };
}

// ---------------------------------------------------------------------------
// Work identity and label enrichment (enqueue-time dedupe)
// ---------------------------------------------------------------------------

interface WorkIdentity {
  kind: string;
  repo: string;
  issue: number;
  family: string;
  key: string;
}

function prNumberIn(text: string): number | null {
  const t = text.trim();
  const pullIdx = t.indexOf("/pull/");
  if (pullIdx >= 0) {
    const rest = t.slice(pullIdx + "/pull/".length);
    const digits = /^[0-9]+/.exec(rest);
    if (!digits) return null;
    const n = Number(digits[0]);
    return Number.isSafeInteger(n) ? n : null;
  }
  const hashIdx = t.lastIndexOf("#");
  if (hashIdx < 0) return null;
  const head = t.slice(0, hashIdx);
  const tail = t.slice(hashIdx + 1);
  if (!head.includes("/") || /\s/.test(head)) return null;
  if (tail === "" || !/^[0-9]+$/.test(tail)) return null;
  const n = Number(tail);
  return Number.isSafeInteger(n) ? n : null;
}

function prNumberFromValue(v: Json): number | null {
  if (typeof v === "number") return asI64(v);
  if (typeof v === "string") return prNumberIn(v);
  if (isObj(v)) {
    const n = intOrNull(v["number"]);
    if (n !== null) return n;
    if (typeof v["url"] === "string") return prNumberIn(v["url"]);
  }
  return null;
}

function workIdentity(row: { queue: string; payload: Json }): WorkIdentity | null {
  const payload = taskPayload(row);
  if (payload["dedupe_exempt"] === true) return null;
  const kind = typeof payload["kind"] === "string" ? payload["kind"] : "";
  const family =
    kind === "small_fix" ? "fix" : kind === "triage" ? "triage" : kind === "pr_shepherd" ? "pr_shepherd" : null;
  if (family === null) return null;
  const issueObj = payload["issue"];
  const issue = isObj(issueObj) ? intOrNull(issueObj["number"]) : null;
  if (issue === null || issue <= 0) return null;
  const repo = taskRepoIdentity(row);
  if (repo === null) return null;
  let prs: number[] = [];
  if (kind === "pr_shepherd") {
    const collect = (v: Json | undefined) => {
      if (v === undefined) return;
      const n = prNumberFromValue(v);
      if (n !== null && n > 0 && !prs.includes(n)) prs.push(n);
    };
    collect(payload["pr"]);
    const prsRaw = payload["prs"];
    if (Array.isArray(prsRaw)) for (const el of prsRaw) collect(el);
    prs = [...prs].sort((a, b) => a - b);
  }
  const key = `${repo}#${issue}:${family}` + (prs.length > 0 ? `:pr${prs.join(",")}` : "");
  return { kind, repo, issue, family, key };
}

function dedupeOverride(packet: JsonObject): string | null {
  const payload = packet["payload"];
  const src = isObj(payload) ? payload : packet;
  const v = src["dedupe_override"];
  if (typeof v === "string" && v.trim() !== "") return v.trim();
  return null;
}

function holdsIdentity(store: QueueStore, env: QueueEnv, row: TaskRow, identity: WorkIdentity): boolean {
  if (!LIVE_STATUSES.includes(row.status)) return false;
  const wid = workIdentity(row);
  if (wid === null || wid.key !== identity.key) return false;
  return claimGuardReason(env, row) === null;
}

async function liveIdentityHolder(store: QueueStore, env: QueueEnv, identity: WorkIdentity): Promise<TaskRow | null> {
  for (const row of await store.allTasks()) {
    const payload = taskPayload(row);
    if (payload["kind"] !== identity.kind) continue;
    const issueObj = payload["issue"];
    const n = isObj(issueObj) ? intOrNull(issueObj["number"]) : null;
    if (n !== identity.issue) continue;
    if (holdsIdentity(store, env, row, identity)) return row;
  }
  return null;
}

function collisionDetail(holder: TaskRow, identity: WorkIdentity): JsonObject {
  return {
    rule: "open_work_identity",
    identity: identity.key,
    task_id: holder.id,
    status: holder.status,
    queue: holder.queue,
    dedupe_key: holder.dedupe_key,
    hint: 'this work already has a live task; attach follow-ups to it, or file with payload.dedupe_override="<reason>" for a deliberate parallel',
  };
}

function repoFromDedupeKey(key: string | null): string | null {
  if (key === null) return null;
  let rest = key;
  if (rest.startsWith("repo-health:v1:")) rest = rest.slice("repo-health:v1:".length);
  const idx = rest.indexOf(":");
  if (idx < 0) return null;
  const tail = rest.slice(idx + 1);
  const j = tail.indexOf(":issue:");
  if (j < 0) return null;
  const repo = tail.slice(0, j);
  if (repo === "" || repo === tail) return null;
  return repo;
}

function payloadIssueRef(payload: JsonObject, dedupeKey: string | null): { repo: string; number: number } | null {
  const issue = payload["issue"];
  const num = isObj(issue) ? intOrNull(issue["number"]) : null;
  if (num === null || num <= 0) return null;
  let repo: string | null =
    typeof payload["repo"] === "string" && payload["repo"] !== "" ? (payload["repo"] as string) : null;
  if (repo === null && isObj(issue)) {
    repo = typeof issue["repo"] === "string" && issue["repo"] !== "" ? (issue["repo"] as string) : null;
  }
  if (repo === null) repo = repoFromDedupeKey(dedupeKey);
  if (repo === null || !repo.includes("/")) return null;
  return { repo, number: num };
}

/** #12744: mirror the issue's GitHub labels into payload.labels
 * when the packet carries none — the mirror never gates work: a
 * lookup failure is a warning, an absent row is silence. */
async function enrichPayloadLabels(
  store: QueueStore,
  payload: Json,
  dedupeKey: string | null,
  warnings: string[] | null,
): Promise<void> {
  if (!isObj(payload)) return;
  if (Array.isArray(payload["labels"])) return;
  const ref = payloadIssueRef(payload, dedupeKey);
  if (ref === null) return;
  let labels: string[] | null;
  try {
    labels = await store.mirrorLabels(ref.repo, ref.number);
  } catch (e) {
    warnings?.push(`payload.labels: GitHub mirror enrichment skipped (${e instanceof Error ? e.message : String(e)})`);
    return;
  }
  if (labels === null) return;
  const strings = labels.filter((l) => typeof l === "string");
  if (strings.length === 0) return;
  payload["labels"] = strings;
}

function dedupePartition(rows: TaskRow[]): { live: TaskRow | null; terminal: TaskRow | null } {
  let live: TaskRow | null = null;
  let terminal: TaskRow | null = null;
  for (const row of rows) {
    if (LIVE_STATUSES.includes(row.status)) {
      if (live === null) live = row;
    } else if (TERMINAL_STATUSES.includes(row.status)) {
      terminal = row;
    }
  }
  return { live, terminal };
}

function emptyTaskRow(fields: Partial<TaskRow> & { queue: string }): Omit<TaskRow, "id"> {
  return {
    manual_only: false,
    metadata: {},
    title: null,
    body: null,
    payload: {},
    dedupe_key: null,
    priority: null,
    status: "queued",
    not_before: null,
    worker_id: null,
    claimed_at: null,
    lease_until: null,
    lane_id: null,
    platform: null,
    attempts: 0,
    max_attempts: 1000,
    claim_provider: null,
    claim_model: null,
    claim_variant: null,
    claim_session_id: null,
    progress: {},
    result: {},
    last_error: null,
    park_kind: null,
    wait_kind: null,
    ready_condition: null,
    required_budget_minutes: null,
    last_checked_at: null,
    check_outcome: null,
    created_at: "",
    updated_at: "",
    completed_at: null,
    blocked_deps: null,
    dep_next_recheck_at: null,
    dep_blocked_until: null,
    dep_recheck_seconds: null,
    ...fields,
  };
}

// ---------------------------------------------------------------------------
// Verbs: validate / enqueue / claim
// ---------------------------------------------------------------------------

function verbValidate(env: QueueEnv, req: Req): Resp {
  const packet = (req["packet"] ?? null) as Json;
  const [errors, warnings] = validatePacket(packet, env.nowEpoch());
  return ok({ errors, warnings });
}

async function verbEnqueue(store: QueueStore, env: QueueEnv, req: Req): Promise<Resp> {
  const now = nowIso(env);
  const packetRaw = (req["packet"] ?? null) as Json;
  const [errors, warnings0] = validatePacket(packetRaw, env.nowEpoch());
  if (errors.length > 0) {
    return {
      ok: false,
      exit: 2,
      error: `packet validation failed (${errors.length} error(s))`,
      errors,
      warnings: warnings0,
    };
  }
  const queue = str(req["queue"]) ?? "";
  if (queue === "") fail(2, "enqueue: queue is required");
  if (queue.endsWith("-shadow") && req["shadow_intent"] !== true) {
    fail(
      2,
      `enqueue: queue ${queue} is a shadow queue; pass shadow_intent (CLI: --shadow-intent) to enqueue there deliberately`,
    );
  }
  const packet = clone(packetRaw) as JsonObject;
  const metadata: JsonObject =
    req["metadata"] === undefined
      ? {}
      : isObj(req["metadata"])
        ? req["metadata"]
        : fail(2, "enqueue: metadata must be a JSON object");
  const warnings = [...warnings0];
  await enrichPayloadLabels(
    store,
    (packet["payload"] ?? null) as Json,
    typeof packet["dedupe_key"] === "string" ? packet["dedupe_key"] : null,
    warnings,
  );
  const task = taskFromPacket(packet, queue);
  let platform: string | null;
  try {
    platform = parsePlatform((req["platform"] ?? packet["platform"]) as Json);
  } catch (e) {
    fail(2, `enqueue: ${e instanceof Error ? e.message : String(e)}`);
  }
  if (req["dry_run"] === true) {
    return ok({ dry_run: true, task: { ...task, platform }, warnings });
  }
  const dedupeKey = typeof task["dedupe_key"] === "string" ? task["dedupe_key"] : null;
  let existing: TaskRow | null = null;
  let predecessor: TaskRow | null = null;
  let collision: JsonObject | null = null;
  if (dedupeKey !== null) {
    const part = dedupePartition(await store.tasksByDedupe(queue, dedupeKey));
    existing = part.live;
    predecessor = part.terminal;
  }
  const identity = workIdentity({ queue, payload: (task["payload"] ?? {}) as Json });
  const override = dedupeOverride(packet);
  if (existing === null && identity !== null && override === null) {
    const holder = await liveIdentityHolder(store, env, identity);
    if (holder) {
      existing = holder;
      collision = collisionDetail(holder, identity);
    }
  }
  if (existing !== null) {
    const resp = ok({ row: existing, rpc: "enqueue_repo_task_turso", warnings });
    if (collision) resp["work_identity_collision"] = collision;
    return resp;
  }
  const progress: JsonObject = predecessor
    ? {
        revived_from: {
          task_id: predecessor.id,
          status: predecessor.status,
          completed_at: predecessor.completed_at,
          revived_at: now,
          via: "enqueue_repo_task_turso",
        },
      }
    : {};
  const row = await store.insertTask(
    emptyTaskRow({
      queue,
      manual_only: task["manual_only"] === true,
      metadata,
      title: typeof task["title"] === "string" ? task["title"] : null,
      body: typeof task["body"] === "string" ? task["body"] : null,
      payload: (task["payload"] ?? {}) as Json,
      dedupe_key: dedupeKey,
      priority: intOrNull(task["priority"]),
      status: "queued",
      not_before: typeof task["not_before"] === "string" ? task["not_before"] : null,
      platform: platform ?? null,
      attempts: 0,
      max_attempts: intOr(task["max_attempts"], 1000),
      progress,
      result: {},
      required_budget_minutes: intOrNull(task["required_budget_minutes"]),
      created_at: now,
      updated_at: now,
    }),
  );
  if (predecessor) {
    const link = (progress["revived_from"] ?? null) as Json;
    await emit(store, env, "enqueue_repo_task", row.id, "task_revived", {
      predecessor: link,
      dedupe_key: dedupeKey,
    });
    await emit(store, env, "enqueue_repo_task", predecessor.id, "revived_by", {
      successor_task_id: row.id,
      dedupe_key: dedupeKey,
    });
  }
  if (override !== null) {
    await emit(store, env, "enqueue_repo_task", row.id, "dedupe_override", {
      reason: override,
      identity: identity?.key ?? null,
    });
  }
  await emit(store, env, "enqueue_repo_task", row.id, "enqueued", {
    queue,
    dedupe_key: dedupeKey,
    manual_only: row.manual_only,
  });
  const resp = ok({ row, rpc: "enqueue_repo_task_turso", warnings });
  if (predecessor) {
    resp["revived_from"] = { task_id: predecessor.id, status: predecessor.status };
  }
  return resp;
}

function scopedQueue(req: Req): string | null {
  const q = str(req["queue"]);
  if (q === undefined || q === "" || q === "*" || q === "all") return null;
  return q;
}

async function verbClaim(store: QueueStore, env: QueueEnv, req: Req): Promise<Resp> {
  const now = nowIso(env);
  const worker = str(req["worker"]) ?? "";
  const queue = scopedQueue(req);
  const lane = rawStr(req["lane"]);
  // Lane preamble: an unknown lane auto-registers unbound; a lane
  // owned by another worker, disabled, or bound to another queue
  // refuses the claim.
  if (lane !== null) {
    let laneRow = await store.getLane(lane);
    if (laneRow === null) {
      await store.insertLane({ lane_id: lane, worker_id: worker, queue: "*", enabled: 1 });
      laneRow = await store.getLane(lane);
    }
    if (laneRow !== null) {
      if (laneRow.worker_id !== worker) {
        fail(4, `claim_repo_task: lane ${lane} belongs to worker ${laneRow.worker_id}, not ${worker}`);
      }
      if (laneRow.enabled === 0) {
        fail(4, `claim_repo_task: lane ${lane} is disabled`);
      }
      const registered = laneRow.queue ?? "";
      if (queue !== null && registered !== "*" && registered !== "all" && registered !== "" && registered !== queue) {
        fail(4, `claim_repo_task: lane ${lane} is for queue ${registered}, not ${queue}`);
      }
    }
  }
  await expireLeases(store, env);
  await quarantineSweep(store, env, queue);

  let filter: FilterExpr | null = null;
  const filterStr = str(req["filter"]);
  if (filterStr !== undefined && filterStr !== "") {
    try {
      filter = parseFilter(filterStr);
    } catch (e) {
      fail(2, `claim_repo_task: bad filter: ${e instanceof Error ? e.message : String(e)}`);
    }
  }
  const pool = rawStr(req["pool"]);
  const leaseSeconds = intOr(req["lease_seconds"], 2700);
  const remaining = intOrNull(req["remaining_budget_minutes"]);
  let family: string | null;
  try {
    family = parsePlatform(req["platform"]);
  } catch (e) {
    fail(2, `claim_repo_task: ${e instanceof Error ? e.message : String(e)}`);
  }
  const claimSession = rawStr(req["session_id"]);
  let model: ClaimedModel | null;
  try {
    model = parseClaimModel(req);
  } catch (e) {
    fail(2, `claim_repo_task: ${e instanceof Error ? e.message : String(e)}`);
  }
  const pickRaw = rawStr(req["pick"]);
  let pickMode: "default" | "handoff" = "default";
  if (pickRaw === "handed_off" || pickRaw === "parked_handoff") {
    pickMode = "handoff";
  } else if (pickRaw !== null) {
    fail(2, `claim_repo_task: unknown pick mode ${JSON.stringify(pickRaw)}`);
  }
  let count = 1;
  if (req["count"] !== undefined && req["count"] !== null) {
    const n = intOrNull(req["count"]);
    if (n === null || n < 1 || n > 5) {
      fail(
        2,
        `claim_repo_task: count ${n ?? JSON.stringify(req["count"])} is out of range: a batch claim takes 1..=5 items (#8217)`,
      );
    }
    count = n;
  }
  const taskId = intOrNull(req["task_id"]);
  const claimer: ClaimerCtx = { family, lane, worker, pool, model, remaining };
  const scope: ClaimScope = { queue, filter, claimer };

  if (count > 1) {
    if (taskId !== null) {
      fail(2, "claim_repo_task: --count cannot be combined with --task: a by-id claim takes exactly one task (#8217)");
    }
    const rows: TaskRow[] = [];
    for (let i = 0; i < count; i++) {
      const row = await claimPickOnce(store, env, scope, pickMode, leaseSeconds, claimSession, model);
      if (row === null) break;
      rows.push(row);
    }
    const resp = ok({ rows, claimed: rows.length, requested: count });
    if (rows.length === 0) Object.assign(resp, await noClaimCounts(store, scope, now));
    return resp;
  }
  if (taskId !== null) {
    const fromSession = rawStr(req["from_session"]);
    const row =
      fromSession !== null
        ? await claimTransfer(store, env, scope, taskId, fromSession, leaseSeconds, claimSession, model)
        : await claimById(store, env, scope, taskId, leaseSeconds, claimSession, model);
    return ok({ row });
  }
  const row = await claimPickOnce(store, env, scope, pickMode, leaseSeconds, claimSession, model);
  if (row !== null) return ok({ row });
  const resp: Resp = { ok: true, row: null };
  if (pickMode === "default") {
    Object.assign(resp, await budgetSkipSummary(store, scope, now));
    Object.assign(resp, await noClaimCounts(store, scope, now));
  }
  return resp;
}

// ---------------------------------------------------------------------------
// The claimed-by guard and the lifecycle verbs
// ---------------------------------------------------------------------------

async function notClaimedReason(store: QueueStore, row: TaskRow | null, worker: string): Promise<string> {
  if (row === null) return "no such task";
  let base: string;
  if (row.status === "claimed" && row.worker_id) {
    base = `it is claimed by ${row.worker_id}`;
  } else {
    base = `it is ${row.status}`;
  }
  if (row.status !== "claimed") {
    const events = (await store.eventsForTask(row.id)).filter(
      (e) => e.kind === "claimed" || e.kind === "lease_expired",
    );
    const last = events[events.length - 1];
    if (last && last.kind === "lease_expired" && last.worker_id === worker) {
      base += `; ${worker}'s lease expired at ${last.created_at} and the task went back to the queue, so claim it again before completing it (heartbeat to keep a lease; checkpoint saves progress but does not renew it)`;
    }
  }
  return base;
}

/** claimed_by: the row, when `worker` holds its live claim —
 * including the foreign-lane rule (two named lanes, neither of
 * them direct/opencode, never share a claim). */
async function claimedBy(store: QueueStore, env: QueueEnv, req: Req, verb: string): Promise<TaskRow> {
  const taskId = intOrNull(req["task_id"]) ?? 0;
  const worker = str(req["worker"]) ?? "";
  const row = await store.getTask(taskId);
  if (row !== null && row.status === "claimed" && row.worker_id === worker) {
    const callerLane = (rawStr(req["lane"]) ?? "").trim();
    const holderLane = (row.lane_id ?? "").trim();
    const generic = (s: string) => s === "" || s === "direct" || s === "opencode";
    if (!generic(callerLane) && !generic(holderLane) && callerLane !== holderLane) {
      try {
        await emit(store, env, worker, row.id, "claim_refused", {
          via: verb,
          reason: "claimed_by_another_lane",
          holder_lane_id: row.lane_id,
          caller_lane_id: callerLane,
        });
      } catch {
        // Best-effort audit (native: let _ =).
      }
      fail(5, `${verb}: task ${taskId} is not claimed by ${worker} on lane ${callerLane}: lane ${holderLane} holds it`);
    }
    return row;
  }
  fail(5, `${verb}: task ${taskId} is not claimed by ${worker}: ${await notClaimedReason(store, row, worker)}`);
}

const LANE_FAMILIES = ["code", "any", "not-code", "opencode", "openchamber", "nix", "darwin-nix"];

function laneFamilyOf(id: string): string | null {
  const idx = id.lastIndexOf("-");
  if (idx < 0) return null;
  const family = id.slice(0, idx);
  const suffix = id.slice(idx + 1);
  if (!LANE_FAMILIES.includes(family)) return null;
  if (suffix === "" || ![...suffix].every((c) => c >= "a" && c <= "z")) return null;
  return family;
}

function executionLaneFields(ex: JsonObject): { mode: string | null; laneId: string | null } {
  const lane = typeof ex["lane"] === "string" ? ex["lane"] : null;
  const laneId = typeof ex["lane_id"] === "string" ? ex["lane_id"] : null;
  if (lane !== null) {
    const family = laneFamilyOf(lane);
    if (family !== null) {
      return { mode: family === "opencode" ? "opencode" : "direct", laneId: laneId ?? lane };
    }
  }
  return { mode: lane, laneId };
}

/** verb_with_execution: after a park/fail/complete, record the
 * execution on the lane ledger when the caller reports one. */
async function verbWithExecution(store: QueueStore, env: QueueEnv, req: Req, row: TaskRow | null): Promise<Resp> {
  if (row === null) return ok({ row: null });
  const ex = req["execution"];
  if (!isObj(ex)) return ok({ row });
  const now = nowIso(env);
  const { mode, laneId } = executionLaneFields(ex);
  const fallbackReason = typeof ex["fallback_reason"] === "string" ? ex["fallback_reason"] : null;
  const sessionId = typeof ex["session_id"] === "string" ? ex["session_id"] : null;
  const existing = await store.getLedger(row.id);
  if (existing) {
    await store.updateLedger(row.id, {
      executed_lane: mode ?? existing.executed_lane,
      executed_lane_id: laneId ?? existing.executed_lane_id,
      fallback_reason: fallbackReason ?? existing.fallback_reason,
      session_id: sessionId ?? existing.session_id,
      executed_at: existing.executed_at ?? now,
      updated_at: now,
    });
  } else {
    await store.insertLedger({
      queue: row.queue,
      task_id: intOrNull(req["task_id"]) ?? 0,
      assigned_lane: mode,
      executed_lane: mode,
      executed_lane_id: laneId,
      exclusion_reason: null,
      opencode_reason: null,
      platform_limit: null,
      platform_limit_reason: null,
      assigned_by: null,
      session_id: sessionId,
      fallback_reason: fallbackReason,
      assigned_at: now,
      executed_at: now,
      updated_at: now,
    });
  }
  const ledger = await store.getLedger(row.id);
  return ledger ? ok({ row, lane_ledger: ledger }) : ok({ row });
}

async function verbHeartbeat(store: QueueStore, env: QueueEnv, req: Req): Promise<Resp> {
  const row = await claimedBy(store, env, req, "heartbeat_task");
  const worker = str(req["worker"]) ?? "";
  const now = nowIso(env);
  const progress = mergeObj(obj(row.progress), obj(req["progress"]));
  const proposed = iso(env.nowEpoch() + intOr(req["extend_seconds"], 900));
  const current = row.lease_until;
  const leaseUntil = current && Date.parse(current) >= Date.parse(proposed) ? current : proposed;
  await store.updateTask(row.id, { lease_until: leaseUntil, progress, updated_at: now });
  await store.upsertLease(
    { task_id: row.id, worker_id: worker, lease_until: leaseUntil, heartbeat_at: now },
    { keepWorker: true },
  );
  await store.touchWorker(worker, true, now);
  return ok({ row: await store.getTask(row.id) });
}

async function verbCheckpoint(store: QueueStore, env: QueueEnv, req: Req): Promise<Resp> {
  const row = await claimedBy(store, env, req, "checkpoint_task");
  const worker = str(req["worker"]) ?? "";
  const now = nowIso(env);
  const stage = str(req["stage"]) ?? "";
  const given = obj(req["progress"]);
  const progress = mergeObj(obj(row.progress), {
    ...given,
    last_checkpoint: { stage, at: now },
  });
  await store.upsertCheckpoint(worker, `task:${row.id}`, { stage, progress: given, at: now });
  await store.updateTask(row.id, { progress, updated_at: now });
  await store.touchWorker(worker, true, now);
  await emit(store, env, worker, row.id, "checkpoint", { stage, via: "checkpoint_task_turso" });
  return ok({ row: await store.getTask(row.id) });
}

function waitKindOf(condition: Json): string {
  const kind = isObj(condition) ? condition["kind"] : undefined;
  switch (kind) {
    case "time":
      return "timer";
    case "triage":
      return "triage";
    case "ci":
    case "ci_green":
    case "pr_merged":
    case "issue_closed":
    case "upstream_pr":
      return "ci";
    case "review":
    case "codex_review":
    case "pr_review":
    case "person_answer":
      return "review";
    case "capacity":
    case "quota_reset":
      return "capacity";
    default:
      return "external";
  }
}

function parkRequiredBudget(req: Req, condition: Json, payload: Json): number | null {
  const explicit = intOrNull(req["required_budget_minutes"]);
  if (explicit !== null) return explicit > 0 ? explicit : null;
  if (isObj(condition) && condition["kind"] === "capacity") {
    const tb = isObj(payload) ? intOrNull(payload["timebox_minutes"]) : null;
    return tb !== null && tb > 0 ? tb : null;
  }
  return null;
}

async function verbPark(store: QueueStore, env: QueueEnv, req: Req): Promise<Resp> {
  const row = await claimedBy(store, env, req, "park_task");
  const worker = str(req["worker"]) ?? "";
  const now = nowIso(env);
  const nowEpoch = env.nowEpoch();
  if (req["not_before"] !== undefined && req["not_before"] !== null) {
    const err = validateNotBeforeAt(req["not_before"], "not_before", nowEpoch);
    if (err) fail(2, `park_task: ${err}`);
  }
  let notBefore: string;
  let parkKind: string;
  if (typeof req["not_before"] === "string") {
    notBefore = req["not_before"];
    parkKind = "absolute";
  } else {
    notBefore = iso(nowEpoch + intOr(req["delay_seconds"], 1200));
    parkKind = "relative";
  }
  const givenProgress = obj(req["progress"]);
  let condition: Json;
  let conditionDefaulted = false;
  if (req["ready_condition"] !== undefined && req["ready_condition"] !== null) {
    condition = req["ready_condition"];
  } else if (givenProgress["ready_condition"] !== undefined && givenProgress["ready_condition"] !== null) {
    condition = givenProgress["ready_condition"];
  } else {
    condition = { kind: "time", until: notBefore };
    conditionDefaulted = true;
  }
  const wk = progressStage(givenProgress) === "awaiting_triage" ? "triage" : waitKindOf(condition);
  const floor = wk === "triage" ? 21600 : wk === "ci" || wk === "capacity" ? 3600 : null;
  if (floor !== null && notBefore < iso(nowEpoch + floor)) {
    notBefore = iso(nowEpoch + floor);
    parkKind = "relative";
  }
  if (conditionDefaulted) {
    condition = { kind: "time", until: notBefore };
  }
  const requiredBudget = parkRequiredBudget(req, condition, row.payload);
  const reason = str(req["reason"]) ?? "";
  const [progress0, cleared] = clearBinding(row.progress, "park_task", env);
  const progress = mergeObj(progress0, givenProgress);
  const result = mergeObj(obj(row.result), { park_reason: reason });
  const explicitDelay = req["not_before"] != null || req["delay_seconds"] != null;
  const toSink =
    wk !== "triage" &&
    handoffShape(progress) &&
    !sessionMarkPresent(progress) &&
    (conditionDefaulted || (isObj(condition) && condition["kind"] === "time"));
  if (toSink) {
    await store.updateTask(row.id, {
      status: "handed_off",
      worker_id: null,
      lease_until: null,
      not_before: explicitDelay ? notBefore : null,
      park_kind: null,
      wait_kind: null,
      ready_condition: null,
      required_budget_minutes: null,
      last_checked_at: now,
      check_outcome: null,
      progress,
      result,
      updated_at: now,
    });
  } else {
    await store.updateTask(row.id, {
      status: "parked",
      worker_id: null,
      lease_until: null,
      not_before: notBefore,
      park_kind: parkKind,
      wait_kind: wk,
      ready_condition: condition,
      required_budget_minutes: requiredBudget,
      last_checked_at: now,
      check_outcome: null,
      progress,
      result,
      updated_at: now,
    });
  }
  await store.deleteLease(row.id);
  const fresh = (await store.getTask(row.id)) as TaskRow;
  if (cleared) await onBindingCleared(store, env, worker, fresh, cleared, "park_task");
  if (toSink) {
    await emitMarked(store, env, worker, fresh, "parked", {
      reason,
      status: "handed_off",
      park_kind: null,
      wait_kind: null,
      not_before: explicitDelay ? notBefore : null,
      ready_condition: null,
      cleared_session_id: cleared,
    });
  } else {
    await emitMarked(store, env, worker, fresh, "parked", {
      reason,
      status: "parked",
      park_kind: parkKind,
      wait_kind: wk,
      not_before: notBefore,
      ready_condition: condition,
      required_budget_minutes: requiredBudget,
      cleared_session_id: cleared,
    });
  }
  return await verbWithExecution(store, env, req, fresh);
}

async function verbRelease(store: QueueStore, env: QueueEnv, req: Req): Promise<Resp> {
  const reason = str(req["reason"]) ?? "";
  if (reason.trim() === "") {
    fail(
      2,
      "release_repo_task: a release requires a reason (--reason): the audit event is the only record of why the item went back",
    );
  }
  const row = await claimedBy(store, env, req, "release_repo_task");
  const worker = str(req["worker"]) ?? "";
  const now = nowIso(env);
  const [progress, cleared] = clearBinding(row.progress, "release_repo_task", env);
  await store.updateTask(row.id, {
    status: "queued",
    worker_id: null,
    lease_until: null,
    not_before: null,
    park_kind: null,
    wait_kind: null,
    progress,
    updated_at: now,
  });
  await store.deleteLease(row.id);
  await store.touchWorker(worker, true, now);
  const fresh = (await store.getTask(row.id)) as TaskRow;
  if (cleared) await onBindingCleared(store, env, worker, fresh, cleared, "release_repo_task");
  await emitMarked(store, env, worker, fresh, "released", {
    reason,
    via: "release_repo_task_turso",
    cleared_session_id: cleared,
  });
  return ok({ row: fresh });
}

async function verbWake(store: QueueStore, env: QueueEnv, req: Req): Promise<Resp> {
  const taskId = intOrNull(req["task_id"]) ?? 0;
  const worker = str(req["worker"]) ?? "fleet-queue-cli";
  const reason = str(req["reason"]) ?? "";
  const row = await store.getTask(taskId);
  if (row === null) fail(5, `wake_task: task ${taskId} does not exist`);
  if (row.status === "blocked" && depBlockActive(row)) {
    await depRecheckOne(store, env, row, new Map());
    return ok({ row: await store.getTask(taskId) });
  }
  if (row.status === "handed_off") {
    fail(
      5,
      `wake_task: task ${taskId}: task status handed_off cannot be woken: a handed_off row is not a wait — it is already claimable, claim it instead`,
    );
  }
  if (row.status !== "parked") {
    fail(5, `wake_task: task ${taskId}: task status ${row.status} cannot be woken (only parked waits)`);
  }
  const updated = await store.conditionalUpdateTask(
    row.id,
    { status: "parked" },
    {
      status: "queued",
      not_before: null,
      park_kind: null,
      wait_kind: null,
      worker_id: null,
      lease_until: null,
      updated_at: nowIso(env),
    },
  );
  const fresh = (await store.getTask(taskId)) as TaskRow;
  if (updated) {
    await emitMarked(store, env, worker, fresh, "woken", {
      reason,
      previous_status: row.status,
      previous_wait_kind: row.wait_kind,
      via: "wake_task_turso",
    });
  }
  return ok({ row: fresh });
}

// --- Block deps and the dep-recheck machinery.

function depsOf(row: TaskRow): JsonObject[] {
  const raw = row.blocked_deps;
  if (Array.isArray(raw)) return raw.filter((d): d is JsonObject => isObj(d)).map((d) => ({ ...d }));
  if (typeof raw === "string") {
    try {
      const parsed = JSON.parse(raw) as Json;
      if (Array.isArray(parsed)) return parsed.filter((d): d is JsonObject => isObj(d));
    } catch {
      return [];
    }
  }
  return [];
}

function depBlockActive(row: TaskRow): boolean {
  return depsOf(row).length > 0;
}

function depKey(dep: JsonObject): string {
  const kind = typeof dep["kind"] === "string" ? dep["kind"] : "";
  const repo = typeof dep["repo"] === "string" ? dep["repo"] : "?";
  if (kind === "pr" || kind === "issue") return `${kind}:${repo}#${intOrNull(dep["number"]) ?? 0}`;
  if (kind === "ci") return `ci:${repo}#${intOrNull(dep["run_id"]) ?? 0}`;
  if (kind === "task") return `task:${intOrNull(dep["task_id"]) ?? 0}`;
  return `${kind}:?`;
}

function validDepRepo(repo: string): boolean {
  if (/\s/.test(repo)) return false;
  const parts = repo.split("/");
  return parts.length === 2 && parts.every((p) => p !== "");
}

function parseBlockDeps(req: Req, worker: string, now: string): JsonObject[] | null {
  const raw = req["deps"];
  if (raw === undefined || raw === null) return null;
  if (!Array.isArray(raw)) fail(2, "block: deps must be an array");
  if (raw.length === 0) return null;
  const deps: JsonObject[] = [];
  for (const d of raw) {
    const dep = isObj(d) ? d : {};
    const kind = typeof dep["kind"] === "string" ? dep["kind"] : "";
    const record: JsonObject = {
      kind,
      state: "open",
      reason: typeof dep["reason"] === "string" ? dep["reason"] : "",
      recorded_by: worker,
      recorded_at: now,
      last_checked_at: null,
      last_observed: null,
    };
    if (kind === "pr" || kind === "issue" || kind === "ci") {
      const repo = typeof dep["repo"] === "string" ? dep["repo"] : "";
      if (!validDepRepo(repo)) fail(2, `block: dep repo must be owner/name, got '${repo}'`);
      record["repo"] = repo;
      if (kind === "ci") {
        const runId = intOrNull(dep["run_id"]);
        if (runId === null || runId <= 0) fail(2, "block: ci dep needs a positive run_id");
        record["run_id"] = runId;
      } else {
        const number = intOrNull(dep["number"]);
        if (number === null || number <= 0) fail(2, `block: ${kind} dep needs a positive number`);
        record["number"] = number;
      }
    } else if (kind === "task") {
      const depTaskId = intOrNull(dep["task_id"]);
      if (depTaskId === null || depTaskId <= 0) fail(2, "block: task dep needs a positive task_id");
      record["task_id"] = depTaskId;
    } else {
      fail(2, `block: unknown dep kind '${kind}' (pr|issue|task|ci)`);
    }
    deps.push(record);
  }
  return deps;
}

function clampRange(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v));
}

async function verbBlock(store: QueueStore, env: QueueEnv, req: Req): Promise<Resp> {
  const row = await claimedBy(store, env, req, "block_task");
  const worker = str(req["worker"]) ?? "";
  const now = nowIso(env);
  const nowEpoch = env.nowEpoch();
  const blocker = str(req["blocker"]) ?? "";
  const reasonClass = str(req["reason_class"]) ?? "";
  let deps = parseBlockDeps(req, worker, now);
  let depEvent: JsonObject = {};
  let depNextRecheckAt: string | null = null;
  let depBlockedUntil: string | null = null;
  let depRecheckSeconds: number | null = null;
  if (deps !== null) {
    depRecheckSeconds = clampRange(intOr(req["dep_recheck_seconds"], 900), 300, 3600);
    const ttl = clampRange(intOr(req["dep_max_ttl_seconds"], 259200), 60, 604800);
    depBlockedUntil = iso(nowEpoch + ttl);
    depNextRecheckAt = now;
    depEvent = { deps, dep_blocked_until: depBlockedUntil, dep_recheck_seconds: depRecheckSeconds };
  }
  const progress = mergeObj(mergeObj(obj(row.progress), obj(req["progress"])), {
    blocker,
    reason_class: reasonClass,
    blocked_via: "lane",
  });
  const result = mergeObj(obj(row.result), {
    blocker,
    reason_class: reasonClass,
    blocked_via: "lane",
  });
  await store.updateTask(row.id, {
    status: "blocked",
    worker_id: null,
    lease_until: null,
    progress,
    result,
    blocked_deps: deps,
    dep_next_recheck_at: depNextRecheckAt,
    dep_blocked_until: depBlockedUntil,
    dep_recheck_seconds: depRecheckSeconds,
    last_error: null,
    updated_at: now,
  });
  await store.deleteLease(row.id);
  const fresh = (await store.getTask(row.id)) as TaskRow;
  await emit(store, env, worker, row.id, "blocked", mergeObj({ blocker, reason_class: reasonClass }, depEvent));
  return ok({ row: fresh });
}

async function taskDepObs(store: QueueStore, depTaskId: number | null): Promise<DepObs> {
  const task = depTaskId !== null ? await store.getTask(depTaskId) : null;
  if (task === null) return { tag: "broken", evidence: `task ${depTaskId ?? 0} not found` };
  switch (task.status) {
    case "done":
      return { tag: "cleared", evidence: "dep task done" };
    case "cancelled":
      return { tag: "cleared", evidence: "dep task cancelled; dependent released for re-evaluation" };
    case "failed":
      return { tag: "cleared", evidence: "dep task failed; dependent released for re-evaluation" };
    default:
      return { tag: "open", evidence: `dep task status ${task.status}` };
  }
}

function probeDep(probe: QueueDepProbe, dep: JsonObject): DepObs {
  const repo = typeof dep["repo"] === "string" ? dep["repo"] : "";
  if (dep["kind"] === "issue") return probe.probeIssue(repo, intOrNull(dep["number"]) ?? 0);
  if (dep["kind"] === "pr") return probe.probePr(repo, intOrNull(dep["number"]) ?? 0);
  return probe.probeCi(repo, intOrNull(dep["run_id"]) ?? 0);
}

function applyDepObs(dep: JsonObject, obs: DepObs, now: string): "cleared" | "broken" | null {
  const state = typeof dep["state"] === "string" ? dep["state"] : "open";
  if (state !== "open") return null;
  dep["last_checked_at"] = now;
  if (obs.tag === "open" || obs.tag === "failed") {
    dep["last_observed"] = obs.evidence;
    return null;
  }
  dep["state"] = obs.tag;
  dep["cleared_at"] = now;
  dep["last_observed"] = obs.evidence;
  return obs.tag;
}

function depsAllResolved(deps: JsonObject[]): boolean {
  return (
    deps.length > 0 &&
    deps.every((d) => {
      const s = typeof d["state"] === "string" ? d["state"] : "open";
      return s === "cleared" || s === "broken";
    })
  );
}

interface DepTransition {
  dep: string;
  transition: string;
  evidence: Json;
}

async function emitDepTransitions(
  store: QueueStore,
  env: QueueEnv,
  task: TaskRow,
  transitions: DepTransition[],
): Promise<void> {
  for (const t of transitions) {
    await emitMarked(store, env, "dep-rechecker", task, t.transition === "cleared" ? "dep_cleared" : "dep_broken", {
      dep: t.dep,
      evidence: t.evidence,
      via: "dep_recheck",
    });
  }
}

async function depRecheckOne(
  store: QueueStore,
  env: QueueEnv,
  row: TaskRow,
  cache: Map<string, DepObs>,
): Promise<[string, number]> {
  const deps = depsOf(row);
  if (deps.length === 0) return ["not_dep_blocked", 0];
  const now = nowIso(env);
  const nowEpoch = env.nowEpoch();
  if (row.dep_blocked_until !== null && row.dep_blocked_until <= now) {
    const updated = await store.conditionalUpdateTask(
      row.id,
      { status: "blocked" },
      {
        status: "queued",
        blocked_deps: null,
        dep_next_recheck_at: null,
        dep_blocked_until: null,
        dep_recheck_seconds: null,
        updated_at: now,
      },
    );
    if (updated) {
      const fresh = (await store.getTask(row.id)) as TaskRow;
      await emitMarked(store, env, "dep-rechecker", fresh, "dep_ttl_expired", {
        deps,
        dep_blocked_until: row.dep_blocked_until,
        via: "dep_recheck",
      });
      return ["ttl_expired", 0];
    }
    return ["still_blocked", 0];
  }
  let failures = 0;
  const transitions: DepTransition[] = [];
  for (const dep of deps) {
    const state = typeof dep["state"] === "string" ? dep["state"] : "open";
    if (state !== "open") continue;
    const kind = typeof dep["kind"] === "string" ? dep["kind"] : "";
    let obs: DepObs;
    if (kind === "task") {
      obs = await taskDepObs(store, intOrNull(dep["task_id"]));
    } else if (kind === "issue" || kind === "pr" || kind === "ci") {
      const key = depKey(dep);
      const cached = cache.get(key);
      if (cached) {
        obs = cached;
      } else {
        obs = probeDep(env.depProbe, dep);
        cache.set(key, obs);
      }
    } else {
      obs = { tag: "broken", evidence: `unknown dep kind '${kind}'` };
    }
    if (obs.tag === "failed") failures++;
    const transition = applyDepObs(dep, obs, now);
    if (transition) {
      transitions.push({ dep: depKey(dep), transition, evidence: (dep["last_observed"] ?? null) as Json });
    }
  }
  if (depsAllResolved(deps)) {
    const updated = await store.conditionalUpdateTask(
      row.id,
      { status: "blocked" },
      {
        status: "queued",
        blocked_deps: null,
        dep_next_recheck_at: null,
        dep_blocked_until: null,
        dep_recheck_seconds: null,
        updated_at: now,
      },
    );
    if (!updated) return ["still_blocked", failures];
    const fresh = (await store.getTask(row.id)) as TaskRow;
    await emitDepTransitions(store, env, fresh, transitions);
    await emitMarked(store, env, "dep-rechecker", fresh, "dep_unblocked", {
      deps,
      transitions: transitions as unknown as Json,
      via: "dep_recheck",
    });
    return ["unblocked", failures];
  }
  const recheckSeconds = row.dep_recheck_seconds ?? 900;
  let next = iso(nowEpoch + recheckSeconds);
  if (row.dep_blocked_until !== null && row.dep_blocked_until < next) {
    next = row.dep_blocked_until;
  }
  await store.updateTask(row.id, {
    blocked_deps: deps,
    dep_next_recheck_at: next,
    updated_at: now,
  });
  const fresh = (await store.getTask(row.id)) as TaskRow;
  await emitDepTransitions(store, env, fresh, transitions);
  return ["still_blocked", failures];
}

async function verbDepRecheck(store: QueueStore, env: QueueEnv, req: Req): Promise<Resp> {
  const taskId = intOrNull(req["task_id"]);
  if (taskId !== null) {
    const row = await store.getTask(taskId);
    if (row === null) return ok({ row: { task_id: taskId, outcome: "no_such_task" } });
    if (row.status !== "blocked" || !depBlockActive(row)) {
      return ok({ row: { task_id: taskId, outcome: "not_dep_blocked" } });
    }
    const [outcome, failures] = await depRecheckOne(store, env, row, new Map());
    return ok({
      row: { task_id: taskId, outcome, check_failures: failures, row: await store.getTask(taskId) },
    });
  }
  const limit = intOr(req["limit"], 200);
  const now = nowIso(env);
  const due = (await store.allTasks())
    .filter(
      (t) =>
        t.status === "blocked" &&
        t.blocked_deps !== null &&
        (t.dep_next_recheck_at === null || t.dep_next_recheck_at <= now),
    )
    .sort((a, b) => {
      const ka = a.dep_next_recheck_at ?? "";
      const kb = b.dep_next_recheck_at ?? "";
      return ka < kb ? -1 : ka > kb ? 1 : 0;
    })
    .slice(0, limit);
  const cache = new Map<string, DepObs>();
  let checked = 0;
  let unblocked = 0;
  let ttlExpired = 0;
  let stillBlocked = 0;
  let checkFailures = 0;
  for (const row of due) {
    checked++;
    const [outcome, failures] = await depRecheckOne(store, env, row, cache);
    checkFailures += failures;
    if (outcome === "unblocked") unblocked++;
    else if (outcome === "ttl_expired") ttlExpired++;
    else if (outcome === "still_blocked") stillBlocked++;
  }
  return ok({
    row: {
      checked,
      unblocked,
      ttl_expired: ttlExpired,
      still_blocked: stillBlocked,
      check_failures: checkFailures,
    },
  });
}

async function verbUnblock(store: QueueStore, env: QueueEnv, req: Req): Promise<Resp> {
  const taskId = intOrNull(req["task_id"]) ?? 0;
  const worker = str(req["worker"]) ?? "";
  const reason = str(req["reason"]) ?? "";
  const row = await store.getTask(taskId);
  if (row === null) fail(5, `unblock_task: task ${taskId} does not exist`);
  if (row.status !== "blocked") {
    fail(5, `unblock_task: task ${taskId} is ${row.status}, not blocked`);
  }
  const prevDeps = depsOf(row);
  await store.conditionalUpdateTask(
    row.id,
    { status: "blocked" },
    {
      status: "queued",
      blocked_deps: null,
      dep_next_recheck_at: null,
      dep_blocked_until: null,
      dep_recheck_seconds: null,
      updated_at: nowIso(env),
    },
  );
  const fresh = (await store.getTask(taskId)) as TaskRow;
  if (prevDeps.length === 0) {
    await emitMarked(store, env, worker, fresh, "unblocked", { reason, via: "unblock_task_turso" });
  } else {
    await emitMarked(store, env, worker, fresh, "dep_unblock_override", {
      reason,
      discarded_deps: prevDeps,
      via: "unblock_task_turso",
    });
  }
  return ok({ row: fresh });
}

async function verbCancel(store: QueueStore, env: QueueEnv, req: Req): Promise<Resp> {
  const taskId = intOrNull(req["task_id"]) ?? 0;
  const worker = str(req["worker"]) ?? "fleet-monitor";
  const reason = str(req["reason"]) ?? "";
  const row = await store.getTask(taskId);
  if (row === null) fail(5, `cancel_repo_task: task ${taskId} does not exist`);
  if (TERMINAL_STATUSES.includes(row.status)) return ok({ row });
  const result = mergeObj(obj(row.result), { cancel_reason: reason });
  await store.updateTask(row.id, {
    status: "cancelled",
    worker_id: null,
    lease_until: null,
    result,
    updated_at: nowIso(env),
  });
  await store.deleteLease(row.id);
  const fresh = (await store.getTask(taskId)) as TaskRow;
  await emit(store, env, worker, taskId, "cancelled", { reason, via: "cancel_repo_task_turso" });
  const sid = sessionIdOf(progressOf(row));
  if (sid !== null) await recordSessionUnbound(store, env, worker, fresh, sid, "cancel");
  return ok({ row: fresh });
}

async function verbFail(store: QueueStore, env: QueueEnv, req: Req): Promise<Resp> {
  const row = await claimedBy(store, env, req, "fail_repo_task");
  const worker = str(req["worker"]) ?? "";
  const now = nowIso(env);
  const error = str(req["error"]) ?? "";
  const reasonClass = str(req["reason_class"]) ?? "transient";
  const maxAttempts = row.max_attempts;
  const requeue = reasonClass === "transient" && row.attempts < maxAttempts;
  const newStatus = requeue ? "queued" : reasonClass === "blocked" ? "blocked" : "failed";
  const notBefore = requeue ? iso(env.nowEpoch() + (row.attempts <= 1 ? 900 : 2700)) : (row.not_before ?? "");
  let progress: Json = row.progress;
  let cleared: string | null = null;
  if (requeue) {
    const [p, c] = clearBinding(row.progress, "fail_repo_task", env);
    progress = p;
    cleared = c;
  }
  const result = mergeObj(obj(row.result), { reason_class: reasonClass });
  await store.updateTask(row.id, {
    status: newStatus,
    worker_id: null,
    lease_until: null,
    not_before: notBefore,
    last_error: error,
    progress,
    result,
    updated_at: now,
  });
  await store.deleteLease(row.id);
  const fresh = (await store.getTask(row.id)) as TaskRow;
  if (requeue && cleared) await onBindingCleared(store, env, worker, fresh, cleared, "fail_repo_task");
  await emitMarked(store, env, worker, fresh, "failed", {
    error,
    reason_class: reasonClass,
    requeued: requeue,
    via: "fail_repo_task_turso",
  });
  return await verbWithExecution(store, env, req, fresh);
}

// ---------------------------------------------------------------------------
// The completion gate (disposition.rs, ported) and verbComplete
// ---------------------------------------------------------------------------

const VERDICTS = [
  "closed_verified",
  "merged_verified",
  "open_draft_with_followup",
  "open_draft_superseded",
  "invalid_no_work",
];
const DISPOSITION_STALENESS_SECONDS = 1800;
const DISPOSITION_FUTURE_SKEW_SECONDS = 300;

interface DispositionPull {
  number: number;
  state: string;
  mergedAt: string | null;
}

interface FollowUp {
  reason: string;
  taskId: number | null;
  successorPr: number | null;
  issueUrl: string | null;
  issueNumber: number | null;
  issueState: string | null;
  issueLinks: number[];
}

interface DispositionRecord {
  checkedAt: string;
  checkedAtEpoch: number;
  issueNumber: number;
  issueState: string;
  stateReason: string | null;
  closedBy: string | null;
  pulls: DispositionPull[];
  verdict: string;
  followUp: FollowUp | null;
}

function trimmedStr(v: Json | undefined): string | null {
  if (typeof v !== "string") return null;
  const t = v.trim();
  return t === "" ? null : t;
}

function parseFollowUp(v: Json): FollowUp {
  if (!isObj(v)) throw new Error("disposition.follow_up is not an object");
  const reason = trimmedStr(v["reason"]);
  if (reason === null) throw new Error("disposition.follow_up.reason missing");
  const taskId = intOrNull(v["task_id"]);
  const successorPr = intOrNull(v["successor_pr"]);
  const issueUrl = trimmedStr(v["issue_url"]);
  let issueNumber: number | null = null;
  let issueState: string | null = null;
  let issueLinks: number[] = [];
  if (issueUrl !== null) {
    const readback = "an issue follow-up needs its readback; re-run `repo-queue verify-disposition --follow-up-issue`";
    issueNumber = intOrNull(v["issue_number"]);
    if (issueNumber === null) {
      throw new Error(`disposition.follow_up.issue_number missing: ${readback}`);
    }
    issueState = trimmedStr(v["issue_state"]);
    if (issueState === null) {
      throw new Error(`disposition.follow_up.issue_state missing: ${readback}`);
    }
    if (issueState !== "open" && issueState !== "closed") {
      throw new Error(`disposition.follow_up.issue_state ${JSON.stringify(issueState)} is not open|closed`);
    }
    const links = v["issue_links"];
    if (!Array.isArray(links)) {
      throw new Error(`disposition.follow_up.issue_links missing: ${readback}`);
    }
    issueLinks = links.map((l) => intOrNull(l)).filter((n): n is number => n !== null);
  }
  if (taskId === null && successorPr === null && issueUrl === null) {
    throw new Error("disposition.follow_up needs task_id, issue_url or successor_pr");
  }
  return { reason, taskId, successorPr, issueUrl, issueNumber, issueState, issueLinks };
}

function parseDispositionRecord(v: Json): DispositionRecord {
  if (!isObj(v)) throw new Error("disposition is not an object");
  const checkedAt = trimmedStr(v["checked_at"]);
  if (checkedAt === null) throw new Error("disposition.checked_at missing");
  const checkedAtEpoch = parseRfc3339Epoch(checkedAt);
  if (checkedAtEpoch === null) {
    throw new Error(`disposition.checked_at ${JSON.stringify(checkedAt)} is not RFC 3339`);
  }
  if (trimmedStr(v["source"]) === null) throw new Error("disposition.source missing");
  const issue = v["issue"];
  if (!isObj(issue)) throw new Error("disposition.issue missing");
  const issueNumber = intOrNull(issue["number"]);
  if (issueNumber === null) throw new Error("disposition.issue.number missing");
  const issueState = trimmedStr(issue["state"]);
  if (issueState === null) throw new Error("disposition.issue.state missing");
  if (issueState !== "open" && issueState !== "closed") {
    throw new Error(`disposition.issue.state ${JSON.stringify(issueState)} is not open|closed`);
  }
  const pulls: DispositionPull[] = [];
  const pullsRaw = v["pulls"];
  if (pullsRaw !== undefined && pullsRaw !== null) {
    if (!Array.isArray(pullsRaw)) throw new Error("disposition.pulls is not an array");
    for (const p of pullsRaw) {
      const po = isObj(p) ? p : {};
      const number = intOrNull(po["number"]);
      if (number === null) throw new Error("disposition.pulls[].number missing");
      const state = trimmedStr(po["state"]);
      if (state === null) throw new Error(`disposition.pulls[${number}].state missing`);
      if (state !== "open" && state !== "closed" && state !== "merged") {
        throw new Error(`disposition.pulls[${number}].state ${JSON.stringify(state)} is not open|closed|merged`);
      }
      let mergedAt: string | null = null;
      const mergedRaw = po["merged_at"];
      if (mergedRaw !== undefined && mergedRaw !== null) {
        if (typeof mergedRaw !== "string" || parseRfc3339Epoch(mergedRaw) === null) {
          throw new Error(`disposition.pulls[${number}].merged_at ${JSON.stringify(mergedRaw)} is not RFC 3339`);
        }
        mergedAt = mergedRaw;
      }
      pulls.push({ number, state, mergedAt });
    }
  }
  const verdict = trimmedStr(v["verdict"]);
  if (verdict === null) throw new Error("disposition.verdict missing");
  if (!VERDICTS.includes(verdict)) {
    throw new Error(
      `disposition.verdict ${JSON.stringify(verdict)} is not one of closed_verified|merged_verified|open_draft_with_followup|open_draft_superseded|invalid_no_work`,
    );
  }
  const fuRaw = v["follow_up"];
  const followUp = fuRaw === undefined || fuRaw === null ? null : parseFollowUp(fuRaw);
  return {
    checkedAt,
    checkedAtEpoch,
    issueNumber,
    issueState,
    stateReason: trimmedStr(issue["state_reason"]),
    closedBy: trimmedStr(issue["closed_by"]),
    pulls,
    verdict,
    followUp,
  };
}

function checkFollowUp(rec: DispositionRecord, fu: FollowUp, unfinished: DispositionPull[]): void {
  if (fu.issueUrl !== null && fu.issueNumber !== null) {
    const n = fu.issueNumber;
    if (n === rec.issueNumber) {
      throw new Error(`follow_up issue #${n} is the issue being completed; a follow-up must be a different item`);
    }
    if (fu.issueState !== "open") {
      throw new Error(`follow_up issue #${n} is ${fu.issueState}; a closed issue cannot own the remaining work`);
    }
    if (unfinished.length === 0) {
      if (!fu.issueLinks.includes(rec.issueNumber)) {
        throw new Error(
          `follow_up issue #${n} does not link #${rec.issueNumber} with an explicit marker (Refs/Fixes/Related program); an unrelated issue owns nothing`,
        );
      }
    } else {
      for (const p of unfinished) {
        if (!fu.issueLinks.includes(p.number)) {
          throw new Error(
            `follow_up issue #${n} does not link unfinished PR #${p.number} with an explicit marker (Refs #${p.number}); it does not own that remainder`,
          );
        }
      }
    }
  }
  if (fu.successorPr !== null) {
    const pr = rec.pulls.find((p) => p.number === fu.successorPr);
    if (!pr) {
      throw new Error(
        `follow_up successor PR #${fu.successorPr} is not in the readback for issue #${rec.issueNumber}; a successor must link the issue`,
      );
    }
    if (pr.mergedAt === null) {
      throw new Error(
        `follow_up successor PR #${fu.successorPr} is not merged; only a merged successor resolves the remainder`,
      );
    }
  }
}

function checkDisposition(
  rec: DispositionRecord,
  expectedIssue: number | null,
  linkedPrs: number[],
  nowEpoch: number,
): void {
  if (rec.checkedAtEpoch > nowEpoch + DISPOSITION_FUTURE_SKEW_SECONDS) {
    throw new Error(`disposition.checked_at ${JSON.stringify(rec.checkedAt)} is in the future; unreadable readback`);
  }
  const age = nowEpoch - rec.checkedAtEpoch;
  if (age > DISPOSITION_STALENESS_SECONDS) {
    throw new Error(
      `disposition.checked_at ${JSON.stringify(rec.checkedAt)} is ${age}s old (bound ${DISPOSITION_STALENESS_SECONDS}s); re-run \`repo-queue verify-disposition\``,
    );
  }
  if (expectedIssue !== null && rec.issueNumber !== expectedIssue) {
    throw new Error(`disposition describes issue #${rec.issueNumber} but the task is for #${expectedIssue}`);
  }
  if (rec.verdict === "closed_verified" && rec.issueState !== "closed") {
    throw new Error(
      `verdict closed_verified but issue #${rec.issueNumber} is ${rec.issueState}; a close older than the latest reopen is not closure evidence`,
    );
  }
  if (rec.verdict === "merged_verified" && !rec.pulls.some((p) => p.mergedAt !== null)) {
    throw new Error(`verdict merged_verified but no pull in the readback is merged (issue #${rec.issueNumber})`);
  }
  if (
    (rec.verdict === "open_draft_with_followup" || rec.verdict === "open_draft_superseded") &&
    rec.followUp === null
  ) {
    throw new Error(
      `verdict ${rec.verdict} needs a recorded follow_up (task_id, issue_url or successor_pr, plus reason)`,
    );
  }
  for (const n of linkedPrs) {
    if (!rec.pulls.some((p) => p.number === n)) {
      throw new Error(
        `linked PR #${n} is not in the readback; the disposition must cover every PR the completion claims (issue #${rec.issueNumber})`,
      );
    }
  }
  const unfinished = rec.pulls.filter((p) => p.mergedAt === null && p.state !== "closed");
  if (rec.followUp !== null) checkFollowUp(rec, rec.followUp, unfinished);
  if (unfinished.length > 0) {
    const list = unfinished.map((p) => `#${p.number}`).join(", ");
    if (rec.verdict === "invalid_no_work") {
      throw new Error(`verdict invalid_no_work but linked PR ${list} still carries work (issue #${rec.issueNumber})`);
    }
    if (
      rec.issueState === "open" &&
      rec.verdict !== "open_draft_with_followup" &&
      rec.verdict !== "open_draft_superseded"
    ) {
      throw new Error(
        `issue #${rec.issueNumber} is OPEN with open/unmerged linked PR ${list}; a terminal exit needs verdict open_draft_with_followup or open_draft_superseded plus a recorded follow_up`,
      );
    }
    if (rec.followUp === null) {
      throw new Error(
        `issue #${rec.issueNumber} is ${rec.issueState} with open/unmerged linked PR ${list}; record a follow_up that owns the remainder (a live task_id, an open issue_url that links ${list}, or a merged successor_pr)`,
      );
    }
    if (rec.issueState === "closed") {
      if (rec.closedBy === null) {
        throw new Error(
          `issue #${rec.issueNumber} is closed with open/unmerged linked PR ${list} but the readback does not say who closed it (issue.closed_by); re-run \`repo-queue verify-disposition\``,
        );
      }
      if (rec.closedBy.endsWith("[bot]")) {
        throw new Error(
          `issue #${rec.issueNumber} was closed by ${rec.closedBy}, an automation account, with open/unmerged linked PR ${list}; an automated close is not an authorized disposition`,
        );
      }
      if (rec.stateReason === null) {
        throw new Error(
          `issue #${rec.issueNumber} is closed with open/unmerged linked PR ${list} but the readback has no state_reason; an intentional close records why`,
        );
      }
    }
  }
}

async function checkSuccessorTask(store: QueueStore, rec: DispositionRecord, completingTaskId: number): Promise<void> {
  const fu = rec.followUp;
  if (fu === null || fu.taskId === null) return;
  const want = fu.taskId;
  const found = await store.getTask(want);
  if (found === null) throw new Error(`follow_up task ${want} does not exist in the queue`);
  if (found.id === completingTaskId) {
    throw new Error(`follow_up task ${want} is the task being completed; it cannot be its own successor`);
  }
  if (!LIVE_STATUSES.includes(found.status)) {
    throw new Error(`follow_up task ${want} is ${found.status}; a terminal task owns nothing`);
  }
  const foundIssue = taskIssue(taskPayload(found), found.dedupe_key);
  if (foundIssue === rec.issueNumber) return;
  if (foundIssue !== null) {
    throw new Error(
      `follow_up task ${want} is for issue #${foundIssue}, not #${rec.issueNumber}; an unrelated task does not own this remainder`,
    );
  }
  throw new Error(
    `follow_up task ${want} names no issue, so it cannot be shown to own #${rec.issueNumber}'s remainder`,
  );
}

function issueNumberFromDedupe(key: string, repo: string): number | null {
  const marker = `:${repo}:issue:`;
  const idx = key.indexOf(marker);
  if (idx < 0) return null;
  const rest = key.slice(idx + marker.length);
  const digits = /^[0-9]+/.exec(rest);
  if (!digits) return null;
  const tail = rest.slice(digits[0].length);
  if (tail !== "" && !tail.startsWith(":")) return null;
  const n = Number(digits[0]);
  return Number.isSafeInteger(n) ? n : null;
}

const LEGACY_REPO_ALIAS: Record<string, string> = {
  "developing-today-forks/opencode": "developing-today/opencode",
  "developing-today/opencode": "developing-today-forks/opencode",
  "developing-today-forks/openchamber": "developing-today/openchamber",
  "developing-today/openchamber": "developing-today-forks/openchamber",
  "dezren39/darwin-nix": "dezren39/nix",
  "dezren39/nix": "dezren39/darwin-nix",
};

function taskIssue(payload: JsonObject, dedupeKey: string | null): number | null {
  const issue = payload["issue"];
  if (isObj(issue)) {
    const n = intOrNull(issue["number"]);
    if (n !== null) return n;
  }
  const repo = payload["repo"];
  if (typeof repo !== "string" || dedupeKey === null) return null;
  const direct = issueNumberFromDedupe(dedupeKey, repo);
  if (direct !== null) return direct;
  const alias = LEGACY_REPO_ALIAS[repo];
  if (alias) return issueNumberFromDedupe(dedupeKey, alias);
  return null;
}

function followOnKey(dedupeKey: string | null): string | null {
  if (dedupeKey === null) return null;
  if (!dedupeKey.includes(":triage:") || !dedupeKey.includes(":issue:")) return null;
  return dedupeKey.replace(":triage:", ":fix:");
}

function collectPrNumbers(v: Json, out: number[]): void {
  if (typeof v === "string") {
    const n = prNumberIn(v);
    if (n !== null && !out.includes(n)) out.push(n);
  } else if (typeof v === "number") {
    const n = asI64(v);
    if (n !== null && !out.includes(n)) out.push(n);
  } else if (Array.isArray(v)) {
    for (const el of v) collectPrNumbers(el, out);
  } else if (isObj(v)) {
    const n = intOrNull(v["number"]);
    if (n !== null && !out.includes(n)) out.push(n);
    for (const val of Object.values(v)) collectPrNumbers(val, out);
  }
}

function linkedPrs(result: JsonObject): number[] {
  const out: number[] = [];
  for (const key of ["pr", "pr_url", "merged_pr", "pull_request", "evidence", "commit", "commit_sha"]) {
    const v = result[key];
    if (v !== undefined) collectPrNumbers(v, out);
  }
  return out;
}

function evidenceRef(result: JsonObject): boolean {
  for (const key of ["evidence", "pr_url", "pr", "merged_pr", "commit", "commit_sha", "verified_closure"]) {
    const v = result[key];
    if (typeof v === "string" && v.trim() !== "") return true;
    if (Array.isArray(v) && v.length > 0) return true;
    if (isObj(v)) return true;
  }
  return false;
}

/** require_verified: returns the parsed record, or the refusal
 * message (the caller fails exit 2 with it — the task remains
 * claimed). */
async function requireVerified(
  store: QueueStore,
  resultIn: JsonObject,
  expectedIssue: number | null,
  nowEpoch: number,
  completingTaskId: number,
): Promise<{ record: DispositionRecord } | { error: string }> {
  const raw = resultIn["disposition"];
  if (raw === undefined || raw === null) {
    return {
      error:
        "complete_task: this completion requires a verified disposition (result.disposition: checked_at, source, issue, pulls, verdict); run `repo-queue verify-disposition <issue> --json`, add follow_up when work remains, and complete again — the task remains claimed",
    };
  }
  let record: DispositionRecord;
  try {
    record = parseDispositionRecord(raw);
  } catch (e) {
    return {
      error: `complete_task: result.disposition is not well-formed (${e instanceof Error ? e.message : String(e)}); the task remains claimed`,
    };
  }
  try {
    checkDisposition(record, expectedIssue, linkedPrs(resultIn), nowEpoch);
  } catch (e) {
    return { error: `complete_task: ${e instanceof Error ? e.message : String(e)}; the task remains claimed` };
  }
  if (record.followUp !== null && record.followUp.taskId !== null) {
    try {
      await checkSuccessorTask(store, record, completingTaskId);
    } catch (e) {
      return { error: `complete_task: ${e instanceof Error ? e.message : String(e)}; the task remains claimed` };
    }
  }
  return { record };
}

type CompletionClass = "fix" | "slice" | "triage" | "ungated";

function completionClass(payload: JsonObject): CompletionClass {
  switch (payload["kind"]) {
    case "small_fix":
    case "ci_investigation":
    case "pr_shepherd":
      return "fix";
    case "feature_slice":
      return "slice";
    case "triage":
      return "triage";
    default:
      return "ungated";
  }
}

async function verbComplete(store: QueueStore, env: QueueEnv, req: Req): Promise<Resp> {
  const row = await claimedBy(store, env, req, "complete_task");
  const worker = str(req["worker"]) ?? "";
  const now = nowIso(env);
  const nowEpoch = env.nowEpoch();
  const payload = taskPayload(row);
  const cls = completionClass(payload);
  const resultIn = obj(req["result"]);
  const expectedIssue = taskIssue(payload, row.dedupe_key);
  const fixKey = followOnKey(row.dedupe_key);
  let outcome: "follow_on" | "invalid" | "already_solved" | null = null;
  if (cls === "fix") {
    if (!evidenceRef(resultIn)) {
      fail(
        2,
        "complete_task: fix-class completion requires an evidence reference on the result (merged PR, commit, or verified-closure note); the task remains claimed",
      );
    }
    const v = await requireVerified(store, resultIn, expectedIssue, nowEpoch, row.id);
    if ("error" in v) fail(2, v.error);
  } else if (cls === "slice") {
    if (!evidenceRef(resultIn)) {
      fail(
        2,
        "complete_task: feature_slice completion requires an evidence reference on the result (merged PR, commit, or verified-closure note); the task remains claimed",
      );
    }
  } else if (cls === "triage") {
    const oc = typeof resultIn["triage_outcome"] === "string" ? resultIn["triage_outcome"] : "follow_on";
    if (oc === "follow_on") {
      if (fixKey !== null || expectedIssue === null) {
        outcome = "follow_on";
      } else {
        const v = await requireVerified(store, resultIn, expectedIssue, nowEpoch, row.id);
        if ("error" in v) {
          fail(2, `${v.error} (this triage key has no fix follow-on, so the handoff must be on the record)`);
        }
        if (v.record.issueState === "open" && v.record.followUp === null) {
          fail(
            2,
            `complete_task: triage of open issue #${expectedIssue} enqueues no fix follow-on for its key and records no follow_up; name the live task or open issue that owns the work (\`repo-queue verify-disposition --follow-up-task|--follow-up-issue\`); the task remains claimed`,
          );
        }
        outcome = "follow_on";
      }
    } else if (oc === "invalid") {
      const note = resultIn["triage_note"];
      if (!evidenceRef(resultIn) && !(typeof note === "string" && note !== "")) {
        fail(
          2,
          "complete_task: triage exit 'invalid' requires the validation evidence on the result (evidence or triage_note); the task remains claimed",
        );
      }
      outcome = "invalid";
    } else if (oc === "already_solved") {
      if (!evidenceRef(resultIn)) {
        fail(
          2,
          "complete_task: triage exit 'already_solved' requires an evidence reference on the result (merged PR, commit, or verified-closure note); the task remains claimed",
        );
      }
      const v = await requireVerified(store, resultIn, expectedIssue, nowEpoch, row.id);
      if ("error" in v) fail(2, v.error);
      outcome = "already_solved";
    } else {
      fail(2, `complete_task: unknown triage_outcome ${JSON.stringify(oc)} (follow_on | invalid | already_solved)`);
    }
  }
  const result = mergeObj(obj(row.result), resultIn);
  await store.updateTask(row.id, {
    status: "done",
    lease_until: null,
    result,
    completed_at: now,
    updated_at: now,
  });
  await store.deleteLease(row.id);
  await store.touchWorker(worker, false, now);
  const fresh = (await store.getTask(row.id)) as TaskRow;
  await emitMarked(store, env, worker, fresh, "completed", { via: "complete_task_turso" });
  const sid = sessionIdOf(progressOf(row));
  if (sid !== null) await recordSessionUnbound(store, env, worker, fresh, sid, "complete");

  let followOnId: number | null = null;
  if (outcome === "follow_on" && cls === "triage") {
    if (fixKey === null) {
      await emit(store, env, worker, row.id, "triage_follow_on_skipped", {
        reason: "dedupe key has no fix-family counterpart",
        dedupe_key: row.dedupe_key,
      });
    } else {
      const part = dedupePartition(await store.tasksByDedupe(row.queue, fixKey));
      if (part.live !== null) {
        followOnId = part.live.id;
      } else {
        const fpayload = clone(payload);
        fpayload["kind"] = "small_fix";
        fpayload["triage_follow_on"] = { from_task: row.id, at: now };
        await enrichPayloadLabels(store, fpayload, fixKey, null);
        const fprogress: JsonObject = { triage_source_task: row.id };
        if (part.terminal !== null) {
          fprogress["revived_from"] = {
            task_id: part.terminal.id,
            status: part.terminal.status,
            via: "triage_completion_follow_on",
          };
        }
        const created = await store.insertTask(
          emptyTaskRow({
            queue: row.queue,
            title: row.title,
            body: row.body,
            payload: fpayload,
            dedupe_key: fixKey,
            priority: labelBandPriority(fpayload) ?? row.priority,
            status: "queued",
            max_attempts: intOrNull(fpayload["max_attempts"]) ?? 1000,
            progress: fprogress,
            result: {},
            required_budget_minutes: intOrNull(fpayload["required_budget_minutes"]),
            created_at: now,
            updated_at: now,
          }),
        );
        followOnId = created.id;
        await emit(store, env, worker, created.id, "enqueued", {
          via: "triage_completion_follow_on",
          source_task: row.id,
          dedupe_key: fixKey,
        });
      }
      await emit(store, env, worker, row.id, "triage_follow_on_enqueued", {
        follow_on_task_id: followOnId,
        dedupe_key: fixKey,
      });
    }
  }
  const resp = await verbWithExecution(store, env, req, await store.getTask(row.id));
  if (followOnId !== null) resp["follow_on_task_id"] = followOnId;
  return resp;
}

// ---------------------------------------------------------------------------
// Lane assignment and session binding
// ---------------------------------------------------------------------------

function boundSession(sessionId: string | null, cleared: string | null): string | null {
  if (sessionId === null || sessionId.trim() === "") return null;
  if (sessionId === cleared) return null;
  return sessionId;
}

async function verbLane(store: QueueStore, env: QueueEnv, req: Req): Promise<Resp> {
  const taskId = intOrNull(req["task_id"]) ?? 0;
  const worker = str(req["worker"]) ?? "";
  const exclusion = rawStr(req["exclusion_reason"]);
  const opencodeReason = rawStr(req["opencode_reason"]);
  const limitPlatform = rawStr(req["platform_limit"]);
  const limitReason = rawStr(req["platform_limit_reason"]);
  const clearLimit = req["clear_platform_limit"] === true;
  const task = await store.getTask(taskId);
  if (task === null) fail(4, `assign_repo_task_lane: task ${taskId} does not exist`);
  if (!clearLimit) {
    try {
      const err = platformLimitError(limitPlatform, limitReason);
      if (err) throw new Error(err);
    } catch (e) {
      fail(4, `assign_repo_task_lane: ${e instanceof Error ? e.message : String(e)}`);
    }
  }
  const now = nowIso(env);
  const applyLimit = async (): Promise<void> => {
    if (clearLimit) {
      await store.updateLedger(taskId, { platform_limit: null, platform_limit_reason: null, updated_at: now });
      const t = (await store.getTask(taskId)) as TaskRow;
      await emitMarked(store, env, worker, t, "lane_assigned", {
        platform_limit: null,
        platform_limit_reason: null,
        platform_limit_cleared: true,
        via: "assign_repo_task_lane_turso",
      });
    } else if (limitPlatform !== null) {
      let fam: string | null = null;
      try {
        fam = parsePlatform(limitPlatform as unknown as Json);
      } catch {
        fam = null;
      }
      await store.updateLedger(taskId, { platform_limit: fam, platform_limit_reason: limitReason, updated_at: now });
      const t = (await store.getTask(taskId)) as TaskRow;
      await emitMarked(store, env, worker, t, "lane_assigned", {
        platform_limit: fam,
        platform_limit_reason: limitReason,
        via: "assign_repo_task_lane_turso",
      });
    }
  };
  const existing = await store.getLedger(taskId);
  if (existing !== null) {
    let lane = existing.assigned_lane ?? "";
    const bound = boundSession(existing.session_id, clearedSessionId(task));
    const opencodeBlank = opencodeReason === null || opencodeReason.trim() === "";
    if (lane === "opencode" && exclusion !== null && bound === null) {
      await store.updateLedger(taskId, {
        assigned_lane: "direct",
        session_id: null,
        exclusion_reason: exclusion,
        updated_at: now,
      });
      const t = (await store.getTask(taskId)) as TaskRow;
      await emitMarked(store, env, worker, t, "lane_assigned", {
        assigned_lane: "direct",
        previous_lane: "opencode",
        exclusion_reason: exclusion,
        opencode_reason: null,
        reason: "exclusion",
        reassignment: true,
        via: "assign_repo_task_lane_turso",
      });
      lane = "direct";
    } else if (lane === "direct" && !opencodeBlank && bound === null) {
      await store.updateLedger(taskId, {
        assigned_lane: "opencode",
        session_id: null,
        opencode_reason: opencodeReason,
        exclusion_reason: null,
        updated_at: now,
      });
      const t = (await store.getTask(taskId)) as TaskRow;
      await emitMarked(store, env, worker, t, "lane_assigned", {
        assigned_lane: "opencode",
        previous_lane: "direct",
        exclusion_reason: null,
        opencode_reason: opencodeReason,
        reason: "opencode_reason",
        reassignment: true,
        via: "assign_repo_task_lane_turso",
      });
      lane = "opencode";
    }
    if (exclusion !== null && lane === "opencode" && bound !== null) {
      fail(
        4,
        `assign_repo_task_lane: task ${taskId} is bound to session ${bound} (live, not cleared) — the #8187 exclusion flip is refused for a live bound session; its owner must release or transfer it`,
      );
    }
    if (!opencodeBlank && lane === "direct" && bound !== null) {
      fail(
        4,
        `assign_repo_task_lane: task ${taskId} is bound to session ${bound} (live, not cleared) — the #8210 opencode flip is refused for a live bound session; its owner must release or transfer it`,
      );
    }
    const curProgress = progressOf(task);
    const curLane = typeof curProgress["lane"] === "string" ? curProgress["lane"] : "";
    if (curLane !== lane) {
      await store.updateTask(taskId, {
        progress: mergeObj(obj(task.progress), { lane }),
        updated_at: now,
      });
    }
    await applyLimit();
    return ok({ row: await store.getLedger(taskId) });
  }
  const lane = exclusion !== null ? "direct" : opencodeReason !== null ? "opencode" : "direct";
  const reason = exclusion !== null ? "exclusion" : opencodeReason !== null ? "opencode_reason" : "default_direct";
  await store.insertLedger({
    queue: task.queue,
    task_id: taskId,
    assigned_lane: lane,
    executed_lane: null,
    executed_lane_id: null,
    exclusion_reason: exclusion,
    opencode_reason: opencodeReason,
    platform_limit: null,
    platform_limit_reason: null,
    assigned_by: worker,
    session_id: null,
    fallback_reason: null,
    assigned_at: now,
    executed_at: null,
    updated_at: now,
  });
  await store.updateTask(taskId, {
    progress: mergeObj(obj(task.progress), { lane }),
    updated_at: now,
  });
  const t = (await store.getTask(taskId)) as TaskRow;
  await emitMarked(store, env, worker, t, "lane_assigned", {
    assigned_lane: lane,
    exclusion_reason: exclusion,
    opencode_reason: opencodeReason,
    reason,
    via: "assign_repo_task_lane_turso",
  });
  await applyLimit();
  return ok({ row: await store.getLedger(taskId) });
}

function validSessionId(s: string): boolean {
  return s.length > 4 && s.startsWith("ses_") && [...s].every((c) => /[A-Za-z0-9_]/.test(c));
}

async function verbBind(store: QueueStore, env: QueueEnv, req: Req): Promise<Resp> {
  const taskId = intOrNull(req["task_id"]) ?? 0;
  const worker = str(req["worker"]) ?? "";
  const sid = str(req["session_id"]) ?? "";
  const launchedBy = str(req["launched_by"]) ?? "opencode-claimant";
  if (!validSessionId(sid)) {
    fail(
      2,
      `bind_repo_task_lane_session: invalid session_id ${JSON.stringify(sid)} (expected ses_ + alphanumeric/underscore)`,
    );
  }
  let model: ClaimedModel | null;
  try {
    model = parseClaimModel(req);
  } catch (e) {
    fail(2, `bind_repo_task_lane_session: ${e instanceof Error ? e.message : String(e)}`);
  }
  const task = await store.getTask(taskId);
  if (task === null) fail(4, `bind_repo_task_lane_session: task ${taskId} does not exist`);
  const now = nowIso(env);
  // Recovery branch (migration-0026/0027 contract): the claimant
  // uses these reserved launched_by values only after the old
  // session is verified deleted-while-zero-message, verified
  // absent, or verified error-stopped, and only after the task
  // lease has expired. Matching the prior session id in BOTH
  // stores before changing either makes recovery an atomic
  // compare-and-clear: it clears the binding, consumes one
  // attempt, and records stage 'launch_failed' so the task is
  // retryable through the handoff shape. The error-stopped mode
  // (#16066) was previously unrecognized here and in
  // migration-0026, so error-stopped recovery deleted the session
  // but never cleared the binding.
  const recoveryReasons: Record<string, string> = {
    "opencode-claimant-recovery:zero-message-deleted": "verified zero-message session deleted before recovery",
    "opencode-claimant-recovery:gone-session": "bound session verified absent before recovery",
    "opencode-claimant-recovery:error-stopped": "verified error-stopped session deleted before recovery",
  };
  const recoveryReason = recoveryReasons[launchedBy];
  if (recoveryReason !== undefined) {
    if (worker !== "opencode-claimant") {
      fail(4, `bind_repo_task_lane_session: recovery must be performed by opencode-claimant`);
    }
    const ledger = await store.getLedger(taskId);
    if (ledger === null) {
      fail(4, `bind_repo_task_lane_session: no lane ledger row for task ${taskId}`);
    }
    if (task.lease_until !== null && task.lease_until > now) {
      fail(
        4,
        `bind_repo_task_lane_session: recovery for task ${taskId} requires an expired lease (lease_until ${task.lease_until})`,
      );
    }
    const progress = progressOf(task);
    if (
      task.status !== "claimed" ||
      progress["lane"] !== "opencode" ||
      progress["session_id"] !== sid ||
      ledger.assigned_lane !== "opencode" ||
      ledger.session_id !== sid
    ) {
      fail(4, `bind_repo_task_lane_session: recovery binding mismatch for task ${taskId}`);
    }
    if (task.attempts >= task.max_attempts) {
      fail(4, `bind_repo_task_lane_session: recovery attempts exhausted for task ${taskId}`);
    }
    await store.updateLedger(taskId, { session_id: null, updated_at: now });
    await store.updateTask(taskId, {
      attempts: task.attempts + 1,
      progress: mergeObj(progress, {
        session_id: null,
        launched_by: null,
        stage: "launch_failed",
        launch_failed_session_id: sid,
        launch_failed_at: now,
        launch_failed_reason: recoveryReason,
      }),
      updated_at: now,
    });
    await emit(store, env, worker, taskId, "lane_session_recovered", {
      session_id: sid,
      reason: recoveryReason,
      stage: "launch_failed",
      via: "bind_repo_task_lane_session",
    });
    return ok({ row: await store.getLedger(taskId) });
  }
  if (!(task.status === "claimed" && task.lease_until !== null && task.lease_until > now)) {
    fail(
      4,
      `bind_repo_task_lane_session: task ${taskId} is not claimed with a live lease (status ${task.status}, lease_until ${task.lease_until ?? ""})`,
    );
  }
  const ledger = await store.getLedger(taskId);
  if (ledger === null) {
    fail(4, `bind_repo_task_lane_session: no lane ledger row for task ${taskId}`);
  }
  if (ledger.session_id !== null && ledger.session_id !== sid) {
    fail(4, `bind_repo_task_lane_session: task ${taskId} already bound to session ${ledger.session_id}`);
  }
  await store.updateLedger(taskId, { session_id: sid, updated_at: now });
  await store.updateTask(taskId, {
    progress: mergeObj(obj(task.progress), { session_id: sid, launched_by: launchedBy }),
    updated_at: now,
  });
  const fresh = (await store.getTask(taskId)) as TaskRow;
  await recordSessionBound(store, env, worker, fresh, sid, "bind");
  if (model) await stampClaimModel(store, env, worker, taskId, model, "bind");
  const fresh2 = (await store.getTask(taskId)) as TaskRow;
  await emitMarked(store, env, worker, fresh2, "lane_session_bound", {
    session_id: sid,
    launched_by: launchedBy,
    via: "bind_repo_task_lane_session_turso",
  });
  return ok({ row: await store.getLedger(taskId) });
}

// ---------------------------------------------------------------------------
// Admin verbs: revive / list / show / health / events
// ---------------------------------------------------------------------------

async function verbRevive(store: QueueStore, env: QueueEnv, req: Req): Promise<Resp> {
  const taskId = intOrNull(req["task_id"]) ?? 0;
  const worker = str(req["worker"]) ?? "fleet-queue-cli";
  const reason = str(req["reason"]) ?? "";
  if (req["not_before"] !== undefined && req["not_before"] !== null) {
    const err = validateNotBeforeAt(req["not_before"], "not_before", env.nowEpoch());
    if (err) fail(2, `revive_task: ${err}`);
  }
  const notBefore = str(req["not_before"]) ?? nowIso(env);
  const row = await store.getTask(taskId);
  if (row === null) fail(5, `revive_task: task ${taskId} does not exist`);
  if (row.status === "failed") {
    // eligible
  } else if (row.status === "queued" && row.attempts >= row.max_attempts) {
    // eligible
  } else if (row.status === "queued") {
    fail(
      5,
      `revive_task: task ${taskId}: task is queued with attempts ${row.attempts} < max_attempts ${row.max_attempts}: not dead at cap, nothing to revive`,
    );
  } else {
    fail(
      5,
      `revive_task: task ${taskId}: task status ${row.status} cannot be revived (only failed, or queued dead-at-cap)`,
    );
  }
  await store.updateTask(taskId, {
    status: "queued",
    attempts: 0,
    last_error: null,
    not_before: notBefore,
    worker_id: null,
    lease_until: null,
    updated_at: nowIso(env),
  });
  const fresh = (await store.getTask(taskId)) as TaskRow;
  await emit(store, env, worker, taskId, "revived", {
    reason,
    previous_status: row.status,
    previous_attempts: row.attempts,
    not_before: notBefore,
    via: "revive_task_turso",
  });
  return ok({ row: fresh });
}

const LIST_BASE_KEYS: (keyof TaskRow)[] = [
  "id",
  "queue",
  "title",
  "priority",
  "status",
  "dedupe_key",
  "created_at",
  "not_before",
  "platform",
];
const LIST_RICH_KEYS: (keyof TaskRow)[] = [
  "worker_id",
  "claimed_at",
  "lease_until",
  "lane_id",
  "attempts",
  "max_attempts",
  "claim_provider",
  "claim_model",
  "claim_variant",
  "claim_session_id",
  "progress",
  "payload",
  "blocked_deps",
  "dep_next_recheck_at",
  "dep_blocked_until",
];

async function verbList(store: QueueStore, env: QueueEnv, req: Req): Promise<Resp> {
  void env;
  const queue = str(req["queue"]) ?? "";
  let limit = 20;
  if (req["limit"] !== undefined && req["limit"] !== null) {
    const n = intOrNull(req["limit"]);
    if (n === null || n < 1 || n > 500) {
      fail(2, `argument --limit: value ${JSON.stringify(req["limit"])} out of range 1..=500`);
    }
    limit = n;
  }
  const rich = req["rich"] === true;
  let rows = await store.allTasks();
  if (queue !== "") rows = rows.filter((t) => t.queue === queue);
  const status = str(req["status"]);
  if (status !== undefined) rows = rows.filter((t) => t.status === status);
  const lane = str(req["lane"]);
  if (lane !== undefined) rows = rows.filter((t) => t.lane_id === lane);
  const workerF = str(req["worker"]);
  if (workerF !== undefined) rows = rows.filter((t) => t.worker_id === workerF);
  const nb = str(req["not_before"]);
  if (nb !== undefined) rows = rows.filter((t) => t.not_before === null || t.not_before <= nb);
  const taskF = intOrNull(req["task"]);
  if (taskF !== null) rows = rows.filter((t) => t.id === taskF);
  const dk = str(req["dedupe_key"]);
  if (dk !== undefined) rows = rows.filter((t) => t.dedupe_key === dk);
  rows.sort(
    (a, b) =>
      (a.priority === null ? 1 : 0) - (b.priority === null ? 1 : 0) ||
      (a.priority ?? 0) - (b.priority ?? 0) ||
      (a.created_at < b.created_at ? -1 : a.created_at > b.created_at ? 1 : 0) ||
      a.id - b.id,
  );
  const total = rows.length;
  const offset = Math.max(intOr(req["offset"], 0), 0);
  const page = rows.slice(offset, offset + limit).map((t) => {
    const out: JsonObject = {};
    for (const k of LIST_BASE_KEYS) out[k] = (t[k] ?? null) as Json;
    if (rich) for (const k of LIST_RICH_KEYS) out[k] = (t[k] ?? null) as Json;
    return out;
  });
  return ok({ rows: page, total, truncated: offset + page.length < total });
}

async function verbShow(store: QueueStore, env: QueueEnv, req: Req): Promise<Resp> {
  void env;
  const taskId = intOrNull(req["task_id"]) ?? 0;
  const row = await store.getTask(taskId);
  if (row === null) fail(5, `show_task: task ${taskId} does not exist`);
  return ok({ row });
}

interface HealthEntry extends JsonObject {
  queue: string;
  queued: number;
  at_cap_queued: number;
  claimed: number;
  parked: number;
  malformed_parked: number;
  handed_off: number;
  blocked: number;
  dep_blocked: number;
  done: number;
  cancelled: number;
  failed: number;
  total: number;
  oldest_queued_at: string | null;
  terminal_events_24h: number;
  terminal_events_7d: number;
}

async function verbHealth(store: QueueStore, env: QueueEnv, req: Req): Promise<Resp> {
  const queueFilter = str(req["queue"]);
  const nowEpoch = env.nowEpoch();
  const entries = new Map<string, HealthEntry>();
  const ensure = (q: string): HealthEntry => {
    let e = entries.get(q);
    if (!e) {
      e = {
        queue: q,
        queued: 0,
        at_cap_queued: 0,
        claimed: 0,
        parked: 0,
        malformed_parked: 0,
        handed_off: 0,
        blocked: 0,
        dep_blocked: 0,
        done: 0,
        cancelled: 0,
        failed: 0,
        total: 0,
        oldest_queued_at: null,
        terminal_events_24h: 0,
        terminal_events_7d: 0,
      };
      entries.set(q, e);
    }
    return e;
  };
  const countKeys = ["queued", "claimed", "parked", "handed_off", "blocked", "done", "cancelled", "failed"] as const;
  for (const t of await store.allTasks()) {
    if (queueFilter !== undefined && t.queue !== queueFilter) continue;
    const e = ensure(t.queue);
    e.total += 1;
    if ((countKeys as readonly string[]).includes(t.status)) {
      e[t.status as (typeof countKeys)[number]] += 1;
    }
    if (t.status === "queued") {
      if (t.attempts >= t.max_attempts) e.at_cap_queued += 1;
      if (e.oldest_queued_at === null || t.created_at < e.oldest_queued_at) {
        e.oldest_queued_at = t.created_at;
      }
    }
    if (t.status === "blocked" && t.blocked_deps !== null) e.dep_blocked += 1;
    if (
      t.status === "parked" &&
      t.not_before !== null &&
      validateNotBeforeAt(t.not_before, "not_before", nowEpoch) !== null
    ) {
      e.malformed_parked += 1;
    }
  }
  const since7 = iso(nowEpoch - 7 * 86400);
  const since24 = iso(nowEpoch - 24 * 3600);
  for (const ev of await store.allEvents()) {
    if (ev.kind !== "completed" && ev.kind !== "failed") continue;
    if (ev.task_id === null) continue;
    if (!(ev.created_at > since7)) continue;
    const task = await store.getTask(ev.task_id);
    if (task === null) continue;
    if (queueFilter !== undefined && task.queue !== queueFilter) continue;
    const e = ensure(task.queue);
    e.terminal_events_7d += 1;
    if (ev.created_at > since24) e.terminal_events_24h += 1;
  }
  const d = await store.describe();
  return ok({
    project_netloc: d.netloc,
    project_source: d.projectSource,
    backend: d.backend,
    rows: [...entries.values()],
  });
}

function eventRowJson(e: EventRow): JsonObject {
  return {
    id: e.id,
    task_id: e.task_id,
    worker_id: e.worker_id,
    kind: e.kind,
    detail: e.detail,
    created_at: e.created_at,
  };
}

async function verbEventsAppend(store: QueueStore, env: QueueEnv, req: Req): Promise<Resp> {
  const kind = str(req["kind"]);
  if (kind === undefined || kind.trim() === "") fail(2, "kind is required");
  const row = await store.appendEvent(
    rawStr(req["worker_id"]),
    intOrNull(req["task_id"]),
    kind,
    (req["detail"] ?? {}) as Json,
    nowIso(env),
  );
  return ok({ row: { id: row.id, kind: row.kind } });
}

async function verbEventsSeen(store: QueueStore, env: QueueEnv, req: Req): Promise<Resp> {
  void env;
  const taskId = intOrNull(req["task_id"]);
  const kind = str(req["kind"]);
  if (taskId === null || kind === undefined) fail(2, "task_id and kind are required");
  const matches = (await store.eventsForTask(taskId)).filter((e) => e.kind === kind);
  let latest: string | null = null;
  for (const e of matches) if (latest === null || e.created_at > latest) latest = e.created_at;
  return ok({
    row: { task_id: taskId, kind, seen: matches.length > 0, count: matches.length, latest_created_at: latest },
  });
}

async function verbEventsCount(store: QueueStore, env: QueueEnv, req: Req): Promise<Resp> {
  void env;
  const kind = str(req["kind"]);
  const since = str(req["since"]);
  const count = (await store.allEvents()).filter(
    (e) => (kind === undefined || e.kind === kind) && (since === undefined || e.created_at > since),
  ).length;
  return ok({ row: { count } });
}

async function verbEventsList(store: QueueStore, env: QueueEnv, req: Req): Promise<Resp> {
  void env;
  const taskId = intOrNull(req["task_id"]);
  const maxLimit = taskId !== null ? 1000 : 50000;
  const limit = clampRange(intOr(req["limit"], 200), 1, maxLimit);
  const kind = str(req["kind"]);
  const since = str(req["since"]);
  const rows = (await store.allEvents())
    .filter(
      (e) =>
        (taskId === null || e.task_id === taskId) &&
        (kind === undefined || e.kind === kind) &&
        (since === undefined || e.created_at >= since),
    )
    .slice(0, limit)
    .map(eventRowJson);
  return ok({ row: { rows, task_id: taskId } });
}

async function verbEventsPrune(store: QueueStore, env: QueueEnv, req: Req): Promise<Resp> {
  const keepDays = clampRange(intOr(req["keep_days"], 30), 1, 3650);
  const cutoff = iso(env.nowEpoch() - keepDays * 86400);
  const deleted = await store.deleteEventsBefore(cutoff);
  await store.appendEvent("prune_events", null, "events_pruned", { deleted, cutoff, keep_days: keepDays }, nowIso(env));
  return ok({ row: { deleted, cutoff } });
}

// ---------------------------------------------------------------------------
// Dispatch (the broker's pre-dispatch gates, then the verb)
// ---------------------------------------------------------------------------

const DURATION_GATES: Record<string, { field: string; flag: string; max: number }> = {
  claim: { field: "lease_seconds", flag: "lease-seconds", max: 86400 },
  heartbeat: { field: "extend_seconds", flag: "extend-seconds", max: 86400 },
  park: { field: "delay_seconds", flag: "delay-seconds", max: 604800 },
};

function validateDurationsGate(verb: string, req: Req): void {
  const gate = DURATION_GATES[verb];
  if (!gate) return;
  const raw = req[gate.field];
  if (raw === undefined || raw === null) return;
  const n = asI64(raw);
  if (n === null || n < 1 || n > gate.max) {
    fail(2, `argument --${gate.flag}: value ${JSON.stringify(raw)} out of range 1..=${gate.max}`);
  }
}

function validateNotBeforeGate(env: QueueEnv, verb: string, req: Req): void {
  if (verb !== "park" && verb !== "revive" && verb !== "list") return;
  const nb = req["not_before"];
  if (nb === undefined || nb === null) return;
  const err = validateNotBeforeAt(nb, "not_before", env.nowEpoch());
  if (err) fail(2, err);
}

export const QUEUE_VERBS = [
  "validate",
  "enqueue",
  "claim",
  "lane",
  "bind",
  "heartbeat",
  "checkpoint",
  "park",
  "release",
  "wake",
  "block",
  "unblock",
  "dep-recheck",
  "cancel",
  "fail",
  "complete",
  "list",
  "show",
  "timeline",
  "health",
  "revive",
  "events-append",
  "events-seen",
  "events-count",
  "events-prune",
] as const;

export type QueueVerb = (typeof QUEUE_VERBS)[number];

/**
 * Run one broker verb against the store. The response IS the
 * broker envelope: {ok:true, ...} on success, {ok:false, exit,
 * error} on refusal — the native broker answers HTTP 200 with this
 * envelope for every verb outcome, and so does the hosted door.
 */
export async function executeQueueVerb(store: QueueStore, verb: string, req: Req, env: QueueEnv): Promise<Resp> {
  try {
    validateDurationsGate(verb, req);
    validateNotBeforeGate(env, verb, req);
    switch (verb) {
      case "validate":
        return verbValidate(env, req);
      case "enqueue":
        return await verbEnqueue(store, env, req);
      case "claim":
        return await verbClaim(store, env, req);
      case "lane":
        return await verbLane(store, env, req);
      case "bind":
        return await verbBind(store, env, req);
      case "heartbeat":
        return await verbHeartbeat(store, env, req);
      case "checkpoint":
        return await verbCheckpoint(store, env, req);
      case "park":
        return await verbPark(store, env, req);
      case "release":
        return await verbRelease(store, env, req);
      case "wake":
        return await verbWake(store, env, req);
      case "block":
        return await verbBlock(store, env, req);
      case "unblock":
        return await verbUnblock(store, env, req);
      case "dep-recheck":
        return await verbDepRecheck(store, env, req);
      case "cancel":
        return await verbCancel(store, env, req);
      case "fail":
        return await verbFail(store, env, req);
      case "complete":
        return await verbComplete(store, env, req);
      case "list":
        return await verbList(store, env, req);
      case "show":
        return await verbShow(store, env, req);
      case "timeline":
        return await verbEventsList(store, env, req);
      case "health":
        return await verbHealth(store, env, req);
      case "revive":
        return await verbRevive(store, env, req);
      case "events-append":
        return await verbEventsAppend(store, env, req);
      case "events-seen":
        return await verbEventsSeen(store, env, req);
      case "events-count":
        return await verbEventsCount(store, env, req);
      case "events-prune":
        return await verbEventsPrune(store, env, req);
      default:
        return { ok: false, exit: 2, error: `unknown command /${verb}` };
    }
  } catch (e) {
    if (e instanceof QueueFail) {
      return { ok: false, exit: e.exit, error: e.message };
    }
    return { ok: false, exit: 4, error: e instanceof Error ? e.message : String(e) };
  }
}
