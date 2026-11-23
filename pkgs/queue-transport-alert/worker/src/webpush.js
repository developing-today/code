// Standard Web Push from the Worker (code#6369): a real phone
// notification, alongside the Telegram message.
//
// Pushes carry no payload, so nothing needs RFC 8291 encryption: the
// page's service worker wakes, reads /status.json (whose `last_push`
// holds the title and text) and shows the notification. Each push is
// authorised with a VAPID JWT (RFC 8292). The VAPID key pair is made
// once inside the Durable Object and kept in its storage, so the
// private key never leaves Cloudflare and needs no secret management.

const enc = new TextEncoder();

export function b64url(bytes) {
  let s = "";
  for (const b of new Uint8Array(bytes)) s += String.fromCharCode(b);
  return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** Push services browsers use; anything else is refused at subscribe
 * time so the Worker only ever POSTs to a real push service. */
const PUSH_HOSTS = [
  /^fcm\.googleapis\.com$/,
  /^android\.googleapis\.com$/,
  /^updates\.push\.services\.mozilla\.com$/,
  /^([a-z0-9-]+\.)*push\.apple\.com$/,
  /^([a-z0-9-]+\.)*notify\.windows\.com$/,
];

export function validEndpoint(value) {
  if (typeof value !== "string" || value.length > 1024) return null;
  let url;
  try {
    url = new URL(value);
  } catch {
    return null;
  }
  if (url.protocol !== "https:" || url.username || url.password || url.port) return null;
  return PUSH_HOSTS.some((re) => re.test(url.hostname)) ? url.toString() : null;
}

/** The stored VAPID pair, made on first use. */
export async function vapidKeys(storage) {
  const have = await storage.get("vapid");
  if (have) return have;
  const pair = await crypto.subtle.generateKey({ name: "ECDSA", namedCurve: "P-256" }, true, [
    "sign",
    "verify",
  ]);
  const keys = {
    publicKey: b64url(await crypto.subtle.exportKey("raw", pair.publicKey)),
    privateJwk: await crypto.subtle.exportKey("jwk", pair.privateKey),
  };
  await storage.put("vapid", keys);
  return keys;
}

export async function vapidAuth(endpoint, keys, subject, nowSec) {
  const header = b64url(enc.encode(JSON.stringify({ typ: "JWT", alg: "ES256" })));
  const claims = b64url(
    enc.encode(JSON.stringify({ aud: new URL(endpoint).origin, exp: nowSec + 12 * 3600, sub: subject })),
  );
  const key = await crypto.subtle.importKey("jwk", keys.privateJwk, { name: "ECDSA", namedCurve: "P-256" }, false, [
    "sign",
  ]);
  // WebCrypto ECDSA signatures are already raw r||s, as JWS ES256 wants.
  const sig = await crypto.subtle.sign({ name: "ECDSA", hash: "SHA-256" }, key, enc.encode(`${header}.${claims}`));
  return `vapid t=${header}.${claims}.${b64url(sig)}, k=${keys.publicKey}`;
}

/**
 * Sends one payloadless push to each subscription. Returns
 * {sent, gone, failed:[detail]}; `gone` lists endpoints the push
 * service says no longer exist (404/410), which the caller drops.
 */
export async function pushAll(subs, keys, subject, nowMs, fetchImpl = fetch) {
  const out = { sent: 0, gone: [], failed: [] };
  for (const sub of subs) {
    try {
      const res = await fetchImpl(sub.endpoint, {
        method: "POST",
        headers: {
          Authorization: await vapidAuth(sub.endpoint, keys, subject, Math.floor(nowMs / 1000)),
          TTL: "86400",
          Urgency: "high",
          "Content-Length": "0",
        },
        signal: AbortSignal.timeout(15000),
      });
      if (res.status === 404 || res.status === 410) out.gone.push(sub.endpoint);
      else if (res.ok) out.sent += 1;
      else out.failed.push(`HTTP ${res.status} from ${new URL(sub.endpoint).hostname}`);
    } catch (err) {
      out.failed.push(String(err && err.message ? err.message : err));
    }
  }
  return out;
}
