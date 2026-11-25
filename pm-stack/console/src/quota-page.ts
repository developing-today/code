// C6 quota page (#15018, parent #9574): one phone-first quota/pool page
// beside the existing questions module, with an explicit human-confirmed
// reset action. This module is pure logic + rendering; the edge wires it
// behind an opt-in flag and injects the existing pool-steward dispatch
// and the existing decision-set store — it creates neither a second
// question store nor a new reset actuator, and never resets unattended.
export const QUOTA_PAGE_DATA_VERSION = "quota-page-data/v1";
export const QUOTA_PAGE_ID = "quota";
export const RESET_PROVIDERS = ["openai", "anthropic", "fable"] as const;
export type ResetProvider = (typeof RESET_PROVIDERS)[number];

export interface QuotaPool {
  id: string; provider: string; label: string;
  used: number; limit: number; unit: string;
  window: string; renewalAt: string;
  projectedWallAt: string | null; wallUntil: string | null; projectionBasis: string;
}
export interface QuotaPageEnvelope {
  version: typeof QUOTA_PAGE_DATA_VERSION; pageId: typeof QUOTA_PAGE_ID;
  revision: number; asOf: string; freshnessBudgetMs: number; pools: QuotaPool[];
}
/** A question-set as the existing decision machinery stores it (read-only here). */
export interface ExistingQuestionSet { setId: string; title?: string; items?: { ledgerId: string; title?: string; questions?: { qid: string; question: string; resolution?: unknown }[] }[] }

const isNum = (v: unknown): v is number => typeof v === "number" && Number.isFinite(v) && v >= 0;
const isStr = (v: unknown): v is string => typeof v === "string" && v.length > 0;

export function validateQuotaEnvelope(raw: unknown): QuotaPageEnvelope {
  const e = raw as Partial<QuotaPageEnvelope> | null;
  if (!e || typeof e !== "object") throw new Error("envelope: expected an object");
  if (e.version !== QUOTA_PAGE_DATA_VERSION) throw new Error(`envelope: version must be ${QUOTA_PAGE_DATA_VERSION}`);
  if (e.pageId !== QUOTA_PAGE_ID) throw new Error("envelope: pageId must be quota");
  if (!isNum(e.revision) || !Number.isInteger(e.revision)) throw new Error("envelope: revision must be a non-negative integer");
  if (!isStr(e.asOf) || Number.isNaN(Date.parse(e.asOf))) throw new Error("envelope: asOf must be an ISO time");
  if (!isNum(e.freshnessBudgetMs) || e.freshnessBudgetMs === 0) throw new Error("envelope: freshnessBudgetMs must be > 0");
  if (!Array.isArray(e.pools) || e.pools.length === 0) throw new Error("envelope: pools must be a non-empty array");
  for (const p of e.pools) {
    if (!isStr(p?.id) || !isStr(p.provider) || !isStr(p.label) || !isNum(p.used) || !isNum(p.limit) || p.limit === 0 || !isStr(p.unit) || !isStr(p.window) || !isStr(p.renewalAt) || Number.isNaN(Date.parse(p.renewalAt)) || !isStr(p.projectionBasis)) throw new Error(`envelope: pool ${String((p as QuotaPool)?.id ?? "?")} is incomplete`);
    if (p.projectedWallAt !== null && (!isStr(p.projectedWallAt) || Number.isNaN(Date.parse(p.projectedWallAt)))) throw new Error("envelope: projectedWallAt must be ISO or null");
  }
  return e as QuotaPageEnvelope;
}

export const freshness = (env: QuotaPageEnvelope, now: number) => {
  const ageMs = Math.max(0, now - Date.parse(env.asOf));
  return { ageMs, stale: ageMs > env.freshnessBudgetMs, budgetExceeded: ageMs > env.freshnessBudgetMs };
};

/** Embeds existing question sets by reference — no second store. */
export function embedQuestions(sets: ExistingQuestionSet[]) {
  const open: { set: string; ledger: string; qid: string; question: string }[] = [];
  for (const s of sets ?? []) for (const item of s.items ?? []) for (const q of item.questions ?? [])
    if (!q.resolution) open.push({ set: s.setId, ledger: item.ledgerId, qid: q.qid, question: q.question });
  return { total: open.length, sets: new Set(open.map((q) => q.set)).size, open };
}

export interface QuotaException { key: string; at: string; reason: string }
/** Holds the last good envelope; a failed refresh never replaces it. */
export class QuotaPageStore {
  good: QuotaPageEnvelope | null = null; lastError: string | null = null;
  exceptions: QuotaException[] = []; private routed = new Set<string>();
  constructor(private routeException: (e: QuotaException) => void = () => {}) {}
  applySuccess(raw: unknown, now = Date.now()): QuotaPageEnvelope {
    const env = validateQuotaEnvelope(raw);
    if (this.good && env.revision < this.good.revision) throw new Error("envelope: revision went backwards; keeping last good");
    this.good = env; this.lastError = null; this.checkBudget(now); return env;
  }
  applyFailure(error: string, now = Date.now()): QuotaPageEnvelope | null {
    this.lastError = error.slice(0, 300); this.checkBudget(now); return this.good;
  }
  private checkBudget(now: number) {
    if (!this.good || !freshness(this.good, now).budgetExceeded) return;
    const key = `quota-stale:${this.good.pageId}:${this.good.revision}`;
    if (this.routed.has(key)) return; // deduplicated by page+revision
    this.routed.add(key);
    const e = { key, at: new Date(now).toISOString(), reason: `quota page stale beyond ${this.good.freshnessBudgetMs}ms budget (last error: ${this.lastError ?? "none"})` };
    this.exceptions.push(e); this.routeException(e);
  }
}

export const resetEligible = (pool: QuotaPool): { eligible: boolean; reason: string } =>
  !(RESET_PROVIDERS as readonly string[]).includes(pool.provider)
    ? { eligible: false, reason: `provider ${pool.provider} has no reset action` }
    : pool.used < pool.limit
      ? { eligible: false, reason: "quota has not reached 100%" }
      : { eligible: true, reason: "quota at 100% for an eligible provider" };

export interface ResetProposal { id: string; poolId: string; provider: string; envelopeRevision: number; proposedBy: string; proposedAt: string; idempotencyKey: string }
export interface ResetRecord extends ResetProposal { status: "proposed" | "dispatched" | "failed"; confirmedBy?: string; confirmedAt?: string; queueTaskId?: string; result?: string }
export interface ResetDispatch { (proposal: ResetProposal): Promise<{ queueTaskId: string; result: string }> }

/** Confirmed reset: audit-before-dispatch, idempotent, stale-version rejection. */
export class ResetFlow {
  records: ResetRecord[] = []; audit: { at: string; phase: string; id: string }[] = [];
  constructor(private dispatch: ResetDispatch) {}
  propose(env: QuotaPageEnvelope, poolId: string, by: string, idempotencyKey: string, now = Date.now()): ResetRecord {
    const existing = this.records.find((r) => r.idempotencyKey === idempotencyKey);
    if (existing) return existing;
    const pool = env.pools.find((p) => p.id === poolId);
    if (!pool) throw new Error("pool: not on this page");
    const elig = resetEligible(pool);
    if (!elig.eligible) throw new Error(`reset: not eligible — ${elig.reason}`);
    const rec: ResetRecord = { id: `reset-${idempotencyKey}`, poolId, provider: pool.provider, envelopeRevision: env.revision, proposedBy: by, proposedAt: new Date(now).toISOString(), idempotencyKey, status: "proposed" };
    this.records.push(rec); return rec;
  }
  async confirm(env: QuotaPageEnvelope, id: string, actor: string, now = Date.now()): Promise<ResetRecord> {
    const rec = this.records.find((r) => r.id === id);
    if (!rec) throw new Error("reset: no such proposal");
    if (rec.status !== "proposed") return rec; // idempotent: never dispatch twice
    if (rec.envelopeRevision !== env.revision) throw new Error("reset: stale action version — fresh confirmation required");
    rec.confirmedBy = actor; rec.confirmedAt = new Date(now).toISOString();
    this.audit.push({ at: rec.confirmedAt, phase: "audit-before-dispatch", id: rec.id }); // audit BEFORE dispatch
    try {
      const out = await this.dispatch(rec);
      rec.queueTaskId = out.queueTaskId; rec.result = out.result; rec.status = "dispatched"; // result write-back
    } catch (e) { rec.status = "failed"; rec.result = (e as Error).message.slice(0, 300); }
    return rec;
  }
}

const esc = (s: unknown) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
const pct = (p: QuotaPool) => Math.min(100, Math.round((p.used / p.limit) * 100));

/** Phone-first render at 320px: no fixed widths, no page-level horizontal scroll. */
export function renderQuotaPage(env: QuotaPageEnvelope, opts: { now?: number; questions?: ReturnType<typeof embedQuestions>; resets?: ResetRecord[] } = {}): string {
  const now = opts.now ?? Date.now(); const f = freshness(env, now);
  const pools = env.pools.map((p) => { const elig = resetEligible(p); return `<section aria-label="${esc(p.label)}"><h2>${esc(p.label)} (${esc(p.provider)})</h2>
<p>Quota: ${esc(p.used)} / ${esc(p.limit)} ${esc(p.unit)} (${pct(p)}%) · Window: ${esc(p.window)} · Renews: ${esc(p.renewalAt)}</p>
<p>Projected wall: ${esc(p.projectedWallAt ?? "not projected")} · Wall-until: ${esc(p.wallUntil ?? "unknown")} · Basis: ${esc(p.projectionBasis)}</p>
${elig.eligible ? `<form method="post" action="/console/api/quota/reset"><input type="hidden" name="pool" value="${esc(p.id)}"><label><input type="checkbox" required name="confirm"> I confirm this reset</label> <button type="submit">Reset ${esc(p.provider)}</button></form>` : `<p>Reset: not eligible — ${esc(elig.reason)}</p>`}</section>`; }).join("\n");
  const qs = opts.questions ? `<section aria-label="Questions"><h2>Questions (${opts.questions.total})</h2>${opts.questions.open.map((q) => `<p data-set="${esc(q.set)}" data-qid="${esc(q.qid)}">${esc(q.question)}</p>`).join("") || "<p>No open questions.</p>"}</section>` : "";
  const rs = (opts.resets ?? []).map((r) => `<p>Reset ${esc(r.poolId)}: ${esc(r.status)}${r.queueTaskId ? ` · queue/task ${esc(r.queueTaskId)}` : ""}${r.confirmedBy ? ` · confirmed by ${esc(r.confirmedBy)} at ${esc(r.confirmedAt)}` : ""}${r.result ? ` · ${esc(r.result)}` : ""}</p>`).join("");
  return `<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Quota</title>
<style>*{box-sizing:border-box}body{font:18px/1.5 system-ui,sans-serif;margin:0;padding:1rem;max-width:100%;overflow-x:hidden;overflow-wrap:anywhere}section{margin:0 0 1.25rem}button{min-height:44px}</style>
<main><h1>Quota</h1><p>As of ${esc(env.asOf)} · Revision ${env.revision} · ${f.stale ? `STALE — showing last-good data (age ${Math.round(f.ageMs / 1000)}s, budget ${Math.round(env.freshnessBudgetMs / 1000)}s)` : "Fresh"}</p>
${pools}${qs}${rs}</main>`;
}
