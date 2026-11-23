// Stand-ins the tests share: a browser that can open a push message,
// and a sign-in issuer that mints tokens.
import { expect } from "bun:test";

export const b64u = (b: ArrayBuffer | Uint8Array) => Buffer.from(b as ArrayBuffer).toString("base64url");
export const raw = (s: string) => new Uint8Array(Buffer.from(s, "base64url"));

export async function browser(host = "fcm.googleapis.com") {
  const pair = (await crypto.subtle.generateKey({ name: "ECDH", namedCurve: "P-256" }, true, [
    "deriveBits",
  ])) as CryptoKeyPair;
  const auth = crypto.getRandomValues(new Uint8Array(16));
  const p256dh = new Uint8Array(await crypto.subtle.exportKey("raw", pair.publicKey));
  const hkdf = async (salt: Uint8Array, ikm: Uint8Array, info: Uint8Array, n: number) =>
    new Uint8Array(
      await crypto.subtle.deriveBits(
        { name: "HKDF", hash: "SHA-256", salt, info },
        await crypto.subtle.importKey("raw", ikm, "HKDF", false, ["deriveBits"]),
        n * 8,
      ),
    );
  const cat = (...p: Uint8Array[]) => new Uint8Array(p.flatMap((x) => [...x]));
  const t = new TextEncoder();
  return {
    subscription: { endpoint: `https://${host}/fcm/send/abc123`, keys: { p256dh: b64u(p256dh), auth: b64u(auth) } },
    /** RFC 8291 section 3.4 and RFC 8188: what a user agent does with the body. */
    async open(body: Uint8Array): Promise<string> {
      const salt = body.slice(0, 16);
      const rs = new DataView(body.buffer, body.byteOffset).getUint32(16);
      const idlen = body[20];
      const sender = body.slice(21, 21 + idlen);
      const sealed = body.slice(21 + idlen);
      expect(rs).toBe(4096);
      expect(idlen).toBe(65);
      const shared = new Uint8Array(
        await crypto.subtle.deriveBits(
          {
            name: "ECDH",
            public: await crypto.subtle.importKey("raw", sender, { name: "ECDH", namedCurve: "P-256" }, false, []),
          },
          pair.privateKey,
          256,
        ),
      );
      const ikm = await hkdf(auth, shared, cat(t.encode("WebPush: info\0"), p256dh, sender), 32);
      const cek = await hkdf(salt, ikm, t.encode("Content-Encoding: aes128gcm\0"), 16);
      const nonce = await hkdf(salt, ikm, t.encode("Content-Encoding: nonce\0"), 12);
      const plain = new Uint8Array(
        await crypto.subtle.decrypt(
          { name: "AES-GCM", iv: nonce },
          await crypto.subtle.importKey("raw", cek, "AES-GCM", false, ["decrypt"]),
          sealed,
        ),
      );
      expect(plain[plain.length - 1]).toBe(2); // the last-record delimiter
      return new TextDecoder().decode(plain.slice(0, -1));
    },
  };
}

export interface Signer {
  jwk: JsonWebKey & { kid: string };
  sign(claims: Record<string, unknown>): Promise<string>;
}

/** An issuer's signing key (RS256), and tokens signed with it. */
export async function signer(kid: string): Promise<Signer> {
  const pair = (await crypto.subtle.generateKey(
    { name: "RSASSA-PKCS1-v1_5", modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: "SHA-256" },
    true,
    ["sign", "verify"],
  )) as CryptoKeyPair;
  const jwk = { ...(await crypto.subtle.exportKey("jwk", pair.publicKey)), kid };
  const part = (value: unknown) => b64u(new TextEncoder().encode(JSON.stringify(value)));
  return {
    jwk,
    async sign(claims) {
      const signed = `${part({ alg: "RS256", kid, typ: "JWT" })}.${part(claims)}`;
      const sig = await crypto.subtle.sign("RSASSA-PKCS1-v1_5", pair.privateKey, new TextEncoder().encode(signed));
      return `${signed}.${b64u(sig)}`;
    },
  };
}

/** A stand-in for the platform's fetch that behaves as the Worker
 * runtime's does in one respect bun's does not: it refuses to be called
 * as a method of some other object. There, `this.fetchImpl(url)` with
 * the platform's fetch kept in `fetchImpl` throws "Illegal invocation",
 * and under bun it works, so a test that hands a class a plain function
 * cannot see the fault. Hand it this. */
export function asThePlatformFetch<A extends unknown[], R>(answer: (...args: A) => R): (...args: A) => R {
  return function (this: unknown, ...args: A): R {
    if (this !== undefined && this !== globalThis) {
      throw new TypeError("Illegal invocation: function called with incorrect `this` reference.");
    }
    return answer(...args);
  };
}
