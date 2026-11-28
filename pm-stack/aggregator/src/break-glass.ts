import { createHash, randomBytes, randomUUID, timingSafeEqual } from "node:crypto";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import type { NewAuditEvent } from "./events.ts";

/**
 * Owner break-glass tokens (#9608, design #1253 section 6).
 *
 * When Access or the IdP is down, asserted authorship must not come
 * back as a fallback. The one recovery path the design allows is
 * local: the owner, from the laptop directly, issues a single-use
 * admin stack token via the aggregator's local CLI
 * (scripts/issue-break-glass-token.ts). Physical/local access is the
 * factor, and the path is deliberately inconvenient:
 *
 * - a token lives at most one hour (BREAK_GLASS_MAX_TTL_MS);
 * - a token works exactly once — the first valid presentation
 *   consumes it;
 * - the issuance is itself an audit event, written BEFORE the token
 *   record is persisted: if the audit write fails, no token exists
 *   and issuance throws (fail closed);
 * - only the token's sha256 hash is ever persisted — the token value
 *   exists solely in the CLI's output on the laptop;
 * - unknown, expired, or already-consumed tokens fail closed.
 *
 * The store is read fresh from disk on every operation (no cache):
 * the CLI and the aggregator server are separate processes, and a
 * cached copy could resurrect a consumed token.
 */

export const BREAK_GLASS_MAX_TTL_MS = 60 * 60 * 1000;

export interface BreakGlassTokenRecord {
  id: string;
  /** sha256 hex of the token value; the value itself is never stored. */
  tokenHash: string;
  issuedAt: string;
  expiresAt: string;
  consumedAt: string | null;
}

export interface BreakGlassTokenFile {
  version: 1;
  tokens: BreakGlassTokenRecord[];
}

/** The slice of the audit stream issuance depends on (EventsStore). */
export interface BreakGlassAuditSink {
  record: (event: NewAuditEvent) => Promise<unknown>;
}

export function breakGlassIssuanceAuditEvent(input: { id: string; expiresAt: string; ttlMs: number }): NewAuditEvent {
  return {
    actor: "owner:local-cli",
    kind: "break-glass-token-issued",
    ref: `break-glass:${input.id}`,
    summary: `Break-glass admin token issued via the local CLI; single-use, expires ${input.expiresAt} (ttl ${Math.round(input.ttlMs / 1000)}s)`,
    attribution: { confidence: "authenticated", asserted: true },
    source: { identity: "pm-aggregator", link: "#9608" },
  };
}

function hashToken(token: string): string {
  return createHash("sha256").update(token, "utf8").digest("hex");
}

function tokenHashEqual(a: string, b: string): boolean {
  const left = Buffer.from(a, "hex");
  const right = Buffer.from(b, "hex");
  return left.length === right.length && timingSafeEqual(left, right);
}

function validateRecord(row: unknown, file: string): BreakGlassTokenRecord {
  if (typeof row !== "object" || row === null || Array.isArray(row)) {
    throw new Error(`invalid break-glass token record in ${file}`);
  }
  const value = row as Record<string, unknown>;
  if (
    typeof value.id !== "string" ||
    value.id === "" ||
    typeof value.tokenHash !== "string" ||
    !/^[0-9a-f]{64}$/.test(value.tokenHash) ||
    typeof value.issuedAt !== "string" ||
    Number.isNaN(Date.parse(value.issuedAt)) ||
    typeof value.expiresAt !== "string" ||
    Number.isNaN(Date.parse(value.expiresAt)) ||
    (value.consumedAt !== null && (typeof value.consumedAt !== "string" || Number.isNaN(Date.parse(value.consumedAt))))
  ) {
    throw new Error(`invalid break-glass token record in ${file}`);
  }
  return value as unknown as BreakGlassTokenRecord;
}

export type ConsumeOutcome =
  | { outcome: "consumed"; record: BreakGlassTokenRecord }
  | { outcome: "unknown" | "already-consumed" | "expired" };

export class BreakGlassTokenStore {
  private chain: Promise<void> = Promise.resolve();

  constructor(private readonly file: string) {}

  async list(): Promise<BreakGlassTokenFile> {
    return this.load();
  }

  async add(record: BreakGlassTokenRecord): Promise<void> {
    await this.serialize(async () => {
      const current = await this.load();
      current.tokens.push(record);
      await this.persist(current);
    });
  }

  /**
   * Atomically check-and-consume by token hash: the record is stamped
   * consumed and persisted inside the serialized section, so a second
   * presentation — in this process or another — can never also win.
   */
  async consumeByHash(tokenHash: string, now: Date): Promise<ConsumeOutcome> {
    return this.serialize(async () => {
      const current = await this.load();
      const record = current.tokens.find((row) => tokenHashEqual(row.tokenHash, tokenHash));
      if (!record) return { outcome: "unknown" } as const;
      if (record.consumedAt !== null) return { outcome: "already-consumed" } as const;
      if (now.getTime() >= Date.parse(record.expiresAt)) return { outcome: "expired" } as const;
      record.consumedAt = now.toISOString();
      await this.persist(current);
      return { outcome: "consumed", record: { ...record } } as const;
    });
  }

  private async serialize<T>(fn: () => Promise<T>): Promise<T> {
    let result!: T;
    const run = this.chain.then(async () => {
      result = await fn();
    });
    this.chain = run.then(
      () => {},
      () => {},
    );
    await run;
    return result;
  }

  private async load(): Promise<BreakGlassTokenFile> {
    let raw: string;
    try {
      raw = await readFile(this.file, "utf8");
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return { version: 1, tokens: [] };
      throw error;
    }
    const parsed = JSON.parse(raw) as BreakGlassTokenFile;
    if (parsed.version !== 1 || !Array.isArray(parsed.tokens)) {
      throw new Error(`invalid break-glass token file: ${this.file}`);
    }
    return { version: 1, tokens: parsed.tokens.map((row) => validateRecord(row, this.file)) };
  }

  private async persist(value: BreakGlassTokenFile): Promise<void> {
    await mkdir(dirname(this.file), { recursive: true });
    const tmp = `${this.file}.${randomUUID()}.tmp`;
    await writeFile(tmp, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 });
    await rename(tmp, this.file);
  }
}

export interface IssuedBreakGlassToken {
  id: string;
  /** The token value — shown once by the CLI, never persisted. */
  token: string;
  issuedAt: string;
  expiresAt: string;
}

/**
 * Issue a break-glass token. The audit event is recorded first; only
 * after it lands is the token record persisted and the value returned.
 * If the audit write throws, no token exists (fail closed).
 */
export async function issueBreakGlassToken(input: {
  store: BreakGlassTokenStore;
  events: BreakGlassAuditSink;
  ttlMs?: number;
  now?: Date;
}): Promise<IssuedBreakGlassToken> {
  const now = input.now ?? new Date();
  const ttlMs = input.ttlMs ?? BREAK_GLASS_MAX_TTL_MS;
  if (!Number.isSafeInteger(ttlMs) || ttlMs <= 0) {
    throw new Error("break-glass ttl must be a positive number of milliseconds");
  }
  if (ttlMs > BREAK_GLASS_MAX_TTL_MS) {
    throw new Error("break-glass ttl exceeds the one hour maximum");
  }
  const id = `bg_${randomUUID()}`;
  const token = randomBytes(32).toString("base64url");
  const issuedAt = now.toISOString();
  const expiresAt = new Date(now.getTime() + ttlMs).toISOString();
  await input.events.record(breakGlassIssuanceAuditEvent({ id, expiresAt, ttlMs }));
  await input.store.add({ id, tokenHash: hashToken(token), issuedAt, expiresAt, consumedAt: null });
  return { id, token, issuedAt, expiresAt };
}

export type ConsumeBreakGlassResult =
  | { ok: true; id: string; issuedAt: string; expiresAt: string }
  | { ok: false; error: string };

/** Present a break-glass token. Single-use; anything invalid fails closed. */
export async function consumeBreakGlassToken(input: {
  store: BreakGlassTokenStore;
  token: string;
  now?: Date;
}): Promise<ConsumeBreakGlassResult> {
  const now = input.now ?? new Date();
  if (typeof input.token !== "string" || input.token.trim() === "") {
    return { ok: false, error: "no break-glass token presented" };
  }
  const result = await input.store.consumeByHash(hashToken(input.token), now);
  switch (result.outcome) {
    case "consumed":
      return {
        ok: true,
        id: result.record.id,
        issuedAt: result.record.issuedAt,
        expiresAt: result.record.expiresAt,
      };
    case "unknown":
      return { ok: false, error: "unknown break-glass token" };
    case "already-consumed":
      return { ok: false, error: "break-glass token already consumed" };
    case "expired":
      return { ok: false, error: "break-glass token expired" };
  }
}

export function breakGlassConsumptionAuditEvent(input: { id: string; consumedAt: string }): NewAuditEvent {
  return {
    actor: `owner:break-glass:${input.id}`,
    kind: "break-glass-token-consumed",
    ref: `break-glass:${input.id}`,
    summary: `Break-glass admin token consumed; authorizes exactly one admin request (consumed ${input.consumedAt})`,
    attribution: { confidence: "authenticated", asserted: true },
    source: { identity: "pm-aggregator", link: "#9608" },
  };
}

/**
 * The break-glass token presented on a request, if any: the
 * `Authorization: Bearer <token>` convention the codebase already
 * uses (see alertsIntakeAuthorized in server.ts). Returns "" when
 * no token is presented.
 */
export function presentedBreakGlassToken(req: Request): string {
  const header = req.headers.get("authorization") ?? "";
  return header.startsWith("Bearer ") ? header.slice("Bearer ".length).trim() : "";
}

export type AuthorizeBreakGlassResult = { ok: true; id: string; auditActor: string } | { ok: false; error: string };

/**
 * Consume a presented break-glass token and record the consumption
 * as an audit event, mirroring issuance. Fail closed in both
 * directions: consumption happens first (the token is burned), so
 * if the audit write fails the request is NOT authorized and the
 * token cannot be replayed for a second attempt.
 */
export async function authorizeWithBreakGlassToken(input: {
  store: BreakGlassTokenStore;
  events: BreakGlassAuditSink;
  token: string;
  now?: Date;
}): Promise<AuthorizeBreakGlassResult> {
  const now = input.now ?? new Date();
  const consumed = await consumeBreakGlassToken({ store: input.store, token: input.token, now });
  if (!consumed.ok) return { ok: false, error: consumed.error };
  try {
    await input.events.record(breakGlassConsumptionAuditEvent({ id: consumed.id, consumedAt: now.toISOString() }));
  } catch {
    return { ok: false, error: "break-glass consumption audit write failed" };
  }
  return { ok: true, id: consumed.id, auditActor: `owner:break-glass:${consumed.id}` };
}
