import type { Database } from "bun:sqlite";
import { createHmac, timingSafeEqual } from "node:crypto";
import {
  buildTriagePacket,
  runMetadataCheck,
  storeOutcome,
  type EpicSlugMap,
  type IngestionItem,
  type NativeParentLookup,
} from "./metadata-ingestion.ts";

const MAX_WEBHOOK_BODY_BYTES = 100_000;
const WEBHOOK_INGESTOR = "metadata-webhook-listener";
const ACCEPTED_ACTIONS = new Set(["opened", "edited", "closed", "labeled"]);

type JsonRecord = Record<string, unknown>;

function record(value: unknown): JsonRecord | null {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? (value as JsonRecord) : null;
}

function stringValue(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

function readLabels(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((entry) => {
    if (typeof entry === "string") return [entry];
    const label = record(entry);
    return typeof label?.name === "string" ? [label.name] : [];
  });
}

async function readRawBody(request: Request): Promise<Uint8Array> {
  const declaredLength = request.headers.get("content-length");
  if (declaredLength !== null && Number(declaredLength) > MAX_WEBHOOK_BODY_BYTES) {
    await request.body?.cancel().catch(() => {});
    throw new RangeError("body too large");
  }

  const reader = request.body?.getReader();
  const chunks: Uint8Array[] = [];
  let byteLength = 0;
  if (reader) {
    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        byteLength += value.byteLength;
        if (byteLength > MAX_WEBHOOK_BODY_BYTES) {
          await reader.cancel().catch(() => {});
          throw new RangeError("body too large");
        }
        chunks.push(value);
      }
    } finally {
      reader.releaseLock();
    }
  }

  const bytes = new Uint8Array(byteLength);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return bytes;
}

function hasValidSignature(rawBody: Uint8Array, secret: string, supplied: string | null): boolean {
  if (!supplied || !/^sha256=[0-9a-f]{64}$/i.test(supplied)) return false;
  const expected = createHmac("sha256", secret).update(rawBody).digest();
  const actual = Buffer.from(supplied.slice("sha256=".length), "hex");
  return actual.length === expected.length && timingSafeEqual(actual, expected);
}

function toIngestionItem(payload: JsonRecord, event: string): IngestionItem | null {
  if (event !== "issues" && event !== "pull_request") return null;
  const repository = record(payload.repository);
  const repo = stringValue(repository?.full_name);
  const rawItem = record(event === "issues" ? payload.issue : payload.pull_request);
  if (!rawItem) return null;
  const number = rawItem.number;
  const title = stringValue(rawItem.title);
  const createdAt = stringValue(rawItem.created_at);
  const updatedAt = stringValue(rawItem.updated_at);
  if (
    !repo ||
    !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repo) ||
    !Number.isSafeInteger(number) ||
    (number as number) < 1 ||
    title === null ||
    createdAt === null ||
    updatedAt === null
  ) {
    return null;
  }

  const user = record(rawItem.user);
  const milestone = record(rawItem.milestone);
  const mergedAt = stringValue(rawItem.merged_at);
  const closedAt = stringValue(rawItem.closed_at) ?? mergedAt;
  const isMerged = event === "pull_request" && (rawItem.merged === true || mergedAt !== null);
  const state = isMerged ? "MERGED" : (stringValue(rawItem.state)?.toUpperCase() ?? "OPEN");
  // The issue webhook schema carries no `parent` field, so this is null
  // for issues in practice; the handler resolves native parentage
  // authoritatively via resolveNativeParent before finalizing (#16011).
  const parent = record(rawItem.parent);
  const parentRepo = stringValue(record(parent?.repository)?.full_name) ?? stringValue(parent?.repository_full_name);
  const parentNumber = parent?.number;
  const nativeParent =
    parentRepo && Number.isSafeInteger(parentNumber) && (parentNumber as number) > 0
      ? { repo: parentRepo, number: parentNumber as number, kind: "sub-issue" }
      : null;

  return {
    repo,
    number: number as number,
    kind: event === "issues" ? "issue" : "pr",
    title,
    body: stringValue(rawItem.body),
    labels: readLabels(rawItem.labels),
    milestoneTitle: stringValue(milestone?.title),
    milestoneDueOn: stringValue(milestone?.due_on),
    state,
    author: stringValue(user?.login) ?? "unknown",
    createdAt,
    updatedAt,
    closedAt,
    nativeParent,
  };
}

export interface MetadataWebhookDependencies {
  secret: () => Promise<string | null>;
  db: Database;
  epicSlugs: (item: IngestionItem) => Promise<EpicSlugMap>;
  /**
   * Authoritative native-parent lookup for `issues` events (#16011).
   * The issue webhook schema carries no `parent` field, so the payload
   * alone proves nothing about sub-issue parentage: the listener
   * resolves it here before the shared check finalizes an ascertained
   * outcome. `undefined` is a retryable failure — the handler answers
   * non-2xx with no store or body writes so GitHub redelivers.
   */
  resolveNativeParent: (item: { repo: string; number: number }) => Promise<NativeParentLookup>;
  writeBack: (item: IngestionItem, body: string) => Promise<void>;
  enqueueTriage: (packet: Record<string, unknown>) => Promise<void>;
  /**
   * Re-read the live item before a body write-back (issue #16012). The
   * snapshot above was captured before the awaited epic lookup, so a
   * concurrent user edit would otherwise be overwritten by stale content.
   * Return null when the item no longer exists (the write is skipped);
   * throw when the re-read itself fails (the delivery retries).
   */
  readCurrent: (item: IngestionItem) => Promise<IngestionItem | null>;
  now?: () => string;
}

export function createMetadataWebhookHandler(
  dependencies: MetadataWebhookDependencies,
): (request: Request) => Promise<Response> {
  return async (request) => {
    if (request.method !== "POST") return Response.json({ error: "method not allowed" }, { status: 405 });

    let rawBody: Uint8Array;
    try {
      rawBody = await readRawBody(request);
    } catch (error) {
      if (error instanceof RangeError) return Response.json({ error: "payload too large" }, { status: 413 });
      return Response.json({ error: "could not read payload" }, { status: 400 });
    }

    let secret: string | null;
    try {
      secret = await dependencies.secret();
    } catch {
      secret = null;
    }
    if (!secret) return Response.json({ error: "webhook secret unavailable" }, { status: 503 });
    if (!hasValidSignature(rawBody, secret, request.headers.get("x-hub-signature-256"))) {
      return Response.json({ error: "invalid signature" }, { status: 401 });
    }

    let payload: JsonRecord;
    try {
      const parsed: unknown = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(rawBody));
      const object = record(parsed);
      if (!object) return Response.json({ error: "invalid payload" }, { status: 400 });
      payload = object;
    } catch {
      return Response.json({ error: "invalid payload" }, { status: 400 });
    }

    const event = request.headers.get("x-github-event") ?? "";
    const action = stringValue(payload.action) ?? "";
    if ((event !== "issues" && event !== "pull_request") || !ACCEPTED_ACTIONS.has(action)) {
      return Response.json({ accepted: true, ignored: true }, { status: 202 });
    }
    const item = toIngestionItem(payload, event);
    if (!item) return Response.json({ error: "invalid issue or pull request payload" }, { status: 400 });

    const now = (dependencies.now ?? (() => new Date().toISOString()))();
    // #16011: native parentage is authoritative and the issue payload
    // carries no parent field, so resolve it before the shared check
    // finalizes anything. PRs skip this — sub-issue parents are an issue
    // concept, so their label path stands (same rule as the sweeper).
    let checked = item;
    if (item.kind === "issue" && !item.nativeParent) {
      let parent: NativeParentLookup;
      try {
        parent = await dependencies.resolveNativeParent(item);
      } catch {
        parent = undefined;
      }
      if (parent === undefined) {
        // Retryable: no store, enqueue, or body writes — GitHub redelivers
        // non-2xx webhook deliveries.
        return Response.json({ error: "native parent lookup failed" }, { status: 500 });
      }
      if (parent) checked = { ...item, nativeParent: parent };
    }
    try {
      const epicSlugMap = await dependencies.epicSlugs(checked);
      const outcome = runMetadataCheck(checked, {
        epicSlugs: epicSlugMap,
        ingestedBy: WEBHOOK_INGESTOR,
        now,
      });
      if (outcome.kind === "needs-triage") {
        await dependencies.enqueueTriage(buildTriagePacket(outcome.evidence, WEBHOOK_INGESTOR, now));
        return Response.json({ accepted: true, outcome: outcome.kind }, { status: 202 });
      }
      storeOutcome(dependencies.db, outcome);
      if (outcome.kind === "generated") {
        // Stale-snapshot protection (issue #16012): the check ran on the
        // event payload captured before the epic lookup. Re-read the live
        // item and compare body and version; when either moved, recompute
        // from current data so a concurrent user edit is preserved instead
        // of overwritten. A throw above (re-read failure) returns 500
        // without writing, so GitHub retries the delivery.
        const current = await dependencies.readCurrent(checked);
        if (current === null) {
          // The item is gone (or unreadable as gone): the stored row
          // stands, and there is nothing to write back to.
          return Response.json({ accepted: true, outcome: outcome.kind }, { status: 202 });
        }
        if (current.body !== checked.body || current.updatedAt !== checked.updatedAt) {
          // Preserve the authoritative native parent resolved above
          // (#16011): the live re-read carries payload parentage only.
          const freshItem = { ...current, nativeParent: current.nativeParent ?? checked.nativeParent };
          const fresh = runMetadataCheck(freshItem, {
            epicSlugs: epicSlugMap,
            ingestedBy: WEBHOOK_INGESTOR,
            now,
          });
          if (fresh.kind === "needs-triage") {
            await dependencies.enqueueTriage(buildTriagePacket(fresh.evidence, WEBHOOK_INGESTOR, now));
          } else {
            storeOutcome(dependencies.db, fresh);
            if (fresh.kind === "generated") await dependencies.writeBack(current, fresh.body);
          }
          return Response.json({ accepted: true, outcome: fresh.kind }, { status: 202 });
        }
        await dependencies.writeBack(checked, outcome.body);
      }
      return Response.json({ accepted: true, outcome: outcome.kind }, { status: 202 });
    } catch {
      // GitHub retries non-2xx deliveries. Upserts and triage dedupe keys
      // make a retry safe if a side effect completed before the failure.
      return Response.json({ error: "metadata ingestion failed" }, { status: 500 });
    }
  };
}
