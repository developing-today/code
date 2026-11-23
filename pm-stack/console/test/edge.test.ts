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
import { type EdgeConfig, edgeConfig as parseEdgeConfig, keyId, newKeyPair, sign } from "../src/edge-proto.ts";
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

async function setup(options: { direct?: boolean } = {}) {
  const now = { t: START };
  const clock = () => now.t;
  const access = await signer("k1");
  const cfg = readConfig({ HOME: "/h", PM_CONSOLE_OWNER_EMAILS: OWNER });
  cfg.issuers = [ISSUER];
  const pushes: { url: string; body: Uint8Array; authorization: string }[] = [];
  const pushFailures = { remaining: 0 };
  /** The release GitHub holds at the app's fixed address: its
   * description and its file, either of which may be missing; whether
   * GitHub can be reached at all; and every file asked for, in order. */
  const github: { version: unknown; apk: Uint8Array | null; down: boolean; asked: string[] } = {
    version: null,
    apk: null,
    down: false,
    asked: [],
  };
  const releaseBase = `https://github.com/${cfg.repo}/releases/download/pm-console-android/`;
  const outward = async (url: string | URL | Request, init?: { body?: unknown; headers?: Record<string, string> }) => {
    const u = String(url);
    if (u === ISSUER.jwksUri) return { ok: true, status: 200, json: async () => ({ keys: [access.jwk] }) };
    if (u.startsWith(releaseBase)) {
      const file = u.slice(releaseBase.length);
      github.asked.push(file);
      if (github.down) throw new Error("github: connection reset");
      if (file === "version.json" && github.version !== null) return new Response(JSON.stringify(github.version));
      if (file === "pm-console.apk" && github.apk) return new Response(github.apk);
      return new Response("Not Found", { status: 404 });
    }
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

  // ---- the outside world, as the edge reaches it directly --------------
  // GitHub and the queue store, standing in for the real ones: the edge's
  // direct reads and writes go through these, and only these. The data
  // here is newer than the host's last report, as it would be after the
  // laptop has been away for a while.
  const githubCalls: { method: string; url: string; body?: string }[] = [];
  const queueCalls: { sql: string; args: (string | number)[] }[] = [];
  const ghIssues: Record<
    number,
    {
      number: number;
      title: string;
      body: string;
      html_url: string;
      state: string;
      created_at: string;
      updated_at: string;
      comments: number;
    }
  > = {
    201: {
      number: 201,
      title: "Owner: Do the docs",
      body: "Do the docs\n\n---\nAn instruction from the owner.",
      html_url: "https://github.com/developing-today/code/issues/201",
      state: "open",
      created_at: "2026-11-22T11:00:00Z",
      updated_at: "2026-11-22T12:31:00Z",
      comments: 1,
    },
    202: {
      number: 202,
      title: "Owner: Check the queue",
      body: "Check the queue",
      html_url: "https://github.com/developing-today/code/issues/202",
      state: "open",
      created_at: "2026-11-22T11:30:00Z",
      updated_at: "2026-11-22T12:32:00Z",
      comments: 0,
    },
  };
  const ghComments: Record<number, { body: string; created_at: string; user: { type: string } }[]> = {
    201: [{ body: "Done: the docs are updated.", created_at: "2026-11-22T12:31:00Z", user: { type: "User" } }],
  };
  const ghLabels: Record<number, string[]> = { 201: ["owner/prompt"], 202: ["owner/prompt"], 301: ["now"] };
  const ghLabelNames = new Set(["owner/prompt"]);
  const ghPrs = [
    {
      number: 55,
      title: "Fresh PR",
      draft: false,
      html_url: "https://github.com/developing-today/code/pull/55",
      created_at: "2026-11-22T12:30:00Z",
      user: { login: "worker" },
      head: { ref: "queue/55" },
    },
  ];
  const ghSearch = [
    {
      number: 301,
      title: "Ranked fresh",
      html_url: "https://github.com/developing-today/code/issues/301",
      updated_at: "2026-11-22T12:31:00Z",
      comments: 2,
      labels: [{ name: "now" }],
    },
  ];
  let ghNextIssue = 500;
  const queueTasks: {
    id: number;
    title: string;
    priority: number;
    status: string;
    attempts: number;
    claimed: string | null;
    lease: string | null;
    created: string;
    worker: string | null;
    lane: string | null;
    model: string | null;
    kind: string | null;
    issue: number | null;
    pr: number | null;
    step: string | null;
    goal: string | null;
  }[] = [
    {
      id: 21047,
      title: "Owner prompt developing-today/code#201: Do the docs",
      priority: 10,
      status: "claimed",
      attempts: 1,
      claimed: "2026-11-22T12:00:00",
      lease: "2026-11-22T12:45:00",
      created: "2026-11-22T11:00:00",
      worker: "xaltha",
      lane: null,
      model: "muse",
      kind: "triage",
      issue: 201,
      pr: null,
      step: "reading the issue",
      goal: "Carry out the owner's instruction",
    },
    {
      id: 21048,
      title: "Next task",
      priority: 20,
      status: "queued",
      attempts: 0,
      claimed: null,
      lease: null,
      created: "2026-11-22T11:30:00",
      worker: null,
      lane: null,
      model: null,
      kind: "triage",
      issue: 301,
      pr: null,
      step: null,
      goal: "Triage",
    },
    {
      id: 21049,
      title: "Blocked task",
      priority: 30,
      status: "blocked",
      attempts: 2,
      claimed: null,
      lease: null,
      created: "2026-11-22T10:00:00",
      worker: null,
      lane: null,
      model: null,
      kind: "triage",
      issue: null,
      pr: null,
      step: null,
      goal: null,
    },
  ];
  const TASK_COLS = [
    "id",
    "title",
    "priority",
    "status",
    "attempts",
    "claimed",
    "lease",
    "created",
    "worker",
    "lane",
    "model",
    "kind",
    "issue",
    "pr",
    "step",
    "goal",
  ];
  const taskRow = (t: (typeof queueTasks)[number]) =>
    TASK_COLS.map((c) => ({ value: (t as Record<string, string | number | null>)[c] ?? null }));
  const queueAnswer = (sql: string, args: (string | number)[]) => {
    const cols = (names: string[]) => names.map((name) => ({ name }));
    const rows = (rs: { value: string | number | null }[][]) => rs;
    if (sql.includes("count(*) AS n") && sql.includes("GROUP BY status")) {
      const counts = new Map<string, number>();
      for (const t of queueTasks) counts.set(t.status, (counts.get(t.status) ?? 0) + 1);
      return {
        cols: cols(["status", "n"]),
        rows: rows([...counts.entries()].map(([status, n]) => [{ value: status }, { value: n }])),
      };
    }
    // The single-row reads first: they also carry a status clause.
    if (sql.startsWith("SELECT priority FROM tasks")) {
      const found = queueTasks.find((t) => t.id === Number(args[0]) && t.status === "queued");
      return { cols: cols(["priority"]), rows: rows(found ? [[{ value: found.priority }]] : []) };
    }
    if (sql.startsWith("UPDATE tasks SET priority")) {
      const found = queueTasks.find((t) => t.id === Number(args[1]) && t.status === "queued");
      if (found) found.priority = Number(args[0]);
      return { cols: cols([]), rows: [], affected_row_count: found ? 1 : 0 };
    }
    if (sql.includes("status = 'claimed'")) {
      return { cols: cols(TASK_COLS), rows: rows(queueTasks.filter((t) => t.status === "claimed").map(taskRow)) };
    }
    if (sql.includes("status = 'queued'")) {
      return { cols: cols(TASK_COLS), rows: rows(queueTasks.filter((t) => t.status === "queued").map(taskRow)) };
    }
    if (sql.includes("status = 'blocked'")) {
      return {
        cols: cols(["why", "n"]),
        rows: rows([
          [{ value: "waiting on the owner" }, { value: queueTasks.filter((t) => t.status === "blocked").length }],
        ]),
      };
    }
    if (sql.includes("dedupe_key IN")) {
      const keys = args.slice(1) as string[];
      return {
        cols: cols(["dedupe_key", "status", "worker"]),
        rows: rows(
          queueTasks
            .filter((t) => t.issue !== null && keys.includes(`repo-health:v1:triage:${cfg.repo}:issue:${t.issue}`))
            .map((t) => [
              { value: `repo-health:v1:triage:${cfg.repo}:issue:${t.issue}` },
              { value: t.status },
              { value: t.worker },
            ]),
        ),
      };
    }
    throw new Error(`unexpected queue SQL: ${sql}`);
  };
  const githubAnswer = (method: string, url: string, body?: string) => {
    const u = new URL(url);
    const path = u.pathname.replace(/^\/repos\/developing-today\/code\//, "");
    const ok = (data: unknown, status = 200) => ({ ok: true, status, json: async () => data });
    const fail = (status: number, data: unknown) => ({ ok: false, status, json: async () => data });
    if (method === "GET" && path === "pulls") return ok(ghPrs);
    if (method === "GET" && u.pathname === "/search/issues")
      return ok({ items: ghSearch, total_count: ghSearch.length });
    if (method === "GET" && path === "issues") {
      return ok(
        Object.values(ghIssues)
          .sort((a, b) => b.updated_at.localeCompare(a.updated_at))
          .map((i) => ({ ...i, pull_request: undefined })),
      );
    }
    const issueMatch = /^issues\/(\d+)$/.exec(path);
    if (method === "GET" && issueMatch) {
      const n = Number(issueMatch[1]);
      return ok({
        number: n,
        state: ghIssues[n]?.state ?? "open",
        labels: (ghLabels[n] ?? []).map((name) => ({ name })),
      });
    }
    const commentsMatch = /^issues\/(\d+)\/comments$/.exec(path);
    if (method === "GET" && commentsMatch) return ok(ghComments[Number(commentsMatch[1])] ?? []);
    if (method === "POST" && commentsMatch) {
      const n = Number(commentsMatch[1]);
      const parsed = JSON.parse(body ?? "{}") as { body?: string };
      (ghComments[n] ??= []).push({
        body: parsed.body ?? "",
        created_at: new Date(now.t).toISOString(),
        user: { type: "User" },
      });
      if (ghIssues[n]) {
        ghIssues[n].comments = (ghComments[n] ?? []).length;
        ghIssues[n].updated_at = new Date(now.t).toISOString();
      }
      return ok({ html_url: `https://github.com/developing-today/code/issues/${n}#issuecomment-1` }, 201);
    }
    if (method === "POST" && path === "issues") {
      const parsed = JSON.parse(body ?? "{}") as { title?: string; body?: string; labels?: string[] };
      const n = ghNextIssue++;
      ghIssues[n] = {
        number: n,
        title: parsed.title ?? "",
        body: parsed.body ?? "",
        html_url: `https://github.com/developing-today/code/issues/${n}`,
        state: "open",
        created_at: new Date(now.t).toISOString(),
        updated_at: new Date(now.t).toISOString(),
        comments: 0,
      };
      ghLabels[n] = parsed.labels ?? [];
      return ok({ number: n, title: parsed.title, html_url: ghIssues[n].html_url }, 201);
    }
    if (method === "PATCH" && issueMatch) {
      const n = Number(issueMatch[1]);
      const parsed = JSON.parse(body ?? "{}") as { state?: string };
      if (parsed.state) ghIssues[n].state = parsed.state;
      ghIssues[n].updated_at = new Date(now.t).toISOString();
      return ok({ number: n, state: ghIssues[n].state });
    }
    const labelsMatch = /^issues\/(\d+)\/labels$/.exec(path);
    if (method === "POST" && labelsMatch) {
      const n = Number(labelsMatch[1]);
      const parsed = JSON.parse(body ?? "{}") as { labels?: string[] };
      for (const label of parsed.labels ?? []) if (!ghLabels[n].includes(label)) ghLabels[n].push(label);
      return ok(
        ghLabels[n].map((name) => ({ name })),
        201,
      );
    }
    const labelMatch = /^issues\/(\d+)\/labels\/(.+)$/.exec(path);
    if (method === "DELETE" && labelMatch) {
      const n = Number(labelMatch[1]);
      ghLabels[n] = ghLabels[n].filter((l) => l !== decodeURIComponent(labelMatch[2]));
      return ok({});
    }
    if (method === "POST" && path === "labels") {
      const parsed = JSON.parse(body ?? "{}") as { name?: string };
      if (parsed.name && ghLabelNames.has(parsed.name)) {
        return fail(422, {
          message: "Validation Failed",
          errors: [{ resource: "Label", code: "already_exists", field: "name" }],
        });
      }
      if (parsed.name) ghLabelNames.add(parsed.name);
      return ok({ name: parsed.name }, 201);
    }
    throw new Error(`unexpected GitHub call ${method} ${url}`);
  };
  /** The direct source, made to fail by a test that turns it on. */
  const directDown = { on: false };
  const directFetch = (async (
    url: string,
    init: { method?: string; headers?: Record<string, string>; body?: string },
  ) => {
    // Made to fail by a test: the direct source is down, not the data.
    if (directDown.on) throw new Error("the direct source is down");
    const u = String(url);
    const method = init?.method ?? "GET";
    if (u.startsWith("https://api.github.com/")) {
      githubCalls.push({ method, url: u, body: init?.body });
      return githubAnswer(method, u, init?.body);
    }
    if (u === `${cfg.queueDbUrl}/v2/pipeline`) {
      const parsed = JSON.parse(init?.body ?? "{}") as {
        requests?: { stmt?: { sql: string; args: { value: string }[] } }[];
      };
      const results: unknown[] = [];
      for (const req of parsed.requests ?? []) {
        if (!req.stmt) {
          results.push({ type: "ok" });
          continue;
        }
        const args = (req.stmt.args ?? []).map((a) => a.value);
        queueCalls.push({ sql: req.stmt.sql, args });
        results.push({ type: "ok", response: { result: queueAnswer(req.stmt.sql, args) } });
      }
      return { ok: true, status: 200, json: async () => ({ results }) };
    }
    throw new Error(`unexpected direct fetch ${u}`);
  }) as never;

  // ---- the edge ----------------------------------------------------------
  const stored = new Map<string, unknown>();
  /** Every key written to the edge's storage, in order. */
  const puts: string[] = [];
  /** Keys the edge's storage refuses to write, as a store in trouble would. */
  const storeFails = new Set<string>();
  const kv: KV = {
    get: async (key) => structuredClone(stored.get(key)),
    put: async (key, value) => {
      if (storeFails.has(key)) throw new Error(`storage: could not write ${key}`);
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
      direct: options.direct
        ? { enabled: true, githubToken: "gh-test", queueToken: "queue-test", fetchImpl: directFetch }
        : undefined,
      setAlarm: async (at) => {
        alarm.at = at;
      },
    });
  const edge = { now: makeEdge() };

  // ---- the host ----------------------------------------------------------
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
    // The laptop's half of a prompt the edge filed: enqueue its task.
    "prompt.enqueue": {
      run: async (a) => {
        ran.push({ kind: "prompt.enqueue", args: a });
        return { issue: a.issue, queued: true };
      },
      touch: ["board"],
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
  const net: {
    hostUp: boolean;
    edgeUp: boolean;
    loseAnswer: boolean;
    giveUpEarly: boolean;
    /** A report of more than this many bytes does not get through: it
     * is cut off on the way ("lost", a slow connection running out of
     * time) or turned away by something in front of the edge ("refused"). */
    large: { over: number; fate: "lost" | "refused" } | null;
  } = { hostUp: true, edgeUp: true, loseAnswer: false, giveUpEarly: false, large: null };
  /** The size of each report the host sent, whether or not it arrived. */
  const reportSizes: number[] = [];
  /** The signed Android build on the host, if any, and how often it was read to be sent. */
  const build: { bytes: Uint8Array | null; at: number; reads: number; corrupt: boolean; unreadable: boolean } = {
    bytes: null,
    at: 0,
    reads: 0,
    corrupt: false,
    unreadable: false,
  };
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
    queueDbUrl: cfg.queueDbUrl,
    queueTokenName: cfg.queueTokenName,
    promptLabel: cfg.promptLabel,
    needsRepos: cfg.needsRepos,
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
    // The platform's file store tags each file by its content; the icon
    // here has none, as a store that sent no tag would not.
    asset: async (path) =>
      files.has(path)
        ? new Response(files.get(path), path.startsWith("/icons/") ? {} : { headers: { ETag: `"tag-of-${path}"` } })
        : new Response("", { status: 404 }),
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
      version: "0123abcd4567",
      apk: async () => {
        if (!build.bytes) return null;
        const bytes = build.bytes;
        const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
        return {
          sha256: [...digest].map((b) => b.toString(16).padStart(2, "0")).join(""),
          size: bytes.byteLength,
          at: build.at,
          read: async () => {
            build.reads++;
            if (build.unreadable) throw new Error("EACCES: permission denied");
            return build.corrupt ? bytes.map((b) => b ^ 1) : bytes;
          },
        };
      },
      url: `https://${HOST}`,
      keys,
      done,
      docs: hostDocs,
      phones: hostDeps.phones!.devices,
      config: async () => edgeConfig,
      run: heldRunner(hostDeps),
      fetchImpl: (async (url: string, init: RequestInit) => {
        if (!net.edgeUp) throw new Error("no route to the edge");
        const size = (init.body as Uint8Array).byteLength;
        reportSizes.push(size);
        if (net.large && size > net.large.over) {
          if (net.large.fate === "lost") throw new Error("The operation timed out.");
          return new Response("<html>bad gateway</html>", { status: 502 });
        }
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
    storeFails,
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
    build,
    github,
    reportSizes,
    failing,
    gate,
    stillRunning,
    table,
    access,
    edgeConfig,
    directDown,
    githubCalls,
    queueCalls,
    ghIssues,
    ghComments,
    ghLabels,
    ghPrs,
    ghSearch,
    queueTasks,
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
      directEnabled: false,
      githubTokenConfigured: false,
      queueTokenConfigured: false,
      directReady: false,
      selfCheck: null,
      hostSeen: null,
      hostVersion: null,
      androidBuild: null,
      androidRelease: null,
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

  test("the edge says which commit the host is running, as the host last reported it", async () => {
    const w = await setup();
    const health = async () =>
      (await (await w.browse("/console/health", { token: null })).json()) as Record<string, unknown>;
    expect((await health()).hostVersion).toBeNull();
    await w.hostReads();
    await w.link.now.once();
    expect((await health()).hostVersion).toBe("0123abcd4567");
    // It is remembered across the edge's own restarts.
    w.edge.now = w.makeEdge();
    expect((await health()).hostVersion).toBe("0123abcd4567");
    // Anything that is not a commit's name is not repeated to the public.
    const kept = JSON.parse(w.keys.text() as string) as { privateJwk: JsonWebKey };
    w.now.t += 1_000;
    const body = new TextEncoder().encode(
      JSON.stringify({
        v: 1,
        at: w.now.t,
        version: "<b>not a commit</b>",
        config: w.edgeConfig,
        sections: [],
        have: {},
        acks: [],
      }),
    );
    const res = await route(
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
    expect(res.status).toBe(200);
    expect((await health()).hostVersion).toBeNull();
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
    // A visit does not download the app again. Each file says which
    // version of itself it is; a browser that names that version is told
    // "still that" and sent nothing, and any other is sent the file.
    const script = await w.browse("/console/app.js");
    const tag = script.headers.get("etag");
    expect(tag).toBe('"tag-of-/console/app.js"');
    expect(script.headers.get("cache-control")).toBe("no-cache"); // it always asks, so a deploy shows at once
    for (const named of [tag, `W/${tag}`, `"something-else", ${tag}`, "*"]) {
      const still = await w.browse("/console/app.js", { headers: { "if-none-match": named as string } });
      expect([named, still.status, await still.text()]).toEqual([named, 304, ""]);
      expect(still.headers.get("etag")).toBe(tag);
      expect(still.headers.get("content-security-policy")).toContain("default-src 'self'");
    }
    for (const named of ['"an-older-version"', '"tag-of-/console/app.css"', "", "tag-of-/console/app.js"]) {
      const sent = await w.browse("/console/app.js", { headers: { "if-none-match": named } });
      expect([named, sent.status, await sent.text()]).toEqual([named, 200, "// app"]);
    }
    const stillPage = await w.browse("/console/", { headers: { "if-none-match": '"tag-of-/console/index.html"' } });
    expect(stillPage.status).toBe(304);
    // A file the store gave no tag for is always sent: nothing is made up.
    const icon = await w.browse("/icons/icon-192.png", { headers: { "if-none-match": "*" } });
    expect([icon.status, icon.headers.get("etag"), await icon.text()]).toEqual([200, null, "PNG"]);
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

describe("the Android build", () => {
  const apk = (seed: number, size = 700_000) => Uint8Array.from({ length: size }, (_, i) => (i * 31 + seed) % 251);
  const report = async (w: Awaited<ReturnType<typeof setup>>) => {
    w.now.t += 1_000;
    return w.link.now.once();
  };
  const download = async (w: Awaited<ReturnType<typeof setup>>) => {
    const res = await w.browse("/console/pm-console.apk");
    return { res, bytes: new Uint8Array(await res.arrayBuffer()) };
  };
  /** The questions as the edge holds them, put back together. */
  const questionsAtEdge = (w: Awaited<ReturnType<typeof setup>>) => {
    const pieces = (w.stored.get("m:questions") as { chunks: number } | undefined)?.chunks ?? 0;
    let text = "";
    for (let i = 0; i < pieces; i++) text += String(w.stored.get(`m:questions:${i}`));
    return text === "" ? null : (JSON.parse(text) as Record<string, any>);
  };

  test("a copy is kept at the edge, sent once, and served when the laptop is away", async () => {
    const w = await setup();
    await w.hostReads();
    // The host has no build: nothing is asked for, and the app is told there is none.
    expect((await report(w))?.want).not.toContain("apk");
    w.build.bytes = apk(1);
    w.build.at = w.now.t;
    // It has one now. The first report only describes it; the edge asks; the next carries it.
    // Being asked for the build does not hurry the host the way a missing section does.
    w.now.t += 1_000;
    expect(await w.link.now.turn()).toBeGreaterThan(500);
    expect(w.link.now.last?.want).toContain("apk");
    expect(w.build.reads).toBe(0);
    expect((await report(w))?.want).not.toContain("apk");
    expect(w.build.reads).toBe(1);
    // After that it is not sent again, however many reports follow.
    await report(w);
    await report(w);
    expect(w.build.reads).toBe(1);
    // The copy is the laptop's build, so it is served from here without asking the laptop.
    w.hostSaw.length = 0;
    let got = await download(w);
    expect(got.res.status).toBe(200);
    expect(got.res.headers.get("content-type")).toBe("application/vnd.android.package-archive");
    expect(got.res.headers.get("content-disposition")).toBe('attachment; filename="pm-console.apk"');
    expect(got.bytes).toEqual(apk(1));
    expect(w.hostSaw).toEqual([]);
    // The edge restarts and the laptop goes away: still served, byte for byte.
    w.edge.now = w.makeEdge();
    w.net.hostUp = false;
    w.now.t += 10 * 60_000;
    got = await download(w);
    expect(got.res.status).toBe(200);
    expect(got.bytes).toEqual(apk(1));
    const me = (await (await w.browse("/console/api/me")).json()) as Record<string, any>;
    expect(me.androidApp).toBe(true);
    expect(me.androidBuild).toEqual({ at: w.build.at, current: true });
    // Health says the same of the copy, to anyone: it is how its arrival is checked from outside.
    const health = (await (await w.browse("/console/health", { token: null })).json()) as Record<string, unknown>;
    expect(health.androidBuild).toEqual({ at: w.build.at, current: true });
    // Only the owner gets it.
    expect((await w.browse("/console/pm-console.apk", { token: null })).status).not.toBe(200);
  });

  test("a newer build on the laptop is fetched, and until then the app says the copy is older", async () => {
    const w = await setup();
    await w.hostReads();
    w.build.bytes = apk(1);
    w.build.at = w.now.t;
    await report(w);
    await report(w);
    const first = w.build.at;
    // A new build is signed on the laptop. The edge has heard of it but does not have it yet.
    w.build.bytes = apk(2, 900_000);
    w.build.at = w.now.t + 500;
    expect((await report(w))?.want).toContain("apk");
    let me = (await (await w.browse("/console/api/me")).json()) as Record<string, any>;
    expect(me.androidBuild).toEqual({ at: first, current: false });
    // The laptop is there, so the download is the laptop's: the newest.
    w.hostSaw.length = 0;
    await w.browse("/console/pm-console.apk");
    expect(w.hostSaw).toEqual(["GET /console/pm-console.apk"]);
    // The next report brings it, and then the copy is current again.
    await report(w);
    me = (await (await w.browse("/console/api/me")).json()) as Record<string, any>;
    expect(me.androidBuild).toEqual({ at: w.build.at, current: true });
    w.net.hostUp = false;
    w.now.t += 10 * 60_000;
    expect((await download(w)).bytes).toEqual(apk(2, 900_000));
  });

  test("after the edge restarts, a laptop that is there is still asked for its newer build", async () => {
    const w = await setup();
    await w.hostReads();
    w.build.bytes = apk(1);
    w.build.at = w.now.t;
    await report(w);
    await report(w);
    // A newer build is announced, and the edge restarts before it has a copy.
    w.build.bytes = apk(2);
    w.build.at = w.now.t + 500;
    await report(w);
    w.edge.now = w.makeEdge();
    w.hostSaw.length = 0;
    const got = await download(w);
    // The laptop reported seconds ago: the download is its newest build, not the older copy.
    expect(w.hostSaw).toEqual(["GET /console/pm-console.apk"]);
    expect(got.bytes).not.toEqual(apk(1));
  });

  test("a build and large sections that will not fit one report go in two, and both arrive", async () => {
    const w = await setup();
    await w.hostReads();
    await report(w);
    // Nearly the largest build that is carried, and three megabytes of questions changed at once.
    w.build.bytes = apk(5, 4_900_000);
    w.build.at = w.now.t;
    await report(w); // described; the edge asks for it
    w.data.questions = { sets: [{ id: "big", text: "q".repeat(3_000_000) }] };
    w.now.t += 11_000;
    await w.hostReads();
    // The build goes alone, and the host does not wait to send what it displaced.
    w.now.t += 1_000;
    expect(await w.link.now.turn()).toBe(500);
    expect(w.link.now.last?.ok).toBe(true);
    expect((w.stored.get("apk") as { size: number }).size).toBe(4_900_000);
    expect(questionsAtEdge(w)).toEqual({ open: [] });
    // The questions arrive whole in the report after it, and then there is no hurry.
    w.now.t += 1_000;
    expect(await w.link.now.turn()).toBeGreaterThan(500);
    expect(w.link.now.last?.ok).toBe(true);
    expect((w.stored.get("m:questions") as { chunks: number }).chunks).toBeGreaterThan(1);
    expect(questionsAtEdge(w)?.sets[0].text.length).toBe(3_000_000);
    // No report was as large as the edge refuses (8 MiB), and the build's kept the room it is given.
    expect(Math.max(...w.reportSizes)).toBeLessThanOrEqual(7.5 * 1024 * 1024);
  });

  test("room in a report is counted in bytes, so text outside ASCII cannot push it past the limit", async () => {
    const w = await setup();
    await w.hostReads();
    await report(w);
    w.build.bytes = apk(5, 4_900_000);
    w.build.at = w.now.t;
    await report(w); // described; the edge asks for it
    // 700,000 characters, each three bytes as sent. Counted as characters
    // they fit beside the build; as bytes the two are past what the edge takes.
    const text = "\u65e5".repeat(700_000);
    expect(new TextEncoder().encode(text).byteLength).toBe(2_100_000);
    w.data.questions = { sets: [{ id: "wide", text }] };
    w.now.t += 11_000;
    await w.hostReads();
    const answers = [await report(w), await report(w)];
    expect(answers.every((answer) => answer?.ok === true)).toBe(true);
    expect(Math.max(...w.reportSizes)).toBeLessThanOrEqual(7.5 * 1024 * 1024);
    expect((w.stored.get("apk") as { size: number }).size).toBe(4_900_000);
    expect(questionsAtEdge(w)?.sets[0].text).toBe(text);
  });

  test("a build that does not get through does not hold up the reports after it", async () => {
    for (const fate of ["lost", "refused"] as const) {
      const w = await setup();
      await w.hostReads();
      await report(w);
      w.build.bytes = apk(1);
      w.build.at = w.now.t;
      expect((await report(w))?.want).toContain("apk"); // described; the edge asks for it
      // A connection too slow for a report that large, or something in
      // front of the edge that will not pass one.
      w.net.large = { over: 500_000, fate };
      w.data.questions = { sets: [{ id: "new", text: "what next?" }] };
      w.now.t += 11_000;
      await w.hostReads();
      // The report with the build in it fails. The host does not wait to
      // send the next, and that one goes without the build...
      w.now.t += 1_000;
      expect(await w.link.now.turn()).toBe(500);
      expect(w.link.now.last).toBeNull();
      expect(w.reportSizes.at(-1)).toBeGreaterThan(500_000);
      w.now.t += 1_000;
      expect(await w.link.now.turn()).toBeGreaterThan(500);
      expect(w.link.now.last?.ok).toBe(true);
      expect(w.reportSizes.at(-1)).toBeLessThan(500_000);
      // ...and brings what had changed.
      expect(questionsAtEdge(w)).toEqual({ sets: [{ id: "new", text: "what next?" }] });
      // An edge that cannot be reached at all is another matter: a report
      // without the build that fails is not hurried after.
      w.net.edgeUp = false;
      w.now.t += 1_000;
      expect(await w.link.now.turn()).toBeGreaterThan(500);
      expect(w.link.now.last).toBeNull();
      w.net.edgeUp = true;
      // The edge still asks for the build. For ten minutes it is not sent again.
      for (let i = 0; i < 20; i++) expect((await report(w))?.want).toContain("apk");
      expect(w.build.reads).toBe(1);
      // Then it is tried once more. If that fails too, again only that one report is lost.
      w.now.t += 10 * 60_000;
      expect(await report(w)).toBeNull();
      expect(w.build.reads).toBe(2);
      expect((await report(w))?.ok).toBe(true);
      expect(w.build.reads).toBe(2);
      // On a connection that can carry it, it arrives at the next try.
      w.net.large = null;
      w.now.t += 10 * 60_000;
      expect((await report(w))?.want).not.toContain("apk");
      expect(w.build.reads).toBe(3);
      expect((w.stored.get("apk") as { size: number }).size).toBe(700_000);
    }
  });

  /** A release as android/release.sh describes it, for these bytes. */
  const described = async (bytes: Uint8Array, name: string, code: number) => {
    const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
    return {
      name,
      code,
      commit: "a".repeat(40),
      sha256: [...digest].map((b) => b.toString(16).padStart(2, "0")).join(""),
      size: bytes.byteLength,
      built: "2026-11-22T11:00:00Z",
      certificate: "8B:82",
    };
  };

  test("a build GitHub has released is handed out first, and needs nothing of the laptop", async () => {
    const w = await setup();
    await w.hostReads();
    // The laptop has an older build of its own, and the edge a copy of it.
    w.build.bytes = apk(1);
    w.build.at = w.now.t;
    await report(w);
    await report(w);
    // No release is published: the download is the copy, as before, and the app is told there is none.
    expect((await download(w)).bytes).toEqual(apk(1));
    let said = (await (await w.browse("/console/api/android")).json()) as Record<string, any>;
    expect(said).toEqual({ release: null, copy: { at: w.build.at, current: true } });
    expect(((await (await w.browse("/console/health", { token: null })).json()) as any).androidRelease).toBeNull();
    // GitHub Actions releases a build. The edge asks again within ten minutes and hands that out.
    w.github.apk = apk(9, 450_000);
    w.github.version = await described(w.github.apk, "2026.11.23-a3a547e", 21755935);
    w.now.t += 10 * 60_000;
    w.hostSaw.length = 0;
    const got = await download(w);
    expect(got.res.status).toBe(200);
    expect(got.bytes).toEqual(apk(9, 450_000));
    expect(got.res.headers.get("content-type")).toBe("application/vnd.android.package-archive");
    expect(got.res.headers.get("content-disposition")).toBe('attachment; filename="pm-console.apk"');
    expect(got.res.headers.get("x-build")).toBe("2026.11.23-a3a547e");
    expect(w.hostSaw).toEqual([]);
    said = (await (await w.browse("/console/api/android")).json()) as Record<string, any>;
    expect(said.release).toEqual({ name: "2026.11.23-a3a547e", code: 21755935, built: "2026-11-22T11:00:00.000Z" });
    // Health says which release this side knows of, to anyone (the
    // release is public): it is how its arrival here is checked from
    // outside. Asking does not send the edge to GitHub each time.
    const askedBefore = w.github.asked.length;
    for (let i = 0; i < 3; i++) {
      const health = (await (await w.browse("/console/health", { token: null })).json()) as Record<string, unknown>;
      expect(health.androidRelease).toEqual(said.release);
    }
    expect(w.github.asked.length).toBe(askedBefore);
    // The laptop gone, the edge restarted, and no copy of the laptop's build at all: still handed out.
    w.net.hostUp = false;
    w.stored.delete("apk");
    w.edge.now = w.makeEdge();
    w.now.t += 60 * 60_000;
    expect((await download(w)).bytes).toEqual(apk(9, 450_000));
    // Only the owner gets it.
    expect((await w.browse("/console/pm-console.apk", { token: null })).status).not.toBe(200);
    expect((await w.browse("/console/api/android", { token: null })).status).not.toBe(200);
  });

  test("a released build that is not the one described is not handed out", async () => {
    const w = await setup();
    await w.hostReads();
    w.build.bytes = apk(1);
    w.build.at = w.now.t;
    await report(w);
    await report(w);
    // A new build is half put up: the file is the new one, the description still the old one's.
    w.github.apk = apk(9, 450_000);
    w.github.version = await described(apk(8, 440_000), "2026.11.22-0000000", 21000000);
    w.hostSaw.length = 0;
    let got = await download(w);
    // What arrived does not match: the laptop's build is handed out instead.
    expect(got.bytes).toEqual(apk(1));
    // And the description is read again at the next request, not trusted for ten minutes.
    w.github.version = await described(w.github.apk, "2026.11.23-a3a547e", 21755935);
    got = await download(w);
    expect(got.bytes).toEqual(apk(9, 450_000));
    // The file gone missing, cut short, or the right length and the
    // wrong bytes, is the same: not handed out.
    w.github.apk = apk(9, 449_999);
    expect((await download(w)).bytes).toEqual(apk(1));
    w.github.apk = apk(7, 450_000);
    expect((await download(w)).bytes).toEqual(apk(1));
    w.github.apk = null;
    expect((await download(w)).bytes).toEqual(apk(1));
    // A description that is not one (no size, a path for a name) is no release.
    w.now.t += 10 * 60_000;
    w.github.apk = apk(9, 450_000);
    w.github.version = { ...(await described(w.github.apk, "../../etc", 1)), size: undefined };
    expect((await download(w)).bytes).toEqual(apk(1));
    expect(((await (await w.browse("/console/api/android")).json()) as Record<string, any>).release).toBeNull();
  });

  test("GitHub is asked now and then, not on every request, and what it last said stands while it is out of reach", async () => {
    const w = await setup();
    await w.hostReads();
    await report(w);
    const ask = async () => ((await (await w.browse("/console/api/android")).json()) as Record<string, any>).release;
    // Nothing published. Asked once, however many times the app asks.
    for (let i = 0; i < 5; i++) expect(await ask()).toBeNull();
    expect(w.github.asked).toEqual(["version.json"]);
    // A release goes up. It is seen at the next asking, ten minutes on.
    w.github.apk = apk(9, 450_000);
    w.github.version = await described(w.github.apk, "2026.11.23-a3a547e", 21755935);
    w.now.t += 9 * 60_000;
    expect(await ask()).toBeNull();
    w.now.t += 60_000;
    expect((await ask())?.name).toBe("2026.11.23-a3a547e");
    expect(w.github.asked).toEqual(["version.json", "version.json"]);
    // GitHub goes out of reach. The release is still known, and GitHub is tried again each minute, not each request.
    w.github.down = true;
    w.now.t += 10 * 60_000;
    for (let i = 0; i < 5; i++) expect((await ask())?.name).toBe("2026.11.23-a3a547e");
    expect(w.github.asked.length).toBe(3);
    w.now.t += 60_000;
    expect((await ask())?.name).toBe("2026.11.23-a3a547e");
    expect(w.github.asked.length).toBe(4);
    // With GitHub out of reach the file cannot be fetched either: the download falls back to the laptop.
    w.build.bytes = apk(1);
    w.build.at = w.now.t;
    w.hostSaw.length = 0;
    await w.browse("/console/pm-console.apk");
    expect(w.hostSaw).toEqual(["GET /console/pm-console.apk"]);
    // Back in reach, and the release withdrawn: that is believed too.
    w.github.down = false;
    w.github.version = null;
    w.now.t += 60_000;
    expect(await ask()).toBeNull();
  });

  test("a copy whose replacement stops half-way is not handed out as either build", async () => {
    const w = await setup();
    await w.hostReads();
    w.build.bytes = apk(1, 1_500_000);
    w.build.at = w.now.t;
    await report(w);
    await report(w);
    expect((w.stored.get("apk") as { chunks: number }).chunks).toBe(3);
    // A newer build arrives, and the edge's storage fails on its second piece.
    w.build.bytes = apk(2, 1_500_000);
    w.build.at = w.now.t + 500;
    await report(w); // described; the edge asks for it
    w.storeFails.add("apk:1");
    expect((await report(w))?.ok).toBe(true); // the report itself is taken
    // The first piece is now the new build's and the rest are the old one's.
    // There is no record over them: the edge has no copy, and says so.
    expect(w.stored.has("apk")).toBe(false);
    const health = (await (await w.browse("/console/health", { token: null })).json()) as Record<string, unknown>;
    expect(health.androidBuild).toBeNull();
    // The laptop goes away. Nothing is handed out from those pieces.
    w.net.hostUp = false;
    w.now.t += 2 * 60_000;
    expect((await download(w)).res.status).not.toBe(200);
    // It comes back with a smaller build. That one is kept whole, and the
    // pieces of the larger ones before it are gone, record or no record.
    w.net.hostUp = true;
    w.storeFails.clear();
    w.build.bytes = apk(3, 400_000);
    w.build.at = w.now.t + 500;
    expect((await report(w))?.want).toContain("apk");
    expect((await report(w))?.want).not.toContain("apk");
    expect([...w.stored.keys()].filter((k) => /^apk:\d+$/.test(k))).toEqual(["apk:0"]);
    w.net.hostUp = false;
    w.now.t += 2 * 60_000;
    const got = await download(w);
    expect(got.res.status).toBe(200);
    expect(got.bytes).toEqual(apk(3, 400_000));
  });

  test("a build that cannot be read is left out, and the report goes anyway", async () => {
    const w = await setup();
    await w.hostReads();
    await report(w);
    w.build.bytes = apk(4);
    w.build.at = w.now.t;
    expect((await report(w))?.want).toContain("apk");
    w.build.unreadable = true;
    w.data.questions = { sets: [{ id: "new", text: "what next?" }] };
    w.now.t += 11_000;
    await w.hostReads();
    // The report is not given up for the build's sake.
    expect((await report(w))?.ok).toBe(true);
    expect(questionsAtEdge(w)).toEqual({ sets: [{ id: "new", text: "what next?" }] });
    expect(w.build.reads).toBe(1);
    // Nor is the file tried again with every report.
    for (let i = 0; i < 20; i++) expect((await report(w))?.ok).toBe(true);
    expect(w.build.reads).toBe(1);
    // Ten minutes on it can be read, and is sent.
    w.build.unreadable = false;
    w.now.t += 10 * 60_000;
    expect((await report(w))?.want).not.toContain("apk");
    expect(w.build.reads).toBe(2);
    expect((w.stored.get("apk") as { size: number }).size).toBe(700_000);
  });

  test("a build that is not the one described is not kept, and one too large is not sent", async () => {
    const w = await setup();
    await w.hostReads();
    w.build.bytes = apk(3);
    w.build.at = w.now.t;
    w.build.corrupt = true;
    await report(w);
    await report(w);
    // What arrived did not match its fingerprint: nothing is kept.
    expect(w.stored.has("apk")).toBe(false);
    expect(w.build.reads).toBe(1);
    // It is not asked for again with every report: a build that fails
    // every time would be half a megabyte in each of them.
    for (let i = 0; i < 20; i++) expect((await report(w))?.want).not.toContain("apk");
    expect(w.build.reads).toBe(1);
    // Ten minutes on it is asked for once more, and this time it is right.
    w.build.corrupt = false;
    w.now.t += 10 * 60_000;
    expect((await report(w))?.want).toContain("apk");
    await report(w);
    expect(w.build.reads).toBe(2);
    expect((w.stored.get("apk") as { size: number }).size).toBe(700_000);
    // A build too large to carry in a report is never read to be sent.
    const big = await setup();
    await big.hostReads();
    big.build.bytes = new Uint8Array(5 * 1024 * 1024 + 1);
    big.build.at = big.now.t;
    await report(big);
    await report(big);
    expect(big.build.reads).toBe(0);
    expect(big.stored.has("apk")).toBe(false);
  });
});

describe("the host's report is the host's", () => {
  const report = async (
    w: Awaited<ReturnType<typeof setup>>,
    change: (msg: Record<string, unknown>) => void,
    sig?: (body: Uint8Array) => Promise<string>,
    kid?: string,
    from?: string,
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
          ...(from ? { "cf-connecting-ip": from } : {}),
        },
      }),
      w.io,
    );
  };

  test("a report under a key the edge has not seen is read to its end before the key is asked for", async () => {
    // In the Worker runtime the report is sent to the state a second
    // time, once the key is known, from a second copy of the same
    // upload, and a first copy left unread holds the second back until
    // the laptop gives up waiting. That cannot be seen under bun, which
    // keeps the whole upload in memory; what can be seen is whether the
    // first copy was read. (scripts/edge-smoke.ts shows the real thing,
    // and runs before every deploy.)
    const w = await setup();
    const pair = await newKeyPair();
    const body = new TextEncoder().encode(JSON.stringify({ v: 1, at: w.now.t, sections: [], have: {}, acks: [] }));
    const first = new Request(`https://${HOST}/console/edge/sync`, {
      method: "POST",
      body,
      headers: { "x-pm-key": await keyId(pair.publicJwk), "x-pm-signature": await sign(pair.privateJwk, body) },
    });
    const answer = await w.edge.now.fetch(first);
    expect(answer.headers.get("x-pm-next")).toBe("key");
    expect(first.bodyUsed).toBe(true);
    // A second stranger within the same minute is refused, and that one is not read at all.
    const second = new Request(`https://${HOST}/console/edge/sync`, {
      method: "POST",
      body,
      headers: { "x-pm-key": await keyId(pair.publicJwk), "x-pm-signature": await sign(pair.privateJwk, body) },
    });
    expect((await w.edge.now.fetch(second)).status).toBe(401);
    expect(second.bodyUsed).toBe(false);
  });

  test("an address whose reports keep failing is refused unread, and nobody else is", async () => {
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    const before = JSON.stringify([...w.stored.entries()]);
    const stranger = await newKeyPair();
    const forged = (body: Uint8Array) => sign(stranger.privateJwk, body);
    const kept = JSON.parse(w.keys.text() as string) as { privateJwk: JsonWebKey };
    const hosts = (body: Uint8Array) => sign(kept.privateJwk, body);
    const flooder = "198.51.100.7";
    // Twenty forged reports in a minute are each read, checked and refused.
    for (let i = 0; i < 20; i++) {
      w.now.t += 100;
      expect((await report(w, () => {}, forged, undefined, flooder)).status).toBe(401);
    }
    // From then on that address is refused before anything is read or
    // checked: even a report the host itself signed gets the same answer.
    w.now.t += 100;
    expect((await report(w, () => {}, forged, undefined, flooder)).status).toBe(429);
    w.now.t += 100;
    const unread = await report(w, () => {}, hosts, undefined, flooder);
    expect(unread.status).toBe(429);
    expect((await unread.json()).error).toMatch(/too many reports from this address/);
    // "Before anything is read" is meant literally: a body that fails
    // the moment it is touched is never touched.
    const untouchable = (headers: Record<string, string>) =>
      route(
        new Request(`https://${HOST}/console/edge/sync`, {
          method: "POST",
          body: new ReadableStream({
            pull(controller) {
              controller.error(new Error("this body was read"));
            },
          }),
          headers: { "cf-ray": "1", ...headers },
          duplex: "half",
        } as RequestInit),
        w.io,
      );
    const wellFormed = { "x-pm-key": await keyId(await w.link.now.publicKey()), "x-pm-signature": "A".repeat(86) };
    expect((await untouchable({ ...wellFormed, "cf-connecting-ip": flooder })).status).toBe(429);
    // So is one that is not shaped like a report, from anywhere.
    expect((await untouchable({ "x-pm-key": wellFormed["x-pm-key"], "cf-connecting-ip": "192.0.2.61" })).status).toBe(
      401,
    );
    expect(
      (await untouchable({ ...wellFormed, "x-pm-signature": "short", "cf-connecting-ip": "192.0.2.61" })).status,
    ).toBe(401);
    // Another address is not held to that, and neither is the host's own report.
    w.now.t += 100;
    expect((await report(w, () => {}, forged, undefined, "203.0.113.9")).status).toBe(401);
    w.now.t += 1_000;
    expect((await w.link.now.once())?.ok).toBe(true);
    // What is not shaped like a report counts too, and is never read.
    for (let i = 0; i < 20; i++)
      expect((await report(w, () => {}, undefined, undefined, "192.0.2.44")).status).toBe(401);
    expect((await report(w, () => {}, hosts, undefined, "192.0.2.44")).status).toBe(429);
    // A minute later the address may try again; one the host signed is taken.
    w.now.t += 61_000;
    expect((await report(w, () => {}, hosts, undefined, flooder)).status).toBe(200);
    // And a report the host signed that is merely wrong is not held against its address.
    for (let i = 0; i < 25; i++) {
      const stale = await report(
        w,
        (m) => Object.assign(m, { at: w.now.t - 10 * 60_000 }),
        hosts,
        undefined,
        "192.0.2.80",
      );
      expect(stale.status).toBe(409);
    }
    // None of the refused ones changed anything that is kept.
    w.now.t += 1_000;
    expect((await report(w, () => {}, forged, undefined, "192.0.2.90")).status).toBe(401);
    expect(typeof before).toBe("string");
  });

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
  test("a unit that stays failed is reported from the edge, though every report reads the same", async () => {
    const w = await setup();
    const phone = await browser();
    const units = [
      { unit: "pm-git-sync.service", active: "inactive", description: "keep the checkout on main" },
      { unit: "pm-git-sync.timer", active: "active" },
    ];
    w.data.services = units;
    const round = async (ms: number) => {
      w.now.t += ms;
      await w.hostReads();
      return w.link.now.once();
    };
    await round(0);
    await w.act("push.on", { subscription: phone.subscription, label: "Pocket" });
    await round(11_000);
    const before = w.pushes.length;
    // It starts failing, and from then on the host's reports all say the same thing.
    units[0].active = "failed";
    await round(11_000);
    for (let i = 0; i < 9; i++) await round(30_000);
    expect(w.pushes.length).toBe(before); // four and a half minutes: not yet
    await round(30_000);
    await round(30_000);
    expect(w.pushes.length).toBe(before + 1);
    expect(JSON.parse(await phone.open(w.pushes[before].body))).toMatchObject({
      title: "A unit keeps failing",
      url: "/console/#more/services",
    });
    // Said once, and its mending is said once.
    for (let i = 0; i < 4; i++) await round(30_000);
    expect(w.pushes.length).toBe(before + 1);
    units[0].active = "inactive";
    await round(30_000);
    expect(w.pushes.length).toBe(before + 2);
    expect(JSON.parse(await phone.open(w.pushes[before + 1].body)).title).toBe("A unit is working again");
  });

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

describe("direct reads and writes while the host is away", () => {
  /** The host has reported, then gone quiet for a minute. */
  async function quiet() {
    const w = await setup({ direct: true });
    await w.hostReads();
    await w.link.now.once();
    w.net.hostUp = false;
    w.net.edgeUp = false;
    w.now.t += 60_000;
    w.hostSaw.length = 0;
    return w;
  }
  /** Only the laptop's doc pokes, never an ask for it to do anything. */
  const onlyPokes = (w: Awaited<ReturnType<typeof setup>>) =>
    w.hostSaw.every((s) => s.startsWith("GET /console/api/doc/"));

  test("the board, pull requests, ranked issues and prompts come from GitHub and the queue store after a minute of silence", async () => {
    const w = await setup({ direct: true });
    await w.hostReads();
    await w.link.now.once();
    // Before the minute is up the host's copy is served, and nothing is read directly.
    const stale = await w.core();
    expect(stale.doc.board).toEqual(w.data.board);
    expect(stale.doc.asks).toEqual([]);
    expect(w.githubCalls).toEqual([]);
    expect(w.queueCalls).toEqual([]);
    // A minute of silence: the next read is fresh.
    w.net.hostUp = false;
    w.net.edgeUp = false;
    w.now.t += 60_000;
    w.hostSaw.length = 0;
    const { doc } = await w.core();
    expect(doc.board.counts).toEqual({ claimed: 1, queued: 1, blocked: 1 });
    expect(doc.board.now.map((t: { id: number }) => t.id)).toEqual([21047]);
    expect(doc.board.next.map((t: { id: number }) => t.id)).toEqual([21048]);
    expect(doc.asks.map((a: { issue: number }) => a.issue)).toEqual([202, 201]);
    expect(doc.asks[1].replies).toEqual([
      { you: false, at: "2026-11-22T12:31:00", text: "Done: the docs are updated." },
    ]);
    const ops = await w.browse("/console/api/doc/ops");
    const opsDoc = (await ops.json()) as {
      prs: unknown[];
      priorities: { issues: { number: number; rank: string }[] };
    };
    expect(opsDoc.prs).toEqual([
      {
        number: 55,
        title: "Fresh PR",
        draft: false,
        url: "https://github.com/developing-today/code/pull/55",
        created: "2026-11-22T12:30",
        author: "worker",
        branch: "queue/55",
      },
    ]);
    expect(opsDoc.priorities.issues.map((i) => i.number)).toEqual([301]);
    expect(opsDoc.priorities.issues[0].rank).toBe("now");
    // The reads went to GitHub and the queue store; the laptop was never asked for anything.
    expect(onlyPokes(w)).toBe(true);
    expect(w.githubCalls.map((c) => c.url)).toContain(
      "https://api.github.com/repos/developing-today/code/pulls?state=open&per_page=60",
    );
    expect(w.queueCalls.some((c) => c.sql.includes("GROUP BY status"))).toBe(true);
  });

  test("a reply to a prompt is posted once at the edge and not repeated when the laptop returns", async () => {
    const w = await quiet();
    const res = await w.act("ask.reply", { issue: 201, text: "Thanks — noted." });
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ ok: true, result: { issue: 201, queued: true } });
    const comments = w.githubCalls.filter((c) => c.method === "POST" && c.url.includes("/issues/201/comments"));
    expect(comments).toHaveLength(1);
    expect(JSON.parse(comments[0].body ?? "{}").body).toContain("Thanks — noted.");
    // The reply shows up in the next read at the edge.
    const { doc } = await w.core();
    const asked = doc.asks.find((a: { issue: number }) => a.issue === 201) as {
      replies: { you: boolean; text: string }[];
    };
    expect(asked.replies[asked.replies.length - 1]).toEqual({
      you: true,
      at: "2026-11-22T12:01:00",
      text: "Thanks — noted.",
    });
    // Only the queue task waits for the laptop.
    expect((w.stored.get("held") as { kind: string }[]).map((h) => h.kind)).toEqual(["prompt.enqueue"]);
    // The laptop returns: it runs the held enqueue once, and the reply is not posted again.
    w.net.hostUp = true;
    w.net.edgeUp = true;
    w.now.t += 60_000;
    const first = await w.link.now.once();
    expect(first?.commands.map((c) => c.kind)).toEqual(["prompt.enqueue"]);
    expect(w.ran.filter((r) => r.kind === "prompt.enqueue")).toHaveLength(1);
    w.now.t += 1_000;
    const second = await w.link.now.once();
    expect(second?.commands).toEqual([]);
    expect(w.ran.filter((r) => r.kind === "prompt.enqueue")).toHaveLength(1);
    expect(w.githubCalls.filter((c) => c.method === "POST" && c.url.includes("/issues/201/comments"))).toHaveLength(1);
  });

  test("a new prompt is filed at the edge and only its queue task waits for the laptop", async () => {
    const w = await quiet();
    const res = await w.act("ask.send", { text: "File this from the edge" });
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({
      ok: true,
      result: { issue: 500, url: "https://github.com/developing-today/code/issues/500", queued: true },
    });
    expect(w.githubCalls.filter((c) => c.method === "POST" && c.url.endsWith("/issues"))).toHaveLength(1);
    const held = w.stored.get("held") as { kind: string; args: Record<string, unknown> }[];
    expect(held).toHaveLength(1);
    expect(held[0].kind).toBe("prompt.enqueue");
    expect(held[0].args.issue).toBe(500);
    // The laptop returns and enqueues it once; the issue is not filed again.
    w.net.hostUp = true;
    w.net.edgeUp = true;
    w.now.t += 60_000;
    await w.link.now.once();
    expect(w.ran.filter((r) => r.kind === "prompt.enqueue")).toHaveLength(1);
    expect(w.githubCalls.filter((c) => c.method === "POST" && c.url.endsWith("/issues"))).toHaveLength(1);
  });

  test("closing a prompt, ranking an issue, commenting on a need and moving a task run at the edge", async () => {
    const w = await quiet();
    const closed = await w.act("ask.close", { issue: 202 });
    expect(closed.status).toBe(200);
    expect(await closed.json()).toEqual({ ok: true, result: { issue: 202, closed: true } });
    expect(w.ghIssues[202].state).toBe("closed");
    const ranked = await w.act("issue.rank", { issue: 301, rank: "next" });
    expect(ranked.status).toBe(200);
    expect(await ranked.json()).toEqual({ ok: true, result: { issue: 301, rank: "next" } });
    expect(w.ghLabels[301]).toEqual(["next"]);
    const need = await w.act("need.comment", { ref: "developing-today/code#301", text: "Noted." });
    expect(need.status).toBe(200);
    expect((w.ghComments[301] ?? []).some((c) => c.body.startsWith("Noted."))).toBe(true);
    const moved = await w.act("task.move", { task: 21048, to: "first" });
    expect(moved.status).toBe(200);
    expect(await moved.json()).toEqual({ ok: true, result: { task: 21048, priority: 10 } });
    expect(w.queueTasks.find((t) => t.id === 21048)?.priority).toBe(10);
    // Nothing was held for the laptop: all four were done here.
    expect((w.stored.get("held") as unknown[] | undefined) ?? []).toEqual([]);
    expect(onlyPokes(w)).toBe(true);
    // And the board the edge serves already shows the move.
    const { doc } = await w.core();
    expect(doc.board.next.find((t: { id: number }) => t.id === 21048)?.priority).toBe(10);
  });

  test("health reports the direct capability and the self-check", async () => {
    const w = await setup({ direct: true });
    await w.hostReads();
    await w.link.now.once();
    const health = (await (await w.browse("/console/health", { token: null })).json()) as Record<string, unknown>;
    expect(health.directEnabled).toBe(true);
    expect(health.githubTokenConfigured).toBe(true);
    expect(health.queueTokenConfigured).toBe(true);
    expect(health.directReady).toBe(true);
    const selfCheck = health.selfCheck as { at: number; kinds: Record<string, { ok: boolean }> };
    expect(selfCheck.at).toBe(START);
    expect(Object.keys(selfCheck.kinds).sort()).toEqual(["asks", "board", "priorities", "prs"]);
    expect(Object.values(selfCheck.kinds).every((k) => k.ok)).toBe(true);
    // A second health read reuses the stored self-check: no more direct reads.
    const calls = w.githubCalls.length + w.queueCalls.length;
    await w.browse("/console/health", { token: null });
    expect(w.githubCalls.length + w.queueCalls.length).toBe(calls);
  });

  test("without the direct switch, or before the laptop is quiet, actions go to the host as before", async () => {
    // No direct switch: the host is away, so the action is held, not run here.
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    w.net.hostUp = false;
    w.net.edgeUp = false;
    const held = await w.act("issue.comment", { issue: 14, text: "late" });
    expect(await held.json()).toEqual({ ok: true, held: true, result: null });
    expect(w.githubCalls).toEqual([]);
    expect(w.queueCalls).toEqual([]);
    // Direct switch on but the laptop still reporting: the action goes to the host.
    // A second setup signs with a fresh key under the same kid, so the
    // cached key set from the first setup must go first.
    clearJwksCache();
    const d = await setup({ direct: true });
    await d.hostReads();
    await d.link.now.once();
    const res = await d.act("issue.comment", { issue: 14, text: "now" });
    expect(res.status).toBe(200);
    expect(d.ran).toEqual([{ kind: "issue.comment", args: { issue: 14, text: "now" } }]);
    expect(d.githubCalls).toEqual([]);
    expect(d.queueCalls).toEqual([]);
  });

  test("a config stored before direct mode still serves the offline app, without acting itself", async () => {
    const w = await setup({ direct: true });
    await w.hostReads();
    await w.link.now.once();
    // What an older Worker stored: the same settings, without the fields
    // direct mode added. That shape still validates, with those empty.
    const legacy = { ...w.edgeConfig } as Record<string, unknown>;
    delete legacy.queueDbUrl;
    delete legacy.queueTokenName;
    delete legacy.promptLabel;
    delete legacy.needsRepos;
    expect(parseEdgeConfig(legacy)).toMatchObject({
      repo: w.edgeConfig.repo,
      queueDbUrl: "",
      queueTokenName: "",
      promptLabel: "",
      needsRepos: [],
    });
    w.stored.set("cfg", legacy);
    w.edge.now = w.makeEdge();
    // The laptop is out of reach: the app is still served from the
    // mirror, not relayed to a host that cannot answer.
    w.net.hostUp = false;
    w.net.edgeUp = false;
    const { res, doc } = await w.core();
    expect(res.status).toBe(200);
    expect(doc.board).toEqual(w.data.board);
    expect(doc.asks).toEqual([]);
    expect(w.githubCalls).toEqual([]);
    expect(w.queueCalls).toEqual([]);
    const health = (await (await w.browse("/console/health", { token: null })).json()) as Record<string, unknown>;
    expect(health.configured).toBe(true);
    expect(health.directReady).toBe(false);
  });

  test("a notification about a directly read reply is sent, and the fresh read is not overwritten", async () => {
    const w = await setup({ direct: true });
    const phone = await browser();
    await w.hostReads();
    await w.link.now.once();
    // Subscribing primes the notifier on the host's last copy: no replies yet.
    expect((await w.act("push.on", { subscription: phone.subscription, label: "Pocket" })).status).toBe(200);
    expect(w.pushes).toHaveLength(0);
    // The laptop goes quiet; the next read comes from GitHub, where the
    // fleet has since answered prompt #201.
    w.net.hostUp = false;
    w.net.edgeUp = false;
    w.now.t += 60_000;
    const { doc } = await w.core();
    const asked = doc.asks.find((a: { issue: number }) => a.issue === 201) as {
      replies: { you: boolean; at: string; text: string }[];
    };
    expect(asked.replies).toEqual([{ you: false, at: "2026-11-22T12:31:00", text: "Done: the docs are updated." }]);
    // The notifier saw that fresh reply, not the mirror's empty asks.
    expect(w.pushes).toHaveLength(1);
    expect(JSON.parse(await phone.open(w.pushes[0].body)).title).toBe("The fleet replied on #201");
  });

  test("after a restart, a failed first direct read still shows the mirrored copy", async () => {
    const w = await setup({ direct: true });
    await w.hostReads();
    await w.link.now.once();
    // The edge restarts: nothing is in memory, but the mirror is durable.
    w.edge.now = w.makeEdge();
    w.directDown.on = true;
    w.net.hostUp = false;
    w.net.edgeUp = false;
    w.now.t += 60_000;
    const { doc } = await w.core();
    expect(doc.board).toEqual(w.data.board);
    expect(doc.asks).toEqual([]);
    expect(doc.health.board).toEqual({ ok: false, error: expect.stringContaining("the direct source is down") });
    expect(doc.health.asks.ok).toBe(false);
  });
});
