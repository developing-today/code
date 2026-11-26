// Bridge authentication: ID-key proof against the binding contract.
//
// A bridge session or HTTPS call is authenticated twice over, like
// every other route in this package: the front door's routed
// binding guard (the caller is the bound account), and a proof that
// the caller holds the ID key the binding record names — the
// current Iroh root key of binding #3808's BindingRecord. The proof
// is an Ed25519 signature by that key over a transcript naming the
// binding, a single-use server nonce, the leg being authenticated,
// and the hash of the body it authorizes:
//
//   id.bridge-auth.v1\n<binding_id>\n<nonce hex>\n<context>\n<body hash hex>
//
// Nonces are minted by the store side (the challenge route, or the
// first WebSocket frame), live five minutes like the binding
// ceremony's challenges, and are consumed on first use — success or
// failure — so a captured proof cannot be replayed. A store with no
// bound key on record refuses every bridge route: fail closed, the
// package's standing rule. Key material is never logged and never
// leaves the key holder; the store keeps only the public key the
// binding already publishes.

import { blake3 } from "./blake3.ts";
import { bytesToHex, hexToBytes, verifyEd25519 } from "./ed25519.ts";
import type { BridgeStore } from "./bridge-store.ts";

export const NONCE_TTL_MS = 5 * 60 * 1000;
export const RESUME_TOKEN_TTL_MS = 24 * 3600 * 1000;

export type AuthContext = "push" | "pull" | "heads" | "conflicts" | "ws" | "export";

export function authTranscript(
  bindingId: string,
  nonceHex: string,
  context: AuthContext,
  bodyHashHex: string,
): Uint8Array {
  return new TextEncoder().encode(`id.bridge-auth.v1\n${bindingId}\n${nonceHex}\n${context}\n${bodyHashHex}`);
}

export function bodyHashHex(body: Uint8Array): string {
  return bytesToHex(blake3(body));
}

function randomHex(bytes: number): string {
  const out = new Uint8Array(bytes);
  crypto.getRandomValues(out);
  return bytesToHex(out);
}

/** Mints a challenge nonce and records it in the store's meta. One
 * live nonce per binding: minting replaces the previous one. */
export function issueNonce(store: BridgeStore, nowMs: number): { nonce: string; expiresAt: number } {
  const nonce = randomHex(32);
  const expiresAt = nowMs + NONCE_TTL_MS;
  store.metaSet("bridge:nonce", JSON.stringify({ nonce, expiresAt }));
  return { nonce, expiresAt };
}

/** Verifies a proof and consumes the nonce either way. Returns true
 * only for a fresh nonce and a signature by the bound key over the
 * exact transcript for this leg and body. */
export async function verifyProof(
  store: BridgeStore,
  args: {
    bindingId: string;
    irohHex: string;
    context: AuthContext;
    body: Uint8Array;
    nonce: string;
    signatureHex: string;
    nowMs: number;
  },
): Promise<boolean> {
  const raw = store.metaGet("bridge:nonce");
  store.metaDelete("bridge:nonce");
  if (!raw) return false;
  let record: { nonce: string; expiresAt: number };
  try {
    record = JSON.parse(raw) as { nonce: string; expiresAt: number };
  } catch {
    return false;
  }
  if (record.nonce !== args.nonce || args.nowMs > record.expiresAt) return false;
  let signature: Uint8Array;
  let key: Uint8Array;
  try {
    signature = hexToBytes(args.signatureHex);
    key = hexToBytes(args.irohHex);
  } catch {
    return false;
  }
  const transcript = authTranscript(args.bindingId, args.nonce, args.context, bodyHashHex(args.body));
  return verifyEd25519(key, signature, transcript);
}

/** Issues a resume token at the end of a completed session. The
 * token is a bearer reference to the session's end state, stored
 * server-side with its binding and expiry; the positions a client
 * presents with it are evidence, never authority — the range
 * exchange re-derives the truth either way. */
export function issueResumeToken(store: BridgeStore, bindingId: string, nowMs: number): string {
  const token = randomHex(32);
  store.metaSet(`bridge:resume:${token}`, JSON.stringify({ bindingId, expiresAt: nowMs + RESUME_TOKEN_TTL_MS }));
  return token;
}

export function checkResumeToken(store: BridgeStore, token: string, bindingId: string, nowMs: number): boolean {
  const raw = store.metaGet(`bridge:resume:${token}`);
  if (!raw) return false;
  try {
    const record = JSON.parse(raw) as { bindingId: string; expiresAt: number };
    return record.bindingId === bindingId && nowMs <= record.expiresAt;
  } catch {
    return false;
  }
}
