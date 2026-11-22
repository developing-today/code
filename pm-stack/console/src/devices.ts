// Phones that may read a short summary without the browser's sign-in.
//
// The Android app's home-screen widget cannot use the sign-in the
// browser holds, so it gets a token of its own that reads one thing: the
// counts and the top line of each list. Nothing can be changed with it.
//
// Linking never passes a secret through an address or another app. The
// phone makes a random verifier and shows the owner only its hash (the
// challenge) inside the signed-in app. The owner approves the challenge;
// the phone then presents the verifier, which only it knows, and is
// given its token. The store keeps hashes of tokens, never the tokens.
import { createHash, randomBytes, timingSafeEqual } from "node:crypto";
import { OpError } from "./errors.ts";
import type { Store as Kept } from "./store.ts";

export interface Device {
  id: string;
  name: string;
  /** SHA-256 of the token, in hex. */
  hash: string;
  createdAt: string;
  seenAt: string | null;
}

interface Pending {
  challenge: string;
  name: string;
  expiresAt: number;
}

interface Store {
  devices: Device[];
  pending: Pending[];
}

const CHALLENGE = /^[A-Za-z0-9_-]{43}$/;
const VERIFIER = /^[A-Za-z0-9_-]{43,128}$/;
const TOKEN = /^[A-Za-z0-9_-]{43}$/;
const APPROVAL_MS = 10 * 60_000;
const MAX_DEVICES = 8;
const MAX_PENDING = 4;
/** `seenAt` is written at most this often, so reading does not mean writing. */
const SEEN_EVERY_MS = 10 * 60_000;

const sha = (text: string) => createHash("sha256").update(text).digest();

/** The challenge a verifier answers: its SHA-256, in base64url. */
export function challengeOf(verifier: string): string {
  return sha(verifier).toString("base64url");
}

function same(a: string, b: string): boolean {
  const x = Buffer.from(a);
  const y = Buffer.from(b);
  return x.length === y.length && timingSafeEqual(x, y);
}

/** So many tries in a window and no more: for the two paths that are
 * reachable without sign-in. */
export class Limiter {
  private start = 0;
  private used = 0;
  constructor(
    private max: number,
    private windowMs: number,
    private now: () => number = Date.now,
  ) {}

  allow(): boolean {
    const t = this.now();
    if (t - this.start >= this.windowMs) {
      this.start = t;
      this.used = 0;
    }
    this.used++;
    return this.used <= this.max;
  }
}

export class Devices {
  private store: Store | null = null;

  constructor(
    private kept: Kept,
    private now: () => number = Date.now,
    private random: (bytes: number) => Buffer = randomBytes,
  ) {}

  private async load(): Promise<Store> {
    if (this.store) return this.store;
    let store: Store = { devices: [], pending: [] };
    try {
      const parsed = JSON.parse((await this.kept.read()) ?? "");
      if (parsed && Array.isArray(parsed.devices) && Array.isArray(parsed.pending)) store = parsed;
    } catch {
      // nothing kept yet, or something that cannot be read: start with no devices
    }
    this.store ??= store;
    return this.store;
  }

  private save(): Promise<void> {
    return this.kept.write(JSON.stringify(this.store));
  }

  /** The owner, signed in, approves a phone that is asking to be linked. */
  async approve(args: { challenge?: unknown; name?: unknown }): Promise<{ expires: string }> {
    const challenge = String(args.challenge ?? "");
    if (!CHALLENGE.test(challenge)) throw new OpError(400, "that is not a link request from the app");
    const name =
      String(args.name ?? "")
        .trim()
        .slice(0, 60) || "Android phone";
    const store = await this.load();
    const t = this.now();
    if (store.devices.length >= MAX_DEVICES) {
      throw new OpError(409, `${MAX_DEVICES} phones are linked already: remove one first`);
    }
    const expiresAt = t + APPROVAL_MS;
    store.pending = [
      ...store.pending.filter((p) => p.expiresAt > t && p.challenge !== challenge),
      { challenge, name, expiresAt },
    ].slice(-MAX_PENDING);
    await this.save();
    return { expires: new Date(expiresAt).toISOString() };
  }

  /** The phone presents its verifier. If the owner approved the matching
   * challenge, the phone is given its token, once. */
  async link(verifier: unknown): Promise<{ id: string; token: string } | null> {
    if (typeof verifier !== "string" || !VERIFIER.test(verifier)) return null;
    const challenge = challengeOf(verifier);
    const store = await this.load();
    const t = this.now();
    const found = store.pending.find((p) => p.expiresAt > t && same(p.challenge, challenge));
    if (!found || store.devices.length >= MAX_DEVICES) return null;
    const token = this.random(32).toString("base64url");
    const device: Device = {
      id: this.random(6).toString("base64url"),
      name: found.name,
      hash: sha(token).toString("hex"),
      createdAt: new Date(t).toISOString(),
      seenAt: null,
    };
    store.pending = store.pending.filter((p) => p !== found && p.expiresAt > t);
    store.devices.push(device);
    await this.save();
    return { id: device.id, token };
  }

  /** The device a token belongs to, or null. */
  async check(token: unknown): Promise<Device | null> {
    if (typeof token !== "string" || !TOKEN.test(token)) return null;
    const hash = sha(token).toString("hex");
    const store = await this.load();
    const device = store.devices.find((d) => same(d.hash, hash)) ?? null;
    if (device) {
      const t = this.now();
      if (!device.seenAt || t - Date.parse(device.seenAt) > SEEN_EVERY_MS) {
        device.seenAt = new Date(t).toISOString();
        // A read is answered whether or not this could be written down.
        this.save().catch(() => {});
      }
    }
    return device;
  }

  /** What the owner is shown: never a token or its hash. */
  async list(): Promise<{ id: string; name: string; createdAt: string; seenAt: string | null }[]> {
    const store = await this.load();
    return store.devices.map(({ id, name, createdAt, seenAt }) => ({ id, name, createdAt, seenAt }));
  }

  async remove(args: { id?: unknown }): Promise<{ removed: string }> {
    const id = String(args.id ?? "");
    const store = await this.load();
    const device = store.devices.find((d) => d.id === id);
    if (!device) throw new OpError(404, "that phone is not linked");
    store.devices = store.devices.filter((d) => d !== device);
    await this.save();
    return { removed: device.name };
  }
}

const clip = (text: unknown, max: number): string => {
  const s = String(text ?? "")
    .replace(/\s+/g, " ")
    .trim();
  return s.length > max ? `${s.slice(0, max - 1)}…` : s;
};
const list = (value: unknown): Record<string, unknown>[] =>
  Array.isArray(value) ? (value as Record<string, unknown>[]) : [];
const count = (value: unknown): number => (Number.isFinite(Number(value)) ? Number(value) : 0);

/** What a linked phone is shown: counts, and the top line of each list.
 * Built from sections the app already keeps; `at` is the oldest of them. */
export function summary(sections: Record<string, unknown>, at: number): Record<string, unknown> {
  const board = (sections.board ?? {}) as { counts?: Record<string, unknown>; now?: unknown; next?: unknown };
  const census = (sections.census ?? {}) as { hands?: unknown; reviews?: unknown };
  const waiting = list((sections.sessions as { waiting?: unknown } | null)?.waiting).length;
  const asks = list(sections.asks);
  const questions = count((sections.questionCount as { total?: unknown } | null)?.total);
  // The same sum as the app's own "Needs me" badge.
  const hands = list(census.hands).length + waiting;
  const reviews = list(census.reviews).length;
  return {
    v: 1,
    at: at > 0 ? new Date(at).toISOString() : null,
    now: {
      count: count(board.counts?.claimed),
      top: list(board.now)
        .slice(0, 3)
        .map((t) => ({ title: clip(t.title, 90), step: clip(t.step ?? t.goal ?? "", 120) })),
    },
    next: {
      count: count(board.counts?.queued),
      top: list(board.next)
        .slice(0, 3)
        .map((t) => clip(t.title, 90)),
    },
    needs: { total: questions + hands + reviews, questions, hands, reviews },
    asks: {
      answered: asks.filter((a) => a.status === "answered").length,
      waiting: asks.filter((a) => a.status !== "answered" && a.status !== "done").length,
    },
  };
}
