// Ed25519 over WebCrypto — the one crypto seam of the bridge.
// workerd and Node both provide Ed25519 in SubtleCrypto (verified
// against the RFC 8032 test vectors in test/bridge-event.test.ts).
// p2panda's verifier is ed25519-dalek's verify_strict; the practical
// difference (small-order public-key components) cannot arise for
// keys produced by the binding ceremony or any real signer, and any
// key whose signature does not verify is refused either way.

import { base64UrlToBytes, bytesToHex, hexToBytes } from "../../../hosted-primitives/src/encoding.ts";

// Single-implementation re-exports (G2 of #9085): the canonical hex
// helpers live in hosted-primitives; existing import sites keep working.
export { bytesToHex, hexToBytes };

const keyCache = new Map<string, Promise<CryptoKey>>();

function importVerifyKey(publicRaw: Uint8Array): Promise<CryptoKey> {
  const cacheKey = bytesToHex(publicRaw);
  let cached = keyCache.get(cacheKey);
  if (!cached) {
    cached = crypto.subtle.importKey("raw", publicRaw as BufferSource, "Ed25519", false, ["verify"]);
    keyCache.set(cacheKey, cached);
  }
  return cached;
}

/** True iff `signature` is `publicRaw`'s signature over `message`.
 * Any error (an undecodable key, a malformed signature) is false:
 * verification is a refusal-shaped question. */
export async function verifyEd25519(
  publicRaw: Uint8Array,
  signature: Uint8Array,
  message: Uint8Array,
): Promise<boolean> {
  if (publicRaw.length !== 32 || signature.length !== 64) return false;
  try {
    const key = await importVerifyKey(publicRaw);
    return await crypto.subtle.verify("Ed25519", key, signature as BufferSource, message as BufferSource);
  } catch {
    return false;
  }
}

/** An ID key (the Iroh root key is an Ed25519 key) imported from its
 * 32-byte seed — p2panda's SigningKey::from_bytes. The seed is a
 * secret: it is only ever held by the key's owner (a native node, or
 * a test), never by the hosted store. */
export interface IdKey {
  publicRaw: Uint8Array;
  publicHex: string;
  sign(message: Uint8Array): Promise<Uint8Array>;
}

const PKCS8_SEED_PREFIX = new Uint8Array([
  0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
]);

export async function importIdKey(seed: Uint8Array): Promise<IdKey> {
  if (seed.length !== 32) throw new Error("an Ed25519 seed is 32 bytes");
  const pkcs8 = new Uint8Array(PKCS8_SEED_PREFIX.length + 32);
  pkcs8.set(PKCS8_SEED_PREFIX);
  pkcs8.set(seed, PKCS8_SEED_PREFIX.length);
  const privateKey = await crypto.subtle.importKey("pkcs8", pkcs8 as BufferSource, "Ed25519", true, ["sign"]);
  const jwk = await crypto.subtle.exportKey("jwk", privateKey);
  if (!jwk.x) throw new Error("exported key has no public component");
  const publicRaw = base64UrlToBytes(jwk.x);
  return {
    publicRaw,
    publicHex: bytesToHex(publicRaw),
    async sign(message: Uint8Array): Promise<Uint8Array> {
      const sig = await crypto.subtle.sign("Ed25519", privateKey, message as BufferSource);
      return new Uint8Array(sig);
    },
  };
}
