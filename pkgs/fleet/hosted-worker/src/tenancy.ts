// Tenancy mapping for fleet-hosted (#12583; design §4.1, §3.1).
//
// The tenancy plane (component E) owns accounts. This layer consumes
// exactly two of its facts:
//   1. subject -> account: an RP token's (issuer, subject) pair maps to
//      a tenancy account whose key is its `binding_id` — the minted
//      32-lowercase-hex id of the E/#2258 binding contract (the same
//      shape pkgs/id/hosted-worker's routing.ts validates). Stamps and
//      caller contexts record the binding_id, NEVER an email or a
//      provider subject (E's keying rule, design §4.1).
//   2. service tokens: per-caller tokens recorded by the tenancy plane,
//      one per caller, scoped to named routes (design §4.1). Records
//      here are keyed by the token's SHA-256 — the plaintext token is
//      never stored or looked up directly.
//
// The service accepts exactly the accounts the tenancy plane marks
// active (design §4.1); an unmapped subject or an inactive account
// fails closed.
//
// SKELETON SEAM: component E's tenancy DB does not exist yet, so the
// directory below is an interface plus a fixture implementation fed
// from configuration (TENANCY_FIXTURE_JSON — tests and local dev).
// When E lands, a DB-backed TenancyDirectory replaces the fixture
// behind this same interface; no caller changes.

/** Minted binding_id shape (component E / #2258 binding contract). */
export const BINDING_ID_RE = /^[0-9a-f]{32}$/;

export interface SubjectRecord {
  bindingId: string;
  active: boolean;
}

export interface ServiceTokenRecord {
  bindingId: string;
  /** Door ids this token may call (design §4.1: scoped to named routes). */
  routes: string[];
  active: boolean;
  label?: string;
}

export interface TenancyDirectory {
  /** Map a validated RP token's (issuer, subject) to its account. */
  resolveSubject(issuer: string, subject: string): Promise<SubjectRecord | null>;
  /** Find a service-token record by the token's SHA-256 hex digest. */
  findServiceToken(tokenSha256Hex: string): Promise<ServiceTokenRecord | null>;
}

interface FixtureSubject {
  issuer: string;
  subject: string;
  binding_id: string;
  active: boolean;
}

interface FixtureServiceToken {
  sha256: string;
  binding_id: string;
  routes: string[];
  active: boolean;
  label?: string;
}

export interface TenancyFixture {
  subjects?: FixtureSubject[];
  service_tokens?: FixtureServiceToken[];
}

function checkedBindingId(raw: string, where: string): string {
  if (!BINDING_ID_RE.test(raw)) {
    throw new Error(`tenancy fixture: ${where} binding_id is not a minted binding id`);
  }
  return raw;
}

/** Fixture-backed directory for the skeleton (see the seam note above). */
export class FixtureTenancyDirectory implements TenancyDirectory {
  private readonly subjects = new Map<string, SubjectRecord>();
  private readonly serviceTokens = new Map<string, ServiceTokenRecord>();

  constructor(fixture: TenancyFixture) {
    for (const s of fixture.subjects ?? []) {
      this.subjects.set(`${s.issuer}\n${s.subject}`, {
        bindingId: checkedBindingId(s.binding_id, "subject"),
        active: s.active === true,
      });
    }
    for (const t of fixture.service_tokens ?? []) {
      if (!/^[0-9a-f]{64}$/.test(t.sha256)) {
        throw new Error("tenancy fixture: service token sha256 is not a SHA-256 hex digest");
      }
      this.serviceTokens.set(t.sha256, {
        bindingId: checkedBindingId(t.binding_id, "service token"),
        routes: [...t.routes],
        active: t.active === true,
        label: t.label,
      });
    }
  }

  async resolveSubject(issuer: string, subject: string): Promise<SubjectRecord | null> {
    return this.subjects.get(`${issuer}\n${subject}`) ?? null;
  }

  async findServiceToken(tokenSha256Hex: string): Promise<ServiceTokenRecord | null> {
    return this.serviceTokens.get(tokenSha256Hex) ?? null;
  }
}

export function parseTenancyFixture(json: string): FixtureTenancyDirectory {
  const parsed: unknown = JSON.parse(json);
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new Error("tenancy fixture: top level is not a JSON object");
  }
  return new FixtureTenancyDirectory(parsed as TenancyFixture);
}
