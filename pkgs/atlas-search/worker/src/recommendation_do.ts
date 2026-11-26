/**
 * Durable per-tenant/subject recommendation page accumulator (C9139-02).
 *
 * Each instance is one tenant/subject partition. Callers must derive its DO
 * name from the authenticated partition identity. The instance binds itself
 * to the first parent envelope it accepts and rejects every later request
 * with a different tenant or subject.
 *
 * Storage layout (schema v2, #15003): the accumulator state is split across
 * per-part keys so a page advance rewrites only the new page and the cursor
 * instead of the whole accumulator:
 *
 *   recommendation-accumulator-v2:meta           tenant/subject + snapshot id
 *   recommendation-accumulator-v2:parent         pinned signed parent envelope
 *   recommendation-accumulator-v2:order          total order for the snapshot
 *   recommendation-accumulator-v2:cursor         { cursor_position, retained_pages }
 *   recommendation-accumulator-v2:page:<position>  one retained replay page
 *
 * The parent and order are written once by `start()` and never rewritten for
 * the life of the snapshot. Page creation and cursor advancement are
 * committed together by one atomic multi-key put (Durable Object storage
 * applies a multi-key put as a single transaction) carrying only the new
 * page value and the small cursor value: O(page) bytes per advance,
 * independent of the candidate-set size. If the caller loses the response
 * after that put, replay reads the committed page instead of signing a
 * different page. The signing service must produce deterministic signatures
 * for the same domain-prefixed canonical body.
 *
 * Schema v1 stored everything under the single key
 * `recommendation-accumulator-v1`. A v1 value is migrated on first read:
 * read once, fan out to the v2 keys, then delete the v1 key. The fan-out
 * put lands before the delete, so a crash mid-migration simply re-runs it
 * and the v2 keys win once present.
 */
import {
  createSnapshot,
  deterministicCbor,
  hexToBytes,
  pageEnvelopeId,
  pageFromCursor,
  type PageCursor,
  type PageEntry,
  type PageEnvelope,
  type ParentEnvelope,
} from "./recommendation_state";
import { recordRecommendation } from "./recommendation_history";
import { recordHotSnapshot } from "./hot_snapshot";

const LEGACY_STORE_KEY = "recommendation-accumulator-v1";
const META_KEY = "recommendation-accumulator-v2:meta";
const PARENT_KEY = "recommendation-accumulator-v2:parent";
const ORDER_KEY = "recommendation-accumulator-v2:order";
const CURSOR_KEY = "recommendation-accumulator-v2:cursor";
const PAGE_KEY_PREFIX = "recommendation-accumulator-v2:page:";
const PAGE_SIZE = 10;
const MAX_REPLAY_PAGES = 16;
const PAGE_DOMAIN = "developing-today/recommendation-page/v1\0";

function pageKey(position: number): string {
  return `${PAGE_KEY_PREFIX}${position}`;
}

interface RecommendationParentBody {
  tenant: string;
  subject: string;
  page_size: number;
  candidate_set: Array<{ id: string; score_micros: number; arrival_sequence: number }>;
  [key: string]: unknown;
}

interface RecommendationParent extends ParentEnvelope {
  body: ParentEnvelope["body"] & RecommendationParentBody;
}

interface StoredPage {
  envelope: PageEnvelope;
  envelope_id: string;
  next_cursor: PageCursor;
}

interface StoredMeta {
  schema_version: 2;
  tenant: string;
  subject: string;
  parent_snapshot_id: string;
}

interface StoredCursor {
  cursor_position: number;
  retained_pages: number[];
}

/** Schema v1 single-key value, retained only to migrate live instances. */
interface LegacyStoredAccumulator {
  schema_version: 1;
  tenant: string;
  subject: string;
  parent: RecommendationParent;
  parent_snapshot_id: string;
  ordered: PageEntry[];
  cursor_position: number;
  pages: Record<string, StoredPage>;
}

interface DurableStorageLike {
  get<T = unknown>(key: string): Promise<T | undefined>;
  put<T = unknown>(key: string, value: T): Promise<void>;
  put(entries: Record<string, unknown>): Promise<void>;
  delete(keys: string | string[]): Promise<unknown>;
}

interface DurableStateLike {
  storage: DurableStorageLike;
}

/** Cloudflare service binding that signs the canonical page-body preimage. */
interface PageSignerBinding {
  fetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response>;
}

interface RecommendationEnv {
  PAGE_SIGNER: PageSignerBinding;
  /**
   * Where recomputes are retained as `recommendation` versions and the
   * hot window as `hot_snapshot` versions (#9140).
   * Optional: without it, or while the class is disabled, nothing is kept.
   */
  RETAINED?: D1Database;
}

export class RecommendationPartitionError extends Error {
  readonly code = "RecommendationPartitionMismatch";
  constructor() {
    super("request tenant/subject does not match this accumulator partition");
    this.name = "RecommendationPartitionError";
  }
}

export class RecommendationReplayExpiredError extends Error {
  readonly code = "RecommendationReplayExpired";
  constructor(position: number) {
    super(`page replay at position ${position} is outside the retained horizon`);
    this.name = "RecommendationReplayExpiredError";
  }
}

export class RecommendationCursorAheadError extends Error {
  readonly code = "RecommendationCursorAhead";
  constructor(position: number, committed: number) {
    super(`cursor position ${position} is ahead of committed position ${committed}`);
    this.name = "RecommendationCursorAheadError";
  }
}

function objectBody(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error("body must be a JSON object");
  }
  return value as Record<string, unknown>;
}

function hex(bytes: Uint8Array): string {
  return [...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

function pageSigningPreimage(body: PageEnvelope["body"]): Uint8Array {
  const domain = new TextEncoder().encode(PAGE_DOMAIN);
  const canonical = deterministicCbor(body as unknown as Parameters<typeof deterministicCbor>[0]);
  const preimage = new Uint8Array(domain.length + canonical.length);
  preimage.set(domain);
  preimage.set(canonical, domain.length);
  return preimage;
}

function requireIdentity(tenant: unknown, subject: unknown): asserts tenant is string {
  if (typeof tenant !== "string" || tenant.length === 0 || typeof subject !== "string" || subject.length === 0) {
    throw new Error("tenant and subject are required");
  }
}

export class RecommendationAccumulator {
  private readonly state: DurableStateLike;
  private readonly env: RecommendationEnv;

  constructor(state: DurableObjectState | DurableStateLike, env: RecommendationEnv) {
    this.state = state as DurableStateLike;
    this.env = env;
  }

  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    try {
      if (request.method === "POST" && url.pathname === "/start") {
        const body = objectBody(await request.json());
        return Response.json(await this.start(body.parent));
      }
      if (request.method === "POST" && url.pathname === "/next") {
        const body = objectBody(await request.json());
        const page = await this.next(body);
        return Response.json({
          ...page,
          envelope: {
            body: page.envelope.body,
            signature_hex: hex(page.envelope.signature),
          },
        });
      }
      return Response.json({ error: { code: "not_found", message: "no route" } }, { status: 404 });
    } catch (error) {
      const code = (error as { code?: string }).code ?? "recommendation_failed";
      const status =
        code === "RecommendationPartitionMismatch"
          ? 403
          : code === "CursorExpired" || code === "RecommendationReplayExpired"
            ? 410
            : code === "recommendation_failed"
              ? 500
              : 400;
      return Response.json({ error: { code, message: (error as Error).message } }, { status });
    }
  }

  /**
   * Reads the v2 meta record, migrating a legacy v1 single-key value on
   * first touch: fan the combined value out to the per-part keys in one
   * put, then delete the legacy key so the combined value is never
   * written again.
   */
  private async readMeta(): Promise<StoredMeta | undefined> {
    const meta = await this.state.storage.get<StoredMeta>(META_KEY);
    if (meta) return meta;
    const legacy = await this.state.storage.get<LegacyStoredAccumulator>(LEGACY_STORE_KEY);
    if (!legacy) return undefined;
    const retained = Object.keys(legacy.pages)
      .map(Number)
      .sort((a, b) => a - b);
    const migrated: StoredMeta = {
      schema_version: 2,
      tenant: legacy.tenant,
      subject: legacy.subject,
      parent_snapshot_id: legacy.parent_snapshot_id,
    };
    const entries: Record<string, unknown> = {
      [META_KEY]: migrated,
      [PARENT_KEY]: legacy.parent,
      [ORDER_KEY]: legacy.ordered,
      [CURSOR_KEY]: {
        cursor_position: legacy.cursor_position,
        retained_pages: retained,
      } satisfies StoredCursor,
    };
    for (const position of retained) {
      entries[pageKey(position)] = legacy.pages[String(position)];
    }
    await this.state.storage.put(entries);
    await this.state.storage.delete(LEGACY_STORE_KEY);
    return migrated;
  }

  private async readCursor(): Promise<StoredCursor> {
    const cursor = await this.state.storage.get<StoredCursor>(CURSOR_KEY);
    return cursor ?? { cursor_position: 0, retained_pages: [] };
  }

  private async readParent(): Promise<RecommendationParent> {
    const parent = await this.state.storage.get<RecommendationParent>(PARENT_KEY);
    if (!parent) throw new Error("recommendation snapshot has not been started");
    return parent;
  }

  private async readOrder(): Promise<PageEntry[]> {
    const ordered = await this.state.storage.get<PageEntry[]>(ORDER_KEY);
    if (!ordered) throw new Error("recommendation snapshot has not been started");
    return ordered;
  }

  private async start(rawParent: unknown): Promise<Record<string, unknown>> {
    const wireParent = objectBody(rawParent);
    const bodyValue = objectBody(wireParent.body);
    const signatureHex = wireParent.signature_hex;
    const rawSignature = wireParent.signature;
    let signature: Uint8Array;
    if (typeof signatureHex === "string" && /^(?:[0-9a-f]{2})+$/.test(signatureHex)) {
      signature = hexToBytes(signatureHex);
    } else if (
      Array.isArray(rawSignature) &&
      rawSignature.every((value) => Number.isInteger(value) && value >= 0 && value <= 255)
    ) {
      signature = new Uint8Array(rawSignature as number[]);
    } else if (typeof rawSignature === "object" && rawSignature !== null && !Array.isArray(rawSignature)) {
      const indexed = Object.entries(rawSignature as Record<string, unknown>).sort(([a], [b]) => Number(a) - Number(b));
      if (
        indexed.every(
          ([key, value], index) =>
            key === String(index) && Number.isInteger(value) && Number(value) >= 0 && Number(value) <= 255,
        )
      ) {
        signature = new Uint8Array(indexed.map(([, value]) => Number(value)));
      } else {
        throw new Error("parent signature must be byte data or signature_hex");
      }
    } else {
      throw new Error("a signed parent envelope is required");
    }
    if (Object.keys(bodyValue).length === 0) {
      throw new Error("a signed parent envelope is required");
    }
    const parent = { body: bodyValue, signature } as unknown as RecommendationParent;
    const body = parent.body as RecommendationParentBody;
    requireIdentity(body.tenant, body.subject);
    if (body.page_size !== PAGE_SIZE) throw new Error(`page_size must be ${PAGE_SIZE}`);
    if (!Array.isArray(body.candidate_set)) throw new Error("candidate_set must be an array");

    const existing = await this.readMeta();
    if (existing && (existing.tenant !== body.tenant || existing.subject !== body.subject)) {
      throw new RecommendationPartitionError();
    }

    const snapshot = await createSnapshot(parent);
    if (existing?.parent_snapshot_id === snapshot.parent_snapshot_id) {
      const cursor = await this.readCursor();
      return {
        started: false,
        tenant: body.tenant,
        subject: body.subject,
        parent_snapshot_id: snapshot.parent_snapshot_id,
        cursor: { parent_snapshot_id: snapshot.parent_snapshot_id, position: cursor.cursor_position },
      };
    }

    // Replacing a previous snapshot: its retained page keys are stale and
    // are deleted after the new snapshot's keys land.
    const stalePageKeys = existing ? (await this.readCursor()).retained_pages.map(pageKey) : [];
    await this.state.storage.put({
      [META_KEY]: {
        schema_version: 2,
        tenant: body.tenant,
        subject: body.subject,
        parent_snapshot_id: snapshot.parent_snapshot_id,
      } satisfies StoredMeta,
      [PARENT_KEY]: parent,
      [ORDER_KEY]: snapshot.ordered,
      [CURSOR_KEY]: { cursor_position: 0, retained_pages: [] } satisfies StoredCursor,
    });
    if (stalePageKeys.length > 0) await this.state.storage.delete(stalePageKeys);
    // The snapshot is committed; keeping its history is best effort and
    // never fails the start. A retried start of the same parent returns
    // above, so one recompute is recorded once.
    let retained: number | undefined;
    try {
      const outcome = await recordRecommendation(this.env.RETAINED, snapshot, body.tenant, body.subject);
      if (outcome.recorded) retained = outcome.version;
    } catch (error) {
      console.warn(`recommendation history not recorded: ${(error as Error).message}`);
    }
    // The same cadence captures the hot window under the short class.
    let hot: number | undefined;
    try {
      const outcome = await recordHotSnapshot(this.env.RETAINED, snapshot, body.tenant, body.subject);
      if (outcome.recorded) hot = outcome.version;
    } catch (error) {
      console.warn(`hot snapshot not recorded: ${(error as Error).message}`);
    }
    return {
      ...(retained === undefined ? {} : { retained_version: retained }),
      ...(hot === undefined ? {} : { hot_version: hot }),
      started: true,
      tenant: body.tenant,
      subject: body.subject,
      parent_snapshot_id: snapshot.parent_snapshot_id,
      cursor: { parent_snapshot_id: snapshot.parent_snapshot_id, position: 0 },
    };
  }

  private async sign(body: PageEnvelope["body"]): Promise<Uint8Array> {
    const message = pageSigningPreimage(body);
    const response = await this.env.PAGE_SIGNER.fetch("https://page-signer/v1/sign", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ domain: PAGE_DOMAIN.replace(/\0$/, ""), message_hex: hex(message) }),
    });
    if (!response.ok) throw new Error(`page signing failed with status ${response.status}`);
    const result = objectBody(await response.json());
    if (typeof result.signature_hex !== "string" || !/^(?:[0-9a-f]{2})+$/.test(result.signature_hex)) {
      throw new Error("page signer returned an invalid signature_hex");
    }
    return hexToBytes(result.signature_hex);
  }

  private async next(raw: Record<string, unknown>): Promise<StoredPage> {
    requireIdentity(raw.tenant, raw.subject);
    const meta = await this.readMeta();
    if (!meta) throw new Error("recommendation snapshot has not been started");
    if (meta.tenant !== raw.tenant || meta.subject !== raw.subject) {
      throw new RecommendationPartitionError();
    }
    const cursorState = await this.readCursor();

    const cursor =
      raw.cursor === undefined
        ? { parent_snapshot_id: meta.parent_snapshot_id, position: 0 }
        : (objectBody(raw.cursor) as unknown as PageCursor);
    if (cursor.parent_snapshot_id !== meta.parent_snapshot_id) {
      // Use the canonical state-machine error and keep parent binding in one place.
      await pageFromCursor(
        {
          parent: await this.readParent(),
          parent_snapshot_id: meta.parent_snapshot_id,
          ordered: await this.readOrder(),
          page_size: PAGE_SIZE,
        },
        cursor,
        "00",
      );
      throw new Error("unreachable wrong-parent cursor path");
    }
    if (!Number.isInteger(cursor.position) || cursor.position < 0) {
      throw new Error("cursor position must be a non-negative integer");
    }
    if (cursor.expires_at !== undefined && Date.now() >= cursor.expires_at) {
      await pageFromCursor(
        {
          parent: await this.readParent(),
          parent_snapshot_id: meta.parent_snapshot_id,
          ordered: await this.readOrder(),
          page_size: PAGE_SIZE,
        },
        cursor,
        "00",
      );
    }

    const position = cursor.position;
    if (cursorState.retained_pages.includes(position)) {
      const cached = await this.state.storage.get<StoredPage>(pageKey(position));
      if (cached) return cached;
      throw new RecommendationReplayExpiredError(position);
    }
    if (position < cursorState.cursor_position) throw new RecommendationReplayExpiredError(position);
    if (position > cursorState.cursor_position) {
      throw new RecommendationCursorAheadError(position, cursorState.cursor_position);
    }
    if (position % PAGE_SIZE !== 0) throw new Error(`cursor position must be a multiple of ${PAGE_SIZE}`);

    const ordered = await this.readOrder();
    const entries = ordered.slice(position, position + PAGE_SIZE);
    const pageBody: PageEnvelope["body"] = {
      parent_snapshot_id: meta.parent_snapshot_id,
      position,
      entries,
      done: position + entries.length >= ordered.length,
    };
    const envelope: PageEnvelope = { body: pageBody, signature: await this.sign(pageBody) };
    const envelope_id = await pageEnvelopeId(envelope);
    const nextCursor: PageCursor = {
      parent_snapshot_id: meta.parent_snapshot_id,
      position: position + entries.length,
    };
    const page: StoredPage = { envelope, envelope_id, next_cursor: nextCursor };
    const retainedAll = [...cursorState.retained_pages, position].sort((a, b) => a - b);
    const retained = retainedAll.slice(-MAX_REPLAY_PAGES);
    const pruned = retainedAll.slice(0, retainedAll.length - retained.length);
    // The only write on the advance path: the new page plus the small
    // cursor record, committed atomically by one multi-key put. The
    // parent envelope and total order are not carried along (#15003).
    await this.state.storage.put({
      [pageKey(position)]: page,
      [CURSOR_KEY]: {
        cursor_position: nextCursor.position,
        retained_pages: retained,
      } satisfies StoredCursor,
    });
    if (pruned.length > 0) await this.state.storage.delete(pruned.map(pageKey));
    return page;
  }
}
