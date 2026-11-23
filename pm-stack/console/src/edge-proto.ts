// What the host and the edge say to each other.
//
// The app is served from the edge (a Cloudflare Worker in front of the
// public hostname). The host stays what it is, the machine with the
// fleet's tools on it, and reports to the edge:
//
//   host -> edge   POST /console/edge/sync, signed with the host's own
//                  key: the sections of state that changed, and what
//                  became of anything the edge was holding for it
//   edge -> host   in the answer: whether anyone is looking, and the
//                  actions the owner took while the host could not be
//                  reached
//
// The host's key pair is made on the host and never leaves it. The edge
// learns the public half by asking the host through the tunnel, which
// only the host can answer, so there is no shared secret to keep
// anywhere. A sync is refused unless its signature verifies, its time is
// near the edge's own clock, and that time is later than the last sync
// accepted.
import type { Issuer } from "./config.ts";

/** What the edge needs to know to recognise the owner. It comes from the
 * host in every sync, so there is one place it is set. The last four
 * fields are what the edge needs to read and write by itself while the
 * laptop is away: where the queue store is and which token opens it, how
 * the owner's prompts are marked, and which repositories hold his
 * needs-me items. They are absent from configs stored or reported
 * before direct mode existed: those still validate, with the fields
 * empty, and direct capabilities stay off until a config carries them
 * (see Edge.directConfigReady). */
export interface EdgeConfig {
  owners: string[];
  certNames: string[];
  issuers: Issuer[];
  repo: string;
  queue: string;
  androidPackage: string;
  androidFingerprints: string[];
  /** Whether the host has a signed Android build to hand out. */
  androidApp: boolean;
  queueDbUrl: string;
  queueTokenName: string;
  promptLabel: string;
  needsRepos: string[];
}

export interface SyncSection {
  name: string;
  /** When the host read it, in milliseconds. */
  at: number;
  /** A fingerprint of the data: how each side tells it has not changed. */
  hash: string;
  data: unknown;
}

export interface SyncRequest {
  v: 1;
  /** Milliseconds, later than every sync before it. */
  at: number;
  /** The signed Android build the host has, if it has one: its
   * fingerprint, size and date, and the build itself (base64) only in a
   * report that follows the edge asking for it. The edge keeps a copy,
   * so the app can be downloaded while the host is away. */
  apk?: ApkReport;
  /** The commit the host's console is running. The edge says it in its
   * health answer, so whether a merge has reached the host can be seen
   * from anywhere, without reaching the host. */
  version?: string;
  config: EdgeConfig;
  /** Sections that changed since the edge last had them. */
  sections: SyncSection[];
  /** Every section the host has: its fingerprint, when the host last
   * read it, and why its latest read failed if it did. A section that
   * has not changed is not sent again, and this is how the edge knows
   * its copy is still current as of that read, shows the same "could
   * not read" notice the host would, and asks for a copy it has lost. */
  have: Record<string, { hash: string; at: number; error?: string | null }>;
  /** What became of the held actions the host was given. */
  acks: { id: string; ok: boolean; error?: string }[];
  /** Linked-phone hashes and metadata only; never phone tokens. */
  phones: PhoneLinks;
}

export interface PhoneLinks {
  devices: { id: string; name: string; hash: string; createdAt: string; seenAt: string | null }[];
  removed: { hash: string; at: string }[];
}

/** An action the owner took while the host could not be reached. */
export interface Held {
  id: string;
  kind: string;
  args: Record<string, unknown>;
  /** Who took it, as the edge verified them. */
  who: string;
  at: string;
}

export interface ApkReport {
  sha256: string;
  size: number;
  /** When the build was put in place on the host, in milliseconds. */
  at: number;
  data?: string;
}

/** The largest Android build that is carried in a report. */
export const APK_LIMIT = 5 * 1024 * 1024;
/** What the host keeps a report under, with room to spare below what
 * the edge refuses as too large (8 MiB). A report that would carry the
 * build past this carries the build alone, and the sections follow in
 * the next one. */
export const REPORT_ROOM = 7.5 * 1024 * 1024;

/** A build's description as a report may give it, or null. */
export function apkReport(value: unknown): ApkReport | null {
  const v = value as Record<string, unknown> | null;
  if (!v || typeof v !== "object") return null;
  if (typeof v.sha256 !== "string" || !/^[0-9a-f]{64}$/.test(v.sha256)) return null;
  if (typeof v.size !== "number" || !Number.isSafeInteger(v.size) || v.size <= 0 || v.size > APK_LIMIT) return null;
  if (typeof v.at !== "number" || !Number.isFinite(v.at)) return null;
  return {
    sha256: v.sha256,
    size: v.size,
    at: v.at,
    ...(typeof v.data === "string" ? { data: v.data } : {}),
  };
}

export interface SyncResponse {
  ok: true;
  /** Someone has the app open: keep every section fresh. */
  watching: boolean;
  /** A device takes notifications or a phone is linked: keep the
   * sections those are built from fresh even with nobody looking. */
  notify: boolean;
  /** Sections the edge lacks and wants sent whole. */
  want: string[];
  commands: Held[];
  /** The edge's merged phone-link state, containing hashes only. */
  phones: PhoneLinks;
}

/** Actions that may be done late, when the host is back. The rest act on
 * something that may have changed by then (a merge, a unit, a task's
 * state), so they are refused while the host cannot be reached.
 * `prompt.enqueue` is the second half of a prompt the edge filed itself:
 * the issue already exists, so only the queue task is left to do. */
export const HOLDABLE = new Set([
  "ask.send",
  "ask.reply",
  "ask.close",
  "question.answer",
  "question.defer",
  "questions.accept",
  "hands.done",
  "need.comment",
  "issue.comment",
  "issue.rank",
  "task.move",
  "message.send",
  "prompt.enqueue",
]);
/** A held action older than this is dropped, and said to have been. */
export const HELD_FOR_MS = 24 * 60 * 60_000;
/** A sync whose time is further than this from the edge's clock is refused. */
export const SYNC_SKEW_MS = 2 * 60_000;

const SIGN = { name: "ECDSA", hash: "SHA-256" } as const;
const CURVE = { name: "ECDSA", namedCurve: "P-256" } as const;

const b64u = (bytes: ArrayBuffer | Uint8Array): string => {
  let bin = "";
  for (const b of new Uint8Array(bytes as ArrayBuffer)) bin += String.fromCharCode(b);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
};
const unb64u = (text: string): Uint8Array => {
  if (!/^[A-Za-z0-9_-]*$/.test(text)) throw new Error("not base64url");
  const bin = atob(text.replace(/-/g, "+").replace(/_/g, "/"));
  return Uint8Array.from(bin, (c) => c.charCodeAt(0));
};

/** The public half of a key, with only the fields that define it. */
export interface PublicKey {
  kty: "EC";
  crv: "P-256";
  x: string;
  y: string;
}

export function publicKey(value: unknown): PublicKey | null {
  const v = value as Record<string, unknown> | null;
  if (!v || v.kty !== "EC" || v.crv !== "P-256") return null;
  if (typeof v.x !== "string" || typeof v.y !== "string") return null;
  if (!/^[A-Za-z0-9_-]{43}$/.test(v.x) || !/^[A-Za-z0-9_-]{43}$/.test(v.y)) return null;
  return { kty: "EC", crv: "P-256", x: v.x, y: v.y };
}

/** A short name for a public key: the start of its SHA-256. */
export async function keyId(key: PublicKey): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(`${key.x}.${key.y}`));
  return b64u(digest).slice(0, 16);
}

export async function newKeyPair(): Promise<{ publicJwk: PublicKey; privateJwk: JsonWebKey }> {
  const pair = (await crypto.subtle.generateKey(CURVE, true, ["sign", "verify"])) as CryptoKeyPair;
  const pub = publicKey(await crypto.subtle.exportKey("jwk", pair.publicKey));
  if (!pub) throw new Error("the platform made a key this cannot read");
  return { publicJwk: pub, privateJwk: await crypto.subtle.exportKey("jwk", pair.privateKey) };
}

export async function sign(privateJwk: JsonWebKey, body: Uint8Array): Promise<string> {
  const key = await crypto.subtle.importKey("jwk", privateJwk, CURVE, false, ["sign"]);
  return b64u(await crypto.subtle.sign(SIGN, key, body));
}

export async function verify(key: PublicKey, body: Uint8Array, signature: string): Promise<boolean> {
  let sig: Uint8Array;
  try {
    sig = unb64u(signature);
  } catch {
    return false;
  }
  if (sig.length !== 64) return false;
  const imported = await crypto.subtle.importKey("jwk", key, CURVE, false, ["verify"]);
  return crypto.subtle.verify(SIGN, imported, sig, body);
}

const strings = (value: unknown): value is string[] =>
  Array.isArray(value) && value.every((v) => typeof v === "string");

/** The config a sync carries, checked: the edge decides who the owner is
 * from this, so a malformed one is refused whole. */
export function edgeConfig(value: unknown): EdgeConfig | null {
  const v = value as Record<string, unknown> | null;
  if (!v || !strings(v.owners) || !strings(v.certNames) || !strings(v.androidFingerprints)) return null;
  if (!Array.isArray(v.issuers)) return null;
  const issuers: Issuer[] = [];
  for (const raw of v.issuers as Record<string, unknown>[]) {
    if (!raw || typeof raw !== "object") return null;
    for (const key of ["name", "issuer", "jwksUri", "audience"]) {
      if (typeof raw[key] !== "string" || raw[key] === "") return null;
    }
    if (!/^https:\/\//.test(raw.issuer as string) || !/^https:\/\//.test(raw.jwksUri as string)) return null;
    if (raw.source !== "access" && raw.source !== "bearer") return null;
    issuers.push({
      name: raw.name as string,
      issuer: raw.issuer as string,
      jwksUri: raw.jwksUri as string,
      audience: raw.audience as string,
      source: raw.source,
    });
  }
  if (typeof v.repo !== "string" || typeof v.queue !== "string" || typeof v.androidPackage !== "string") return null;
  // The direct fields: present ones must be well-formed, absent ones
  // are a config from before direct mode, which still validates.
  if (v.queueDbUrl !== undefined && (typeof v.queueDbUrl !== "string" || !/^https:\/\//.test(v.queueDbUrl)))
    return null;
  if (v.queueTokenName !== undefined && typeof v.queueTokenName !== "string") return null;
  if (v.promptLabel !== undefined && typeof v.promptLabel !== "string") return null;
  if (v.needsRepos !== undefined && !strings(v.needsRepos)) return null;
  return {
    owners: v.owners.map((o) => o.toLowerCase()),
    certNames: v.certNames,
    issuers,
    repo: v.repo,
    queue: v.queue,
    androidPackage: v.androidPackage,
    androidFingerprints: v.androidFingerprints,
    androidApp: v.androidApp === true,
    queueDbUrl: typeof v.queueDbUrl === "string" ? v.queueDbUrl : "",
    queueTokenName: typeof v.queueTokenName === "string" ? v.queueTokenName : "",
    promptLabel: typeof v.promptLabel === "string" ? v.promptLabel : "",
    needsRepos: strings(v.needsRepos) ? v.needsRepos : [],
  };
}
