/**
 * Durable per-tenant/subject recommendation page accumulator (C9139-02).
 *
 * Each instance is one tenant/subject partition. Callers must derive its DO
 * name from the authenticated partition identity. The instance binds itself
 * to the first parent envelope it accepts and rejects every later request
 * with a different tenant or subject.
 *
 * A single structured-clone storage value contains the pinned parent, total
 * order, cursor and replay pages. Page creation and cursor advancement are
 * committed together by one durable put. If the caller loses the response
 * after that put, replay reads the committed envelope instead of signing a
 * different page. The signing service must produce deterministic signatures
 * for the same domain-prefixed canonical body.
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

const STORE_KEY = "recommendation-accumulator-v1";
const PAGE_SIZE = 10;
const MAX_REPLAY_PAGES = 16;
const PAGE_DOMAIN = "developing-today/recommendation-page/v1\0";

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

interface StoredAccumulator {
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

  private async read(): Promise<StoredAccumulator | undefined> {
    return this.state.storage.get<StoredAccumulator>(STORE_KEY);
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

    const existing = await this.read();
    if (existing && (existing.tenant !== body.tenant || existing.subject !== body.subject)) {
      throw new RecommendationPartitionError();
    }

    const snapshot = await createSnapshot(parent);
    if (existing?.parent_snapshot_id === snapshot.parent_snapshot_id) {
      return {
        started: false,
        tenant: body.tenant,
        subject: body.subject,
        parent_snapshot_id: snapshot.parent_snapshot_id,
        cursor: { parent_snapshot_id: snapshot.parent_snapshot_id, position: existing.cursor_position },
      };
    }

    const next: StoredAccumulator = {
      schema_version: 1,
      tenant: body.tenant,
      subject: body.subject,
      parent,
      parent_snapshot_id: snapshot.parent_snapshot_id,
      ordered: snapshot.ordered,
      cursor_position: 0,
      pages: {},
    };
    await this.state.storage.put(STORE_KEY, next);
    return {
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
    const stored = await this.read();
    if (!stored) throw new Error("recommendation snapshot has not been started");
    if (stored.tenant !== raw.tenant || stored.subject !== raw.subject) {
      throw new RecommendationPartitionError();
    }

    const cursor =
      raw.cursor === undefined
        ? { parent_snapshot_id: stored.parent_snapshot_id, position: 0 }
        : (objectBody(raw.cursor) as unknown as PageCursor);
    if (cursor.parent_snapshot_id !== stored.parent_snapshot_id) {
      // Use the canonical state-machine error and keep parent binding in one place.
      await pageFromCursor(
        {
          parent: stored.parent,
          parent_snapshot_id: stored.parent_snapshot_id,
          ordered: stored.ordered,
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
          parent: stored.parent,
          parent_snapshot_id: stored.parent_snapshot_id,
          ordered: stored.ordered,
          page_size: PAGE_SIZE,
        },
        cursor,
        "00",
      );
    }

    const position = cursor.position;
    const cached = stored.pages[String(position)];
    if (cached) return cached;
    if (position < stored.cursor_position) throw new RecommendationReplayExpiredError(position);
    if (position > stored.cursor_position) {
      throw new RecommendationCursorAheadError(position, stored.cursor_position);
    }
    if (position % PAGE_SIZE !== 0) throw new Error(`cursor position must be a multiple of ${PAGE_SIZE}`);

    const entries = stored.ordered.slice(position, position + PAGE_SIZE);
    const pageBody: PageEnvelope["body"] = {
      parent_snapshot_id: stored.parent_snapshot_id,
      position,
      entries,
      done: position + entries.length >= stored.ordered.length,
    };
    const envelope: PageEnvelope = { body: pageBody, signature: await this.sign(pageBody) };
    const envelope_id = await pageEnvelopeId(envelope);
    const nextCursor: PageCursor = {
      parent_snapshot_id: stored.parent_snapshot_id,
      position: position + entries.length,
    };
    const page: StoredPage = { envelope, envelope_id, next_cursor: nextCursor };
    const pages = { ...stored.pages, [String(position)]: page };
    const retained = Object.keys(pages)
      .map(Number)
      .sort((a, b) => a - b)
      .slice(-MAX_REPLAY_PAGES)
      .reduce<Record<string, StoredPage>>((result, key) => {
        result[String(key)] = pages[String(key)];
        return result;
      }, {});
    await this.state.storage.put(STORE_KEY, {
      ...stored,
      cursor_position: nextCursor.position,
      pages: retained,
    } satisfies StoredAccumulator);
    return page;
  }
}
