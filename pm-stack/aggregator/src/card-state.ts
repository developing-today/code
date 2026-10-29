/**
 * Render-time effective state for one dashboard card (#1443/#1449/
 * #1450/#1451, epic #1442).
 *
 * The bug this fixes: dashboard clicks recorded actions in the store,
 * but the rendered card never changed — the only consumer runs hourly,
 * so a snooze/dismiss sat invisible for up to an hour and the card
 * "would not go away". The dashboard owns the store, so the card's
 * effective state is derived HERE, at render, from the same records
 * the consumers will later apply:
 *
 * - Suppression: the latest effective snooze (until in the future),
 *   dismiss, or mark-done action on the object takes the card out of
 *   its main list immediately — pending (state "new") counts exactly
 *   like applied. A consumer REJECTION lifts the suppression again
 *   (the item resurfaces, with the rejection badge explaining why).
 * - Restore (#1450): a `restore` action cancels the action it names
 *   in `supersedes`, even one the consumer already applied — the card
 *   returns to its list with its full history intact. (The store's
 *   own supersede transition only reaches still-new actions; cancels
 *   of applied actions are a derivation concern, so they live here.)
 * - Acknowledged: an effective acknowledge (pending or applied) marks
 *   the card quiet — seen, not resolved.
 * - Survey answers (#1451): answer actions merge per question, the
 *   latest answer to a question wins, so partial submits accumulate
 *   on the card until every required question is answered.
 *
 * Pure derivation over the object's action list (oldest first, the
 * store's order) — no I/O, so renderers and tests share one truth.
 */
import { latestInFamily, type DashAction } from "./actions.ts";

export type CardSuppression =
  | { kind: "snoozed"; until: string; action: DashAction }
  | { kind: "dismissed"; action: DashAction }
  | { kind: "done"; action: DashAction };

export interface CardState {
  /** The card leaves its main list while this is set. */
  suppressed: CardSuppression | null;
  /** An effective acknowledge exists (pending or applied). */
  acked: boolean;
  /** Actions still waiting for their consumer (state "new"). */
  pending: DashAction[];
  /** Merged survey answers by question id, latest answer wins. */
  answered: Map<string, string | string[]>;
  /** The option picked by the latest effective option answer, if any. */
  answeredOptionId?: string;
}

/** Ids cancelled by a restore: any non-superseded, non-rejected
 *  restore action cancels the action it names in `supersedes`
 *  (a consumer rejection is non-effective, as for every other verb). */
export function cancelledActionIds(actions: DashAction[]): Set<string> {
  const out = new Set<string>();
  for (const a of actions) {
    if (a.verb === "restore" && a.state !== "superseded" && a.state !== "rejected" && a.supersedes)
      out.add(a.supersedes);
  }
  return out;
}

/** The actions that still speak for the object: not superseded, not
 *  cancelled by a restore. Input order (oldest first) is preserved. */
export function effectiveActions(actions: DashAction[] | undefined): DashAction[] {
  if (!actions?.length) return [];
  const cancelled = cancelledActionIds(actions);
  return actions.filter((a) => a.state !== "superseded" && !cancelled.has(a.id));
}

export function cardState(actions: DashAction[] | undefined, now = Date.now()): CardState {
  const eff = effectiveActions(actions);
  const suppressors: CardSuppression[] = [];
  const snooze = latestInFamily(eff, "snooze");
  if (snooze && snooze.state !== "rejected" && snooze.payload.until && Date.parse(snooze.payload.until) > now) {
    suppressors.push({ kind: "snoozed", until: snooze.payload.until, action: snooze });
  }
  const dismiss = latestInFamily(eff, "dismiss");
  if (dismiss && dismiss.state !== "rejected") suppressors.push({ kind: "dismissed", action: dismiss });
  const done = latestInFamily(eff, "done");
  if (done && done.state !== "rejected") suppressors.push({ kind: "done", action: done });
  suppressors.sort((a, b) => a.action.createdAt.localeCompare(b.action.createdAt));

  const ack = latestInFamily(eff, "ack");
  const answered = new Map<string, string | string[]>();
  let answeredOptionId: string | undefined;
  for (const a of eff) {
    if (a.verb !== "answer" || a.state === "rejected") continue;
    for (const entry of a.payload.answers ?? []) answered.set(entry.questionId, entry.value);
    if (a.payload.optionId) answeredOptionId = a.payload.optionId;
  }
  return {
    suppressed: suppressors.at(-1) ?? null,
    acked: ack !== undefined && ack.state !== "rejected",
    pending: eff.filter((a) => a.state === "new"),
    answered,
    ...(answeredOptionId ? { answeredOptionId } : {}),
  };
}

/** The pending action of one verb on a card, if any — the render uses
 *  it to disable the matching control so repeat clicks cannot pile up
 *  duplicate pending actions (#1443). */
export function pendingVerb(cs: CardState, verb: DashAction["verb"]): DashAction | undefined {
  return cs.pending.find((a) => a.verb === verb);
}
