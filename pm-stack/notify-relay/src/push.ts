// Web Push sending for the pm-notify relay (#8569 Phase 2, design
// record doc/2026-11-17_design_pm8569_notifications_phase2.md §2/§4).
//
// Same construction as pm-stack/console/src/push.ts (RFC 8291
// aes128gcm payload encryption + RFC 8292 VAPID signing, WebCrypto
// only — the `web-push` package the laptop uses is Node-only), with
// one custody difference the design mandates: the keypair is NOT
// generated at the edge. It is the aggregator's existing VAPID
// keypair, so subscriptions registered with the laptop keep working
// when the edge sends. The public key ships here (it is public by
// design — the same constant as pm-stack/aggregator/src/push.ts);
// the private key arrives as the Worker secret VAPID_PRIVATE_KEY,
// in the base64url raw-scalar form the laptop fetches from
// Bitwarden by name `pm-aggregator.vapid-private-key`.

/** The aggregator's VAPID public key (uncompressed P-256 point,
 * base64url). Public by design; identical to the key the laptop's
 * subscriptions were registered against. */
export const VAPID_PUBLIC_KEY =
  "BNBCc8IjMrerQY8aGviwq71OoJwf6BCnVOJd0UMAnSwRuXJ8nlIkfh2TFEmAPyPGKU2OOpDEnqdhIGTcBv4Jb2E";
export const VAPID_SUBJECT = "mailto:drewrypope@gmail.com";

const enc = new TextEncoder();

export function b64uEncode(bytes: Uint8Array | ArrayBuffer): string {
  const view = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  let binary = "";
  for (const byte of view) binary += String.fromCharCode(byte);
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
}

export function b64uDecode(text: string): Uint8Array {
  const padded = text.replaceAll("-", "+").replaceAll("_", "/") + "=".repeat((4 - (text.length % 4)) % 4);
  const binary = atob(padded);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i);
  return out;
}

const concat = (...parts: Uint8Array[]) => {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
};

export interface PushTarget {
  endpoint: string;
  p256dh: string;
  auth: string;
}

async function hkdf(salt: Uint8Array, ikm: Uint8Array, info: Uint8Array, length: number): Promise<Uint8Array> {
  const key = await crypto.subtle.importKey("raw", ikm, "HKDF", false, ["deriveBits"]);
  return new Uint8Array(await crypto.subtle.deriveBits({ name: "HKDF", hash: "SHA-256", salt, info }, key, length * 8));
}

/** Encrypt one message for one subscription (RFC 8291, aes128gcm). */
export async function encryptPayload(target: PushTarget, plaintext: Uint8Array): Promise<Uint8Array> {
  const uaPublic = b64uDecode(target.p256dh);
  const authSecret = b64uDecode(target.auth);
  const local = (await crypto.subtle.generateKey({ name: "ECDH", namedCurve: "P-256" }, true, [
    "deriveBits",
  ])) as CryptoKeyPair;
  const localPublic = new Uint8Array(await crypto.subtle.exportKey("raw", local.publicKey));
  const peer = await crypto.subtle.importKey("raw", uaPublic, { name: "ECDH", namedCurve: "P-256" }, false, []);
  const shared = new Uint8Array(await crypto.subtle.deriveBits({ name: "ECDH", public: peer }, local.privateKey, 256));
  const ikm = await hkdf(authSecret, shared, concat(enc.encode("WebPush: info\0"), uaPublic, localPublic), 32);
  const salt = crypto.getRandomValues(new Uint8Array(16));
  const cek = await hkdf(salt, ikm, enc.encode("Content-Encoding: aes128gcm\0"), 16);
  const nonce = await hkdf(salt, ikm, enc.encode("Content-Encoding: nonce\0"), 12);
  const key = await crypto.subtle.importKey("raw", cek, "AES-GCM", false, ["encrypt"]);
  // One record: the message, then the delimiter that marks the last record.
  const sealed = new Uint8Array(
    await crypto.subtle.encrypt({ name: "AES-GCM", iv: nonce }, key, concat(plaintext, new Uint8Array([2]))),
  );
  const header = new Uint8Array(16 + 4 + 1 + localPublic.length);
  header.set(salt, 0);
  new DataView(header.buffer).setUint32(16, 4096);
  header[20] = localPublic.length;
  header.set(localPublic, 21);
  return concat(header, sealed);
}

/** Import the VAPID signing key from the raw private scalar (the
 * Worker secret's form). The JWK's x/y come from the public point —
 * the pair is only usable together, and a mismatched secret simply
 * fails at the push service, never silently elsewhere. */
export async function importVapidSigner(privateKeyB64u: string): Promise<CryptoKey> {
  const point = b64uDecode(VAPID_PUBLIC_KEY);
  if (point.length !== 65 || point[0] !== 4) throw new Error("VAPID public key is not an uncompressed P-256 point");
  const jwk: JsonWebKey = {
    kty: "EC",
    crv: "P-256",
    x: b64uEncode(point.subarray(1, 33)),
    y: b64uEncode(point.subarray(33, 65)),
    d: privateKeyB64u,
  };
  return crypto.subtle.importKey("jwk", jwk, { name: "ECDSA", namedCurve: "P-256" }, false, ["sign"]);
}

/** The RFC 8292 Authorization header value for one push endpoint. */
export async function vapidAuthorization(endpoint: string, signer: CryptoKey, now: number): Promise<string> {
  const head = b64uEncode(enc.encode(JSON.stringify({ typ: "JWT", alg: "ES256" })));
  const body = b64uEncode(
    enc.encode(
      JSON.stringify({
        aud: new URL(endpoint).origin,
        exp: Math.floor(now / 1000) + 12 * 3600,
        sub: VAPID_SUBJECT,
      }),
    ),
  );
  const sig = await crypto.subtle.sign({ name: "ECDSA", hash: "SHA-256" }, signer, enc.encode(`${head}.${body}`));
  return `vapid t=${head}.${body}.${b64uEncode(sig)}, k=${VAPID_PUBLIC_KEY}`;
}

export type Poster = (
  url: string,
  init: { method: string; headers: Record<string, string>; body: Uint8Array; signal?: AbortSignal },
) => Promise<{ status: number }>;

export interface SendSummary {
  sent: number;
  /** Endpoints the push service reports gone (404/410). The relay's
   * snapshot is a mirror — the caller records the count but never
   * edits the mirror locally (design invariant 1). */
  gone: number;
  failed: number;
  refused: { host: string; status: number }[];
}

export interface SendOptions {
  vapidPrivateKey: string;
  poster?: Poster;
  now?: () => number;
  /** Matches the laptop's incident sends (aggregator push.ts). */
  ttlSeconds?: number;
}

/** Send one payload to every target. Never throws: per-target
 * failures are counted in the summary. */
export async function sendPush(targets: PushTarget[], payload: unknown, opts: SendOptions): Promise<SendSummary> {
  const poster: Poster = opts.poster ?? ((url, init) => fetch(url, init as RequestInit));
  const now = opts.now ?? Date.now;
  const signer = await importVapidSigner(opts.vapidPrivateKey);
  const body = enc.encode(JSON.stringify(payload));
  const summary: SendSummary = { sent: 0, gone: 0, failed: 0, refused: [] };
  for (const target of targets) {
    const host = (() => {
      try {
        return new URL(target.endpoint).hostname;
      } catch {
        return "invalid-endpoint";
      }
    })();
    try {
      const authorization = await vapidAuthorization(target.endpoint, signer, now());
      const res = await poster(target.endpoint, {
        method: "POST",
        headers: {
          Authorization: authorization,
          "Content-Encoding": "aes128gcm",
          "Content-Type": "application/octet-stream",
          TTL: String(opts.ttlSeconds ?? 3600),
          Urgency: "high",
        },
        body: await encryptPayload(target, body),
        signal: AbortSignal.timeout(15_000),
      });
      if (res.status === 404 || res.status === 410) summary.gone++;
      else if (res.status >= 200 && res.status < 300) summary.sent++;
      else {
        summary.failed++;
        summary.refused.push({ host, status: res.status });
      }
    } catch {
      summary.failed++;
      summary.refused.push({ host, status: 0 });
    }
  }
  summary.refused = summary.refused.slice(0, 5);
  return summary;
}
