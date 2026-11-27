// The hosted session registry (#12584; design §4.2).
//
// Native session state is env + worktree pwd-file (#9075). Hosted,
// there may be no worktree and no persistent process environment, so
// the session record moves server-side: the service mints the session
// id, records it against the caller's tenancy binding_id, and resolves
// ids against the registry on the stamp path. An unknown or closed id
// is refused with a named error, so a stamp can never cite a session
// the service did not open — the hosted form of #9075's "stale
// pwd-files do not silently misattribute new work" acceptance.
//
// Session ids use the native #9075 format — `ses_` followed by ASCII
// letters or digits (pkgs/fleet/crates/fleet/src/session_state.rs
// valid_session_id) — so an id means the same thing in both editions.
// The service mints `ses_` + 32 lowercase hex characters from 16
// crypto-random bytes (128 bits of entropy); the client never chooses
// its session id, and a client-sent id is validated against this same
// shape before the registry is consulted, exactly as the native
// resolver validates before it trusts any source.
//
// STORAGE: the registry below is an interface plus a fixture
// in-memory implementation — the TenancyDirectory idiom of
// src/tenancy.ts. The DB-backed implementation is D1SessionRegistry
// (src/d1-session-registry.ts), which the Worker resolves whenever
// its D1 binding is present; no caller changes.

import { FailClosed } from "./errors.ts";

/** Native session id shape (#9075): ses_ + ASCII alphanumerics. */
export const SESSION_ID_RE = /^ses_[A-Za-z0-9]+$/;

export function isValidSessionId(value: string): boolean {
  return SESSION_ID_RE.test(value);
}

/** Mint a session id: ses_ + 128 bits of crypto randomness as hex. */
export function mintSessionId(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  const hex = Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
  return `ses_${hex}`;
}

/** One registry row (design §4.2): the service's session record. */
export interface SessionRecord {
  sessionId: string;
  /** Tenancy account key (component E binding_id) of the opener. The
   * only identity the registry records — from the validated
   * credential, never from a client assertion. */
  bindingId: string;
  /** Declared lane, or the agent credential's lane; null when the
   * opener declared none. */
  lane: string | null;
  /** Declared model; null when undeclared. */
  model: string | null;
  /** Service-clock open time (ISO 8601). */
  openedAt: string;
  /** Service-clock close time (ISO 8601); null while open. */
  closedAt: string | null;
  /** The lane-start issue set as opened (issue refs, verbatim). */
  laneStartIssues: string[];
}

/** The registry's storage contract. Times are the service's clock,
 * supplied by the caller of open/close, so implementations stay pure
 * storage. */
export interface SessionRegistry {
  insert(record: SessionRecord): Promise<void>;
  get(sessionId: string): Promise<SessionRecord | null>;
  markClosed(sessionId: string, closedAt: string): Promise<SessionRecord | null>;
}

/** Fixture-backed registry for the skeleton (see the seam note
 * above). Records are copied in and out so callers cannot mutate the
 * registry's rows. */
export class FixtureSessionRegistry implements SessionRegistry {
  private readonly sessions = new Map<string, SessionRecord>();

  async insert(record: SessionRecord): Promise<void> {
    if (this.sessions.has(record.sessionId)) {
      throw new Error(`session registry: duplicate session id ${record.sessionId}`);
    }
    this.sessions.set(record.sessionId, { ...record, laneStartIssues: [...record.laneStartIssues] });
  }

  async get(sessionId: string): Promise<SessionRecord | null> {
    const record = this.sessions.get(sessionId);
    return record ? { ...record, laneStartIssues: [...record.laneStartIssues] } : null;
  }

  async markClosed(sessionId: string, closedAt: string): Promise<SessionRecord | null> {
    const record = this.sessions.get(sessionId);
    if (!record) return null;
    record.closedAt = closedAt;
    return { ...record, laneStartIssues: [...record.laneStartIssues] };
  }
}

/** The wire shape of a session record (the stamp/provenance
 * vocabulary: session_id, binding_id, lane_start_issues). */
export function sessionRecordJson(record: SessionRecord): Record<string, unknown> {
  return {
    session_id: record.sessionId,
    binding_id: record.bindingId,
    lane: record.lane,
    model: record.model,
    opened_at: record.openedAt,
    closed_at: record.closedAt,
    lane_start_issues: [...record.laneStartIssues],
  };
}

/** Validate a client-sent session id the way the native resolver
 * validates every source (checked_identity in session_state.rs): a
 * malformed id fails closed before the registry is consulted. */
export function checkedSessionId(raw: unknown): string {
  if (typeof raw !== "string" || !isValidSessionId(raw)) {
    throw new FailClosed(400, "invalid_session_id", "session ID must be ses_ followed by ASCII letters or digits");
  }
  return raw;
}

/** Look up a session owned by this binding. A session recorded
 * against another tenancy answers exactly like one that does not
 * exist — session_unknown either way — so the registry is never a
 * cross-tenant existence oracle. */
export async function getOwnedSession(
  registry: SessionRegistry,
  bindingId: string,
  sessionId: string,
): Promise<SessionRecord> {
  const record = await registry.get(sessionId);
  if (!record || record.bindingId !== bindingId) {
    throw new FailClosed(404, "session_unknown", "session id was not minted by this service for this account");
  }
  return record;
}

/** Stamp-path resolution (design §4.2): the session must exist, belong
 * to the caller's tenancy, and be open. Unknown and closed ids are
 * refused with named errors — no silent misattribution. */
export async function requireOpenSession(
  registry: SessionRegistry,
  bindingId: string,
  sessionId: string,
): Promise<SessionRecord> {
  const record = await getOwnedSession(registry, bindingId, sessionId);
  if (record.closedAt !== null) {
    throw new FailClosed(409, "session_closed", `session ${sessionId} is closed and cannot be cited by new work`);
  }
  return record;
}
