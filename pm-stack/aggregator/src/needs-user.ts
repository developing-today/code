// The needs-user list (#14084 S1, slice 2; Drew's ruling in comment
// 7124146989, points 3 and 5).
//
// A question raised through the needs-drew ledger (#8568) is an item
// that reached the user, so it enters the staged pipeline in
// push-registry.ts at `needs-user` (with a review stage on its record).
// Every question offers "Send back" as a standard answer: it returns the
// item to `needs-review`, to be decided there. Any other answer resolves
// it. The list orders open questions by the priority the user set, then
// by how many issues wait on the answer, then by age.
//
// The ledger file stays the record of the question and its answer; the
// stage rows carry only what the ledger has no seat for (stage,
// priority, blocked-by count). Questions raised before this slice have
// no stage row and read as unprioritised needs-user items.
import type { NeedsDrewOption, NeedsDrewQuestion } from "./needs-drew.ts";
import {
  MAX_PRIORITY,
  MIN_PRIORITY,
  SEND_BACK_OPTION,
  sortForAnswering,
  type ItemStage,
  type ItemStageRow,
  type ItemStageStore,
} from "./push-registry.ts";

/** The stage-row key for a ledger question. */
export function needsUserItemRef(ledgerId: string): string {
  return `ledger:${ledgerId}`;
}

/** The options a question offers: its own, plus "Send back" unless the
 * raiser already gave an option with that id. Applied at read time, so
 * questions raised before this slice offer it too. */
export function questionOptions(item: Pick<NeedsDrewQuestion, "options">): NeedsDrewOption[] {
  if (item.options.some((o) => o.id === SEND_BACK_OPTION.id)) return item.options;
  return [...item.options, { id: SEND_BACK_OPTION.id, label: SEND_BACK_OPTION.label }];
}

export function isSendBack(optionId: string | undefined): boolean {
  return optionId === SEND_BACK_OPTION.id;
}

/** Stage bookkeeping for a fresh raise. Best-effort by contract: the
 * caller logs a failure and never fails the raise. */
export async function onQuestionRaised(stages: ItemStageStore, item: NeedsDrewQuestion): Promise<void> {
  if (item.answer) return;
  await stages.escalate(needsUserItemRef(item.ledgerId), `raised by ${item.source}`);
}

/** Stage bookkeeping for a recorded answer: "Send back" returns the item
 * to review, anything else resolves it. */
export async function onQuestionAnswered(
  stages: ItemStageStore,
  ledgerId: string,
  optionId: string | undefined,
): Promise<ItemStage | null> {
  const ref = needsUserItemRef(ledgerId);
  // A question raised before this slice has no row yet: give it one, so
  // the answer lands on a recorded stage.
  if (!(await stages.get(ref))) await stages.escalate(ref, "raised before the stage pipeline");
  const moved = isSendBack(optionId)
    ? await stages.sendBack(ref, "sent back by the user")
    : await stages.resolve(ref, "answered");
  return moved.item?.stage ?? null;
}

export interface NeedsUserListItem {
  ledgerId: string;
  question: string;
  source: string;
  raisedAt: string;
  stage: ItemStage;
  priority: number | null;
  blockedByCount: number;
}

/** Join open questions with their stage rows, in answering order. The
 * answer token never leaves the ledger through this list. */
export function buildNeedsUserList(
  questions: NeedsDrewQuestion[],
  rows: ReadonlyMap<string, ItemStageRow>,
): NeedsUserListItem[] {
  const joined = questions
    .filter((q) => !q.answer)
    .map((q) => {
      const row = rows.get(needsUserItemRef(q.ledgerId));
      const stageRow: ItemStageRow = row ?? {
        item_ref: needsUserItemRef(q.ledgerId),
        stage: "needs-user",
        priority: null,
        blocked_by_count: 0,
        entered_stage_at: q.raisedAt,
        updated_at: q.raisedAt,
      };
      return { q, row: stageRow };
    })
    // A question whose row says it left needs-user (resolved or sent
    // back elsewhere) is not the user's to answer.
    .filter(({ row }) => row.stage === "needs-user");
  const byRef = new Map(joined.map((j) => [j.row.item_ref, j]));
  return sortForAnswering(joined.map((j) => j.row)).map((row) => {
    const { q } = byRef.get(row.item_ref)!;
    return {
      ledgerId: q.ledgerId,
      question: q.question,
      source: q.source,
      raisedAt: q.raisedAt,
      stage: row.stage,
      priority: row.priority === null || row.priority === undefined ? null : Number(row.priority),
      blockedByCount: Number(row.blocked_by_count) || 0,
    };
  });
}

/** Read the stage rows for a set of questions. */
export async function stageRowsFor(
  stages: ItemStageStore,
  questions: NeedsDrewQuestion[],
): Promise<Map<string, ItemStageRow>> {
  const rows = new Map<string, ItemStageRow>();
  for (const q of questions) {
    const row = await stages.get(needsUserItemRef(q.ledgerId));
    if (row) rows.set(row.item_ref, row);
  }
  return rows;
}

/** Parse a priority body: an integer 1..5, or null to clear. */
export function parsePriority(value: unknown): { ok: true; value: number | null } | { ok: false; error: string } {
  if (value === null) return { ok: true, value: null };
  if (typeof value === "number" && Number.isInteger(value) && value >= MIN_PRIORITY && value <= MAX_PRIORITY) {
    return { ok: true, value };
  }
  return { ok: false, error: `priority must be an integer ${MIN_PRIORITY}..${MAX_PRIORITY} or null` };
}

/** Parse a blocked-by count body: a non-negative integer. */
export function parseBlockedByCount(value: unknown): { ok: true; value: number } | { ok: false; error: string } {
  if (typeof value === "number" && Number.isInteger(value) && value >= 0 && value <= 1_000_000) {
    return { ok: true, value };
  }
  return { ok: false, error: "count must be a non-negative integer" };
}

/** Set a question's priority, giving a pre-pipeline question its row
 * first so the setting lands. */
export async function setQuestionPriority(
  stages: ItemStageStore,
  ledgerId: string,
  priority: number | null,
): Promise<ItemStageRow | null> {
  const ref = needsUserItemRef(ledgerId);
  if (!(await stages.get(ref))) await stages.escalate(ref, "raised before the stage pipeline");
  return stages.setPriority(ref, priority);
}

export async function setQuestionBlockedByCount(
  stages: ItemStageStore,
  ledgerId: string,
  count: number,
): Promise<ItemStageRow | null> {
  const ref = needsUserItemRef(ledgerId);
  if (!(await stages.get(ref))) await stages.escalate(ref, "raised before the stage pipeline");
  return stages.setBlockedByCount(ref, count);
}
