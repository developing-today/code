// Read-only intent matching for the voice line (#9800).
//
// Step 1 ships exactly ONE intent end to end — queue state — per the
// issue's acceptance. Intents are read-only by design: the voice line
// never takes a write action. Matching is deliberately small and
// deterministic (no model in the loop): a transcript either names the
// queue-state intent or falls back to a spoken explanation of what the
// line can do, which still exercises both speech directions.

export type IntentName = "queue_state" | "unknown";

export interface QueueSnapshot {
  queued: number;
  claimed: number;
  parked: number;
  blocked: number;
  /** ISO timestamp of the snapshot. */
  at: string;
}

const QUEUE_PATTERNS: RegExp[] = [
  /\bqueue\b/,
  /\btasks?\b.*\b(waiting|queued|left|open)\b/,
  /\bhow many\b.*\btasks?\b/,
  /\bbacklog\b/,
  /\bfleet\b.*\b(state|status)\b/,
];

export function matchIntent(transcript: string): IntentName {
  const text = transcript.toLowerCase();
  return QUEUE_PATTERNS.some((re) => re.test(text)) ? "queue_state" : "unknown";
}

export function buildAnswer(intent: IntentName, snapshot: QueueSnapshot | null): string {
  if (intent === "queue_state") {
    if (!snapshot) {
      return "I don't have a fresh queue snapshot right now. Ask again in a moment and I should know the queue state.";
    }
    const parts = [
      `The fleet queue has ${snapshot.queued} tasks waiting`,
      `${snapshot.claimed} being worked`,
      `and ${snapshot.parked} parked`,
    ];
    if (snapshot.blocked > 0) parts.push(`with ${snapshot.blocked} blocked`);
    return parts.join(", ") + ".";
  }
  return "Right now I can only report the fleet queue state. Try asking: how many tasks are in the queue?";
}
