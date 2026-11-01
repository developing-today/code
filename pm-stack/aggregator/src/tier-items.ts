/**
 * Needs-user item contract (#1290): per-item sanitization, fallbacks, and
 * boundary checks for the tier-item extension fields (whyUser, summary,
 * ifIgnored, options, questions, delegation).
 *
 * Fail-soft by design (the #949/#1110 lesson): the section gate in
 * feed.ts never learns these fields, and nothing here can reject an item
 * or a section. A malformed extension drops — the whole field for
 * `delegation`, which is validated as a unit; the offending entry for
 * `options`/`questions` — and the item renders with the pre-#1290
 * controls.
 *
 * Naming is role-based (2026-10-27 decision): `whyUser`, mode `user`.
 * The legacy published tier array name (`needsDrew`) is the existing
 * shape and stays untouched.
 */
import type { DashAction, NewAction } from "./actions.ts";
import type {
  DelegationMode,
  OrchestrationSection,
  TierItem,
  TierItemDelegation,
  TierItemOption,
  TierItemQuestion,
  WhyUserClass,
} from "./model.ts";

export const WHY_USER_CLASSES: readonly WhyUserClass[] = ["decision", "missing-info", "approval", "access", "policy"];

/** Classes the standing auto toggle may cover. `access` and `policy`
 *  NEVER auto (2026-10-27 decision) — enforced here, at the POST
 *  boundary, and in the publisher's self-check. */
export const AUTO_ELIGIBLE_CLASSES: readonly WhyUserClass[] = ["decision", "missing-info", "approval"];

export function isAutoEligibleClass(why: WhyUserClass | undefined): boolean {
  return why !== undefined && AUTO_ELIGIBLE_CLASSES.includes(why);
}

/** The standing delegation-default preference (#1290/#1292): Drew's
 *  2026-10-27 decision (auto ON for decision/missing-info/approval) is
 *  the initial record; dashboard `preference` actions supersede it. */
export const DELEGATION_DEFAULT_OBJECT_REF = "needs-user-delegation-default";
export const INITIAL_STANDING_AUTO = true;

/** A first-pass proposal older than this renders expired (#1293) and the
 *  publisher's self-check retires it on the next run. */
export const PROPOSAL_TTL_MS = 48 * 3_600_000;

const SLUG_RE = /^[a-z0-9][a-z0-9-]{0,39}$/;
const MAX_OPTIONS = 6;
const MAX_QUESTIONS = 5;

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function bounded(v: unknown, max: number): string | undefined {
  if (typeof v !== "string") return undefined;
  const t = v.trim();
  return t.length >= 1 && t.length <= max ? t : undefined;
}

function optionalBounded(v: unknown, max: number): string | undefined {
  if (v === undefined) return undefined;
  return bounded(v, max);
}

function isIso(v: unknown): v is string {
  return typeof v === "string" && v.includes("T") && !Number.isNaN(Date.parse(v));
}

function sanitizeOptions(raw: unknown): TierItemOption[] | undefined {
  if (!Array.isArray(raw)) return undefined;
  const out: TierItemOption[] = [];
  const seen = new Set<string>();
  for (const entry of raw) {
    if (out.length >= MAX_OPTIONS) break;
    if (!isRecord(entry)) continue;
    const id = entry.id;
    if (typeof id !== "string" || !SLUG_RE.test(id) || seen.has(id)) continue;
    const label = bounded(entry.label, 80);
    if (!label) continue;
    seen.add(id);
    const questionId =
      typeof entry.questionId === "string" && SLUG_RE.test(entry.questionId) ? entry.questionId : undefined;
    const opt: TierItemOption = { id, label, ...(questionId ? { questionId } : {}) };
    const description = optionalBounded(entry.description, 300);
    if (description) opt.description = description;
    const effect = optionalBounded(entry.effect, 300);
    if (effect) opt.effect = effect;
    out.push(opt);
  }
  return out.length ? out : undefined;
}

function sanitizeQuestions(raw: unknown): TierItemQuestion[] | undefined {
  if (!Array.isArray(raw)) return undefined;
  const out: TierItemQuestion[] = [];
  const seen = new Set<string>();
  for (const entry of raw) {
    if (out.length >= MAX_QUESTIONS) break;
    if (!isRecord(entry)) continue;
    const id = entry.id;
    if (typeof id !== "string" || !SLUG_RE.test(id) || seen.has(id)) continue;
    const prompt = bounded(entry.prompt, 200);
    const covers = bounded(entry.covers, 200);
    const type = entry.type;
    if (!prompt || !covers) continue;
    if (type !== "single" && type !== "multi" && type !== "text" && type !== "yesno") continue;
    const q: TierItemQuestion = {
      id,
      prompt,
      type,
      required: typeof entry.required === "boolean" ? entry.required : true,
      covers,
    };
    if (type === "single" || type === "multi") {
      if (!Array.isArray(entry.options)) continue;
      const opts: { id: string; label: string }[] = [];
      const oseen = new Set<string>();
      for (const o of entry.options) {
        if (!isRecord(o)) continue;
        if (typeof o.id !== "string" || !SLUG_RE.test(o.id) || oseen.has(o.id)) continue;
        const label = bounded(o.label, 80);
        if (!label) continue;
        oseen.add(o.id);
        opts.push({ id: o.id, label });
      }
      if (opts.length < 2) continue;
      q.options = opts;
    }
    seen.add(id);
    out.push(q);
  }
  return out.length ? out : undefined;
}

/** Resolve a shortcut option to its canonical survey question. Explicit
 *  publisher mapping wins; otherwise match the option id among question
 *  choices, or use the sole question when the item has exactly one. */
export function questionIdForOption(item: TierItem | undefined, optionId: string): string | undefined {
  const option = item?.options?.find((candidate) => candidate.id === optionId);
  if (!option || !item) return undefined;
  const questions = item.questions ?? [];
  if (option.questionId && questions.some((question) => question.id === option.questionId)) return option.questionId;
  const matches = questions.filter((question) => question.options?.some((choice) => choice.id === optionId));
  if (matches.length === 1) return matches[0]?.id;
  return questions.length === 1 ? questions[0]?.id : undefined;
}

function sanitizeDelegation(raw: unknown, whyUser: WhyUserClass | undefined): TierItemDelegation | undefined {
  if (!isRecord(raw)) return undefined;
  let mode: DelegationMode | undefined;
  if (raw.mode === "user" || raw.mode === "drew") mode = "user";
  else if (raw.mode === "first-pass" || raw.mode === "auto") mode = raw.mode;
  if (!mode) return undefined;
  // Access/policy items never sit in auto, whatever the publisher sent.
  if (mode === "auto" && !isAutoEligibleClass(whyUser)) return undefined;
  const status = raw.status;
  if (
    status !== "idle" &&
    status !== "working" &&
    status !== "proposal-ready" &&
    status !== "unable" &&
    status !== "done"
  )
    return undefined;
  if (!isIso(raw.updatedAt)) return undefined;
  const d: TierItemDelegation = { mode, status, updatedAt: raw.updatedAt };
  const runId = optionalBounded(raw.runId, 80);
  if (runId) d.runId = runId;

  const hasProposal = raw.proposal !== undefined;
  const hasUnable = raw.unable !== undefined;
  // Payloads are present exactly when their status says so.
  if (hasProposal !== (status === "proposal-ready")) return undefined;
  if (hasUnable !== (status === "unable")) return undefined;

  if (hasProposal) {
    const p = raw.proposal;
    if (!isRecord(p)) return undefined;
    const summary = bounded(p.summary, 500);
    if (!summary || !isIso(p.at)) return undefined;
    d.proposal = { summary, at: p.at };
    const detail = optionalBounded(p.detail, 2000);
    if (detail) d.proposal.detail = detail;
    if (Array.isArray(p.proposedChanges)) {
      const changes = p.proposedChanges
        .map((c) => bounded(c, 300))
        .filter((c): c is string => c !== undefined)
        .slice(0, 10);
      if (changes.length) d.proposal.proposedChanges = changes;
    }
  }
  if (hasUnable) {
    const u = raw.unable;
    if (!isRecord(u)) return undefined;
    const reason = bounded(u.reason, 500);
    const whatItNeeds = bounded(u.whatItNeeds, 500);
    if (!reason || !whatItNeeds || !isIso(u.at)) return undefined;
    if (!Array.isArray(u.whatItTried)) return undefined;
    const tried = u.whatItTried
      .map((t) => bounded(t, 300))
      .filter((t): t is string => t !== undefined)
      .slice(0, 10);
    if (!tried.length) return undefined;
    d.unable = { reason, whatItTried: tried, whatItNeeds, at: u.at };
  }
  return d;
}

/**
 * Validate one tier item's extension fields. Returns a shallow copy with
 * only well-formed extensions kept; the base item is never touched and
 * never rejected.
 */
export function sanitizeTierItemExtensions(item: TierItem): TierItem {
  const raw = item as unknown as Record<string, unknown>;
  const out: TierItem = { ...item };
  delete out.whyUser;
  delete out.summary;
  delete out.ifIgnored;
  delete out.options;
  delete out.questions;
  delete out.delegation;

  if (typeof raw.whyUser === "string" && (WHY_USER_CLASSES as readonly string[]).includes(raw.whyUser))
    out.whyUser = raw.whyUser as WhyUserClass;
  const summary = optionalBounded(raw.summary, 500);
  if (summary) out.summary = summary;
  const ifIgnored = optionalBounded(raw.ifIgnored, 500);
  if (ifIgnored) out.ifIgnored = ifIgnored;
  const options = sanitizeOptions(raw.options);
  if (options) out.options = options;
  const questions = sanitizeQuestions(raw.questions);
  if (questions) out.questions = questions;
  const delegation = sanitizeDelegation(raw.delegation, out.whyUser);
  if (delegation) out.delegation = delegation;
  return out;
}

/**
 * Needs-user fallbacks (#1290): when the publisher omits whyUser /
 * ifIgnored / delegation, the aggregator derives them so every card —
 * and every JSON twin — carries the full contract. A published
 * delegation always wins over the standing-toggle default.
 */
export function withUserFallbacks(item: TierItem, sectionGeneratedAt: string, standingAuto: boolean): TierItem {
  const out: TierItem = { ...item };
  if (!out.whyUser) {
    out.whyUser = item.kind !== "decision" && item.question ? "missing-info" : "decision";
  }
  if (!out.ifIgnored) {
    const sinceDate = /^\d{4}-\d{2}-\d{2}/.test(item.since) ? item.since.slice(0, 10) : "an earlier run";
    out.ifIgnored =
      "Stays in your queue and is re-published every run until it " +
      `resolves or expires — in your queue since ${sinceDate}.`;
  }
  if (!out.delegation) {
    out.delegation = {
      mode: standingAuto && isAutoEligibleClass(out.whyUser) ? "auto" : "user",
      status: "idle",
      updatedAt: sectionGeneratedAt,
    };
  }
  return out;
}

/** The effective standing auto default from the preference records:
 *  the latest non-superseded preference action (pending or applied)
 *  wins; before any exists, Drew's 2026-10-27 decision (ON) applies. */
export function effectiveStandingAuto(records: DashAction[]): {
  standingAuto: boolean;
  pending: boolean;
} {
  const prefs = records
    .filter(
      (r) =>
        r.kind === "preference" &&
        r.verb === "set-delegation-default" &&
        (r.state === "new" || r.state === "applied") &&
        typeof r.payload?.standingAuto === "boolean",
    )
    .sort((a, b) => b.createdAt.localeCompare(a.createdAt));
  const latest = prefs[0];
  if (!latest) return { standingAuto: INITIAL_STANDING_AUTO, pending: false };
  return {
    standingAuto: latest.payload?.standingAuto === true,
    pending: latest.state === "new",
  };
}

/** The Needs-you card order preference (#1456): persisted as a
 *  `preference` action (verb set-needs-order) on this object ref. */
export const NEEDS_ORDER_OBJECT_REF = "needs-user-order";
export type NeedsOrder = "urgency" | "newest" | "oldest" | "deadline";
export const NEEDS_ORDERS: readonly NeedsOrder[] = ["urgency", "newest", "oldest", "deadline"];

/** The effective Needs-you card order from the preference records:
 *  the latest non-superseded record (pending or applied) wins; before
 *  any exists, the orchestrator's published order ("urgency") applies.
 *  Mirrors effectiveStandingAuto. */
export function effectiveNeedsOrder(records: DashAction[]): { order: NeedsOrder; pending: boolean } {
  const prefs = records
    .filter(
      (r) =>
        r.kind === "preference" &&
        r.verb === "set-needs-order" &&
        (r.state === "new" || r.state === "applied") &&
        typeof r.payload?.needsOrder === "string" &&
        (NEEDS_ORDERS as readonly string[]).includes(r.payload.needsOrder),
    )
    .sort((a, b) => b.createdAt.localeCompare(a.createdAt));
  const latest = prefs[0];
  if (!latest) return { order: "urgency", pending: false };
  return { order: latest.payload!.needsOrder as NeedsOrder, pending: latest.state === "new" };
}

/** #1293: a proposal-ready delegation past its TTL is expired — it
 *  renders as expired and the item is decidable again. The clock is the
 *  proposal's own draft time (`proposal.at`, #1623): `updatedAt` moves on
 *  any delegation write, so clocking from it would let churn resurrect an
 *  old proposal indefinitely (and read a fresh proposal as expired when
 *  the delegation record itself is old). */
export function proposalExpired(d: TierItemDelegation | undefined, nowMs: number): boolean {
  if (!d || d.status !== "proposal-ready") return false;
  const at = Date.parse(d.proposal?.at ?? "");
  return !Number.isNaN(at) && nowMs - at > PROPOSAL_TTL_MS;
}

/** The live proposal's identity triple (#1293), or null when the item
 *  carries no live proposal. */
export function proposalIdentity(item: TierItem): { itemId: string; runId: string; updatedAt: string } | null {
  const d = item.delegation;
  if (!d || d.status !== "proposal-ready" || !d.proposal || !d.runId) return null;
  return { itemId: item.id, runId: d.runId, updatedAt: d.updatedAt };
}

/** Resolve a dashboard objectRef (`orch:<itemId>` or `gh:<owner/repo#N>`)
 *  to the tier item it names in the currently published section. */
export function findTierItem(section: OrchestrationSection, objectRef: string): TierItem | undefined {
  const items = [...section.tiers.needsDrew, ...section.tiers.needsOrchestrator];
  if (objectRef.startsWith("orch:")) return items.find((i) => i.id === objectRef.slice(5));
  if (objectRef.startsWith("gh:")) return items.find((i) => i.ref === objectRef.slice(3));
  return undefined;
}

/**
 * POST-boundary checks that need the currently published item (#1291 /
 * #1293): option and survey membership, required-question coverage,
 * proposal identity, and the access/policy auto ban. Returns a legible
 * error message, or null when the action is acceptable. `item` is the
 * item as currently published (undefined when it has left the tier).
 */
export function checkActionAgainstItem(input: NewAction, item: TierItem | undefined, nowMs: number): string | null {
  const p = input.payload ?? {};
  if (input.verb === "answer") {
    if (p.answers && p.answers.length) {
      if (!item) return "this item is no longer on the published Needs User tier — refresh and try again";
      const qs = item.questions ?? [];
      if (!qs.length) return "this item has no survey questions to answer";
      const byId = new Map(qs.map((q) => [q.id, q]));
      for (const a of p.answers) {
        const q = byId.get(a.questionId);
        if (!q) return `unknown question id "${a.questionId}" for this item`;
        if ((q.type === "single" || q.type === "yesno") && q.options) {
          if (typeof a.value !== "string" || !q.options.some((o) => o.id === a.value))
            return `answer for "${q.id}" must be one of its options`;
        }
        if (q.type === "multi" && q.options) {
          if (
            !Array.isArray(a.value) ||
            a.value.some((v) => typeof v !== "string" || !q.options?.some((o) => o.id === v))
          )
            return `answers for "${q.id}" must be chosen from its options`;
        }
      }
      // #1451 (Drew, 2026-10-27): partial survey submits are allowed —
      // the card merges answers across submits and shows which required
      // questions remain open. Membership and value checks above still
      // reject anything the item never asked. (Before #1451 this point
      // rejected a submit missing required answers outright.)
    } else if (p.optionId) {
      if (!item) return "this item is no longer on the published Needs User tier — refresh and try again";
      if (!(item.options ?? []).some((o) => o.id === p.optionId))
        return `option "${p.optionId}" is not one of this item's published options`;
    } else if (p.text) {
      const required = (item?.questions ?? []).filter((q) => q.required).map((q) => q.id);
      if (required.length)
        return `this item asks survey questions — the required ones (${required.join(", ")}) must be answered`;
    }
  }
  if (input.verb === "approve-proposal" || input.verb === "reject-proposal") {
    if (!p.proposalRunId || !p.proposalUpdatedAt)
      return "proposal identity (proposalRunId and proposalUpdatedAt) is required for this action";
    if (!item) return "this item is no longer on the published Needs User tier — the proposal is no longer applicable";
    const d = item.delegation;
    if (!d || d.status !== "proposal-ready" || !d.proposal)
      return "there is no live proposal on this item — it may have been superseded or withdrawn";
    if (proposalExpired(d, nowMs))
      return "this proposal has expired — a fresh first pass will be drafted on a later run";
    if (d.runId !== p.proposalRunId || d.updatedAt !== p.proposalUpdatedAt)
      return "proposal superseded/withdrawn — this is no longer the live proposal; refresh to review the current one";
  }
  if (input.verb === "delegate" && p.mode === "auto" && item) {
    if (item.whyUser === "access" || item.whyUser === "policy")
      return `${item.whyUser} items can never be delegated to auto — they stay with the user`;
  }
  return null;
}
