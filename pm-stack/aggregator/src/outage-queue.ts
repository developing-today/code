import { randomUUID } from "node:crypto";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import type { NewAction } from "./actions.ts";
import { validateAuthor } from "./comments.ts";
import type { NewAuditEvent } from "./events.ts";
import type { DashAuthor } from "./model.ts";

/**
 * Outage queueing for dashboard action writes (#9608, design #1253
 * section 6 — the break-glass posture).
 *
 * When Access or the IdP is down, the stack must not choose between
 * "closed" and "asserted authorship returns". Reads keep working from
 * last-known state (the stack is local-first; nothing here touches
 * the read path). For writes, this module is the queue half of the
 * posture:
 *
 * - A dashboard action write that arrives while identity verification
 *   is failing (the server routes only the enforcer's 401 here — a
 *   403 means a verified identity contradicted the claim, which is
 *   not an outage) may be QUEUED instead of refused, but only from a
 *   stack-local address (see isStackLocalAddress) and only when the
 *   write is eligible (see outageQueueEligible).
 * - A queued write is stamped `provenance: "outage-queued"` with the
 *   claimed author kept visibly a claim. It lives ONLY in this store:
 *   it never enters the actions store, so consumer polls
 *   (GET /api/actions.json?state=new) and every verified-authorship
 *   surface never see it. Nothing outage-queued is ever silently
 *   promoted.
 * - The entry recovering is not enough: the claimant confirms with a
 *   fresh login. Confirmation requires the Access-derived author of
 *   that fresh login to match the claim (kind + id); the write is
 *   then released into the actions store under the VERIFIED author —
 *   the re-stamp — or rejected by its claimant. A verified owner may
 *   also reject a claim that is not theirs: rejection can never
 *   create authorship, and without it a forged claim could never be
 *   cleared from the owner's own queue. Confirmation stays
 *   claimant-only, strictly.
 * - Asserted authorship as a fallback mode does not exist: a write
 *   that cannot queue under the rules above gets the enforcer's
 *   refusal, exactly as before this module existed.
 *
 * Queueing, confirmation, and rejection are each written as an audit
 * event BEFORE the queue record changes state (the break-glass
 * issuance pattern): if the audit write fails, nothing changes.
 *
 * The store is read fresh from disk on every operation (no cache),
 * mirroring BreakGlassTokenStore: the file is the record, and a
 * cached copy could resurrect a resolved claim.
 */

export const OUTAGE_QUEUED_PROVENANCE = "outage-queued" as const;

export type OutageQueueState = "queued" | "confirmed" | "rejected";

export interface OutageQueuedWrite {
  id: string;
  queuedAt: string;
  /** The design section 6 stamp; the write is a claim until confirmed. */
  provenance: typeof OUTAGE_QUEUED_PROVENANCE;
  /** The author the write CLAIMED at queue time. Never trusted. */
  claimedAuthor: DashAuthor;
  /** The validated write, exactly as validateNewAction produced it
   *  (its author is the claim). Released only under a verified author. */
  action: NewAction;
  state: OutageQueueState;
  /** Set when state leaves "queued". */
  resolvedAt?: string;
  /** The actions-store id a confirmation released the write as. */
  resolvedActionId?: string;
}

export interface OutageQueueFile {
  version: 1;
  writes: OutageQueuedWrite[];
}

/** The slice of the audit stream this module depends on (EventsStore). */
export interface OutageQueueAuditSink {
  record: (event: NewAuditEvent) => Promise<unknown>;
}

/**
 * Whether a validated action write may be outage-queued. Eligibility
 * is narrow on purpose:
 *
 * - The claimed author kind must be one a fresh Access login can
 *   derive ("drew" for the allowlisted owner, "agent" for any other
 *   verified identity — see deriveAuthorFromAccess). A "muse" claim
 *   has no Access derivation, so it could never be confirmed; it
 *   fails closed instead of stranding in the queue forever.
 * - The four preference view verbs (save-view, rename-view,
 *   delete-view, set-default-view) are applied by the aggregator
 *   itself inside the POST request; a queue cannot honour that
 *   applied-in-request contract, so they fail closed during an
 *   outage. Every other action kind/verb is consumer-applied and
 *   queues.
 *
 * Comments are never routed here at all: the comment surface is
 * gated on Drew's answer to design question 4 on #1253.
 */
const AGGREGATOR_APPLIED_VIEW_VERBS: ReadonlySet<string> = new Set([
  "save-view",
  "rename-view",
  "delete-view",
  "set-default-view",
]);

export function outageQueueEligible(action: NewAction): boolean {
  if (action.author.kind !== "drew" && action.author.kind !== "agent") return false;
  if (action.kind === "preference" && AGGREGATOR_APPLIED_VIEW_VERBS.has(action.verb)) return false;
  return true;
}

/**
 * Whether an address is on the stack-local network: loopback, the
 * tailnet CGNAT range (100.64.0.0/10), RFC1918 private ranges, or
 * IPv6 loopback / unique-local / link-local. The aggregator binds
 * loopback + the tailnet address only, so every request it serves
 * already arrived over one of those; this check is the fail-closed
 * guard that keeps outage queueing from ever engaging if the server
 * is bound somewhere wider. Unknown or missing addresses are NOT
 * local.
 */
export function isStackLocalAddress(address: string | null | undefined): boolean {
  if (typeof address !== "string" || address === "") return false;
  let addr = address.trim().toLowerCase();
  // Strip an IPv6 zone id and brackets, if present.
  const zone = addr.indexOf("%");
  if (zone >= 0) addr = addr.slice(0, zone);
  if (addr.startsWith("[") && addr.endsWith("]")) addr = addr.slice(1, -1);
  // IPv4-mapped IPv6 (::ffff:127.0.0.1) is judged as its IPv4 address.
  if (addr.startsWith("::ffff:")) addr = addr.slice("::ffff:".length);
  if (addr.includes(".")) {
    const parts = addr.split(".");
    if (parts.length !== 4) return false;
    const octets = parts.map((p) => (/^\d{1,3}$/.test(p) ? Number(p) : NaN));
    if (octets.some((n) => !Number.isInteger(n) || n < 0 || n > 255)) return false;
    const [a, b] = octets as [number, number, number, number];
    if (a === 127) return true; // loopback 127.0.0.0/8
    if (a === 10) return true; // private 10.0.0.0/8
    if (a === 172 && b >= 16 && b <= 31) return true; // private 172.16.0.0/12
    if (a === 192 && b === 168) return true; // private 192.168.0.0/16
    if (a === 100 && b >= 64 && b <= 127) return true; // tailnet CGNAT 100.64.0.0/10
    if (a === 169 && b === 254) return true; // link-local 169.254.0.0/16
    return false;
  }
  if (addr === "::1" || addr === "::") return true; // IPv6 loopback / unspecified
  if (addr.startsWith("fc") || addr.startsWith("fd")) return true; // unique-local fc00::/7
  if (/^fe[89ab]/.test(addr)) return true; // link-local fe80::/10
  return false;
}

/**
 * The confirmation binding: a fresh login confirms a claim only when
 * the author DERIVED from its verified Access identity is the claimed
 * author (kind + id). The label is display text and never part of the
 * match; the released write carries the derived author, label and all.
 */
export function outageClaimMatchesAuthor(claimed: DashAuthor, derived: DashAuthor): boolean {
  return claimed.kind === derived.kind && claimed.id === derived.id;
}

function authorRef(author: DashAuthor): string {
  return `${author.kind}:${author.id}`;
}

export function outageWriteQueuedAuditEvent(write: OutageQueuedWrite): NewAuditEvent {
  return {
    actor: `claimed:${authorRef(write.claimedAuthor)}`,
    kind: "outage-write-queued",
    ref: write.action.objectRef,
    summary:
      `Outage-queued write held for identity confirmation: ${write.action.kind}/${write.action.verb} on ` +
      `${write.action.objectRef}, claimed by ${authorRef(write.claimedAuthor)} (unverified claim, ` +
      `provenance ${OUTAGE_QUEUED_PROVENANCE}, queue ${write.id})`,
    attribution: { confidence: "unknown", asserted: true },
    source: { identity: "pm-aggregator", link: "#9608" },
  };
}

export function outageWriteConfirmedAuditEvent(input: {
  write: OutageQueuedWrite;
  derivedAuthor: DashAuthor;
  actionId: string;
}): NewAuditEvent {
  return {
    actor: authorRef(input.derivedAuthor),
    kind: "outage-write-confirmed",
    ref: input.write.action.objectRef,
    summary:
      `Outage-queued write confirmed by a fresh verified login and released as action ${input.actionId}: ` +
      `${input.write.action.kind}/${input.write.action.verb} on ${input.write.action.objectRef} ` +
      `(queue ${input.write.id}, claimed by ${authorRef(input.write.claimedAuthor)})`,
    attribution: { confidence: "authenticated", asserted: true },
    source: { identity: "pm-aggregator", link: "#9608" },
  };
}

export function outageWriteRejectedAuditEvent(input: {
  write: OutageQueuedWrite;
  derivedAuthor: DashAuthor;
  byOwner: boolean;
}): NewAuditEvent {
  return {
    actor: authorRef(input.derivedAuthor),
    kind: "outage-write-rejected",
    ref: input.write.action.objectRef,
    summary:
      `Outage-queued write rejected ${input.byOwner ? "by the verified owner" : "by its claimant"}: ` +
      `${input.write.action.kind}/${input.write.action.verb} on ${input.write.action.objectRef} ` +
      `(queue ${input.write.id}, claimed by ${authorRef(input.write.claimedAuthor)})`,
    attribution: { confidence: "authenticated", asserted: true },
    source: { identity: "pm-aggregator", link: "#9608" },
  };
}

function validateStoredAction(value: unknown, file: string): NewAction {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`invalid outage-queue action in ${file}`);
  }
  const action = value as Record<string, unknown>;
  const author = validateAuthor(action.author);
  if (
    typeof action.clientToken !== "string" ||
    action.clientToken === "" ||
    typeof action.kind !== "string" ||
    typeof action.verb !== "string" ||
    typeof action.objectRef !== "string" ||
    action.objectRef === "" ||
    typeof action.payload !== "object" ||
    action.payload === null ||
    !author
  ) {
    throw new Error(`invalid outage-queue action in ${file}`);
  }
  return action as unknown as NewAction;
}

function validateRecord(row: unknown, file: string): OutageQueuedWrite {
  if (typeof row !== "object" || row === null || Array.isArray(row)) {
    throw new Error(`invalid outage-queue record in ${file}`);
  }
  const value = row as Record<string, unknown>;
  const claimedAuthor = validateAuthor(value.claimedAuthor);
  if (
    typeof value.id !== "string" ||
    value.id === "" ||
    typeof value.queuedAt !== "string" ||
    Number.isNaN(Date.parse(value.queuedAt)) ||
    value.provenance !== OUTAGE_QUEUED_PROVENANCE ||
    !claimedAuthor ||
    (value.state !== "queued" && value.state !== "confirmed" && value.state !== "rejected") ||
    (value.resolvedAt !== undefined &&
      (typeof value.resolvedAt !== "string" || Number.isNaN(Date.parse(value.resolvedAt)))) ||
    (value.resolvedActionId !== undefined && typeof value.resolvedActionId !== "string")
  ) {
    throw new Error(`invalid outage-queue record in ${file}`);
  }
  return {
    id: value.id,
    queuedAt: value.queuedAt,
    provenance: OUTAGE_QUEUED_PROVENANCE,
    claimedAuthor,
    action: validateStoredAction(value.action, file),
    state: value.state as OutageQueueState,
    ...(typeof value.resolvedAt === "string" ? { resolvedAt: value.resolvedAt } : {}),
    ...(typeof value.resolvedActionId === "string" ? { resolvedActionId: value.resolvedActionId } : {}),
  };
}

export class OutageQueueStore {
  private chain: Promise<void> = Promise.resolve();

  constructor(private readonly file: string) {}

  async list(): Promise<OutageQueuedWrite[]> {
    return (await this.load()).writes;
  }

  async get(id: string): Promise<OutageQueuedWrite | null> {
    const current = await this.load();
    return current.writes.find((w) => w.id === id) ?? null;
  }

  /** Find any record (any state) carrying this clientToken. */
  async byClientToken(clientToken: string): Promise<OutageQueuedWrite | null> {
    const current = await this.load();
    return current.writes.find((w) => w.action.clientToken === clientToken) ?? null;
  }

  async add(write: OutageQueuedWrite): Promise<void> {
    await this.serialize(async () => {
      const current = await this.load();
      current.writes.push(write);
      await this.persist(current);
    });
  }

  /**
   * Move a record out of "queued" exactly once, inside the serialized
   * section: a record that already resolved (or does not exist)
   * returns null and nothing is written.
   */
  async resolve(
    id: string,
    resolution: { state: "confirmed" | "rejected"; resolvedAt: string; resolvedActionId?: string },
  ): Promise<OutageQueuedWrite | null> {
    return this.serialize(async () => {
      const current = await this.load();
      const record = current.writes.find((w) => w.id === id);
      if (!record || record.state !== "queued") return null;
      record.state = resolution.state;
      record.resolvedAt = resolution.resolvedAt;
      if (resolution.resolvedActionId !== undefined) record.resolvedActionId = resolution.resolvedActionId;
      await this.persist(current);
      return { ...record };
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

  private async load(): Promise<OutageQueueFile> {
    let raw: string;
    try {
      raw = await readFile(this.file, "utf8");
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return { version: 1, writes: [] };
      throw error;
    }
    const parsed = JSON.parse(raw) as OutageQueueFile;
    if (parsed.version !== 1 || !Array.isArray(parsed.writes)) {
      throw new Error(`invalid outage-queue file: ${this.file}`);
    }
    return { version: 1, writes: parsed.writes.map((row) => validateRecord(row, this.file)) };
  }

  private async persist(value: OutageQueueFile): Promise<void> {
    await mkdir(dirname(this.file), { recursive: true });
    const tmp = `${this.file}.${randomUUID()}.tmp`;
    await writeFile(tmp, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 });
    await rename(tmp, this.file);
  }
}

/**
 * Queue a validated write under its claimed author. Idempotent by
 * clientToken, mirroring the actions store: a repost of a token that
 * is already in the queue (in any state) returns the existing record
 * and writes nothing — no second record, no second audit event. For a
 * new record the audit event is recorded BEFORE the record persists:
 * if the audit write fails, nothing is queued (fail closed).
 */
export async function queueOutageWrite(input: {
  store: OutageQueueStore;
  events: OutageQueueAuditSink;
  action: NewAction;
  now?: Date;
}): Promise<{ record: OutageQueuedWrite; created: boolean }> {
  const existing = await input.store.byClientToken(input.action.clientToken);
  if (existing) return { record: existing, created: false };
  const record: OutageQueuedWrite = {
    id: `oq_${randomUUID()}`,
    queuedAt: (input.now ?? new Date()).toISOString(),
    provenance: OUTAGE_QUEUED_PROVENANCE,
    claimedAuthor: input.action.author,
    action: input.action,
    state: "queued",
  };
  await input.events.record(outageWriteQueuedAuditEvent(record));
  await input.store.add(record);
  return { record, created: true };
}

export type ReleaseOutageWrite = (
  action: NewAction,
) => Promise<{ ok: true; actionId: string } | { ok: false; status: number; error: string }>;

export type ConfirmOutageWriteResult =
  | { ok: true; record: OutageQueuedWrite; actionId: string }
  | { ok: false; status: number; error: string };

/**
 * Confirm a queued write against the author derived from a fresh
 * verified login (the caller enforces the login; this function owns
 * the claim match, the release, the audit, and the state move):
 *
 * - unknown id -> 404; already resolved -> 409;
 * - derived author does not match the claim -> 403, nothing changes;
 * - the release (the caller's actions-store add, with its item
 *   checks) fails -> its status/error, record stays queued;
 * - the audit write fails -> 500, record stays queued — a retry is
 *   safe because the release dedupes by clientToken;
 * - only after the audit lands is the record marked confirmed, with
 *   the released action id.
 */
export async function confirmOutageWrite(input: {
  store: OutageQueueStore;
  events: OutageQueueAuditSink;
  id: string;
  derivedAuthor: DashAuthor;
  release: ReleaseOutageWrite;
  now?: Date;
}): Promise<ConfirmOutageWriteResult> {
  const record = await input.store.get(input.id);
  if (!record) return { ok: false, status: 404, error: "no such outage-queued write" };
  if (record.state !== "queued") {
    return { ok: false, status: 409, error: `outage-queued write already ${record.state}` };
  }
  if (!outageClaimMatchesAuthor(record.claimedAuthor, input.derivedAuthor)) {
    return {
      ok: false,
      status: 403,
      error: "the verified identity does not match the queued claim; only its claimant can confirm it",
    };
  }
  const released = await input.release({
    ...record.action,
    author: input.derivedAuthor,
    payload: { ...record.action.payload },
  });
  if (!released.ok) return { ok: false, status: released.status, error: released.error };
  try {
    await input.events.record(
      outageWriteConfirmedAuditEvent({ write: record, derivedAuthor: input.derivedAuthor, actionId: released.actionId }),
    );
  } catch {
    return { ok: false, status: 500, error: "outage-write confirmation audit write failed; the write stays queued" };
  }
  const resolved = await input.store.resolve(record.id, {
    state: "confirmed",
    resolvedAt: (input.now ?? new Date()).toISOString(),
    resolvedActionId: released.actionId,
  });
  if (!resolved) {
    return { ok: false, status: 409, error: "outage-queued write was resolved concurrently" };
  }
  return { ok: true, record: resolved, actionId: released.actionId };
}

export type RejectOutageWriteResult =
  | { ok: true; record: OutageQueuedWrite; byOwner: boolean }
  | { ok: false; status: number; error: string };

/**
 * Reject a queued write. The claimant rejects their own claim (same
 * match as confirmation); a verified owner (a fresh login deriving
 * the drew author) may reject any claim. The audit event is recorded
 * before the state move: if it fails, the record stays queued.
 */
export async function rejectOutageWrite(input: {
  store: OutageQueueStore;
  events: OutageQueueAuditSink;
  id: string;
  derivedAuthor: DashAuthor;
  now?: Date;
}): Promise<RejectOutageWriteResult> {
  const record = await input.store.get(input.id);
  if (!record) return { ok: false, status: 404, error: "no such outage-queued write" };
  if (record.state !== "queued") {
    return { ok: false, status: 409, error: `outage-queued write already ${record.state}` };
  }
  const isClaimant = outageClaimMatchesAuthor(record.claimedAuthor, input.derivedAuthor);
  const isOwner = input.derivedAuthor.kind === "drew" && input.derivedAuthor.id === "drew";
  if (!isClaimant && !isOwner) {
    return {
      ok: false,
      status: 403,
      error: "only the queued write's claimant (or the verified owner) can reject it",
    };
  }
  const byOwner = !isClaimant && isOwner;
  try {
    await input.events.record(
      outageWriteRejectedAuditEvent({ write: record, derivedAuthor: input.derivedAuthor, byOwner }),
    );
  } catch {
    return { ok: false, status: 500, error: "outage-write rejection audit write failed; the write stays queued" };
  }
  const resolved = await input.store.resolve(record.id, {
    state: "rejected",
    resolvedAt: (input.now ?? new Date()).toISOString(),
  });
  if (!resolved) {
    return { ok: false, status: 409, error: "outage-queued write was resolved concurrently" };
  }
  return { ok: true, record: resolved, byOwner };
}
