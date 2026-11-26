// Front-door caller authentication (#15479, from the #9080 gap
// analysis).
//
// Before this module the front door trusted the X-Binding-Id /
// X-Jurisdiction headers outright: the header comment said they were
// "set by the authenticated internal caller", but no such caller
// existed and nothing authenticated anyone — any client that could
// reach the worker could name any binding. Now the front door
// authenticates the caller FIRST and resolves the binding and
// jurisdiction server-side from the caller's credential:
//
//   Authorization: Bearer <caller token>
//
// The token is resolved through a caller directory: a token names
// exactly one caller record — one binding, one jurisdiction, and
// optionally the binding's ID key (#3808) — and the request is
// routed ONLY to that record's binding, under that record's
// jurisdiction. Client-supplied X-Binding-Id / X-Jurisdiction are
// never routing input any more: when present they must agree with
// the resolved record exactly, and a disagreement is a mis-bound
// request, refused 403 before any namespace access. The same goes
// for the ID key: X-Iroh-Id from the client is dropped, never
// forwarded; the internal key header is set only from the caller
// record.
//
// The shipped directory is the FRONT_DOOR_CALLERS env var (a JSON
// array of caller records), because the package has no H-issued
// session/OIDC verifier yet — the PM-side SSO relying party
// (#9086) owns that, and when it lands it plugs in behind the
// FrontDoorCallerDirectory seam below without the front door
// changing shape. Fail closed throughout, the package's standing
// rule: no directory configured -> 503 for every request (the
// header-only path grants nothing); a malformed directory -> 503;
// a missing or unknown credential -> 401.

import { FailClosed, parseBindingId, parseJurisdiction, type Jurisdiction } from "./routing.ts";

/** One authenticated caller: the identity a credential resolves to. */
export interface FrontDoorCaller {
  /** The only binding this caller may reach. */
  bindingId: string;
  /** The binding's jurisdiction, resolved server-side. */
  jurisdiction: Jurisdiction;
  /** The binding record's ID key (hex, #3808), when the record names one. */
  irohId?: string;
}

/**
 * The caller-resolution seam: given the presented credential, return
 * the caller it authenticates as, or undefined to refuse. An H-issued
 * session verifier (#9086) implements this same shape.
 */
export interface FrontDoorCallerDirectory {
  resolve(token: string): FrontDoorCaller | undefined;
}

/** Byte-wise comparison that does not short-circuit on a mismatch. */
function tokenEquals(a: string, b: string): boolean {
  const ab = new TextEncoder().encode(a);
  const bb = new TextEncoder().encode(b);
  let diff = ab.length ^ bb.length;
  for (let i = 0; i < Math.max(ab.length, bb.length); i++) {
    diff |= (ab[i] ?? 0) ^ (bb[i] ?? 0);
  }
  return diff === 0;
}

/** A caller directory over a fixed record set. */
export class StaticFrontDoorCallerDirectory implements FrontDoorCallerDirectory {
  readonly #callers: ReadonlyArray<{ token: string; caller: FrontDoorCaller }>;

  constructor(callers: ReadonlyArray<{ token: string; caller: FrontDoorCaller }>) {
    this.#callers = callers;
  }

  resolve(token: string): FrontDoorCaller | undefined {
    let found: FrontDoorCaller | undefined;
    for (const entry of this.#callers) {
      if (tokenEquals(entry.token, token)) found = entry.caller;
    }
    return found;
  }
}

function misconfigured(message: string): FailClosed {
  return new FailClosed(503, "front_door_auth_misconfigured", message);
}

/**
 * Builds the caller directory from the FRONT_DOOR_CALLERS env var: a
 * JSON array of `{ token, bindingId, jurisdiction, irohId? }`
 * records. Absent/empty fails closed as unconfigured — a deployment
 * that configures no callers admits no one. Any malformed record
 * fails the whole directory closed: a partially parsed caller set
 * must never silently admit a subset.
 */
export function frontDoorCallersFromEnv(raw: string | undefined): FrontDoorCallerDirectory {
  if (raw === undefined || raw.trim() === "") {
    throw new FailClosed(
      503,
      "front_door_auth_unconfigured",
      "no front-door caller directory is configured; failing closed",
    );
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    throw misconfigured("FRONT_DOOR_CALLERS is not valid JSON; failing closed");
  }
  if (!Array.isArray(parsed) || parsed.length === 0) {
    throw misconfigured("FRONT_DOOR_CALLERS must be a non-empty JSON array of caller records; failing closed");
  }
  const seenTokens = new Set<string>();
  const callers: Array<{ token: string; caller: FrontDoorCaller }> = [];
  for (const entry of parsed) {
    if (typeof entry !== "object" || entry === null) {
      throw misconfigured("FRONT_DOOR_CALLERS entries must be objects; failing closed");
    }
    const { token, bindingId, jurisdiction, irohId } = entry as Record<string, unknown>;
    if (typeof token !== "string" || token === "") {
      throw misconfigured("every FRONT_DOOR_CALLERS entry needs a non-empty token; failing closed");
    }
    if (seenTokens.has(token)) {
      throw misconfigured("FRONT_DOOR_CALLERS names the same token twice; failing closed");
    }
    seenTokens.add(token);
    let caller: FrontDoorCaller;
    try {
      caller = {
        bindingId: parseBindingId(typeof bindingId === "string" ? bindingId : null),
        jurisdiction: parseJurisdiction(typeof jurisdiction === "string" ? jurisdiction : null),
      };
    } catch {
      throw misconfigured("a FRONT_DOOR_CALLERS entry has an invalid bindingId or jurisdiction; failing closed");
    }
    if (irohId !== undefined) {
      if (typeof irohId !== "string" || !/^[0-9a-f]{64}$/.test(irohId)) {
        throw misconfigured("a FRONT_DOOR_CALLERS entry has an invalid irohId; failing closed");
      }
      caller.irohId = irohId;
    }
    callers.push({ token, caller });
  }
  return new StaticFrontDoorCallerDirectory(callers);
}

/**
 * Authenticates the request's caller and returns the caller record
 * the request may act as. The binding and jurisdiction come ONLY
 * from this record. Client-supplied routing headers are checked
 * against it when present — never consumed: a header naming another
 * binding or jurisdiction is a mis-bound request and is refused.
 */
export function authenticateFrontDoor(req: Request, directory: FrontDoorCallerDirectory): FrontDoorCaller {
  const authorization = req.headers.get("authorization");
  const match = authorization === null ? null : /^Bearer (\S+)$/.exec(authorization);
  if (match === null) {
    throw new FailClosed(
      401,
      "unauthenticated_caller",
      "the front door requires an Authorization: Bearer caller credential; binding headers alone grant nothing",
    );
  }
  const caller = directory.resolve(match[1]);
  if (caller === undefined) {
    throw new FailClosed(401, "invalid_caller_credential", "the caller credential is not recognized; failing closed");
  }
  const claimedBinding = req.headers.get("x-binding-id");
  if (claimedBinding !== null && claimedBinding !== caller.bindingId) {
    throw new FailClosed(
      403,
      "caller_binding_mismatch",
      "the request names a binding other than the authenticated caller's own; failing closed",
    );
  }
  const claimedJurisdiction = req.headers.get("x-jurisdiction");
  if (claimedJurisdiction !== null && claimedJurisdiction !== caller.jurisdiction) {
    throw new FailClosed(
      403,
      "caller_jurisdiction_mismatch",
      "the request names a jurisdiction other than the authenticated caller's own; failing closed",
    );
  }
  return caller;
}
