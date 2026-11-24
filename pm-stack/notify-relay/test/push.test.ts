// Oracles for the relay's Web Push sender (#8569):
//  - RFC 8291: only the subscribed browser can read the payload —
//    proven by decrypting with the receiver's private key and auth
//    secret, through the RFC's own HKDF chain.
//  - RFC 8292: the VAPID JWT is signed by the keypair's private
//    half, names the push service's origin as audience, and
//    carries the production public key in `k=`.
//  - Delivery accounting: 2xx = sent, 404/410 = gone, anything
//    else (or a throw) = failed — and a failure never throws.
import { describe, expect, test } from "bun:test";
import {
  b64uDecode,
  b64uEncode,
  encryptPayload,
  sendPush,
  vapidAuthorization,
  VAPID_PUBLIC_KEY,
  type PushTarget,
} from "../src/push.ts";

const enc = new TextEncoder();
const concat = (...parts: Uint8Array[]) => {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
};

async function hkdf(salt: Uint8Array, ikm: Uint8Array, info: Uint8Array, length: number): Promise<Uint8Array> {
  const key = await crypto.subtle.importKey("raw", ikm, "HKDF", false, ["deriveBits"]);
  return new Uint8Array(await crypto.subtle.deriveBits({ name: "HKDF", hash: "SHA-256", salt, info }, key, length * 8));
}

/** A fresh receiver: an ECDH keypair + auth secret, as a browser
 * would hand to the subscription store. */
async function makeReceiver() {
  const pair = (await crypto.subtle.generateKey({ name: "ECDH", namedCurve: "P-256" }, true, [
    "deriveBits",
  ])) as CryptoKeyPair;
  const publicRaw = new Uint8Array(await crypto.subtle.exportKey("raw", pair.publicKey));
  const auth = crypto.getRandomValues(new Uint8Array(16));
  const target: PushTarget = {
    endpoint: "https://fcm.googleapis.com/fcm/send/fixture",
    p256dh: b64uEncode(publicRaw),
    auth: b64uEncode(auth),
  };
  return { pair, publicRaw, auth, target };
}

/** Decrypt an aes128gcm body the way the browser's push service
 * client would (RFC 8291 §3.3 in reverse). */
async function decrypt(receiver: Awaited<ReturnType<typeof makeReceiver>>, body: Uint8Array): Promise<Uint8Array> {
  const salt = body.subarray(0, 16);
  const idLen = body[20];
  const senderPublic = body.subarray(21, 21 + idLen);
  const sealed = body.subarray(21 + idLen);
  const senderKey = await crypto.subtle.importKey(
    "raw",
    senderPublic,
    { name: "ECDH", namedCurve: "P-256" },
    false,
    [],
  );
  const shared = new Uint8Array(
    await crypto.subtle.deriveBits({ name: "ECDH", public: senderKey }, receiver.pair.privateKey, 256),
  );
  const ikm = await hkdf(
    receiver.auth,
    shared,
    concat(enc.encode("WebPush: info\0"), receiver.publicRaw, senderPublic),
    32,
  );
  const cek = await hkdf(salt, ikm, enc.encode("Content-Encoding: aes128gcm\0"), 16);
  const nonce = await hkdf(salt, ikm, enc.encode("Content-Encoding: nonce\0"), 12);
  const key = await crypto.subtle.importKey("raw", cek, "AES-GCM", false, ["decrypt"]);
  const plain = new Uint8Array(await crypto.subtle.decrypt({ name: "AES-GCM", iv: nonce }, key, sealed));
  return plain;
}

describe("RFC 8291 payload encryption", () => {
  test("only the subscribed browser can read it; the last-record delimiter rides along", async () => {
    const receiver = await makeReceiver();
    const message = enc.encode(JSON.stringify({ title: "PM stack is down" }));
    const body = await encryptPayload(receiver.target, message);
    const plain = await decrypt(receiver, body);
    expect(plain[plain.length - 1]).toBe(2);
    expect(new TextDecoder().decode(plain.subarray(0, plain.length - 1))).toBe(new TextDecoder().decode(message));
  });

  test("a different auth secret cannot read it", async () => {
    const receiver = await makeReceiver();
    const body = await encryptPayload(receiver.target, enc.encode("secret"));
    const impostor = { ...receiver, auth: crypto.getRandomValues(new Uint8Array(16)) };
    await expect(decrypt(impostor, body)).rejects.toThrow();
  });
});

describe("RFC 8292 VAPID signing", () => {
  test("the JWT verifies against the keypair and names the push origin", async () => {
    const pair = (await crypto.subtle.generateKey({ name: "ECDSA", namedCurve: "P-256" }, true, [
      "sign",
      "verify",
    ])) as CryptoKeyPair;
    const now = 1_700_000_000_000;
    const header = await vapidAuthorization("https://fcm.googleapis.com/fcm/send/x", pair.privateKey, now);
    expect(header.startsWith("vapid t=")).toBe(true);
    expect(header.endsWith(`, k=${VAPID_PUBLIC_KEY}`)).toBe(true);
    const jwt = header.slice("vapid t=".length, header.indexOf(", k="));
    const [head, bodyPart, sig] = jwt.split(".");
    const claims = JSON.parse(new TextDecoder().decode(b64uDecode(bodyPart)));
    expect(claims.aud).toBe("https://fcm.googleapis.com");
    expect(claims.exp).toBe(Math.floor(now / 1000) + 12 * 3600);
    expect(claims.sub).toBe("mailto:drewrypope@gmail.com");
    const ok = await crypto.subtle.verify(
      { name: "ECDSA", hash: "SHA-256" },
      pair.publicKey,
      b64uDecode(sig),
      enc.encode(`${head}.${bodyPart}`),
    );
    expect(ok).toBe(true);
  });
});

describe("delivery accounting", () => {
  /** A throwaway VAPID keypair for the sender under test (the
   * production pair is a secret and never appears in tests;
   * WebCrypto validates the JWK as a set, so the pair must be
   * consistent — private scalar + its own public point). */
  const testKeyPair = async (): Promise<{ vapidPrivateKey: string; vapidPublicKey: string }> => {
    const pair = (await crypto.subtle.generateKey({ name: "ECDSA", namedCurve: "P-256" }, true, [
      "sign",
    ])) as CryptoKeyPair;
    const jwk = await crypto.subtle.exportKey("jwk", pair.privateKey);
    const raw = new Uint8Array(await crypto.subtle.exportKey("raw", pair.publicKey));
    return { vapidPrivateKey: jwk.d as string, vapidPublicKey: b64uEncode(raw) };
  };

  test("2xx counts sent and the posted body is the sealed payload", async () => {
    const receiver = await makeReceiver();
    let posted: { url: string; body: Uint8Array; headers: Record<string, string> } | null = null;
    const summary = await sendPush(
      [receiver.target],
      { title: "down" },
      {
        ...(await testKeyPair()),
        poster: async (url, init) => {
          posted = { url, body: init.body, headers: init.headers };
          return { status: 201 };
        },
      },
    );
    expect(summary).toMatchObject({ sent: 1, gone: 0, failed: 0 });
    expect(posted!.headers["Content-Encoding"]).toBe("aes128gcm");
    expect(posted!.headers.Authorization.startsWith("vapid t=")).toBe(true);
    const plain = await decrypt(receiver, posted!.body);
    expect(new TextDecoder().decode(plain.subarray(0, plain.length - 1))).toBe(JSON.stringify({ title: "down" }));
  });

  test("410 counts gone; a 500 and a throw count failed; sendPush never throws", async () => {
    const a = (await makeReceiver()).target;
    const b = (await makeReceiver()).target;
    const c = (await makeReceiver()).target;
    const statuses = [410, 500];
    const summary = await sendPush(
      [a, b, c],
      { title: "x" },
      {
        ...(await testKeyPair()),
        poster: async () => {
          const status = statuses.shift();
          if (status === undefined) throw new Error("network down");
          return { status };
        },
      },
    );
    expect(summary.sent).toBe(0);
    expect(summary.gone).toBe(1);
    expect(summary.failed).toBe(2);
  });
});
