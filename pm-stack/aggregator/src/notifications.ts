import type { AttentionView } from "./derive.ts";
import type { HealthView } from "./health.ts";
import type { QuotaView, TierItem, TieredEntry } from "./model.ts";

export const NOTIFICATION_CLASS_DEFAULTS: Record<string, boolean> = {
  incident: true,
  "needs-drew": true,
  "quota-wall": true,
  "quota-near-wall": false,
  "stack-health": true,
  "blocked-age": true,
};

export const NOTIFICATION_COOLDOWN_MS = 6 * 60 * 60_000;
export const BLOCKED_AGE_REMINDER_MS = 24 * 60 * 60_000;
export const BLOCKED_AGE_BREAKTHROUGH_MS = 48 * 60 * 60_000;
export const QUOTA_WELL_BEFORE_RESET_MS = 24 * 60 * 60_000;

export interface NotificationCandidate {
  class: "needs-drew" | "blocked-age" | "quota-wall" | "quota-near-wall" | "stack-health";
  objectRef: string;
  stateFingerprint: string;
  title: string;
  body: string;
  url: string;
  observedAt: string;
  breakthrough: boolean;
  digestOnly?: boolean;
  escalationStep?: 24 | 48;
}

export interface NotificationHistoryEntry {
  at: string;
  class: NotificationCandidate["class"];
  objectRef: string;
  stateFingerprint: string;
  outcome: "sent" | "suppressed-quiet" | "suppressed-dedupe" | "suppressed-cooldown" | "gone" | "error";
  subscriptionId?: string;
}

export type NotificationDecision = "send" | "digest" | "suppress-dedupe" | "suppress-cooldown";

export interface NotificationSnapshot {
  attention: AttentionView;
  quota: QuotaView;
  health: Pick<HealthView, "services">;
  /** Pool ids identified as having no alternative route. */
  soleDoorPoolIds?: ReadonlySet<string>;
}

function candidate(
  value: Omit<NotificationCandidate, "stateFingerprint" | "breakthrough"> & {
    state: unknown;
    breakthrough?: boolean;
  },
): NotificationCandidate {
  const { state, breakthrough = false, ...rest } = value;
  return { ...rest, stateFingerprint: JSON.stringify(state), breakthrough };
}

function tierItemCandidate(item: TierItem | TieredEntry, now: number): NotificationCandidate[] {
  const isTierItem = "id" in item;
  const sessionId = item.sessionId;
  const itemId = isTierItem ? item.id : sessionId;
  const itemRef = isTierItem ? item.ref : undefined;
  const objectRef = sessionId ? `session:${sessionId}` : `item:${itemId}`;
  const url = sessionId
    ? `/sessions?id=${encodeURIComponent(sessionId)}`
    : itemRef && /^[-\w.]+\/[-\w.]+#\d+$/.test(itemRef)
      ? `https://github.com/${itemRef.replace("#", "/issues/")}`
      : `/attention#${encodeURIComponent(itemId ?? "unknown")}`;
  const observedAt = isTierItem ? item.since : item.tierSince ?? item.createdAt ?? item.lastActivityAt ?? new Date(now).toISOString();
  const entry = candidate({
    class: "needs-drew",
    objectRef,
    title: item.title ?? itemId ?? "Needs-Drew item",
    body: "This item entered the needs-Drew tier.",
    url,
    observedAt,
    state: { tier: "needs-drew", state: "state" in item ? item.state : undefined, kind: isTierItem ? item.kind : undefined },
  });
  const since = Date.parse(observedAt);
  if (!Number.isFinite(since) || now - since < BLOCKED_AGE_REMINDER_MS) return [entry];
  const step: 24 | 48 = now - since >= BLOCKED_AGE_BREAKTHROUGH_MS ? 48 : 24;
  return [
    entry,
    candidate({
      class: "blocked-age",
      objectRef,
      title: step === 48 ? `Blocked for 48 hours: ${item.title ?? itemId}` : `Needs attention: ${item.title ?? itemId}`,
      body: "This needs-Drew item remains unacted.",
      url,
      observedAt,
      state: { step },
      breakthrough: step === 48,
      digestOnly: step === 24,
      escalationStep: step,
    }),
  ];
}

/** Apply enter-state dedupe, the per-item cooldown, and quiet/digest policy. */
export function decideNotification(
  candidate: NotificationCandidate,
  history: NotificationHistoryEntry[],
  now: number,
  quietHours: boolean,
): NotificationDecision {
  const sent = history.filter((entry) => entry.outcome === "sent");
  if (
    sent.some(
      (entry) =>
        entry.class === candidate.class &&
        entry.objectRef === candidate.objectRef &&
        entry.stateFingerprint === candidate.stateFingerprint,
    )
  ) {
    return "suppress-dedupe";
  }
  const lastForItem = sent
    .filter((entry) => entry.class === candidate.class && entry.objectRef === candidate.objectRef)
    .map((entry) => Date.parse(entry.at))
    .filter(Number.isFinite)
    .sort((a, b) => b - a)[0];
  const escalation = candidate.escalationStep !== undefined;
  if (!escalation && lastForItem !== undefined && now - lastForItem < NOTIFICATION_COOLDOWN_MS) {
    return "suppress-cooldown";
  }
  if (candidate.digestOnly || (quietHours && !candidate.breakthrough)) return "digest";
  return "send";
}

/** Derive stable, fixture-testable notifications from the same views the UI serves. */
export function buildNotificationCandidates(snapshot: NotificationSnapshot, now = Date.now()): NotificationCandidate[] {
  const attentionItems: Array<TierItem | TieredEntry> = [
    ...snapshot.attention.drew,
    ...snapshot.attention.drewExtra,
  ];
  const candidates = attentionItems.flatMap((item) => tierItemCandidate(item, now));

  for (const pool of snapshot.quota.pools) {
    const soleDoor = snapshot.soleDoorPoolIds?.has(pool.id) ?? false;
    const windows = pool.windows.filter((w) => w.limit !== null);
    const wall = pool.state === "walled" || windows.some((w) => (w.percent ?? (w.used / (w.limit || 1)) * 100) >= 100);
    if (wall) {
      const resetAt = windows.map((w) => Date.parse(w.resetsAt ?? "")).filter(Number.isFinite).sort((a, b) => a - b)[0];
      const wellBeforeReset = resetAt !== undefined && resetAt - now >= QUOTA_WELL_BEFORE_RESET_MS;
      if (soleDoor || wellBeforeReset) {
        candidates.push(
          candidate({
            class: "quota-wall",
            objectRef: `quota:${pool.id}`,
            title: `Quota wall: ${pool.provider} ${pool.plan}`,
            body: `Pool ${pool.id} is walled${soleDoor ? " and has no alternative route" : " well before reset"}.`,
            url: "/quota",
            observedAt: new Date(now).toISOString(),
            state: { state: "walled", resetAt: Number.isFinite(resetAt) ? new Date(resetAt).toISOString() : null },
            breakthrough: soleDoor,
          }),
        );
      }
    } else if (soleDoor && windows.some((w) => (w.percent ?? 0) >= 85 && (w.percent ?? 0) < 100)) {
      candidates.push(
        candidate({
          class: "quota-near-wall",
          objectRef: `quota:${pool.id}`,
          title: `Quota near wall: ${pool.provider} ${pool.plan}`,
          body: `Sole-door pool ${pool.id} has crossed 85%.`,
          url: "/quota",
          observedAt: new Date(now).toISOString(),
          state: { state: "near", windows: windows.map((w) => [w.name, Math.floor(w.percent ?? 0)]) },
        }),
      );
    }
  }

  for (const service of snapshot.health.services) {
    if (service.ok !== false) continue;
    candidates.push(
      candidate({
        class: "stack-health",
        objectRef: `health:${service.def.id}`,
        title: `Stack health: ${service.def.name}`,
        body: "The monitored service is failing.",
        url: "/health",
        observedAt: service.probedAt ?? new Date(now).toISOString(),
        state: { state: service.state },
      }),
    );
  }
  return candidates;
}
