// POST /ingest-status body validation (#11014). The stored snapshot is
// read back and spoken by the queue_state intent, so a bad push must be
// refused with a 400 rather than stored as null, negative or
// fractional counts.

export interface QueueCounts {
  queued: number;
  claimed: number;
  parked: number;
  blocked: number;
}

export type IngestResult =
  | { ok: true; snapshot: QueueCounts }
  | { ok: false; error: "invalid_json" | "expected_object" | "invalid_count"; field?: keyof QueueCounts };

const FIELDS: (keyof QueueCounts)[] = ["queued", "claimed", "parked", "blocked"];

/** Parse and validate a raw ingest body. Missing counts default to 0;
 * present counts must be non-negative safe integers. */
export function parseIngestStatus(raw: string): IngestResult {
  let body: unknown;
  try {
    body = JSON.parse(raw);
  } catch {
    return { ok: false, error: "invalid_json" };
  }
  if (typeof body !== "object" || body === null || Array.isArray(body)) {
    return { ok: false, error: "expected_object" };
  }
  const record = body as Record<string, unknown>;
  const snapshot = {} as QueueCounts;
  for (const field of FIELDS) {
    const value = record[field] ?? 0;
    if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) {
      return { ok: false, error: "invalid_count", field };
    }
    snapshot[field] = value;
  }
  return { ok: true, snapshot };
}
