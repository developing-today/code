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
  ALLOWANCES,
  directRead,
  type GitHubMemory,
  githubMemory,
  type Outward,
  QUIET_MOST_MS,
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
  type HeldFor,
  HOLDABLE,
  keyId,
  type MachineReport,
  type MachinesAnswer,
  machineReport,
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
import { goesBy, platformSettings, settingsDifferences, settingsThatDiffer } from "../../src/settings.ts";
import type { Store } from "../../src/store.ts";
import { timed, unread } from "../../src/timed.ts";

/** A rank set twice ends at the last one: an older held rank never
 * runs when a newer rank for the same issue is done. Anything else
 * held runs first, in the order it was done. */
const LAST_WINS = new Set(["issue.rank"]);

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
  done: 120_000,
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
/** After a pass over what is held that could do none of it, how long
 * before another is tried. */
const DRAIN_AGAIN_MS = 60_000;
/** Said of something begun by an object that was put away before it
 * ended. */
const RESTARTED = "Cloudflare restarted while doing this; check whether it happened before trying again.";
/** What the owner is told of something held, by who it waits for. */
const HELD_SAID: Record<HeldFor, string> = {
  either:
    "Held: it could not be done just now. Cloudflare does it as soon as it can again, or the laptop if it is back first.",
  machine: "Held: the laptop cannot be reached. It will be sent when it is back.",
  asked: "Held: the laptop did not answer. It may have got this, and says what became of it when it is back.",
};
const ACTS_KEPT = 30;
const AUDIT_KEPT = 300;
/** "Someone is looking" lasts this long after the last read of a document. */
const WATCHING_MS = 90_000;
/** The host is reported silent after this long without a sync. */
export const SILENT_MS = 10 * 60_000;
/** With nobody looking, what the edge reads itself and tells the owner
 * about is read again this often. */
const OWN_CLOCK_MS = 60_000;
/** While the app is connected, how often this side wakes to read again
 * what has gone stale (each list by its own age), so that what changed
 * is said to the app without the app asking. */
export const LIVE_MS = 20_000;
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
/** A check in which something could not be read is made again this
 * soon: what stopped it (one request too many, a moment's outage) has
 * usually passed, and a failure should not stand for an hour. */
const SELF_CHECK_AGAIN_MS = 5 * 60_000;
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
  /** "given": handed to a machine under this name. Nothing was done
   * here, and only the machine knows what became of it. */
  state: "running" | "done" | "given";
  status?: number;
  body?: Record<string, unknown>;
  /** For "given": how many times it has set off toward a machine. Word
   * that one asking never arrived says the action reached no machine
   * only when that asking was the only one. (A record from before this
   * was counted has none, and is taken to have set off already.) */
  relays?: number;
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

/** One stored piece of a section, marked with whose it is. Pieces are
 * written first and the index (`m:<name>`) last: a write stopped
 * between pieces leaves pieces the index never names, and an index
 * naming pieces another section's write left behind never matches,
 * so neither reads as the section. */
interface MirrorPiece {
  n: string;
  h: string;
  t: string;
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
    /** Which kinds this object does itself: every one it can
     * (DIRECT_ACTIONS) unless said. A test says, to stand for a kind
     * only a machine can do that is about the same thing as one done
     * here: since "done" became this side's no kind of today's is, and
     * the rule for one (what comes after it waits behind it) has to
     * hold for the next that is. */
    kinds?: readonly string[];
  };
  /** The live connections to the app, which the platform holds (they
   * stay open while this object is put away): take a request to be
   * one and give the answer for it, say something to all of them, and
   * how many there are. Absent where there are none to hold (a test
   * that has no need of them): nobody is connected then. */
  sockets?: {
    accept: (req: Request) => Response;
    send: (text: string) => number;
    count: () => number;
  };
  /** Ask to be woken at a time: to read again what the owner is told
   * about, and to notice a machine gone quiet. */
  setAlarm?: (at: number) => Promise<void>;
  /** When this object has asked to be woken, as the platform holds it:
   * null when it has not. For health. */
  getAlarm?: () => Promise<number | null>;
  /** Work that may finish after the answer has gone. */
  later?: (work: Promise<unknown>) => void;
  /** How often the object's own clock wakes to read again what the
   * owner is told about, in milliseconds. A local run shortens it so
   * a smoke run can watch an alarm fire; where this is deployed it
   * is left unset and the minute stands. */
  ownClockMs?: number;
}

/** An action this side tried and its token was refused for, with
 * nothing done: to be done by a machine instead. */
class HandOver extends Error {}

/** What GitHub last said of the released build: when it was asked,
 * whether it could be asked at all, what it described, and if it could
 * not be asked, why. */
interface Released {
  at: number;
  asked: boolean;
  found: AndroidRelease | null;
  trouble: string | null;
}

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
  /** What this side remembers of GitHub between calls: its answers by
   * address, asked for again by their tags, when it said to ask again,
   * and how many of its answers were "not modified". All of it in the
   * object's storage: its memory does not last from one read of a list
   * to the next. */
  private github: GitHubMemory;
  /** When this object started. Cloudflare puts an idle one away within
   * seconds and starts another: health says this, so that how long one
   * lasts can be seen from outside. */
  private woke: number;
  /** Actions this side's token was turned away for, and until when
   * they are left to a machine. */
  private refused = new Map<string, number>();
  /** Sections this side's token was turned away for, and until when
   * they are a machine's to read, as they were before this side read
   * anything. */
  private unread = new Map<string, number>();
  /** Names of actions this object is doing right now. */
  private doing = new Set<string>();
  /** Held actions this side has taken to do itself: out of the held
   * list, so that no machine is handed them, and not yet settled. Kept
   * in storage under "taken", so that an object that starts again
   * knows what became of each (see recoverTaken). Null until read. */
  private taken: Held[] | null = null;
  /** For each thing a taken action is about, the doing of it: a newer
   * action about the same thing waits for it to end. */
  private busy = new Map<string, Promise<void>>();
  /** A pass over what is held is under way in this object. */
  private draining = false;
  /** No pass before this time: the last one could do nothing. */
  private drainNotBefore = 0;
  /** Both of those have been read back from storage. They are kept
   * there because this object is put away when idle: one that forgot
   * them would claim each section again on every start, and a machine,
   * told so, would never send its copy. */
  private awayRead = false;
  /** The time this object last asked to be woken at. */
  private alarmFor = 0;
  private later: (work: Promise<unknown>) => void;
  private ownClockMs: number;
  /** The last self-check: one direct read of each kind, and how it went. */
  private selfCheckResult: { at: number; kinds: Record<string, { ok: boolean; error?: string }> } | null = null;
  private setAlarm: (at: number) => Promise<void>;
  private getAlarm: () => Promise<number | null>;
  private sockets: NonNullable<EdgeOptions["sockets"]> | null;
  /** Documents that changed and have not been said to the connected yet. */
  private changedDocs = new Set<DocName>();
  /** When this side last read by itself for those connected. */
  private liveAt = 0;
  /** What this side goes by (see config()), worked out from `told`. */
  private cfg: EdgeConfig | null | undefined;
  /** What a machine told, as it is kept; undefined until read. */
  private told: EdgeConfig | null | undefined;
  private mirrors = new Map<string, Mirror | null>();
  private held: Held[] | null = null;
  /** Held actions already handed to a machine in a report answer and not
   * yet acknowledged: the machine may be doing them, so the edge never
   * does them itself. Anything held and not in here was never handed
   * out, and the edge may do it itself first. Kept in storage under
   * "handed", so an object that starts again knows. */
  private handed: Set<string> | null = null;
  private ownActs: Act[] | null = null;
  private hostActs: Act[] = [];
  /** When a document was last read. The app reads while it is open
   * (every 20 s), far more often than an object lasts once idle, so a
   * new object would always say nobody is looking unless this is kept
   * in storage and read back. Stored under "docAt", write-through. */
  private lastDocAt = 0;
  private docAtRead = false;
  /** When the host's key was last asked for. Strangers trigger the
   * fetch, gated to 30 s; without storage the gate lasts only as long
   * as the object answering, so every new object would ask again.
   * Stored under "keyFetch", write-through. */
  private lastKeyFetch = 0;
  private keyFetchRead = false;
  private hostSeen: number | null = null;
  private hostSeq: number | null = null;
  /** For each section, the content the host last said it has and when
   * it read it: a copy with that content is current as of that read. */
  private confirmed: Record<string, { hash: string; at: number; error: string | null }> = {};
  private sure = 0;
  private unsure = 0;
  private hostVersion: string | null = null;
  /** What the machine that reports says of itself: its name and what
   * it can do. Null until one has said, or from a console older than
   * the saying. */
  private machine: MachineReport | null = null;
  private latestApk: ApkMeta | null | undefined;
  private keptApk: (ApkMeta & { chunks: number }) | null | undefined;
  /** The build that could not be kept when it was last sent, and when. */
  private apkFailed: { sha256: string; at: number } | null = null;
  /** What GitHub last said of the released build, when, whether it
   * could be asked at all, and if not, why. Null when nothing is known;
   * undefined until it has been read back from storage, where it is
   * kept under "released": an object that has just started (in service,
   * nearly every one that is asked) would otherwise ask GitHub again
   * each time, and the ten minutes between askings would mean nothing. */
  private released: Released | null | undefined = undefined;
  /** Reports refused per address. Memory only, by decision: a flood
   * keeps the object in memory while it lasts, which is when the
   * limiter matters; across objects each new one starts counting
   * afresh, and the host's own reports never fail these checks. */
  private syncRefused: KeyedLimiter;
  readonly docs: Docs;
  readonly push: Push;
  /** Phone link/attempt limiters. Memory only, by the same decision:
   * they blunt a burst against the object answering, and a new object
   * starting over does not let a burst through to anything but
   * another in-memory count. */
  readonly phones: Phones;
  private notifier: Notifier;
  private table: Record<string, Action>;

  constructor(options: EdgeOptions) {
    this.kv = options.kv;
    this.now = options.now ?? Date.now;
    this.github = githubMemory(options.kv, { now: this.now });
    this.woke = this.now();
    this.fetchImpl = plain(options.fetchImpl ?? fetch) as typeof fetch;
    this.direct = options.direct;
    this.sockets = options.sockets ?? null;
    const setAlarm = options.setAlarm ?? (async () => {});
    this.setAlarm = async (at) => {
      this.alarmFor = at;
      await setAlarm(at);
    };
    this.getAlarm = options.getAlarm ?? (async () => null);
    this.later = options.later ?? (() => {});
    this.ownClockMs = options.ownClockMs ?? OWN_CLOCK_MS;
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
    // A document that has changed is said to whoever is connected.
    this.docs.onChange = (docs) => this.changed(docs);
    this.push = new Push(
      kvStore(this.kv, "push"),
      // The contact a push service is given: the project's page.
      () => contact(platformSettings().repo),
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
          // The owner's word outweighs GitHub's "not before": if it is
          // still turned away, the first call says so again.
          await this.github.free();
          await this.saveAway();
          // What is held and this side can do is done on its next wake,
          // which the scheduling after this action sets for at once.
          this.drainNotBefore = 0;
          return { tried };
        },
        touch: [],
        audit: () => ({}),
      },
      // Something waiting for a machine is taken back by the owner
      // before one has it. (One a machine was already handed may still
      // be done: its answer then finds nothing waiting, and is let go.)
      "held.drop": {
        run: async (a) => {
          await this.ready();
          const held = await this.heldList();
          const at = held.findIndex((h) => h.id === a.id);
          if (at < 0) {
            throw new OpError(
              404,
              "That is no longer waiting: it was done or is being done, or was taken back already.",
            );
          }
          const [gone] = held.splice(at, 1);
          await this.saveHeld();
          await this.unhand([gone.id]);
          // Under its name it is over. A device that never heard it was
          // kept offers it again: it is told it was taken back, and it
          // is neither kept again nor handed to a machine.
          await this.kv.put(`a:${gone.id}`, {
            at: this.now(),
            kind: gone.kind,
            state: "done",
            status: 410,
            body: { error: "This was taken back before a machine took it." },
          } satisfies Acted);
          await this.actedIndex(gone.id);
          await this.logAct({
            id: gone.id,
            kind: gone.kind,
            at: gone.at,
            by: gone.who,
            status: "failed",
            note: "Not sent: taken back before a machine took it.",
          });
          return { dropped: gone.id, kind: gone.kind };
        },
        touch: [],
        audit: (a) => ({ id: a.id }),
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
    const known = await this.releasedKnown();
    if (known && now - known.at < (known.asked ? RELEASE_EVERY_MS : RELEASE_RETRY_MS)) return known.found;
    let said: Released;
    try {
      const asked = await timed(RELEASE_ASK_MS, async (signal) => {
        const res = await this.fetchImpl(this.releaseUrl(cfg, "version.json"), { redirect: "follow", signal });
        if (res.ok) return { found: androidRelease(await res.json()) };
        unread(res);
        if (res.status === 404) return { found: null };
        throw new Error(`GitHub answered ${res.status}`);
      });
      said = { at: now, asked: true, found: asked.found, trouble: null };
    } catch (error) {
      // Why it could not be asked is kept, and health says it: "none
      // published" and "could not ask" are different things, and for a
      // day the second looked like the first.
      const e = error as Error;
      const trouble = `${e.name && e.name !== "Error" ? `${e.name}: ` : ""}${e.message}`.slice(0, 160);
      said = { at: now, asked: false, found: known?.found ?? null, trouble };
    }
    await this.releasedIs(said);
    return said.found;
  }

  /** What is known of the released build, read back from storage the
   * first time this object is asked. What was stored is held to what
   * could have been: a description that is one, and a time that has
   * come. One yet to come would never fall due, in this object or the
   * next, so what it is stored with is asked for again at once. */
  private async releasedKnown(): Promise<Released | null> {
    if (this.released !== undefined) return this.released;
    let kept: Partial<Released> | undefined;
    try {
      kept = (await this.kv.get("released")) as Partial<Released> | undefined;
    } catch {
      kept = undefined;
    }
    this.released =
      kept && typeof kept.at === "number" && typeof kept.asked === "boolean"
        ? {
            at: kept.at > this.now() ? 0 : kept.at,
            asked: kept.asked,
            found: androidRelease(kept.found),
            trouble: typeof kept.trouble === "string" ? kept.trouble.slice(0, 160) : null,
          }
        : null;
    return this.released;
  }

  /** What is known of the released build from now on, here and in
   * storage; null when it is to be asked for afresh. */
  private async releasedIs(said: Released | null): Promise<void> {
    this.released = said;
    try {
      if (said) await this.kv.put("released", said);
      else await this.kv.delete("released");
    } catch {
      // Not stored: the next object asks GitHub itself.
    }
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
      const bytes = await timed(RELEASE_FETCH_MS, async (signal) => {
        const res = await this.fetchImpl(this.releaseUrl(cfg, "pm-console.apk"), { redirect: "follow", signal });
        if (res.ok) return new Uint8Array(await res.arrayBuffer());
        unread(res);
        return null;
      });
      if (!bytes) return null;
      const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
      const sha256 = [...digest].map((b) => b.toString(16).padStart(2, "0")).join("");
      if (bytes.byteLength !== release.size || sha256 !== release.sha256) {
        // The description is out of step with the file: read it again
        // next time instead of trusting it for ten minutes.
        await this.releasedIs(null);
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

  /** What a machine told, as it told it and as it is kept. */
  private async toldConfig(): Promise<EdgeConfig | null> {
    if (this.told === undefined) this.told = edgeConfig(await this.kv.get("cfg"));
    return this.told;
  }

  /** What this side goes by; null until a machine has said who may
   * sign in. Who may sign in is as the machine told it (the
   * deployment's, until that is applied from Actions: #13495). The
   * repository's settings are this side's own, as its deploy carries
   * them, whatever a machine told for them; the list of repositories
   * alone is still the machine's (src/settings.ts says why). */
  private async config(): Promise<EdgeConfig | null> {
    if (this.cfg === undefined) {
      const told = await this.toldConfig();
      this.cfg = told === null ? null : goesBy(told);
    }
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
      let marked = true;
      for (let i = 0; i < meta.chunks; i++) {
        const kept = (await this.kv.get(`m:${name}:${i}`)) as Partial<MirrorPiece> | string | undefined;
        if (typeof kept === "string") {
          // Written before pieces were marked: still this section's.
          parts.push(kept);
          continue;
        }
        // A piece of another write, or none: what the index names was
        // never finished, or is being replaced. Nothing is kept.
        if (!kept || kept.n !== name || kept.h !== meta.hash || typeof kept.t !== "string") {
          marked = false;
          break;
        }
        parts.push(kept.t);
      }
      try {
        if (!marked) found = null;
        else {
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
        }
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
    // Pieces first, each marked with the section and content they hold,
    // then the index last: see `mirror`.
    for (let i = 0; i < chunks; i++)
      await this.kv.put(`m:${section.name}:${i}`, {
        n: section.name,
        h: section.hash,
        t: text.slice(i * CHUNK, (i + 1) * CHUNK),
      } satisfies MirrorPiece);
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
      // Unchanged content is not written again: the host sends every
      // section it holds with each report, and a write per report per
      // section is the steady-state write load. The time is still taken
      // as current through `confirmed` (see `mirror`), not stored.
      if (have && !have.own && have.hash === raw.hash) continue;
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

  /** Names of held actions already handed to a machine. */
  private async handedList(): Promise<Set<string>> {
    if (this.handed === null) {
      const stored = ((await this.kv.get("handed")) as string[] | undefined) ?? [];
      this.handed = new Set(stored.filter((id) => typeof id === "string"));
    }
    return this.handed;
  }

  private async saveHanded(): Promise<void> {
    await this.kv.put("handed", [...(this.handed ?? [])]);
  }

  /** Forget handed names that are no longer held (acknowledged, dropped,
   * expired, or done here). */
  private async pruneHanded(): Promise<void> {
    const held = await this.heldList();
    const handed = await this.handedList();
    let changed = false;
    for (const id of [...handed]) {
      if (!held.some((h) => h.id === id)) {
        handed.delete(id);
        changed = true;
      }
    }
    if (changed) await this.saveHanded();
  }

  private async markHanded(ids: string[]): Promise<void> {
    if (ids.length === 0) return;
    const handed = await this.handedList();
    let changed = false;
    for (const id of ids) {
      if (!handed.has(id)) {
        handed.add(id);
        changed = true;
      }
    }
    if (changed) await this.saveHanded();
  }

  private async unhand(ids: string[]): Promise<void> {
    if (ids.length === 0) return;
    const handed = await this.handedList();
    let changed = false;
    for (const id of ids) {
      if (handed.delete(id)) changed = true;
    }
    if (changed) await this.saveHanded();
  }

  // ---- what is held, done here ------------------------------------------
  //
  // What the owner did while this side was turned away and no machine
  // could be reached is held for a machine. When this side can act
  // again it does what it holds itself, so that nothing waits for a
  // machine that may not come. The rule that keeps anything from being
  // done twice is that an action is one side's or the other's, never
  // both. It is a machine's once it may have reached one: handed out in
  // a report's answer, or sent toward one with nothing to say that it
  // never arrived (see withMachine). It is this side's once it has been
  // taken out of the held list, which is done before anything is begun,
  // and only ever with one that certainly reached no machine.

  /** A machine has this, or may have. It was handed out in a report's
   * answer; or it is not known to have reached no machine (it went
   * toward one and nothing said that it never arrived, or it was kept
   * before that was written down). It is that machine's by name, which
   * sorts out whether it did it. This side never does it itself, and
   * nothing newer about the same thing goes ahead of it. */
  private withMachine(h: Held, handed: Set<string>): boolean {
    return handed.has(h.id) || h.unsent !== true;
  }

  /** What a held action is about, or itself when it stands alone. */
  private heldAbout(h: Held): string {
    return subjectOf(h.kind, h.args) ?? `:${h.id}`;
  }

  /** Who each held action waits for, as the owner is told it. The same
   * rule heldMine goes by: this side does the oldest about a thing, and
   * then the next, for as long as each is a kind it does and nothing
   * about the thing is, or may be, with a machine. */
  private async heldWaits(cfg: EdgeConfig | null): Promise<Map<string, HeldFor>> {
    const held = await this.heldList();
    const handed = await this.handedList();
    const can = this.primary(cfg) && this.mine(cfg) !== null;
    const withMachine = new Set(held.filter((h) => this.withMachine(h, handed)).map((h) => this.heldAbout(h)));
    // Things with something ahead that only a machine can do.
    const behind = new Set<string>();
    const waits = new Map<string, HeldFor>();
    for (const h of held) {
      const about = this.heldAbout(h);
      if (this.withMachine(h, handed)) waits.set(h.id, "asked");
      else if (can && this.does(h.kind) && !withMachine.has(about) && !behind.has(about)) {
        waits.set(h.id, "either");
      } else {
        waits.set(h.id, "machine");
        behind.add(about);
      }
    }
    return waits;
  }

  /** A held action put where its time says it goes: oldest first. */
  private insertHeld(held: Held[], item: Held): void {
    const at = held.findIndex((h) => h.at > item.at);
    if (at < 0) held.push(item);
    else held.splice(at, 0, item);
  }

  /** What this side has taken, read back when an object starts, and each
   * settled by what its record says. One that is in the held list again
   * (or never left it) is not taken. One whose record says it ended,
   * ended. One begun and not ended was being done by an object that was
   * put away: whether it happened is not known, and neither side does
   * it again; the owner is told to look. One never begun waits again,
   * where it was. */
  private async recoverTaken(): Promise<void> {
    const stored = (await this.kv.get("taken")) as unknown;
    this.taken = [];
    if (!Array.isArray(stored) || stored.length === 0) return;
    const held = await this.heldList();
    let back = false;
    for (const item of stored as Partial<Held>[]) {
      if (
        !item ||
        typeof item.id !== "string" ||
        typeof item.kind !== "string" ||
        typeof item.at !== "string" ||
        typeof item.who !== "string" ||
        item.args === null ||
        typeof item.args !== "object"
      ) {
        continue;
      }
      const was = item as Held;
      if (held.some((h) => h.id === was.id)) continue;
      const key = `a:${was.id}`;
      const record = ((await this.kv.get(key)) as Acted | undefined) ?? null;
      if (record?.state === "done") continue;
      if (record?.state === "running") {
        await this.kv.put(key, {
          at: this.now(),
          kind: was.kind,
          state: "done",
          status: 502,
          body: { error: RESTARTED },
        } satisfies Acted);
        await this.logAct({ id: was.id, kind: was.kind, at: was.at, by: was.who, status: "failed", note: RESTARTED });
        continue;
      }
      this.insertHeld(held, was);
      back = true;
      await this.kv.put(key, { at: this.now(), kind: was.kind, state: "given" } satisfies Acted);
    }
    if (back) await this.saveHeld();
    await this.kv.delete("taken");
  }

  /** Take a held action to do here: out of the held list before anything
   * is begun, so that a machine reporting meanwhile is not handed it.
   * Null if it is not this side's to take: gone from the list, or a
   * machine's (see withMachine), which may be doing it. */
  private async takeHeld(id: string): Promise<{ item: Held; ended: () => void } | null> {
    await this.ready();
    const held = await this.heldList();
    const handed = await this.handedList();
    // From here until the lists are changed nothing is waited for: a
    // report answered meanwhile sees the list with it or without it,
    // and a newer action about the same thing finds this one being done.
    const at = held.findIndex((h) => h.id === id);
    if (at < 0 || this.withMachine(held[at], handed)) return null;
    const [item] = held.splice(at, 1);
    const taken = (this.taken ??= []);
    taken.push(item);
    const about = this.heldAbout(item);
    let over: () => void = () => {};
    const doing = new Promise<void>((resolve) => {
      over = resolve;
    });
    this.busy.set(about, doing);
    const ended = () => {
      if (this.busy.get(about) === doing) this.busy.delete(about);
      over();
    };
    try {
      // Written down as taken before it leaves the held list: an object
      // put away between the two finds it in both, and it was not begun.
      await this.kv.put("taken", taken);
      await this.saveHeld();
    } catch (error) {
      // It could not be written down: it was not taken, and waits as
      // it did (nothing was handed out meanwhile that is about the
      // same thing, since it was this side's for as long as this took).
      this.taken = (this.taken ?? []).filter((t) => t.id !== item.id);
      if (!held.some((h) => h.id === item.id)) this.insertHeld(held, item);
      ended();
      throw error;
    }
    return { item, ended };
  }

  /** Do here a held action that never reached a machine. "done": it
   * ended here, well or badly, and its record and "What you did" say
   * how. "back": this side was turned away before anything was written,
   * and it waits again where it was, a machine's by name as before.
   * "gone": it was not this side's to take. */
  private async doHeld(item: Held): Promise<"done" | "back" | "gone"> {
    const took = await this.takeHeld(item.id);
    if (!took) return "gone";
    const { item: taken, ended } = took;
    try {
      let outcome: "done" | "back" = "done";
      try {
        await this.once(taken.id, taken.kind, () =>
          this.directAct(taken.kind, taken.args, taken.who, "edge", taken.id),
        );
      } catch (error) {
        // Anything else is a failure of this side's own (its storage):
        // it stays taken, and the next object settles it by its record.
        if (!(error instanceof HandOver)) throw error;
        outcome = "back";
      }
      if (outcome === "back") {
        await this.kv.put(`a:${taken.id}`, { at: this.now(), kind: taken.kind, state: "given" } satisfies Acted);
        await this.actedIndex(taken.id);
        this.insertHeld(await this.heldList(), taken);
        await this.saveHeld();
      }
      this.taken = (this.taken ?? []).filter((t) => t.id !== taken.id);
      await this.kv.put("taken", this.taken);
      return outcome;
    } finally {
      ended();
    }
  }

  /** A held action that a newer one of its kind, about the same thing,
   * makes pointless (a rank set twice ends at the last): dropped, and
   * said to have been replaced. */
  private async replaceHeld(old: Held, by: string): Promise<void> {
    const list = await this.heldList();
    const at = list.findIndex((h) => h.id === old.id);
    if (at >= 0) list.splice(at, 1);
    await this.saveHeld();
    await this.unhand([old.id]);
    await this.audit({
      at: new Date(this.now()).toISOString(),
      who: old.who,
      via: "edge",
      action: old.kind,
      phase: "done",
      ok: false,
      error: `replaced by a newer ${by}: the last one wins`,
    });
    await this.logAct({
      id: old.id,
      kind: old.kind,
      at: old.at,
      by: old.who,
      status: "failed",
      note: `Replaced by a newer ${by}: the last one wins.`,
    });
    await this.kv.put(`a:${old.id}`, {
      at: this.now(),
      kind: old.kind,
      state: "done",
      status: 409,
      body: { error: "This was replaced by a newer action about the same thing." },
    } satisfies Acted);
    await this.actedIndex(old.id);
  }

  /** For each thing held actions are about (one that stands alone is
   * its own), the oldest of them, where it is one this side could do
   * itself: a kind it runs, that certainly reached no machine, and
   * nothing else about the same thing with a machine either. Only the
   * oldest about a thing can be done next, so these are all a pass can
   * begin with. */
  private async heldMine(cfg: EdgeConfig | null): Promise<Held[]> {
    if (!this.primary(cfg) || !this.mine(cfg)) return [];
    const held = await this.heldList();
    const handed = await this.handedList();
    const withMachine = new Set(held.filter((h) => this.withMachine(h, handed)).map((h) => this.heldAbout(h)));
    const seen = new Set<string>();
    const first: Held[] = [];
    for (const h of held) {
      const about = this.heldAbout(h);
      if (seen.has(about)) continue;
      seen.add(about);
      if (this.does(h.kind) && !withMachine.has(about)) first.push(h);
    }
    return first;
  }

  /** When this side may next try one of its kind: not while it is
   * turned away for the kind, nor before the time GitHub gave. (Every
   * kind is kept to GitHub's time, though one or two ask only the queue
   * store: tried sooner, the rest would only be turned away again.) */
  private async heldReadyAt(kind: string): Promise<number> {
    this.refusedKinds();
    return Math.max(this.refused.get(kind) ?? 0, (await this.github.quiet()).core?.until ?? 0);
  }

  /** The time of the next thing to do about what is held: the first of
   * it this side could do itself, and the day's end of the oldest of it
   * whoever's it is. Null when nothing is held. */
  private async heldDue(cfg: EdgeConfig | null): Promise<number | null> {
    const held = await this.heldList();
    if (held.length === 0) return null;
    const times = held.map((h) => Date.parse(h.at) + HELD_FOR_MS + 1000).filter((t) => Number.isFinite(t));
    for (const h of await this.heldMine(cfg)) {
      times.push(Math.max(await this.heldReadyAt(h.kind), this.drainNotBefore));
    }
    return times.length > 0 ? Math.min(...times) : null;
  }

  /** What has waited more than a day is dropped, and said to have been. */
  private async expireHeld(now: number): Promise<boolean> {
    const held = await this.heldList();
    const handed = await this.handedList();
    let dropped = false;
    for (const old of held.filter((h) => now - Date.parse(h.at) > HELD_FOR_MS)) {
      // Said before it leaves the list: whether a machine may have had it.
      const maybe = this.withMachine(old, handed);
      held.splice(held.indexOf(old), 1);
      dropped = true;
      await this.logAct({
        id: old.id,
        kind: old.kind,
        at: old.at,
        by: old.who,
        status: "failed",
        note: maybe
          ? "The laptop may have got this, and did not say what became of it within a day: check whether it happened before trying again."
          : "Not sent: the laptop was out of reach for more than a day.",
      });
    }
    return dropped;
  }

  /** What is held and never reached a machine is done here, now that
   * this side can: the oldest about each thing first, and nothing about
   * a thing behind one that could not be done. One pass at a time. */
  private async drainHeld(cfg: EdgeConfig | null): Promise<void> {
    if (this.draining || this.now() < this.drainNotBefore) return;
    this.draining = true;
    let did = 0;
    let tried = false;
    try {
      await this.ready();
      // Left for another pass: what this one could not do, and
      // everything behind it about the same thing.
      const left = new Set<string>();
      for (let turn = 0; turn < 2 * HELD_MAX; turn++) {
        const now = this.now();
        let pick: Held | null = null;
        for (const h of await this.heldMine(cfg)) {
          const about = this.heldAbout(h);
          if (left.has(about)) continue;
          if (this.doing.has(h.id) || (await this.heldReadyAt(h.kind)) > now) {
            left.add(about);
            continue;
          }
          pick = h;
          break;
        }
        if (!pick) break;
        tried = true;
        const about = subjectOf(pick.kind, pick.args);
        const one = pick;
        if (
          LAST_WINS.has(one.kind) &&
          about !== null &&
          (await this.heldList()).some(
            (h) => h.id !== one.id && h.kind === one.kind && subjectOf(h.kind, h.args) === about,
          )
        ) {
          await this.replaceHeld(one, one.kind);
          did++;
          continue;
        }
        const went = await this.doHeld(one);
        if (went === "done") did++;
        else left.add(about ?? `:${one.id}`);
      }
    } finally {
      this.draining = false;
      // A pass that tried and could do nothing is not tried again at
      // once. (One that found nothing due yet sets no such pause.)
      if (tried && did === 0) this.drainNotBefore = this.now() + DRAIN_AGAIN_MS;
    }
  }

  /** The platform's own part, each machine that reports, and what is
   * held for one: what the Machines screen is drawn from. Answered
   * here, so it is there with every machine away. */
  private async machinesSaid(cfg: EdgeConfig | null): Promise<MachinesAnswer> {
    await this.ready();
    const primary = this.primary(cfg);
    const refused = this.refusedKinds();
    const seen = this.hostSeen || null;
    const told = await this.toldConfig();
    const differs = told === null ? [] : settingsDifferences(told);
    return {
      served: "edge",
      platform: {
        primary,
        // What it does itself right now. Its own (notifications, linked
        // phones, what is held) always; the core when it has the means,
        // less what it was turned away for and a machine does for the hour.
        actions: [
          ...Object.keys(this.table).sort(),
          ...(primary ? this.kinds().filter((kind) => !refused.includes(kind)) : []),
        ],
        sections: primary ? this.owned(cfg) : [],
        refused: [...refused, ...this.refusedReads().map((name) => `read.${name}`)],
      },
      // One machine can report today: the one whose key this side holds.
      machines:
        seen === null
          ? []
          : [
              {
                name: this.machine?.name ?? "A machine",
                seen,
                quiet: this.now() - seen >= HOST_QUIET_MS,
                version: this.hostVersion,
                actions: this.machine?.actions ?? [],
                sections: this.machine?.sections ?? [],
                ...(differs.length > 0 ? { differs } : {}),
              },
            ],
      held: await this.heldSaid(cfg),
    };
  }

  /** How much is held, by who it waits for. */
  private async heldCounts(cfg: EdgeConfig | null): Promise<Record<HeldFor, number>> {
    const counts: Record<HeldFor, number> = { either: 0, machine: 0, asked: 0 };
    for (const waits of (await this.heldWaits(cfg)).values()) counts[waits]++;
    return counts;
  }

  /** What is held, each with who it waits for. */
  private async heldSaid(cfg: EdgeConfig | null): Promise<(Held & { waits: HeldFor })[]> {
    const waits = await this.heldWaits(cfg);
    return (await this.heldList()).map((h) => ({ ...h, waits: waits.get(h.id) ?? "machine" }));
  }

  /** What the app is told of this side, beside its state: how much is
   * waiting to be done, and whether this side does the core itself. */
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
    if (this.taken === null) await this.recoverTaken();
    if (this.ownActs === null) await this.showActs();
    if (this.hostSeen === null) {
      const host = (await this.kv.get("host")) as
        | {
            seen: number;
            seq: number;
            confirmed?: Record<string, { hash: string; at: number; error: string | null }>;
            version?: string | null;
            machine?: unknown;
          }
        | undefined;
      this.hostSeen = host?.seen ?? 0;
      this.hostSeq = host?.seq ?? 0;
      this.confirmed = host?.confirmed ?? {};
      this.hostVersion = host?.version ?? null;
      this.machine = machineReport(host?.machine);
    }
    if (!this.awayRead) {
      this.awayRead = true;
      const kept = (await this.kv.get("away")) as
        | { kinds?: Record<string, unknown>; reads?: Record<string, unknown> }
        | undefined;
      const now = this.now();
      // Only what still stands, and nothing further off than it could
      // have been set for (the longest is GitHub's own wait): a time
      // like that is not one this side wrote, and is not believed at
      // all. (Cut down to the longest instead, as it once was, it was
      // cut down afresh by every object that started, each from its
      // own now, and never came.)
      const still = (from: Record<string, unknown> | undefined, into: Map<string, number>) => {
        for (const [name, until] of Object.entries(from ?? {})) {
          if (typeof until === "number" && until > now && until <= now + QUIET_MOST_MS && !into.has(name)) {
            into.set(name, until);
          }
        }
      };
      still(kept?.kinds, this.refused);
      still(kept?.reads, this.unread);
    }
    if (!this.docAtRead) {
      this.docAtRead = true;
      try {
        const at = (await this.kv.get("docAt")) as number | undefined;
        const now = this.now();
        if (typeof at === "number" && at > 0 && at <= now) this.lastDocAt = at;
      } catch {
        // Unread storage is no storage: nobody is looking until a read.
      }
    }
    if (!this.keyFetchRead) {
      this.keyFetchRead = true;
      try {
        const at = (await this.kv.get("keyFetch")) as number | undefined;
        const now = this.now();
        if (typeof at === "number" && at > 0 && at <= now) this.lastKeyFetch = at;
      } catch {
        // Unread storage is no storage: the next stranger fetches.
      }
    }
  }

  /** Write down what this side is turned away for, so that an object
   * that starts again knows. */
  private async saveAway(): Promise<void> {
    await this.kv.put("away", { kinds: Object.fromEntries(this.refused), reads: Object.fromEntries(this.unread) });
  }

  // ---- doing the core here ------------------------------------------------

  /** The kinds this side does itself when it does the core. */
  private kinds(): readonly string[] {
    return this.direct?.kinds ?? DIRECT_ACTIONS;
  }

  /** A kind this side does itself when it does the core; any other is
   * a machine's whenever one comes. */
  private does(kind: string): boolean {
    return this.kinds().includes(kind);
  }

  /** The switch is on and both tokens are here. */
  private directCapable(): boolean {
    return this.direct?.enabled === true && Boolean(this.direct.githubToken) && Boolean(this.direct.queueToken);
  }

  /** This side does the core itself: it has the switch and the tokens,
   * and a machine has said who may sign in. Where things are it knows
   * by itself (src/settings.ts). Whether a machine is reporting makes
   * no difference to this. */
  private primary(cfg: EdgeConfig | null): boolean {
    return this.directCapable() && cfg !== null;
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
    return DIRECT_READ_SECTIONS.filter(
      (name) =>
        !refused.includes(name) &&
        // What is labelled as needing the owner is read repository by
        // repository: a machine that has not said which they are (one
        // from before it did) goes on reporting the list itself. Read
        // from no repositories it would be an empty list, and whole.
        (name !== "census" || (cfg?.needsRepos.length ?? 0) > 0),
    );
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
        github: this.github,
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
   * may not do the thing (401, 403), and a few minutes when it was
   * only calling too often. A section read from named services is
   * left until the time GitHub gave for asking again, where it gave a
   * further one: nothing would be read before then, and claiming the
   * section back every few minutes only to give it up again would have
   * a machine stop and start sending its copy. Calls made at the same
   * time by something else are counted too: that can only hand a
   * machine something this side could have done, never keep from it
   * what this side cannot. Which is why an action (no services named)
   * is only ever left for the few minutes: one that failed for a
   * reason of its own, beside a call held back for the hour, is not
   * then a machine's for the hour. */
  private away(from: number, to?: Outward["to"][]): number | null {
    const mine = this.outward.filter((call) => call.n > from && (to === undefined || to.includes(call.to)));
    if (mine.some((call) => (call.status === 401 || call.status === 403) && !call.limited)) return REFUSED_FOR_MS;
    const limited = mine.filter((call) => call.limited);
    if (limited.length === 0) return null;
    if (to === undefined) return LIMITED_FOR_MS;
    const until = Math.max(0, ...limited.map((call) => call.until ?? 0));
    return Math.min(QUIET_MOST_MS, Math.max(LIMITED_FOR_MS, until - this.now()));
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
   * capability arrives and at most once an hour after (five minutes
   * after one in which something could not be read), its result kept
   * for health. It proves the path without taking the laptop down. */
  private async maybeSelfCheck(cfg: EdgeConfig | null): Promise<void> {
    const direct = this.mine(cfg);
    if (!direct) return;
    const now = this.now();
    const stored = (await this.kv.get("selfcheck")) as
      | { at: number; kinds: Record<string, { ok: boolean; error?: string }> }
      | undefined;
    const allRead = stored !== undefined && Object.values(stored.kinds).every((kind) => kind.ok);
    if (stored && now - stored.at < (allRead ? SELF_CHECK_EVERY_MS : SELF_CHECK_AGAIN_MS)) {
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
      // on every stranger's request. Nothing has been read yet. The
      // gate is kept in storage: without it every new object would ask
      // again, since objects last seconds and the gate is thirty.
      if (!this.keyFetchRead) {
        this.keyFetchRead = true;
        try {
          const at = (await this.kv.get("keyFetch")) as number | undefined;
          if (typeof at === "number" && at > 0 && at <= now) this.lastKeyFetch = at;
        } catch {
          // Unread storage is no storage.
        }
      }
      if (req.headers.get("x-pm-key-fetched") === "1" || now - this.lastKeyFetch < KEY_FETCH_EVERY_MS) {
        return refuse({ error: "this is not signed with the host's key" }, 401);
      }
      this.lastKeyFetch = now;
      try {
        await this.kv.put("keyFetch", now);
      } catch (failed) {
        console.error(`pm-edge: keeping key fetch: ${(failed as Error).message}`);
      }
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
    const says = edgeConfig(msg.config);
    if (!says) return json({ error: "config: not one this side can use" }, 400);
    if (JSON.stringify(says) !== JSON.stringify(await this.toldConfig())) {
      // Kept as told: what is gone by is worked out from it (config()),
      // and where the machine differs is said from it.
      await this.kv.put("cfg", says);
      this.told = says;
      this.cfg = undefined;
      // Who may sign in, or the list of repositories, may have changed.
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
    if (await this.expireHeld(now)) settled = true;
    if (settled) await this.saveHeld();
    await this.pruneHanded();
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
    this.machine = machineReport(msg.machine);
    await this.kv.put("host", {
      seen: now,
      seq: msg.at,
      confirmed: this.confirmed,
      version: this.hostVersion,
      machine: this.machine,
    });
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
    const watching = now - this.lastDocAt < WATCHING_MS;
    const notify = await this.someoneToTell();
    const phones = await this.phones.devices.links();
    const handed = await this.handedList();
    // What the machine is handed: everything held, less anything about
    // a thing this side is doing something about right now (it would
    // be done out of turn). Chosen and marked as handed with nothing
    // waited for in between, so that this side cannot take one of them
    // for itself meanwhile; and written down before the answer goes.
    const mine = new Set(
      (this.taken ?? []).map((t) => subjectOf(t.kind, t.args)).filter((about): about is string => about !== null),
    );
    const commands = held.filter((h) => {
      const about = subjectOf(h.kind, h.args);
      return about === null || !mine.has(about);
    });
    let marked = false;
    for (const command of commands) {
      if (!handed.has(command.id)) {
        handed.add(command.id);
        marked = true;
      }
    }
    if (marked) await this.saveHanded();
    const answer: SyncResponse = {
      ok: true,
      watching,
      notify,
      want,
      commands,
      phones,
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
    if (await this.clockWanted(cfg)) wakes.push(Math.max(now + 1000, this.clockAt + this.ownClockMs));
    // The app is connected: this side reads by itself what has gone
    // stale, so there is something to say when it changes.
    if (this.listening() > 0 && this.owned(cfg).length > 0) wakes.push(Math.max(now + 1000, this.liveAt + LIVE_MS));
    // Something the owner is told about that this side was turned away
    // for is tried again when its time is up; with nobody looking and
    // no machine reporting, only a wake would notice that it is.
    if (this.primary(cfg) && (await this.push.status()).devices > 0) {
      for (const name of this.refusedReads()) {
        if (NOTIFIED.includes(name)) wakes.push(Math.max(now + 1000, (this.unread.get(name) ?? now) + 1000));
      }
    }
    // What is held and never reached a machine is this side's to do
    // when it can, and what has waited a day is dropped: it wakes for
    // the first of either.
    const due = await this.heldDue(cfg);
    if (due !== null) wakes.push(Math.max(now + 1000, due));
    if (wakes.length > 0) await this.setAlarm(Math.min(...wakes));
  }

  /** How many copies of the app are connected. */
  private listening(): number {
    return this.sockets?.count() ?? 0;
  }

  /** Documents whose content has just changed: said once to everyone
   * connected, however many changes arrive together (a pass that reads
   * five lists says "core" once). The app reads the document again by
   * its tag, so a message that turns out to be about nothing new to it
   * costs it one "not modified". */
  private changed(docs: DocName[]): void {
    for (const doc of docs) this.changedDocs.add(doc);
    // Said after whatever is running now has finished changing things:
    // the first to get there says all of it, and the rest find nothing left.
    queueMicrotask(() => {
      const names = DOC_NAMES.filter((doc) => this.changedDocs.has(doc));
      this.changedDocs.clear();
      if (names.length === 0 || this.listening() === 0) return;
      try {
        this.sockets?.send(JSON.stringify({ t: "changed", docs: names }));
      } catch (error) {
        console.error(`pm-edge: telling the app: ${(error as Error).message}`);
      }
    });
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
            ? "It has not reported for ten minutes. Cloudflare carries on with the board, your prompts, pull requests, the plan and what is labelled as needing you; what needs the laptop waits for it."
            : "It has not reported for ten minutes. The app still opens; what you do is held until it is back.",
          url: "/console/",
          tag: "host",
        });
        if (delivery.failed === 0) await this.kv.put("silent", true);
      } catch {
        // Leave the transition pending so a later alarm can retry it.
      }
    }
    if (now - this.clockAt >= this.ownClockMs && (await this.clockWanted(cfg))) {
      this.clockAt = now;
      // Read again what is stale, and wait for it: this wake is the
      // only thing looking. Twice, because the first look of an object
      // that has just started shows the copy kept from before, and it
      // is the second that reads.
      for (let look = 0; look < 2; look++) await this.docs.current(this.ownedTold(cfg), CLOCK_WAIT_MS);
      await this.tell(cfg).catch((error) => console.error(`pm-edge: notify: ${(error as Error).message}`));
    }
    // The app is connected: what this side reads itself and has gone
    // stale is read again (each list by its own age; one that has not
    // changed costs nothing), and a change is said to the app as the
    // read ends.
    if (this.listening() > 0 && now - this.liveAt >= LIVE_MS) {
      this.liveAt = now;
      const mine = this.owned(cfg);
      if (mine.length > 0) {
        this.docs.keepAlive();
        await this.docs.current(mine, CLOCK_WAIT_MS);
      }
    }
    // What is held: the day's limit is kept with no machine reporting,
    // and what this side can now do itself, it does.
    if (await this.expireHeld(now)) await this.saveHeld();
    await this.drainHeld(cfg).catch((error) =>
      console.error(`pm-edge: doing what was held: ${(error as Error).message}`),
    );
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
      const told = await this.toldConfig();
      // The self-check runs when the capability is there: one direct
      // read of each kind, and how it went. Nothing here is a secret.
      await this.maybeSelfCheck(cfg);
      const [said, quiet] = [await this.github.said(), await this.github.quiet()];
      return json({
        ok: true,
        edge: true,
        configured: cfg !== null,
        directEnabled: this.direct?.enabled === true,
        githubTokenConfigured: Boolean(this.direct?.githubToken),
        queueTokenConfigured: Boolean(this.direct?.queueToken),
        // This side does the core itself: the switch and the tokens are
        // here, and a machine has said who may sign in. `directReady` is
        // the same thing under the name it had when this side acted
        // only while the laptop was away.
        primary: this.primary(cfg),
        directReady: this.primary(cfg),
        // What this side's token was refused for within the hour, and
        // a machine does instead: actions by their names, and sections
        // it could not read as `read.<section>`.
        refused: [...this.refusedKinds(), ...this.refusedReads().map((name) => `read.${name}`)],
        selfCheck: this.selfCheckResult,
        // What GitHub answered this side since `since`, how many of
        // those were "not modified" (which cost nothing of the hour's
        // allowance), how many answers are kept to ask by, and until
        // when GitHub said not to ask, if it has: for searches, which
        // it counts apart by the minute, and for everything else.
        github: {
          answers: said.answers,
          unchanged: said.unchanged,
          since: said.since,
          kept: said.kept,
          quiet: Object.fromEntries(
            ALLOWANCES.map((allowance) => {
              const until = quiet[allowance]?.until ?? 0;
              return [allowance, until > this.now() ? until : null];
            }),
          ),
        },
        // When this object started (Cloudflare starts another whenever
        // one has been put away).
        woke: this.woke,
        // Its own clock: whether it wakes itself each minute to read
        // what the owner is told about (it does while it does the core
        // itself and a device takes notifications), and when it has
        // next asked to be woken, for that or to notice a machine gone
        // quiet. Notifications with no machine depend on this, and it
        // is how that is seen to be alive from outside.
        clock: { wanted: await this.clockWanted(cfg), next: await this.getAlarm().catch(() => null) },
        // How many things the owner did are waiting to be done, by who
        // each waits for: either side, whichever can first; a machine
        // only; or a machine that was given it or may have got it.
        // Counts, and nothing of what they are.
        held: await this.heldCounts(cfg),
        hostSeen: this.hostSeen || null,
        // The commit the host's console is running, as it last said.
        hostVersion: this.hostVersion,
        // How many copies of the app are connected to be told of
        // changes as they happen.
        listening: this.listening(),
        // In which of the platform's own settings (the repository's:
        // src/settings.ts) the machine said it runs with others, by
        // name and never by value; null while no machine has said any.
        // A machine on other code than this, or with an environment of
        // its own, shows here. This side goes by its own all the same.
        machineSettingsDiffer: told === null ? null : settingsThatDiffer(told),
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
          try {
            await this.kv.put("docAt", this.lastDocAt);
          } catch (failed) {
            console.error(`pm-edge: keeping doc read: ${(failed as Error).message}`);
          }
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
        if (seg.length === 1 && seg[0] === "machines") return json(await this.machinesSaid(cfg));
        if (seg.length === 1 && seg[0] === "events") {
          // The app asking to be told when something changes: a
          // connection the platform holds. Only the owner gets here.
          if ((req.headers.get("upgrade") ?? "").toLowerCase() !== "websocket") {
            return json({ error: "this is where the app connects to be told of changes (a WebSocket)" }, 426);
          }
          if (!this.sockets) return json({ error: "no live connections here" }, 404);
          const res = this.sockets.accept(req);
          // Someone is there to tell: this side wakes by itself to read.
          this.docs.keepAlive();
          await this.schedule(cfg);
          return res;
        }
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
        // A name keeps to the side that first took it, whatever would
        // be chosen for it today. What this side did is answered from
        // its record; what was given to a machine goes to a machine
        // again, which has its own. Chosen afresh each time, a name
        // whose answer was lost while the choice changed (this side was
        // turned away for an hour, say) would be done once on each side.
        const before = id === null ? null : (((await this.kv.get(`a:${id}`)) as Acted | undefined) ?? null);
        const given = before?.state === "given";
        const here =
          before !== null ? !given : this.does(kind) && this.primary(cfg) && !this.refusedKinds().includes(kind);
        if (here) {
          // An older action held for a machine about the same thing
          // comes first. One already handed to a machine may be being
          // done there: the newer waits behind it instead. One never
          // handed out never reached a machine, so the edge does it
          // itself first, under its name; a kind where the last one
          // wins is dropped as replaced by the newer instead.
          const about = subjectOf(kind, args as Record<string, unknown>);
          if (about !== null) {
            const newer = { id: id ?? crypto.randomUUID(), kind, args, who: by };
            // Each turn waits out one thing being done here about the
            // same thing, or settles the oldest of what is held about
            // it; then it looks again, since either may have changed
            // what is waiting. (There cannot be more turns than this.)
            let clear = false;
            for (let turn = 0; turn < 4 * HELD_MAX; turn++) {
              const held = await this.heldList();
              const handed = await this.handedList();
              // From here to the choice nothing is waited for.
              // Something older about the same thing may be being done
              // here right now (what was held, on this side's own wake):
              // this one waits for it to end, so that the two are done
              // in the order the owner did them.
              const doing = this.busy.get(about);
              if (doing) {
                await doing;
                continue;
              }
              const older = held.filter((h) => h.id !== id && subjectOf(h.kind, h.args) === about);
              if (older.length === 0) {
                clear = true;
                break;
              }
              // One a machine has, or may have, may be being done
              // there: the newer waits behind it.
              if (older.some((h) => this.withMachine(h, handed))) return this.keep(newer);
              // One that only a machine can do is a machine's whenever
              // one comes: the newer waits behind it too.
              if (older.some((h) => !this.does(h.kind))) return this.keep(newer);
              const old = older[0];
              // One of the same kind where the last one wins is dropped
              // as replaced by this. (Only by its own kind: a comment
              // about an issue does not replace its rank.)
              if (LAST_WINS.has(old.kind) && old.kind === kind) {
                await this.replaceHeld(old, kind);
                continue;
              }
              // Refused here too: the newer waits behind the older. (If
              // it was not this side's to take after all, the next look
              // finds why: being done here, or a machine's.)
              if ((await this.doHeld(old)) === "back") return this.keep(newer);
            }
            if (!clear) return this.keep(newer);
          }
          try {
            return await this.once(id, kind, () =>
              this.directAct(kind, args as Record<string, unknown>, by, who.via, id),
            );
          } catch (error) {
            // Refused here before anything was done: a machine's, below.
            if (!(error instanceof HandOver)) throw error;
          }
        }
        // Written down as a machine's before it goes anywhere.
        if (id !== null && !given) {
          await this.kv.put(`a:${id}`, { at: this.now(), kind, state: "given", relays: 0 } satisfies Acted);
          await this.actedIndex(id);
        }
        // What may be done late goes behind anything already waiting
        // for a machine that is about the same thing. Sent straight to
        // a machine that has just come back, it would be done before
        // the older one, which would undo it when its turn came.
        if (HOLDABLE.has(kind)) {
          const about = subjectOf(kind, args as Record<string, unknown>);
          // A retry under the same name is the same action, not a later
          // one: it goes back to the host, which sorts out whether it
          // already did it, instead of waiting behind itself.
          const held = await this.heldList();
          if (about !== null && held.some((h) => h.id !== id && subjectOf(h.kind, h.args) === about)) {
            // A name that was a machine's before this arrival set off
            // toward one already: it may be there now, and is kept as
            // that machine's. (One already waiting here is kept once,
            // as it is.)
            return this.keep({ id: id ?? crypto.randomUUID(), kind, args, who: by }, given ? "unknown" : "nowhere");
          }
        }
        if (id !== null) {
          await this.ready();
          const handed = await this.handedList();
          const record = ((await this.kv.get(`a:${id}`)) as Acted | undefined) ?? null;
          // From here to the choice nothing is waited for.
          // This side took the name to do itself a moment ago: it is
          // not sent to a machine as well.
          if ((this.taken ?? []).some((t) => t.id === id)) return this.takenSaid(id);
          // Offered again while it waits here, having reached no
          // machine: it goes on waiting as it is, for this side or for
          // a machine's next report, whichever is first. Sent toward
          // the host now, it would be on its way there while this side
          // may be taking it for its own.
          const waiting = (this.held ?? []).find((h) => h.id === id);
          if (waiting && !this.withMachine(waiting, handed)) {
            const waits = (await this.heldWaits(cfg)).get(id) ?? "machine";
            return json({ ok: true, held: true, result: null, again: true, waits });
          }
          // It sets off toward a machine, and that is counted under its
          // name before it goes (a record from before the counting is
          // taken to have set off once already).
          const relays = (record?.state === "given" ? (record.relays ?? 1) : 0) + 1;
          await this.kv.put(`a:${id}`, { at: this.now(), kind, state: "given", relays } satisfies Acted);
          await this.actedIndex(id);
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
          // Whether the name is the device's (it may be offered again)
          // or one given here for this one asking.
          named: id !== null,
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
        const body = { error: RESTARTED };
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
    // Once in the list, however often it is noted (a name given to a
    // machine after this side was turned away is noted twice).
    const index = (((await this.kv.get("acted")) as { id: string; at: number }[] | undefined) ?? []).filter(
      (entry) => entry.id !== id,
    );
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

  /** The host did not answer: keep the action, if it is one that may be
   * done late. The stateless part, which saw what came back in the
   * host's place, says whether that was word that the host was never
   * reached. */
  private async hold(req: Request): Promise<Response> {
    const body = (await req.json().catch(() => null)) as {
      id?: unknown;
      kind?: unknown;
      args?: unknown;
      who?: unknown;
      unreached?: unknown;
      named?: unknown;
    } | null;
    if (body?.unreached !== true) return this.keep(body, "unknown");
    // Word that this asking never arrived says the action reached no
    // machine only if it set off this once: another asking under the
    // same name (the device offered it again meanwhile) may be at the
    // host now. A name given here, and not by the device, set off once.
    const record =
      body.named === false || typeof body.id !== "string"
        ? null
        : (((await this.kv.get(`a:${body.id}`)) as Acted | undefined) ?? null);
    const once = body.named === false || (record?.state === "given" && record.relays === 1);
    return this.keep(body, once ? "unreached" : "unknown");
  }

  /** Keep an action for later, at the back of what already waits.
   * `went` is how far it got toward a machine. "nowhere": it was never
   * sent to one. "unreached": it was sent, and what answered in the
   * machine's place said it never arrived. "unknown": it was sent, and
   * nothing says what became of it, so the machine may have done it. */
  private async keep(
    body: { id?: unknown; kind?: unknown; args?: unknown; who?: unknown } | null,
    went: "nowhere" | "unreached" | "unknown" = "nowhere",
  ): Promise<Response> {
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
    const cfg = await this.config();
    const held = await this.heldList();
    // Offered again under a name that is already waiting (the device
    // never heard that it was kept): it is kept once, as it is. (One
    // waiting here only sets off toward a machine again when it is a
    // machine's already, so there is nothing about it to change.)
    const waiting = typeof body?.id === "string" ? held.find((h) => h.id === body.id) : undefined;
    if (waiting) {
      const waits = (await this.heldWaits(cfg)).get(waiting.id) ?? "machine";
      return json({ ok: true, held: true, result: null, again: true, waits });
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
      ...(went === "unknown" ? {} : { unsent: true as const }),
    };
    held.push(item);
    await this.saveHeld();
    const waits = (await this.heldWaits(cfg)).get(item.id) ?? "machine";
    await this.audit({
      at: item.at,
      who: item.who,
      via: "edge",
      action: kind,
      phase: "held",
      id: item.id,
      // Whether it is known to have reached no machine.
      unsent: item.unsent === true,
    });
    await this.logAct({ id: item.id, kind, at: item.at, by: item.who, status: "running", note: HELD_SAID[waits] });
    // If it is one this side could do itself when it can again, it
    // asks to be woken for that; and for the day's end of it either way.
    await this.schedule(cfg).catch(() => {});
    return json({ ok: true, held: true, result: null, waits });
  }

  /** A name this side has taken to do itself is not sent to a machine
   * as well: its doing is waited for, and the answer is what came of
   * it. If nothing did (this side was turned away again, or could not
   * write down how it went), the device is told it is not over, and
   * offers it again. (Called only for a name in the taken list.) */
  private async takenSaid(id: string): Promise<Response> {
    const mine = (this.taken ?? []).find((t) => t.id === id);
    if (mine) await this.busy.get(this.heldAbout(mine));
    const after = ((await this.kv.get(`a:${id}`)) as Acted | undefined) ?? null;
    if (after?.state === "done") return json({ ...after.body, again: true }, after.status ?? 200);
    return json({ error: "This is still being done.", running: true }, 409);
  }

  /** After the host did a relayed action: the sections it read again and
   * its record of what was done, so the next document already shows it.
   * The relayed id is settled: a hold left behind by a lost answer for
   * the same name is dropped, since the host has now answered it. */
  private async relayed(req: Request): Promise<Response> {
    const body = (await req.json().catch(() => null)) as { sections?: unknown; acts?: unknown; id?: unknown } | null;
    await this.ready();
    if (typeof body?.id === "string") {
      const held = await this.heldList();
      const at = held.findIndex((h) => h.id === body.id);
      if (at >= 0) {
        held.splice(at, 1);
        await this.saveHeld();
        await this.unhand([body.id]);
      }
    }
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
