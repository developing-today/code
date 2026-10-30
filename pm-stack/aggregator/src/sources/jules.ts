// Jules sessions from the keeper's state file (#898 §5). jules-review
// (the keeper cron) tracks the Jules sessions it supervises in
// jules-review-state.json: an `items` map keyed by the Jules session id
// with { state, title, source, created, last_action, last_action_at,
// note }, plus free-text run notes (jules_note_<HHMM>) carrying the
// run totals. This module maps that shape to SessionFeedEntry rows
// with source "jules" — pure functions, no I/O, so the collector owns
// the file read and tests can feed it literals.
import type { JulesSummary, SessionFeedEntry } from "../model.ts";

export interface JulesItem {
  state?: string;
  title?: string;
  /** "sources/github/<owner>/<repo>" */
  source?: string;
  created?: string;
  last_action?: string;
  last_action_at?: string;
  note?: string;
}

export interface JulesReviewState {
  items?: Record<string, JulesItem>;
  last_run?: string;
  [key: string]: unknown;
}

function repoOf(item: JulesItem): string | undefined {
  const m = /^sources\/github\/(.+)$/.exec(item.source ?? "");
  return m ? m[1] : item.source;
}

/** The question a Jules session is waiting on, when its note quotes one. */
function askedQuestion(note: string | undefined): string | null {
  if (!note) return null;
  const m = /asked ['"]([^'"]{4,200})['"]/i.exec(note) ?? /['"]([^'"]{4,200}\?)['"]/.exec(note);
  return m ? m[1] : null;
}

/** Map the keeper's tracked Jules sessions to feed entries. */
export function mapJulesItems(state: JulesReviewState): SessionFeedEntry[] {
  const out: SessionFeedEntry[] = [];
  for (const [id, item] of Object.entries(state.items ?? {})) {
    const entry: SessionFeedEntry = {
      sessionId: id,
      source: "jules",
      title: item.title,
      directory: repoOf(item),
      createdAt: item.created,
      lastActivityAt: item.last_action_at ?? item.created ?? null,
      activeToolCalls: 0,
      supervisionNote: `Jules ${item.state ?? "UNKNOWN"}${item.last_action ? ` · keeper: ${item.last_action}` : ""}`,
    };
    switch ((item.state ?? "").toUpperCase()) {
      case "IN_PROGRESS":
        entry.activeToolCalls = 1;
        break;
      case "AWAITING_USER_FEEDBACK":
        entry.awaitingQuestion =
          askedQuestion(item.note) ?? "Awaiting feedback in Jules (the keeper usually replies automatically)";
        break;
      case "AWAITING_PLAN_APPROVAL":
        entry.awaitingQuestion = "Plan awaiting approval in Jules";
        break;
      case "FAILED":
        entry.lastTurnError = "Jules run failed";
        entry.outcome = "failed";
        break;
      case "COMPLETED":
        entry.outcome = "succeeded";
        break;
    }
    out.push(entry);
  }
  return out;
}

const TOTALS =
  /(\d+) sessions - (\d+) COMPLETED, (\d+) FAILED, (\d+) AWAITING_USER_FEEDBACK, (\d+) AWAITING_PLAN_APPROVAL, (\d+) IN_PROGRESS/;

/** Keeper totals from the freshest jules_note_<HHMM> run note, when it parses. */
export function julesSummary(state: JulesReviewState): JulesSummary {
  const summary: JulesSummary = {
    at: state.last_run ?? null,
    tracked: Object.keys(state.items ?? {}).length,
  };
  let bestKey = "";
  let bestText = "";
  for (const [k, v] of Object.entries(state)) {
    if (/^jules_note_\d+$/.test(k) && typeof v === "string" && k > bestKey) {
      bestKey = k;
      bestText = v;
    }
  }
  const m = TOTALS.exec(bestText);
  if (m) {
    summary.total = Number(m[1]);
    summary.completed = Number(m[2]);
    summary.failed = Number(m[3]);
    summary.awaitingFeedback = Number(m[4]);
    summary.awaitingPlan = Number(m[5]);
    summary.inProgress = Number(m[6]);
  }
  return summary;
}
