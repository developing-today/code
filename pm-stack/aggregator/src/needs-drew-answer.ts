// One answer path for needs-Drew questions (#8568, #13469): the
// laptop's POST /api/needs-drew/:ledgerId/answer and the edge relay's
// drained answers validate and apply an answer through these two
// functions, so an answer taken at the edge lands in the ledger
// exactly as one taken here (design record
// doc/2026-11-17_design_pm8569_notifications_phase2.md §1: the
// Phase-1 catcher stays the single writer to the ledger).
import { createHash } from "node:crypto";
import { getNeedsDrew, recordAnswer, type NeedsDrewQuestion } from "./needs-drew.ts";
import { questionOptions } from "./needs-user.ts";

export interface NeedsDrewChoice {
  optionId?: string;
  optionIds?: string[];
  freeText?: string;
}

/** Validate an answer body against the question's options. */
export function validateNeedsDrewChoice(
  item: Pick<NeedsDrewQuestion, "options">,
  body: { optionId?: unknown; optionIds?: unknown; freeText?: unknown },
): { ok: true; value: NeedsDrewChoice } | { ok: false; error: string } {
  const known = new Set(questionOptions(item).map((o) => o.id));
  const optionId = typeof body.optionId === "string" ? body.optionId : undefined;
  if (optionId !== undefined && !known.has(optionId)) {
    return { ok: false, error: "optionId is not one of the question options" };
  }
  // #14149: a checkbox answer ticks several options at once.
  let optionIds: string[] | undefined;
  if (body.optionIds !== undefined) {
    if (!Array.isArray(body.optionIds) || body.optionIds.some((id) => typeof id !== "string")) {
      return { ok: false, error: "optionIds must be an array of strings" };
    }
    if ((body.optionIds as string[]).some((id) => !known.has(id))) {
      return { ok: false, error: "optionIds must all be question options" };
    }
    optionIds = body.optionIds as string[];
  }
  const freeText = typeof body.freeText === "string" ? body.freeText : undefined;
  if (body.freeText !== undefined && freeText === undefined) {
    return { ok: false, error: "freeText must be a string" };
  }
  if (optionId === undefined && !optionIds?.length && !freeText?.trim()) {
    return { ok: false, error: "optionId, optionIds or freeText required" };
  }
  return {
    ok: true,
    value: {
      ...(optionId !== undefined ? { optionId } : {}),
      ...(optionIds !== undefined ? { optionIds } : {}),
      ...(freeText !== undefined ? { freeText } : {}),
    },
  };
}

/** One answer as the edge relay's drain returns it. */
export interface EdgeAnswer {
  answerId: string;
  ledgerId: string;
  optionId?: string;
  optionIds?: string[];
  freeText?: string;
  answeredAt: string;
  answerTokenHash: string;
}

/** What became of a drained answer. Every outcome but a thrown error
 * is final, so the drain acks it; a throw leaves it at the edge for
 * the next contact. */
export type EdgeApplyOutcome = "applied" | "duplicate" | "unknown" | "rejected";

/** The side effects an answer sets off once recorded: close its alert
 * incident, move its stage. Supplied by the server; best-effort. */
export type AfterAnswered = (item: NeedsDrewQuestion, answered: NeedsDrewQuestion) => Promise<void>;

export const tokenHashOf = (token: string): string => createHash("sha256").update(token).digest("hex");

/** Apply one drained edge answer to the ledger, once. */
export async function applyEdgeAnswer(
  file: string,
  answer: EdgeAnswer,
  afterAnswered: AfterAnswered,
): Promise<EdgeApplyOutcome> {
  const item = await getNeedsDrew(file, answer.ledgerId);
  if (!item) return "unknown";
  // The edge took the answer against the token hash it mirrored; a
  // mismatch means it is not this question's answer.
  if (typeof answer.answerTokenHash !== "string" || tokenHashOf(item.token) !== answer.answerTokenHash) {
    return "rejected";
  }
  if (item.answer) return "duplicate";
  const choice = validateNeedsDrewChoice(item, answer);
  if (!choice.ok) return "rejected";
  const updated = await recordAnswer(file, answer.ledgerId, { ...choice.value, via: "edge" });
  if (!updated?.answer) throw new Error(`needs-drew ${answer.ledgerId}: edge answer not recorded`);
  // recordAnswer keeps the first answer: a laptop answer that landed
  // between the read above and the write wins, and this one is a repeat.
  if (updated.answer.via !== "edge") return "duplicate";
  await afterAnswered(item, updated);
  return "applied";
}
