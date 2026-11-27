// Conformance tests for the direct-Turso Metadata read path (#12592;
// design §3.3). The oracle is the prudence rules themselves, asserted
// as exact refusal codes/reasons so a fail-open change names itself:
//
//   1. Direct access is read-only, metadata-DB only, under a scoped,
//      expiring token minted for one tenancy account.
//   2. Never direct: any write, the queue DB, any other DB, any
//      cross-tenant use, and any post-expiry use.
//   3. A direct-read token is a data-plane credential: the service
//      refuses it as authentication (the stamp path never accepts
//      one) — asserted here against the real `authenticate`.

import assert from "node:assert/strict";
import { test } from "node:test";

import { authenticate } from "../src/auth.ts";
import { configFromEnv, type HostedEnv } from "../src/config.ts";
import { inspectDirectReadToken, mintDirectReadToken, type DirectReadConfig } from "../src/direct-read.ts";
import { FailClosed } from "../src/errors.ts";
import { parseJwt } from "../src/jwt.ts";
import { BINDING_A, BINDING_B, makeHarness } from "./helpers.ts";

const METADATA_DB_ID = "metadata-db-1";
const QUEUE_DB_ID = "queue-db-1";
const KEY_ID = "dr-key-1";
const NOW = 1_800_000_000;

function b64url(bytes: Uint8Array): string {
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

interface MintMaterial {
  env: HostedEnv;
  config: DirectReadConfig;
  privateKey: CryptoKey;
}

async function makeMintMaterial(envOverrides?: Partial<HostedEnv>): Promise<MintMaterial> {
  const pair = await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"]);
  const pkcs8 = new Uint8Array(await crypto.subtle.exportKey("pkcs8", pair.privateKey));
  const rawPublic = new Uint8Array(await crypto.subtle.exportKey("raw", pair.publicKey));
  const env: HostedEnv = {
    METADATA_DB_ID,
    QUEUE_DB_ID,
    DIRECT_READ_SIGNING_KEY_PKCS8: b64url(pkcs8),
    DIRECT_READ_SIGNING_KEY_ID: KEY_ID,
    DIRECT_READ_VERIFY_KEYS_JSON: JSON.stringify({ [KEY_ID]: b64url(rawPublic) }),
    ...envOverrides,
  };
  const config = configFromEnv(env).directRead;
  assert.ok(config !== null, "fixture env must parse to a direct-read config");
  return { env, config, privateKey: pair.privateKey };
}

/** Sign an arbitrary payload with the fixture key (forgery bench). */
async function signRaw(
  privateKey: CryptoKey,
  payload: Record<string, unknown>,
  header?: Record<string, unknown>,
): Promise<string> {
  const enc = new TextEncoder();
  const head = b64url(enc.encode(JSON.stringify(header ?? { alg: "EdDSA", typ: "JWT", kid: KEY_ID })));
  const body = b64url(enc.encode(JSON.stringify(payload)));
  const sig = new Uint8Array(await crypto.subtle.sign({ name: "Ed25519" }, privateKey, enc.encode(`${head}.${body}`)));
  return `${head}.${body}.${b64url(sig)}`;
}

function failClosedWith(code: string) {
  return (e: unknown) => e instanceof FailClosed && e.code === code;
}

test("config: a complete direct-read env parses; gaps fail closed to null", async () => {
  const { config } = await makeMintMaterial();
  assert.equal(config.metadataDbId, METADATA_DB_ID);
  assert.equal(config.queueDbId, QUEUE_DB_ID);

  const noQueue = await makeMintMaterial();
  assert.equal(configFromEnv({ ...noQueue.env, QUEUE_DB_ID: undefined }).directRead, null);
  // The metadata and queue DBs must be distinct, or the scope is a lie.
  assert.equal(configFromEnv({ ...noQueue.env, QUEUE_DB_ID: METADATA_DB_ID }).directRead, null);
  assert.equal(configFromEnv({ ...noQueue.env, DIRECT_READ_SIGNING_KEY_PKCS8: "AAAA" }).directRead, null);
  assert.equal(configFromEnv({}).directRead, null);
});

test("mint: the token carries read-only access, the metadata DB, the binding, an expiry", async () => {
  const { config } = await makeMintMaterial();
  const token = await mintDirectReadToken(config, {
    bindingId: BINDING_A,
    ttlSeconds: 600,
    now: NOW,
  });
  const { header, payload } = parseJwt(token);
  assert.equal(header.alg, "EdDSA");
  assert.equal(header.kid, KEY_ID);
  assert.equal(payload.a, "ro");
  assert.equal(payload.id, METADATA_DB_ID);
  assert.equal(payload.binding_id, BINDING_A);
  assert.equal(payload.iat, NOW);
  assert.equal(payload.exp, NOW + 600);
});

test("inspect: a minted token is in scope for its account until its expiry", async () => {
  const { config } = await makeMintMaterial();
  const token = await mintDirectReadToken(config, {
    bindingId: BINDING_A,
    ttlSeconds: 600,
    now: NOW,
  });
  const result = await inspectDirectReadToken(token, config, NOW + 599);
  assert.deepEqual(result, { status: "ok", bindingId: BINDING_A, expiresAt: NOW + 600 });
});

test("mint: refuses unconfigured, malformed binding, and out-of-range TTLs", async () => {
  const { config } = await makeMintMaterial({ DIRECT_READ_MAX_TTL_SECONDS: "600" });
  await assert.rejects(
    mintDirectReadToken(null, { bindingId: BINDING_A, ttlSeconds: 60, now: NOW }),
    failClosedWith("direct_read_not_configured"),
  );
  await assert.rejects(
    mintDirectReadToken(config, { bindingId: "not-a-binding", ttlSeconds: 60, now: NOW }),
    failClosedWith("invalid_binding_id"),
  );
  await assert.rejects(
    mintDirectReadToken(config, { bindingId: BINDING_A, ttlSeconds: 0, now: NOW }),
    failClosedWith("invalid_ttl"),
  );
  await assert.rejects(
    mintDirectReadToken(config, { bindingId: BINDING_A, ttlSeconds: 601, now: NOW }),
    failClosedWith("invalid_ttl"),
  );
});

test("inspect: a write-capable token is refused — direct access is read-only", async () => {
  const { config, privateKey } = await makeMintMaterial();
  const forged = await signRaw(privateKey, {
    a: "rw",
    id: METADATA_DB_ID,
    binding_id: BINDING_A,
    iat: NOW,
    exp: NOW + 600,
  });
  const result = await inspectDirectReadToken(forged, config, NOW);
  assert.deepEqual(result, { status: "refused", reason: "not_read_only" });
});

test("inspect: the queue DB and any other DB are refused — metadata only", async () => {
  const { config, privateKey } = await makeMintMaterial();
  const base = { a: "ro", binding_id: BINDING_A, iat: NOW, exp: NOW + 600 };
  const queueToken = await signRaw(privateKey, { ...base, id: QUEUE_DB_ID });
  assert.deepEqual(await inspectDirectReadToken(queueToken, config, NOW), {
    status: "refused",
    reason: "queue_database",
  });
  const otherToken = await signRaw(privateKey, { ...base, id: "some-other-db" });
  assert.deepEqual(await inspectDirectReadToken(otherToken, config, NOW), {
    status: "refused",
    reason: "wrong_database",
  });
});

test("inspect: expired and expiry-less tokens are refused", async () => {
  const { config, privateKey } = await makeMintMaterial();
  const token = await mintDirectReadToken(config, {
    bindingId: BINDING_A,
    ttlSeconds: 60,
    now: NOW,
  });
  assert.deepEqual(await inspectDirectReadToken(token, config, NOW + 60), {
    status: "refused",
    reason: "expired",
  });
  const noExpiry = await signRaw(privateKey, {
    a: "ro",
    id: METADATA_DB_ID,
    binding_id: BINDING_A,
    iat: NOW,
  });
  assert.deepEqual(await inspectDirectReadToken(noExpiry, config, NOW), {
    status: "refused",
    reason: "missing_expiry",
  });
});

test("inspect: tampered, unknown-key, and cross-account-shaped tokens are refused", async () => {
  const { config, privateKey } = await makeMintMaterial();
  const token = await mintDirectReadToken(config, {
    bindingId: BINDING_A,
    ttlSeconds: 600,
    now: NOW,
  });
  // Rebind the payload to another account without re-signing.
  const [head, , sig] = token.split(".");
  const rebound = b64url(
    new TextEncoder().encode(
      JSON.stringify({
        a: "ro",
        id: METADATA_DB_ID,
        binding_id: BINDING_B,
        iat: NOW,
        exp: NOW + 600,
      }),
    ),
  );
  assert.deepEqual(await inspectDirectReadToken(`${head}.${rebound}.${sig}`, config, NOW), {
    status: "refused",
    reason: "bad_signature",
  });
  const unknownKey = await signRaw(
    privateKey,
    { a: "ro", id: METADATA_DB_ID, binding_id: BINDING_A, iat: NOW, exp: NOW + 600 },
    { alg: "EdDSA", typ: "JWT", kid: "no-such-key" },
  );
  assert.deepEqual(await inspectDirectReadToken(unknownKey, config, NOW), {
    status: "refused",
    reason: "unknown_key",
  });
  assert.deepEqual(await inspectDirectReadToken("not-a-jwt", config, NOW), {
    status: "refused",
    reason: "malformed_token",
  });
});

test("service: a minted direct-read token is refused as service authentication (§3.3 rule 3)", async () => {
  const { config } = await makeMintMaterial();
  const token = await mintDirectReadToken(config, {
    bindingId: BINDING_A,
    ttlSeconds: 600,
    now: NOW,
  });
  const harness = await makeHarness();
  const req = new Request("https://fleet-hosted.test/stamp", {
    headers: { authorization: `Bearer ${token}` },
  });
  await assert.rejects(authenticate(req, harness.deps), failClosedWith("data_plane_credential_refused"));
});
