// The app, served from the edge.
//
// A Cloudflare Worker sits in front of the public hostname. This is the
// part of it that holds state (it runs in one Durable Object): it checks
// who is calling, serves the app's state from the copy the host last
// reported, runs the actions that need nothing from the host, holds the
// ones the owner takes while the host cannot be reached, sends
// notifications, and answers a linked phone's widget.
//
// The host is still the machine with the fleet's tools on it. When it is
// reachable, an action that needs it is relayed to it at once and its
// answer comes straight back. When it is not, the app still opens, shows
// the state as of the host's last report with the time beside it, and
// keeps working for everything that is safe to do late.
//
// Nothing here trusts where a request came from. The owner is whoever
// presents a sign-in token that verifies; the host is whoever signs with
// the key the host itself published through its tunnel; a phone is
// whoever holds a token this service issued.
import { actions, type Action, ownActions } from "../../src/actions.ts";
import { AuthError, type Identity, identify } from "../../src/auth.ts";
import { DIRECT_ACTIONS, DIRECT_READ_SECTIONS, directClasses, directRead } from "./direct.ts";
import { Devices, KeyedLimiter, Limiter } from "../../src/devices.ts";
import { type Act, DOC_NAMES, type DocName, Docs, Stamped } from "../../src/docs.ts";
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
import { Notifier } from "../../src/notify.ts";
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
/** The laptop is quiet long enough to bypass it for safe reads and writes. */
const DIRECT_READ_AFTER_MS = 60_000;
/** How often the self-check may run again. */
const SELF_CHECK_EVERY_MS = 60 * 60_000;
const KEY_FETCH_EVERY_MS = 30_000;
const STATIC: Record<string, string> = {
  "app.js": "text/javascript; charset=utf-8",
  "app.css": "text/css; charset=utf-8",
  "icon.svg": "image/svg+xml",
  "sw.js": "text/javascript; charset=utf-8",
};
const ACT_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const ICONS = /^\/icons\/(icon-192|icon-512|icon-maskable-512|apple-touch-icon)\.png$/;

interface Mirror {
  at: number;
  hash: string;
  data: unknown;
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
  /** What the edge may do itself while the laptop is away: the switch,
   * the two tokens, and the fetch the direct reads and writes use. */
  direct?: {
    enabled: boolean;
    githubToken?: string;
    queueToken?: string;
    fetchImpl?: typeof fetch;
  };
  /** Ask to be woken at a time: the check for a silent host. */
  setAlarm?: (at: number) => Promise<void>;
}

export class Edge {
  private kv: KV;
  private now: () => number;
  private fetchImpl: typeof fetch;
  private direct?: EdgeOptions["direct"];
  /** When each section was last read directly, so a fresh read is not
   * made more often than the section's own interval. */
  private directAt = new Map<string, number>();
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
  /** What GitHub last said of the released build, when, and whether it
   * could be asked at all. */
  private released: { at: number; asked: boolean; found: AndroidRelease | null } | null = null;
  private syncRefused: KeyedLimiter;
  readonly docs: Docs;
  readonly push: Push;
  readonly phones: Phones;
  private notifier: Notifier;
  private table: Record<string, Action>;

  constructor(options: EdgeOptions) {
    this.kv = options.kv;
    this.now = options.now ?? Date.now;
    this.fetchImpl = options.fetchImpl ?? fetch;
    this.direct = options.direct;
    this.setAlarm = options.setAlarm ?? (async () => {});
    this.syncRefused = new KeyedLimiter(SYNC_REFUSALS, 60_000, this.now);
    this.docs = new Docs(
      Object.fromEntries(
        Object.entries(SECTIONS).map(([name, ttlMs]) => [
          name,
          {
            ttlMs,
            load: async () => {
              const m = await this.mirror(name);
              if (!m) throw new Error("the laptop has not reported this yet");
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
    this.table = ownActions(this.push, devices);
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
      if (res.status === 404) this.released = { at: now, asked: true, found: null };
      else if (res.ok) this.released = { at: now, asked: true, found: androidRelease(await res.json()) };
      else throw new Error(`http ${res.status}`);
    } catch {
      this.released = { at: now, asked: false, found: known?.found ?? null };
    }
    return this.released.found;
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
    const meta = (await this.kv.get(`m:${name}`)) as { at: number; hash: string; chunks: number } | undefined;
    let found: Mirror | null = null;
    if (meta) {
      const parts: string[] = [];
      for (let i = 0; i < meta.chunks; i++) parts.push(String((await this.kv.get(`m:${name}:${i}`)) ?? ""));
      try {
        // As current as the host last said it was, which may be later
        // than when the copy itself was sent.
        const at = Math.max(meta.at, this.confirmed[name]?.hash === meta.hash ? this.confirmed[name].at : 0);
        found = { at, hash: meta.hash, data: JSON.parse(parts.join("")) };
      } catch {
        found = null; // a copy that cannot be read is no copy: the host is asked for it again
      }
    }
    this.mirrors.set(name, found);
    return found;
  }

  private async putMirror(section: SyncSection): Promise<void> {
    const text = JSON.stringify(section.data);
    const chunks = Math.max(1, Math.ceil(text.length / CHUNK));
    const before = (await this.kv.get(`m:${section.name}`)) as { chunks: number } | undefined;
    for (let i = 0; i < chunks; i++)
      await this.kv.put(`m:${section.name}:${i}`, text.slice(i * CHUNK, (i + 1) * CHUNK));
    for (let i = chunks; i < (before?.chunks ?? 0); i++) await this.kv.delete(`m:${section.name}:${i}`);
    await this.kv.put(`m:${section.name}`, { at: section.at, hash: section.hash, chunks });
    this.mirrors.set(section.name, { at: section.at, hash: section.hash, data: section.data });
  }

  /** Sections from the host, taken in; returns the names that changed. */
  private async take(sections: unknown): Promise<string[]> {
    const names: string[] = [];
    if (!Array.isArray(sections)) return names;
    for (const raw of sections as SyncSection[]) {
      if (!raw || typeof raw.name !== "string" || !Object.hasOwn(SECTIONS, raw.name)) continue;
      if (typeof raw.at !== "number" || typeof raw.hash !== "string" || raw.data === undefined) continue;
      const have = await this.mirror(raw.name);
      // An older read never replaces a newer one.
      if (have && have.at > raw.at) continue;
      await this.putMirror({ name: raw.name, at: Math.min(raw.at, this.now()), hash: raw.hash, data: raw.data });
      names.push(raw.name);
    }
    if (names.length > 0) await this.docs.touch(names);
    return names;
  }

  private async heldList(): Promise<Held[]> {
    if (this.held === null) this.held = ((await this.kv.get("held")) as Held[] | undefined) ?? [];
    return this.held;
  }

  private async saveHeld(): Promise<void> {
    await this.kv.put("held", this.held ?? []);
    this.docs.extra("edge", { held: (this.held ?? []).length });
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
      this.docs.extra("edge", { held: (this.held ?? []).length });
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
  }

  // ---- doing it here while the laptop is away --------------------------

  /** The laptop has been quiet long enough to bypass it. */
  private directQuiet(): boolean {
    return this.now() - (this.hostSeen ?? 0) >= DIRECT_READ_AFTER_MS;
  }

  /** The switch is on and both tokens are here: the edge may act itself.
   * The quiet gate is separate, so health can say the capability is
   * ready even while the laptop is still reporting. */
  private directCapable(): boolean {
    return this.direct?.enabled === true && Boolean(this.direct.githubToken) && Boolean(this.direct.queueToken);
  }

  /** May act itself right now: capable, and the laptop known quiet. */
  private directReady(cfg: EdgeConfig | null): boolean {
    return this.directCapable() && this.directConfigReady(cfg) && this.directQuiet();
  }

  /** The config carries what direct mode needs: a config stored or
   * reported before direct mode existed has these empty, and the edge
   * serves the offline app from it without acting itself. */
  private directConfigReady(cfg: EdgeConfig | null): boolean {
    return cfg !== null && cfg.queueDbUrl !== "" && cfg.queueTokenName !== "" && cfg.promptLabel !== "";
  }

  /** The laptop's own classes, wired for the edge. Null while the
   * capability is not there. */
  private directClasses(cfg: EdgeConfig | null): ReturnType<typeof directClasses> | null {
    if (cfg === null || !this.directCapable() || !this.directConfigReady(cfg)) return null;
    return directClasses(cfg, {
      githubToken: this.direct?.githubToken,
      queueToken: this.direct?.queueToken,
      fetchImpl: this.direct?.fetchImpl as never,
    });
  }

  /** One direct read of a section, recorded in the mirror the documents
   * are built from. A section is not read again within its own interval,
   * and a failed read keeps the last good copy and says why. After an
   * action of this side's own, the read is forced: the change must show
   * at once, not when the interval runs out. */
  private async directReadSection(name: string, cfg: EdgeConfig | null, force = false): Promise<void> {
    const now = this.now();
    const last = this.directAt.get(name) ?? 0;
    if (!force && now - last < (SECTIONS[name] ?? 0)) return;
    this.directAt.set(name, now);
    const direct = this.directClasses(cfg);
    if (!direct) return;
    // After a restart the in-memory sections are empty even though the
    // durable mirror holds the host's last copy: load it first, so a
    // failed first read keeps last-good data instead of nothing.
    if (this.docs.good(name) === null) await this.docs.refresh(name);
    try {
      this.docs.record(name, await directRead(name, direct.ops, direct.work), now);
    } catch (error) {
      this.docs.recordFailure(name, error);
    }
  }

  /** The sections of one document that the edge reads itself, when it may. */
  private async directRefresh(doc: DocName): Promise<void> {
    const names = doc === "core" ? ["board", "asks"] : doc === "ops" ? ["prs", "priorities"] : [];
    if (names.length === 0) return;
    const cfg = await this.config();
    if (!this.directReady(cfg)) return;
    for (const name of names) await this.directReadSection(name, cfg);
    // The notifier is told of the sections just read directly, so its
    // refresh does not reload the laptop's mirror over them.
    const watched = names.filter((n) => WATCHED.includes(n));
    if (watched.length > 0) {
      await this.notifier.tick(watched).catch(() => {});
    }
  }

  /** The self-check: one direct read of each kind, run when the
   * capability arrives and at most once an hour after, its result kept
   * for health. It proves the path without taking the laptop down. */
  private async maybeSelfCheck(cfg: EdgeConfig | null): Promise<void> {
    const direct = this.directClasses(cfg);
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

  /** The stand-in for the queue's enqueue in a prompt the edge files or
   * answers itself: the issue already exists on GitHub, so only its
   * queue task is left, held for the laptop's return as prompt.enqueue.
   * The packet carries the issue's number; the text is the owner's own. */
  private enqueueRecorder(
    args: Record<string, unknown>,
    by: string,
  ): (packet: unknown) => Promise<{ code: number; stdout: string; stderr: string }> {
    return async (packet) => {
      const issue = Number((packet as { payload?: { issue?: { number?: unknown } } })?.payload?.issue?.number);
      if (!Number.isSafeInteger(issue) || issue <= 0) {
        throw new Error("the prompt's issue could not be read from its packet");
      }
      const at = new Date(this.now()).toISOString();
      const held = await this.heldList();
      held.push({
        id: crypto.randomUUID(),
        kind: "prompt.enqueue",
        args: { issue, text: String(args.text ?? ""), at },
        who: by,
        at,
      });
      await this.saveHeld();
      return { code: 0, stdout: "", stderr: "" };
    };
  }

  /** An action this side runs itself through the laptop's own classes,
   * recorded before and after like own(). Only the sections it touched
   * that the edge reads itself are read again at once. */
  private async directAct(kind: string, args: Record<string, unknown>, by: string, via: string): Promise<Response> {
    const cfg = await this.config();
    const direct = this.directClasses(cfg);
    if (!direct) throw new Error("the edge cannot act itself right now");
    const table = actions(direct.ops, direct.work, this.push, this.phones.devices);
    const action = table[kind];
    const base = { at: new Date(this.now()).toISOString(), who: by, via, action: kind, args: action.audit(args) };
    await this.audit({ ...base, phase: "start" });
    const entry = { id: crypto.randomUUID(), kind, at: base.at, by, note: "" };
    try {
      let result: unknown;
      if (kind === "ask.send") result = await direct.work.ask(args, this.enqueueRecorder(args, by));
      else if (kind === "ask.reply") result = await direct.work.askReply(args, this.enqueueRecorder(args, by));
      else result = await action.run(args);
      await this.audit({ ...base, phase: "done", ok: true });
      await this.logAct({ ...entry, status: "done" });
      for (const name of action.touch) {
        if ((DIRECT_READ_SECTIONS as readonly string[]).includes(name)) {
          await this.directReadSection(name, cfg, true);
        }
      }
      if (action.touch.some((n) => WATCHED.includes(n))) {
        // The sections just read directly are not reloaded from the
        // laptop's mirror by the notifier's refresh.
        const refreshed = action.touch.filter(
          (n) => (DIRECT_READ_SECTIONS as readonly string[]).includes(n) && WATCHED.includes(n),
        );
        await this.notifier.tick(refreshed).catch(() => {});
      }
      return json({ ok: true, result });
    } catch (error) {
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
    }
    // What the host says of every section it has: how current it is, and
    // whether it could still read it. Taken first, so a section that
    // arrives in this report is shown with its health as of this report.
    const told: { name: string; healthChanged: boolean }[] = [];
    for (const [name, theirs] of Object.entries(msg.have ?? {})) {
      if (!Object.hasOwn(SECTIONS, name) || typeof theirs?.hash !== "string" || typeof theirs.at !== "number") continue;
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
    if (fresher.length > 0) await this.docs.touch(fresher);
    if (Array.isArray((msg as { acts?: unknown }).acts)) {
      this.hostActs = ((msg as { acts?: Act[] }).acts ?? []).slice(0, ACTS_KEPT);
      await this.showActs();
    }

    // What became of the held actions the host was given.
    const held = await this.heldList();
    let settled = false;
    for (const ack of Array.isArray(msg.acks) ? msg.acks : []) {
      const at = held.findIndex((h) => h.id === ack?.id);
      if (at < 0) continue;
      const [done] = held.splice(at, 1);
      settled = true;
      if (ack.ok)
        await this.dropAct(done.id); // the host's own record shows it done
      else {
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
    await this.setAlarm(now + SILENT_MS + 1000);
    // Something the owner is told about changed, or a unit's failing is
    // being timed: that clock only moves when the laptop's fresh word on
    // its units is looked at, and a unit that stays failed reads the
    // same in every report.
    if (changed.some((n) => WATCHED.includes(n)) || (await this.notifier.pending())) {
      await this.notifier.tick().catch((error) => console.error(`pm-edge: notify: ${(error as Error).message}`));
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
      if (typeof msg.have?.[name]?.hash === "string" && !(await this.mirror(name))) want.push(name);
    }
    const answer: SyncResponse = {
      ok: true,
      watching: now - this.lastDocAt < WATCHING_MS,
      notify: (await this.push.status()).devices > 0 || (await this.phones.devices.list()).length > 0,
      want,
      commands: held,
      phones: await this.phones.devices.links(),
    };
    return json(answer);
  }

  /** Woken some time after the last sync: if there has been none since,
   * the owner is told once that the host has gone quiet. */
  async alarm(): Promise<void> {
    await this.ready();
    const now = this.now();
    if (!this.hostSeen || now - this.hostSeen < SILENT_MS) return;
    if ((await this.kv.get("silent")) === true) return;
    try {
      const delivery = await this.push.send("trouble", {
        title: "The laptop has gone quiet",
        body: "It has not reported for ten minutes. The app still opens; what you do is held until it is back.",
        url: "/console/",
        tag: "host",
      });
      if (delivery.failed === 0) await this.kv.put("silent", true);
    } catch {
      // Leave the transition pending so a later alarm can retry it.
    }
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
        // Ready to act itself once the laptop is quiet: the switch and
        // tokens are here, and the config carries what direct mode
        // needs. The quiet gate itself is not part of this answer.
        directReady: this.directCapable() && this.directConfigReady(cfg),
        selfCheck: this.selfCheckResult,
        hostSeen: this.hostSeen || null,
        // The commit the host's console is running, as it last said.
        hostVersion: this.hostVersion,
        // The copy of the Android build kept here: when it was built,
        // and whether it is the host's newest. Null if there is none.
        androidBuild: await this.apkState(),
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
          // A document the edge reads itself: refresh its sections
          // directly first, when the laptop has gone quiet.
          await this.directRefresh(seg[1] as DocName);
          const res = docResponse(req, await this.docs.doc(seg[1] as DocName));
          // When the host last reported, beside the document rather than
          // in it: it changes with every report and the content does not.
          const headers = new Headers(res.headers);
          headers.set("X-Host-Seen", String(this.hostSeen ?? 0));
          return new Response(res.body, { status: res.status, headers });
        }
        if (seg.length === 1 && seg[0] === "android") {
          // Where a download would come from: the build GitHub released,
          // if there is one, and the copy of the laptop's build kept here.
          const release = await this.androidRelease(cfg);
          return json({
            release: release && { name: release.name, code: release.code, built: release.built },
            copy: await this.apkState(),
          });
        }
        if (seg.length === 1 && seg[0] === "push") return json(await this.push.status());
        if (seg.length === 1 && seg[0] === "devices") return json(await this.phones.devices.list());
        // Everything else is read from the host as it is now.
        return next("relay", { mode: "live" });
      }
      if (method !== "POST") return json({ error: "method not allowed" }, 405);
      if (!sameSiteWrite(req)) return json({ error: "this request did not come from the console" }, 403);
      if (!(seg.length === 1 && seg[0] === "act")) return next("relay", { mode: "live" });
      if (Number(req.headers.get("content-length") ?? "0") > ACT_LIMIT) return json({ error: "too large" }, 413);
      let body: { kind?: unknown; args?: unknown };
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
      if (!Object.hasOwn(this.table, kind)) {
        // One the edge does itself: while the laptop is known quiet it
        // runs here, through the laptop's own classes, and is never
        // sent to the laptop — so the laptop cannot do it a second
        // time. Otherwise it is the host's to do, as before.
        if ((DIRECT_ACTIONS as readonly string[]).includes(kind) && this.directReady(cfg)) {
          return await this.directAct(kind, args as Record<string, unknown>, by, who.via);
        }
        // The host's to do. The stateless part relays it, and if the
        // host cannot be reached comes back to have it held.
        // It is named here, before it goes anywhere. If the host's answer
        // is lost on the way back, the action is held under the same
        // name, and the host, which wrote the name down before starting,
        // does not do it a second time.
        return next("relay", {
          mode: "act",
          id: crypto.randomUUID(),
          kind,
          args,
          who: by,
          holdable: HOLDABLE.has(kind),
        });
      }
      return await this.own(kind, args as Record<string, unknown>, by, who.via);
    } catch (error) {
      if (error instanceof OpError) return json({ error: error.message }, error.status);
      console.error(`pm-edge: ${method} ${path}: ${(error as Error).message}`);
      return json({ error: "the edge could not do that" }, 500);
    }
  }

  /** An action this side runs itself, recorded before and after. */
  private async own(kind: string, args: Record<string, unknown>, by: string, via: string): Promise<Response> {
    const action = this.table[kind];
    const base = { at: new Date(this.now()).toISOString(), who: by, via, action: kind, args: action.audit(args) };
    await this.audit({ ...base, phase: "start" });
    const entry = { id: crypto.randomUUID(), kind, at: base.at, by, note: "" };
    try {
      const result = await action.run(args);
      await this.audit({ ...base, phase: "done", ok: true });
      await this.logAct({ ...entry, status: "done" });
      // A device has just subscribed: take stock now, so that it is told
      // of what changes from here on and not of what was already there.
      if (kind === "push.on") await this.notifier.tick().catch(() => {});
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
    const body = (await req.json().catch(() => null)) as {
      id?: unknown;
      kind?: unknown;
      args?: unknown;
      who?: unknown;
    } | null;
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
    const changed = await this.take(body?.sections);
    if (Array.isArray(body?.acts)) {
      this.hostActs = (body.acts as Act[]).slice(0, ACTS_KEPT);
      await this.showActs();
    }
    return json({ ok: true, changed });
  }
}
