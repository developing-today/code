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
import type { SettingDiffers } from "./settings.ts";

/** What a machine tells the edge in every sync. Who may sign in (the
 * owners, the certificate names, the issuers, the Android signing
 * fingerprints) the edge goes by: that is the deployment's, set where
 * the hostname is set up. The repository's settings among these
 * (src/settings.ts) the edge has of its own and goes by its own, bar
 * the list of repositories (see MACHINES_FOR_NOW there); what a machine
 * tells for them is compared and shown, not used. The last four fields
 * are absent from configs stored or reported before the edge could act
 * by itself: those still validate, with the fields empty. */
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
  /** What this machine is called and what it can do. Absent from a
   * console older than this: the edge then knows it only as a machine
   * that reports. */
  machine?: MachineReport;
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

/** What a machine says of itself in each report. A machine is anything
 * that reports to the edge under a key the edge knows. It reports the
 * state only it has (`sections`) and does what only it can (`actions`,
 * by kind); everything else the platform does without it. */
export interface MachineReport {
  name: string;
  actions: string[];
  sections: string[];
}

// A letter or digit first, never a blank last (two names that differ
// only by one would look the same on the screen), 63 at most.
const MACHINE_NAME = /^[A-Za-z0-9](?:[A-Za-z0-9 ._-]{0,61}[A-Za-z0-9._-])?$/;
const MACHINE_CAN = /^[A-Za-z][A-Za-z0-9._-]{0,47}$/;
/** How many kinds of action, or sections, one machine may say it has. */
const MACHINE_CAN_MAX = 200;

/** A name fit to show for a machine, from whatever it is called
 * locally; empty when nothing of it is. */
export function machineName(value: unknown): string {
  const name = String(value ?? "")
    .trim()
    .replace(/[^A-Za-z0-9 ._-]+/g, "-")
    .replace(/^[^A-Za-z0-9]+/, "")
    .slice(0, 63)
    // The cut may land just after a blank.
    .trimEnd();
  return MACHINE_NAME.test(name) ? name : "";
}

/** A machine's account of itself as a report may give it, or null. */
export function machineReport(value: unknown): MachineReport | null {
  const v = value as Record<string, unknown> | null;
  if (!v || typeof v !== "object") return null;
  if (typeof v.name !== "string" || !MACHINE_NAME.test(v.name)) return null;
  const list = (given: unknown): string[] | null => {
    if (!Array.isArray(given) || given.length > MACHINE_CAN_MAX) return null;
    if (!given.every((item) => typeof item === "string" && MACHINE_CAN.test(item))) return null;
    return [...new Set(given as string[])].sort();
  };
  const actions = list(v.actions);
  const sections = list(v.sections);
  return actions && sections ? { name: v.name, actions, sections } : null;
}

/** What the Machines screen is drawn from: the platform's own part,
 * each machine that reports, and what is held for one. */
export interface MachinesAnswer {
  /** Who answered: the edge, or a machine serving the app by itself. */
  served: "edge" | "machine";
  /** What the platform does with no machine: null when it is not in
   * front, and an empty list of each when it only serves and waits. */
  platform: { primary: boolean; actions: string[]; sections: string[]; refused: string[] } | null;
  machines: {
    name: string;
    /** When it last reported, in milliseconds; null if it never has. */
    seen: number | null;
    /** It has not reported for long enough to be taken for away. */
    quiet: boolean;
    version: string | null;
    actions: string[];
    sections: string[];
    /** The platform's settings it runs with others of, each with both
     * values and which the platform goes by. Absent when there is none
     * (and from an answer older than the saying). */
    differs?: SettingDiffers[];
  }[];
  /** What the owner did that waits to be done, oldest first, and who
   * each waits for (absent from an answer older than the saying). */
  held: (Held & { waits?: HeldFor })[];
}

/** Who a held action waits for. "either": the platform does it itself
 * as soon as it can again, or a machine that is back first. "machine":
 * only a machine can do it, or it comes after something that waits for
 * one. "asked": a machine was given it, or may have got it, and says
 * what became of it when it is back; nobody else does it meanwhile. */
export type HeldFor = "either" | "machine" | "asked";

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
  /** It certainly reached no machine: it was never sent toward one, or
   * the one time it was, what answered in the machine's place said it
   * never arrived. Only then may the platform do it itself. Without
   * this a machine may have got it (no answer came, or one that does
   * not say) and may have done it: it stays that machine's by name. */
  unsent?: true;
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

/** A released build of the Android app, as the version.json beside it
 * in the GitHub release describes it (android/release.sh writes it). */
export interface AndroidRelease {
  name: string;
  code: number;
  sha256: string;
  size: number;
  /** When it was released, as an ISO time. */
  built: string;
}

/** The largest released build that is passed on. */
export const RELEASE_LIMIT = 32 * 1024 * 1024;

/** A release's description as GitHub may serve it, or null. */
export function androidRelease(value: unknown): AndroidRelease | null {
  const v = value as Record<string, unknown> | null;
  if (!v || typeof v !== "object") return null;
  if (typeof v.name !== "string" || !/^[0-9A-Za-z][0-9A-Za-z.+-]{0,63}$/.test(v.name)) return null;
  if (typeof v.code !== "number" || !Number.isSafeInteger(v.code) || v.code <= 0) return null;
  if (typeof v.sha256 !== "string" || !/^[0-9a-f]{64}$/.test(v.sha256)) return null;
  if (typeof v.size !== "number" || !Number.isSafeInteger(v.size) || v.size <= 0 || v.size > RELEASE_LIMIT) return null;
  if (typeof v.built !== "string" || Number.isNaN(Date.parse(v.built))) return null;
  return {
    name: v.name,
    code: v.code,
    sha256: v.sha256,
    size: v.size,
    built: new Date(Date.parse(v.built)).toISOString(),
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
  /** Sections the edge reads itself, and so need not be sent to it.
   * Absent from an edge that reads none. */
  owns?: string[];
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
  // What the owner says is never lost for want of a machine: it is
  // kept and filed when one is back (or filed here, when the edge can).
  "intake.send",
  "triage.enqueue",
]);
/** What an action is about, where the order of two matters: a rank set
 * twice, a task moved twice. Two about one thing are done in the order
 * the owner did them: one that may be done late goes behind an earlier
 * one about the same thing that is still held for a machine. Null: it
 * stands alone. (The device keeps the same order for what it has not
 * sent yet: `subject` in public/outbox.js says the same, and a test
 * holds the two together.) */
export function subjectOf(kind: string, args: Record<string, unknown> | null | undefined): string | null {
  const a = args ?? {};
  const of = (what: string, value: unknown) => {
    const said = typeof value === "string" || typeof value === "number" ? String(value) : "";
    return said === "" ? null : `${what}:${said}`;
  };
  switch (kind) {
    case "ask.reply":
    case "ask.close":
    case "issue.comment":
    case "issue.rank":
      return of("issue", a.issue);
    case "task.move":
    case "task.cancel":
    case "task.unblock":
      return of("task", a.task);
    case "hands.done":
    case "need.comment":
      return of("ref", a.ref);
    case "question.answer":
    case "question.defer":
    case "questions.accept":
      return of("questions", a.set);
    case "pr.merge":
      return of("pr", a.pr);
    case "unit.act":
      return of("unit", a.unit);
    default:
      return null;
  }
}
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
const unb64u = (text: string): Uint8Array<ArrayBuffer> => {
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
  return b64u(await crypto.subtle.sign(SIGN, key, body as BufferSource));
}

export async function verify(key: PublicKey, body: Uint8Array, signature: string): Promise<boolean> {
  let sig: Uint8Array<ArrayBuffer>;
  try {
    sig = unb64u(signature);
  } catch {
    return false;
  }
  if (sig.length !== 64) return false;
  const imported = await crypto.subtle.importKey("jwk", key, CURVE, false, ["verify"]);
  return crypto.subtle.verify(SIGN, imported, sig, body as BufferSource);
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
  // The fields the edge acts by: present ones must be well-formed,
  // absent ones are a config from before it could, which still validates.
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
