import { test } from "node:test";
import assert from "node:assert/strict";

import {
  AdmissionGate,
  AdmissionRejection,
  ScopeBindingAuthorizer,
  StaticSignerDirectory,
  admissionContext,
  type AdmissionContext,
} from "../src/admission-gate.ts";
import { encodeCanonical, type CborValue } from "../../../hosted-primitives/src/canonical-cbor.ts";

// Mirror of crates/id-core/tests/admission_gate.rs: the same
// actor table and the same scenario → rejection-class table, so
// the hosted worker's gate is held to the Rust gate's decisions.
// Payloads use the atlas-chunk-sha256-utf8 profile — the profile
// this build compiles a verifier for.

const DOMAIN = "id/atlas";
const OTHER_DOMAIN = "id/other";
const PROFILE = "atlas-chunk-sha256-utf8";

const text = (value: string): CborValue => ({ kind: "text", value });
const bytes = (value: Uint8Array): CborValue => ({ kind: "bytes", value });
const uint = (value: bigint): CborValue => ({ kind: "uint", value });
const map = (entries: [CborValue, CborValue][]): CborValue => ({
  kind: "map",
  value: entries,
});

const encoder = new TextEncoder();

function toHex(data: Uint8Array): string {
  return [...data].map((b) => b.toString(16).padStart(2, "0")).join("");
}

async function sha256(data: Uint8Array): Promise<Uint8Array> {
  return new Uint8Array(await crypto.subtle.digest("SHA-256", data as BufferSource));
}

interface Actor {
  signerId: string;
  pair: CryptoKeyPair;
  publicRaw: Uint8Array;
}

async function makeActor(signerId: string): Promise<Actor> {
  const pair = (await crypto.subtle.generateKey("Ed25519", true, ["sign", "verify"])) as CryptoKeyPair;
  const publicRaw = new Uint8Array(await crypto.subtle.exportKey("raw", pair.publicKey));
  return { signerId, pair, publicRaw };
}

const alice = await makeActor("alice");
const bob = await makeActor("bob");
const mallory = await makeActor("mallory");

function directory(): StaticSignerDirectory {
  const dir = new StaticSignerDirectory();
  dir.register("alice", alice.publicRaw);
  dir.register("bob", bob.publicRaw);
  return dir;
}

function gate(policy?: { admittedDomains?: Set<string>; maxPayloadBytes?: number }): AdmissionGate {
  return new AdmissionGate(
    policy ?? { admittedDomains: new Set([DOMAIN]), maxPayloadBytes: 64 },
    directory(),
    new ScopeBindingAuthorizer(),
  );
}

function context(principal: string, tenant: string): AdmissionContext {
  return admissionContext(principal, tenant, "admission:write");
}

/** The signature preimage, built independently of the gate's own copy. */
function signaturePreimage(body: CborValue, domain: string): Uint8Array {
  const prefix = encoder.encode("id/signed-body/v1\0");
  const domainBytes = encoder.encode(domain);
  const encoded = encodeCanonical(body);
  const out = new Uint8Array(prefix.length + 4 + domainBytes.length + encoded.length);
  let pos = 0;
  out.set(prefix, pos);
  pos += prefix.length;
  new DataView(out.buffer).setUint32(pos, domainBytes.length, false);
  pos += 4;
  out.set(domainBytes, pos);
  pos += domainBytes.length;
  out.set(encoded, pos);
  return out;
}

interface EnvelopeOptions {
  actor: Actor;
  tenant: string;
  domain: string;
  payload: Uint8Array;
  profile?: string;
  digestOverride?: Uint8Array;
  signWith?: CryptoKey;
  signatureOverride?: Uint8Array;
}

async function buildEnvelope(opts: EnvelopeOptions): Promise<Uint8Array> {
  const digest = opts.digestOverride ?? (await sha256(opts.payload));
  const body = map([
    [text("v"), uint(1n)],
    [text("domain"), text(opts.domain)],
    [text("context"), text(opts.tenant)],
    [text("signer_id"), text(opts.actor.signerId)],
    [text("key"), bytes(opts.actor.publicRaw)],
    [text("suite"), text("ed25519")],
    [
      text("payload_id"),
      map([
        [text("digest"), bytes(digest)],
        [text("profile"), text(opts.profile ?? PROFILE)],
      ]),
    ],
    [text("payload_length"), uint(BigInt(opts.payload.length))],
  ]);
  const signature =
    opts.signatureOverride ??
    new Uint8Array(
      await crypto.subtle.sign(
        "Ed25519",
        opts.signWith ?? opts.actor.pair.privateKey,
        signaturePreimage(body, opts.domain) as BufferSource,
      ),
    );
  return encodeCanonical(
    map([
      [text("body"), body],
      [text("payload"), bytes(opts.payload)],
      [text("signature"), bytes(signature)],
    ]),
  );
}

async function rejectionClass(promise: Promise<unknown>): Promise<string> {
  try {
    await promise;
  } catch (error) {
    assert.ok(error instanceof AdmissionRejection, `expected AdmissionRejection, got ${error}`);
    return error.rejectionClass;
  }
  assert.fail("expected the admission to be rejected");
}

const payload = encoder.encode("hello atlas");

test("valid producer is admitted with a full receipt", async () => {
  const envelope = await buildEnvelope({
    actor: alice,
    tenant: "tenant-a",
    domain: DOMAIN,
    payload,
  });
  const receipt = await gate().admitCanonical(envelope, context("alice", "tenant-a"));
  assert.equal(receipt.signerId, "alice");
  assert.equal(receipt.tenantScope, "tenant-a");
  assert.equal(receipt.payloadLength, payload.length);
  assert.equal(receipt.payloadProfile, PROFILE);
  assert.equal(receipt.payloadDigestHex, toHex(await sha256(payload)));
});

test("tampered payload is refused at the payload stage", async () => {
  const envelope = await buildEnvelope({
    actor: alice,
    tenant: "tenant-a",
    domain: DOMAIN,
    payload: encoder.encode("forged payload"),
    digestOverride: await sha256(payload),
  });
  assert.equal(await rejectionClass(gate().admitCanonical(envelope, context("alice", "tenant-a"))), "payload");
});

test("forged and unsigned envelopes are refused as bad signatures", async () => {
  const forged = await buildEnvelope({
    actor: alice,
    tenant: "tenant-a",
    domain: DOMAIN,
    payload,
    signWith: bob.pair.privateKey,
  });
  assert.equal(await rejectionClass(gate().admitCanonical(forged, context("alice", "tenant-a"))), "bad_signature");

  // Invariant 1: no unsigned-local exception.
  const unsigned = await buildEnvelope({
    actor: alice,
    tenant: "tenant-a",
    domain: DOMAIN,
    payload,
    signatureOverride: new Uint8Array(64),
  });
  assert.equal(await rejectionClass(gate().admitCanonical(unsigned, context("alice", "tenant-a"))), "bad_signature");
});

test("unknown signer and wrong principal fail signer binding", async () => {
  const unknown = await buildEnvelope({
    actor: mallory,
    tenant: "tenant-a",
    domain: DOMAIN,
    payload,
  });
  assert.equal(await rejectionClass(gate().admitCanonical(unknown, context("mallory", "tenant-a"))), "signer_binding");

  const alices = await buildEnvelope({
    actor: alice,
    tenant: "tenant-a",
    domain: DOMAIN,
    payload,
  });
  assert.equal(await rejectionClass(gate().admitCanonical(alices, context("bob", "tenant-a"))), "signer_binding");
});

test("wrong tenant scope is unauthorized", async () => {
  const envelope = await buildEnvelope({
    actor: alice,
    tenant: "tenant-a",
    domain: DOMAIN,
    payload,
  });
  assert.equal(await rejectionClass(gate().admitCanonical(envelope, context("alice", "tenant-b"))), "unauthorized");
});

test("oversized payload and unadmitted domain are policy refusals", async () => {
  const oversized = await buildEnvelope({
    actor: alice,
    tenant: "tenant-a",
    domain: DOMAIN,
    payload: encoder.encode("123456789"),
  });
  const smallGate = gate({ admittedDomains: new Set([DOMAIN]), maxPayloadBytes: 8 });
  assert.equal(await rejectionClass(smallGate.admitCanonical(oversized, context("alice", "tenant-a"))), "policy");

  const otherDomain = await buildEnvelope({
    actor: alice,
    tenant: "tenant-a",
    domain: OTHER_DOMAIN,
    payload,
  });
  assert.equal(await rejectionClass(gate().admitCanonical(otherDomain, context("alice", "tenant-a"))), "policy");
});

test("garbage bytes are malformed; the blake3 profile is unsupported in this build", async () => {
  assert.equal(
    await rejectionClass(gate().admitCanonical(new Uint8Array(24).fill(0xff), context("alice", "tenant-a"))),
    "malformed",
  );

  const blake3 = await buildEnvelope({
    actor: alice,
    tenant: "tenant-a",
    domain: DOMAIN,
    payload,
    profile: "raw-blake3-256",
    digestOverride: new Uint8Array(32),
  });
  assert.equal(await rejectionClass(gate().admitCanonical(blake3, context("alice", "tenant-a"))), "unsupported");
});

test("context fields are validated", () => {
  assert.throws(
    () => admissionContext("", "tenant-a", "admission:write"),
    (error: unknown) => error instanceof AdmissionRejection && error.rejectionClass === "malformed",
  );
  assert.throws(
    () => admissionContext("alice", "tenant a", "admission:write"),
    (error: unknown) => error instanceof AdmissionRejection && error.rejectionClass === "malformed",
  );
  assert.ok(admissionContext("alice", "tenant-a", "x".repeat(255)));
});
