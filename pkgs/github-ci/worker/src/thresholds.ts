/**
 * C2 per-kind queue thresholds (#9570 design record, D3) — the one
 * checked-in thresholds file, read by this Worker (derivation, count
 * signal, age cron) and by the drain runner (slice C2-S2). Tuning a
 * kind is a reviewed change to this file, never a hand edit on a
 * host.
 *
 * A per-kind queue is a cursor over the fleet-queue `events` log
 * (D2): queue K is the rows with kind = K and id greater than the
 * `drain:K` watermark in `agent_watermarks`. A kind drains when its
 * depth reaches `count` items OR its oldest item reaches
 * `ageMinutes` minutes (D4). Kinds where someone is waiting on a
 * reply (a failed default-branch run, a mention) drain on the first
 * item with a 5-minute age backstop; the rest use the directive's
 * default of 10 items or 15 minutes.
 */
export interface QueueThreshold {
  count: number;
  ageMinutes: number;
}

export const QUEUE_THRESHOLDS: Record<string, QueueThreshold> = {
  issue_new: { count: 10, ageMinutes: 15 },
  pr_open: { count: 10, ageMinutes: 15 },
  ci_failure: { count: 1, ageMinutes: 5 },
  mention: { count: 1, ageMinutes: 5 },
  pr_review: { count: 10, ageMinutes: 15 },
};

/** The queued kinds, in threshold-table order. Log-only kinds
 * (`push`, `sub_issues`, other actions) are absent on purpose: they
 * never raise a drain signal. */
export const QUEUE_KINDS: string[] = Object.keys(QUEUE_THRESHOLDS);

/**
 * Fleet identities whose naming in a comment or review body derives
 * a `mention` row (D3). Checked-in configuration: extend this list
 * when a new fleet identity should be woken by mentions. Matching is
 * case-insensitive on a non-login-character boundary (see derive.ts).
 */
export const MENTION_IDENTITIES: string[] = ["dezren39", "xaltha"];
