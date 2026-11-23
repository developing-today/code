// The app, served from the edge.
//
// A Cloudflare Worker sits in front of the public hostname. This is the
// part of it that holds state (it runs in one Durable Object): it checks
// who is calling, serves the app's state, runs the owner's actions,
// sends notifications, and answers a linked phone's widget.
//
// With its two tokens (GitHub's and the queue store's) it does the core
// itself, whether or not a machine is connected: it reads the board,
// the owner's prompts, pull requests, ranked issues, roadmaps, bugs and
// intake, and does what is one call to GitHub or the queue store: ask,
// reply, comment, rank, move a task, merge, deploy, file a note or a
// bug. A machine (the laptop) connects and reports what only a machine
// has: sessions, questions, things for hands, its units and agents; and
// it does what only a machine can, at once when it is in reach, or
// later, held here, when it is not.
//
// Without the tokens it still serves the app from the copy a machine
// last reported and holds what may be done late.
//
// Nothing here trusts where a request came from. The owner is whoever
// presents a sign-in token that verifies; the host is whoever signs with
// the key the host itself published through its tunnel; a phone is
// whoever holds a token this service issued.
import { actions, type Action, ownActions } from "../../src/actions.ts";
import { AuthError, type Identity, identify } from "../../src/auth.ts";
import {
  DIRECT_ACTIONS,
  DIRECT_READ_SECTIONS,
  type DirectClasses,
  directClasses,
  directRead,
  type Outward,
  READ_FROM,
} from "./direct.ts";
import { Devices, KeyedLimiter, Limiter } from "../../src/devices.ts";
import { type Act, DOC_NAMES, type DocName, Docs, fingerprint, Stamped } from "../../src/docs.ts";
import {
  type AndroidRelease,
  androidRelease,
  APK_LIMIT,
  type ApkReport,
  apkReport,
  type EdgeConfig,
  edgeConfig,
  HELD_FOR_MS,
  type Held,
  HOLDABLE,
  keyId,
  type PublicKey,
  publicKey,
  subjectOf,
  SYNC_SKEW_MS,
  type SyncRequest,
  type SyncResponse,
  type SyncSection,
  verify,
} from "../../src/edge-proto.ts";
import { OpError } from "../../src/errors.ts";
import {
  assetLinks,
  BASE,
  deviceLink,
  deviceSummary,
  docResponse,
  json,
  manifest,
  type Phones,
  SECURITY,
  sameSiteWrite,
} from "../../src/http.ts";
import { plain } from "../../src/plain.ts";
import type { Later } from "../../src/plan.ts";
import { platformRead } from "../../src/reads.ts";
import { Notifier, WATCHED as NOTIFIED } from "../../src/notify.ts";
import { contact, Push } from "../../src/push.ts";
import type { Store } from "../../src/store.ts";

/** The edge's own storage: small values by key. */
export interface KV {
  get(key: string): Promise<unknown>;
  put(key: string, value: unknown): Promise<void>;
  delete(key: string): Promise<void>;
}

/** How long each section may go unreported before the app says the
 * state is old: the host's own interval for it, and room for a sync. */
const SECTIONS: Record<string, number> = {
  board: 30_000,
  sessions: 45_000,
  census: 120_000,
  asks: 45_000,
  questionCount: 45_000,
  questions: 45_000,
  prs: 90_000,
  priorities: 180_000,
  agents: 75_000,
  services: 45_000,
  more: 2 * 60 * 60_000,
};
/** Sections the notifier and a phone's widget are built from. */
const WATCHED = ["board", "asks", "census", "sessions", "questionCount", "services"];
/** One stored value holds at most this many characters of a section. */
const CHUNK = 600_000;
const SYNC_LIMIT = 8 * 1024 * 1024;
/** A report's signature: 64 bytes, as unpadded base64url. */
const SIGNATURE = /^[A-Za-z0-9_-]{86}$/;
/** Reports from one address that may fail their checks in a minute
 * before that address is refused unread. The host's own never fail. */
const SYNC_REFUSALS = 20;
/** The Android build is kept in pieces of this many bytes. */
const APK_CHUNK = 512 * 1024;
/** The most pieces a build that may be carried can take. */
const APK_PIECES = Math.ceil(APK_LIMIT / APK_CHUNK);
/** A build that could not be kept is asked for again no sooner than this. */
const APK_RETRY_MS = 10 * 60_000;
/** With no report for this long, the laptop is not there to ask. */
const HOST_QUIET_MS = 60_000;
/** The GitHub release whose address always holds the app's newest build. */
const RELEASE_TAG = "pm-console-android";
/** GitHub is asked what that release holds no more often than this, */
const RELEASE_EVERY_MS = 10 * 60_000;
/** this soon again when it could not be asked, */
const RELEASE_RETRY_MS = 60_000;
/** and is given this long to answer, and this long to send the build. */
const RELEASE_ASK_MS = 8_000;
const RELEASE_FETCH_MS = 25_000;

interface ApkMeta {
  sha256: string;
  size: number;
  at: number;
}
const ACT_LIMIT = 256 * 1024;
const HELD_MAX = 100;
const ACTS_KEPT = 30;
const AUDIT_KEPT = 300;
/** "Someone is looking" lasts this long after the last read of a document. */
const WATCHING_MS = 90_000;
/** The host is reported silent after this long without a sync. */
export const SILENT_MS = 10 * 60_000;
/** With nobody looking, what the edge reads itself and tells the owner
 * about is read again this often. */
const OWN_CLOCK_MS = 60_000;
/** How long that read is waited for before the owner is told what there is. */
const CLOCK_WAIT_MS = 20_000;
/** A wake that did not get its work done is tried again this soon. */
const ALARM_RETRY_MS = 60_000;
/** A copy read here that has not changed has its time written down
 * again no more often than this. */
const OWN_TIME_EVERY_MS = 5 * 60_000;
/** An action or a section this side's token was refused for is left
 * to a machine for this long before this side tries it again. */
const REFUSED_FOR_MS = 60 * 60_000;
/** The same, when it was turned away for calling too often: that
 * passes by itself, and soon. */
const LIMITED_FOR_MS = 5 * 60_000;
/** What a section says while this side's token is refused for it and
 * no machine has sent its copy yet. */
const REFUSED_READ = "Cloudflare was turned away when it read this, and no machine has reported it yet";
/** How many of this side's own calls are remembered, to tell what an
 * action that failed had got done. */
const OUTWARD_KEPT = 200;
/** How often the self-check may run again. */
const SELF_CHECK_EVERY_MS = 60 * 60_000;
const KEY_FETCH_EVERY_MS = 30_000;
const STATIC: Record<string, string> = {
  "app.js": "text/javascript; charset=utf-8",
  "outbox.js": "text/javascript; charset=utf-8",
  "app.css": "text/css; charset=utf-8",
  "icon.svg": "image/svg+xml",
  "sw.js": "text/javascript; charset=utf-8",
};
const ACT_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
/** How many actions done here under a name are remembered, for how
 * long, and how much of each answer is kept with it. A device offline
 * for longer than this that sends again is answered as if for the
 * first time; the names are its own, so nobody else can collide. */
const ACTED_KEPT = 300;
const ACTED_FOR_MS = 7 * 24 * 60 * 60_000;
const ACTED_BODY_CHARS = 4000;

/** An action done here under the name the device sent it with. */
interface Acted {
  at: number;
  kind: string;
  state: "running" | "done";
  status?: number;
  body?: Record<string, unknown>;
}
const ICONS = /^\/icons\/(icon-192|icon-512|icon-maskable-512|apple-touch-icon)\.png$/;

interface Mirror {
  at: number;
  hash: string;
  data: unknown;
  /** Read by this side itself, not reported by a machine. */
  own: boolean;
  /** How many stored pieces hold it. */
  chunks: number;
  /** The time the stored record of it says. */
  written: number;
}

/** What the stateless part of the Worker must do with a request this
 * part has checked: serve a file of the app, or pass the request to the
 * host. Anything else is an ordinary response. */
export function next(what: "asset" | "relay" | "key", detail: Record<string, unknown>): Response {
  return new Response(JSON.stringify(detail), {
    status: 200,
    headers: { "content-type": "application/json", "x-pm-next": what },
  });
}

const kvStore = (kv: KV, key: string): Store => ({
  read: async () => ((await kv.get(key)) as string | undefined) ?? null,
  write: (text) => kv.put(key, text),
});

export interface EdgeOptions {
  kv: KV;
  now?: () => number;
  /** Reaches the sign-in issuer's keys and the push services. */
  fetchImpl?: typeof fetch;
  /** What the edge does itself: the switch, the two tokens, and the
   * fetch its own reads and writes use. */
  direct?: {
    enabled: boolean;
    githubToken?: string;
    queueToken?: string;
    fetchImpl?: typeof fetch;
    /** Where GitHub's API and the queue store are, when a local run
     * puts stand-ins in their place. */
    githubApi?: string;
    queueDb?: string;
  };
  /** Ask to be woken at a time: to read again what the owner is told
   * about, and to notice a machine gone quiet. */
  setAlarm?: (at: number) => Promise<void>;
  /** Work that may finish after the answer has gone. */
  later?: (work: Promise<unknown>) => void;
}

/** An action this side tried and its token was refused for, with
 * nothing done: to be done by a machine instead. */
class HandOver extends Error {}

export class Edge {
  private kv: KV;
  private now: () => number;
  private fetchImpl: typeof fetch;
  private direct?: EdgeOptions["direct"];
  /** The console's own classes, wired for the edge and kept for as long
   * as the settings they were made for stand. */
  private kept: (DirectClasses & { cfg: EdgeConfig; table: Record<string, Action>; recalled: boolean }) | null = null;
  /** Sections read here that have been shown since this object started:
   * the first look shows the copy kept from before, and reads behind it. */
  private shown = new Set<string>();
  /** When what the owner is told about was last read by the clock. */
  private clockAt = 0;
  /** This side's latest calls of GitHub and the queue store, numbered. */
  private outward: (Outward & { n: number })[] = [];
  private outwardN = 0;
  /** Actions this side's token was turned away for, and until when
   * they are left to a machine. */
  private refused = new Map<string, number>();
  /** Sections this side's token was turned away for, and until when
   * they are a machine's to read, as they were before this side read
   * anything. */
  private unread = new Map<string, number>();
  /** Names of actions this object is doing right now. */
  private doing = new Set<string>();
  /** Both of those have been read back from storage. They are kept
   * there because this object is put away when idle: one that forgot
   * them would claim each section again on every start, and a machine,
   * told so, would never send its copy. */
  private awayRead = false;
  /** The time this object last asked to be woken at. */
  private alarmFor = 0;
  private later: (work: Promise<unknown>) => void;
  /** The last self-check: one direct read of each kind, and how it went. */
  private selfCheckResult: { at: number; kinds: Record<string, { ok: boolean; error?: string }> } | null = null;
  private setAlarm: (at: number) => Promise<void>;
  private cfg: EdgeConfig | null | undefined;
  private mirrors = new Map<string, Mirror | null>();
  private held: Held[] | null = null;
  private ownActs: Act[] | null = null;
  private hostActs: Act[] = [];
  private lastDocAt = 0;
  private lastKeyFetch = 0;
  private hostSeen: number | null = null;
  private hostSeq: number | null = null;
  /** For each section, the content the host last said it has and when
   * it read it: a copy with that content is current as of that read. */
  private confirmed: Record<string, { hash: string; at: number; error: string | null }> = {};
  private sure = 0;
  private unsure = 0;
  private hostVersion: string | null = null;
  private latestApk: ApkMeta | null | undefined;
  private keptApk: (ApkMeta & { chunks: number }) | null | undefined;
  /** The build that could not be kept when it was last sent, and when. */
  private apkFailed: { sha256: string; at: number } | null = null;
  /** What GitHub last said of the released build, when, whether it
   * could be asked at all, and if not, why. */
  private released: { at: number; asked: boolean; found: AndroidRelease | null; trouble: string | null } | null = null;
  private syncRefused: KeyedLimiter;
  readonly docs: Docs;
  readonly push: Push;
  readonly phones: Phones;
  private notifier: Notifier;
  private table: Record<string, Action>;

  constructor(options: EdgeOptions) {
    this.kv = options.kv;
    this.now = options.now ?? Date.now;
    this.fetchImpl = plain(options.fetchImpl ?? fetch);
    this.direct = options.direct;
    const setAlarm = options.setAlarm ?? (async () => {});
    this.setAlarm = async (at) => {
      this.alarmFor = at;
      await setAlarm(at);
    };
    this.later = options.later ?? (() => {});
    this.syncRefused = new KeyedLimiter(SYNC_REFUSALS, 60_000, this.now);
    this.docs = new Docs(
      Object.fromEntries(
        Object.entries(SECTIONS).map(([name, ttlMs]) => [
          name,
          {
            ttlMs,
            load: async () => {
              // What this side reads itself is read here; the rest is
              // the copy a machine last reported.
              const cfg = await this.config();
              if (this.owns(name, cfg)) return this.readOwn(name, cfg);
              const m = await this.mirror(name);
              // Left to a machine because this side was turned away for
              // it: said, with this side's last copy if it has one,
              // until a machine's comes.
              const away = this.unread.has(name);
              if (!m) throw new Error(away ? REFUSED_READ : "the laptop has not reported this yet");
              if (m.own && away) return new Stamped(m.data, m.at, REFUSED_READ);
              // With the laptop's own word on whether it could still read it.
              return new Stamped(m.data, m.at, this.confirmed[name]?.error ?? null);
            },
          },
        ]),
      ),
      this.now,
    );
    this.push = new Push(
      kvStore(this.kv, "push"),
      // The contact a push service is given: the project's page.
      () => contact(this.cfg?.repo ?? "developing-today/code"),
      (url, init) => this.fetchImpl(url, init as RequestInit),
      this.now,
    );
    const devices = new Devices(kvStore(this.kv, "devices"), this.now);
    this.phones = {
      devices,
      link: new Limiter(30, 60_000, this.now),
      refused: new Limiter(60, 60_000, this.now),
    };
    this.notifier = new Notifier(this.docs, this.push, kvStore(this.kv, "seen"), this.now);
    this.table = {
      ...ownActions(this.push, devices),
      // What this side was turned away for is tried here again now, not
      // when its time is up: for after the token has been put right.
      "edge.retry": {
        run: async () => {
          await this.ready();
          const tried = [...this.refusedKinds(), ...this.refusedReads().map((name) => `read.${name}`)];
          this.refused.clear();
          this.unread.clear();
          await this.saveAway();
          return { tried };
        },
        touch: [],
        audit: () => ({}),
      },
    };
  }

  // ---- the Android build -------------------------------------------------

  /** Where the release with the fixed address keeps a file. */
  private releaseUrl(cfg: EdgeConfig, file: string): string {
    return `https://github.com/${cfg.repo}/releases/download/${RELEASE_TAG}/${file}`;
  }

  /** The newest build GitHub Actions has released, by its own
   * description, or null when none is published. GitHub is not asked on
   * every request; when it cannot be asked, what was last known stands. */
  private async androidRelease(cfg: EdgeConfig): Promise<AndroidRelease | null> {
    const now = this.now();
    const known = this.released;
    if (known && now - known.at < (known.asked ? RELEASE_EVERY_MS : RELEASE_RETRY_MS)) return known.found;
    try {
      const res = await this.fetchImpl(this.releaseUrl(cfg, "version.json"), {
        redirect: "follow",
        signal: AbortSignal.timeout(RELEASE_ASK_MS),
      });
      if (res.status === 404) this.released = { at: now, asked: true, found: null, trouble: null };
      else if (res.ok) this.released = { at: now, asked: true, found: androidRelease(await res.json()), trouble: null };
      else throw new Error(`GitHub answered ${res.status}`);
    } catch (error) {
      // Why it could not be asked is kept, and health says it: "none
      // published" and "could not ask" are different things, and for a
      // day the second looked like the first.
      const e = error as Error;
      const trouble = `${e.name && e.name !== "Error" ? `${e.name}: ` : ""}${e.message}`.slice(0, 160);
      this.released = { at: now, asked: false, found: known?.found ?? null, trouble };
    }
    return this.released.found;
  }

  /** What is said of the released build: its name, its code and when it
   * was built. Null when none is published, or before the host has said
   * which repository this is. */
  private async releaseSaid(cfg: EdgeConfig | null): Promise<{ name: string; code: number; built: string } | null> {
    const release = cfg ? await this.androidRelease(cfg) : null;
    return release && { name: release.name, code: release.code, built: release.built };
  }

  /** The released build, as a download, or null if it did not arrive as
   * its description says (GitHub out of reach, or the description read
   * while a new build was being put up). */
  private async serveRelease(cfg: EdgeConfig, release: AndroidRelease): Promise<Response | null> {
    try {
      const res = await this.fetchImpl(this.releaseUrl(cfg, "pm-console.apk"), {
        redirect: "follow",
        signal: AbortSignal.timeout(RELEASE_FETCH_MS),
      });
      if (!res.ok) return null;
      const bytes = new Uint8Array(await res.arrayBuffer());
      const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
      const sha256 = [...digest].map((b) => b.toString(16).padStart(2, "0")).join("");
      if (bytes.byteLength !== release.size || sha256 !== release.sha256) {
        // The description is out of step with the file: read it again
        // next time instead of trusting it for ten minutes.
        this.released = null;
        return null;
      }
      return new Response(bytes, {
        headers: {
          "Content-Type": "application/vnd.android.package-archive",
          "Content-Disposition": 'attachment; filename="pm-console.apk"',
          "Cache-Control": "no-store",
          "X-Build": release.name,
          ...SECURITY,
        },
      });
    } catch {
      return null;
    }
  }

  /** What the app is told of the copy kept here. */
  private async apkState(): Promise<{ at: number; current: boolean } | null> {
    const kept = await this.apkKept();
    if (!kept) return null;
    const latest = await this.apkLatest();
    return { at: kept.at, current: !latest || latest.sha256 === kept.sha256 };
  }

  /** The build the host last said it has. */
  private async apkLatest(): Promise<ApkMeta | null> {
    if (this.latestApk === undefined)
      this.latestApk = ((await this.kv.get("apk:latest")) as ApkMeta | undefined) ?? null;
    return this.latestApk;
  }

  /** The build a copy of which is kept here. */
  private async apkKept(): Promise<(ApkMeta & { chunks: number }) | null> {
    if (this.keptApk === undefined) {
      this.keptApk = ((await this.kv.get("apk")) as (ApkMeta & { chunks: number }) | undefined) ?? null;
    }
    return this.keptApk;
  }

  /** Take in a build the host sent, if it is what the host says it is. */
  private async keepApk(build: ApkReport): Promise<void> {
    const bytes = Uint8Array.from(atob(build.data ?? ""), (c) => c.charCodeAt(0));
    const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
    const sha256 = [...digest].map((b) => b.toString(16).padStart(2, "0")).join("");
    if (bytes.byteLength !== build.size || sha256 !== build.sha256)
      throw new Error("the build sent is not the build described");
    // The record of the copy kept so far goes first. Its pieces are about
    // to be written over, and a write that stops half-way must not leave
    // pieces of two builds under a record that describes one of them.
    // With no record there is no copy: nothing is handed out, and the
    // build is asked for again.
    if (await this.apkKept()) {
      this.keptApk = null;
      await this.kv.delete("apk");
    }
    const chunks = Math.ceil(bytes.byteLength / APK_CHUNK);
    for (let i = 0; i < chunks; i++) await this.kv.put(`apk:${i}`, bytes.slice(i * APK_CHUNK, (i + 1) * APK_CHUNK));
    // Pieces left by a larger build, whether or not its record survived.
    for (let i = chunks; i < APK_PIECES; i++) await this.kv.delete(`apk:${i}`);
    this.keptApk = { sha256, size: build.size, at: build.at, chunks };
    await this.kv.put("apk", this.keptApk);
  }

  /** The kept build, as a download. */
  private async serveApk(kept: ApkMeta & { chunks: number }): Promise<Response> {
    const bytes = new Uint8Array(kept.size);
    let at = 0;
    for (let i = 0; i < kept.chunks; i++) {
      const part = (await this.kv.get(`apk:${i}`)) as Uint8Array | undefined;
      if (!part) return json({ error: "the copy of the Android build kept here is incomplete" }, 503);
      bytes.set(part, at);
      at += part.byteLength;
    }
    return new Response(bytes, {
      headers: {
        "Content-Type": "application/vnd.android.package-archive",
        "Content-Disposition": 'attachment; filename="pm-console.apk"',
        "Cache-Control": "no-store",
        "X-Build-At": String(kept.at),
        ...SECURITY,
      },
    });
  }

  // ---- what is kept ------------------------------------------------------

  private async config(): Promise<EdgeConfig | null> {
    if (this.cfg === undefined) this.cfg = edgeConfig(await this.kv.get("cfg"));
    return this.cfg;
  }

  private async mirror(name: string): Promise<Mirror | null> {
    const cached = this.mirrors.get(name);
    if (cached !== undefined) return cached;
    await this.ready();
    const meta = (await this.kv.get(`m:${name}`)) as
      | { at: number; hash: string; chunks: number; own?: boolean }
      | undefined;
    let found: Mirror | null = null;
    if (meta) {
      const parts: string[] = [];
      for (let i = 0; i < meta.chunks; i++) parts.push(String((await this.kv.get(`m:${name}:${i}`)) ?? ""));
      try {
        const own = meta.own === true;
        // As current as the host last said it was, which may be later
        // than when the copy itself was sent. A copy read here is as
        // current as that read, whatever a machine says of its own.
        const told = !own && this.confirmed[name]?.hash === meta.hash ? this.confirmed[name].at : 0;
        found = {
          at: Math.max(meta.at, told),
          hash: meta.hash,
          data: JSON.parse(parts.join("")),
          own,
          chunks: meta.chunks,
          written: meta.at,
        };
      } catch {
        found = null; // a copy that cannot be read is no copy: the host is asked for it again
      }
    }
    this.mirrors.set(name, found);
    return found;
  }

  /** Keep a copy of a section. `own` says this side read it itself. */
  private async putMirror(section: SyncSection, own = false): Promise<void> {
    const text = JSON.stringify(section.data);
    const chunks = Math.max(1, Math.ceil(text.length / CHUNK));
    const before = (await this.kv.get(`m:${section.name}`)) as { chunks: number } | undefined;
    for (let i = 0; i < chunks; i++)
      await this.kv.put(`m:${section.name}:${i}`, text.slice(i * CHUNK, (i + 1) * CHUNK));
    for (let i = chunks; i < (before?.chunks ?? 0); i++) await this.kv.delete(`m:${section.name}:${i}`);
    await this.kv.put(`m:${section.name}`, {
      at: section.at,
      hash: section.hash,
      chunks,
      ...(own ? { own: true } : {}),
    });
    this.mirrors.set(section.name, {
      at: section.at,
      hash: section.hash,
      data: section.data,
      own,
      chunks,
      written: section.at,
    });
  }

  /** Sections from the host, taken in; returns the names that changed. */
  private async take(sections: unknown): Promise<string[]> {
    const names: string[] = [];
    if (!Array.isArray(sections)) return names;
    const cfg = await this.config();
    for (const raw of sections as SyncSection[]) {
      if (!raw || typeof raw.name !== "string" || !Object.hasOwn(SECTIONS, raw.name)) continue;
      if (typeof raw.at !== "number" || typeof raw.hash !== "string" || raw.data === undefined) continue;
      // What this side reads itself is not taken from a machine: two
      // sources for one section would take turns being shown.
      if (this.owns(raw.name, cfg)) continue;
      const have = await this.mirror(raw.name);
      // An older read never replaces a newer one. (A copy this side read
      // itself, while it could, gives way to the machine's whenever it
      // comes: this side is no longer reading it.)
      if (have && !have.own && have.at > raw.at) continue;
      await this.putMirror({ name: raw.name, at: Math.min(raw.at, this.now()), hash: raw.hash, data: raw.data });
      names.push(raw.name);
    }
    if (names.length > 0) await this.docs.reread(names);
    return names;
  }

  private async heldList(): Promise<Held[]> {
    if (this.held === null) this.held = ((await this.kv.get("held")) as Held[] | undefined) ?? [];
    return this.held;
  }

  private async saveHeld(): Promise<void> {
    await this.kv.put("held", this.held ?? []);
    await this.showEdge();
  }

  /** What the app is told of this side, beside its state: how much is
   * waiting for a machine, and whether this side does the core itself. */
  private async showEdge(): Promise<void> {
    this.docs.extra("edge", { held: (this.held ?? []).length, primary: this.primary(await this.config()) });
  }

  /** What the owner did lately: this side's own record (what it ran or
   * is holding) together with the host's, newest first. */
  private async showActs(): Promise<void> {
    if (this.ownActs === null) this.ownActs = ((await this.kv.get("acts")) as Act[] | undefined) ?? [];
    const merged = [...this.ownActs, ...this.hostActs.filter((h) => !this.ownActs?.some((o) => o.id === h.id))];
    merged.sort((a, b) => (a.at < b.at ? 1 : a.at > b.at ? -1 : 0));
    this.docs.restoreActs(merged);
  }

  private async logAct(act: Act): Promise<void> {
    if (this.ownActs === null) this.ownActs = ((await this.kv.get("acts")) as Act[] | undefined) ?? [];
    this.ownActs = [act, ...this.ownActs.filter((a) => a.id !== act.id)].slice(0, ACTS_KEPT);
    await this.kv.put("acts", this.ownActs);
    await this.showActs();
  }

  private async dropAct(id: string): Promise<void> {
    if (this.ownActs === null) this.ownActs = ((await this.kv.get("acts")) as Act[] | undefined) ?? [];
    this.ownActs = this.ownActs.filter((a) => a.id !== id);
    await this.kv.put("acts", this.ownActs);
    await this.showActs();
  }

  private async audit(entry: Record<string, unknown>): Promise<void> {
    const log = ((await this.kv.get("audit")) as Record<string, unknown>[] | undefined) ?? [];
    log.push(entry);
    await this.kv.put("audit", log.slice(-AUDIT_KEPT));
  }

  private async ready(): Promise<void> {
    // After a restart: the held count and the record of what was done
    // are read back before the first document is built.
    if (this.held === null) {
      await this.heldList();
      await this.showEdge();
    }
    if (this.ownActs === null) await this.showActs();
    if (this.hostSeen === null) {
      const host = (await this.kv.get("host")) as
        | {
            seen: number;
            seq: number;
            confirmed?: Record<string, { hash: string; at: number; error: string | null }>;
            version?: string | null;
          }
        | undefined;
      this.hostSeen = host?.seen ?? 0;
      this.hostSeq = host?.seq ?? 0;
      this.confirmed = host?.confirmed ?? {};
      this.hostVersion = host?.version ?? null;
    }
    if (!this.awayRead) {
      this.awayRead = true;
      const kept = (await this.kv.get("away")) as
        | { kinds?: Record<string, unknown>; reads?: Record<string, unknown> }
        | undefined;
      const now = this.now();
      // Only what still stands, and never for longer than it could
      // have been set for.
      const still = (from: Record<string, unknown> | undefined, into: Map<string, number>) => {
        for (const [name, until] of Object.entries(from ?? {})) {
          if (typeof until === "number" && until > now && !into.has(name)) {
            into.set(name, Math.min(until, now + REFUSED_FOR_MS));
          }
        }
      };
      still(kept?.kinds, this.refused);
      still(kept?.reads, this.unread);
    }
  }

  /** Write down what this side is turned away for, so that an object
   * that starts again knows. */
  private async saveAway(): Promise<void> {
    await this.kv.put("away", { kinds: Object.fromEntries(this.refused), reads: Object.fromEntries(this.unread) });
  }

  // ---- doing the core here ------------------------------------------------

  /** The switch is on and both tokens are here. */
  private directCapable(): boolean {
    return this.direct?.enabled === true && Boolean(this.direct.githubToken) && Boolean(this.direct.queueToken);
  }

  /** The config carries what this side needs to do the core: a config
   * stored or reported before that existed has these empty, and the
   * edge serves the app from the mirror without acting itself. */
  private directConfigReady(cfg: EdgeConfig | null): boolean {
    return cfg !== null && cfg.queueDbUrl !== "" && cfg.queueTokenName !== "" && cfg.promptLabel !== "";
  }

  /** This side does the core itself: it has the switch, the tokens, and
   * has been told where things are. Whether a machine is reporting
   * makes no difference to this. */
  private primary(cfg: EdgeConfig | null): boolean {
    return this.directCapable() && this.directConfigReady(cfg);
  }

  /** A section this side reads itself. */
  private owns(name: string, cfg: EdgeConfig | null): boolean {
    return this.owned(cfg).includes(name);
  }

  /** The sections this side reads itself: all it can while it does the
   * core, less any its token was refused for within the hour. Those are
   * a machine's to read meanwhile, exactly as before this side read
   * anything: a token that may do less than was thought leaves the app
   * no worse off than it was. */
  private owned(cfg: EdgeConfig | null): string[] {
    if (!this.primary(cfg)) return [];
    const refused = this.refusedReads();
    return DIRECT_READ_SECTIONS.filter((name) => !refused.includes(name));
  }

  /** Sections this side's token was turned away for, and still is. */
  private refusedReads(): string[] {
    const now = this.now();
    for (const [name, until] of this.unread) if (now >= until) this.unread.delete(name);
    return [...this.unread.keys()].sort();
  }

  /** The console's own classes and its table of actions, wired for the
   * edge. Made once for the settings they were made from, and kept:
   * what they remember between reads is what keeps this side from
   * asking GitHub the same thing over again. Null while this side does
   * not do the core itself. */
  private mine(cfg: EdgeConfig | null): (DirectClasses & { table: Record<string, Action> }) | null {
    if (cfg === null || !this.primary(cfg)) return null;
    if (this.kept?.cfg !== cfg) {
      const made = directClasses(cfg, {
        githubToken: this.direct?.githubToken,
        queueToken: this.direct?.queueToken,
        githubApi: this.direct?.githubApi,
        queueDb: this.direct?.queueDb,
        fetchImpl: this.direct?.fetchImpl as never,
        now: this.now,
        seen: (call) => {
          this.outward.push({ ...call, n: ++this.outwardN });
          if (this.outward.length > OUTWARD_KEPT) this.outward.splice(0, this.outward.length - OUTWARD_KEPT);
        },
      });
      this.kept = {
        ...made,
        cfg,
        table: actions(made.ops, made.work, this.push, this.phones.devices, made.plan),
        recalled: false,
      };
    }
    return this.kept;
  }

  /** One section, read from its source by this side. The first look
   * since this object started shows the copy kept from before at once,
   * and it is read again behind that; a read that fails leaves the last
   * good copy and says why (Docs does both). Every good read is kept,
   * so a restart has it to show. */
  private async readOwn(name: string, cfg: EdgeConfig | null): Promise<unknown> {
    const mine = this.mine(cfg);
    if (!mine) throw new Error("this cannot be read from here right now");
    const before = await this.mirror(name);
    // The prompts kept from before say what each thread held: one that
    // has not changed since is not fetched again after a restart.
    if (name === "asks" && before && this.kept && !this.kept.recalled) {
      this.kept.recalled = true;
      mine.work.recall(before.data);
    }
    if (!this.shown.has(name)) {
      this.shown.add(name);
      if (before) return new Stamped(before.data, before.at, null, true);
    }
    const from = this.outwardN;
    let data: unknown;
    try {
      data = await directRead(name, mine.ops, mine.work);
    } catch (error) {
      // This side's token may not read this. For the next hour (or, if
      // it was only calling too often, a few minutes) it is a machine's
      // to read, as it was before: the copy a machine sent is shown if
      // there is one here, and its next reports bring it.
      const away = this.away(from, READ_FROM[name as keyof typeof READ_FROM]);
      if (away === null) throw error;
      this.unread.set(name, this.now() + away);
      await this.saveAway().catch((failed) => console.error(`pm-edge: keeping what was refused: ${failed.message}`));
      console.error(`pm-edge: reading ${name} was refused here; left to a machine`);
      const theirs = await this.mirror(name);
      if (!theirs || theirs.own) throw error;
      return new Stamped(theirs.data, theirs.at, this.confirmed[name]?.error ?? null);
    }
    await this.keepOwn(name, data).catch((error) =>
      console.error(`pm-edge: keeping ${name}: ${(error as Error).message}`),
    );
    return data;
  }

  /** Keep what this side just read. Content that has not changed is not
   * written again; only its time is, and that now and then. */
  private async keepOwn(name: string, data: unknown): Promise<void> {
    const now = this.now();
    const hash = fingerprint(data);
    const before = await this.mirror(name);
    if (before?.own && before.hash === hash) {
      before.at = now;
      before.data = data;
      if (now - before.written < OWN_TIME_EVERY_MS) return;
      before.written = now;
      await this.kv.put(`m:${name}`, { at: now, hash, chunks: before.chunks, own: true });
      return;
    }
    await this.putMirror({ name, at: now, hash, data }, true);
  }

  /** What this side reads itself that the owner is told about. */
  private ownedTold(cfg: EdgeConfig | null): string[] {
    return this.owned(cfg).filter((name) => NOTIFIED.includes(name));
  }

  /** Whether this side's token was turned away in a call since number
   * `from` (of these services only, if any are named), and if so for
   * how long what was being done is left to a machine: an hour when it
   * may not do the thing (401, 403), a few minutes when it was only
   * calling too often. Calls made at the same time by something else
   * are counted too: that can only hand a machine something this side
   * could have done, never keep from it what this side cannot. */
  private away(from: number, to?: Outward["to"][]): number | null {
    const mine = this.outward.filter((call) => call.n > from && (to === undefined || to.includes(call.to)));
    if (mine.some((call) => (call.status === 401 || call.status === 403) && !call.limited)) return REFUSED_FOR_MS;
    if (mine.some((call) => call.limited)) return LIMITED_FOR_MS;
    return null;
  }

  /** Whether something was written since call number `from`, or may
   * have been: a write that got no answer may have been taken. What
   * failed after that is not a machine's to do again from the start. */
  private wrote(from: number): boolean {
    return this.outward.some(
      (call) => call.n > from && call.write && (call.status === 0 || (call.status >= 200 && call.status < 300)),
    );
  }

  /** Actions this side's token was turned away for, and still is. */
  private refusedKinds(): string[] {
    const now = this.now();
    for (const [kind, until] of this.refused) if (now >= until) this.refused.delete(kind);
    return [...this.refused.keys()].sort();
  }

  /** The source of these sections has just changed: those this side
   * reads itself are read again from it, so the next document shows the
   * change. Never from the copy kept from before a restart, which the
   * first look would otherwise show: that copy is from before the
   * change. */
  private async reread(names: string[], cfg: EdgeConfig | null): Promise<void> {
    const mine = [...new Set(names)].filter((name) => this.owns(name, cfg));
    for (const name of mine) this.shown.add(name);
    if (mine.length > 0) await this.docs.reread(mine);
  }

  /** A machine reported within the minute, so it can be asked. */
  private machineThere(): boolean {
    return this.now() - (this.hostSeen ?? 0) < HOST_QUIET_MS;
  }

  /** Tell the owner what changed. What this side reads itself is
   * compared as it stands (after a first look, if this object has not
   * had one: the copy kept from before); the rest is read from the copy
   * a machine last sent. */
  private async tell(cfg: EdgeConfig | null): Promise<void> {
    const unread = this.ownedTold(cfg).filter((name) => !this.docs.tried(name));
    if (unread.length > 0) await this.docs.touch(unread);
    await this.notifier.tick(this.owned(cfg));
  }

  /** The self-check: one direct read of each kind, run when the
   * capability arrives and at most once an hour after, its result kept
   * for health. It proves the path without taking the laptop down. */
  private async maybeSelfCheck(cfg: EdgeConfig | null): Promise<void> {
    const direct = this.mine(cfg);
    if (!direct) return;
    const now = this.now();
    const stored = (await this.kv.get("selfcheck")) as
      | { at: number; kinds: Record<string, { ok: boolean; error?: string }> }
      | undefined;
    if (stored && now - stored.at < SELF_CHECK_EVERY_MS) {
      this.selfCheckResult = stored;
      return;
    }
    const kinds: Record<string, { ok: boolean; error?: string }> = {};
    for (const name of DIRECT_READ_SECTIONS) {
      try {
        await directRead(name, direct.ops, direct.work);
        kinds[name] = { ok: true };
      } catch (error) {
        kinds[name] = { ok: false, error: (error as Error).message.slice(0, 200) };
      }
    }
    this.selfCheckResult = { at: now, kinds };
    await this.kv.put("selfcheck", this.selfCheckResult);
  }

  /** The stand-in for the queue's enqueue in something this side files
   * itself: the issue already exists on GitHub, so only its queue task
   * is left, and filing a task is the fleet binary's. `later` is the
   * action a machine does in its place; it is held here until one
   * takes it. Too much already waiting is answered as "not queued", so
   * that the owner is told: the issue stands either way. */
  private async holdLater(later: Later, by: string): Promise<{ code: number; stdout: string; stderr: string }> {
    const issue = Number(later.args.issue);
    if (!Number.isSafeInteger(issue) || issue <= 0) {
      throw new Error("the issue could not be read from its task");
    }
    const held = await this.heldList();
    if (held.length >= HELD_MAX) {
      return { code: 1, stdout: "", stderr: "too much is already waiting for a machine" };
    }
    const at = new Date(this.now()).toISOString();
    held.push({ id: crypto.randomUUID(), kind: later.kind, args: later.args, who: by, at });
    await this.saveHeld();
    return { code: 0, stdout: "", stderr: "" };
  }

  /** An action this side runs itself through the console's own classes,
   * recorded before and after like own(). It is never sent to a machine,
   * so no machine can do it a second time. The sections it changed that
   * this side reads are read again before the answer goes back. */
  private async directAct(
    kind: string,
    args: Record<string, unknown>,
    by: string,
    via: string,
    id: string | null = null,
  ): Promise<Response> {
    const cfg = await this.config();
    const direct = this.mine(cfg);
    if (!direct) throw new Error("this cannot be done from here right now");
    const action = direct.table[kind];
    const base = { at: new Date(this.now()).toISOString(), who: by, via, action: kind, args: action.audit(args) };
    await this.audit({ ...base, phase: "start" });
    // On the record under the name it was sent with, so the device that
    // sent it can find how it came out.
    const entry = { id: id ?? crypto.randomUUID(), kind, at: base.at, by, note: "" };
    /** The task of a prompt: the packet carries the issue's number, and
     * the text is the owner's own. */
    const prompt = (packet: unknown) =>
      this.holdLater(
        {
          kind: "prompt.enqueue",
          args: {
            issue: Number((packet as { payload?: { issue?: { number?: unknown } } })?.payload?.issue?.number),
            text: String(args.text ?? ""),
            at: new Date(this.now()).toISOString(),
          },
        },
        by,
      );
    const from = this.outwardN;
    try {
      let result: unknown;
      if (kind === "ask.send") result = await direct.work.ask(args, prompt);
      else if (kind === "ask.reply") result = await direct.work.askReply(args, prompt);
      else if (kind === "intake.send")
        result = await direct.plan.intake(args, (_packet, later) => this.holdLater(later, by));
      else result = await action.run(args);
      await this.audit({ ...base, phase: "done", ok: true });
      await this.logAct({ ...entry, status: "done" });
      await this.reread(action.touch, cfg);
      if (action.touch.some((n) => WATCHED.includes(n))) await this.tell(cfg).catch(() => {});
      return json({ ok: true, result });
    } catch (error) {
      const away = this.wrote(from) ? null : this.away(from);
      if (away !== null) {
        // The token here may not do this, and nothing was written: it
        // is a machine's to do, now and for the next hour (a few
        // minutes, if it was only calling too often).
        this.refused.set(kind, this.now() + away);
        await this.saveAway().catch((failed) => console.error(`pm-edge: keeping what was refused: ${failed.message}`));
        await this.audit({ ...base, phase: "done", ok: false, error: "refused here; left to a machine" });
        throw new HandOver((error as Error).message);
      }
      await this.audit({ ...base, phase: "done", ok: false, error: (error as Error).message });
      await this.logAct({ ...entry, status: "failed", note: (error as Error).message.slice(0, 200) });
      throw error;
    }
  }

  // ---- the host's report -------------------------------------------------

  /** The host's public key, given by the stateless part after it asked
   * the host through the tunnel. */
  private async enroll(req: Request): Promise<Response> {
    const key = publicKey(((await req.json().catch(() => null)) as { key?: unknown } | null)?.key);
    if (!key) return json({ error: "not a key" }, 400);
    await this.kv.put("key", { kid: await keyId(key), jwk: key });
    return json({ ok: true });
  }

  private async sync(req: Request): Promise<Response> {
    // The laptop cannot sign in, so this path is open to anyone, and a
    // report may be megabytes. Every report that fails a check below
    // counts against the address it came from, and an address with too
    // many is refused here, before its body is read.
    const from = req.headers.get("cf-connecting-ip") ?? "unknown";
    if (this.syncRefused.blocked(from)) {
      return json({ error: "too many reports from this address were not the host's; try again in a minute" }, 429);
    }
    const refuse = (body: unknown, status: number) => {
      this.syncRefused.count(from);
      return json(body, status);
    };
    // Too large by its own account, or not shaped like a report: refused unread.
    if (Number(req.headers.get("content-length") ?? "0") > SYNC_LIMIT) return refuse({ error: "too large" }, 413);
    const kid = req.headers.get("x-pm-key") ?? "";
    const signature = req.headers.get("x-pm-signature") ?? "";
    if (kid === "" || !SIGNATURE.test(signature)) {
      return refuse({ error: "this is not signed with the host's key" }, 401);
    }
    const key = (await this.kv.get("key")) as { kid: string; jwk: PublicKey } | undefined;
    const now = this.now();
    if (!key || key.kid !== kid) {
      // A key this side has not seen: ask the host for its key, but not
      // on every stranger's request. Nothing has been read yet.
      if (req.headers.get("x-pm-key-fetched") === "1" || now - this.lastKeyFetch < KEY_FETCH_EVERY_MS) {
        return refuse({ error: "this is not signed with the host's key" }, 401);
      }
      this.lastKeyFetch = now;
      // The report is sent again once the key is known, from a second
      // copy of the same upload. This copy has to be read to its end
      // first: in the Worker runtime the two share one stream, and a
      // copy left unread holds the other back until the laptop gives up
      // waiting. (It is read at most once in KEY_FETCH_EVERY_MS, and
      // only up to what a report may be, so this is no way to make the
      // edge read at will.)
      await req.arrayBuffer().catch(() => {});
      return next("key", {});
    }
    const body = new Uint8Array(await req.arrayBuffer());
    if (body.byteLength > SYNC_LIMIT) return refuse({ error: "too large" }, 413);
    if (!(await verify(key.jwk, body, signature))) {
      return refuse({ error: "this is not signed with the host's key" }, 401);
    }
    // Signed by the host from here on: what is wrong with it is the
    // host's to hear about, and is not held against the address.
    let msg: SyncRequest;
    try {
      msg = JSON.parse(new TextDecoder().decode(body)) as SyncRequest;
    } catch {
      return json({ error: "body: expected JSON" }, 400);
    }
    if (msg?.v !== 1 || typeof msg.at !== "number") return json({ error: "not a sync this side understands" }, 400);
    if (Math.abs(msg.at - now) > SYNC_SKEW_MS) return json({ error: "clock", now }, 409);
    await this.ready();
    const last = this.hostSeq ?? 0;
    if (msg.at <= last) return json({ error: "replay", last }, 409);
    const cfg = edgeConfig(msg.config);
    if (!cfg) return json({ error: "config: not one this side can use" }, 400);
    if (JSON.stringify(cfg) !== JSON.stringify(await this.config())) {
      await this.kv.put("cfg", cfg);
      this.cfg = cfg;
      // New settings may be what lets this side do the core itself.
      await this.showEdge();
    }
    const settings = await this.config();
    // What the host says of every section it has: how current it is, and
    // whether it could still read it. Taken first, so a section that
    // arrives in this report is shown with its health as of this report.
    const told: { name: string; healthChanged: boolean }[] = [];
    for (const [name, theirs] of Object.entries(msg.have ?? {})) {
      if (!Object.hasOwn(SECTIONS, name) || typeof theirs?.hash !== "string" || typeof theirs.at !== "number") continue;
      // A machine's word on a section this side reads itself is not taken.
      if (this.owns(name, settings)) continue;
      const error = typeof theirs.error === "string" && theirs.error !== "" ? theirs.error.slice(0, 300) : null;
      told.push({ name, healthChanged: (this.confirmed[name]?.error ?? null) !== error });
      this.confirmed[name] = { hash: theirs.hash, at: Math.min(theirs.at, now), error };
    }
    const changed = await this.take(msg.sections);
    await this.phones.devices.mergeLinks(msg.phones);
    // A section that has not changed is not sent again: the copy here is
    // as current as the host says, and as healthy.
    const fresher: string[] = [];
    for (const { name, healthChanged } of told) {
      const copy = await this.mirror(name);
      if (!copy || copy.hash !== this.confirmed[name].hash) continue;
      if (this.confirmed[name].at > copy.at || healthChanged) {
        copy.at = Math.max(copy.at, this.confirmed[name].at);
        if (!changed.includes(name)) fresher.push(name);
      }
    }
    if (fresher.length > 0) await this.docs.reread(fresher);
    if (Array.isArray((msg as { acts?: unknown }).acts)) {
      this.hostActs = ((msg as { acts?: Act[] }).acts ?? []).slice(0, ACTS_KEPT);
      await this.showActs();
    }

    // What became of the held actions the host was given.
    const held = await this.heldList();
    let settled = false;
    const did: string[] = [];
    for (const ack of Array.isArray(msg.acks) ? msg.acks : []) {
      const at = held.findIndex((h) => h.id === ack?.id);
      if (at < 0) continue;
      const [done] = held.splice(at, 1);
      settled = true;
      if (ack.ok) {
        await this.dropAct(done.id); // the host's own record shows it done
        did.push(done.kind);
      } else {
        await this.logAct({
          id: done.id,
          kind: done.kind,
          at: done.at,
          by: done.who,
          status: "failed",
          note: `Sent late, and the laptop could not do it: ${String(ack.error ?? "no reason given").slice(0, 160)}`,
        });
      }
    }
    // What has waited too long is dropped, and said to have been.
    for (const old of held.filter((h) => now - Date.parse(h.at) > HELD_FOR_MS)) {
      held.splice(held.indexOf(old), 1);
      settled = true;
      await this.logAct({
        id: old.id,
        kind: old.kind,
        at: old.at,
        by: old.who,
        status: "failed",
        note: "Not sent: the laptop was out of reach for more than a day.",
      });
    }
    if (settled) await this.saveHeld();
    // What a machine has just done late changed things this side reads
    // itself: they are read again, so the next document shows it.
    if (did.length > 0) {
      const table = this.mine(settings)?.table ?? {};
      await this.reread(
        did.flatMap((kind) => (Object.hasOwn(table, kind) ? table[kind].touch : [])),
        settings,
      );
    }

    // The host is back, if it had been reported silent.
    const wasSilent = (await this.kv.get("silent")) === true;
    // One value, written once a report: when the host last reported, the
    // time its report carried (which the next must exceed), and how
    // current it says each section is.
    this.hostSeen = now;
    this.hostSeq = msg.at;
    this.hostVersion = typeof msg.version === "string" && /^[0-9a-f]{7,40}$/.test(msg.version) ? msg.version : null;
    await this.kv.put("host", { seen: now, seq: msg.at, confirmed: this.confirmed, version: this.hostVersion });
    if (wasSilent) {
      try {
        const delivery = await this.push.send("trouble", {
          title: "The laptop is back",
          body: "It is reporting again.",
          url: "/console/",
          tag: "host",
        });
        if (delivery.failed === 0) await this.kv.delete("silent");
      } catch {
        // Keep the transition pending so the next host report can retry it.
      }
    }
    await this.schedule(settings);
    // Something the owner is told about changed, or a unit's failing is
    // being timed: that clock only moves when the laptop's fresh word on
    // its units is looked at, and a unit that stays failed reads the
    // same in every report.
    if (changed.some((n) => WATCHED.includes(n)) || (await this.notifier.pending())) {
      await this.tell(settings).catch((error) => console.error(`pm-edge: notify: ${(error as Error).message}`));
    }

    // The Android build: what the host has now, and the build itself
    // when the host sent it because this side asked.
    const build = apkReport(msg.apk);
    if (
      JSON.stringify(build && { sha256: build.sha256, size: build.size, at: build.at }) !==
      JSON.stringify(await this.apkLatest())
    ) {
      this.latestApk = build && { sha256: build.sha256, size: build.size, at: build.at };
      if (this.latestApk) await this.kv.put("apk:latest", this.latestApk);
      else await this.kv.delete("apk:latest");
    }
    if (build?.data) {
      await this.keepApk(build).catch((error) => {
        // Not kept (it did not match its description, or could not be
        // stored). It is not asked for again at once: a build that
        // fails every time would otherwise be sent with every report.
        this.apkFailed = { sha256: build.sha256, at: now };
        console.error(`pm-edge: apk: ${(error as Error).message}`);
      });
    }

    // What the host has and this side has lost (its storage was reset).
    const want: string[] = [];
    const kept = await this.apkKept();
    const failedLately =
      this.apkFailed?.sha256 === this.latestApk?.sha256 && now - (this.apkFailed?.at ?? 0) < APK_RETRY_MS;
    if (this.latestApk && kept?.sha256 !== this.latestApk.sha256 && !failedLately) want.push("apk");
    for (const name of Object.keys(SECTIONS)) {
      if (typeof msg.have?.[name]?.hash !== "string" || this.owns(name, settings)) continue;
      // Also wanted: a section this side used to read itself and no
      // longer does. Its copy here is this side's own, and the machine,
      // which thinks it sent its own long ago, would not send it again.
      const copy = await this.mirror(name);
      if (!copy || copy.own) want.push(name);
    }
    const answer: SyncResponse = {
      ok: true,
      watching: now - this.lastDocAt < WATCHING_MS,
      notify: await this.someoneToTell(),
      want,
      commands: held,
      phones: await this.phones.devices.links(),
      // What this side reads itself, and so need not be sent.
      owns: this.owned(settings),
    };
    return json(answer);
  }

  /** A device takes notifications, or a phone is linked. */
  private async someoneToTell(): Promise<boolean> {
    return (await this.push.status()).devices > 0 || (await this.phones.devices.list()).length > 0;
  }

  /** Ask to be woken next: to notice a machine gone quiet, and, while
   * this side does the core itself and there is someone to tell, to
   * read again what the owner is told about. */
  private async schedule(cfg: EdgeConfig | null): Promise<void> {
    const now = this.now();
    const wakes: number[] = [];
    if (this.hostSeen && (await this.kv.get("silent")) !== true) {
      // When the check would be due; or, if it was and the owner could
      // not be told, soon again.
      wakes.push(Math.max(this.hostSeen + SILENT_MS + 1000, now + ALARM_RETRY_MS));
    }
    if (await this.clockWanted(cfg)) wakes.push(Math.max(now + 1000, this.clockAt + OWN_CLOCK_MS));
    // Something the owner is told about that this side was turned away
    // for is tried again when its time is up; with nobody looking and
    // no machine reporting, only a wake would notice that it is.
    if (this.primary(cfg) && (await this.push.status()).devices > 0) {
      for (const name of this.refusedReads()) {
        if (NOTIFIED.includes(name)) wakes.push(Math.max(now + 1000, (this.unread.get(name) ?? now) + 1000));
      }
    }
    if (wakes.length > 0) await this.setAlarm(Math.min(...wakes));
  }

  /** This side reads something the owner is told about, and a device
   * is there to be told: it has to wake by itself to read it. */
  private async clockWanted(cfg: EdgeConfig | null): Promise<boolean> {
    return this.ownedTold(cfg).length > 0 && (await this.push.status()).devices > 0;
  }

  /** Woken. Two things are looked at, each when it is due. A machine
   * that has not reported for ten minutes: the owner is told once. And,
   * while this side does the core itself and someone is there to tell:
   * what it reads is read again, and the owner told of what changed.
   * That is what makes notifications need no machine. */
  async alarm(): Promise<void> {
    await this.ready();
    const cfg = await this.config();
    const now = this.now();
    if (this.hostSeen && now - this.hostSeen >= SILENT_MS && (await this.kv.get("silent")) !== true) {
      try {
        const delivery = await this.push.send("trouble", {
          title: "The laptop has gone quiet",
          body: this.primary(cfg)
            ? "It has not reported for ten minutes. Cloudflare carries on with the board, your prompts, pull requests and the plan; what needs the laptop waits for it."
            : "It has not reported for ten minutes. The app still opens; what you do is held until it is back.",
          url: "/console/",
          tag: "host",
        });
        if (delivery.failed === 0) await this.kv.put("silent", true);
      } catch {
        // Leave the transition pending so a later alarm can retry it.
      }
    }
    if (now - this.clockAt >= OWN_CLOCK_MS && (await this.clockWanted(cfg))) {
      this.clockAt = now;
      // Read again what is stale, and wait for it: this wake is the
      // only thing looking. Twice, because the first look of an object
      // that has just started shows the copy kept from before, and it
      // is the second that reads.
      for (let look = 0; look < 2; look++) await this.docs.current(this.ownedTold(cfg), CLOCK_WAIT_MS);
      await this.tell(cfg).catch((error) => console.error(`pm-edge: notify: ${(error as Error).message}`));
    }
    await this.schedule(cfg);
  }

  // ---- requests ----------------------------------------------------------

  async fetch(req: Request): Promise<Response> {
    const url = new URL(req.url);
    const path = url.pathname;
    const method = req.method;

    // From the stateless part of this Worker only: it never passes a
    // request from outside to a path under /internal/.
    if (path.startsWith("/internal/")) {
      if (method !== "POST") return json({ error: "not found" }, 404);
      if (path === "/internal/key") return this.enroll(req);
      if (path === "/internal/hold") return this.hold(req);
      if (path === "/internal/relayed") return this.relayed(req);
      return json({ error: "not found" }, 404);
    }

    const cfg = await this.config();
    // Public: nothing here says anything about the fleet.
    if (path === `${BASE}/health` && method === "GET") {
      await this.ready();
      // The self-check runs when the capability is there: one direct
      // read of each kind, and how it went. Nothing here is a secret.
      await this.maybeSelfCheck(cfg);
      return json({
        ok: true,
        edge: true,
        configured: cfg !== null,
        directEnabled: this.direct?.enabled === true,
        githubTokenConfigured: Boolean(this.direct?.githubToken),
        queueTokenConfigured: Boolean(this.direct?.queueToken),
        // This side does the core itself: the switch and the tokens are
        // here, and it has been told where things are. `directReady` is
        // the same thing under the name it had when this side acted
        // only while the laptop was away.
        primary: this.primary(cfg),
        directReady: this.primary(cfg),
        // What this side's token was refused for within the hour, and
        // a machine does instead: actions by their names, and sections
        // it could not read as `read.<section>`.
        refused: [...this.refusedKinds(), ...this.refusedReads().map((name) => `read.${name}`)],
        selfCheck: this.selfCheckResult,
        hostSeen: this.hostSeen || null,
        // The commit the host's console is running, as it last said.
        hostVersion: this.hostVersion,
        // The copy of the Android build kept here: when it was built,
        // and whether it is the host's newest. Null if there is none.
        androidBuild: await this.apkState(),
        // The newest build of the app GitHub has released, as last read
        // here (within ten minutes): what a download is, when there is
        // one. The release is public; this is how its arrival here is
        // checked from outside. Null when none is published.
        androidRelease: await this.releaseSaid(cfg),
        // Why GitHub could not be asked, the last time it was tried;
        // null when it answered (also when it answered "none").
        androidReleaseTrouble: this.released?.trouble ?? null,
        // Since this object last started: requests it answered as the
        // owner's, and requests it left to the host to judge.
        answered: this.sure,
        deferred: this.unsure,
      });
    }
    if (path === "/manifest.webmanifest" && method === "GET") return manifest();
    if (ICONS.test(path) && (method === "GET" || method === "HEAD")) return next("asset", { path, type: "image/png" });
    if (path === `${BASE}/edge/sync` && method === "POST") return this.sync(req);
    // Before the host's first report this side knows no owner: it stays
    // out of the way, and the host answers as it did before.
    if (cfg === null) return next("relay", { mode: "page" });
    if (path === "/.well-known/assetlinks.json" && method === "GET") return json(assetLinks(cfg));
    // Not behind the sign-in, and checked here instead: a linked phone.
    if (path === `${BASE}/device/link` && method === "POST") {
      return deviceLink(req, this.phones, (entry) => this.audit(entry));
    }
    if (path === `${BASE}/device/summary` && method === "GET") return deviceSummary(req, this.docs, this.phones);

    // This side answers only when it is sure of the owner. When it is
    // not, it does not refuse: it passes the request to the host, which
    // decides as it always has. So a mistake here can cost the speed of
    // the edge, never the owner's way in, and a stranger gets from the
    // host exactly what a stranger got before.
    let who: Identity;
    try {
      who = await identify(req, cfg, { peer: null, fetchImpl: this.fetchImpl as never, now: this.now() });
      this.sure++;
    } catch (error) {
      if (!(error instanceof AuthError)) throw error;
      this.unsure++;
      return next("relay", { mode: "page" });
    }
    const by = who.email ?? who.certName ?? who.sub ?? who.via;
    const ours = path === BASE || path.startsWith(`${BASE}/`);
    if (!ours) {
      // The front door is the app; every other page is the host's.
      if (method === "GET" && path === "/" && url.search === "") return Response.redirect(`${url.origin}${BASE}/`, 302);
      return next("relay", { mode: "page" });
    }

    const rest = path.slice(BASE.length).replace(/^\/+/, "");
    if (method === "GET" && rest === "") {
      if (path === BASE) return Response.redirect(`${url.origin}${BASE}/`, 308);
      return next("asset", { path: `${BASE}/index.html`, type: "text/html; charset=utf-8" });
    }
    if (method === "GET" && Object.hasOwn(STATIC, rest)) {
      return next("asset", { path: `${BASE}/${rest}`, type: STATIC[rest] });
    }
    if (method === "GET" && rest === "pm-console.apk") {
      // A build GitHub Actions has released comes first: it is the
      // newest there is, and getting it needs nothing of the laptop.
      const release = await this.androidRelease(cfg);
      const released = release ? await this.serveRelease(cfg, release) : null;
      if (released) return released;
      // Otherwise the newest build is the laptop's. The copy here is
      // served when it is that build, or when the laptop is not there
      // to ask. After a restart, when the laptop was last heard from
      // has to be read back first, or it would look as if it never had
      // been.
      await this.ready();
      const kept = await this.apkKept();
      const latest = await this.apkLatest();
      const quiet = this.now() - (this.hostSeen ?? 0) >= HOST_QUIET_MS;
      if (kept && (kept.sha256 === latest?.sha256 || quiet)) return this.serveApk(kept);
    }
    if (!rest.startsWith("api/")) return next("relay", { mode: "live" });

    const seg = rest.slice(4).split("/");
    await this.ready();
    try {
      if (method === "GET") {
        if (seg.length === 1 && seg[0] === "me") {
          return json({
            ...who,
            repo: cfg.repo,
            queue: cfg.queue,
            // There is a build to hand out if the laptop has one or a
            // copy is kept here; `androidBuild` says of the copy when it
            // was built and whether it is the laptop's newest.
            androidApp: cfg.androidApp || (await this.apkKept()) !== null,
            androidBuild: await this.apkState(),
            androidPackage: cfg.androidPackage,
            edge: true,
          });
        }
        if (seg.length === 2 && seg[0] === "doc" && DOC_NAMES.includes(seg[1] as DocName)) {
          this.lastDocAt = this.now();
          const res = docResponse(req, await this.docs.doc(seg[1] as DocName));
          // What the document started reading behind this answer (the
          // sections this side reads itself, when they have gone stale)
          // is given the time to finish.
          this.later(this.docs.settled(this.owned(cfg)));
          // The clock that tells the owner of changes starts itself.
          if (this.alarmFor <= this.now()) await this.schedule(cfg);
          // When the host last reported, beside the document rather than
          // in it: it changes with every report and the content does not.
          const headers = new Headers(res.headers);
          headers.set("X-Host-Seen", String(this.hostSeen ?? 0));
          return new Response(res.body, { status: res.status, headers });
        }
        if (seg.length === 1 && seg[0] === "android") {
          // Where a download would come from: the build GitHub released,
          // if there is one, and the copy of the laptop's build kept here.
          return json({ release: await this.releaseSaid(cfg), copy: await this.apkState() });
        }
        if (seg.length === 1 && seg[0] === "push") return json(await this.push.status());
        if (seg.length === 1 && seg[0] === "devices") return json(await this.phones.devices.list());
        // What needs only GitHub and the queue store is read here, by
        // the code a machine would read it with.
        const mine = this.mine(cfg);
        if (mine) {
          const from = this.outwardN;
          let read: unknown;
          try {
            read = await platformRead(seg, url.searchParams, mine.ops, mine.plan);
          } catch (error) {
            // Refused here: a machine has its own way in, and a read may
            // be asked twice. Anything else is said as it is; so is this,
            // when there is no machine to ask.
            if (this.away(from) === null) throw error;
            if (!this.machineThere()) {
              throw new OpError(
                503,
                "Cloudflare was turned away when it read this, and no machine is reporting that could read it instead.",
              );
            }
          }
          // Some of these say what they could not read in place of
          // failing (a list with its error beside it). One that was
          // refused part-way is a machine's too, when one is there; with
          // none, what could be read is better than nothing.
          if (read !== undefined && !(this.away(from) !== null && this.machineThere())) return json(read);
        }
        // Everything else is read from the host as it is now.
        return next("relay", { mode: "live" });
      }
      if (method !== "POST") return json({ error: "method not allowed" }, 405);
      if (!sameSiteWrite(req)) return json({ error: "this request did not come from the console" }, 403);
      if (!(seg.length === 1 && seg[0] === "act")) return next("relay", { mode: "live" });
      if (Number(req.headers.get("content-length") ?? "0") > ACT_LIMIT) return json({ error: "too large" }, 413);
      let body: { kind?: unknown; args?: unknown; id?: unknown };
      try {
        const text = await req.text();
        if (text.length > ACT_LIMIT) return json({ error: "too large" }, 413);
        body = JSON.parse(text);
        if (body === null || typeof body !== "object" || Array.isArray(body)) throw new Error("not an object");
      } catch {
        return json({ error: "body: expected a JSON object" }, 400);
      }
      const kind = typeof body.kind === "string" ? body.kind : "";
      const args = body.args ?? {};
      if (args === null || typeof args !== "object" || Array.isArray(args)) {
        return json({ error: "args: expected an object" }, 400);
      }
      // The name the device gave this, if it gave one: whatever does
      // it, here or on a machine, does it once under that name.
      const id = typeof body.id === "string" && ACT_ID.test(body.id) ? body.id : null;
      if (!Object.hasOwn(this.table, kind)) {
        // One this side does itself: it runs here, through the
        // console's own classes, and is never sent to a machine, so no
        // machine can do it a second time. Otherwise it is a machine's
        // to do.
        if (
          (DIRECT_ACTIONS as readonly string[]).includes(kind) &&
          this.primary(cfg) &&
          !this.refusedKinds().includes(kind)
        ) {
          try {
            return await this.once(id, kind, () =>
              this.directAct(kind, args as Record<string, unknown>, by, who.via, id),
            );
          } catch (error) {
            // Refused here before anything was done: a machine's, below.
            if (!(error instanceof HandOver)) throw error;
          }
        }
        // What may be done late goes behind anything already waiting
        // for a machine that is about the same thing. Sent straight to
        // a machine that has just come back, it would be done before
        // the older one, which would undo it when its turn came.
        if (HOLDABLE.has(kind)) {
          const about = subjectOf(kind, args as Record<string, unknown>);
          if (about !== null && (await this.heldList()).some((h) => subjectOf(h.kind, h.args) === about)) {
            return this.keep({ id: id ?? crypto.randomUUID(), kind, args, who: by });
          }
        }
        // The host's to do. The stateless part relays it, and if the
        // host cannot be reached comes back to have it held.
        // It goes under the device's name for it, or one given here,
        // before it goes anywhere. If the host's answer is lost on the
        // way back, the action is held under the same name, and the
        // host, which wrote the name down before starting, does not do
        // it a second time.
        return next("relay", {
          mode: "act",
          id: id ?? crypto.randomUUID(),
          kind,
          args,
          who: by,
          holdable: HOLDABLE.has(kind),
        });
      }
      return await this.once(id, kind, () => this.own(kind, args as Record<string, unknown>, by, who.via, id));
    } catch (error) {
      return this.failed(error, `${method} ${path}`);
    }
  }

  /** What a thrown error is answered with. */
  private failed(error: unknown, what: string): Response {
    if (error instanceof OpError) return json({ error: error.message }, error.status);
    console.error(`pm-edge: ${what}: ${(error as Error).message}`);
    return json({ error: "the edge could not do that" }, 500);
  }

  /** Do something once under the name the device sent it with. A device
   * that is offline keeps what the owner did and sends it when it is
   * connected again, and sends it again if it never heard back: the
   * second arrival is answered from what was written down, not done
   * again. With no name it is simply done. `work` may hand the action
   * over to a machine (HandOver): then nothing was done here, nothing
   * is written down, and the machine keeps its own record under the
   * same name. */
  private async once(id: string | null, kind: string, work: () => Promise<Response>): Promise<Response> {
    if (id === null) return work();
    // Taken for doing here and now, before anything is waited for: of
    // two arrivals under one name, only one can take it.
    if (this.doing.has(id)) return json({ error: "This is still being done.", running: true }, 409);
    this.doing.add(id);
    const key = `a:${id}`;
    try {
      const before = ((await this.kv.get(key)) as Acted | undefined) ?? null;
      if (before?.state === "done") return json({ ...before.body, again: true }, before.status ?? 200);
      if (before?.state === "running") {
        // Started, and not by this object (it would hold the name):
        // one that was put away before it ended. Whether it happened
        // is not known, and doing it again could do it twice.
        const body = { error: "Cloudflare restarted while doing this; check whether it happened before trying again." };
        await this.kv.put(key, { at: this.now(), kind, state: "done", status: 502, body } satisfies Acted);
        return json({ ...body, again: true }, 502);
      }
      await this.kv.put(key, { at: this.now(), kind, state: "running" } satisfies Acted);
      await this.actedIndex(id);
      let res: Response;
      try {
        res = await work();
      } catch (error) {
        if (error instanceof HandOver) {
          await this.kv.delete(key);
          throw error;
        }
        res = this.failed(error, `${kind} (${id})`);
      }
      const body = (await res
        .clone()
        .json()
        .catch(() => null)) as Record<string, unknown> | null;
      const small = body !== null && JSON.stringify(body).length <= ACTED_BODY_CHARS;
      await this.kv.put(key, {
        at: this.now(),
        kind,
        state: "done",
        status: res.status,
        // An answer too long to keep is kept as how it went, without what it said.
        body: small ? body : res.ok ? { ok: true, result: null } : { error: "it could not be done" },
      } satisfies Acted);
      return res;
    } finally {
      this.doing.delete(id);
    }
  }

  /** Note a name as used, and forget the ones that are too old or too many. */
  private async actedIndex(id: string): Promise<void> {
    const now = this.now();
    const index = ((await this.kv.get("acted")) as { id: string; at: number }[] | undefined) ?? [];
    index.push({ id, at: now });
    const keep = index.filter((entry) => now - entry.at < ACTED_FOR_MS).slice(-ACTED_KEPT);
    const kept = new Set(keep.map((entry) => entry.id));
    for (const entry of index) if (!kept.has(entry.id)) await this.kv.delete(`a:${entry.id}`);
    await this.kv.put("acted", keep);
  }

  /** An action this side runs itself, recorded before and after. */
  private async own(
    kind: string,
    args: Record<string, unknown>,
    by: string,
    via: string,
    id: string | null = null,
  ): Promise<Response> {
    const action = this.table[kind];
    const base = { at: new Date(this.now()).toISOString(), who: by, via, action: kind, args: action.audit(args) };
    await this.audit({ ...base, phase: "start" });
    const entry = { id: id ?? crypto.randomUUID(), kind, at: base.at, by, note: "" };
    try {
      const result = await action.run(args);
      await this.audit({ ...base, phase: "done", ok: true });
      await this.logAct({ ...entry, status: "done" });
      const cfg = await this.config();
      // A device has just subscribed: take stock now, so that it is told
      // of what changes from here on and not of what was already there.
      if (kind === "push.on") await this.tell(cfg).catch(() => {});
      // Who there is to tell may have changed, and with it whether this
      // side needs waking to read.
      await this.schedule(cfg).catch(() => {});
      return json({ ok: true, result });
    } catch (error) {
      await this.audit({ ...base, phase: "done", ok: false, error: (error as Error).message });
      await this.logAct({ ...entry, status: "failed", note: (error as Error).message.slice(0, 200) });
      throw error;
    }
  }

  /** The host could not be reached: keep the action for it, if it is one
   * that may be done late. */
  private async hold(req: Request): Promise<Response> {
    return this.keep(
      (await req.json().catch(() => null)) as {
        id?: unknown;
        kind?: unknown;
        args?: unknown;
        who?: unknown;
      } | null,
    );
  }

  /** Keep an action for a machine, at the back of what already waits. */
  private async keep(body: { id?: unknown; kind?: unknown; args?: unknown; who?: unknown } | null): Promise<Response> {
    const kind = typeof body?.kind === "string" ? body.kind : "";
    const args = body?.args;
    if (!HOLDABLE.has(kind) || args === null || typeof args !== "object" || Array.isArray(args)) {
      return json(
        // No answer is not "not done": the laptop may have got it and its
        // answer been lost. Nothing is kept to do later, and it says so.
        {
          error:
            "The laptop did not answer, and this is not something to do late, so it is not kept. Check whether it happened before trying again.",
        },
        503,
      );
    }
    await this.ready();
    const held = await this.heldList();
    // Offered again under a name that is already waiting (the device
    // never heard that it was kept): it is kept once.
    if (typeof body?.id === "string" && held.some((h) => h.id === body.id)) {
      return json({ ok: true, held: true, result: null, again: true });
    }
    if (held.length >= HELD_MAX) {
      return json({ error: "The laptop cannot be reached, and too much is already waiting for it." }, 503);
    }
    const item: Held = {
      // The name it was relayed under, so the host can tell it has seen it.
      id: typeof body?.id === "string" && ACT_ID.test(body.id) ? body.id : crypto.randomUUID(),
      kind,
      args: args as Record<string, unknown>,
      who: String(body?.who ?? "owner"),
      at: new Date(this.now()).toISOString(),
    };
    held.push(item);
    await this.saveHeld();
    await this.audit({ at: item.at, who: item.who, via: "edge", action: kind, phase: "held", id: item.id });
    await this.logAct({
      id: item.id,
      kind,
      at: item.at,
      by: item.who,
      status: "running",
      note: "Held: the laptop cannot be reached. It will be sent when it is back.",
    });
    return json({ ok: true, held: true, result: null });
  }

  /** After the host did a relayed action: the sections it read again and
   * its record of what was done, so the next document already shows it. */
  private async relayed(req: Request): Promise<Response> {
    const body = (await req.json().catch(() => null)) as { sections?: unknown; acts?: unknown } | null;
    await this.ready();
    // What the machine read again after doing this, and this side reads
    // itself, is not taken from it. That it was sent says its source
    // has just changed, so it is read again here: first, so that one
    // this side turns out to be refused for is taken from this answer.
    const sent = (Array.isArray(body?.sections) ? body.sections : [])
      .map((section) => (section as { name?: unknown } | null)?.name)
      .filter((name): name is string => typeof name === "string");
    await this.reread(sent, await this.config());
    const changed = await this.take(body?.sections);
    if (Array.isArray(body?.acts)) {
      this.hostActs = (body.acts as Act[]).slice(0, ACTS_KEPT);
      await this.showActs();
    }
    return json({ ok: true, changed });
  }
}
