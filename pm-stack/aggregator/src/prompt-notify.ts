// Prompt-completion push (#10035 S6, design #9539 §4): "push
// notification on async completion (opt-in per submission)".
//
// The opt-in is the record's `notify` flag, set at submission time.
// The send itself rides the alert throat's machinery, exactly like
// the escalation sink (#9991): sendPush fanned out over the alerts
// store's subscriptions, gated per subscription by the "prompt"
// notification class (classEnabled), one sends-ledger row per
// attempted subscription. A subscription that predates the class
// takes the class's reviewed default via classEnabled (the #13940
// backfill); only an explicit opt-out on /alerts/preferences keeps
// it from receiving prompt pushes.
//
// The trigger seam is the single answer path: answers land through
// PromptsStore.update() via PATCH /api/prompt/:id, so the route
// compares the record before and after the patch and calls
// notifyPromptCompletion only on the transition into a terminal
// status — a re-patch of an already-answered prompt never re-pushes.
import { classEnabled, type AlertsStore, type PushPayload } from "./alerts.ts";
import { sendPush, type PushTarget } from "./push.ts";
import type { PromptRecord, PromptStatus } from "./prompts.ts";

export const PROMPT_PUSH_CLASS = "prompt";

export function isTerminalPromptStatus(status: PromptStatus): boolean {
  return status === "answered" || status === "failed";
}

/**
 * The transition that earns a push: the record was not terminal
 * before the patch, is terminal after it, and its submitter opted
 * in. Anything else — a first patch that only archives, a re-answer
 * of a terminal record, notify off — sends nothing.
 */
export function promptCompletionTransition(before: PromptRecord | null, after: PromptRecord): boolean {
  return (
    before !== null &&
    !isTerminalPromptStatus(before.status) &&
    isTerminalPromptStatus(after.status) &&
    after.notify === true
  );
}

/** Deep link: the prompt's own card on the chat page. */
export function promptCompletionUrl(origin: string, record: PromptRecord): string {
  return `${origin}/chat#prompt-${record.id}`;
}

const EXCERPT_CHARS = 140;

function excerpt(text: string): string {
  const flat = text.replace(/\s+/g, " ").trim();
  return flat.length > EXCERPT_CHARS ? `${flat.slice(0, EXCERPT_CHARS - 1)}…` : flat;
}

export function buildPromptCompletionPayload(record: PromptRecord, origin: string): PushPayload {
  const failed = record.status === "failed";
  const detail = failed ? record.error : record.answer;
  const promptText = excerpt(record.prompt);
  return {
    class: PROMPT_PUSH_CLASS,
    objectRef: record.id,
    url: promptCompletionUrl(origin, record),
    title: failed ? "Prompt failed" : "Prompt answered",
    body: detail ? `${promptText} — ${excerpt(detail)}` : promptText,
  };
}

export interface PromptPushSummary {
  subscriptions: number;
  /** Subscriptions the "prompt" class toggle let through. */
  eligible: number;
  sent: number;
  gone: number;
  errors: number;
}

/**
 * Fan the completion push out over the alerts store's subscriptions.
 * The sender is injectable, like sendClassTestPush's. One sends-
 * ledger row per attempted subscription, under the prompt class
 * with the prompt's own id as the object ref.
 */
export async function notifyPromptCompletion(
  store: Pick<AlertsStore, "listSubscriptions" | "logSend" | "noteSubscriptionGone" | "noteSubscriptionAlive">,
  record: PromptRecord,
  origin: string,
  send: typeof sendPush = sendPush,
): Promise<PromptPushSummary> {
  const payload = buildPromptCompletionPayload(record, origin);
  const subs = await store.listSubscriptions();
  const targets = subs.filter((sub) => classEnabled(sub, PROMPT_PUSH_CLASS));
  const summary: PromptPushSummary = {
    subscriptions: subs.length,
    eligible: targets.length,
    sent: 0,
    gone: 0,
    errors: 0,
  };
  for (const sub of targets) {
    const target: PushTarget = { endpoint: sub.endpoint, p256dh: sub.p256dh, auth: sub.auth };
    const outcome = await send(target, payload);
    if (outcome.ok) {
      summary.sent += 1;
      await store.logSend({ class: PROMPT_PUSH_CLASS, objectRef: payload.objectRef, outcome: "sent" });
      await store.noteSubscriptionAlive(sub.endpoint);
    } else if (outcome.gone) {
      summary.gone += 1;
      await store.logSend({ class: PROMPT_PUSH_CLASS, objectRef: payload.objectRef, outcome: "gone" });
      await store.noteSubscriptionGone(sub.endpoint);
    } else {
      summary.errors += 1;
      await store.logSend({ class: PROMPT_PUSH_CLASS, objectRef: payload.objectRef, outcome: "error" });
    }
  }
  return summary;
}
