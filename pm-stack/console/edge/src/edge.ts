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
import { type Action, ownActions } from "../../src/actions.ts";
import { AuthError, type Identity, identify } from "../../src/auth.ts";
import { Devices, Limiter } from "../../src/devices.ts";
import { type Act, DOC_NAMES, type DocName, Docs, Stamped } from "../../src/docs.ts";
import {
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
const ACT_LIMIT = 256 * 1024;
const HELD_MAX = 100;
const ACTS_KEPT = 30;
const AUDIT_KEPT = 300;
/** "Someone is looking" lasts this long after the last read of a document. */
const WATCHING_MS = 90_000;
/** The host is reported silent after this long without a sync. */
export const SILENT_MS = 10 * 60_000;
/** The laptop is quiet long enough to bypass it for safe reads. */
const DIRECT_READ_AFTER_MS = 60_000;
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
  /** Reads a section from its source when the host has gone quiet. */
  freshRead?: (section: string, config: EdgeConfig) => Promise<unknown>;
  githubReadEnabled?: boolean;
  githubTokenConfigured?: boolean;
  /** Ask to be woken at a time: the check for a silent host. */
  setAlarm?: (at: number) => Promise<void>;
  /** Work that may finish after the response has gone: the object's
   * own waitUntil, so a background read is not cut off with the
   * response. Defaults to letting it run unattended. */
  later?: (work: Promise<unknown>) => void;
}

export class Edge {
  private kv: KV;
  private now: () => number;
  private fetchImpl: typeof fetch;
  private setAlarm: (at: number) => Promise<void>;
  private later: (work: Promise<unknown>) => void;
  private freshRead?: EdgeOptions["freshRead"];
  private githubReadEnabled: boolean;
  private githubTokenConfigured: boolean;
  /** A pull-request read already running: shared by every request
   * while it runs, so a slow GitHub is asked once, not once a poll. */
  private prsRefresh: Promise<void> | null = null;
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
  readonly docs: Docs;
  readonly push: Push;
  readonly phones: Phones;
  private notifier: Notifier;
  private table: Record<string, Action>;

  constructor(options: EdgeOptions) {
    this.kv = options.kv;
    this.now = options.now ?? Date.now;
    this.fetchImpl = options.fetchImpl ?? fetch;
    this.freshRead = options.freshRead;
    this.githubReadEnabled = options.githubReadEnabled ?? false;
    this.githubTokenConfigured = options.githubTokenConfigured ?? false;
    this.setAlarm = options.setAlarm ?? (async () => {});
    this.later = options.later ?? ((work) => void work);
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
    this.notifier = new Notifier(this.docs, this.push, kvStore(this.kv, "seen"));
    this.table = ownActions(this.push, devices);
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
        | { seen: number; seq: number; confirmed?: Record<string, { hash: string; at: number; error: string | null }> }
        | undefined;
      this.hostSeen = host?.seen ?? 0;
      this.hostSeq = host?.seq ?? 0;
      this.confirmed = host?.confirmed ?? {};
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
    if (Number(req.headers.get("content-length") ?? "0") > SYNC_LIMIT) return json({ error: "too large" }, 413);
    const body = new Uint8Array(await req.arrayBuffer());
    if (body.byteLength > SYNC_LIMIT) return json({ error: "too large" }, 413);
    const kid = req.headers.get("x-pm-key") ?? "";
    const key = (await this.kv.get("key")) as { kid: string; jwk: PublicKey } | undefined;
    const now = this.now();
    if (!key || key.kid !== kid) {
      // A key this side has not seen: ask the host for its key, but not
      // on every stranger's request.
      if (req.headers.get("x-pm-key-fetched") === "1" || now - this.lastKeyFetch < KEY_FETCH_EVERY_MS) {
        return json({ error: "this is not signed with the host's key" }, 401);
      }
      this.lastKeyFetch = now;
      return next("key", {});
    }
    if (!(await verify(key.jwk, body, req.headers.get("x-pm-signature") ?? ""))) {
      return json({ error: "this is not signed with the host's key" }, 401);
    }
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
    await this.kv.put("host", { seen: now, seq: msg.at, confirmed: this.confirmed });
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
    if (changed.some((n) => WATCHED.includes(n))) {
      await this.notifier.tick().catch((error) => console.error(`pm-edge: notify: ${(error as Error).message}`));
    }

    // What the host has and this side has lost (its storage was reset).
    const want: string[] = [];
    for (const name of Object.keys(SECTIONS)) {
      if (typeof msg.have?.[name]?.hash === "string" && !(await this.mirror(name))) want.push(name);
    }
    const answer: SyncResponse = {
      ok: true,
      watching: now - this.lastDocAt < WATCHING_MS,
      notify: (await this.push.status()).devices > 0 || (await this.phones.devices.list()).length > 0,
      want,
      commands: held,
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

  /** Read pull requests from GitHub again without making a document
   * wait for it: the read runs after the response, through the
   * object's waitUntil, and a request that arrives while it runs
   * shares it instead of starting another. */
  private refreshPrs(cfg: EdgeConfig): void {
    if (this.prsRefresh) return;
    const read = this.freshRead;
    if (!read) return;
    this.prsRefresh = (async () => {
      try {
        this.docs.record("prs", await read("prs", cfg), this.now());
      } catch (error) {
        // The stored copy is still the last good list: read it back
        // before saying the fresh read failed, so the failure does not
        // hide it. A failed read stamps the section's time, which is
        // what decides whether the stored copy is ever loaded.
        await this.docs.refresh("prs");
        this.docs.recordFailure("prs", error);
      } finally {
        this.prsRefresh = null;
      }
    })();
    this.later(this.prsRefresh);
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
      return json({
        ok: true,
        edge: true,
        configured: cfg !== null,
        githubReadEnabled: this.githubReadEnabled,
        githubTokenConfigured: this.githubTokenConfigured,
        githubReadReady: this.freshRead !== undefined,
        hostSeen: this.hostSeen || null,
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
            androidApp: cfg.androidApp,
            androidPackage: cfg.androidPackage,
            edge: true,
          });
        }
        if (seg.length === 2 && seg[0] === "doc" && DOC_NAMES.includes(seg[1] as DocName)) {
          this.lastDocAt = this.now();
          // Pull requests are a read-only GitHub view. When the laptop is
          // quiet, refresh them directly if this Worker has that capability.
          // The read runs in the background: the document answers at once
          // from what is cached, and the fresh list arrives on the next
          // poll. Only a stale section is refreshed, so GitHub is asked
          // at most once per interval while the laptop is quiet.
          if (
            seg[1] === "ops" &&
            this.freshRead &&
            this.now() - (this.hostSeen ?? 0) >= DIRECT_READ_AFTER_MS &&
            this.docs.isStale("prs")
          ) {
            this.refreshPrs(cfg);
          }
          const res = docResponse(req, await this.docs.doc(seg[1] as DocName));
          // When the host last reported, beside the document rather than
          // in it: it changes with every report and the content does not.
          const headers = new Headers(res.headers);
          headers.set("X-Host-Seen", String(this.hostSeen ?? 0));
          return new Response(res.body, { status: res.status, headers });
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
