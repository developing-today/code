// Local-only observations of the four conditional UPSERTs. This is not a
// billing meter: successful statements are a subset of all billed activity.
const labels = [
  "activity.ingestion",
  "activity.gaps",
  "actions",
  "comments",
] as const;
type WriteLabel = (typeof labels)[number];

export interface TursoWriteContext {
  operation: WriteLabel;
  candidates: number;
}

function counter() {
  return { knownBatches: 0, missingBatches: 0, rows: 0 };
}
function bucket() {
  return {
    candidates: 0,
    affected: counter(),
    skipped: 0,
    read: counter(),
    written: counter(),
  };
}
function boundedAdd(current: number, increment: number): number {
  return Math.min(Number.MAX_SAFE_INTEGER, current + increment);
}
function rowCount(
  value: unknown,
  maximum = Number.MAX_SAFE_INTEGER,
): number | undefined {
  return typeof value === "number" &&
    Number.isSafeInteger(value) &&
    value >= 0 &&
    value <= maximum
    ? value
    : undefined;
}
function addCounter(
  target: ReturnType<typeof counter>,
  value: number | undefined,
): void {
  if (value === undefined)
    target.missingBatches = boundedAdd(target.missingBatches, 1);
  else {
    target.knownBatches = boundedAdd(target.knownBatches, 1);
    target.rows = boundedAdd(target.rows, value);
  }
}

export class TursoWriteMetrics {
  #buckets = {
    "activity.ingestion": bucket(),
    "activity.gaps": bucket(),
    actions: bucket(),
    comments: bucket(),
  } satisfies Record<WriteLabel, ReturnType<typeof bucket>>;

  observe(
    context: TursoWriteContext,
    affected: unknown,
    read: unknown,
    written: unknown,
  ): void {
    if (!labels.includes(context.operation)) return;
    const candidates = rowCount(context.candidates);
    if (candidates === undefined || candidates === 0) return;
    const target = this.#buckets[context.operation];
    const affectedRows = rowCount(affected, candidates);
    target.candidates = boundedAdd(target.candidates, candidates);
    addCounter(target.affected, affectedRows);
    addCounter(target.read, rowCount(read));
    addCounter(target.written, rowCount(written));
    if (affectedRows !== undefined)
      target.skipped = boundedAdd(target.skipped, candidates - affectedRows);
  }

  // Cumulative since this meter was created. Fixed-size, detached, and
  // available only to internal callers who explicitly install a meter.
  snapshot() {
    return structuredClone(this.#buckets);
  }
}
