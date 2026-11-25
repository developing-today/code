// Authentication + route authorization for fleet-hosted (#12583;
// design §4.1, §3.3 rule 3). One plumbing layer shared by every door
// (design §3.1: the routes "share the auth validation, the tenancy
// lookup, and the DB clients").
//
// Three caller kinds, one Bearer header:
//   - "rp":      a person/agent-runtime caller with a #9077
//                relying-party token. The token is validated
//                (src/jwt.ts) and its subject mapped to a tenancy
//                binding_id (src/tenancy.ts).
//   - "agent":   an RP token carrying the #1253 agent role. Its lane /
//                session claims are the ceiling of what the caller may
//                assert (design §4.1), so both claims are required —
//                an agent credential without them is incomplete and
//                refused.
//   - "service": an opaque per-caller service token recorded in the
//                tenancy plane, scoped to named routes (door ids).
//                Route scope is enforced in authorizeRoute.
//
// Refused outright, by design:
//   - Per-user Turso DB credentials (component E data-plane tokens).
//     They are not service authentication and confer no rights here
//     (design §3.3 rule 3, §4.1). libsql/Turso tokens are JWTs whose
//     payload carries data-plane access claims (`a` / `p`) instead of
//     an RP issuer/audience; that shape is detected before signature
//     validation and refused with its own code, so the refusal is a
//     stated rule, not an incidental signature failure.
//   - A caller asserting a tenancy other than its credential's. The
//     resolved binding comes only from the credential; a client-sent
//     x-binding-id / x-tenancy-binding header that disagrees with it
//     is a cross-tenant attempt and fails closed (the pkgs/id
//     hosted-worker posture: internal context is never
//     client-controlled).

import type { BackfillRunStore } from "./backfill.ts";
import type { BridgeSurface } from "./bridge.ts";
import type { HostedConfig } from "./config.ts";
import { FailClosed } from "./errors.ts";
import type {
  GitHubBodyEditor,
  GitHubCommentSurface,
  GitHubItemSource,
  GitHubProjectSurface,
  GitHubWriter,
} from "./github.ts";
import { looksLikeJwt, parseJwt, validateRpToken, type JwksFetcher } from "./jwt.ts";
import type { KvBlobBucket } from "./kv-blobs.ts";
import type { KvPrimary } from "./kv-door.ts";
import type { QueueDepProbe, QueueStore } from "./queue-store.ts";
import type { SessionRegistry } from "./sessions.ts";
import type { MetadataStore, QueueRecordSource } from "./stamping.ts";

export type CallerKind = "rp" | "agent" | "service";

export interface CallerContext {
  kind: CallerKind;
  /** Tenancy account key (component E binding_id). The only identity
   * downstream layers record — never an email or provider subject. */
  bindingId: string;
  roles: string[];
  /** Agent callers only: the lane / session their credential names. */
  lane?: string;
  sessionId?: string;
  /** Service callers only: the door ids the token is scoped to. */
  serviceRoutes?: string[];
  serviceLabel?: string;
}

export interface AuthDeps {
  config: HostedConfig;
  jwksFetcher?: JwksFetcher;
  now?: () => number;
  /** Session registry (#12584, src/sessions.ts). Absent → the
   * session routes fail closed with 503, like unconfigured auth. */
  sessions?: SessionRegistry;
  /** Metadata store (#12585, src/stamping.ts). Absent → the stamping
   * route fails closed with 503, like unconfigured auth. */
  metadataStore?: MetadataStore;
  /** GitHub writer (#12585, src/github.ts): the hosted rotation
   * point or its fixture. Absent → the stamping route fails closed
   * with 503. */
  githubWriter?: GitHubWriter;
  /** GitHub item source (#12587, src/github.ts): the query door's
   * read path to current bodies — the §13 fallback scan and the
   * §15 divergence check. Absent → the query door serves DB rows
   * with body_checked false and runs no fallback; the store stays
   * the required dependency. */
  githubItems?: GitHubItemSource;
  /** Queue record source (#12585, src/stamping.ts). Absent → no
   * queue record is readable, so queue fields stamp null (the
   * design §5 rule for work that is not queue-originated); the queue
   * front door (#12589) wires the real source behind this seam. */
  queueRecords?: QueueRecordSource;
  /** Queue store (#12589, src/queue-store.ts): the queue door's
   * backend — the production home is the existing fleet-queue
   * Turso DB (#6741) behind the QueueStore interface. Absent → the
   * queue door fails closed with 503, like unconfigured auth. */
  queueStore?: QueueStore;
  /** GitHub dep probe for the queue door's dep-recheck (#12589).
   * Absent → the no-probe posture the native broker also runs:
   * GitHub deps report unprobeable and stay blocked until their
   * TTL; task deps resolve from the store either way. */
  queueDepProbe?: QueueDepProbe;
  /** GitHub comment surface (#12589, src/github.ts): the fallback
   * claim protocol's transport. Absent → the fallback verbs fail
   * closed with 503; the broker verbs do not need it. */
  githubComments?: GitHubCommentSurface;
  /** GitHub body editor (#12590, src/github.ts): ingestion's
   * write-back of ascertained Metadata blocks. Absent → the
   * ingestion check stores ascertained blocks DB-only and reports
   * writeback "not_attempted"; the store stays the required
   * dependency. */
  githubEditor?: GitHubBodyEditor;
  /** GitHub project surface (#12591, src/github.ts): the sync
   * door's project-sync backend — Projects V2 state and writes
   * over the rotation. Absent → the project-sync route fails
   * closed with 503, like unconfigured auth. */
  githubProjects?: GitHubProjectSurface;
  /** Bridge surface (#14373, src/bridge.ts): the bridge door's path
   * to the message bridge over its service binding, presenting the
   * PM credential. Absent → the bridge door fails closed with 503,
   * like unconfigured auth. */
  bridge?: BridgeSurface;
  /** Backfill run store (#14312, src/backfill.ts): the record of
   * ingestion backfill runs. Absent → the backfill routes fail
   * closed with 503, like unconfigured auth; the sweep and the
   * divergence report do not need it. */
  backfillRunStore?: BackfillRunStore;
  /** The fleet-kv central primary (#13316, src/kv-door.ts): the
   * shared Turso DB the kv door forwards catalogued statements to.
   * Absent → the kv door fails closed with 503, like unconfigured
   * auth. */
  kvPrimary?: KvPrimary;
  /** The kv blob bucket (#13318, src/kv-blobs.ts). Absent → the blob
   * upload door fails closed with 503. */
  kvBlobs?: KvBlobBucket;
}

export async function sha256Hex(text: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, "0")).join("");
}

function bearerToken(req: Request): string {
  const header = req.headers.get("authorization");
  if (!header) {
    throw new FailClosed(401, "missing_token", "no Authorization header");
  }
  const match = /^Bearer\s+(.+)$/i.exec(header.trim());
  if (!match) {
    throw new FailClosed(401, "missing_token", "Authorization is not a Bearer token");
  }
  return match[1].trim();
}

/** libsql/Turso data-plane tokens carry access claims (`a`: access
 * level, `p`: namespace permissions) in place of an RP iss/aud pair. */
function isDataPlaneCredential(payload: Record<string, unknown>): boolean {
  return "a" in payload || "p" in payload;
}

export async function authenticate(req: Request, deps: AuthDeps): Promise<CallerContext> {
  const { config } = deps;
  if (!config.authConfigured || !config.rp || !config.tenancy) {
    throw new FailClosed(
      503,
      "auth_not_configured",
      "this deployment has no RP issuer / tenancy directory configured; failing closed",
    );
  }
  const token = bearerToken(req);

  if (looksLikeJwt(token)) {
    const { payload } = parseJwt(token);
    if (isDataPlaneCredential(payload)) {
      throw new FailClosed(
        401,
        "data_plane_credential_refused",
        "a per-user Turso DB credential is a data-plane credential, not service authentication (design §3.3 rule 3)",
      );
    }
    const claims = await validateRpToken(token, config.rp, deps.jwksFetcher, deps.now?.());
    const record = await config.tenancy.resolveSubject(config.rp.issuer, claims.subject);
    if (!record) {
      throw new FailClosed(403, "unknown_subject", "token subject maps to no tenancy account");
    }
    if (!record.active) {
      throw new FailClosed(403, "account_inactive", "tenancy account is not marked active");
    }
    if (claims.roles.includes("agent")) {
      if (!claims.lane || !claims.sessionId) {
        throw new FailClosed(
          401,
          "agent_claims_incomplete",
          "an agent credential must name its lane and session (design §4.1)",
        );
      }
      return {
        kind: "agent",
        bindingId: record.bindingId,
        roles: claims.roles,
        lane: claims.lane,
        sessionId: claims.sessionId,
      };
    }
    return { kind: "rp", bindingId: record.bindingId, roles: claims.roles };
  }

  const record = await config.tenancy.findServiceToken(await sha256Hex(token));
  if (!record) {
    throw new FailClosed(401, "invalid_token", "token is not a known service token");
  }
  if (!record.active) {
    throw new FailClosed(403, "service_token_inactive", "service token is not marked active");
  }
  return {
    kind: "service",
    bindingId: record.bindingId,
    roles: [],
    serviceRoutes: record.routes,
    serviceLabel: record.label,
  };
}

/** Service tokens are scoped to named routes (door ids); RP and agent
 * callers are not route-scoped in this slice (their roles gate later
 * slices' business logic, not the plumbing). */
export function authorizeRoute(ctx: CallerContext, doorId: string): void {
  if (ctx.kind === "service" && !(ctx.serviceRoutes ?? []).includes(doorId)) {
    throw new FailClosed(403, "service_token_route_scope", `service token is not scoped to the ${doorId} door`);
  }
}

/** The credential is the only source of tenancy. A client-sent binding
 * header that disagrees with the resolved binding is a cross-tenant
 * attempt; one that agrees is redundant and ignored. */
export function assertCallerTenancy(req: Request, ctx: CallerContext): void {
  for (const name of ["x-binding-id", "x-tenancy-binding"]) {
    const asserted = req.headers.get(name);
    if (asserted !== null && asserted !== ctx.bindingId) {
      throw new FailClosed(403, "cross_tenant_refused", "request asserts a tenancy other than its credential's");
    }
  }
}
