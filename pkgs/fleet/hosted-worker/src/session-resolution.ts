// Session resolution for hosted callers (#12584; design §3.2, §4.2).
//
// The thin TS CLI carries the session id exactly as native resolution
// prescribes — explicit argument > environment > stored state — the
// #9075 resolution order with the pwd-file replaced by the CLI's
// per-workspace state file (the hosted analogue of .fleet-session-id:
// same recovery property, without presuming a git worktree exists).
//
// This module is a step-for-step transcription of the native resolver
// (pkgs/fleet/crates/fleet/src/session_state.rs, resolve /
// resolve_marker), and test/session-parity.test.ts runs one case
// table, transcribed from that file's tests, against it:
//
//   - Values are trimmed; empty or whitespace-only values are absent
//     at every level (native nonempty()).
//   - The environment level is FLEET_SESSION_ID, then
//     OPENCODE_SESSION_ID (pkgs/fleet/README, #9075 section).
//   - An id from any consulted source is format-checked before it is
//     trusted (native checked_identity): a malformed nonempty value
//     fails closed — it never falls through to a lower source.
//   - The stored id is used only when it is live. Hosted, liveness is
//     a registry question (the session exists and is still open — the
//     caller wires the predicate to a resolve call); a stored id that
//     is ended or unverifiable is a named error, never a silent
//     fallback (native: "stale .fleet-session-id refers to ended or
//     unverifiable session …").
//   - An identity supplied by the explicit or environment level is
//     persisted to the stored state for a later bare invocation, and
//     the stored state is not even read when a higher level supplies
//     one (native resolve_marker).

import { isValidSessionId } from "./sessions.ts";

export const FLEET_SESSION_ENV = "FLEET_SESSION_ID";
export const OPENCODE_SESSION_ENV = "OPENCODE_SESSION_ID";

export type SessionResolutionSource = "explicit" | "env" | "stored";

export type SessionResolution =
  | { kind: "session"; sessionId: string; source: SessionResolutionSource }
  | { kind: "none" };

export class SessionResolutionError extends Error {
  readonly code: "invalid_session_id" | "stale_session_state";
  constructor(code: "invalid_session_id" | "stale_session_state", message: string) {
    super(message);
    this.code = code;
  }
}

function nonempty(value: string | null | undefined): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  return trimmed === "" ? null : trimmed;
}

function checkedIdentity(id: string): string {
  if (!isValidSessionId(id)) {
    throw new SessionResolutionError(
      "invalid_session_id",
      "session ID must be ses_ followed by ASCII letters or digits",
    );
  }
  return id;
}

export interface SessionResolutionSources {
  /** The explicit argument (a --session-id flag or equivalent). */
  explicit?: string | null;
  /** Environment lookup by variable name (injectable for tests). */
  env?: (name: string) => string | null | undefined;
  /** Contents of the per-workspace state file, or null when absent. */
  stored?: string | null;
}

/**
 * Resolve the session id in #9075's order. `isLive` is consulted at
 * most once, and only for the stored id — explicit and environment
 * identities are trusted after format validation, as native trusts
 * them (their liveness is the service's answer when they are used).
 */
export function resolveSessionId(
  sources: SessionResolutionSources,
  isLive: (sessionId: string) => boolean,
): SessionResolution {
  const explicit = nonempty(sources.explicit);
  if (explicit !== null) {
    return { kind: "session", sessionId: checkedIdentity(explicit), source: "explicit" };
  }
  const env = sources.env ?? (() => null);
  const fromEnv = nonempty(env(FLEET_SESSION_ENV)) ?? nonempty(env(OPENCODE_SESSION_ENV));
  if (fromEnv !== null) {
    return { kind: "session", sessionId: checkedIdentity(fromEnv), source: "env" };
  }
  const stored = nonempty(sources.stored);
  if (stored === null) {
    return { kind: "none" };
  }
  const id = checkedIdentity(stored);
  if (isLive(id)) {
    return { kind: "session", sessionId: id, source: "stored" };
  }
  throw new SessionResolutionError(
    "stale_session_state",
    `stale stored session state refers to ended or unverifiable session ${id}; remove it or start a new Fleet session`,
  );
}

/** The CLI's per-workspace state file, behind an interface so the
 * resolver stays testable and storage stays the CLI's business. */
export interface SessionStateStore {
  read(): string | null;
  write(sessionId: string): void;
}

/** In-memory store for tests and embedded callers. */
export class InMemorySessionStateStore implements SessionStateStore {
  private value: string | null = null;

  read(): string | null {
    return this.value;
  }

  write(sessionId: string): void {
    this.value = sessionId;
  }
}

/**
 * resolve_marker, hosted form: resolve, and persist an identity the
 * explicit or environment level supplied so a later bare invocation
 * in the same workspace recovers it. The store is read only when no
 * higher level supplies an identity — a corrupt stored state cannot
 * break an invocation that was told its session.
 */
export function resolveAndBindSession(args: {
  explicit?: string | null;
  env?: (name: string) => string | null | undefined;
  store: SessionStateStore;
  isLive: (sessionId: string) => boolean;
}): SessionResolution {
  const env = args.env ?? (() => null);
  const supplied =
    nonempty(args.explicit) !== null ||
    nonempty(env(FLEET_SESSION_ENV)) !== null ||
    nonempty(env(OPENCODE_SESSION_ENV)) !== null;
  const resolution = resolveSessionId(
    { explicit: args.explicit, env, stored: supplied ? null : args.store.read() },
    args.isLive,
  );
  if (resolution.kind === "session" && resolution.source !== "stored") {
    args.store.write(resolution.sessionId);
  }
  return resolution;
}
