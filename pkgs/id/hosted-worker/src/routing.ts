// Routing for the hosted data plane (#5199; design
// doc/2026-11-01_jurisdiction_aware_user_store_design.md as reconciled
// with the binding_id contract of #2258 / #3811 /
// pkgs/id/src/hosted_binding.rs).
//
// One routing module owns the raw USERS namespace. Objects are named by
// the stable binding_id minted by the binding ceremony (32 lowercase
// hex chars, random_hex_id in hosted_binding.rs) — never by email,
// never by a provider subject (no "google:<sub>" derivation), and the
// ID and the stub always come from the SAME jurisdiction-restricted
// namespace. There is no unrestricted fallback: any failure to obtain
// the restricted namespace fails closed before any object is touched.

export const SUPPORTED_JURISDICTIONS = ["us", "eu", "fedramp"] as const;
export type Jurisdiction = (typeof SUPPORTED_JURISDICTIONS)[number];

const BINDING_ID_RE = /^[0-9a-f]{32}$/;

export class FailClosed extends Error {
  readonly status: number;
  readonly code: string;
  constructor(status: number, code: string, message: string) {
    super(message);
    this.status = status;
    this.code = code;
  }
}

export function parseBindingId(raw: string | null): string {
  if (raw === null || raw === "") {
    throw new FailClosed(401, "missing_binding", "no resolved binding for this request");
  }
  if (!BINDING_ID_RE.test(raw)) {
    throw new FailClosed(401, "invalid_binding", "binding_id is not a minted binding id");
  }
  return raw;
}

export function parseJurisdiction(raw: string | null): Jurisdiction {
  if (raw === null || raw === "") {
    throw new FailClosed(403, "missing_jurisdiction", "account jurisdiction is missing; failing closed");
  }
  if (!(SUPPORTED_JURISDICTIONS as readonly string[]).includes(raw)) {
    throw new FailClosed(403, "unsupported_jurisdiction", "account jurisdiction is not supported; failing closed");
  }
  return raw as Jurisdiction;
}

export interface ObjectNamespaceLike {
  jurisdiction(j: string): ObjectNamespaceLike;
  idFromName(name: string): unknown;
  get(id: unknown): { fetch(req: Request): Promise<Response> };
}

// Obtain the stub for (bindingId, jurisdiction) from the restricted
// namespace only. jurisdiction() throwing (e.g. local workerd, which
// does not implement jurisdiction restrictions) is a fail-closed
// error, never a reason to use the unrestricted namespace.
export function userStub(
  ns: ObjectNamespaceLike,
  bindingId: string,
  jurisdiction: Jurisdiction,
): { fetch(req: Request): Promise<Response> } {
  let restricted: ObjectNamespaceLike;
  try {
    restricted = ns.jurisdiction(jurisdiction);
  } catch {
    throw new FailClosed(
      503,
      "jurisdiction_unavailable",
      "jurisdiction-restricted namespace unavailable; failing closed",
    );
  }
  if (!restricted || typeof restricted.idFromName !== "function" || typeof restricted.get !== "function") {
    throw new FailClosed(
      503,
      "jurisdiction_unavailable",
      "jurisdiction-restricted namespace unavailable; failing closed",
    );
  }
  const id = restricted.idFromName(bindingId);
  return restricted.get(id);
}
