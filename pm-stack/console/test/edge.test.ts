// The app served from the edge, with the host behind it.
//
// Nothing here is mocked at the seam under test. A request goes through
// the Worker's real routing (entry.ts) to the real state (edge.ts), and
// when it is the host's it goes on to the host's real handler
// (server.ts), whose own link (sync.ts) reports back through the same
// routing. The stand-ins are at the outside: the platform's storage (a
// map), the sign-in issuer (a key made here that signs real tokens), the
// push service (a list), and the host's sources (plain objects).
//
// Oracles: what the host holds is compared with what the edge serves;
// "run once" is counted on the host; a signature is made here with a key
// the edge was never given; stored values are measured in the map.
import { beforeEach, describe, expect, test } from "bun:test";
import { Edge, type KV, SILENT_MS } from "../edge/src/edge.ts";
import { forgetPoke, type Io, route } from "../edge/src/entry.ts";
import type { Action } from "../src/actions.ts";
import { clearJwksCache, identify } from "../src/auth.ts";
import { readConfig } from "../src/config.ts";
import { Devices, Limiter } from "../src/devices.ts";
import { Docs } from "../src/docs.ts";
import { type EdgeConfig, keyId, newKeyPair, sign } from "../src/edge-proto.ts";
import { OpError } from "../src/errors.ts";
import { Ops } from "../src/ops.ts";
import { type Deps, handle, heldRunner, marked } from "../src/server.ts";
import { memoryStore } from "../src/store.ts";
import { EdgeLink } from "../src/sync.ts";
import { browser, signer } from "./support.ts";

const HOST = "pm.example";
const OWNER = "owner@example.com";
const ISSUER = {
  name: "cloudflare-access",
  issuer: "https://team.cloudflareaccess.com",
  jwksUri: "https://team.cloudflareaccess.com/cdn-cgi/access/certs",
  audience: "aud-1",
  source: "access" as const,
};
const START = Date.parse("2026-11-22T12:00:00Z");
const SECTION_NAMES = [
  "board",
  "sessions",
  "census",
  "asks",
  "questionCount",
  "questions",
  "prs",
  "priorities",
  "agents",
  "services",
  "more",
];

async function setup() {
  const now = { t: START };
  const clock = () => now.t;
  const access = await signer("k1");
  const pushes: { url: string; body: Uint8Array; authorization: string }[] = [];
  const pushFailures = { remaining: 0 };
  const outward = async (url: string | URL | Request, init?: { body?: unknown; headers?: Record<string, string> }) => {
    const u = String(url);
    if (u === ISSUER.jwksUri) return { ok: true, status: 200, json: async () => ({ keys: [access.jwk] }) };
    if (u.startsWith("https://fcm.googleapis.com/")) {
      pushes.push({ url: u, body: init?.body as Uint8Array, authorization: init?.headers?.Authorization ?? "" });
      if (pushFailures.remaining > 0) {
        pushFailures.remaining--;
        return { status: 500 };
      }
      return { status: 201 };
    }
    throw new Error(`unexpected fetch ${u}`);
  };

  // ---- the edge ----------------------------------------------------------
  const stored = new Map<string, unknown>();
  /** Every key written to the edge's storage, in order. */
  const puts: string[] = [];
  const kv: KV = {
    get: async (key) => structuredClone(stored.get(key)),
    put: async (key, value) => {
      puts.push(key);
      stored.set(key, structuredClone(value));
    },
    delete: async (key) => {
      stored.delete(key);
    },
  };
  const alarm = { at: 0 };
  const makeEdge = () =>
    new Edge({
      kv,
      now: clock,
      fetchImpl: outward as never,
      setAlarm: async (at) => {
        alarm.at = at;
      },
    });
  const edge = { now: makeEdge() };

  // ---- the host ----------------------------------------------------------
  const cfg = readConfig({ HOME: "/h", PM_CONSOLE_OWNER_EMAILS: OWNER });
  cfg.issuers = [ISSUER];
  const data: Record<string, unknown> = {
    board: { counts: { claimed: 1, queued: 2 }, now: [{ id: 1, title: "First task" }], next: [], blocked: [] },
    sessions: { moving: [], waiting: [] },
    census: { hands: [], reviews: [], other: [] },
    asks: [] as unknown[],
    questionCount: { total: 0, sets: 0 },
    questions: { open: [] },
    prs: [],
    priorities: { issues: [], truncated: false },
    agents: [],
    services: [] as unknown[],
    more: [],
  };
  const hostDocs = new Docs(
    Object.fromEntries(
      SECTION_NAMES.map((name) => [
        name,
        {
          ttlMs: 10_000,
          load: async () => {
            if (failing.has(name)) throw new Error(`${name}: the source is down`);
            return structuredClone(data[name]);
          },
        },
      ]),
    ),
    clock,
  );
  /** Sections whose source cannot be read just now. */
  const failing = new Set<string>();
  /** Held open by a test to keep an action running. */
  const gate: { wait: Promise<void> | null } = { wait: null };
  const ran: { kind: string; args: Record<string, unknown> }[] = [];
  const table: Record<string, Action> = {
    "ask.send": {
      run: async (a) => {
        ran.push({ kind: "ask.send", args: a });
        const issue = 100 + ran.length;
        (data.asks as unknown[]).unshift({
          issue,
          text: a.text,
          status: "queued",
          replies: [],
          updated: new Date(now.t).toISOString(),
        });
        return { issue };
      },
      touch: ["asks"],
      audit: () => ({}),
    },
    "issue.comment": {
      run: async (a) => {
        ran.push({ kind: "issue.comment", args: a });
        if (a.issue === 13) throw new OpError(404, "no such issue");
        return { commented: a.issue };
      },
      touch: [],
      audit: (a) => ({ issue: a.issue }),
    },
    "pr.merge": {
      run: async (a) => {
        ran.push({ kind: "pr.merge", args: a });
        return { merged: a.pr };
      },
      touch: ["prs"],
      audit: (a) => ({ pr: a.pr }),
    },
    // An answer the dashboard may refuse without anything being thrown.
    "question.answer": {
      run: async (a) => {
        ran.push({ kind: "question.answer", args: a });
        return a.qid === "settled"
          ? { applied: 0, refused: [{ qid: a.qid, why: "already-resolved" }] }
          : { applied: 1, refused: [] };
      },
      touch: [],
      audit: (a) => ({ qid: a.qid }),
    },
    "need.comment": {
      run: async (a) => {
        ran.push({ kind: "need.comment", args: a });
        await gate.wait;
        return { commented: a.ref };
      },
      touch: [],
      audit: (a) => ({ ref: a.ref }),
    },
  };
  const audited: Record<string, unknown>[] = [];
  const hostDeps: Deps = {
    cfg,
    ops: new Ops(cfg, async () => ({ code: 0, stdout: "{}", stderr: "" })),
    docs: hostDocs,
    actions: table,
    phones: { devices: new Devices(memoryStore()), link: new Limiter(30, 60_000), refused: new Limiter(60, 60_000) },
    identify: (req, peer) => identify(req, cfg, { peer, fetchImpl: outward as never, now: clock() }),
    audit: async (entry) => {
      audited.push(entry);
    },
  };
  const net = { hostUp: true, edgeUp: true, loseAnswer: false, giveUpEarly: false };
  /** Requests to the host still running after the edge stopped waiting. */
  const stillRunning: Promise<unknown>[] = [];
  const hostSaw: string[] = [];
  const edgeConfig: EdgeConfig = {
    owners: [OWNER],
    certNames: [],
    issuers: [ISSUER],
    repo: cfg.repo,
    queue: cfg.queue,
    androidPackage: cfg.androidPackage,
    androidFingerprints: [],
    androidApp: true,
  };

  // ---- the Worker's routing, with the two wired together -----------------
  const later: Promise<unknown>[] = [];
  const files = new Map([
    ["/console/index.html", "<!doctype html><title>PM</title>"],
    ["/console/app.js", "// app"],
    ["/icons/icon-192.png", "PNG"],
  ]);
  const io: Io = {
    state: (r) => edge.now.fetch(r),
    asset: async (path) => (files.has(path) ? new Response(files.get(path)) : new Response("", { status: 404 })),
    host: async (r) => {
      if (!net.hostUp) return null;
      hostSaw.push(`${r.method} ${new URL(r.url).pathname}`);
      // Through the tunnel: the connection is local, the headers are the edge's.
      const answer = handle(r, hostDeps, "127.0.0.1").then(marked);
      // The edge stops waiting while the host is still at it.
      if (net.giveUpEarly) {
        stillRunning.push(answer);
        return null;
      }
      // The host does the work and its answer is lost on the way back.
      if (net.loseAnswer) {
        await answer;
        return null;
      }
      return answer;
    },
    publicHost: HOST,
    version: "test",
    later: (work) => later.push(work),
    now: clock,
  };
  const keys = memoryStore();
  const done = memoryStore();
  const makeLink = () =>
    new EdgeLink({
      url: `https://${HOST}`,
      keys,
      done,
      docs: hostDocs,
      phones: hostDeps.phones!.devices,
      config: async () => edgeConfig,
      run: heldRunner(hostDeps),
      fetchImpl: (async (url: string, init: RequestInit) => {
        if (!net.edgeUp) throw new Error("no route to the edge");
        return route(
          new Request(url, { ...init, headers: { ...(init.headers as Record<string, string>), "cf-ray": "1" } }),
          io,
        );
      }) as never,
      now: clock,
      log: () => {},
    });
  const link = { now: makeLink() };
  hostDeps.link = {
    publicKey: () => link.now.publicKey(),
    begin: (id) => link.now.begin(id),
    end: (id, fate) => link.now.end(id, fate),
  };

  const token = (claims: Record<string, unknown> = {}) =>
    access.sign({
      iss: ISSUER.issuer,
      aud: [ISSUER.audience],
      exp: now.t / 1000 + 600,
      iat: now.t / 1000 - 5,
      sub: "u1",
      email: OWNER,
      ...claims,
    });
  /** A request from the owner's browser, as it reaches the Worker, with
   * a sign-in token good at this moment unless another is given. */
  const browse = async (
    path: string,
    init: {
      method?: string;
      body?: unknown;
      token?: string | null;
      host?: string;
      headers?: Record<string, string>;
    } = {},
  ) =>
    route(
      new Request(`https://${init.host ?? HOST}${path}`, {
        method: init.method ?? (init.body === undefined ? "GET" : "POST"),
        body: init.body === undefined ? undefined : JSON.stringify(init.body),
        headers: {
          "cf-ray": "1",
          ...(init.token === null ? {} : { "cf-access-jwt-assertion": init.token ?? (await token()) }),
          ...(init.body === undefined ? {} : { "content-type": "application/json", "x-pm-console": "1" }),
          ...init.headers,
        },
      }),
      io,
    );
  const act = (kind: string, args: Record<string, unknown> = {}) =>
    browse("/console/api/act", { body: { kind, args } });
  const core = async () => {
    const res = await browse("/console/api/doc/core");
    return { res, doc: (await res.json()) as Record<string, any> };
  };
  /** Load every section on the host, as the host does while someone looks. */
  const hostReads = () => hostDocs.touch(SECTION_NAMES);
  return {
    now,
    edge,
    makeEdge,
    stored,
    puts,
    keys,
    done,
    alarm,
    pushes,
    pushFailures,
    hostDocs,
    hostDeps,
    data,
    ran,
    audited,
    net,
    hostSaw,
    io,
    link,
    makeLink,
    later,
    token,
    browse,
    act,
    core,
    hostReads,
    failing,
    gate,
    stillRunning,
    table,
    access,
    edgeConfig,
  };
}

beforeEach(() => {
  clearJwksCache();
  forgetPoke();
});

describe("before the host's first report", () => {
  test("the edge stays out of the way: the host answers everything, as before", async () => {
    const w = await setup();
    const health = (await (await w.browse("/console/health", { token: null })).json()) as Record<string, unknown>;
    expect(health).toEqual({
      ok: true,
      edge: true,
      configured: false,
      hostSeen: null,
      answered: 0,
      deferred: 0,
      version: "test",
      host: HOST,
    });
    // The owner's page is the host's own, reached through the edge.
    const page = await w.browse("/console/api/me");
    expect(page.headers.get("x-pm-console")).toBe("1");
    expect(((await page.json()) as { email: string; edge?: boolean }).edge).toBeUndefined();
    // And the host still refuses a stranger itself.
    expect((await w.browse("/console/api/me", { token: null })).status).toBe(401);
    expect(w.hostSaw).toEqual(["GET /console/api/me", "GET /console/api/me"]);
  });

  test("the first report teaches the edge the host's key and who the owner is", async () => {
    const w = await setup();
    await w.hostReads();
    const answer = await w.link.now.once();
    expect(answer).toMatchObject({ ok: true, watching: false, notify: false, want: [], commands: [] });
    // The key came from the host itself, through the tunnel.
    expect(w.hostSaw).toEqual(["GET /console/edge/key"]);
    const kept = w.stored.get("key") as { kid: string; jwk: Record<string, string> };
    expect(kept.jwk).toEqual(await w.link.now.publicKey());
    expect(JSON.stringify([...w.stored.entries()])).not.toContain('"d":'); // never a private key
    const health = (await (await w.browse("/console/health", { token: null })).json()) as Record<string, unknown>;
    expect(health).toMatchObject({ configured: true, hostSeen: START });
    // A second report needs no key fetch and sends nothing that has not changed.
    w.now.t += 5_000;
    w.puts.length = 0;
    await w.link.now.once();
    expect(w.hostSaw).toEqual(["GET /console/edge/key"]);
    expect(w.puts.filter((k) => k.startsWith("m:"))).toEqual([]);
    // One section changes: that one is sent, and only that one.
    (w.data.board as { counts: { queued: number } }).counts.queued = 9;
    w.now.t += 11_000;
    await w.hostReads();
    await w.link.now.once();
    expect(w.puts.filter((k) => k.startsWith("m:"))).toEqual(["m:board:0", "m:board"]);
  });
});

describe("the owner, at the edge", () => {
  test("the app and its state come from the edge, and only for a verified owner", async () => {
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    w.hostSaw.length = 0;

    const page = await w.browse("/console/");
    expect(page.status).toBe(200);
    expect(await page.text()).toContain("<title>PM</title>");
    expect(page.headers.get("content-security-policy")).toContain("default-src 'self'");
    expect((await w.browse("/console/app.js")).headers.get("content-type")).toBe("text/javascript; charset=utf-8");
    expect((await w.browse("/console")).status).toBe(308);
    const home = await w.browse("/");
    expect([home.status, home.headers.get("location")]).toEqual([302, `https://${HOST}/console/`]);

    const { res, doc } = await w.core();
    expect(res.status).toBe(200);
    // Exactly what the host holds, with the time the host read it.
    expect(doc.board).toEqual(w.data.board);
    expect(doc.asks).toEqual([]);
    expect(Number(res.headers.get("x-as-of"))).toBe(START);
    expect(Number(res.headers.get("x-host-seen"))).toBe(START);
    expect(doc.edge).toEqual({ held: 0 });
    // Unchanged: the tag says so and nothing is sent again.
    const again = await w.browse("/console/api/doc/core", {
      headers: { "if-none-match": res.headers.get("etag") as string },
    });
    expect(again.status).toBe(304);
    expect(await (await w.browse("/console/api/me")).json()).toMatchObject({
      email: OWNER,
      edge: true,
      androidApp: true,
    });
    // None of that asked the host for the page or its state (only the
    // one background word that someone is looking).
    await Promise.all(w.later);
    expect(w.hostSaw).toEqual(["GET /console/api/doc/core"]);

    // Not surely the owner: the edge says nothing itself and leaves it
    // to the host, which refuses as it always has.
    w.hostSaw.length = 0;
    const strangers = [
      null,
      "junk",
      await w.token({ email: "someone@else.example" }),
      await w.token({ aud: ["another-app"] }),
      await w.token({ exp: w.now.t / 1000 - 3600 }),
      await (await signer("k1")).sign({
        iss: ISSUER.issuer,
        aud: [ISSUER.audience],
        exp: w.now.t / 1000 + 600,
        email: OWNER,
      }),
    ];
    const paths = ["/console/", "/console/app.js", "/console/api/doc/core", "/console/api/me", "/", "/attention"];
    for (const bad of strangers) {
      for (const path of paths) {
        const refused = await w.browse(path, { token: bad });
        expect([401, 403]).toContain(refused.status);
        expect(refused.headers.get("x-pm-console")).toBe("1"); // the host's own refusal
        expect(await refused.text()).not.toContain("First task");
      }
    }
    expect(w.hostSaw).toHaveLength(strangers.length * paths.length);
    // With the host out of reach there is nobody to judge: nothing is said at all.
    w.net.hostUp = false;
    for (const bad of strangers) {
      for (const path of paths) {
        const closed = await w.browse(path, { token: bad });
        expect(closed.status).toBe(503);
        expect(await closed.text()).not.toContain("First task");
      }
    }
    const health = (await (await w.browse("/console/health", { token: null })).json()) as {
      answered: number;
      deferred: number;
    };
    expect(health.deferred).toBe(strangers.length * paths.length * 2);
    expect(health.answered).toBeGreaterThan(0);
  });

  test("the state is as old as the host's last report, and says so", async () => {
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    // The host goes on reporting: the document stays current.
    for (let i = 0; i < 3; i++) {
      w.now.t += 5_000;
      (w.data.board as { counts: { queued: number } }).counts.queued++;
      await w.hostReads();
      await w.link.now.once();
    }
    let { res, doc } = await w.core();
    expect(doc.board.counts.queued).toBe(5);
    expect(Number(res.headers.get("x-as-of"))).toBe(w.now.t);
    // What counts is when the host read the state, not when it was
    // reported: a read at one time, reported seven seconds later.
    w.now.t += 11_000;
    await w.hostReads();
    const read = w.now.t;
    w.now.t += 7_000;
    await w.link.now.once();
    const last = w.now.t;
    // The host goes quiet for an hour: same state, and its age is plain,
    // however many times it is asked for.
    w.net.edgeUp = false;
    w.now.t += 60 * 60_000;
    for (let asked = 0; asked < 3; asked++) {
      ({ res, doc } = await w.core());
      expect(doc.board.counts.queued).toBe(5);
      expect(Number(res.headers.get("x-as-of"))).toBe(read + 30_000); // the board may be 30 seconds unreported
      expect(Number(res.headers.get("x-host-seen"))).toBe(last);
      w.now.t += 1_000;
    }
    // The edge restarts: its copy is exactly as current as the host last said, no more and no less.
    w.edge.now = w.makeEdge();
    ({ res } = await w.core());
    expect(Number(res.headers.get("x-as-of"))).toBe(read + 30_000);
    expect(Number(res.headers.get("x-host-seen"))).toBe(last);
  });

  test("when the host cannot read a source, the edge says so, and stops saying so when it can", async () => {
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    expect((await w.core()).doc.health.census).toEqual({ ok: true, error: null });
    // The source goes down. The host keeps its last good copy, so the
    // data does not change, and neither does its fingerprint.
    w.failing.add("census");
    w.now.t += 11_000;
    await w.hostReads();
    await w.link.now.once();
    let { doc } = await w.core();
    expect(doc.health.census).toEqual({ ok: false, error: "census: the source is down" });
    expect(doc.census).toEqual(w.data.census);
    expect(doc.health.board).toEqual({ ok: true, error: null });
    // The edge restarts: it still knows.
    w.edge.now = w.makeEdge();
    expect((await w.core()).doc.health.census.ok).toBe(false);
    // The source comes back with the same data.
    w.failing.delete("census");
    w.now.t += 11_000;
    await w.hostReads();
    await w.link.now.once();
    ({ doc } = await w.core());
    expect(doc.health.census).toEqual({ ok: true, error: null });
  });

  test("a large section is kept in pieces and read back whole after a restart", async () => {
    const w = await setup();
    const big = { open: Array.from({ length: 4000 }, (_, i) => ({ qid: `q${i}`, question: "é".repeat(400) })) };
    w.data.questions = big;
    await w.hostReads();
    await w.link.now.once();
    const pieces = [...w.stored.entries()].filter(([k]) => k.startsWith("m:questions:"));
    expect(pieces.length).toBeGreaterThan(2);
    for (const [, value] of pieces) expect((value as string).length).toBeLessThanOrEqual(600_000);
    // A new object with the same storage: nothing in memory.
    w.edge.now = w.makeEdge();
    const res = await w.browse("/console/api/doc/questions");
    expect(((await res.json()) as { questions: unknown }).questions).toEqual(big);
    // A shorter one leaves no stale piece behind.
    w.data.questions = { open: [] };
    w.now.t += 11_000;
    await w.hostReads();
    await w.link.now.once();
    expect([...w.stored.keys()].filter((k) => k.startsWith("m:questions:"))).toEqual(["m:questions:0"]);
  });

  test("opening the app tells the host that someone is looking, and not on every read", async () => {
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    w.hostSaw.length = 0;
    await w.core();
    await w.core();
    await Promise.all(w.later);
    expect(w.hostSaw).toEqual(["GET /console/api/doc/core"]);
    // The host now knows, and the edge says so too.
    expect(w.hostDocs.watched()).toBe(true);
    w.now.t += 1_000;
    expect((await w.link.now.once())?.watching).toBe(true);
    w.now.t += 21_000;
    await w.core();
    await Promise.all(w.later);
    expect(w.hostSaw).toHaveLength(2);
    // Nobody reads for a while: not watching.
    w.now.t += 91_000;
    expect((await w.link.now.once())?.watching).toBe(false);
  });
});

describe("actions", () => {
  test("with the host in reach, an action is done there at once and the edge's copy already shows it", async () => {
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    const res = await w.act("ask.send", { text: "What is blocking the queue?" });
    expect(res.status).toBe(200);
    // The owner gets the result and nothing of the edge's housekeeping.
    expect(await res.json()).toEqual({ ok: true, result: { issue: 101 } });
    expect(w.ran).toEqual([{ kind: "ask.send", args: { text: "What is blocking the queue?" } }]);
    // No report has been made since, and the document has the new prompt and the record of the action.
    const { doc } = await w.core();
    expect(doc.asks.map((a: { issue: number }) => a.issue)).toEqual([101]);
    expect(doc.acts.map((a: { kind: string; status: string }) => [a.kind, a.status])).toEqual([["ask.send", "done"]]);
    // The host recorded it as the owner's, through the sign-in.
    expect(w.audited.filter((e) => e.phase === "done")).toEqual([
      expect.objectContaining({ who: OWNER, via: "cloudflare-access", action: "ask.send", ok: true }),
    ]);
    // A refusal by the host is the host's answer, not "unreachable".
    const refused = await w.act("issue.comment", { issue: 13, text: "x" });
    expect([refused.status, await refused.json()]).toEqual([404, { error: "no such issue" }]);
    expect((w.stored.get("held") as unknown[] | undefined) ?? []).toEqual([]);
  });

  test("with the host out of reach, what may be done late is held, done once when it is back, and the rest is refused", async () => {
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    w.net.hostUp = false;
    w.net.edgeUp = false;

    const held = await w.act("ask.send", { text: "Do this when you are back" });
    expect(await held.json()).toEqual({ ok: true, held: true, result: null });
    // A merge acts on what the owner saw; it is never done late.
    const merge = await w.act("pr.merge", { pr: 7 });
    expect(merge.status).toBe(503);
    expect(((await merge.json()) as { error: string }).error).toMatch(
      /not something to do late, so it is not kept\. Check whether it happened/,
    );
    expect(w.ran).toEqual([]);
    // The app says what is waiting.
    let { doc } = await w.core();
    expect(doc.edge).toEqual({ held: 1 });
    expect(doc.acts[0]).toMatchObject({ kind: "ask.send", status: "running", by: OWNER });
    expect(doc.acts[0].note).toMatch(/^Held/);
    // Reads that are the host's own say why they cannot be had.
    const live = await w.browse("/console/api/prs/7");
    expect([live.status, ((await live.json()) as { error: string }).error]).toEqual([
      503,
      "The laptop cannot be reached, and this is read from it as it is now.",
    ]);
    const pageRes = await w.browse("/attention");
    expect(pageRes.status).toBe(503);
    expect(await pageRes.text()).toContain("Open the app");
    // The edge restarts while the host is away: nothing held is lost.
    w.edge.now = w.makeEdge();
    expect((await w.core()).doc.edge).toEqual({ held: 1 });

    // The host is back. Its report brings the held action down, it runs
    // it, and the next report says so.
    w.net.hostUp = true;
    w.net.edgeUp = true;
    w.now.t += 60_000;
    const first = await w.link.now.once();
    expect(first?.commands.map((c) => c.kind)).toEqual(["ask.send"]);
    expect(w.ran).toEqual([{ kind: "ask.send", args: { text: "Do this when you are back" } }]);
    // The host restarts before its answer reaches the edge (what it had
    // to say was only in memory). The edge offers the action again; the
    // host finds it written down as done, does not run it again, and
    // says so.
    w.link.now = w.makeLink();
    w.hostDeps.link = { publicKey: () => w.link.now.publicKey() };
    w.now.t += 1_000;
    const offered = await w.link.now.once();
    expect(offered?.commands.map((c) => c.kind)).toEqual(["ask.send"]);
    expect(w.ran).toHaveLength(1);
    w.now.t += 1_000;
    const second = await w.link.now.once();
    expect(second?.commands).toEqual([]);
    expect(w.ran).toHaveLength(1);
    ({ doc } = await w.core());
    expect(doc.edge).toEqual({ held: 0 });
    expect(doc.asks.map((a: { text: string }) => a.text)).toEqual(["Do this when you are back"]);
    // One record of it, the host's, saying it was sent late.
    expect(doc.acts.filter((a: { kind: string }) => a.kind === "ask.send")).toEqual([
      expect.objectContaining({ status: "done", by: OWNER, note: expect.stringMatching(/^Sent late/) }),
    ]);
    expect(w.audited.filter((e) => e.via === "edge-held").map((e) => [e.phase, e.who, e.action])).toEqual([
      ["start", OWNER, "ask.send"],
      ["done", OWNER, "ask.send"],
    ]);
  });

  test("a held action the host cannot do is reported failed, and one it was interrupted in is not tried again", async () => {
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    w.net.hostUp = false;
    await w.act("issue.comment", { issue: 13, text: "late" });
    await w.act("issue.comment", { issue: 14, text: "late too" });
    w.net.hostUp = true;
    // The host dies after writing down that it started the second one:
    // stand in for that by marking it started before the report.
    const ids = (w.stored.get("held") as { id: string }[]).map((h) => h.id);
    const done = { [ids[1]]: "started" };
    w.link.now = new EdgeLink({
      url: `https://${HOST}`,
      keys: memoryStore(
        JSON.stringify(await (async () => ({ publicJwk: await w.link.now.publicKey(), privateJwk: null }))()),
      ),
      done: memoryStore(JSON.stringify(done)),
      docs: w.hostDocs,
      phones: w.hostDeps.phones!.devices,
      config: async () => w.edgeConfig,
      run: heldRunner(w.hostDeps),
      fetchImpl: (async (url: string, init: RequestInit) =>
        route(
          new Request(url, { ...init, headers: { ...(init.headers as Record<string, string>), "cf-ray": "1" } }),
          w.io,
        )) as never,
      now: () => w.now.t,
      log: () => {},
    });
    w.hostDeps.link = { publicKey: () => w.link.now.publicKey() };
    w.now.t += 31_000; // past the wait before a changed key is fetched again
    await w.link.now.once();
    w.now.t += 1_000;
    await w.link.now.once();
    // The first was tried once and failed; the second was never run.
    expect(w.ran).toEqual([{ kind: "issue.comment", args: { issue: 13, text: "late" } }]);
    const { doc } = await w.core();
    expect(doc.edge).toEqual({ held: 0 });
    const notes = doc.acts
      .filter((a: { status: string }) => a.status === "failed")
      .map((a: { note: string }) => a.note);
    expect(notes).toHaveLength(2);
    expect(notes.join(" | ")).toMatch(/no such issue/);
    expect(notes.join(" | ")).toMatch(/restarted while doing this/);
  });

  test("an action the host did, whose answer was lost on the way back, is not done again", async () => {
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    w.net.loseAnswer = true;
    const res = await w.act("ask.send", { text: "only once" });
    w.net.loseAnswer = false;
    // The edge cannot tell "never arrived" from "done, answer lost": it holds it.
    expect(await res.json()).toEqual({ ok: true, held: true, result: null });
    expect(w.ran).toHaveLength(1);
    // The host is offered it under the name it was relayed with, knows
    // it did it, and says so instead of doing it again.
    w.now.t += 1_000;
    const offered = await w.link.now.once();
    expect(offered?.commands.map((c) => c.kind)).toEqual(["ask.send"]);
    expect(w.ran).toHaveLength(1);
    w.now.t += 1_000;
    expect((await w.link.now.once())?.commands).toEqual([]);
    const { doc } = await w.core();
    expect(doc.edge).toEqual({ held: 0 });
    expect(doc.asks.map((a: { text: string }) => a.text)).toEqual(["only once"]);
    expect(
      doc.acts.filter((a: { kind: string }) => a.kind === "ask.send").map((a: { status: string }) => a.status),
    ).toEqual(["done"]);
  });

  test("an action the host is still doing when the edge stops waiting is neither repeated nor reported early", async () => {
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    let open = () => {};
    w.gate.wait = new Promise<void>((resolve) => {
      open = resolve;
    });
    w.net.giveUpEarly = true;
    const res = await w.act("need.comment", { ref: "code#1", text: "slow" });
    w.net.giveUpEarly = false;
    expect(await res.json()).toEqual({ ok: true, held: true, result: null });
    // Offered again while still running: not run a second time, and not answered for yet.
    w.now.t += 1_000;
    const during = await w.link.now.once();
    expect(during?.commands.map((c) => c.kind)).toEqual(["need.comment"]);
    w.now.t += 1_000;
    expect((await w.link.now.once())?.commands).toHaveLength(1);
    expect(w.ran).toHaveLength(1);
    // It finishes. Now the host answers for it, once.
    open();
    await Promise.all(w.stillRunning);
    w.now.t += 1_000;
    await w.link.now.once();
    w.now.t += 1_000;
    expect((await w.link.now.once())?.commands).toEqual([]);
    expect(w.ran).toHaveLength(1);
    expect((await w.core()).doc.edge).toEqual({ held: 0 });
  });

  test("an answer sent late that the dashboard no longer takes is reported as not recorded", async () => {
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    w.net.hostUp = false;
    await w.act("question.answer", { set: "s", ledger: "l", qid: "settled", optionId: "a" });
    await w.act("question.answer", { set: "s", ledger: "l", qid: "open", optionId: "a" });
    w.net.hostUp = true;
    w.now.t += 1_000;
    await w.link.now.once();
    w.now.t += 1_000;
    await w.link.now.once();
    expect(w.ran.map((r) => r.args.qid)).toEqual(["settled", "open"]);
    const { doc } = await w.core();
    expect(doc.edge).toEqual({ held: 0 });
    const answers = doc.acts.filter((a: { kind: string }) => a.kind === "question.answer");
    expect(answers.map((a: { status: string }) => a.status).sort()).toEqual(["done", "failed"]);
    expect(answers.find((a: { status: string }) => a.status === "failed").note).toMatch(
      /not recorded: already-resolved/,
    );
  });

  test("a held action is dropped after a day, and said to have been", async () => {
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    w.net.hostUp = false;
    await w.act("ask.send", { text: "too late" });
    w.net.hostUp = true;
    w.now.t += 24 * 60 * 60_000 + 1;
    const answer = await w.link.now.once();
    expect(answer?.commands).toEqual([]);
    expect(w.ran).toEqual([]);
    const { doc } = await w.core();
    expect(doc.edge).toEqual({ held: 0 });
    expect(doc.acts[0]).toMatchObject({
      kind: "ask.send",
      status: "failed",
      note: expect.stringMatching(/more than a day/),
    });
  });

  test("a write must come from the app's own page, and no more than a hundred wait", async () => {
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    const foreign = await w.browse("/console/api/act", {
      body: { kind: "ask.send", args: {} },
      headers: { origin: "https://evil.example" },
    });
    expect(foreign.status).toBe(403);
    const noHeader = await route(
      new Request(`https://${HOST}/console/api/act`, {
        method: "POST",
        body: "{}",
        headers: { "cf-access-jwt-assertion": await w.token(), "cf-ray": "1" },
      }),
      w.io,
    );
    expect(noHeader.status).toBe(403);
    expect((await w.act("ask.send", [] as never)).status).toBe(400);
    // The same for an action the edge runs itself: refused here, and nothing subscribed.
    const phone = await browser();
    const crossSite = await w.browse("/console/api/act", {
      body: { kind: "push.on", args: { subscription: phone.subscription, label: "x" } },
      headers: { origin: "https://evil.example" },
    });
    expect(crossSite.status).toBe(403);
    expect(((await (await w.browse("/console/api/push")).json()) as { devices: number }).devices).toBe(0);
    w.net.hostUp = false;
    for (let i = 0; i < 100; i++) expect((await w.act("ask.send", { text: `n${i}` })).status).toBe(200);
    const full = await w.act("ask.send", { text: "one too many" });
    expect(full.status).toBe(503);
    expect((w.stored.get("held") as unknown[]).length).toBe(100);
  });
});

describe("the host's report is the host's", () => {
  const report = async (
    w: Awaited<ReturnType<typeof setup>>,
    change: (msg: Record<string, unknown>) => void,
    sig?: (body: Uint8Array) => Promise<string>,
    kid?: string,
  ) => {
    const msg: Record<string, unknown> = { v: 1, at: w.now.t, config: w.edgeConfig, sections: [], have: {}, acks: [] };
    change(msg);
    const body = new TextEncoder().encode(JSON.stringify(msg));
    const pub = await w.link.now.publicKey();
    return route(
      new Request(`https://${HOST}/console/edge/sync`, {
        method: "POST",
        body,
        headers: {
          "x-pm-key": kid ?? (await keyId(pub)),
          "x-pm-signature": sig ? await sig(body) : "AAAA",
          "cf-ray": "1",
        },
      }),
      w.io,
    );
  };

  test("a report not signed by the host changes nothing", async () => {
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    const before = JSON.stringify([...w.stored.entries()]);
    const forged = { owners: ["attacker@example.com"] };
    // No signature worth the name.
    expect((await report(w, (m) => Object.assign(m.config as object, forged))).status).toBe(401);
    // A real signature, by a key that is not the host's, under the host's key name.
    const stranger = await newKeyPair();
    const bySomeoneElse = (body: Uint8Array) => sign(stranger.privateJwk, body);
    w.now.t += 1_000;
    expect((await report(w, (m) => Object.assign(m.config as object, forged), bySomeoneElse)).status).toBe(401);
    // The same under its own key name: the edge asks the host for its key, gets the host's, and refuses.
    w.hostSaw.length = 0;
    w.now.t += 31_000;
    const own = await report(
      w,
      (m) => Object.assign(m.config as object, forged),
      bySomeoneElse,
      await keyId(stranger.publicJwk),
    );
    expect(own.status).toBe(401);
    expect(w.hostSaw).toEqual(["GET /console/edge/key"]);
    // And it cannot make the edge ask the host again and again.
    w.now.t += 1_000;
    expect((await report(w, () => {}, bySomeoneElse, await keyId(stranger.publicJwk))).status).toBe(401);
    expect(w.hostSaw).toEqual(["GET /console/edge/key"]);
    expect(JSON.stringify([...w.stored.entries()])).toBe(before);
    // The owner is still the owner, and the forged one is nobody.
    expect((await w.browse("/console/api/me")).status).toBe(200);
    expect(
      (await w.browse("/console/api/me", { token: await w.token({ email: "attacker@example.com" }) })).status,
    ).toBe(403);
  });

  test("an old report, a replayed one and a malformed one are refused", async () => {
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    // The host's own private key is not reachable from here, so a second
    // link over the same key store plays the host.
    const captured: { url: string; init: RequestInit }[] = [];
    const tap = w.makeLink();
    (tap as unknown as { fetchImpl: unknown }).fetchImpl = async (url: string, init: RequestInit) => {
      captured.push({ url, init });
      return route(
        new Request(url, { ...init, headers: { ...(init.headers as Record<string, string>), "cf-ray": "1" } }),
        w.io,
      );
    };
    w.now.t += 5_000;
    expect(await tap.once()).not.toBeNull();
    const again = () =>
      route(
        new Request(captured[0].url, {
          ...captured[0].init,
          headers: { ...(captured[0].init.headers as Record<string, string>), "cf-ray": "1" },
        }),
        w.io,
      );
    // Sent again, byte for byte: refused.
    const replay = await again();
    expect([replay.status, ((await replay.json()) as { error: string }).error]).toEqual([409, "replay"]);
    // Kept and sent an hour later: refused for its age.
    w.now.t += 60 * 60_000;
    const stale = await again();
    expect([stale.status, ((await stale.json()) as { error: string }).error]).toEqual([409, "clock"]);
    // Too large to be a report.
    const huge = await route(
      new Request(`https://${HOST}/console/edge/sync`, {
        method: "POST",
        body: "x",
        headers: { "content-length": String(9 * 1024 * 1024), "cf-ray": "1" },
      }),
      w.io,
    );
    expect(huge.status).toBe(413);
  });

  test("a report from the host with settings the edge cannot use is refused whole", async () => {
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    const kept = JSON.parse(w.keys.text() as string) as { privateJwk: JsonWebKey };
    const signed = async (config: unknown) => {
      w.now.t += 1_000;
      const body = new TextEncoder().encode(
        JSON.stringify({ v: 1, at: w.now.t, config, sections: [], have: {}, acks: [] }),
      );
      return route(
        new Request(`https://${HOST}/console/edge/sync`, {
          method: "POST",
          body,
          headers: {
            "x-pm-key": await keyId(await w.link.now.publicKey()),
            "x-pm-signature": await sign(kept.privateJwk, body),
            "cf-ray": "1",
          },
        }),
        w.io,
      );
    };
    const before = JSON.stringify(w.stored.get("cfg"));
    for (const bad of [
      { ...w.edgeConfig, issuers: [{ ...ISSUER, jwksUri: "http://team.cloudflareaccess.com/certs" }] }, // keys over plain http
      { ...w.edgeConfig, issuers: [{ ...ISSUER, issuer: "http://team.cloudflareaccess.com" }] },
      { ...w.edgeConfig, issuers: [{ ...ISSUER, source: "cookie" }] },
      { ...w.edgeConfig, owners: OWNER }, // not a list
      { ...w.edgeConfig, issuers: "none" },
      null,
    ]) {
      expect((await signed(bad)).status).toBe(400);
    }
    expect(JSON.stringify(w.stored.get("cfg"))).toBe(before);
    // The same key signing a well-formed one is accepted: the refusals above were about the settings.
    expect((await signed(w.edgeConfig)).status).toBe(200);
  });

  test("the host itself will not run late what may not be done late", async () => {
    const w = await setup();
    const runHeld = heldRunner(w.hostDeps);
    const at = new Date(w.now.t).toISOString();
    await expect(runHeld({ id: "x1", kind: "pr.merge", args: { pr: 7 }, who: OWNER, at })).rejects.toThrow(
      /may be done late/,
    );
    await expect(runHeld({ id: "x2", kind: "no.such.action", args: {}, who: OWNER, at })).rejects.toThrow(
      /may be done late/,
    );
    expect(w.ran).toEqual([]);
    expect(w.audited).toEqual([]);
    // One that may: run, and recorded before and after as the owner's, late.
    await runHeld({ id: "x3", kind: "ask.send", args: { text: "late" }, who: OWNER, at });
    expect(w.ran).toEqual([{ kind: "ask.send", args: { text: "late" } }]);
    expect(w.audited.map((e) => [e.phase, e.via, e.who])).toEqual([
      ["start", "edge-held", OWNER],
      ["done", "edge-held", OWNER],
    ]);
  });

  test("an address that is not the routed hostname has no host behind it", async () => {
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    w.hostSaw.length = 0;
    const elsewhere = "pm-console-edge.example.workers.dev";
    expect((await w.browse("/console/health", { host: elsewhere, token: null })).status).toBe(200);
    expect((await w.browse("/manifest.webmanifest", { host: elsewhere, token: null })).status).toBe(200);
    expect((await w.browse("/icons/icon-192.png", { host: elsewhere, token: null })).status).toBe(200);
    expect((await w.browse("/attention", { host: elsewhere })).status).toBe(503);
    expect((await w.browse("/internal/hold", { host: elsewhere, body: { kind: "ask.send", args: {} } })).status).toBe(
      404,
    );
    expect(w.hostSaw).toEqual([]);
    // On the routed hostname an /internal path is the host's, never the state's.
    const inside = await w.browse("/internal/hold", {
      body: { kind: "ask.send", args: { text: "x" }, who: "attacker" },
    });
    expect(inside.headers.get("x-pm-console")).toBe("1");
    expect(w.hostSaw).toEqual(["POST /internal/hold"]);
    expect((w.stored.get("held") as unknown[] | undefined) ?? []).toEqual([]);
  });
});

describe("notifications and phones, from the edge", () => {
  test("a subscribed device is told of a reply, and once that the host has gone quiet and that it is back", async () => {
    const w = await setup();
    const phone = await browser();
    (w.data.asks as unknown[]).push({
      issue: 5,
      title: "Q",
      status: "working",
      replies: [],
      updated: "2026-11-22T11:00:00Z",
    });
    await w.hostReads();
    await w.link.now.once();
    // Subscribing is the edge's own: the host is not asked.
    w.hostSaw.length = 0;
    expect((await w.act("push.on", { subscription: phone.subscription, label: "Pocket" })).status).toBe(200);
    expect(w.hostSaw).toEqual([]);
    expect(((await (await w.browse("/console/api/push")).json()) as { devices: number }).devices).toBe(1);
    // The host is told to keep the sections notifications need fresh.
    w.now.t += 1_000;
    expect((await w.link.now.once())?.notify).toBe(true);
    // The first pass only takes stock; then the fleet answers.
    w.data.asks = [
      {
        issue: 5,
        title: "Q",
        status: "answered",
        replies: [{ you: false, at: "2026-11-22T12:00:30Z", text: "Here is why." }],
        updated: "2026-11-22T12:00:30Z",
      },
    ];
    w.now.t += 11_000;
    await w.hostReads();
    await w.link.now.once();
    expect(w.pushes).toHaveLength(1);
    expect(JSON.parse(await phone.open(w.pushes[0].body))).toMatchObject({ url: "/console/#ask" });
    // The push service is given the project's page as the contact, and nobody's address.
    const claims = JSON.parse(
      Buffer.from((/^vapid t=[^.]+\.([^.]+)\./.exec(w.pushes[0].authorization) ?? [])[1] ?? "", "base64url").toString(),
    );
    expect(claims.sub).toBe("https://github.com/developing-today/code");
    expect(w.pushes[0].authorization).not.toContain("mailto");

    // The host stops reporting. The alarm set at its last report finds it quiet, once.
    expect(w.alarm.at).toBe(w.now.t + SILENT_MS + 1000);
    w.now.t = w.alarm.at;
    await w.edge.now.alarm();
    await w.edge.now.alarm();
    expect(w.pushes).toHaveLength(2);
    expect(JSON.parse(await phone.open(w.pushes[1].body)).title).toBe("The laptop has gone quiet");
    // An alarm that fires while the host is reporting says nothing.
    w.now.t += 1_000;
    await w.link.now.once();
    expect(JSON.parse(await phone.open(w.pushes[2].body)).title).toBe("The laptop is back");
    w.now.t += 60_000;
    await w.edge.now.alarm();
    expect(w.pushes).toHaveLength(3);
  });

  test("a failed quiet-state push is retried by a later alarm", async () => {
    const w = await setup();
    const phone = await browser();
    await w.hostReads();
    await w.link.now.once();
    await w.act("push.on", { subscription: phone.subscription, label: "Pocket" });

    w.now.t = w.alarm.at;
    w.pushFailures.remaining = 1;
    await w.edge.now.alarm();
    expect(w.stored.get("silent")).toBeUndefined();
    expect(w.pushes).toHaveLength(1);

    await w.edge.now.alarm();
    expect(w.stored.get("silent")).toBe(true);
    expect(w.pushes).toHaveLength(2);
    await w.edge.now.alarm();
    expect(w.pushes).toHaveLength(2);
  });

  test("a failed recovery push remains pending and is retried on the next host report", async () => {
    const w = await setup();
    const phone = await browser();
    await w.hostReads();
    await w.link.now.once();
    await w.act("push.on", { subscription: phone.subscription, label: "Pocket" });

    w.now.t = w.alarm.at;
    await w.edge.now.alarm();
    expect(w.stored.get("silent")).toBe(true);

    w.pushFailures.remaining = 1;
    w.now.t += 1_000;
    await w.link.now.once();
    expect(w.stored.get("silent")).toBe(true);
    expect(w.pushes).toHaveLength(2);

    w.now.t += 1_000;
    await w.link.now.once();
    expect(w.stored.get("silent")).toBeUndefined();
    expect(w.pushes).toHaveLength(3);
  });

  test("a phone is linked at the edge and reads its summary there, host or no host", async () => {
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    const verifier = "v".repeat(43);
    const challenge = Buffer.from(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier))).toString(
      "base64url",
    );
    w.hostSaw.length = 0;
    w.net.hostUp = false;
    expect((await w.act("device.approve", { challenge, name: "Pocket" })).status).toBe(200);
    const linked = await route(
      new Request(`https://${HOST}/console/device/link`, { method: "POST", body: JSON.stringify({ verifier }) }),
      w.io,
    );
    const { token, id } = (await linked.json()) as { token: string; id: string };
    // Phone identity crosses only the signed sync as a hash; the host can
    // validate the token locally after an edge/host endpoint switch.
    await w.link.now.once();
    expect((await w.hostDeps.phones!.devices.check(token))?.name).toBe("Pocket");
    const read = await route(
      new Request(`https://${HOST}/console/device/summary`, { headers: { authorization: `Bearer ${token}` } }),
      w.io,
    );
    expect(read.status).toBe(200);
    expect(await read.json()).toMatchObject({
      now: { count: 1, top: [{ title: "First task", step: "" }] },
      next: { count: 2 },
    });
    expect(await (await w.browse("/console/api/devices")).json()).toEqual([
      expect.objectContaining({ name: "Pocket" }),
    ]);
    expect(JSON.stringify(await w.hostDeps.phones!.devices.links())).not.toContain(token);
    // Linked phones live at the edge and survive its restart; the token is not in its storage.
    expect(JSON.stringify([...w.stored.entries()])).not.toContain(token);
    w.edge.now = w.makeEdge();
    const after = await route(
      new Request(`https://${HOST}/console/device/summary`, { headers: { authorization: `Bearer ${token}` } }),
      w.io,
    );
    expect(after.status).toBe(200);
    expect(w.hostSaw).toEqual([]);
    await w.act("device.remove", { id });
    await w.link.now.once();
    expect(await w.hostDeps.phones!.devices.check(token)).toBeNull();
  });

  test("a host-linked phone survives the edge cutover and an owner removal reaches both stores", async () => {
    const w = await setup();
    await w.hostReads();
    const verifier = "h".repeat(43);
    const challenge = Buffer.from(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier))).toString(
      "base64url",
    );
    await w.hostDeps.phones!.devices.approve({ challenge, name: "Host phone" });
    const linked = await w.hostDeps.phones!.devices.link(verifier);
    expect(linked).not.toBeNull();
    await w.link.now.once();
    expect((await w.edge.now.phones.devices.check(linked!.token))?.name).toBe("Host phone");
    expect(JSON.stringify(await w.edge.now.phones.devices.links())).not.toContain(linked!.token);

    await w.hostDeps.phones!.devices.remove({ id: linked!.id });
    await w.link.now.once();
    expect(await w.edge.now.phones.devices.check(linked!.token)).toBeNull();
    expect(await w.hostDeps.phones!.devices.check(linked!.token)).toBeNull();
  });
});
