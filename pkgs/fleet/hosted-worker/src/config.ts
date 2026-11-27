// Environment configuration for fleet-hosted (#12583).
//
// Everything the auth layer needs is configuration, because its two
// upstreams are other components' deliverables: the #9077 relying-party
// provider (issuer / JWKS) and component E's tenancy plane (here, the
// fixture seam in tenancy.ts). Missing configuration never fails open:
// it makes authConfigured false, protected routes refuse with 503, and
// health reports the gap as booleans (never values).
//
// The stamp signing material (#12586) follows the same rule: keys are
// configuration referenced by name, parsed and length-checked here,
// held as bytes, and never logged or reported. Malformed material is a
// configuration failure — that piece parses to null and the stamp path
// fails closed (see stamp-sig.ts), exactly like a malformed tenancy
// fixture. The fleet App material for the hosted GitHub rotation
// (#12585) parses the same way: malformed → null → the fixture writer
// stands in locally and no real GitHub write is attempted with bad
// material.

import type { RpValidationConfig } from "./jwt.ts";
import { base64UrlToBytes } from "../../../hosted-primitives/src/encoding.ts";
import { parseTenancyFixture, type TenancyDirectory } from "./tenancy.ts";
import type { D1DatabaseLike } from "./d1-queue-store.ts";
import type { R2BucketLike } from "./kv-blobs.ts";

export const HOSTED_VERSION = "0.1.0";
export const SERVICE_NAME = "fleet-hosted";
export const DEFAULT_AUDIENCE = "fleet-hosted";

export interface HostedEnv {
  RP_ISSUER?: string;
  RP_AUDIENCE?: string;
  RP_JWKS_URI?: string;
  TENANCY_FIXTURE_JSON?: string;
  /** Stamp signing key (#12586): base64url PKCS#8 Ed25519 private key.
   * A secret, referenced by name only; never logged. */
  STAMP_SIGNING_KEY_PKCS8?: string;
  /** Key id the signing key mints stamps under ([A-Za-z0-9_-]+). */
  STAMP_SIGNING_KEY_ID?: string;
  /** Stamp verifier keys (#12586): JSON object mapping key id to a
   * base64url raw Ed25519 public key. Several ids = the rotation
   * window; both editions' authorities appear here side by side. */
  STAMP_VERIFY_KEYS_JSON?: string;
  /** Fleet App id (#12585): the hosted rotation point's App. */
  GITHUB_APP_ID?: string;
  /** Fleet App private key (#12585): base64url PKCS#8 RSA key. A
   * secret, referenced by name only; never logged. (GitHub issues
   * the key as a PEM download; provisioning converts it to PKCS#8 —
   * the only RSA form WebCrypto imports.) */
  GITHUB_APP_PRIVATE_KEY_PKCS8?: string;
  /** Fleet App installation ids (#12585), comma-separated, in
   * rotation order: rate-limit / permission refusals fail over to
   * the next installation (the native #8887 rule). */
  GITHUB_APP_INSTALLATION_IDS?: string;
  /** GitHub webhook secret (#12590): the shared secret GitHub signs
   * deliveries with (X-Hub-Signature-256, HMAC-SHA256). A secret,
   * referenced by name only; never logged. Absent → the webhook
   * receiver fails closed (503), like unconfigured auth. */
  GITHUB_WEBHOOK_SECRET?: string;
  /** Ingestion scope (#12590): the repos the scheduled sweeper
   * re-scans, comma-separated owner/name — "in-scope repos" in
   * #9060 §16's terms (all repos the fleet works in). The sweep
   * route may name repos per request instead; this is the
   * scheduled run's standing scope. */
  INGESTION_REPOS?: string;
  /** Metadata DB id (#12592): the one database a direct-read token
   * may be bound to. A plain identifier, not a secret. */
  METADATA_DB_ID?: string;
  /** Queue DB id (#12592): named in configuration so the direct-read
   * scope can refuse it by name — direct access to the queue DB is
   * forbidden (design §3.3 rule 2). A plain identifier. */
  QUEUE_DB_ID?: string;
  /** Direct-read mint signing key (#12592): base64url PKCS#8 Ed25519
   * private key. A secret, referenced by name only; never logged. */
  DIRECT_READ_SIGNING_KEY_PKCS8?: string;
  /** Key id the direct-read mint signs under ([A-Za-z0-9_-]+). */
  DIRECT_READ_SIGNING_KEY_ID?: string;
  /** Direct-read verifier keys (#12592): JSON object mapping key id
   * to a base64url raw Ed25519 public key. */
  DIRECT_READ_VERIFY_KEYS_JSON?: string;
  /** Direct-read TTL cap in seconds (#12592): a token's requested
   * lifetime may not exceed it. Defaults to 3600 when unset. */
  DIRECT_READ_MAX_TTL_SECONDS?: string;
  /** Queue libSQL file (#12589 follow-on): path of the file-backed
   * local libSQL DB holding the shared fleet-queue schema (#6741).
   * Empty/unset → the fixture store (tests/local-dev default). A
   * remote Turso DB is QUEUE_LIBSQL_URL below, not a second schema. */
  QUEUE_LIBSQL_FILE?: string;
  /** The remote fleet-queue Turso DB (#14592): the queue verbs run
   *  against it through the Hrana opener — the production backend,
   *  the live DB the native broker serves (#6741). A libsql:// or
   *  https:// URL (wrangler [vars]); takes precedence over
   *  QUEUE_LIBSQL_FILE when both are set. */
  QUEUE_LIBSQL_URL?: string;
  /** The Turso auth token for QUEUE_LIBSQL_URL (a secret binding,
   *  never committed). Unset for unauthenticated local endpoints. */
  QUEUE_LIBSQL_AUTH_TOKEN?: string;
  /** The fleet-kv central primary (#13316): the shared Turso DB the
   *  kv door forwards catalogued statements to over Hrana. A
   *  libsql:// or https:// URL (wrangler [vars]); unset → the kv
   *  door fails closed (503). */
  KV_LIBSQL_URL?: string;
  /** The Turso auth token for KV_LIBSQL_URL (a secret binding,
   *  never committed). Held by the Worker only: kv callers present
   *  their own RP/agent JWT or kv-scoped service token, never this
   *  data-plane credential. */
  KV_LIBSQL_AUTH_TOKEN?: string;
  /** The fleet-queue D1 database, by binding (#14866): the queue
   *  verbs run against it through D1QueueStore when the binding is
   *  present \u2014 the Cloudflare-first data plane. A binding, not a
   *  value: present when the deployed shape carries it
   *  (wrangler.toml declares it); its presence is the selection
   *  signal, and it takes precedence over the Turso/libSQL queue
   *  backends in buildDeps. Absent \u2192 those backends, unchanged. */
  QUEUE_D1?: D1DatabaseLike;
  /** The kv blob bucket (#13318): an R2 binding for the blob upload
   * door. Absent → that door fails closed (503). */
  KV_BLOBS?: R2BucketLike;
  /** The kv blob read-capability key (#13318): a secret, set by name
   * at deployment, never in wrangler.toml. Absent → the read routes
   * fail closed (503). */
  KV_BLOB_CAPABILITY_KEY?: string;
  /** The message bridge, by service binding (#14373): the bridge
   * door calls it with the PM credential below. A binding, not a
   * value: present when the deployed shape carries it (wrangler.toml
   * declares it; local dev and tests inject their own or none). */
  BRIDGE?: { fetch(request: Request): Promise<Response> };
  /** The PM stack's bridge credential (#14373): minted on the bridge
   * for service:pm-stack, held as a Worker secret. A secret,
   * referenced by name only; never logged. Absent or short → the
   * bridge door fails closed (503). */
  BRIDGE_PM_TOKEN?: string;
}

/** Stamp signing key material: the parsed key, bytes only. */
export interface StampSigningMaterial {
  keyId: string;
  pkcs8: Uint8Array;
}

export interface StampConfig {
  /** Signing material, or null when no (valid) signing key is set. */
  signing: StampSigningMaterial | null;
  /** Verifier public keys by key id (raw Ed25519 bytes), or null when
   * no (valid) verifier set is configured. */
  verifyKeys: ReadonlyMap<string, Uint8Array> | null;
}

/** Direct-read configuration for the direct-Turso Metadata read path
 * (#12592; design §3.3): the two DB ids the scope distinguishes, the
 * mint's signing material, the verifier keys, and the TTL cap. Null
 * as a whole when any piece is missing or malformed — minting then
 * fails closed and inspection refuses everything. */
export interface DirectReadConfig {
  metadataDbId: string;
  queueDbId: string;
  signing: StampSigningMaterial;
  verifyKeys: ReadonlyMap<string, Uint8Array>;
  maxTtlSeconds: number;
}

/** Fleet App configuration for the hosted rotation point (#12585):
 * the parsed App id, private-key bytes, and installation ids in
 * rotation order. */
export interface GitHubAppConfig {
  appId: string;
  privateKeyPkcs8: Uint8Array;
  installationIds: number[];
}

export interface HostedConfig {
  version: string;
  /** RP validation config, or null when no issuer is configured. */
  rp: RpValidationConfig | null;
  /** Tenancy directory, or null when none is configured/readable. */
  tenancy: TenancyDirectory | null;
  /** True only when every auth input is present and parsed. Note
   * (#14978): authenticate() no longer requires this as a whole —
   * service-token callers need only the tenancy directory; this flag
   * stays the "full auth (RP + tenancy)" summary for reporting. */
  authConfigured: boolean;
  /** True when a tenancy directory is configured: service-token
   * callers can authenticate (#14978 / CF-2b) even before an RP
   * issuer exists. */
  serviceAuthConfigured: boolean;
  /** Stamp signature key material (#12586), independent of auth. */
  stamp: StampConfig;
  /** Fleet App rotation configuration (#12585), or null when no
   * (valid) App material is set — the Worker then wires the fixture
   * GitHub writer (local/dev semantics, src/github.ts). */
  github: GitHubAppConfig | null;
  /** GitHub webhook secret (#12590), or null when unset/empty —
   * the ingestion receiver then fails closed (503). */
  webhookSecret: string | null;
  /** The sweeper's standing repo scope (#12590), parsed from
   * INGESTION_REPOS: owner/name entries only, in listed order.
   * Empty when unconfigured — the scheduled sweep then has
   * nothing to scan and the sweep route must be given repos. */
  ingestionRepos: string[];
  /** Direct-Turso read-path configuration (#12592), or null when no
   * (valid) DB ids / mint material are set. Independent of auth,
   * like the stamp material. */
  directRead: DirectReadConfig | null;
  /** File-backed queue DB path (#12589 follow-on task 23569), or
   * null when unset — the Worker then wires the fixture store
   * (local/dev semantics). Named in configuration so the choice is
   * explicit; the opener that turns the path into a connection is
   * injected at the worker seam (tests inject node:sqlite — the
   * Worker bundle itself stays runtime-agnostic). */
  queueLibsqlFile: string | null;
  /** The remote fleet-queue Turso DB the queue verbs run against
   *  through the Hrana opener (#14592) — the production backend.
   *  Null → the file backend above, or the fixture. */
  queueLibsqlUrl: string | null;
  /** The auth token for queueLibsqlUrl (secret, never a [vars]
   *  value); null for unauthenticated endpoints. */
  queueLibsqlAuthToken: string | null;
  /** The fleet-kv central primary (#13316), or null when unset —
   *  the kv door then fails closed (503). */
  kvLibsqlUrl: string | null;
  /** The auth token for kvLibsqlUrl (secret); null for an
   *  unauthenticated local endpoint. */
  kvLibsqlAuthToken: string | null;
  /** The PM bridge credential (#14373), or null when unset or too
   * short to be one — the bridge door then fails closed (503). The
   * value is held only to be presented to the bridge; it is never
   * logged or reported. Independent of auth, like the webhook
   * secret. */
  bridge: { token: string } | null;
}

/** Stamp key ids name a key inside the stamp_sig value, so they may
 * not contain the format's `.` separator (see stamp-sig.ts). */
export function isValidStampKeyId(value: string): boolean {
  return /^[A-Za-z0-9_-]+$/.test(value);
}

function b64urlToBytesOrNull(text: string): Uint8Array | null {
  try {
    return base64UrlToBytes(text);
  } catch {
    return null;
  }
}

const ED25519_PKCS8_BYTES = 48;
const ED25519_PUBLIC_KEY_BYTES = 32;

function parseStampConfig(env: HostedEnv): StampConfig {
  let signing: StampSigningMaterial | null = null;
  if (env.STAMP_SIGNING_KEY_PKCS8 && env.STAMP_SIGNING_KEY_ID) {
    const pkcs8 = b64urlToBytesOrNull(env.STAMP_SIGNING_KEY_PKCS8);
    if (pkcs8 !== null && pkcs8.length === ED25519_PKCS8_BYTES && isValidStampKeyId(env.STAMP_SIGNING_KEY_ID)) {
      signing = { keyId: env.STAMP_SIGNING_KEY_ID, pkcs8 };
    }
    // Malformed signing material stays null: signing fails closed.
  }

  let verifyKeys: Map<string, Uint8Array> | null = null;
  if (env.STAMP_VERIFY_KEYS_JSON) {
    try {
      const parsed: unknown = JSON.parse(env.STAMP_VERIFY_KEYS_JSON);
      if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
        throw new Error("verify keys must be a JSON object");
      }
      const map = new Map<string, Uint8Array>();
      for (const [keyId, text] of Object.entries(parsed as Record<string, unknown>)) {
        if (!isValidStampKeyId(keyId) || typeof text !== "string") {
          throw new Error("verify key entry is malformed");
        }
        const raw = b64urlToBytesOrNull(text);
        if (raw === null || raw.length !== ED25519_PUBLIC_KEY_BYTES) {
          throw new Error("verify key is not a raw Ed25519 public key");
        }
        map.set(keyId, raw);
      }
      verifyKeys = map.size > 0 ? map : null;
    } catch {
      // A malformed verifier set is a configuration failure: leave it
      // null so verification reports unverifiable, never verified.
      verifyKeys = null;
    }
  }

  return { signing, verifyKeys };
}

function parseGitHubAppConfig(env: HostedEnv): GitHubAppConfig | null {
  if (!env.GITHUB_APP_ID || !env.GITHUB_APP_PRIVATE_KEY_PKCS8 || !env.GITHUB_APP_INSTALLATION_IDS) {
    return null;
  }
  if (!/^[0-9]+$/.test(env.GITHUB_APP_ID)) return null;
  const pkcs8 = b64urlToBytesOrNull(env.GITHUB_APP_PRIVATE_KEY_PKCS8);
  // A PKCS#8 RSA key is far larger than any Ed25519 material; the
  // floor only rejects obvious garbage — the real check is the
  // WebCrypto import at first use, which fails closed (502).
  if (pkcs8 === null || pkcs8.length < 256) return null;
  const installationIds: number[] = [];
  for (const part of env.GITHUB_APP_INSTALLATION_IDS.split(",")) {
    const trimmed = part.trim();
    if (!/^[0-9]+$/.test(trimmed)) return null;
    installationIds.push(Number(trimmed));
  }
  if (installationIds.length === 0) return null;
  return { appId: env.GITHUB_APP_ID, privateKeyPkcs8: pkcs8, installationIds };
}

export function configFromEnv(env: HostedEnv): HostedConfig {
  const rp: RpValidationConfig | null = env.RP_ISSUER
    ? {
        issuer: env.RP_ISSUER,
        audience: env.RP_AUDIENCE ?? DEFAULT_AUDIENCE,
        jwksUri: env.RP_JWKS_URI ?? `${env.RP_ISSUER.replace(/\/$/, "")}/.well-known/jwks.json`,
      }
    : null;

  let tenancy: TenancyDirectory | null = null;
  if (env.TENANCY_FIXTURE_JSON) {
    try {
      tenancy = parseTenancyFixture(env.TENANCY_FIXTURE_JSON);
    } catch {
      // A malformed tenancy fixture is a configuration failure, not a
      // per-request one: leave tenancy null so auth fails closed and
      // health shows tenancy unconfigured.
      tenancy = null;
    }
  }

  return {
    version: HOSTED_VERSION,
    rp,
    tenancy,
    authConfigured: rp !== null && tenancy !== null,
    serviceAuthConfigured: tenancy !== null,
    stamp: parseStampConfig(env),
    github: parseGitHubAppConfig(env),
    webhookSecret: env.GITHUB_WEBHOOK_SECRET && env.GITHUB_WEBHOOK_SECRET !== "" ? env.GITHUB_WEBHOOK_SECRET : null,
    ingestionRepos: parseIngestionRepos(env.INGESTION_REPOS),
    directRead: parseDirectReadConfig(env),
    queueLibsqlFile: parseQueueLibsqlFile(env.QUEUE_LIBSQL_FILE),
    queueLibsqlUrl: parseQueueLibsqlUrl(env.QUEUE_LIBSQL_URL),
    queueLibsqlAuthToken: parseQueueLibsqlUrl(env.QUEUE_LIBSQL_AUTH_TOKEN),
    kvLibsqlUrl: parseQueueLibsqlUrl(env.KV_LIBSQL_URL),
    kvLibsqlAuthToken: parseQueueLibsqlUrl(env.KV_LIBSQL_AUTH_TOKEN),
    bridge: parseBridgeConfig(env),
  };
}

/** The queue libSQL file (#12589 follow-on): a trimmed path, or
 * null when unset/blank — the fixture default. Whitespace-only is
 * unconfigured, not a file. */
function parseQueueLibsqlFile(raw: string | undefined): string | null {
  const trimmed = raw?.trim();
  return trimmed ? trimmed : null;
}

/** The remote queue Turso URL and its token (#14592): trimmed, or
 * null when unset/blank — same shape as the file parse above. The
 * URL's scheme is validated where it is opened (the opener maps
 * libsql:// → https and lets URL parsing fail loudly there). */
function parseQueueLibsqlUrl(raw: string | undefined): string | null {
  const trimmed = raw?.trim();
  return trimmed ? trimmed : null;
}

/** The PM bridge credential (#14373): present only when the secret
 * is set and long enough to be a bridge credential (the bridge's own
 * minimum is 32 characters); anything shorter is treated as unset,
 * so a truncated paste fails closed instead of sending garbage. */
function parseBridgeConfig(env: HostedEnv): { token: string } | null {
  const token = env.BRIDGE_PM_TOKEN?.trim();
  return token && token.length >= 32 ? { token } : null;
}

/** The sweeper's repo scope: comma-separated owner/name entries.
 * A malformed entry is dropped, not fatal — the residue is a
 * smaller scope, and an all-malformed value parses to the empty
 * (unconfigured) scope, whose sweep fails closed like every
 * unconfigured dependency. */
function parseIngestionRepos(raw: string | undefined): string[] {
  if (!raw) return [];
  const out: string[] = [];
  for (const part of raw.split(",")) {
    const trimmed = part.trim();
    if (/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(trimmed) && !out.includes(trimmed)) {
      out.push(trimmed);
    }
  }
  return out;
}

/** Default cap on a direct-read token's lifetime (design §3.3:
 * tokens are expiring; the cap keeps "expiring" short). */
export const DEFAULT_DIRECT_READ_MAX_TTL_SECONDS = 3600;

/** Parse the direct-read configuration (#12592). Every piece is
 * required and any malformed piece nulls the whole: a half-configured
 * mint that guessed a DB id or a key would be a fail-open scope. The
 * metadata and queue DB ids must be distinct — a scope that cannot
 * tell its one allowed DB from the forbidden one is no scope. */
function parseDirectReadConfig(env: HostedEnv): DirectReadConfig | null {
  const metadataDbId = env.METADATA_DB_ID?.trim();
  const queueDbId = env.QUEUE_DB_ID?.trim();
  if (!metadataDbId || !queueDbId || metadataDbId === queueDbId) return null;
  if (!env.DIRECT_READ_SIGNING_KEY_PKCS8 || !env.DIRECT_READ_SIGNING_KEY_ID) return null;
  if (!isValidStampKeyId(env.DIRECT_READ_SIGNING_KEY_ID)) return null;
  const pkcs8 = b64urlToBytesOrNull(env.DIRECT_READ_SIGNING_KEY_PKCS8);
  if (pkcs8 === null || pkcs8.length !== ED25519_PKCS8_BYTES) return null;
  if (!env.DIRECT_READ_VERIFY_KEYS_JSON) return null;
  let verifyKeys: Map<string, Uint8Array>;
  try {
    const parsed: unknown = JSON.parse(env.DIRECT_READ_VERIFY_KEYS_JSON);
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) return null;
    verifyKeys = new Map();
    for (const [keyId, text] of Object.entries(parsed as Record<string, unknown>)) {
      if (!isValidStampKeyId(keyId) || typeof text !== "string") return null;
      const raw = b64urlToBytesOrNull(text);
      if (raw === null || raw.length !== ED25519_PUBLIC_KEY_BYTES) return null;
      verifyKeys.set(keyId, raw);
    }
  } catch {
    return null;
  }
  if (verifyKeys.size === 0) return null;
  let maxTtlSeconds = DEFAULT_DIRECT_READ_MAX_TTL_SECONDS;
  if (env.DIRECT_READ_MAX_TTL_SECONDS !== undefined) {
    if (!/^[0-9]+$/.test(env.DIRECT_READ_MAX_TTL_SECONDS)) return null;
    maxTtlSeconds = Number(env.DIRECT_READ_MAX_TTL_SECONDS);
    if (!Number.isSafeInteger(maxTtlSeconds) || maxTtlSeconds <= 0) return null;
  }
  return {
    metadataDbId,
    queueDbId,
    signing: { keyId: env.DIRECT_READ_SIGNING_KEY_ID, pkcs8 },
    verifyKeys,
    maxTtlSeconds,
  };
}
