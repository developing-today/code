// Actions-exec answer applier — S3 of #14043 (design
// doc/2026-11-24_design_pm14043_answer_writeback.md §3/§5/§6).
//
// The needs-drew store is the only place of record for Drew's answer.
// This module carries a recorded answer for a `fleet-actions-exec`
// alert to the broker's decide verb, once, with a signed envelope:
//
//   {handle, approve, answer_id, answered_at, sig}
//
// where sig is the lowercase hex HMAC-SHA256 over
// `handle|approve|answer_id|answered_at` (approve as "true"/"false",
// answered_at the house `YYYY-MM-DDTHH:MM:SSZ` the broker's `iso()`
// produces). The HMAC key is resolved by NAME through fleet-secrets
// (Bitwarden); the value never appears in the repo, logs or ledger.
//
// Exactly-once (§3): the ledger below records each answer_id with one
// of pending | applied | superseded | rejected | failed. An answer_id
// reaches the broker at most once in the `applied` state. The broker's
// existing decide outcomes read as terminal:
//
//   decided:true                       -> applied
//   silence-approved                   -> superseded (note, not error)
//   fail 5 "no extension request..."   -> superseded (settled) or
//                                        rejected (see §3 table)
//   expired decision grace             -> superseded (settle-first)
//
// A second answer for the same handle with a different answer_id is
// recorded as superseded and never sent (double-tap guard).
//
// Failure behaviour here is S3 only: transient broker/throat failures
// stay `failed` for retry with bounded backoff; S4 adds the alert
// (`actions-exec-apply:<answer_id>`) and the re-raise when the throat
// returns. This module takes an injectable decide caller + key so
// tests never touch the network or the secrets chain.

import { createHmac } from "node:crypto";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import { listAnswers, needsDrewAnswerHandle, type NeedsDrewQuestion } from "./needs-drew.ts";
import { readBrokerToken } from "./queue.ts";
import { resolveBwsSecret } from "./secrets.ts";

/** fleet-secrets name of the applier HMAC key (design §2; the broker
 *  default is `pm-aggregator.actions-exec-apply-key`, overridable via
 *  `FLEET_ACTIONS_EXEC_APPLY_KEY_NAME`). */
export const APPLY_KEY_SECRET_NAME_DEFAULT = "pm-aggregator.actions-exec-apply-key";

export function applyKeySecretName(env: Record<string, string | undefined> = process.env): string {
  const v = env.FLEET_ACTIONS_EXEC_APPLY_KEY_NAME?.trim();
  return v ? v : APPLY_KEY_SECRET_NAME_DEFAULT;
}

export type ApplyState = "pending" | "applied" | "superseded" | "rejected" | "failed";

export interface ApplyRecord {
  answerId: string;
  ledgerId: string;
  handle: string;
  approve: boolean;
  answeredAt: string;
  state: ApplyState;
  /** Broker outcome that settled the row (approved/denied/...). */
  outcome?: string;
  /** "signed-apply" for applier calls. */
  via?: string;
  note?: string;
  updatedAt: string;
}

interface ApplyFileShape {
  version: 1;
  records: ApplyRecord[];
}

/** Canonical signed message — byte-identical to the broker's
 *  `signed_decide_message` (S1): `handle|approve|answer_id|answered_at`. */
export function signedDecideMessage(handle: string, approve: boolean, answerId: string, answeredAt: string): string {
  return `${handle}|${approve ? "true" : "false"}|${answerId}|${answeredAt}`;
}

/** Lowercase hex HMAC-SHA256 over the canonical message. */
export function signDecide(
  key: Uint8Array | string,
  handle: string,
  approve: boolean,
  answerId: string,
  answeredAt: string,
): string {
  return createHmac("sha256", key)
    .update(signedDecideMessage(handle, approve, answerId, answeredAt))
    .digest("hex");
}

/** House ISO stamp the broker parses (`YYYY-MM-DDTHH:MM:SSZ`). */
export function toHouseIso(atMs: number): string {
  return new Date(atMs).toISOString().slice(0, 19) + "Z";
}

/** Map a recorded answer to its approve boolean. Tier-2 offers
 *  deny/approve-now; tier-3 approve/deny. Unknown option shapes are
 *  not an approval — the caller records them `rejected`. */
export function approveFromAnswer(question: NeedsDrewQuestion): boolean | null {
  const optionId = question.answer?.optionId?.trim().toLowerCase() ?? "";
  if (optionId === "approve" || optionId === "approve-now" || optionId === "yes") return true;
  if (optionId === "deny" || optionId === "no") return false;
  // Free-text answers never auto-apply: a tap carries an optionId.
  return null;
}

/** The applier's idempotency key for one recorded answer: the ledger
 *  id plus its answer instant. Intake is first-answer-wins per
 *  ledgerId, so one ledgerId yields at most one answer_id; a second
 *  answer for the same handle arrives under a different ledgerId
 *  (extension ordinal) and is superseded, never sent. */
export function answerIdFor(question: NeedsDrewQuestion): string | null {
  if (!question.answer) return null;
  return `${question.ledgerId}@${question.answer.at}`;
}

async function readApplyRecords(file: string): Promise<ApplyRecord[]> {
  try {
    const parsed: unknown = JSON.parse(await readFile(file, "utf8"));
    if (parsed && typeof parsed === "object" && Array.isArray((parsed as ApplyFileShape).records)) {
      return (parsed as ApplyFileShape).records.filter(
        (r): r is ApplyRecord => !!r && typeof r === "object" && typeof (r as ApplyRecord).answerId === "string",
      );
    }
    return [];
  } catch {
    return [];
  }
}

let applyWriteSeq = 0;
const applyChains = new Map<string, Promise<unknown>>();

function serializeApply<T>(file: string, fn: () => Promise<T>): Promise<T> {
  const prev = applyChains.get(file) ?? Promise.resolve();
  const run = prev.then(fn, fn);
  applyChains.set(
    file,
    run.catch(() => {}),
  );
  return run;
}

async function writeApplyRecords(file: string, records: ApplyRecord[]): Promise<void> {
  await mkdir(dirname(file), { recursive: true });
  const tmp = `${file}.tmp-${process.pid}-${++applyWriteSeq}`;
  const data: ApplyFileShape = { version: 1, records };
  await writeFile(tmp, JSON.stringify(data, null, 2));
  await rename(tmp, file);
}

export async function readApplyLedger(file: string): Promise<ApplyRecord[]> {
  return readApplyRecords(file);
}

export type DecideResult =
  | {
      ok: true;
      decided: boolean;
      approved?: boolean;
      tier?: number;
      outcome?: string;
      via?: string;
      note?: string;
    }
  | { ok: false; exit: number; error: string };

export type DecideFn = (body: {
  handle: string;
  approve: boolean;
  answer_id: string;
  answered_at: string;
  sig: string;
}) => Promise<DecideResult>;

export type FetchLike = (
  url: string,
  init?: { method?: string; headers?: Record<string, string>; body?: string },
) => Promise<{ ok: boolean; status: number; json(): Promise<unknown> }>;

function classifyDecideError(error: string): {
  state: ApplyState;
  note: string;
} {
  const msg = error.toLowerCase();
  // Expired decision grace: the broker settles the session first; the
  // late deny does not read as landed (§3 row 5).
  if (msg.includes("grace") || msg.includes("settle")) {
    return { state: "superseded", note: error };
  }
  // No pending request: settled sessions supersede; anything else is a
  // rejection of this answer (nothing to decide).
  if (msg.includes("no extension request is pending")) {
    if (msg.includes("settled") || msg.includes("closed") || msg.includes("expired")) {
      return { state: "superseded", note: error };
    }
    return { state: "rejected", note: error };
  }
  // Auth refusals (bad/expired sig) are terminal: retrying the same
  // envelope cannot succeed.
  if (msg.includes("signature") || msg.includes("replay") || msg.includes("window") || msg.includes("sig")) {
    return { state: "rejected", note: error };
  }
  return { state: "failed", note: error };
}

export interface ApplierDeps {
  needsDrewFile: string;
  applyFile: string;
  brokerUrl: string;
  brokerTokenFile: string;
  keyName?: string;
  now?: () => number;
  listAnswersFn?: (file: string) => Promise<NeedsDrewQuestion[]>;
  readTokenFn?: (file: string) => Promise<string | null>;
  resolveKeyFn?: (name: string) => Promise<string | null>;
  decideFn?: DecideFn;
  fetchImpl?: FetchLike;
}

export interface ApplierTickResult {
  checked: number;
  applied: number;
  superseded: number;
  rejected: number;
  failed: number;
  errors: string[];
}

/** Default decide caller: POST /actions-exec-extend-decide with the
 *  signed envelope over the broker token. Transport/auth failures
 *  surface as {ok:false} for the state machine to classify. */
export async function postSignedDecide(
  brokerUrl: string,
  token: string,
  body: {
    handle: string;
    approve: boolean;
    answer_id: string;
    answered_at: string;
    sig: string;
  },
  fetchImpl: FetchLike = fetch as unknown as FetchLike,
): Promise<DecideResult> {
  let res: { ok: boolean; status: number; json(): Promise<unknown> };
  try {
    res = await fetchImpl(`${brokerUrl}/actions-exec-extend-decide`, {
      method: "POST",
      headers: { "X-Agent-Token": token, "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
  } catch (err) {
    return {
      ok: false,
      exit: 3,
      error: `broker unreachable: ${err instanceof Error ? err.message : String(err)}`,
    };
  }
  let json: Record<string, unknown>;
  try {
    json = (await res.json()) as Record<string, unknown>;
  } catch {
    return {
      ok: false,
      exit: 3,
      error: `broker returned status ${res.status} with unreadable body`,
    };
  }
  if (!res.ok || json.ok === false || typeof json.error === "string") {
    const exit = typeof json.exit === "number" ? json.exit : res.status >= 500 ? 3 : 4;
    return {
      ok: false,
      exit,
      error: String(json.error ?? `broker status ${res.status}`),
    };
  }
  return {
    ok: true,
    decided: json.decided === true,
    ...(typeof json.approved === "boolean" ? { approved: json.approved } : {}),
    ...(typeof json.tier === "number" ? { tier: json.tier } : {}),
    ...(typeof json.outcome === "string" ? { outcome: json.outcome } : {}),
    ...(typeof json.via === "string" ? { via: json.via } : {}),
    ...(typeof json.note === "string" ? { note: json.note } : {}),
  };
}

/** One applier pass (§3/§6): read recorded broker-sourced answers,
 *  sign each unapplied one, POST the decide verb, run the answer_id
 *  state machine. Idempotent: applied/superseded/rejected rows are
 *  never re-sent; a second answer for the same handle is superseded. */
export async function runApplierTick(deps: ApplierDeps): Promise<ApplierTickResult> {
  const now = deps.now ?? (() => Date.now());
  const result: ApplierTickResult = {
    checked: 0,
    applied: 0,
    superseded: 0,
    rejected: 0,
    failed: 0,
    errors: [],
  };
  const answers = await (deps.listAnswersFn ?? listAnswers)(deps.needsDrewFile);
  const brokered = answers.filter((q) => needsDrewAnswerHandle(q) !== null);
  result.checked = brokered.length;
  if (brokered.length === 0) return result;

  const records = await readApplyRecords(deps.applyFile);
  const byAnswer = new Map(records.map((r) => [r.answerId, r]));
  const appliedHandles = new Set(records.filter((r) => r.state === "applied").map((r) => r.handle));

  const token = await (deps.readTokenFn ?? readBrokerToken)(deps.brokerTokenFile);
  if (!token) {
    result.errors.push("queue-broker token unavailable; answers stay pending");
    return result;
  }
  const keyName = deps.keyName ?? applyKeySecretName();
  const key = await (deps.resolveKeyFn ?? resolveBwsSecret)(keyName);
  if (!key) {
    result.errors.push(`applier signing key ${keyName} unavailable; answers stay pending`);
    return result;
  }

  const decide: DecideFn = deps.decideFn ?? ((body) => postSignedDecide(deps.brokerUrl, token, body, deps.fetchImpl));

  const pending: ApplyRecord[] = [];
  for (const q of brokered) {
    const handle = needsDrewAnswerHandle(q);
    if (!handle) continue;
    const answerId = answerIdFor(q);
    if (!answerId) continue;
    const existing = byAnswer.get(answerId);
    if (existing && existing.state !== "pending" && existing.state !== "failed") continue;
    // Double-tap guard: the first applied answer for a handle wins; a
    // second answer for the same handle (different answer_id) is
    // recorded superseded and never sent.
    if (!existing && appliedHandles.has(handle)) {
      pending.push({
        answerId,
        ledgerId: q.ledgerId,
        handle,
        approve: approveFromAnswer(q) ?? false,
        answeredAt: q.answer!.at,
        state: "superseded",
        note: "a prior answer for this handle already applied",
        updatedAt: new Date(now()).toISOString(),
      });
      result.superseded += 1;
      continue;
    }
    const approve = approveFromAnswer(q);
    if (approve === null) {
      pending.push({
        answerId,
        ledgerId: q.ledgerId,
        handle,
        approve: false,
        answeredAt: q.answer!.at,
        state: "rejected",
        note: `unrecognized answer option ${JSON.stringify(q.answer?.optionId ?? null)}; nothing sent`,
        updatedAt: new Date(now()).toISOString(),
      });
      result.rejected += 1;
      continue;
    }
    const answeredAt = toHouseIso(Date.parse(q.answer!.at));
    const sig = signDecide(key, handle, approve, answerId, answeredAt);
    let res: DecideResult;
    try {
      res = await decide({
        handle,
        approve,
        answer_id: answerId,
        answered_at: answeredAt,
        sig,
      });
    } catch (err) {
      res = {
        ok: false,
        exit: 3,
        error: `decide threw: ${err instanceof Error ? err.message : String(err)}`,
      };
    }
    const stamp = new Date(now()).toISOString();
    if (res.ok) {
      if (res.decided) {
        pending.push({
          answerId,
          ledgerId: q.ledgerId,
          handle,
          approve,
          answeredAt,
          state: "applied",
          outcome: res.outcome ?? (res.approved ? "approved" : "denied"),
          via: res.via ?? "signed-apply",
          ...(res.note ? { note: res.note } : {}),
          updatedAt: stamp,
        });
        appliedHandles.add(handle);
        result.applied += 1;
      } else if ((res.outcome ?? "").toLowerCase().includes("silence")) {
        pending.push({
          answerId,
          ledgerId: q.ledgerId,
          handle,
          approve,
          answeredAt,
          state: "superseded",
          outcome: res.outcome,
          via: res.via ?? "signed-apply",
          note: res.note ?? "deny window lapsed before this decision arrived",
          updatedAt: stamp,
        });
        result.superseded += 1;
      } else {
        // Decided:false without a silence outcome is a terminal
        // non-landing (e.g. late deny after settle): supersede.
        pending.push({
          answerId,
          ledgerId: q.ledgerId,
          handle,
          approve,
          answeredAt,
          state: "superseded",
          outcome: res.outcome,
          via: res.via ?? "signed-apply",
          ...(res.note ? { note: res.note } : {}),
          updatedAt: stamp,
        });
        result.superseded += 1;
      }
    } else {
      const classified = classifyDecideError(res.error);
      pending.push({
        answerId,
        ledgerId: q.ledgerId,
        handle,
        approve,
        answeredAt,
        state: classified.state,
        note: classified.note,
        updatedAt: stamp,
      });
      if (classified.state === "superseded") result.superseded += 1;
      else if (classified.state === "rejected") result.rejected += 1;
      else {
        result.failed += 1;
        result.errors.push(`${answerId}: ${res.error}`);
      }
    }
  }

  if (pending.length > 0) {
    await serializeApply(deps.applyFile, async () => {
      const current = await readApplyRecords(deps.applyFile);
      const idx = new Map(current.map((r, i) => [r.answerId, i]));
      for (const rec of pending) {
        const at = idx.get(rec.answerId);
        if (at === undefined) current.push(rec);
        else current[at] = rec;
      }
      await writeApplyRecords(deps.applyFile, current);
    });
  }
  return result;
}

let applierRunning = false;
let applierKick: (() => void) | null = null;

/** Ask the running applier to pass promptly (an answer just landed).
 *  No-op when the applier is not started. */
export function requestActionsExecApplyTick(): void {
  applierKick?.();
}

/** Start the applier loop (live mode only; fail-soft per pass). The
 *  periodic pass also catches answers that arrive by another route
 *  (the #13469 edge drain) and retries `failed` rows. */
export function startActionsExecApplier(deps: Omit<ApplierDeps, "decideFn" | "fetchImpl">, intervalMs = 60_000): void {
  const pass = async (): Promise<void> => {
    if (applierRunning) return;
    applierRunning = true;
    try {
      const r = await runApplierTick(deps);
      for (const e of r.errors) console.error(`actions-exec applier: ${e}`);
    } catch (err) {
      console.error(`actions-exec applier: pass failed (${err instanceof Error ? err.message : "error"})`);
    } finally {
      applierRunning = false;
    }
  };
  applierKick = () => void pass();
  setTimeout(() => void pass(), 10_000);
  const timer = setInterval(() => void pass(), intervalMs);
  timer.unref?.();
}
