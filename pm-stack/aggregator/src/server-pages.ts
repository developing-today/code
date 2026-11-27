// Standalone token-carrying pages rendered outside the dashboard
// shell: the Needs-Drew answer page (#8568) and the decision-set
// pages (#8766), plus the PWA manifest. Split out of server.ts (#13052).
import { type NeedsDrewQuestion } from "./needs-drew.ts";
import {
  decisionSetAging,
  decisionSetProgress,
  isDeferLean,
  decisionSetSummary,
  decisionSetsForView,
  listDecisionSets,
  effectiveQuestionMode,
  type DecisionItem,
  type DecisionSet,
  type DecisionSetView,
  type DecisionSubmission,
  decisionsDraftScript,
} from "./decisions.ts";
import { appendDecisionEvents, type DecisionEventInput } from "./decision-events.ts";
import { questionOptions, type NeedsUserListItem } from "./needs-user.ts";
import type { Comment } from "./model.ts";
import { questionsSectionHtml } from "./render.ts";
import { esc } from "./render.ts";
import { cfg } from "./server-wiring.ts";
import { json, html } from "./server-respond.ts";
import { hostedDecisionPageUrl } from "./decision-links.ts";

// --- Needs-Drew questions (#8568) ------------------------------------------

/** The token-carrying answer-page URL for a raised question. */
export function needsDrewAnswerUrl(item: NeedsDrewQuestion): string {
  return `${cfg.publicOrigin}/answer/${encodeURIComponent(item.ledgerId)}?t=${item.token}`;
}

export const ANSWER_PAGE_STYLE = `<style>
  body { font-family: system-ui, sans-serif; background: #0e1117; color: #e6edf3; margin: 0; padding: 2rem 1rem; }
  main { max-width: 40rem; margin: 0 auto; }
  h1 { font-size: 1.4rem; line-height: 1.35; }
  .context { color: #9da7b1; white-space: pre-wrap; }
  .links a { color: #58a6ff; display: block; margin: 0.35rem 0; overflow-wrap: anywhere; }
  .options { display: grid; gap: 0.6rem; margin: 1.2rem 0; }
  button.option { text-align: left; padding: 0.8rem 1rem; font-size: 1rem; border-radius: 0.5rem;
    border: 1px solid #30363d; background: #161b22; color: #e6edf3; cursor: pointer; }
  button.option:hover { border-color: #58a6ff; }
  textarea { width: 100%; min-height: 4.5rem; box-sizing: border-box; background: #161b22; color: #e6edf3;
    border: 1px solid #30363d; border-radius: 0.5rem; padding: 0.6rem; }
  button.submit { margin-top: 0.6rem; padding: 0.7rem 1.2rem; font-size: 1rem; border-radius: 0.5rem;
    border: 0; background: #1f6feb; color: #fff; cursor: pointer; }
  .confirm { border: 1px solid #238636; border-radius: 0.5rem; padding: 1rem; }
</style>`;

/** Render GET /needs-user: open questions in answering order, each
 * with its blocked-by count and a priority setting. */
export function renderNeedsUserPage(items: NeedsUserListItem[], stagesAvailable: boolean): string {
  const priorityOptions = (current: number | null) =>
    [`<option value=""${current === null ? " selected" : ""}>No priority</option>`]
      .concat([1, 2, 3, 4, 5].map((p) => `<option value="${p}"${current === p ? " selected" : ""}>P${p}</option>`))
      .join("");
  const rows = items
    .map(
      (i) => `<li class="item"><p class="q">${esc(i.question)}</p>
<p class="context">Raised ${esc(i.raisedAt)} by ${esc(i.source)} · <strong>${i.blockedByCount}</strong> ${
        i.blockedByCount === 1 ? "issue waits" : "issues wait"
      } on this answer</p>
<label>Priority <select data-ledger="${esc(i.ledgerId)}"${stagesAvailable ? "" : " disabled"}>${priorityOptions(
        i.priority,
      )}</select></label></li>`,
    )
    .join("");
  return `<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1"><title>Needs user — PM Stack</title>${ANSWER_PAGE_STYLE}
<style>ol { padding-left: 1.2rem; } .item { margin: 0 0 1.2rem; } .q { margin: 0 0 0.3rem; font-size: 1.05rem; }
select { background: #161b22; color: #e6edf3; border: 1px solid #30363d; border-radius: 0.4rem; padding: 0.3rem; }</style></head>
<body><main><h1>Needs user (${items.length})</h1>
${stagesAvailable ? "" : `<p class="context">The stage store is unavailable, so priorities can't be set right now.</p>`}
${items.length ? `<ol>${rows}</ol>` : `<p class="context">Nothing is waiting on you.</p>`}
<p id="status" class="context"></p>
<script>
document.querySelectorAll('select[data-ledger]').forEach((sel) => {
  sel.addEventListener('change', async () => {
    const status = document.getElementById('status');
    const priority = sel.value === '' ? null : Number(sel.value);
    try {
      const res = await fetch('/api/needs-user/' + encodeURIComponent(sel.dataset.ledger) + '/priority', {
        method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ priority }) });
      if (!res.ok) throw new Error('HTTP ' + res.status);
      status.textContent = 'Priority saved.';
    } catch { status.textContent = 'Saving the priority failed. Please try again.'; }
  });
});
</script></main></body></html>`;
}

/** Render GET /answer/<ledgerId>. The caller has already verified the
 * token; the 403 shape renders nothing about the question. */
export function renderAnswerPage(item: NeedsDrewQuestion): string {
  const links = item.links
    .filter((l) => /^https?:\/\//.test(l))
    .map((l) => `<a href="${esc(l)}" rel="noreferrer">${esc(l)}</a>`)
    .join("");
  if (item.answer) {
    const chosen = item.answer.optionId
      ? (questionOptions(item).find((o) => o.id === item.answer?.optionId)?.label ?? item.answer.optionId)
      : null;
    return `<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1"><title>Answered — PM Stack</title>${ANSWER_PAGE_STYLE}</head>
<body><main><h1>${esc(item.question)}</h1>
<div class="confirm"><p>Answered${chosen ? `: <strong>${esc(chosen)}</strong>` : ""}.</p>
${item.answer.freeText ? `<p>${esc(item.answer.freeText)}</p>` : ""}
<p class="context">Recorded at ${esc(item.answer.at)} — the fleet worker has been unblocked.</p></div>
</main></body></html>`;
  }
  const buttons = questionOptions(item)
    .map((o) => `<button class="option" type="button" data-option="${esc(o.id)}">${esc(o.label)}</button>`)
    .join("");
  return `<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1"><title>Needs Drew — PM Stack</title>${ANSWER_PAGE_STYLE}</head>
<body><main><h1>${esc(item.question)}</h1>
${item.context ? `<p class="context">${esc(item.context)}</p>` : ""}
${links ? `<div class="links">${links}</div>` : ""}
<div class="options">${buttons}</div>
<label>Or answer in your own words (optional, sent with a tapped option or on its own):
<textarea id="freeText" placeholder="Free-text answer"></textarea></label>
<button class="submit" type="button" id="sendText">Send free-text answer</button>
<p id="status" class="context"></p>
<script>
const token = ${JSON.stringify(item.token)};
const endpoint = ${JSON.stringify(`/api/needs-drew/${encodeURIComponent(item.ledgerId)}/answer`)};
async function answer(payload) {
  const status = document.getElementById('status');
  status.textContent = 'Sending…';
  try {
    const res = await fetch(endpoint, { method: 'POST', headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ token, ...payload }) });
    if (!res.ok) throw new Error('HTTP ' + res.status);
    status.textContent = 'Answer recorded — thank you. The fleet worker has been unblocked.';
    document.querySelectorAll('button').forEach((b) => { b.disabled = true; });
  } catch { status.textContent = 'Sending failed — please try again.'; }
}
document.querySelectorAll('button.option').forEach((b) => {
  b.addEventListener('click', () => {
    const freeText = document.getElementById('freeText').value.trim();
    answer({ optionId: b.dataset.option, ...(freeText ? { freeText } : {}) });
  });
});
document.getElementById('sendText').addEventListener('click', () => {
  const freeText = document.getElementById('freeText').value.trim();
  if (!freeText) { document.getElementById('status').textContent = 'Write an answer first, or tap an option above.'; return; }
  answer({ freeText });
});
</script>
</main></body></html>`;
}

export function renderAnswerForbidden(): string {
  return `<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1"><title>Not available — PM Stack</title>${ANSWER_PAGE_STYLE}</head>
<body><main><h1>Answer link not available</h1>
<p class="context">This answer link is unknown or its token does not match. Nothing about the question is shown without the correct link.</p>
</main></body></html>`;
}

// --- Decision sets (#8766) -------------------------------------------------

/** The canonical credential-free hosted read URL for a raised set. */
export function decisionsPageUrl(set: DecisionSet): string {
  return hostedDecisionPageUrl(set.setId);
}

/**
 * The live Questions section for Today / Needs-you (#9354): reads the
 * decision-set store at request time, keeps sets with open questions,
 * newest first. A store read failure renders the honest unavailable
 * section (questionsSectionHtml(null, ...)), never a confident zero.
 * (listDecisionSets itself reads an absent/corrupt store as empty,
 * following the store's existing contract.)
 */
export async function questionsHtml(): Promise<string> {
  try {
    const sets = await listDecisionSets(cfg.decisionsFile);
    const byId = new Map(sets.map((s) => [s.setId, s]));
    const summaries = sets
      .map((s) => decisionSetSummary(s))
      .filter((s) => s.openQuestions > 0)
      .sort((a, b) => b.createdAt.localeCompare(a.createdAt));
    return questionsSectionHtml(summaries, (s) => {
      const set = byId.get(s.setId);
      return set ? decisionsPageUrl(set) : hostedDecisionPageUrl(s.setId);
    });
  } catch {
    return questionsSectionHtml(null, () => "/decisions");
  }
}

/** Append decision-set events (#9335 v2): best-effort — an event-log
 * failure is logged and never fails the interaction it records. */
export async function logDecisionEvents(inputs: DecisionEventInput[]): Promise<void> {
  if (!inputs.length) return;
  try {
    await appendDecisionEvents(cfg.decisionsEventsFile, cfg.decisionsOutboxFile, inputs);
  } catch (err) {
    console.error(`decision events: append failed (${err instanceof Error ? err.message : "error"})`);
  }
}

export function questionRefFor(
  set: DecisionSet,
  ledgerId: string,
  qid: string,
): { ledgerId: string; qid: string; questionIndex?: number } {
  const item = set.items.find((i) => i.ledgerId === ledgerId);
  const idx = item ? item.questions.findIndex((q) => q.qid === qid) : -1;
  return { ledgerId, qid, ...(idx >= 0 ? { questionIndex: idx + 1 } : {}) };
}

export function eventTypeForKind(kind: DecisionSubmission["kind"]): "submit" | "defer" | "request-info" {
  return kind === "answered" ? "submit" : kind === "deferred" ? "defer" : "request-info";
}

export function submissionEventPayload(sub: DecisionSubmission): Record<string, unknown> {
  return {
    kind: sub.kind,
    ...(sub.optionId ? { optionId: sub.optionId } : {}),
    ...(sub.optionIds ? { optionIds: sub.optionIds } : {}),
    ...(sub.freeText ? { freeText: sub.freeText.slice(0, 2000) } : {}),
    ...(sub.customAnswer ? { customAnswer: sub.customAnswer.slice(0, 2000) } : {}),
    ...(sub.lean ? { lean: sub.lean } : {}),
    ...(sub.note ? { note: sub.note.slice(0, 2000) } : {}),
    ...(sub.comment ? { comment: sub.comment.slice(0, 2000) } : {}),
  };
}

/** Parse a decisions submissions array — shared by /submit and by
 * /defer's filled-answers-win payload (#8766). */
export function parseDecisionSubmissions(
  raw: unknown,
): { ok: true; value: DecisionSubmission[] } | { ok: false; error: string } {
  if (!Array.isArray(raw) || raw.length < 1 || raw.length > 256) {
    return { ok: false, error: "submissions must be an array of 1-256 entries" };
  }
  const submissions: DecisionSubmission[] = [];
  for (const entry of raw) {
    if (!entry || typeof entry !== "object" || Array.isArray(entry)) {
      return { ok: false, error: "submissions entries must be objects" };
    }
    const s = entry as Record<string, unknown>;
    const ledgerId = typeof s.ledgerId === "string" ? s.ledgerId : "";
    const qid = typeof s.qid === "string" ? s.qid : "";
    const kind = s.kind;
    if (!ledgerId || !qid || (kind !== "answered" && kind !== "deferred" && kind !== "request-info")) {
      return { ok: false, error: "submissions entries require ledgerId, qid and kind answered|deferred|request-info" };
    }
    if (kind === "deferred" && !isDeferLean(s.lean)) {
      return { ok: false, error: "deferred submissions require a lean of none|skeptical|approve" };
    }
    submissions.push({
      ledgerId,
      qid,
      kind,
      ...(typeof s.optionId === "string" ? { optionId: s.optionId } : {}),
      ...(Array.isArray(s.optionIds)
        ? { optionIds: s.optionIds.filter((x): x is string => typeof x === "string") }
        : {}),
      ...(typeof s.freeText === "string" ? { freeText: s.freeText } : {}),
      ...(typeof s.customAnswer === "string" ? { customAnswer: s.customAnswer } : {}),
      ...(isDeferLean(s.lean) ? { lean: s.lean } : {}),
      ...(typeof s.note === "string" ? { note: s.note } : {}),
      ...(typeof s.comment === "string" ? { comment: s.comment } : {}),
    });
  }
  return { ok: true, value: submissions };
}

export const DECISIONS_PAGE_STYLE = `${ANSWER_PAGE_STYLE}<style>
  .group { color: #9da7b1; text-transform: uppercase; letter-spacing: 0.06em; font-size: 0.85rem; margin: 1.6rem 0 0.4rem; }
  .item { border: 1px solid #30363d; border-radius: 0.6rem; padding: 1rem; margin: 0.8rem 0; background: #11161d; }
  .item h2 { font-size: 1.1rem; margin: 0 0 0.3rem; line-height: 1.35; }
  .q { border-top: 1px solid #21262d; padding: 0.8rem 0 0.2rem; margin-top: 0.8rem; }
  .q p.qq { font-weight: 600; margin: 0 0 0.3rem; }
  .opt { display: block; padding: 0.45rem 0; cursor: pointer; }
  .opt input { margin-right: 0.5rem; }
  select.defer { width: 100%; box-sizing: border-box; margin-top: 0.5rem; padding: 0.55rem; background: #161b22; color: #e6edf3;
    border: 1px solid #30363d; border-radius: 0.5rem; }
  .done { color: #3fb950; font-size: 0.92rem; padding: 0.35rem 0; }
  .bar { border: 1px solid #30363d; border-radius: 0.6rem; padding: 1rem; margin: 1.2rem 0; background: #11161d; }
  .bar button { margin: 0.3rem 0.4rem 0.3rem 0; }
  button.sendback { background: #6e3feb; }
  .provided { border: 1px solid #1f6feb; border-radius: 0.5rem; padding: 0.6rem 0.8rem; margin: 0.5rem 0; background: #0d1b2e; }
  .provided .ph { color: #58a6ff; font-weight: 600; margin: 0 0 0.3rem; }
  .provided p { margin: 0; white-space: pre-wrap; }
  button.reqinfo { background: #8250df; margin-top: 0.4rem; }
  .bulkchk { font-size: 0.92rem; color: #9da7b1; }
  #status { min-height: 1.2em; }
  .chip { display: inline-block; border: 1px solid #30363d; border-radius: 999px; padding: 0.05rem 0.5rem; font-size: 0.78rem; color: #9da7b1; margin: 0 0.25rem 0.25rem 0; }
  .chip.reco { border-color: #238636; color: #3fb950; }
  .desc { border: 1px solid #21262d; border-radius: 0.5rem; padding: 0.6rem 0.8rem; margin: 0.5rem 0; background: #0d1117; white-space: pre-wrap; color: #c9d1d9; }
  .combo { color: #9da7b1; font-size: 0.85rem; }
  .reco-note { color: #3fb950; font-size: 0.9rem; }
  label.own { display: block; margin-top: 0.6rem; color: #9da7b1; font-size: 0.92rem; }
  button.defaults { background: #238636; }
  .submitted-bar { margin: 0.6rem 0; color: #9da7b1; font-size: 0.95rem; }
  section.item.submitted { display: none; }
  body.show-submitted section.item.submitted { display: block; opacity: 0.85; }
  .group.submitted-group { display: none; }
  body.show-submitted .group.submitted-group { display: block; }
  button.submit.top { margin: 0.2rem 0 0.4rem; }
</style>`;

/** Render GET /decisions/<setId>. The caller has already verified the
 * token; the 403 shape renders nothing about the set. Only open
 * questions are interactive; resolved ones show as a compact recorded
 * line. An item whose questions are all resolved renders as a
 * submitted card, hidden by default (Drew, 2026-11-13): the page's
 * "Show submitted items" toggle brings it back for review, since the
 * page has no undo. */
export function renderDecisionsPage(set: DecisionSet): string {
  const progress = decisionSetProgress(set);
  const groups: string[] = [];
  for (const item of set.items) if (!groups.includes(item.group)) groups.push(item.group);
  const itemCard = (item: DecisionItem): string => {
    const open = item.questions.filter((q) => !q.resolution);
    const links = item.links
      .filter((l) => /^https?:\/\//.test(l))
      .map((l) => `<a href="${esc(l)}" rel="noreferrer">${esc(l)}</a>`)
      .join("");
    const resolvedLines = item.questions
      .filter((q) => q.resolution)
      .map((q) => {
        const r = q.resolution!;
        const what =
          r.kind === "deferred"
            ? `deferred to the orchestrator${r.lean && r.lean !== "none" ? ` (lean: ${esc(r.lean)})` : ""}${r.optionLabel ? ` — your tentative pick: ${esc(r.optionLabel)}` : ""}${r.comment ? ` — comment: ${esc(r.comment)}` : ""}`
            : r.kind === "request-info"
              ? `more information requested${r.note ? ` — you said: ${esc(r.note)}` : ""}`
              : `answered${
                  r.optionLabels && r.optionLabels.length > 1
                    ? `: ${esc(r.optionLabels.join(" + "))}`
                    : r.optionLabel
                      ? `: ${esc(r.optionLabel)}`
                      : r.freeText
                        ? `: ${esc(r.freeText)}`
                        : ""
                }${r.customAnswer ? ` — own answer: ${esc(r.customAnswer)}` : ""}`;
        return `<div class="done">✓ ${esc(q.question)} — ${what}</div>`;
      })
      .join("");
    if (open.length === 0) {
      return `<section class="item submitted" data-ledger="${esc(item.ledgerId)}" data-submitted="1">
<h2>${esc(item.title)}</h2>
${links ? `<div class="links">${links}</div>` : ""}
${resolvedLines}
<div class="done">✓ Submitted — hidden by default. Tick "Show submitted items" at the top of the page to review it.</div>
</section>`;
    }
    const sortedOpen = open
      .map((q, i) => ({ q, i }))
      .sort((a, b) => (a.q.order ?? a.i) - (b.q.order ?? b.i))
      .map((x) => x.q);
    const questionBlocks = sortedOpen
      .map((q) => {
        const mode = effectiveQuestionMode(q);
        const recommended = q.recommendedOptionIds ?? [];
        const chips = (tags?: string[]): string =>
          (tags ?? []).map((t) => `<span class="chip">${esc(t)}</span>`).join("");
        const inputs = q.options
          .map((o) => {
            const combo =
              o.combinationOf && o.combinationOf.length
                ? ` <span class="combo">combines ${o.combinationOf
                    .map((cid) => esc(q.options.find((x) => x.id === cid)?.label ?? cid))
                    .join(" + ")}</span>`
                : "";
            const isReco = recommended.includes(o.id);
            return `<label class="opt"><input class="optin" type="${mode === "multi" ? "checkbox" : "radio"}" name="opt-${esc(q.qid)}" value="${esc(o.id)}"${isReco ? ` checked data-recommended="1"` : ""}>${esc(o.label)}${isReco ? ` <span class="chip reco">recommended</span>` : ""}${chips(o.tags)}${combo}</label>`;
          })
          .join("");
        return `<div class="q" data-ledger="${esc(item.ledgerId)}" data-qid="${esc(q.qid)}" data-mode="${mode}" data-recommended="${esc(JSON.stringify(recommended))}">
<p class="qq">${esc(q.question)}${chips(q.tags)}</p>
${q.description ? `<div class="desc">${esc(q.description)}</div>` : ""}
${q.infoProvided ? `<div class="provided"><p class="ph">Information you requested</p><p>${esc(q.infoProvided)}</p></div>` : ""}
${q.context ? `<p class="context">${esc(q.context)}</p>` : ""}
${recommended.length ? `<p class="reco-note">Recommended default preselected — it is not recorded until you submit.</p>` : ""}
${inputs}
<label class="own">My own answer<textarea class="ft" placeholder="My own answer — or a comment (sent back with any action on this question)"></textarea></label>
<select class="defer">
<option value="">I'll answer this one</option>
<option value="none">Defer</option>
<option value="skeptical">Defer — leaning no</option>
<option value="approve">Defer — leaning yes</option>
</select>
<textarea class="infonote" placeholder="Need more information first? Say what's missing (optional)"></textarea>
<button class="reqinfo" type="button" data-reqinfo>Request more information</button>
</div>`;
      })
      .join("");
    const submitBtn = (cls: string): string =>
      `<button class="${cls}" type="button" data-submit-item="${esc(item.ledgerId)}">Submit this item</button>`;
    return `<section class="item" data-ledger="${esc(item.ledgerId)}">
<label class="bulkchk"><input type="checkbox" class="bulk"> select this item for a bulk action below (defer or request info)</label>
<h2>${esc(item.title)}</h2>
${links ? `<div class="links">${links}</div>` : ""}
${resolvedLines}
${open.length > 1 ? submitBtn("submit top") : ""}
${questionBlocks}
${submitBtn("submit")}
</section>`;
  };
  const isSubmittedItem = (i: DecisionItem): boolean => i.questions.every((q) => q.resolution);
  const submittedCount = set.items.filter(isSubmittedItem).length;
  const body = groups
    .map((g) => {
      const groupItems = set.items
        .filter((i) => i.group === g)
        .map((item, i) => ({ item, i }))
        .sort((a, b) => (a.item.order ?? a.i) - (b.item.order ?? b.i))
        .map((x) => x.item);
      const cards = groupItems.map(itemCard).join("");
      const allSubmitted = groupItems.length > 0 && groupItems.every(isSubmittedItem);
      return cards ? `<div class="group${allSubmitted ? " submitted-group" : ""}">${esc(g)}</div>${cards}` : "";
    })
    .join("");
  const caughtUp = progress.openQuestions === 0;
  return `<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1"><title>Decisions — PM Stack</title>${DECISIONS_PAGE_STYLE}</head>
<body><main><h1>${esc(set.title)}</h1>
<p class="context">${progress.openQuestions} question${progress.openQuestions === 1 ? "" : "s"} open across ${progress.openItems} item${progress.openItems === 1 ? "" : "s"} · ${progress.answered} answered · ${progress.deferred} deferred · ${progress.infoRequested} sent back for more information so far. Fill in what you want — you can submit a partially-filled item; what you filled in is recorded and the rest stays here.</p>
${caughtUp ? `<div class="confirm"><p>All caught up — every question in this set has been answered or sent back. Nothing left on this page.</p></div>` : ""}
<div class="submitted-bar" id="submittedBar"${submittedCount === 0 ? " hidden" : ""}><label><input type="checkbox" id="showSubmitted"> Show submitted items (<span id="submittedCount">${submittedCount}</span>)</label></div>
${body}
<div class="bar">
<p class="context">Bulk actions</p>
<button class="submit" type="button" id="submitAll">Submit everything I've filled in</button>
<button class="submit defaults" type="button" id="submitDefaults">Submit defaults</button>
<p class="context">Submit defaults records the raiser's recommended preselection for every open question that has one (plus any "my own answer" text you typed on those questions). Preselected defaults are shown selected, but they are NOT an answer until you submit them.</p>
<p class="context">Defer checked items to the orchestrator instead of answering — they leave this page and become the orchestrator's job. Precedence per question: (1) the question's own defer setting wins, with its tentative pick and comment riding along as guidance; (2) a filled-in answer submits as an answer; (3) only a question nobody touched uses the default flavor picked here:</p>
<select class="defer" id="bulkLean">
<option value="none">Defer</option>
<option value="skeptical">Defer — leaning no</option>
<option value="approve">Defer — leaning yes</option>
</select>
<button class="submit sendback" type="button" id="deferChecked">Defer checked to orchestrator</button>
<button class="submit sendback" type="button" id="deferAll">Defer ALL remaining</button>
<p class="context">Or request more information for the checked items — they leave this page and come back in a later set once the information has been gathered:</p>
<textarea class="infonote" id="bulkInfoNote" placeholder="Comment for the bulk actions — rides back with the checked items (for request-info: say what's missing)"></textarea>
<button class="submit reqinfo" type="button" id="reqinfoChecked">Request more info for checked items</button>
</div>
<p id="status" class="context"></p>
<script>
var TOKEN = ${JSON.stringify(set.token)};
var SET_ID = ${JSON.stringify(set.setId)};
function status(msg) { document.getElementById('status').textContent = msg; }
function pickedValues(block) {
  var vals = [];
  block.querySelectorAll('input.optin:checked').forEach(function (i) { vals.push(i.value); });
  return vals;
}
function applySelection(sub, block, vals) {
  if (!vals.length) return;
  if (block.getAttribute('data-mode') === 'multi') sub.optionIds = vals;
  else sub.optionId = vals[0];
}
function collect(scopeLedger) {
  var subs = [];
  document.querySelectorAll('.q').forEach(function (block) {
    var ledger = block.getAttribute('data-ledger');
    if (scopeLedger && ledger !== scopeLedger) return;
    var qid = block.getAttribute('data-qid');
    var lean = block.querySelector('select.defer').value;
    if (lean) {
      var dsub = { ledgerId: ledger, qid: qid, kind: 'deferred', lean: lean };
      applySelection(dsub, block, pickedValues(block));
      var dtext = block.querySelector('textarea.ft').value.trim();
      if (dtext) dsub.comment = dtext;
      subs.push(dsub); return;
    }
    var vals = pickedValues(block);
    var text = block.querySelector('textarea.ft').value.trim();
    if (vals.length || text) {
      var sub = { ledgerId: ledger, qid: qid, kind: 'answered' };
      applySelection(sub, block, vals);
      if (text) { sub.freeText = text; sub.customAnswer = text; }
      subs.push(sub);
    }
  });
  return subs;
}
function applyOpenState(state) {
  var openByItem = {};
  state.items.forEach(function (i) { openByItem[i.ledgerId] = i.openQids; });
  document.querySelectorAll('.q').forEach(function (block) {
    var open = openByItem[block.getAttribute('data-ledger')] || [];
    if (open.indexOf(block.getAttribute('data-qid')) === -1) block.remove();
  });
  document.querySelectorAll('section.item').forEach(function (card) {
    if (card.classList.contains('submitted')) return;
    if (!card.querySelector('.q')) {
      card.classList.add('submitted');
      card.setAttribute('data-submitted', '1');
      card.querySelectorAll('button.submit').forEach(function (el) { el.remove(); });
      var bulk = card.querySelector('.bulkchk');
      if (bulk) bulk.remove();
      var note = document.createElement('div');
      note.className = 'done';
      note.textContent = '✓ Submitted — hidden by default. Tick "Show submitted items" at the top of the page to review it.';
      card.appendChild(note);
    }
  });
  refreshSubmitted();
  if (!document.querySelector('section.item:not(.submitted)')) {
    status('All caught up — every question in this set is recorded. You can close this page.');
  }
}
async function post(path, payload) {
  var res = await fetch(path, { method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify(Object.assign({ token: TOKEN }, payload)) });
  if (!res.ok) throw new Error('HTTP ' + res.status);
  return res.json();
}
async function submit(scopeLedger) {
  var subs = collect(scopeLedger);
  if (subs.length === 0) { status('Nothing filled in yet — pick an option, write an answer, or choose a defer.'); return; }
  status('Sending…');
  try {
    var out = await post('/api/decisions/' + encodeURIComponent(SET_ID) + '/submit', { submissions: subs });
    applyOpenState(out);
    var applied = out.outcomes.filter(function (o) { return o.applied; }).length;
    status('Recorded ' + applied + ' — ' + out.remainingOpenQuestions + ' question(s) still open.');
  } catch (e) { status('Sending failed — please try again.'); }
}
async function sendBack(all) {
  // The flavor picked here is only the DEFAULT for questions nobody
  // touched: the gather below sends each question's own defer setting
  // and each filled answer as submissions, the server records those
  // first, and this payload lean reaches only what is still open.
  var lean = document.getElementById('bulkLean').value;
  var payload;
  var scopeIds = null;
  if (all) { payload = { lean: lean, allRemaining: true }; }
  else {
    var ids = [];
    document.querySelectorAll('section.item').forEach(function (card) {
      if (card.querySelector('input.bulk').checked) ids.push(card.getAttribute('data-ledger'));
    });
    if (ids.length === 0) { status('Check at least one item first, or use ALL remaining.'); return; }
    payload = { lean: lean, ledgerIds: ids };
    scopeIds = ids;
  }
  // Filled answers win over the bulk defer: everything already filled
  // in inside the scope rides along as submissions — the server records
  // them first (answers stay rulings, per-question defers keep their
  // own lean and comment) and defers only what is still unanswered.
  var subs = [];
  document.querySelectorAll('.q').forEach(function (block) {
    var ledger = block.getAttribute('data-ledger');
    if (scopeIds && scopeIds.indexOf(ledger) === -1) return;
    var qid = block.getAttribute('data-qid');
    var qlean = block.querySelector('select.defer').value;
    var text = block.querySelector('textarea.ft').value.trim();
    if (qlean) {
      var dsub = { ledgerId: ledger, qid: qid, kind: 'deferred', lean: qlean };
      applySelection(dsub, block, pickedValues(block));
      if (text) dsub.comment = text;
      subs.push(dsub); return;
    }
    var vals = pickedValues(block);
    if (vals.length || text) {
      var sub = { ledgerId: ledger, qid: qid, kind: 'answered' };
      applySelection(sub, block, vals);
      if (text) { sub.freeText = text; sub.customAnswer = text; }
      subs.push(sub);
    }
  });
  if (subs.length) payload.submissions = subs;
  var bulkComment = document.getElementById('bulkInfoNote').value.trim();
  if (bulkComment) payload.comment = bulkComment;
  status('Sending back…');
  try {
    var out = await post('/api/decisions/' + encodeURIComponent(SET_ID) + '/defer', payload);
    applyOpenState(out);
    var recorded = (out.outcomes || []).filter(function (o) { return o.applied; }).length;
    var msg = 'Deferred ' + out.deferredCount + ' question(s) to the orchestrator';
    if (recorded) msg = 'Recorded ' + recorded + ' filled-in answer(s) first; deferred the rest — ' + out.deferredCount + ' question(s) deferred to the orchestrator';
    status(msg + ' — ' + out.remainingOpenQuestions + ' still open.');
  } catch (e) { status('Sending failed — please try again.'); }
}
document.querySelectorAll('[data-submit-item]').forEach(function (b) {
  b.addEventListener('click', function () { submit(b.getAttribute('data-submit-item')); });
});
document.getElementById('submitAll').addEventListener('click', function () { submit(null); });
document.getElementById('deferChecked').addEventListener('click', function () { sendBack(false); });
document.getElementById('deferAll').addEventListener('click', function () { sendBack(true); });
async function requestInfoOne(block) {
  var note = block.querySelector('textarea.infonote').value.trim();
  var sub = { ledgerId: block.getAttribute('data-ledger'), qid: block.getAttribute('data-qid'), kind: 'request-info' };
  if (note) sub.note = note;
  status('Requesting more information…');
  try {
    var out = await post('/api/decisions/' + encodeURIComponent(SET_ID) + '/submit', { submissions: [sub] });
    applyOpenState(out);
    status('Recorded — that question will come back in a later set with the information added. ' + out.remainingOpenQuestions + ' question(s) still open.');
  } catch (e) { status('Sending failed — please try again.'); }
}
async function requestInfoChecked() {
  var ids = [];
  document.querySelectorAll('section.item').forEach(function (card) {
    if (card.querySelector('input.bulk').checked) ids.push(card.getAttribute('data-ledger'));
  });
  if (ids.length === 0) { status('Check at least one item first.'); return; }
  var note = document.getElementById('bulkInfoNote').value.trim();
  var payload = { ledgerIds: ids };
  if (note) payload.note = note;
  status('Requesting more information…');
  try {
    var out = await post('/api/decisions/' + encodeURIComponent(SET_ID) + '/request-info', payload);
    applyOpenState(out);
    status('Recorded ' + out.requestedCount + ' question(s) to come back with more information — ' + out.remainingOpenQuestions + ' still open.');
  } catch (e) { status('Sending failed — please try again.'); }
}
document.querySelectorAll('[data-reqinfo]').forEach(function (b) {
  b.addEventListener('click', function () { requestInfoOne(b.closest('.q')); });
});
document.getElementById('reqinfoChecked').addEventListener('click', requestInfoChecked);
async function submitDefaults() {
  var subs = [];
  document.querySelectorAll('.q').forEach(function (block) {
    var reco = [];
    try { reco = JSON.parse(block.getAttribute('data-recommended') || '[]'); } catch (e) { reco = []; }
    if (!reco.length) return;
    var sub = { ledgerId: block.getAttribute('data-ledger'), qid: block.getAttribute('data-qid'), kind: 'answered' };
    if (block.getAttribute('data-mode') === 'multi') sub.optionIds = reco; else sub.optionId = reco[0];
    var text = block.querySelector('textarea.ft').value.trim();
    if (text) { sub.freeText = text; sub.customAnswer = text; }
    subs.push(sub);
  });
  if (!subs.length) { status('No recommended defaults in this set.'); return; }
  status('Submitting defaults…');
  try {
    var out = await post('/api/decisions/' + encodeURIComponent(SET_ID) + '/submit', { submissions: subs });
    applyOpenState(out);
    var applied = out.outcomes.filter(function (o) { return o.applied; }).length;
    status('Recorded ' + applied + ' default(s) — ' + out.remainingOpenQuestions + ' question(s) still open.');
  } catch (e) { status('Sending failed — please try again.'); }
}
var defaultsBtn = document.getElementById('submitDefaults');
if (defaultsBtn) defaultsBtn.addEventListener('click', submitDefaults);
// --- Submitted items hide by default (Drew, 2026-11-13): the toggle
// brings them back for review; the count tracks server-rendered
// submitted cards and cards submitted during this session alike.
function refreshSubmitted() {
  var n = document.querySelectorAll('section.item.submitted').length;
  var bar = document.getElementById('submittedBar');
  if (bar) bar.hidden = n === 0;
  var c = document.getElementById('submittedCount');
  if (c) c.textContent = String(n);
}
var showSubmittedBox = document.getElementById('showSubmitted');
if (showSubmittedBox) showSubmittedBox.addEventListener('change', function () {
  document.body.classList.toggle('show-submitted', showSubmittedBox.checked);
});
refreshSubmitted();
// --- Draft event reporting (#9335): typed-but-not-submitted changes are
// events too. Debounced per question (2.5s of inactivity), never per
// keystroke; the server drafts push remains the durable state copy.
var draftEventTimers = {};
function reportDraftEvent(block) {
  var qid = block.getAttribute('data-qid');
  if (!qid) return;
  if (draftEventTimers[qid]) clearTimeout(draftEventTimers[qid]);
  draftEventTimers[qid] = setTimeout(function () {
    var payload = { selected: pickedValues(block) };
    var ft = block.querySelector('textarea.ft');
    if (ft && ft.value.trim()) payload.customAnswer = ft.value.trim().slice(0, 2000);
    var leanEl = block.querySelector('select.defer');
    if (leanEl && leanEl.value) payload.lean = leanEl.value;
    var noteEl = block.querySelector('textarea.infonote');
    if (noteEl && noteEl.value.trim()) payload.note = noteEl.value.trim().slice(0, 2000);
    try {
      fetch('/api/decisions/' + encodeURIComponent(SET_ID) + '/events', {
        method: 'POST', headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ token: TOKEN, events: [{ type: 'draft', questionRef: { ledgerId: block.getAttribute('data-ledger'), qid: qid }, payload: payload }] }),
        keepalive: true
      }).catch(function () {});
    } catch (e) { /* events are best-effort */ }
  }, 2500);
}
document.addEventListener('input', function (e) { var b = e.target && e.target.closest ? e.target.closest('.q') : null; if (b) reportDraftEvent(b); });
document.addEventListener('change', function (e) { var b = e.target && e.target.closest ? e.target.closest('.q') : null; if (b) reportDraftEvent(b); });
${decisionsDraftScript(set.setId)}
</script>
</main></body></html>`;
}

/** Render GET /decisions — the decision-sets index (#8867, aging #14064). Lists every
 * set in the store with counts live from decisionSetSummary (the same
 * numbers the set page itself reports), in today / this-week / recent
 * (last 7 days, rolling — the index default, so the landing view is
 * the current workload, not the backlog) / all views with an
 * open-only toggle, newest first. Sets idle past the abandonment
 * window collapse under an "abandoned" fold on the all view with a
 * pointer at the bulk-dismiss operator endpoint. Each row links to the
 * set's own token-carrying page; the index carries no token of its own
 * and sits behind the same access as every other aggregator page
 * (tailnet origin / Cloudflare Access on pm.security.cab) — no new
 * public surface, and the per-set token model is untouched. */
export const CHICAGO_STAMP = new Intl.DateTimeFormat("en-US", {
  timeZone: "America/Chicago",
  year: "numeric",
  month: "short",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
  timeZoneName: "short",
});

export function renderDecisionsIndex(allSets: DecisionSet[], view: DecisionSetView, openOnly: boolean): string {
  const inView = decisionSetsForView(allSets, view);
  const visible = openOnly ? inView.filter((s) => decisionSetSummary(s).openQuestions > 0) : inView;
  const totalOpen = inView.reduce((n, s) => n + decisionSetSummary(s).openQuestions, 0);
  const abandoned = view === "all" ? inView.filter((s) => decisionSetAging(s) === "abandoned") : [];
  const abandonedIds = abandoned.map((s) => s.setId);
  const viewTab = (v: DecisionSetView, label: string): string =>
    v === view
      ? `<strong>${label}</strong>`
      : `<a href="/decisions?view=${v}${openOnly ? "&open=1" : ""}">${label}</a>`;
  const toggle = openOnly
    ? `<strong>Open only</strong> · <a href="/decisions?view=${view}">Show all</a>`
    : `<a href="/decisions?view=${view}&open=1">Open only</a>`;
  const rows = visible
    .map((set) => {
      const s = decisionSetSummary(set);
      const stamp = Number.isFinite(Date.parse(s.createdAt))
        ? CHICAGO_STAMP.format(new Date(s.createdAt))
        : s.createdAt;
      const aging = decisionSetAging(set);
      const agingChip = aging === "abandoned" ? ` · abandoned` : aging === "settled" ? ` · settled` : "";
      const breakdown = `${s.answered} answered · ${s.deferred} deferred · ${s.infoRequested} sent back for more info`;
      const state = s.fullyResolved ? `all settled — ${breakdown}` : `${s.openQuestions} open · ${breakdown}`;
      return `<div class="item"><h2><a href="${esc(decisionsPageUrl(set))}">${esc(s.title)}</a></h2>
<p class="context">Raised ${esc(stamp)} · ${s.items} item${s.items === 1 ? "" : "s"} · ${s.questions} question${s.questions === 1 ? "" : "s"} · ${state}${esc(agingChip)}</p>
<p class="context">Set <code>${esc(s.setId)}</code></p></div>`;
    })
    .join("");
  const empty =
    allSets.length === 0
      ? `<p class="context">No decision sets have been raised yet.</p>`
      : `<p class="context">No sets in this view${openOnly ? " with open questions" : ""}.</p>`;
  const abandonedFold = abandonedIds.length
    ? `<details><summary>${abandonedIds.length} abandoned — idle over 7 days with open questions (bulk-dismiss via POST /api/decisions/dismiss)</summary>
<p class="context">${abandonedIds.map((id) => `<code>${esc(id)}</code>`).join(" ")}</p></details>`
    : "";
  return `<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1"><title>Decision sets — PM Stack</title>${DECISIONS_PAGE_STYLE}</head>
<body><main><h1>Decision sets</h1>
<p class="context">${inView.length} set${inView.length === 1 ? "" : "s"} · ${totalOpen} question${totalOpen === 1 ? "" : "s"} open. Newest first. Each set opens its own page — the same link its push notification carried.</p>
<p class="context">${viewTab("today", "New today")} · ${viewTab("week", "This week")} · ${viewTab("recent", "Recent (7d)")} · ${viewTab("all", "All")} &nbsp;|&nbsp; ${toggle}</p>
${visible.length ? rows : empty}
${abandonedFold}
</main></body></html>`;
}

export function renderDecisionsForbidden(): string {
  return `<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1"><title>Not available — PM Stack</title>${ANSWER_PAGE_STYLE}</head>
<body><main><h1>Decisions link not available</h1>
<p class="context">This decisions link is unknown or its token does not match. Nothing about the set is shown without the correct link.</p>
</main></body></html>`;
}

/** The PWA manifest (#3315 slice a, landed with #3676): start_url is
 * the attention view per the design; icons incl. maskable + iOS. */
export const PWA_MANIFEST = {
  name: "PM Aggregator",
  short_name: "PM Stack",
  description: "The PM-stack aggregator: needs-you queues, quotas, health \u2014 and the alert throat.",
  start_url: "/attention",
  scope: "/",
  display: "standalone",
  background_color: "#0e1117",
  theme_color: "#161b22",
  icons: [
    { src: "/icons/icon-192.png", sizes: "192x192", type: "image/png", purpose: "any" },
    { src: "/icons/icon-512.png", sizes: "512x512", type: "image/png", purpose: "any" },
    { src: "/icons/icon-maskable-512.png", sizes: "512x512", type: "image/png", purpose: "maskable" },
  ],
};
