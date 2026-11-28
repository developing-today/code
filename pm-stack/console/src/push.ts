// Web Push, with nothing but the platform's own cryptography.
//
// A notification reaches the phone through the browser vendor's push
// service. The message is encrypted for the one browser that subscribed
// (RFC 8291, aes128gcm), and the request is signed with this server's
// own key pair so the push service knows who is sending (RFC 8292,
// VAPID). The key pair is made here on first use and kept in this
// service's own store; nobody ever handles it.
import type { Store } from "./store.ts";
import { timed, unread } from "./timed.ts";

/** How long a push service has to answer one message. */
const PUSH_SEND_MS = 15_000;
const enc = new TextEncoder();
const b64u = (bytes: Uint8Array | ArrayBuffer) => Buffer.from(bytes as ArrayBuffer).toString("base64url");

/** The contact a push service is given with every message (the "sub" a
 * sender must name): the project's page. It is not a person's address:
 * it goes to Google, Mozilla or Apple each time, and nothing about
 * delivery depends on whose it is. */
export const contact = (repo: string): string => `https://github.com/${repo}`;

/** A short mark for a push address. The app compares its own with the
 * marks of the addresses held here, and so can tell that this sender has
 * lost it, without any address being sent back. The app computes the
 * same (public/app.js, endpointMark). */
export async function endpointMark(endpoint: string): Promise<string> {
  const hash = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(endpoint));
  return b64u(hash).slice(0, 16);
}
const unb64u = (text: string) => new Uint8Array(Buffer.from(text, "base64url"));
const concat = (...parts: Uint8Array[]) => {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
};

/** Push services a subscription may name. This server posts to the
 * endpoint a browser gives it, so the endpoint has to be one of these.
 *
 * Chrome names one of two Google hosts: a released Chrome the first, and
 * a Chrome that is not on the stable channel (Beta, Dev, Canary, a build
 * of one's own) the second. Both are in Chromium's
 * components/push_messaging/push_messaging_constants.cc. Leaving the
 * second out refuses every such browser when it turns notifications on. */
const PUSH_HOSTS = [
  /^fcm\.googleapis\.com$/,
  /^jmt17\.google\.com$/,
  /^([a-z0-9-]+\.)*push\.services\.mozilla\.com$/,
  /^([a-z0-9-]+\.)*notify\.windows\.com$/,
  /^([a-z0-9-]+\.)*push\.apple\.com$/,
];

export interface Subscription {
  endpoint: string;
  keys: { p256dh: string; auth: string };
  at?: string;
  label?: string;
}

/** Notification classes the owner can switch off one by one. */
export const CLASSES = ["replies", "needs", "questions", "trouble"] as const;
export type Class = (typeof CLASSES)[number];

/** What came of the last notification that was sent: how many devices
 * the push services took it for, and what a service that refused said.
 * It is how the owner can tell that notifications arrive without
 * waiting for one. */
export interface Delivery {
  at: string;
  class: Class | "test";
  sent: number;
  gone: number;
  failed: number;
  /** The push service's host and its HTTP status (0: it did not answer),
   * for each device it was not taken for. Never the device's address. */
  refused: { host: string; status: number }[];
}

interface PushFile {
  vapid: { publicKey: string; privateJwk: JsonWebKey } | null;
  subscriptions: Subscription[];
  off: Class[];
  last?: Delivery | null;
}

export function validSubscription(value: unknown): Subscription | null {
  const v = value as { endpoint?: unknown; keys?: { p256dh?: unknown; auth?: unknown } } | null;
  if (!v || typeof v.endpoint !== "string" || typeof v.keys?.p256dh !== "string" || typeof v.keys?.auth !== "string") {
    return null;
  }
  let url: URL;
  try {
    url = new URL(v.endpoint);
  } catch {
    return null;
  }
  if (url.protocol !== "https:" || url.username || url.password || !PUSH_HOSTS.some((h) => h.test(url.hostname))) {
    return null;
  }
  // An uncompressed P-256 point is 65 bytes; the auth secret is 16.
  if (unb64u(v.keys.p256dh).length !== 65 || unb64u(v.keys.auth).length !== 16) return null;
  return { endpoint: url.href, keys: { p256dh: v.keys.p256dh, auth: v.keys.auth } };
}

async function hkdf(
  salt: Uint8Array<ArrayBuffer>,
  ikm: Uint8Array<ArrayBuffer>,
  info: Uint8Array<ArrayBuffer>,
  length: number,
): Promise<Uint8Array<ArrayBuffer>> {
  const key = await crypto.subtle.importKey("raw", ikm, "HKDF", false, ["deriveBits"]);
  return new Uint8Array(await crypto.subtle.deriveBits({ name: "HKDF", hash: "SHA-256", salt, info }, key, length * 8));
}

/** Encrypt one message for one subscription (RFC 8291). */
export async function encrypt(sub: Subscription, plaintext: Uint8Array): Promise<Uint8Array> {
  const uaPublic = unb64u(sub.keys.p256dh);
  const authSecret = unb64u(sub.keys.auth);
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

/** RFC 8030 section 7.2: a push service takes a message of 4096 bytes
 * and may refuse a longer one. */
export const PUSH_LIMIT = 4096;
/** What sealing adds to the words (see `encrypt`): the salt, the record
 * size, the length of the key and the key, then one delimiter and the tag. */
const SEALED_AROUND = 16 + 4 + 1 + 65 + 1 + 16;

/** The message as it is sent: whole when it fits in one push. When it
 * does not, the body is cut to the longest start that fits, in whole
 * characters and marked as cut, with the buttons kept; buttons that
 * could not fit beside any words at all are left off. */
function fitted(message: { body: string; actions?: unknown[] } & Record<string, unknown>): Uint8Array {
  const room = PUSH_LIMIT - SEALED_AROUND;
  // The short way round for nearly every message: what follows gives
  // the same answer for one that fits.
  const whole = enc.encode(JSON.stringify(message));
  if (whole.length <= room) return whole;
  const points = Array.from(message.body);
  const cut = (m: Record<string, unknown>, n: number) =>
    enc.encode(JSON.stringify({ ...m, body: n < points.length ? `${points.slice(0, n).join("")}…` : message.body }));
  const { actions: _buttons, ...bare } = message;
  const kept = cut(message, 0).length <= room ? message : bare;
  let fits = 0;
  let over = points.length;
  while (fits < over) {
    const mid = Math.ceil((fits + over) / 2);
    if (cut(kept, mid).length <= room) fits = mid;
    else over = mid - 1;
  }
  return cut(kept, fits);
}

type Poster = (
  url: string,
  init: { method: string; headers: Record<string, string>; body: Uint8Array; signal?: AbortSignal },
) => Promise<{ status: number }>;

/** The one-census shape (#14084 S2, §3.5) as the console status
 * carries it. The aggregator serves it live from the registry; the
 * console's live loader wires in with S4's registry read path, so
 * until then the console reports the slot as unavailable rather than
 * a zero census — a missing reader is not an empty registry. */
export interface PushRegistryCensus {
  available: boolean;
  asOf?: string;
  endpoints?: { total: number; byTransportClient: { transport: string; client: string; count: number }[] };
  lastSuccessfulSendAt?: string | null;
  needsUser?: { open: number; totalBlockedBy: number };
}

export class Push {
  private state: PushFile | null = null;
  private signer: CryptoKey | null = null;

  constructor(
    private store: Store,
    /** The contact a push service is given; a function when it is only
     * known later. */
    private subject: string | (() => string),
    private post: Poster = (url, init) => fetch(url, init as RequestInit),
    private now: () => number = Date.now,
    /** S4 wires the registry-backed census loader; until then null. */
    private registryCensus: (() => Promise<PushRegistryCensus | null>) | null = null,
  ) {}

  private async load(): Promise<PushFile> {
    if (this.state) return this.state;
    try {
      this.state = JSON.parse((await this.store.read()) ?? "") as PushFile;
    } catch {
      this.state = { vapid: null, subscriptions: [], off: [] };
    }
    if (!this.state.vapid) {
      const pair = (await crypto.subtle.generateKey({ name: "ECDSA", namedCurve: "P-256" }, true, [
        "sign",
        "verify",
      ])) as CryptoKeyPair;
      this.state.vapid = {
        publicKey: b64u(await crypto.subtle.exportKey("raw", pair.publicKey)),
        privateJwk: await crypto.subtle.exportKey("jwk", pair.privateKey),
      };
      await this.save();
    }
    return this.state;
  }

  private async save(): Promise<void> {
    if (!this.state) return;
    await this.store.write(JSON.stringify(this.state));
  }

  /** What the page needs to subscribe, and what is switched on. */
  async status() {
    const s = await this.load();
    return {
      key: s.vapid?.publicKey ?? "",
      devices: s.subscriptions.length,
      known: await Promise.all(s.subscriptions.map((x) => endpointMark(x.endpoint))),
      registry: (await this.registryCensus?.()) ?? { available: false },
      classes: Object.fromEntries(CLASSES.map((c) => [c, !s.off.includes(c)])),
      last: s.last ?? null,
    };
  }

  async subscribe(value: unknown, label: unknown) {
    const sub = validSubscription(value);
    if (!sub) throw new Error("subscription: not a push subscription from a known push service");
    const s = await this.load();
    s.subscriptions = [
      ...s.subscriptions.filter((x) => x.endpoint !== sub.endpoint),
      { ...sub, at: new Date(this.now()).toISOString(), label: typeof label === "string" ? label.slice(0, 80) : "" },
    ].slice(-20);
    await this.save();
    return { devices: s.subscriptions.length };
  }

  async unsubscribe(endpoint: unknown) {
    const s = await this.load();
    const before = s.subscriptions.length;
    s.subscriptions = s.subscriptions.filter((x) => x.endpoint !== endpoint);
    if (s.subscriptions.length !== before) await this.save();
    return { devices: s.subscriptions.length };
  }

  async setClass(cls: unknown, on: unknown) {
    if (!CLASSES.includes(cls as Class)) throw new Error(`class: one of ${CLASSES.join(", ")}`);
    const s = await this.load();
    s.off = s.off.filter((c) => c !== cls);
    if (on !== true) s.off.push(cls as Class);
    await this.save();
    return (await this.status()).classes;
  }

  private async vapid(endpoint: string): Promise<string> {
    const s = await this.load();
    const v = s.vapid as NonNullable<PushFile["vapid"]>;
    this.signer ??= await crypto.subtle.importKey("jwk", v.privateJwk, { name: "ECDSA", namedCurve: "P-256" }, false, [
      "sign",
    ]);
    const head = b64u(enc.encode(JSON.stringify({ typ: "JWT", alg: "ES256" })));
    const body = b64u(
      enc.encode(
        JSON.stringify({
          aud: new URL(endpoint).origin,
          exp: Math.floor(this.now() / 1000) + 12 * 3600,
          sub: typeof this.subject === "function" ? this.subject() : this.subject,
        }),
      ),
    );
    const sig = await crypto.subtle.sign(
      { name: "ECDSA", hash: "SHA-256" },
      this.signer,
      enc.encode(`${head}.${body}`),
    );
    return `vapid t=${head}.${body}.${b64u(sig)}, k=${v.publicKey}`;
  }

  /** Send one notification to every subscribed device, unless its class
   * is switched off. A subscription the push service says is gone is
   * forgotten. Returns how many devices took it. */
  async send(
    cls: Class | "test",
    message: { title: string; body: string; url: string; tag?: string; actions?: unknown[] },
  ) {
    const s = await this.load();
    if (cls !== "test" && s.off.includes(cls)) return { sent: 0, gone: 0, failed: 0 };
    const payload = fitted({ ...message, class: cls });
    let sent = 0;
    let failed = 0;
    const gone: string[] = [];
    const refused: Delivery["refused"] = [];
    const attempted = s.subscriptions.length;
    for (const sub of s.subscriptions) {
      const host = new URL(sub.endpoint).hostname;
      try {
        const headers = {
          Authorization: await this.vapid(sub.endpoint),
          "Content-Encoding": "aes128gcm",
          "Content-Type": "application/octet-stream",
          TTL: "86400",
          Urgency: "high",
        };
        const body = await encrypt(sub, payload);
        // Only the status is read, so the deadline is the answer's.
        const res = await timed(PUSH_SEND_MS, (signal) =>
          this.post(sub.endpoint, { method: "POST", headers, body, signal }),
        );
        unread(res);
        if (res.status === 404 || res.status === 410) gone.push(sub.endpoint);
        else if (res.status >= 200 && res.status < 300) sent++;
        else {
          failed++;
          refused.push({ host, status: res.status });
        }
      } catch {
        failed++;
        refused.push({ host, status: 0 });
      }
    }
    if (gone.length) s.subscriptions = s.subscriptions.filter((x) => !gone.includes(x.endpoint));
    if (attempted > 0) {
      s.last = {
        at: new Date(this.now()).toISOString(),
        class: cls,
        sent,
        gone: gone.length,
        failed,
        refused: refused.slice(0, 5),
      };
      await this.save();
    }
    return { sent, gone: gone.length, failed };
  }
}
