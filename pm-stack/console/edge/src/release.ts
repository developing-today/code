// The Android build at the edge: the newest build GitHub has
// released, asked for no more often than its cache allows, and the
// copy of a build kept here to hand to the app.
// Extracted from edge.ts for the production-line guard (#14547):
// moves only, no behaviour change.
import {
  type AndroidRelease,
  androidRelease,
  APK_LIMIT,
  type ApkReport,
  type EdgeConfig,
} from "../../src/edge-proto.ts";
import { json, SECURITY } from "../../src/http.ts";
import { timed, unread } from "../../src/timed.ts";
import type { KV } from "./edge-shared.ts";

/** The Android build is kept in pieces of this many bytes. */
export const APK_CHUNK = 512 * 1024;
/** The most pieces a build that may be carried can take. */
export const APK_PIECES = Math.ceil(APK_LIMIT / APK_CHUNK);
/** A build that could not be kept is asked for again no sooner than this. */
export const APK_RETRY_MS = 10 * 60_000;

/** The GitHub release whose address always holds the app's newest build. */
export const RELEASE_TAG = "pm-console-android";
/** GitHub is asked what that release holds no more often than this, */
export const RELEASE_EVERY_MS = 10 * 60_000;
/** this soon again when it could not be asked, */
export const RELEASE_RETRY_MS = 60_000;
/** and is given this long to answer, and this long to send the build. */
export const RELEASE_ASK_MS = 8_000;
export const RELEASE_FETCH_MS = 25_000;

export interface ApkMeta {
  sha256: string;
  size: number;
  at: number;
}

/** What GitHub last said of the released build: when it was asked,
 * whether it could be asked at all, what it described, and if it could
 * not be asked, why. */
export interface Released {
  at: number;
  asked: boolean;
  found: AndroidRelease | null;
  trouble: string | null;
}

/** The released build and the copy kept here, with what is known of
 * each: in memory, and read back from storage the first time each is
 * asked for. */
export class Builds {
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

  constructor(
    private kv: KV,
    private now: () => number,
    private fetchImpl: typeof fetch,
  ) {}

  /** Where the release with the fixed address keeps a file. */
  private releaseUrl(cfg: EdgeConfig, file: string): string {
    return `https://github.com/${cfg.repo}/releases/download/${RELEASE_TAG}/${file}`;
  }

  /** The newest build GitHub Actions has released, by its own
   * description, or null when none is published. GitHub is not asked on
   * every request; when it cannot be asked, what was last known stands. */
  async androidRelease(cfg: EdgeConfig): Promise<AndroidRelease | null> {
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
  async releaseSaid(cfg: EdgeConfig | null): Promise<{ name: string; code: number; built: string } | null> {
    const release = cfg ? await this.androidRelease(cfg) : null;
    return release && { name: release.name, code: release.code, built: release.built };
  }

  /** The released build, as a download, or null if it did not arrive as
   * its description says (GitHub out of reach, or the description read
   * while a new build was being put up). */
  async serveRelease(cfg: EdgeConfig, release: AndroidRelease): Promise<Response | null> {
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
  async apkState(): Promise<{ at: number; current: boolean } | null> {
    const kept = await this.apkKept();
    if (!kept) return null;
    const latest = await this.apkLatest();
    return { at: kept.at, current: !latest || latest.sha256 === kept.sha256 };
  }

  /** The build the host last said it has. */
  async apkLatest(): Promise<ApkMeta | null> {
    if (this.latestApk === undefined)
      this.latestApk = ((await this.kv.get("apk:latest")) as ApkMeta | undefined) ?? null;
    return this.latestApk;
  }

  /** The build a copy of which is kept here. */
  async apkKept(): Promise<(ApkMeta & { chunks: number }) | null> {
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
  async serveApk(kept: ApkMeta & { chunks: number }): Promise<Response> {
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

  /** What a report said of the Android build, taken in: the newest
   * build is remembered (and kept, when the build itself was sent),
   * and whether the build is wanted — the host has it and this side
   * has lost its copy (its storage was reset). */
  async noteBuild(build: ApkReport | null, now: number): Promise<boolean> {
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
    const kept = await this.apkKept();
    const failedLately =
      this.apkFailed?.sha256 === this.latestApk?.sha256 && now - (this.apkFailed?.at ?? 0) < APK_RETRY_MS;
    return Boolean(this.latestApk && kept?.sha256 !== this.latestApk.sha256 && !failedLately);
  }

  /** Why GitHub could not be asked for the released build, the last
   * time it was tried; null when it answered (also when it answered
   * "none", and before it has been asked at all). */
  get releaseTrouble(): string | null {
    return this.released?.trouble ?? null;
  }
}
