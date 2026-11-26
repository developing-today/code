// Edge answer intake (#8569 Phase 2 step 3, issue #13469; design
// record doc/2026-11-17_design_pm8569_notifications_phase2.md §1-§3).
//
// The pm-notify Durable Object answers needs-Drew questions while the
// laptop is away. Everything here is a pure function of (state, input,
// now) so the oracle tests drive the exact transitions the DO makes:
//
//   - The answerable question comes from the event mirror (step 2):
//     the newest mirrored needs-Drew event for `needs-drew:<ledgerId>`.
//     A `raised` event carries the question, its options and the hash
//     of the per-question answer token; a `cleared` event (the laptop
//     stopped raising it: answered there, or aged out) withdraws it.
//     The DO never holds the token itself (design §4).
//   - The first accepted answer per question wins. It is stored as a
//     pending answer under a fresh answer id until the aggregator
//     drains it (GET /answers, then ack by answer id). A repeat (a
//     double tap, a retry) returns the first answer and stores nothing.
//   - A withdrawn or expired question refuses the answer. The token is
//     checked first, so nothing about a question is told to a caller
//     without its token.
//
// The pending-questions ledger on the laptop stays the place of
// record (invariant 1): the edge is a mailbox. The Phase-1 catcher
// applies drained answers exactly as it applies its own.

/** Open questions age out after 7 days, the same window as the
 * laptop's listOpenNeedsDrew (NEEDS_DREW_OPEN_MAX_AGE_MS). */
export const QUESTION_MAX_AGE_MS = 7 * 24 * 60 * 60_000;
/** Answer ids already drained are remembered this long, so a late
 * retry of an answered question is still told it was answered. */
export const ANSWERED_RETENTION_MS = 30 * 24 * 60 * 60_000;
/** Pending answers wait for a drain. A stack away long enough to
 * fill this has more wrong with it than the mailbox can fix. */
export const MAX_PENDING_ANSWERS = 256;
export const MAX_FREE_TEXT = 4000;
/** The laptop caps a question at 8 options; Send back makes 9. */
export const MAX_OPTIONS = 9;

export const ANSWER_TYPES = ["radio", "checkbox", "text"] as const;
export type AnswerType = (typeof ANSWER_TYPES)[number];

export interface AnswerOption {
  id: string;
  label: string;
}

/** A needs-Drew question as the edge can answer it. */
export interface AnswerableQuestion {
  ledgerId: string;
  question: string;
  context?: string;
  options: AnswerOption[];
  answerType: AnswerType;
  links: string[];
  tokenHash: string;
  raisedAt: string;
}

/** An answer accepted at the edge, waiting for the aggregator. */
export interface PendingAnswer {
  answerId: string;
  ledgerId: string;
  optionId?: string;
  optionIds?: string[];
  freeText?: string;
  answeredAt: string;
  answerTokenHash: string;
}

/** The first answer per question, kept after its drain so a repeat
 * is answered the same way. */
export interface AnsweredRecord {
  ledgerId: string;
  answerId: string;
  answeredAt: string;
}

export interface AnswerState {
  pendingAnswers: PendingAnswer[];
  answered: AnsweredRecord[];
}

/** The slice of a mirrored event this module reads. */
export interface MirroredQuestionEvent {
  objectRef: string;
  transition: string;
  raisedAt: string;
  payload: Record<string, unknown>;
}

/** Legacy and role-term object refs remain readable during the cutover. */
export const needsDrewObjectRef = (ledgerId: string): string => `needs-drew:${ledgerId}`;
export const needsUserObjectRef = (ledgerId: string): string => `needs-user:${ledgerId}`;

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);

/** Read the answerable question out of a mirrored `raised` event's
 * legacy or role-term payload, or null when it does not carry one. */
export function questionFromEvent(event: MirroredQuestionEvent): AnswerableQuestion | null {
  if (event.transition !== "raised" || event.payload.class !== "needs-user") return null;
  if ("needsDrew" in event.payload && "needsUser" in event.payload) return null;
  const nd = "needsUser" in event.payload ? event.payload.needsUser : event.payload.needsDrew;
  if (!isRecord(nd)) return null;
  const { ledgerId, question, context, answerType, links, tokenHash } = nd;
  // The aggregator mirrors every answerable option (Send back
  // included) as answerOptions; options is the push's button list.
  const options = Array.isArray(nd.answerOptions) ? nd.answerOptions : nd.options;
  if (
    typeof ledgerId !== "string" ||
    !ledgerId ||
    ![needsDrewObjectRef(ledgerId), needsUserObjectRef(ledgerId)].includes(event.objectRef)
  ) return null;
  if (typeof question !== "string" || !question) return null;
  if (typeof tokenHash !== "string" || !/^[0-9a-f]{64}$/.test(tokenHash)) return null;
  if (!Array.isArray(options) || options.length < 1 || options.length > MAX_OPTIONS) return null;
  const parsedOptions: AnswerOption[] = [];
  for (const option of options) {
    if (!isRecord(option) || typeof option.id !== "string" || typeof option.label !== "string") return null;
    if (!option.id || !option.label) return null;
    parsedOptions.push({ id: option.id, label: option.label });
  }
  const type: AnswerType =
    typeof answerType === "string" && (ANSWER_TYPES as readonly string[]).includes(answerType)
      ? (answerType as AnswerType)
      : "radio";
  return {
    ledgerId,
    question,
    ...(typeof context === "string" && context ? { context } : {}),
    options: parsedOptions,
    answerType: type,
    links: Array.isArray(links)
      ? links.filter((l): l is string => typeof l === "string" && /^https?:\/\//.test(l))
      : [],
    tokenHash,
    raisedAt: event.raisedAt,
  };
}

export type QuestionLookup =
  | { kind: "unknown" }
  | { kind: "open"; question: AnswerableQuestion }
  | { kind: "withdrawn"; question: AnswerableQuestion }
  | { kind: "expired"; question: AnswerableQuestion };

/** Find a question by ledger id in the mirror log. The newest event
 * across its legacy and role-term object refs decides. A `raised` event
 * with a token hash is open until it ages out; `cleared` withdraws it. */
export function lookupQuestion(events: MirroredQuestionEvent[], ledgerId: string, now: number): QuestionLookup {
  const refs = new Set([needsDrewObjectRef(ledgerId), needsUserObjectRef(ledgerId)]);
  let latest: MirroredQuestionEvent | null = null;
  let lastRaised: AnswerableQuestion | null = null;
  for (const event of events) {
    if (!refs.has(event.objectRef)) continue;
    if (event.transition === "raised") {
      const parsed = questionFromEvent(event);
      if (!parsed) continue;
      lastRaised = parsed;
      latest = event;
    } else if (event.transition === "cleared") {
      latest = event;
    }
  }
  if (!latest || !lastRaised) return { kind: "unknown" };
  if (latest.transition === "cleared") return { kind: "withdrawn", question: lastRaised };
  if (now - Date.parse(lastRaised.raisedAt) >= QUESTION_MAX_AGE_MS) return { kind: "expired", question: lastRaised };
  return { kind: "open", question: lastRaised };
}

/** sha256 hex of the per-question token: what the laptop mirrors. */
export async function hashToken(token: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(token));
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

/** Constant-time compare of two equal-length hex digests. */
export function sameHash(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return diff === 0;
}

export interface AnswerChoice {
  optionId?: string;
  optionIds?: string[];
  freeText?: string;
}

/** Validate an answer body against the question, by the same rules
 * as the laptop's POST /api/needs-drew/:ledgerId/answer. */
export function validateAnswerChoice(
  question: AnswerableQuestion,
  body: Record<string, unknown>,
): { ok: true; value: AnswerChoice } | { ok: false; error: string } {
  const known = new Set(question.options.map((o) => o.id));
  let optionId: string | undefined;
  if (body.optionId !== undefined) {
    if (typeof body.optionId !== "string" || !known.has(body.optionId)) {
      return { ok: false, error: "optionId is not one of the question options" };
    }
    optionId = body.optionId;
  }
  let optionIds: string[] | undefined;
  if (body.optionIds !== undefined) {
    if (!Array.isArray(body.optionIds) || body.optionIds.some((id) => typeof id !== "string")) {
      return { ok: false, error: "optionIds must be an array of strings" };
    }
    if ((body.optionIds as string[]).some((id) => !known.has(id))) {
      return { ok: false, error: "optionIds must all be question options" };
    }
    optionIds = [...new Set(body.optionIds as string[])];
  }
  let freeText: string | undefined;
  if (body.freeText !== undefined) {
    if (typeof body.freeText !== "string") return { ok: false, error: "freeText must be a string" };
    if (body.freeText.length > MAX_FREE_TEXT) {
      return { ok: false, error: `freeText is capped at ${MAX_FREE_TEXT} characters` };
    }
    freeText = body.freeText;
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

/** Drop answered records past retention. Pending answers are never
 * pruned by age: they wait for the drain. */
export function pruneAnswered(records: AnsweredRecord[], now: number): AnsweredRecord[] {
  const cutoff = now - ANSWERED_RETENTION_MS;
  return records.filter((record) => Date.parse(record.answeredAt) >= cutoff);
}

export type AcceptOutcome =
  | { outcome: "stored"; state: AnswerState; answer: PendingAnswer }
  | { outcome: "duplicate"; state: AnswerState; answerId: string; answeredAt: string }
  | { outcome: "full"; state: AnswerState };

/** Accept the first answer for a question; a repeat changes nothing. */
export function acceptAnswer(
  state: AnswerState,
  question: AnswerableQuestion,
  choice: AnswerChoice,
  now: number,
  answerId: string,
): AcceptOutcome {
  const answered = pruneAnswered(state.answered, now);
  const prior = answered.find((record) => record.ledgerId === question.ledgerId);
  if (prior) {
    return {
      outcome: "duplicate",
      state: { ...state, answered },
      answerId: prior.answerId,
      answeredAt: prior.answeredAt,
    };
  }
  if (state.pendingAnswers.length >= MAX_PENDING_ANSWERS) return { outcome: "full", state: { ...state, answered } };
  const answeredAt = new Date(now).toISOString();
  const answer: PendingAnswer = {
    answerId,
    ledgerId: question.ledgerId,
    ...choice,
    answeredAt,
    answerTokenHash: question.tokenHash,
  };
  return {
    outcome: "stored",
    answer,
    state: {
      pendingAnswers: [...state.pendingAnswers, answer],
      answered: [...answered, { ledgerId: question.ledgerId, answerId, answeredAt }],
    },
  };
}

/** The answered record for a question, if it has one. */
export function answeredFor(state: AnswerState, ledgerId: string, now: number): AnsweredRecord | null {
  return pruneAnswered(state.answered, now).find((record) => record.ledgerId === ledgerId) ?? null;
}

/** Drop drained answers by id. Unknown ids are ignored, so an ack
 * that is retried after a lost response changes nothing. */
export function ackAnswers(state: AnswerState, answerIds: string[]): { state: AnswerState; acked: number } {
  const ids = new Set(answerIds);
  const pendingAnswers = state.pendingAnswers.filter((answer) => !ids.has(answer.answerId));
  return { state: { ...state, pendingAnswers }, acked: state.pendingAnswers.length - pendingAnswers.length };
}

export function validateAck(body: unknown): { ok: true; value: string[] } | { ok: false; error: string } {
  if (!isRecord(body) || !Array.isArray(body.answerIds)) return { ok: false, error: "answerIds must be an array" };
  if (body.answerIds.length > MAX_PENDING_ANSWERS) {
    return { ok: false, error: `answerIds is capped at ${MAX_PENDING_ANSWERS} entries` };
  }
  if (body.answerIds.some((id) => typeof id !== "string" || id.length === 0 || id.length > 64)) {
    return { ok: false, error: "answerIds must be non-empty strings of at most 64 chars" };
  }
  return { ok: true, value: body.answerIds as string[] };
}

// ------------------------------------------------------------ pages

const esc = (s: string): string =>
  s.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;");

const PAGE_STYLE = `<style>
  body { font-family: system-ui, sans-serif; background: #0e1117; color: #e6edf3; margin: 0; padding: 2rem 1rem; }
  main { max-width: 40rem; margin: 0 auto; }
  h1 { font-size: 1.4rem; line-height: 1.35; }
  .context { color: #9da7b1; white-space: pre-wrap; }
  .links a { color: #58a6ff; display: block; margin: 0.35rem 0; overflow-wrap: anywhere; }
  .options { display: grid; gap: 0.6rem; margin: 1.2rem 0; }
  button.option, label.option { text-align: left; padding: 0.8rem 1rem; font-size: 1rem; border-radius: 0.5rem;
    border: 1px solid #30363d; background: #161b22; color: #e6edf3; cursor: pointer; }
  button.option:hover { border-color: #58a6ff; }
  textarea { width: 100%; min-height: 4.5rem; box-sizing: border-box; background: #161b22; color: #e6edf3;
    border: 1px solid #30363d; border-radius: 0.5rem; padding: 0.6rem; }
  button.submit { margin-top: 0.6rem; padding: 0.7rem 1.2rem; font-size: 1rem; border-radius: 0.5rem;
    border: 0; background: #1f6feb; color: #fff; cursor: pointer; }
  .confirm { border: 1px solid #238636; border-radius: 0.5rem; padding: 1rem; }
</style>`;

const page = (title: string, body: string): string =>
  `<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1"><meta name="referrer" content="no-referrer">
<title>${esc(title)} — PM Stack</title>${PAGE_STYLE}</head>
<body><main>${body}</main></body></html>`;

/** Unknown question or wrong token: nothing about the question. */
export function renderForbiddenPage(): string {
  return page(
    "Not available",
    `<h1>Answer link not available</h1>
<p class="context">This answer link is unknown or its token does not match. Nothing about the question is shown without the correct link.</p>`,
  );
}

export function renderClosedPage(question: AnswerableQuestion, kind: "withdrawn" | "expired"): string {
  const why =
    kind === "expired"
      ? "It was raised more than 7 days ago and is no longer open."
      : "It has been answered or withdrawn on the stack, so it no longer takes answers here.";
  return page("No longer open", `<h1>${esc(question.question)}</h1><p class="context">${why}</p>`);
}

export function renderAnsweredPage(question: AnswerableQuestion, record: AnsweredRecord): string {
  return page(
    "Answered",
    `<h1>${esc(question.question)}</h1>
<div class="confirm"><p>Your answer was received at ${esc(record.answeredAt)}.</p>
<p class="context">The stack applies it the next time it checks in; the first answer is the one that counts.</p></div>`,
  );
}

/** The open answer page: question, context, links and every option,
 * posting back to this Worker with the token from the link. */
export function renderAnswerPage(question: AnswerableQuestion, token: string): string {
  const links = question.links.map((l) => `<a href="${esc(l)}" rel="noreferrer">${esc(l)}</a>`).join("");
  const checkbox = question.answerType === "checkbox";
  const options =
    question.answerType === "text"
      ? ""
      : checkbox
        ? question.options
            .map(
              (o) =>
                `<label class="option"><input type="checkbox" name="option" value="${esc(o.id)}"> ${esc(o.label)}</label>`,
            )
            .join("")
        : question.options
            .map((o) => `<button class="option" type="button" data-option="${esc(o.id)}">${esc(o.label)}</button>`)
            .join("");
  const endpoint = `/api/needs-user/${encodeURIComponent(question.ledgerId)}/answer`;
  return page(
    "Needs Drew",
    `<h1>${esc(question.question)}</h1>
${question.context ? `<p class="context">${esc(question.context)}</p>` : ""}
${links ? `<div class="links">${links}</div>` : ""}
${options ? `<div class="options">${options}</div>` : ""}
${checkbox ? `<button class="submit" type="button" id="sendChecked">Send ticked options</button>` : ""}
<label>${question.answerType === "text" ? "Your answer:" : "Or answer in your own words (optional, sent with a tapped option or on its own):"}
<textarea id="freeText" maxlength="${MAX_FREE_TEXT}" placeholder="Free-text answer"></textarea></label>
<button class="submit" type="button" id="sendText">Send free-text answer</button>
<p id="status" class="context"></p>
<script>
const token = ${JSON.stringify(token).replaceAll("<", "\\u003c")};
const endpoint = ${JSON.stringify(endpoint)};
async function answer(payload) {
  const status = document.getElementById('status');
  status.textContent = 'Sending…';
  try {
    const res = await fetch(endpoint, { method: 'POST', headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ token, ...payload }) });
    if (res.status === 410) { status.textContent = 'This question is no longer open, so the answer was not kept.'; return; }
    if (!res.ok) throw new Error('HTTP ' + res.status);
    const body = await res.json();
    status.textContent = body.duplicate
      ? 'An answer was already received for this question; the first one counts.'
      : 'Answer received. The stack applies it the next time it checks in.';
    document.querySelectorAll('button').forEach((b) => { b.disabled = true; });
  } catch { status.textContent = 'Sending failed. Please try again.'; }
}
const freeTextValue = () => document.getElementById('freeText').value.trim();
document.querySelectorAll('button.option').forEach((b) => {
  b.addEventListener('click', () => {
    const freeText = freeTextValue();
    answer({ optionId: b.dataset.option, ...(freeText ? { freeText } : {}) });
  });
});
const sendChecked = document.getElementById('sendChecked');
if (sendChecked) sendChecked.addEventListener('click', () => {
  const optionIds = [...document.querySelectorAll('input[name=option]:checked')].map((i) => i.value);
  const freeText = freeTextValue();
  if (!optionIds.length && !freeText) { document.getElementById('status').textContent = 'Tick an option or write an answer first.'; return; }
  answer({ ...(optionIds.length ? { optionIds } : {}), ...(freeText ? { freeText } : {}) });
});
document.getElementById('sendText').addEventListener('click', () => {
  const freeText = freeTextValue();
  if (!freeText) { document.getElementById('status').textContent = 'Write an answer first, or tap an option above.'; return; }
  answer({ freeText });
});
</script>`,
  );
}
