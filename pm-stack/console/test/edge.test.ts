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
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { beforeEach, describe, expect, test } from "bun:test";
import { queueWrites } from "../edge/src/direct.ts";
import { Edge, type KV, SILENT_MS } from "../edge/src/edge.ts";
import { forgetPoke, type Io, route } from "../edge/src/entry.ts";
import type { Action } from "../src/actions.ts";
import { clearJwksCache, identify } from "../src/auth.ts";
import { readConfig } from "../src/config.ts";
import { Devices, Limiter } from "../src/devices.ts";
import { Docs } from "../src/docs.ts";
import {
  type EdgeConfig,
  edgeConfig as parseEdgeConfig,
  keyId,
  type MachinesAnswer,
  machineName,
  machineReport,
  newKeyPair,
  sign,
} from "../src/edge-proto.ts";
import { OpError } from "../src/errors.ts";
import { Ops, triagePacket } from "../src/ops.ts";
import { type Deps, handle, heldRunner, marked } from "../src/server.ts";
import { memoryStore } from "../src/store.ts";
import { EdgeLink } from "../src/sync.ts";
import { asThePlatformFetch, browser, signer } from "./support.ts";

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
/** The order the platform's Workers are shown in, by what they belong to. */
const ORDER = ["platform", "fleet", "data", "id", "lab"];
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
  const github: { version: unknown; apk: Uint8Array<ArrayBuffer> | null; down: boolean; asked: string[] } = {
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
  /** Events the queue store was asked to record, in order. */
  const queueEvents: { worker: string; task: number; kind: string; detail: Record<string, unknown>; created: string }[] =
    [];
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
  // An issue that is not a prompt: ranked, and there to be noted on.
  ghIssues[301] = {
    number: 301,
    title: "Ranked fresh",
    body: "",
    html_url: "https://github.com/developing-today/code/issues/301",
    state: "open",
    created_at: "2026-11-22T10:00:00Z",
    updated_at: "2026-11-22T12:31:00Z",
    comments: 0,
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
      head: { ref: "queue/55", sha: "5555555555555555555555555555555555555555" },
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
  /** The queue store refusing this side's token, and how often it did. */
  const queueRefuses = { on: false };
  const queueRefused = { n: 0 };
  /** Calls GitHub refuses this side's token for (403), and calls it
   * fails on its own account (500). */
  const ghRefuses: ((method: string, url: string) => boolean)[] = [];
  /** Calls GitHub turns away for being one too many just now: the same
   * 403, with other words. */
  const ghLimits: ((method: string, url: string) => boolean)[] = [];
  /** The queue store taking this side's token for reading only. */
  const queueReadOnly = { on: false };
  /** Calls GitHub gives no answer to at all, and calls it is slow to
   * answer: held until the test lets them go. */
  const ghDrops: ((method: string, url: string) => boolean)[] = [];
  /** Calls GitHub answers in a way a test gives: a status and its words. */
  const ghSays: { match: (method: string, url: string) => boolean; status: number; message: string }[] = [];
  const ghHolds: { match: (method: string, url: string) => boolean; wait: Promise<void> }[] = [];
  const ghBreaks: ((method: string, url: string) => boolean)[] = [];
  /** GitHub answering a new issue with no number on it: not something
   * it does, and exactly what must not be passed on as a task. */
  const ghOdd = { noNumber: false, longAddress: false };
  /** Pull requests merged and workflows started through GitHub's API. */
  const ghMerged: { pr: number; body: unknown }[] = [];
  const ghStarted: { workflow: string; body: unknown }[] = [];
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
    /** The row as the broker's verbs read it: the packet, what it
     * did, and why it is where it is. */
    body: string;
    result: Record<string, unknown>;
    progress: Record<string, unknown>;
    last_error: string | null;
    blocked_deps: unknown[] | null;
    payload: Record<string, unknown>;
    max_attempts: number;
    required_budget_minutes: number | null;
    not_before: string | null;
    dedupe_key: string | null;
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
      body: "Do the docs",
      result: {},
      progress: { session_id: "sess-21047" },
      last_error: null,
      blocked_deps: null,
      payload: triagePacket(cfg.repo, cfg.queue, 201, "Do the docs", "now", "2026-11-22T11:00:00Z").payload,
      max_attempts: 1000,
      required_budget_minutes: null,
      not_before: null,
      dedupe_key: "repo-health:v1:triage:developing-today/code:issue:201",
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
      body: "Ranked fresh",
      result: {},
      progress: {},
      last_error: null,
      blocked_deps: null,
      payload: triagePacket(cfg.repo, cfg.queue, 301, "Ranked fresh", "now", "2026-11-22T11:30:00Z").payload,
      max_attempts: 1000,
      required_budget_minutes: null,
      not_before: null,
      dedupe_key: "repo-health:v1:triage:developing-today/code:issue:301",
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
      body: "Blocked task",
      result: {},
      progress: {},
      last_error: null,
      blocked_deps: [{ kind: "issue", repo: "developing-today/code", number: 999, state: "open" }],
      payload: {},
      max_attempts: 1000,
      required_budget_minutes: null,
      not_before: null,
      dedupe_key: null,
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
    // The one row cancel and unblock read: every column their rules
    // decide on. JSON columns travel as the store holds them: text.
    if (sql.startsWith("SELECT id, queue, title")) {
      const names = [
        "id",
        "queue",
        "title",
        "body",
        "priority",
        "payload",
        "result",
        "progress",
        "last_error",
        "blocked_deps",
        "platform",
        "lane_id",
        "claim_provider",
        "claim_model",
        "claim_variant",
        "max_attempts",
        "required_budget_minutes",
        "not_before",
        "dedupe_key",
        "status",
      ];
      const found = queueTasks.find((t) => t.id === Number(args[0]));
      if (!found) return { cols: cols(names), rows: [] };
      const values: Record<string, unknown> = {
        ...found,
        queue: cfg.queue,
        platform: null,
        lane_id: found.lane,
        claim_provider: null,
        claim_model: found.model,
        claim_variant: null,
      };
      const cell = (value: unknown) => ({
        value:
          value === null || value === undefined
            ? null
            : typeof value === "object"
              ? JSON.stringify(value)
              : (value as string | number),
      });
      return { cols: cols(names), rows: rows([names.map((n) => cell(values[n]))]) };
    }
    if (sql.startsWith("UPDATE tasks SET priority")) {
      const found = queueTasks.find((t) => t.id === Number(args[1]) && t.status === "queued");
      if (found) found.priority = Number(args[0]);
      return { cols: cols([]), rows: [], affected_row_count: found ? 1 : 0 };
    }
    if (sql.startsWith("UPDATE tasks SET status = 'cancelled'")) {
      // args: [result, updated_at, id, queue]. The guard is the
      // broker's own: a row already at an end is left as it stands.
      const found = queueTasks.find(
        (t) => t.id === Number(args[2]) && !["done", "cancelled", "failed"].includes(t.status),
      );
      if (found) {
        found.status = "cancelled";
        found.worker = null;
        found.lease = null;
        found.result = JSON.parse(String(args[0])) as Record<string, unknown>;
      }
      return { cols: cols([]), rows: [], affected_row_count: found ? 1 : 0 };
    }
    if (sql.startsWith("DELETE FROM leases")) {
      return { cols: cols([]), rows: [], affected_row_count: 0 };
    }
    if (sql.startsWith("INSERT INTO events")) {
      // args: [worker, task, kind, detail, created_at].
      queueEvents.push({
        worker: String(args[0]),
        task: Number(args[1]),
        kind: String(args[2]),
        detail: JSON.parse(String(args[3])) as Record<string, unknown>,
        created: String(args[4]),
      });
      return { cols: cols([]), rows: [], affected_row_count: 1 };
    }
    if (sql.startsWith("UPDATE tasks SET status = 'queued'")) {
      // args: [updated_at, id, queue]. Only a blocked row releases.
      const found = queueTasks.find((t) => t.id === Number(args[1]) && t.status === "blocked");
      if (found) {
        found.status = "queued";
        found.blocked_deps = null;
      }
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
    if (sql.includes("title LIKE ?")) {
      // The Queue screen's list: one state, soonest first.
      const [, status, like, limit] = args;
      const want = String(like).slice(1, -1).toLowerCase();
      const found = queueTasks
        .filter((t) => t.status === status && t.title.toLowerCase().includes(want))
        .sort((a, b) => a.priority - b.priority || a.id - b.id)
        .slice(0, Number(limit));
      return {
        cols: cols(["id", "status", "priority", "title", "dedupe_key", "updated", "why"]),
        rows: rows(
          found.map((t) => [
            { value: t.id },
            { value: t.status },
            { value: t.priority },
            { value: t.title },
            { value: t.issue ? `repo-health:v1:triage:${cfg.repo}:issue:${t.issue}` : null },
            { value: "2026-11-22T11:59" },
            { value: t.status === "blocked" ? "waiting on the owner" : "" },
          ]),
        ),
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
      // GitHub lists the issues that carry the label asked for.
      const label = u.searchParams.get("labels");
      return ok(
        Object.values(ghIssues)
          .filter((i) => label === null || (ghLabels[i.number] ?? []).includes(label))
          .sort((a, b) => b.updated_at.localeCompare(a.updated_at))
          .map((i) => ({
            ...i,
            labels: (ghLabels[i.number] ?? []).map((name) => ({ name })),
            pull_request: undefined,
          })),
      );
    }
    const prMatch = /^pulls\/(\d+)(\/merge)?$/.exec(path);
    if (prMatch) {
      const pr = ghPrs.find((p) => p.number === Number(prMatch[1]));
      if (!pr) return fail(404, { message: "Not Found" });
      if (method === "PUT" && prMatch[2]) {
        // Asked to merge a commit it no longer stands at: GitHub's own words.
        const asked = (JSON.parse(body ?? "{}") as { sha?: string }).sha;
        if (asked !== undefined && asked !== pr.head.sha) {
          return fail(409, { message: "Head branch was modified. Review and try the merge again." });
        }
        ghMerged.push({ pr: pr.number, body: JSON.parse(body ?? "{}") });
        ghPrs.splice(ghPrs.indexOf(pr), 1);
        return ok({ merged: true, sha: "5a5a5a5a5a5a" });
      }
      if (method === "GET" && !prMatch[2]) {
        return ok({
          ...pr,
          state: "open",
          merged: false,
          mergeable: true,
          mergeable_state: "clean",
          body: "What it does.",
        });
      }
    }
    const workflow = /^actions\/workflows\/([a-z-]+\.yml)\/(runs|dispatches)$/.exec(path);
    if (workflow && method === "POST" && workflow[2] === "dispatches") {
      ghStarted.push({ workflow: workflow[1], body: JSON.parse(body ?? "{}") });
      return {
        ok: true,
        status: 204,
        json: async () => {
          throw new Error("no body");
        },
      };
    }
    if (workflow && method === "GET") {
      return ok({
        workflow_runs: [
          {
            id: 4501,
            head_sha: "0123456789abcdef0123",
            status: "completed",
            conclusion: "success",
            event: "push",
            created_at: "2026-11-22T11:50:00Z",
            html_url: `https://github.com/developing-today/code/actions/runs/4501?wf=${workflow[1]}`,
          },
        ],
      });
    }
    if (method === "GET" && path === "contents/pm-stack/platform/workers.json") {
      // The repository's own list, as it stands in this checkout.
      return ok({ content: btoa(readFileSync(join(import.meta.dir, "..", "..", "platform", "workers.json"), "utf8")) });
    }
    if (method === "GET" && path === "contents/planning/roadmaps/pm-stack.md") {
      const file =
        "## Now\n\n| Title | Status | Intent | Links |\n| - | - | - | - |\n| Coherence | in-progress | One place. | #1184 |\n";
      return ok({ content: btoa(file) });
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
    if (method === "POST" && path === "issues" && ghOdd.noNumber) {
      return ok({ html_url: "https://github.com/developing-today/code/issues/?", title: "?" });
    }
    if (method === "POST" && path === "issues") {
      const parsed = JSON.parse(body ?? "{}") as { title?: string; body?: string; labels?: string[] };
      const n = ghNextIssue++;
      ghIssues[n] = {
        number: n,
        title: parsed.title ?? "",
        body: parsed.body ?? "",
        html_url: `https://github.com/developing-today/code/issues/${n}${ghOdd.longAddress ? `?${"x".repeat(5000)}` : ""}`,
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
      for (const held of ghHolds) if (held.match(method, u)) await held.wait;
      if (ghDrops.some((drops) => drops(method, u))) throw new Error("socket hang up");
      // A token that may not do this, as GitHub says it.
      if (ghRefuses.some((refuse) => refuse(method, u))) {
        return {
          ok: false,
          status: 403,
          json: async () => ({ message: "Resource not accessible by personal access token", status: "403" }),
        };
      }
      if (ghLimits.some((limits) => limits(method, u))) {
        return {
          ok: false,
          status: 403,
          json: async () => ({ message: "API rate limit exceeded for user ID 1.", status: "403" }),
        };
      }
      if (ghBreaks.some((breaks) => breaks(method, u))) {
        return { ok: false, status: 500, json: async () => ({ message: "Server Error" }) };
      }
      const says = ghSays.find((say) => say.match(method, u));
      if (says) return { ok: false, status: says.status, json: async () => ({ message: says.message }) };
      return githubAnswer(method, u, init?.body);
    }
    if (u === `${cfg.queueDbUrl}/v2/pipeline`) {
      // A token the queue store no longer takes.
      if (queueRefuses.on) {
        queueRefused.n++;
        return { ok: false, status: 401, json: async () => ({ error: "unauthorized" }) };
      }
      if (queueReadOnly.on && queueWrites(init?.body)) {
        return { ok: false, status: 403, json: async () => ({ error: "read only" }) };
      }
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
  /** Keys the edge's storage is slow to read: a read waits for its promise. */
  const storeSlow = new Map<string, Promise<void>>();
  const kv: KV = {
    get: async (key) => {
      await storeSlow.get(key);
      return structuredClone(stored.get(key));
    },
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
  /** What the edge left running behind an answer. */
  const behind: Promise<unknown>[] = [];
  /** A new object over the same storage, as after a restart; `direct`
   * says whether it has the switch and the tokens (as set up, unless
   * said: turning the switch on or off restarts the Worker). */
  const makeEdge = (direct: boolean = options.direct === true) =>
    new Edge({
      kv,
      now: clock,
      // As the Worker runtime's fetch is: not to be called as a method.
      fetchImpl: asThePlatformFetch(outward) as never,
      direct: direct
        ? {
            enabled: true,
            githubToken: "gh-test",
            queueToken: "queue-test",
            fetchImpl: asThePlatformFetch(directFetch) as typeof directFetch,
          }
        : undefined,
      setAlarm: async (at) => {
        alarm.at = at;
      },
      later: (work) => {
        behind.push(work);
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
    "triage.enqueue": {
      run: async (a) => {
        ran.push({ kind: "triage.enqueue", args: a });
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
    "task.move": {
      run: async (a) => {
        ran.push({ kind: "task.move", args: a });
        return { task: a.task, priority: 10 };
      },
      touch: ["board"],
      audit: (a) => ({ task: a.task }),
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
    // Something said to an agent: about nothing else, so nothing orders it.
    "message.send": {
      run: async (a) => {
        ran.push({ kind: "message.send", args: a });
        await gate.wait;
        return { sent: a.to };
      },
      touch: [],
      audit: (a) => ({ to: a.to }),
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
  /** Each report the host sent, as it read. */
  const reports: { sections: { name: string }[]; have: Record<string, unknown> }[] = [];
  /** The signed Android build on the host, if any, and how often it was read to be sent. */
  const build: { bytes: Uint8Array<ArrayBuffer> | null; at: number; reads: number; corrupt: boolean; unreadable: boolean } = {
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
    ["/console/outbox.js", "// what is kept"],
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
  /** What the machine says of itself in each report; null for a console
   * older than the saying, which says nothing. */
  const machineSays: { now: { name: string; actions: string[] } | null } = {
    now: { name: "nixos", actions: Object.keys(table).sort() },
  };
  const makeLink = () =>
    new EdgeLink({
      version: "0123abcd4567",
      ...(machineSays.now ? { machine: () => machineSays.now as { name: string; actions: string[] } } : {}),
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
        reports.push(JSON.parse(new TextDecoder().decode(init.body as Uint8Array)));
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
    seen: (id) => link.now.seen(id),
    known: (id) => link.now.known(id),
    take: (id) => link.now.take(id),
    release: (id) => link.now.release(id),
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
  /** With `id`, the action goes under the name a device gave it. */
  const act = (kind: string, args: Record<string, unknown> = {}, id?: string) =>
    browse("/console/api/act", { body: id === undefined ? { kind, args } : { kind, args, id } });
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
    machineSays,
    later,
    token,
    browse,
    act,
    core,
    hostReads,
    build,
    github,
    reportSizes,
    reports,
    behind,
    ghMerged,
    ghStarted,
    ghOdd,
    ghRefuses,
    ghLimits,
    ghDrops,
    ghSays,
    ghHolds,
    storeSlow,
    queueReadOnly,
    ghBreaks,
    queueRefuses,
    queueRefused,
    failing,
    gate,
    stillRunning,
    table,
    access,
    edgeConfig,
    directDown,
    githubCalls,
    queueCalls,
    queueEvents,
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
      primary: false,
      directReady: false,
      refused: [],
      selfCheck: null,
      hostSeen: null,
      hostVersion: null,
      androidBuild: null,
      androidRelease: null,
      androidReleaseTrouble: null,
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
    const kept = w.stored.get("key") as { kid: string; jwk: { kty: string; crv: string; x: string; y: string } };
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
    // The file that keeps what the owner did with no connection is one
    // of the app's files here too: the page and the worker both load it.
    const outbox = await w.browse("/console/outbox.js");
    expect([outbox.status, outbox.headers.get("content-type"), await outbox.text()]).toEqual([
      200,
      "text/javascript; charset=utf-8",
      "// what is kept",
    ]);
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
    expect(doc.edge).toEqual({ held: 0, primary: false });
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

  test("a section the laptop has not sent yet says so", async () => {
    const w = await setup();
    // The laptop reports before it has read anything.
    await w.link.now.once();
    const { doc } = await w.core();
    expect(doc.health.board).toEqual({ ok: false, error: "the laptop has not reported this yet" });
    expect(doc.health.asks).toEqual({ ok: false, error: "the laptop has not reported this yet" });
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
    expect(doc.edge).toEqual({ held: 1, primary: false });
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
    expect((await w.core()).doc.edge).toEqual({ held: 1, primary: false });

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
    expect(doc.edge).toEqual({ held: 0, primary: false });
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
    expect(doc.edge).toEqual({ held: 0, primary: false });
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
    expect(doc.edge).toEqual({ held: 0, primary: false });
    expect(doc.asks.map((a: { text: string }) => a.text)).toEqual(["only once"]);
    expect(
      doc.acts.filter((a: { kind: string }) => a.kind === "ask.send").map((a: { status: string }) => a.status),
    ).toEqual(["done"]);
  });

  test("an action a device sends twice under one name is done once by the laptop, and answered the same", async () => {
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    const id = "0a000000-0000-4000-8000-000000000001";
    const first = await w.act("ask.send", { text: "once, please" }, id);
    expect(await first.json()).toEqual({ ok: true, result: { issue: 101 } });
    // The device never heard, and sends it again.
    const second = await w.act("ask.send", { text: "once, please" }, id);
    expect(second.status).toBe(200);
    expect(await second.json()).toEqual({ ok: true, result: { issue: 101 }, again: true });
    expect(w.ran).toHaveLength(1);
    // It is on the record of what was done under the device's name for it.
    const { doc } = await w.core();
    expect(doc.acts.filter((a: { id: string }) => a.id === id).map((a: { status: string }) => a.status)).toEqual([
      "done",
    ]);
    // One that failed is not tried again under the same name either:
    // the device is told again why, with the status it had.
    const bad = "0a000000-0000-4000-8000-000000000002";
    const failed = await w.act("issue.comment", { issue: 13, text: "no such issue" }, bad);
    expect([failed.status, await failed.json()]).toEqual([404, { error: "no such issue" }]);
    const again = await w.act("issue.comment", { issue: 13, text: "no such issue" }, bad);
    expect([again.status, await again.json()]).toEqual([404, { error: "no such issue", again: true }]);
    expect(w.ran.filter((r) => r.kind === "issue.comment")).toHaveLength(1);
    // A name that is not one (not a UUID) is no name: done each time, as before.
    await w.act("issue.comment", { issue: 7, text: "a" }, "not-a-name");
    await w.act("issue.comment", { issue: 7, text: "a" }, "not-a-name");
    expect(w.ran.filter((r) => r.kind === "issue.comment")).toHaveLength(3);
  });

  test("sent again while the laptop is still doing it, the device is told to wait; sent again after the laptop restarted mid-way, that it is not known", async () => {
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    const id = "0a000000-0000-4000-8000-000000000003";
    let done = () => {};
    w.gate.wait = new Promise<void>((resolve) => {
      done = resolve;
    });
    const slow = w.act("need.comment", { ref: "developing-today/code#1", text: "slow" }, id);
    await new Promise((r) => setTimeout(r, 5));
    const meanwhile = await w.act("need.comment", { ref: "developing-today/code#1", text: "slow" }, id);
    expect([meanwhile.status, await meanwhile.json()]).toEqual([
      409,
      { error: "This is still being done.", running: true },
    ]);
    done();
    expect((await slow).status).toBe(200);
    w.gate.wait = null;
    expect(w.ran.filter((r) => r.kind === "need.comment")).toHaveLength(1);
    // Started, and the laptop's console restarted before it ended.
    const lost = "0a000000-0000-4000-8000-000000000004";
    await w.link.now.begin(lost);
    w.link.now = w.makeLink();
    const asked = await w.act("need.comment", { ref: "developing-today/code#2", text: "lost" }, lost);
    expect(asked.status).toBe(502);
    expect(((await asked.json()) as { error: string }).error).toContain("restarted while doing this");
    expect(w.ran.filter((r) => r.kind === "need.comment")).toHaveLength(1);
  });

  test("sent again while the laptop is doing the copy that was held for it, the device is told to wait, not that it was lost", async () => {
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    const id = "0a000000-0000-4000-8000-000000000006";
    const args = { to: ["lane-1"], text: "held, then slow" };
    w.net.hostUp = false;
    expect(await (await w.act("message.send", args, id)).json()).toMatchObject({ ok: true, held: true });
    // The laptop is back, takes the held action, and is slow doing it.
    w.net.hostUp = true;
    let done: () => void = () => {};
    w.gate.wait = new Promise<void>((resolve) => {
      done = resolve;
    });
    w.now.t += 1_000;
    const taking = w.link.now.once();
    await new Promise((r) => setTimeout(r, 5));
    expect(w.ran.map((r) => r.kind)).toEqual(["message.send"]);
    // The device, which never heard, sends it again meanwhile, and it
    // reaches the laptop a second way.
    const meanwhile = await w.act("message.send", args, id);
    expect([meanwhile.status, await meanwhile.json()]).toEqual([
      409,
      { error: "This is still being done.", running: true },
    ]);
    done();
    await taking;
    w.gate.wait = null;
    // Done once; and asked once more, the device hears how it came out.
    expect(w.ran).toHaveLength(1);
    const after = await w.act("message.send", args, id);
    expect(await after.json()).toMatchObject({ ok: true, again: true });
    expect(w.ran).toHaveLength(1);
  });

  test("done late in the order it was done: what may wait goes behind an earlier one about the same thing that a machine has not taken yet", async () => {
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    // With the laptop out of reach, the owner moves a task to the top.
    w.net.hostUp = false;
    const first = await w.act("task.move", { task: 21016, to: "first" });
    expect(await first.json()).toMatchObject({ ok: true, held: true });
    // The laptop is in reach again, and before it has taken that, the
    // owner moves the same task to the bottom. Sent straight on, it
    // would be done first, and the older move would undo it.
    w.net.hostUp = true;
    const second = await w.act("task.move", { task: 21016, to: "last" });
    expect(await second.json()).toEqual({ ok: true, held: true, result: null });
    expect(w.ran.filter((r) => r.kind === "task.move")).toEqual([]);
    // Another task is nothing to do with it: it is done at once.
    const other = await w.act("task.move", { task: 21017, to: "first" });
    expect(await other.json()).toEqual({ ok: true, result: { task: 21017, priority: 10 } });
    // So is something about nothing in particular.
    expect((await w.act("message.send", { to: ["lane-1"], text: "hello" })).status).toBe(200);
    // The laptop takes what waited, in the order the owner did it.
    w.now.t += 1_000;
    await w.link.now.once();
    expect(w.ran.filter((r) => r.kind === "task.move").map((r) => [r.args.task, r.args.to])).toEqual([
      [21017, "first"],
      [21016, "first"],
      [21016, "last"],
    ]);
    // Nothing waits any more, so the next one about that task goes straight on.
    w.now.t += 1_000;
    await w.link.now.once();
    const third = await w.act("task.move", { task: 21016, to: "first" });
    expect(await third.json()).toEqual({ ok: true, result: { task: 21016, priority: 10 } });
  });

  test("machines: the edge says what it does itself, what each machine that reports can do, and what waits for one, with every machine away too", async () => {
    const w = await setup();
    const machines = async () => (await (await w.browse("/console/api/machines")).json()) as MachinesAnswer;
    await w.hostReads();
    await w.link.now.once();
    const first = await machines();
    expect([first.served, first.held]).toEqual(["edge", []]);
    // It only serves and waits here. Its own it does itself (linked
    // phones, notifications, what is held); everything else is the machine's.
    expect(first.platform).toEqual({
      primary: false,
      actions: [
        "device.approve",
        "device.remove",
        "edge.retry",
        "held.drop",
        "push.class",
        "push.off",
        "push.on",
        "push.test",
      ],
      sections: [],
      refused: [],
    });
    expect(first.machines).toEqual([
      {
        name: "nixos",
        seen: w.now.t,
        quiet: false,
        version: "0123abcd4567",
        // What it said it can do: the kinds of action it has, and every section it reads.
        actions: [
          "ask.send",
          "issue.comment",
          "message.send",
          "need.comment",
          "pr.merge",
          "prompt.enqueue",
          "question.answer",
          "task.move",
          "triage.enqueue",
        ],
        sections: [...SECTION_NAMES].sort(),
      },
    ]);
    // The machine goes away. What the owner does that may wait, waits; the list says what.
    w.net.hostUp = false;
    w.net.edgeUp = false;
    const kept = "0e000000-0000-4000-8000-000000000001";
    expect(await (await w.act("task.move", { task: 21016, to: "first" }, kept)).json()).toMatchObject({ held: true });
    expect(await (await w.act("message.send", { to: ["lane-1"], text: "when you are back" })).json()).toMatchObject({
      held: true,
    });
    w.now.t += 5 * 60_000;
    const away = await machines();
    expect(away.machines.map((m) => [m.name, m.quiet, m.seen])).toEqual([["nixos", true, w.now.t - 5 * 60_000]]);
    expect(away.held.map((h) => [h.kind, h.args, h.who])).toEqual([
      ["task.move", { task: 21016, to: "first" }, OWNER],
      ["message.send", { to: ["lane-1"], text: "when you are back" }, OWNER],
    ]);
    expect(away.held[0].id).toBe(kept);
    // The object is put away and another starts in its place: it still knows the machine.
    w.edge.now = w.makeEdge();
    expect((await machines()).machines.map((m) => [m.name, m.actions.length, m.sections.length])).toEqual([
      ["nixos", 9, SECTION_NAMES.length],
    ]);
    // The owner takes one back before the machine has it.
    const dropped = await w.act("held.drop", { id: kept });
    expect(await dropped.json()).toEqual({ ok: true, result: { dropped: kept, kind: "task.move" } });
    expect((await machines()).held.map((h) => h.kind)).toEqual(["message.send"]);
    // It is on the record as not sent, under its name, so the device that sent it learns so.
    const { doc } = await w.core();
    expect(doc.acts.find((a: { id: string }) => a.id === kept)).toMatchObject({
      kind: "task.move",
      status: "failed",
      note: "Not sent: taken back before a machine took it.",
    });
    expect(doc.edge.held).toBe(1);
    // The device that sent it never heard it was kept, and offers it
    // again under its name: it is told it was taken back. It is not
    // kept a second time, and no machine is handed it.
    const offered = await w.act("task.move", { task: 21016, to: "first" }, kept);
    expect([offered.status, await offered.json()]).toEqual([
      410,
      { error: "This was taken back before a machine took it.", again: true },
    ]);
    expect((await machines()).held.map((h) => h.kind)).toEqual(["message.send"]);
    // Taken back twice, or something that was never waiting: said as that.
    const again = await w.act("held.drop", { id: kept });
    expect([again.status, await again.json()]).toEqual([
      404,
      { error: "That is no longer waiting: a machine took it, or it was taken back already." },
    ]);
    // The machine is back: it is given what still waits, and not what was taken back.
    w.net.hostUp = true;
    w.net.edgeUp = true;
    w.now.t += 1_000;
    await w.link.now.once();
    expect(w.ran.map((r) => r.kind)).toEqual(["message.send"]);
    w.now.t += 1_000;
    await w.link.now.once();
    const back = await machines();
    expect([back.machines[0].quiet, back.held]).toEqual([false, []]);
    // Offered once more with the machine back: still taken back, and still not done.
    const late = await w.act("task.move", { task: 21016, to: "first" }, kept);
    expect(late.status).toBe(410);
    expect(w.ran.map((r) => r.kind)).toEqual(["message.send"]);
  });

  test("machines: a console older than the saying is a machine that has not said what it can do; one that says nonsense, the same", async () => {
    const w = await setup();
    w.machineSays.now = null;
    w.link.now = w.makeLink();
    await w.hostReads();
    await w.link.now.once();
    const machines = async () => (await (await w.browse("/console/api/machines")).json()) as MachinesAnswer;
    expect((await machines()).machines).toEqual([
      { name: "A machine", seen: w.now.t, quiet: false, version: "0123abcd4567", actions: [], sections: [] },
    ]);
    // A name that is not one, or a list that is not a list of kinds.
    for (const says of [
      { name: "<b>nixos</b>", actions: ["task.move"] },
      { name: "nixos", actions: ["task.move", "not a kind"] },
      { name: "", actions: [] },
    ]) {
      w.machineSays.now = says;
      w.link.now = w.makeLink();
      w.now.t += 1_000;
      await w.link.now.once();
      expect([says, (await machines()).machines[0].name]).toEqual([says, "A machine"]);
    }
    // Said properly again, it is known again.
    w.machineSays.now = { name: "amd", actions: ["task.move", "task.move", "ask.send"] };
    w.link.now = w.makeLink();
    w.now.t += 1_000;
    await w.link.now.once();
    expect((await machines()).machines[0]).toMatchObject({ name: "amd", actions: ["ask.send", "task.move"] });
  });

  test("machines: a machine says every section it has a reader for from its first report, before it has read any", async () => {
    const w = await setup();
    await w.link.now.once();
    const said = (await (await w.browse("/console/api/machines")).json()) as MachinesAnswer;
    expect(said.machines.map((m) => [m.name, m.sections])).toEqual([["nixos", [...SECTION_NAMES].sort()]]);
  });

  test("held twice under one name, an action waits once", async () => {
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    w.net.hostUp = false;
    const id = "0a000000-0000-4000-8000-000000000005";
    expect(await (await w.act("ask.send", { text: "kept once" }, id)).json()).toEqual({
      ok: true,
      held: true,
      result: null,
    });
    expect(await (await w.act("ask.send", { text: "kept once" }, id)).json()).toEqual({
      ok: true,
      held: true,
      result: null,
      again: true,
    });
    expect((w.stored.get("held") as { id: string }[]).map((h) => h.id)).toEqual([id]);
    // The laptop comes back and does it once; sent yet again, the device
    // is told how it came out.
    w.net.hostUp = true;
    w.now.t += 1_000;
    await w.link.now.once();
    w.now.t += 1_000;
    await w.link.now.once();
    expect(w.ran).toHaveLength(1);
    const after = await w.act("ask.send", { text: "kept once" }, id);
    expect(await after.json()).toMatchObject({ ok: true, again: true });
    expect(w.ran).toHaveLength(1);
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
    expect((await w.core()).doc.edge).toEqual({ held: 0, primary: false });
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
    expect(doc.edge).toEqual({ held: 0, primary: false });
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
    expect(doc.edge).toEqual({ held: 0, primary: false });
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
  const described = async (bytes: Uint8Array<ArrayBuffer>, name: string, code: number) => {
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
    // Health says why it could not be asked, to anyone: from outside,
    // "none published" and "could not ask" must not look the same.
    const trouble = async () =>
      ((await (await w.browse("/console/health", { token: null })).json()) as Record<string, unknown>)
        .androidReleaseTrouble;
    expect(await trouble()).toBe("github: connection reset");
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
    // And once GitHub answers again (here with "none"), there is no trouble to report.
    expect(await trouble()).toBeNull();
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
    expect(((await unread.json()) as { error: string }).error).toMatch(/too many reports from this address/);
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

describe("the edge does the core itself", () => {
  type World = Awaited<ReturnType<typeof setup>>;
  /** The switch is on, the tokens are there, and a machine has said
   * where things are: from its first report on, this side does the core. */
  async function primary() {
    const w = await setup({ direct: true });
    await w.hostReads();
    await w.link.now.once();
    w.hostSaw.length = 0;
    return w;
  }
  /** Wait for what an answer left reading behind it: exactly what the
   * edge handed over to be given the time, and nothing more. */
  const settle = async (w: World) => {
    await Promise.all(w.behind.splice(0));
  };
  /** Only the laptop's doc pokes, never an ask for it to do anything. */
  const onlyPokes = (w: World) => w.hostSaw.every((s) => s.startsWith("GET /console/api/doc/"));
  const asked = (w: World) => w.githubCalls.length + w.queueCalls.length;
  const QUEUE_NOW = { claimed: 1, queued: 1, blocked: 1 };

  test("the board, prompts, pull requests and ranked issues come from GitHub and the queue store, laptop or no laptop", async () => {
    const w = await primary();
    // The laptop is reporting, and its copy says two are queued. What
    // is shown is the queue store's own word, read here.
    const { res, doc } = await w.core();
    expect(doc.board.counts).toEqual(QUEUE_NOW);
    expect(doc.board.now.map((t: { id: number }) => t.id)).toEqual([21047]);
    expect(doc.board.next.map((t: { id: number }) => t.id)).toEqual([21048]);
    expect(doc.asks.map((a: { issue: number }) => a.issue)).toEqual([202, 201]);
    expect(doc.asks[1].replies).toEqual([
      { you: false, at: "2026-11-22T12:31:00", text: "Done: the docs are updated." },
    ]);
    expect(doc.health.board).toEqual({ ok: true, error: null });
    expect(Number(res.headers.get("x-as-of"))).toBe(START);
    // The app is told that this side does the core.
    expect(doc.edge).toEqual({ held: 0, primary: true });
    // What only a machine has is still the machine's copy.
    expect(doc.sessions).toEqual(w.data.sessions);
    const ops = (await (await w.browse("/console/api/doc/ops")).json()) as {
      prs: unknown[];
      priorities: { issues: { number: number; rank: string }[] };
      services: unknown;
    };
    expect(ops.prs).toEqual([
      {
        number: 55,
        title: "Fresh PR",
        draft: false,
        url: "https://github.com/developing-today/code/pull/55",
        created: "2026-11-22T12:30",
        author: "worker",
        branch: "queue/55",
        head: "5555555555555555555555555555555555555555",
      },
    ]);
    expect(ops.priorities.issues.map((i) => [i.number, i.rank])).toEqual([[301, "now"]]);
    expect(ops.services).toEqual(w.data.services);
    // The reads went to GitHub and the queue store; the laptop was asked for nothing.
    expect(onlyPokes(w)).toBe(true);
    expect(w.githubCalls.map((c) => c.url)).toContain(
      "https://api.github.com/repos/developing-today/code/pulls?state=open&per_page=60",
    );
    expect(w.queueCalls.some((c) => c.sql.includes("GROUP BY status"))).toBe(true);
    // The laptop's copy of what this side reads was never kept here.
    expect((w.stored.get("m:board") as { own?: boolean }).own).toBe(true);
    expect((w.stored.get("m:sessions") as { own?: boolean }).own).toBeUndefined();
    // And with the laptop gone it is the same.
    w.net.hostUp = false;
    w.net.edgeUp = false;
    w.queueTasks[1].status = "claimed";
    w.now.t += 61_000;
    await w.core();
    await settle(w);
    const later = await w.core();
    expect(later.res.status).toBe(200);
    expect(later.doc.board.counts).toEqual({ claimed: 2, blocked: 1 });
    expect(later.doc.read.board).toBe(new Date(w.now.t).toISOString());
  });

  test("a section is read again when its own interval has passed, behind the answer and never in its way", async () => {
    const w = await primary();
    await w.core();
    const first = asked(w);
    w.now.t += 20_000;
    await w.core();
    await settle(w);
    // Inside every section's interval: nothing is asked again.
    expect(asked(w)).toBe(first);
    // The queue moves on. Thirty-five seconds after the read the board
    // (thirty) is due and the prompts (forty-five) are not.
    w.queueTasks[1].status = "claimed";
    w.now.t += 15_000;
    const github = w.githubCalls.length;
    const queue = w.queueCalls.length;
    const before = await w.core();
    // Answered at once from what it had; the read goes on behind it.
    expect(before.doc.board.counts).toEqual(QUEUE_NOW);
    await settle(w);
    const after = await w.core();
    expect(after.doc.board.counts).toEqual({ claimed: 2, blocked: 1 });
    expect(w.queueCalls.length).toBeGreaterThan(queue);
    expect(w.githubCalls.length).toBe(github);
  });

  test("the classes are kept between reads, so a prompt whose thread has not changed is not fetched again", async () => {
    const w = await primary();
    const threads = () => w.githubCalls.filter((c) => c.method === "GET" && c.url.includes("/issues/201/comments"));
    await w.core();
    expect(threads()).toHaveLength(1);
    for (let i = 0; i < 3; i++) {
      w.now.t += 46_000;
      await w.core();
      await settle(w);
    }
    // Read three more times: the list each time, the thread never again.
    expect(w.githubCalls.filter((c) => c.url.includes("/issues?labels=owner%2Fprompt"))).toHaveLength(4);
    expect(threads()).toHaveLength(1);
  });

  test("a read that fails leaves the last good copy and says why, and stops saying so when it works again", async () => {
    const w = await primary();
    await w.core();
    w.directDown.on = true;
    w.now.t += 60_000;
    await w.core();
    await settle(w);
    const { doc } = await w.core();
    expect(doc.board.counts).toEqual(QUEUE_NOW);
    expect(doc.health.board).toEqual({ ok: false, error: expect.stringContaining("the direct source is down") });
    expect(doc.health.asks.ok).toBe(false);
    // The copy is as old as its last good read, and the app is told so.
    expect(doc.read.board).toBe(new Date(w.now.t).toISOString()); // tried now
    expect((w.stored.get("m:board") as { at: number }).at).toBe(START); // good then
    // A source that is down is not a token that was refused: nothing is
    // handed to the laptop, and it is this side that reads it again.
    expect(
      ((await (await w.browse("/console/health", { token: null })).json()) as { refused: string[] }).refused,
    ).toEqual([]);
    w.directDown.on = false;
    w.now.t += 60_000;
    const queue = w.queueCalls.length;
    await w.core();
    await settle(w);
    expect(w.queueCalls.length).toBeGreaterThan(queue);
    const back = await w.core();
    expect(back.doc.health.board).toEqual({ ok: true, error: null });
    expect(back.doc.health.asks).toEqual({ ok: true, error: null });
  });

  test("after a restart what was last read is shown at once, as old as it is, and read again behind it", async () => {
    const w = await primary();
    await w.core();
    w.queueTasks[1].status = "claimed";
    w.now.t += 10 * 60_000;
    w.edge.now = w.makeEdge();
    const queue = w.queueCalls.length;
    const first = await w.core();
    expect(first.doc.board.counts).toEqual(QUEUE_NOW);
    // As old as it is: read ten minutes ago, and said to be.
    expect(first.doc.read.board).toBe(new Date(START).toISOString());
    expect(Number(first.res.headers.get("x-as-of"))).toBeLessThan(w.now.t - 9 * 60_000);
    await settle(w);
    expect(w.queueCalls.length).toBeGreaterThan(queue);
    const second = await w.core();
    expect(second.doc.board.counts).toEqual({ claimed: 2, blocked: 1 });
    expect(second.doc.read.board).toBe(new Date(w.now.t).toISOString());
  });

  test("after a restart, a first read that fails still shows what was kept, and says the read failed", async () => {
    const w = await primary();
    await w.core();
    w.edge.now = w.makeEdge();
    w.directDown.on = true;
    w.net.hostUp = false;
    w.net.edgeUp = false;
    w.now.t += 60_000;
    await w.core();
    await settle(w);
    const { doc } = await w.core();
    expect(doc.board.counts).toEqual(QUEUE_NOW);
    expect(doc.health.board).toEqual({ ok: false, error: expect.stringContaining("the direct source is down") });
    expect(doc.health.asks.ok).toBe(false);
  });

  test("after a restart, what an action changed is read from its source, not shown as it was kept", async () => {
    const w = await primary();
    await w.core();
    await w.browse("/console/api/doc/ops");
    await settle(w);
    // The object starts again, and the first thing asked of it is not a
    // look but a doing: nothing here has been read by this object yet.
    w.edge.now = w.makeEdge();
    const moved = await w.act("task.move", { task: 21048, to: "first" });
    expect(await moved.json()).toEqual({ ok: true, result: { task: 21048, priority: 10 } });
    const merged = await w.act("pr.merge", { pr: 55 });
    expect((await merged.json()) as { ok: boolean }).toMatchObject({ ok: true });
    // No time has passed, so nothing would be read again for being old:
    // the very next look shows both, read from where they changed.
    expect((await w.core()).doc.board.next.find((t: { id: number }) => t.id === 21048)?.priority).toBe(10);
    expect(((await (await w.browse("/console/api/doc/ops")).json()) as { prs: unknown[] }).prs).toEqual([]);
    // What no action touched is still the copy kept from before, shown
    // at once and as old as it is.
    expect((await w.core()).doc.asks.map((a: { issue: number }) => a.issue)).toContain(201);
  });

  test("what was read is kept once: content that has not changed is not written again, only its time, now and then", async () => {
    const w = await primary();
    await w.core();
    const writes = (key: string) => w.puts.filter((k) => k === key).length;
    expect([writes("m:board:0"), writes("m:board")]).toEqual([1, 1]);
    w.now.t += 31_000;
    await w.core();
    await settle(w);
    expect([writes("m:board:0"), writes("m:board")]).toEqual([1, 1]);
    w.now.t += 5 * 60_000;
    await w.core();
    await settle(w);
    expect([writes("m:board:0"), writes("m:board")]).toEqual([1, 2]);
    expect((w.stored.get("m:board") as { at: number }).at).toBe(w.now.t);
    w.queueTasks[1].status = "claimed";
    w.now.t += 31_000;
    await w.core();
    await settle(w);
    expect([writes("m:board:0"), writes("m:board")]).toEqual([2, 3]);
  });

  test("the laptop is told what this side reads, and stops reporting it", async () => {
    const w = await primary();
    // Its first report could not know, and carried everything.
    expect(w.reports[0].sections.map((s) => s.name)).toContain("board");
    // None of it was kept: this side had not read the board yet, and has no copy.
    expect(w.stored.has("m:board")).toBe(false);
    w.now.t += 1_000;
    const answer = await w.link.now.once();
    expect(answer?.owns).toEqual(["board", "prs", "priorities", "asks"]);
    expect(answer?.want).toEqual([]);
    // The laptop's own board changes. Its report carries neither the
    // board nor its word on how current the board is.
    (w.data.board as { counts: Record<string, number> }).counts = { claimed: 9 };
    (w.data.sessions as { moving: unknown[] }).moving = [{ id: "s1", title: "A session" }];
    w.now.t += 11_000;
    await w.hostReads();
    await w.link.now.once();
    const last = w.reports[w.reports.length - 1];
    expect(last.sections.map((s) => s.name)).toEqual(["sessions"]);
    for (const name of ["board", "prs", "priorities", "asks"]) expect(Object.keys(last.have)).not.toContain(name);
    expect(Object.keys(last.have)).toContain("sessions");
    expect((await w.core()).doc.board.counts).toEqual(QUEUE_NOW);
  });

  test("a section the laptop could not read is not said to be broken when this side reads it", async () => {
    const w = await setup({ direct: true });
    // The laptop cannot read the queue; this side can.
    w.failing.add("board");
    await w.hostReads();
    w.failing.delete("board");
    await w.hostReads();
    w.failing.add("board");
    w.now.t += 11_000;
    await w.hostReads();
    await w.link.now.once();
    const { doc } = await w.core();
    expect(doc.health.board).toEqual({ ok: true, error: null });
    expect(doc.board.counts).toEqual(QUEUE_NOW);
  });

  test("with the switch turned off again, the laptop's copies are taken back at once", async () => {
    const w = await primary();
    await w.core(); // this side's own copies are kept
    // Turning the switch off restarts the Worker without it.
    w.edge.now = w.makeEdge(false);
    w.now.t += 1_000;
    const first = await w.link.now.once();
    expect(first?.owns).toEqual([]);
    w.now.t += 1_000;
    await w.link.now.once();
    const { doc } = await w.core();
    expect(doc.board).toEqual(w.data.board);
    expect(doc.asks).toEqual([]);
    expect(doc.edge).toEqual({ held: 0, primary: false });
    expect((w.stored.get("m:board") as { own?: boolean }).own).toBeUndefined();
    expect(asked(w)).toBe(asked(w)); // and nothing more is read from here
    const calls = asked(w);
    w.now.t += 61_000;
    await w.core();
    await settle(w);
    expect(asked(w)).toBe(calls);
  });

  test("a laptop that does not know to send them again is asked for the copies this side used to read", async () => {
    const w = await primary();
    await w.core();
    w.edge.now = w.makeEdge(false);
    // A report as an older laptop makes it: it believes this side has
    // its board (it sent it long ago), so it only says what it has.
    const kept = JSON.parse((await w.keys.read()) ?? "{}") as { privateJwk: JsonWebKey };
    const say = async (sections: unknown[]) => {
      w.now.t += 1_000;
      const msg = {
        v: 1,
        at: w.now.t,
        config: w.edgeConfig,
        sections,
        have: {
          board: { hash: "theirs", at: w.now.t, error: null },
          sessions: { hash: "x", at: w.now.t, error: null },
        },
        acks: [],
      };
      const body = new TextEncoder().encode(JSON.stringify(msg));
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
      return (await res.json()) as { want: string[] };
    };
    // The board here is this side's own old read: wanted. The sessions
    // here are the laptop's: not wanted.
    expect((await say([])).want).toEqual(["board"]);
    // Sent, it replaces this side's copy though that was read later.
    const theirs = { counts: { claimed: 7 }, now: [], next: [], blocked: [] };
    expect((await say([{ name: "board", at: START - 60_000, hash: "theirs", data: theirs }])).want).toEqual([]);
    expect((await w.core()).doc.board).toEqual(theirs);
  });

  test("what the owner does is done here at once, laptop in reach or not, and never sent to the laptop", async () => {
    const w = await primary();
    const reply = await w.act("ask.reply", { issue: 201, text: "Thanks — noted." });
    expect(await reply.json()).toEqual({ ok: true, result: { issue: 201, queued: true } });
    const comments = () => w.githubCalls.filter((c) => c.method === "POST" && c.url.includes("/issues/201/comments"));
    expect(comments()).toHaveLength(1);
    expect(JSON.parse(comments()[0].body ?? "{}").body).toContain("Thanks — noted.");
    // The reply already shows in the state served here.
    const shown = (await w.core()).doc.asks.find((a: { issue: number }) => a.issue === 201) as {
      replies: { you: boolean; at: string; text: string }[];
    };
    expect(shown.replies[shown.replies.length - 1]).toEqual({
      you: true,
      at: "2026-11-22T12:00:00",
      text: "Thanks — noted.",
    });
    const closed = await w.act("ask.close", { issue: 202 });
    expect(await closed.json()).toEqual({ ok: true, result: { issue: 202, closed: true } });
    expect(w.ghIssues[202].state).toBe("closed");
    const ranked = await w.act("issue.rank", { issue: 301, rank: "next" });
    expect(await ranked.json()).toEqual({ ok: true, result: { issue: 301, rank: "next" } });
    expect(w.ghLabels[301]).toEqual(["next"]);
    const need = await w.act("need.comment", { ref: "developing-today/code#301", text: "Noted." });
    expect(need.status).toBe(200);
    expect((w.ghComments[301] ?? []).some((c) => c.body.startsWith("Noted."))).toBe(true);
    const comment = await w.act("issue.comment", { issue: 301, text: "And this." });
    expect(comment.status).toBe(200);
    expect((w.ghComments[301] ?? []).some((c) => c.body === "And this.")).toBe(true);
    const moved = await w.act("task.move", { task: 21048, to: "first" });
    expect(await moved.json()).toEqual({ ok: true, result: { task: 21048, priority: 10 } });
    expect(w.queueTasks.find((t) => t.id === 21048)?.priority).toBe(10);
    // The board served here already shows the move.
    expect((await w.core()).doc.board.next.find((t: { id: number }) => t.id === 21048)?.priority).toBe(10);
    // A merge, and a deploy started again.
    const merged = await w.act("pr.merge", { pr: 55 });
    expect(await merged.json()).toEqual({ ok: true, result: { number: 55, merged: true, sha: "5a5a5a5a5a5a" } });
    expect(w.ghMerged).toEqual([{ pr: 55, body: { merge_method: "merge" } }]);
    // The list of pull requests served here no longer has it.
    expect(((await (await w.browse("/console/api/doc/ops")).json()) as { prs: unknown[] }).prs).toEqual([]);
    const started = await w.act("deploy.start", { what: "site", scratch: true });
    expect(await started.json()).toEqual({ ok: true, result: { started: "site", scratch: true } });
    expect(w.ghStarted).toEqual([
      { workflow: "pm-console-edge-deploy.yml", body: { ref: "main", inputs: { scratch: "true" } } },
    ]);
    // The laptop was in reach the whole time, and was asked to do none of it.
    expect(w.ran).toEqual([]);
    expect(onlyPokes(w)).toBe(true);
    // Only the reply's queue task waits for a machine.
    expect((w.stored.get("held") as { kind: string }[]).map((h) => h.kind)).toEqual(["prompt.enqueue"]);
    // Each is on this side's record of what was done, without what was typed.
    const audit = w.stored.get("audit") as { action: string; phase: string; ok?: boolean; args: unknown }[];
    expect(audit.filter((a) => a.phase === "done" && a.ok).map((a) => a.action)).toEqual([
      "ask.reply",
      "ask.close",
      "issue.rank",
      "need.comment",
      "issue.comment",
      "task.move",
      "pr.merge",
      "deploy.start",
    ]);
    expect(JSON.stringify(audit)).not.toContain("Thanks");
  });

  test("an action a device sends twice under one name is done here once, and answered the same after a restart too", async () => {
    const w = await primary();
    const id = "0b000000-0000-4000-8000-000000000001";
    const comments = () =>
      w.githubCalls.filter((c) => c.method === "POST" && c.url.includes("/issues/201/comments")).length;
    const first = await w.act("ask.reply", { issue: 201, text: "Only once." }, id);
    expect(await first.json()).toEqual({ ok: true, result: { issue: 201, queued: true } });
    const second = await w.act("ask.reply", { issue: 201, text: "Only once." }, id);
    expect(await second.json()).toEqual({ ok: true, result: { issue: 201, queued: true }, again: true });
    w.edge.now = w.makeEdge();
    const third = await w.act("ask.reply", { issue: 201, text: "Only once." }, id);
    expect(await third.json()).toEqual({ ok: true, result: { issue: 201, queued: true }, again: true });
    expect(comments()).toBe(1);
    // Its queue task is waiting for a machine once, too.
    expect((w.stored.get("held") as { kind: string }[]).map((h) => h.kind)).toEqual(["prompt.enqueue"]);
    // On this side's record of what was done, under the device's name for it.
    expect(
      (await w.core()).doc.acts.filter((a: { id: string }) => a.id === id).map((a: { status: string }) => a.status),
    ).toEqual(["done"]);
    expect(w.ran).toEqual([]);
    // One that failed is answered the same again, not tried again: the
    // device asks under a new name if the owner wants it tried.
    const bad = "0b000000-0000-4000-8000-000000000002";
    w.ghBreaks.push((method, url) => method === "PUT" && url.includes("/merge"));
    const failed = await w.act("pr.merge", { pr: 55 }, bad);
    expect(failed.status).toBe(502);
    const said = (await failed.json()) as { error: string };
    w.ghBreaks.length = 0;
    const again = await w.act("pr.merge", { pr: 55 }, bad);
    expect([again.status, await again.json()]).toEqual([502, { ...said, again: true }]);
    expect(w.ghMerged).toEqual([]);
    expect((await w.act("pr.merge", { pr: 55 }, "0b000000-0000-4000-8000-000000000003")).status).toBe(200);
    expect(w.ghMerged).toHaveLength(1);
    // What this side does of its own (a test notification, a device) goes the same way.
    const own = "0b000000-0000-4000-8000-000000000004";
    const tried = await w.act("edge.retry", {}, own);
    expect(await tried.json()).toEqual({ ok: true, result: { tried: [] } });
    expect(await (await w.act("edge.retry", {}, own)).json()).toEqual({ ok: true, result: { tried: [] }, again: true });
  });

  test("sent again while it is being done here, the device is told to wait; after this side restarted mid-way, that it is not known", async () => {
    const w = await primary();
    const id = "0b000000-0000-4000-8000-000000000005";
    let letGo = () => {};
    const wait = new Promise<void>((r) => {
      letGo = r;
    });
    const merge = (method: string, url: string) => method === "PUT" && url.includes("/merge");
    w.ghHolds.push({ match: merge, wait });
    const merging = w.act("pr.merge", { pr: 55 }, id);
    await new Promise((r) => setTimeout(r, 5));
    const meanwhile = await w.act("pr.merge", { pr: 55 }, id);
    expect([meanwhile.status, await meanwhile.json()]).toEqual([
      409,
      { error: "This is still being done.", running: true },
    ]);
    // The object is put away while the merge is still out, and the
    // device asks the one that starts in its place.
    w.edge.now = w.makeEdge();
    const asked = await w.act("pr.merge", { pr: 55 }, id);
    expect(asked.status).toBe(502);
    expect(((await asked.json()) as { error: string }).error).toContain("Cloudflare restarted while doing this");
    letGo();
    await merging;
    // GitHub was asked to merge once.
    expect(w.githubCalls.filter((c) => merge(c.method, c.url))).toHaveLength(1);
  });

  test("two arrivals under one name at the same moment are one doing, here", async () => {
    const w = await primary();
    const id = "0b000000-0000-4000-8000-000000000007";
    // The first is still reading what is known of the name when the
    // second arrives: neither has written anything down.
    let open: () => void = () => {};
    w.storeSlow.set(
      `a:${id}`,
      new Promise<void>((r) => {
        open = r;
      }),
    );
    const first = w.act("pr.merge", { pr: 55 }, id);
    await new Promise((r) => setTimeout(r, 5));
    const second = w.act("pr.merge", { pr: 55 }, id);
    await new Promise((r) => setTimeout(r, 5));
    w.storeSlow.clear();
    open();
    const both = await Promise.all([first, second]);
    expect(both.map((r) => r.status).sort()).toEqual([200, 409]);
    expect(await (both.find((r) => r.status === 409) as Response).json()).toEqual({
      error: "This is still being done.",
      running: true,
    });
    expect(w.ghMerged).toHaveLength(1);
    // The name is free again afterwards: asked once more, it is answered from what was written down.
    expect(await (await w.act("pr.merge", { pr: 55 }, id)).json()).toMatchObject({ ok: true, again: true });
    expect(w.ghMerged).toHaveLength(1);
  });

  test("machines: doing the core itself, the edge says which kinds it does and which sections it reads, less what it was turned away for", async () => {
    const w = await primary();
    const machines = async () => (await (await w.browse("/console/api/machines")).json()) as MachinesAnswer;
    const first = await machines();
    expect(first.platform).toEqual({
      primary: true,
      actions: [
        "device.approve",
        "device.remove",
        "edge.retry",
        "held.drop",
        "push.class",
        "push.off",
        "push.on",
        "push.test",
        "ask.send",
        "ask.reply",
        "ask.close",
        "issue.comment",
        "need.comment",
        "issue.rank",
        "task.move",
        "task.cancel",
        "task.unblock",
        "pr.merge",
        "deploy.start",
        "intake.send",
      ],
      sections: ["board", "prs", "priorities", "asks"],
      refused: [],
    });
    expect(first.machines.map((m) => m.name)).toEqual(["nixos"]);
    // Turned away for merging: that is a machine's for the hour, and said so.
    w.ghRefuses.push((method, url) => method === "PUT" && url.includes("/merge"));
    await w.act("pr.merge", { pr: 55 });
    const turned = (await machines()).platform;
    expect(turned?.actions).not.toContain("pr.merge");
    expect(turned?.actions).toContain("issue.rank");
    expect(turned?.refused).toEqual(["pr.merge"]);
  });

  test("the platform's Workers are read at the edge from the repository's own list, with the last deploy of each that Actions deploys", async () => {
    const w = await primary();
    w.net.hostUp = false;
    const res = await w.browse("/console/api/platform");
    expect(res.status).toBe(200);
    const p = (await res.json()) as {
      checked: string;
      malformed: number;
      workers: { name: string; group: string; state: string; deploy: string; workflow: string | null; run: unknown }[];
    };
    expect([p.malformed, p.checked]).toEqual([0, "2026-11-23"]);
    // The platform first, then the rest by what they belong to.
    expect(p.workers[0]).toMatchObject({
      name: "pm-console-edge",
      group: "platform",
      state: "in service",
      deploy: "actions",
      workflow: "pm-console-edge-deploy.yml",
      // The latest run of its deploy on main, read from GitHub here.
      run: { commit: expect.stringMatching(/^[0-9a-f]{12}$/), state: "success", at: "2026-11-22T11:50:00Z" },
    });
    const byName = new Map(p.workers.map((x) => [x.name, x]));
    // One that a machine deploys by hand has no run to show; one that is not deployed, neither.
    expect(byName.get("message-bridge")).toMatchObject({ deploy: "by hand", workflow: null, run: null });
    expect(byName.get("fleet-hosted")).toMatchObject({ state: "not deployed", deploy: "none", run: null });
    expect(p.workers.map((x) => x.group)).toEqual(
      [...p.workers.map((x) => x.group)].sort((a, b) => ORDER.indexOf(a) - ORDER.indexOf(b)),
    );
    // No machine was asked.
    expect(w.hostSaw.filter((seen) => seen.includes("/api/platform"))).toEqual([]);
    // GitHub did not answer about the deploys: the list still shows, and says so for those.
    w.ghDrops.push((method, url) => method === "GET" && url.includes("/actions/workflows/"));
    const partial = (await (await w.browse("/console/api/platform")).json()) as typeof p;
    expect(partial.workers).toHaveLength(p.workers.length);
    expect(partial.workers[0].run).toEqual({ error: expect.any(String) });
  });

  test("handed on to the laptop, an action keeps the device's name for it: the laptop does it once", async () => {
    const w = await primary();
    const id = "0b000000-0000-4000-8000-000000000006";
    w.ghRefuses.push((method, url) => method === "PUT" && url.includes("/merge"));
    const first = await w.act("pr.merge", { pr: 55 }, id);
    expect(await first.json()).toMatchObject({ ok: true, result: { merged: 55 } });
    // Nothing was done here: what is written down under the name is
    // that it was given to a machine ...
    expect(w.stored.get(`a:${id}`)).toMatchObject({ kind: "pr.merge", state: "given" });
    // ... and sent again, it goes to the laptop, which knows it did it.
    const second = await w.act("pr.merge", { pr: 55 }, id);
    expect(await second.json()).toMatchObject({ ok: true, result: { merged: 55 }, again: true });
    expect(w.ran).toEqual([{ kind: "pr.merge", args: { pr: 55 } }]);
    // Noted once in the list of names, though it was noted twice.
    expect((w.stored.get("acted") as { id: string }[]).filter((e) => e.id === id)).toHaveLength(1);
  });

  test("a name keeps to the side that first took it: given to the laptop, it is not done here later, and done here, it is not given to the laptop later", async () => {
    const w = await primary();
    const merge = (method: string, url: string) => method === "PUT" && url.includes("/merge");
    const comment = (method: string, url: string) => method === "POST" && /\/issues\/\d+\/comments$/.test(url);
    const posted = () => w.githubCalls.filter((c) => comment(c.method, c.url)).length;
    // This side is turned away for comments, so one goes to the laptop,
    // which posts it; and its answer is lost on the way back.
    w.ghRefuses.push(comment);
    const given = "0d000000-0000-4000-8000-000000000001";
    w.net.loseAnswer = true;
    const first = await w.act("issue.comment", { issue: 7, text: "once, by the laptop" }, given);
    w.net.loseAnswer = false;
    expect(await first.json()).toMatchObject({ ok: true, held: true });
    expect(w.ran.filter((r) => r.kind === "issue.comment")).toHaveLength(1);
    const before = posted();
    // The token is put right and this side takes comments back.
    w.ghRefuses.length = 0;
    expect((await w.act("edge.retry", {})).status).toBe(200);
    expect(((await (await w.browse("/console/health")).json()) as { refused: string[] }).refused).toEqual([]);
    // The device never heard, and sends the comment again under its name.
    // It is the laptop's: this side does not post it.
    const again = await w.act("issue.comment", { issue: 7, text: "once, by the laptop" }, given);
    expect(await again.json()).toMatchObject({ ok: true });
    expect(posted()).toBe(before);
    expect(w.ran.filter((r) => r.kind === "issue.comment")).toHaveLength(1);
    // A new comment, under a new name, is this side's again.
    const fresh = await w.act("issue.comment", { issue: 7, text: "a new one" }, "0d000000-0000-4000-8000-000000000002");
    expect(await fresh.json()).toMatchObject({ ok: true });
    expect(posted()).toBe(before + 1);
    expect(w.ran.filter((r) => r.kind === "issue.comment")).toHaveLength(1);

    // Something that may not be done late, given to the laptop the same
    // way: its answer lost, the device is told it is not known ...
    w.ghRefuses.push(merge);
    const merged = "0d000000-0000-4000-8000-000000000003";
    w.net.loseAnswer = true;
    const lost = await w.act("pr.merge", { pr: 55 }, merged);
    w.net.loseAnswer = false;
    expect(lost.status).toBe(503);
    expect(w.ran.filter((r) => r.kind === "pr.merge")).toHaveLength(1);
    w.ghRefuses.length = 0;
    await w.act("edge.retry", {});
    // ... and asking again under the name reaches the laptop, which
    // says it merged it. This side never asks GitHub to merge.
    const asked = await w.act("pr.merge", { pr: 55 }, merged);
    expect(await asked.json()).toMatchObject({ ok: true, result: { merged: 55 }, again: true });
    expect(w.ghMerged).toEqual([]);
    expect(w.ran.filter((r) => r.kind === "pr.merge")).toHaveLength(1);

    // The other way about: done here, then this side is turned away for
    // the kind. The name is still answered from here, and the laptop is
    // not asked to do it.
    const here = "0d000000-0000-4000-8000-000000000004";
    const done = await w.act("issue.rank", { issue: 301, rank: "next" }, here);
    expect(await done.json()).toMatchObject({ ok: true });
    w.ghRefuses.push((method) => method !== "GET");
    const turned = await w.act("issue.rank", { issue: 201, rank: "next" }, "0d000000-0000-4000-8000-000000000005");
    expect(turned.status).not.toBe(200);
    expect(((await (await w.browse("/console/health")).json()) as { refused: string[] }).refused).toContain(
      "issue.rank",
    );
    const repeat = await w.act("issue.rank", { issue: 301, rank: "next" }, here);
    expect(await repeat.json()).toMatchObject({ ok: true, again: true });
    expect(w.ran.filter((r) => r.kind === "issue.rank")).toEqual([]);
  });

  test("the names remembered here are bounded: the oldest are forgotten, and an answer too long to keep is kept as how it went", async () => {
    const w = await primary();
    const name = (n: number) => `0c000000-0000-4000-8000-${String(n).padStart(12, "0")}`;
    for (let n = 1; n <= 302; n++) {
      expect((await w.act("edge.retry", {}, name(n))).status).toBe(200);
    }
    expect(w.stored.has(`a:${name(1)}`)).toBe(false);
    expect(w.stored.has(`a:${name(2)}`)).toBe(false);
    expect(w.stored.has(`a:${name(3)}`)).toBe(true);
    expect((w.stored.get("acted") as unknown[]).length).toBe(300);
    // After a week a name is forgotten.
    w.now.t += 7 * 24 * 3600_000 + 1;
    await w.act("edge.retry", {}, name(999));
    expect(w.stored.has(`a:${name(302)}`)).toBe(false);
    expect((w.stored.get("acted") as unknown[]).length).toBe(1);
    // An answer too long to keep is kept as how it went, without what it said.
    const long = name(1000);
    w.ghOdd.longAddress = true;
    const filed = await w.act("ask.send", { text: "With a long address" }, long);
    expect(filed.status).toBe(200);
    expect(((await filed.json()) as { result: { url: string } }).result.url.length).toBeGreaterThan(5000);
    const again = await w.act("ask.send", { text: "With a long address" }, long);
    expect([again.status, await again.json()]).toEqual([200, { ok: true, result: null, again: true }]);
    expect(w.githubCalls.filter((c) => c.method === "POST" && c.url.endsWith("/issues"))).toHaveLength(1);
  });

  test("served straight from the laptop, an action sent twice under one name is done once", async () => {
    const w = await setup();
    await w.hostReads();
    const id = "0d000000-0000-4000-8000-000000000001";
    const direct = async (body: unknown) =>
      handle(
        new Request(`https://${HOST}/console/api/act`, {
          method: "POST",
          headers: {
            "content-type": "application/json",
            "x-pm-console": "1",
            "cf-access-jwt-assertion": await w.token(),
          },
          body: JSON.stringify(body),
        }),
        w.hostDeps,
        "127.0.0.1",
      );
    const first = await direct({ kind: "ask.send", args: { text: "no edge in between" }, id });
    expect(await first.json()).toEqual({ ok: true, result: { issue: 101 } });
    const second = await direct({ kind: "ask.send", args: { text: "no edge in between" }, id });
    expect(await second.json()).toEqual({ ok: true, result: { issue: 101 }, again: true });
    expect(w.ran).toHaveLength(1);
    // Without a name it is done each time it is asked, as it always was.
    await direct({ kind: "ask.send", args: { text: "no edge in between" } });
    expect(w.ran).toHaveLength(2);
  });

  test("a merge asked for from the list is a merge of what the list showed, not of what was pushed since", async () => {
    const w = await primary();
    const shown = (
      (await (await w.browse("/console/api/doc/ops")).json()) as { prs: { number: number; head: string }[] }
    ).prs[0];
    // Something is pushed to it after the owner looked (and, offline,
    // asked for the merge).
    w.ghPrs[0].head.sha = "6666666666666666666666666666666666666666";
    const late = await w.act("pr.merge", { pr: shown.number, head: shown.head });
    expect(late.status).toBe(502);
    expect(((await late.json()) as { error: string }).error).toContain("Head branch was modified");
    expect(w.ghMerged).toEqual([]);
    // That was GitHub's answer to the question asked, not a refusal of
    // this side's token: it is not handed to the laptop to merge blind.
    expect(w.ran).toEqual([]);
    // Looked at again and asked again, it is merged as it now stands.
    const now = await w.act("pr.merge", { pr: shown.number, head: "6666666666666666666666666666666666666666" });
    expect(now.status).toBe(200);
    expect(w.ghMerged).toEqual([
      { pr: 55, body: { merge_method: "merge", sha: "6666666666666666666666666666666666666666" } },
    ]);
  });

  test("a new prompt is filed here and only its queue task waits for a machine, which files it once", async () => {
    const w = await primary();
    w.net.hostUp = false;
    w.net.edgeUp = false;
    const res = await w.act("ask.send", { text: "File this from the edge" });
    expect(await res.json()).toEqual({
      ok: true,
      result: { issue: 500, url: "https://github.com/developing-today/code/issues/500", queued: true },
    });
    expect(w.githubCalls.filter((c) => c.method === "POST" && c.url.endsWith("/issues"))).toHaveLength(1);
    const held = w.stored.get("held") as { kind: string; args: Record<string, unknown> }[];
    expect(held.map((h) => [h.kind, h.args.issue, h.args.text])).toEqual([
      ["prompt.enqueue", 500, "File this from the edge"],
    ]);
    // The app says one thing is waiting.
    expect((await w.core()).doc.edge).toEqual({ held: 1, primary: true });
    // The laptop connects and files the task once; the issue is not filed again.
    w.net.hostUp = true;
    w.net.edgeUp = true;
    w.now.t += 60_000;
    expect((await w.link.now.once())?.commands.map((c) => c.kind)).toEqual(["prompt.enqueue"]);
    w.now.t += 1_000;
    expect((await w.link.now.once())?.commands).toEqual([]);
    expect(w.ran.filter((r) => r.kind === "prompt.enqueue")).toHaveLength(1);
    expect(w.githubCalls.filter((c) => c.method === "POST" && c.url.endsWith("/issues"))).toHaveLength(1);
    expect((await w.core()).doc.edge).toEqual({ held: 0, primary: true });
  });

  test("a task the laptop files late is on the board here as soon as the laptop says it is done", async () => {
    const w = await primary();
    await w.core();
    await settle(w);
    w.table["prompt.enqueue"].run = async (a) => {
      w.ran.push({ kind: "prompt.enqueue", args: a });
      w.queueTasks.push({ ...w.queueTasks[1], id: 21061, title: "Owner prompt, filed late", issue: Number(a.issue) });
      return { issue: a.issue, queued: true };
    };
    expect((await w.act("ask.send", { text: "File this, and its task after" })).status).toBe(200);
    await w.core();
    await settle(w);
    // The laptop takes the task to file, and files it.
    w.now.t += 1_000;
    expect((await w.link.now.once())?.commands.map((c) => c.kind)).toEqual(["prompt.enqueue"]);
    expect(w.ran.map((r) => r.kind)).toEqual(["prompt.enqueue"]);
    const before = w.queueCalls.length;
    expect((await w.core()).doc.board.next.map((t: { id: number }) => t.id)).not.toContain(21061);
    await settle(w);
    expect(w.queueCalls.length).toBe(before);
    // Its next report says so; the board is read again then, not when
    // its half minute comes round. Only the board: filing a task
    // changed nothing else that is read here.
    const github = w.githubCalls.length;
    w.now.t += 1_000;
    await w.link.now.once();
    expect(w.queueCalls.length).toBeGreaterThan(before);
    expect(w.githubCalls.length).toBe(github);
    expect((await w.core()).doc.board.next.map((t: { id: number }) => t.id)).toContain(21061);
    // One that failed changed nothing, and nothing is read for it.
    w.table["prompt.enqueue"].run = async () => {
      throw new OpError(502, "the queue would not take it");
    };
    expect((await w.act("ask.send", { text: "And another" })).status).toBe(200);
    await w.core();
    await settle(w);
    w.now.t += 1_000;
    await w.link.now.once();
    const quiet = w.queueCalls.length;
    w.now.t += 1_000;
    await w.link.now.once();
    expect(w.queueCalls.length).toBe(quiet);
  });

  test("a bug, a note and a session are filed here with his words first; their tasks are a machine's to file", async () => {
    const w = await primary();
    w.net.hostUp = false;
    w.net.edgeUp = false;
    const WORDS = "the badge count is one too many\non the Needs tab";
    const bug = await w.act("intake.send", { id: "d1", kind: "bug", area: "pm-stack", text: WORDS });
    expect(await bug.json()).toEqual({
      ok: true,
      result: {
        issue: 500,
        url: "https://github.com/developing-today/code/issues/500",
        commented: false,
        queued: true,
      },
    });
    const filed = w.githubCalls.filter((c) => c.method === "POST" && c.url.endsWith("/issues"));
    expect(filed).toHaveLength(1);
    const body = JSON.parse(filed[0].body ?? "{}") as { title: string; body: string; labels: string[] };
    expect(body.body.startsWith(`${WORDS}\n\n---\n`)).toBe(true);
    expect(body.labels).toEqual(["kind/bug", "source/human", "owner/intake", "app/pm-stack", "now"]);
    // A note on an issue that is already there: a comment, and a task of its own kind.
    const note = await w.act("intake.send", {
      kind: "note",
      text: "do this first",
      about: { issue: 301 },
      rank: "expedite",
    });
    expect(((await note.json()) as { result: Record<string, unknown> }).result).toMatchObject({ issue: 301, commented: true, queued: true });
    expect((w.ghComments[301] ?? []).some((c) => c.body.startsWith("do this first\n\n---\n"))).toBe(true);
    expect(w.ghLabels[301]).toEqual(["owner/intake", "expedite"]);
    // As a session: the owner's prompt, with the routing labels added.
    const session = await w.act("intake.send", {
      kind: "note",
      area: "fleet",
      text: "pick these up first",
      mode: "session",
    });
    expect(((await session.json()) as { result: Record<string, unknown> }).result).toMatchObject({ issue: 501, commented: false, queued: true });
    expect(w.ghLabels[501]).toEqual(["owner/prompt", "expedite", "source/human", "owner/intake", "app/fleet"]);
    // Three tasks wait for a machine, each as the action that builds it there.
    const at = new Date(w.now.t).toISOString();
    expect(
      (w.stored.get("held") as { kind: string; args: Record<string, unknown> }[]).map((h) => [h.kind, h.args]),
    ).toEqual([
      ["triage.enqueue", { issue: 500, title: body.title, rank: "now", at }],
      ["triage.enqueue", { issue: 301, title: "do this first", rank: "expedite", at, note: true }],
      ["prompt.enqueue", { issue: 501, text: "pick these up first\n\n(note, Fleet; from the PM app)", at }],
    ]);
    // What he typed is not on the record of what was done.
    expect(JSON.stringify(w.stored.get("audit"))).not.toContain("badge count");
    // The Intake screen, read here, already has them.
    const intake = (await (await w.browse("/console/api/intake")).json()) as {
      items: { number: number; kind: string }[];
    };
    expect(intake.items.map((i) => [i.number, i.kind]).sort()).toEqual([
      [301, "note"],
      [500, "bug"],
      [501, "session"],
    ]);
    // A machine connects: each task is filed once.
    w.net.hostUp = true;
    w.net.edgeUp = true;
    w.now.t += 60_000;
    await w.link.now.once();
    w.now.t += 1_000;
    await w.link.now.once();
    expect(w.ran.map((r) => [r.kind, r.args.issue])).toEqual([
      ["triage.enqueue", 500],
      ["triage.enqueue", 301],
      ["prompt.enqueue", 501],
    ]);
    expect(w.stored.get("held")).toEqual([]);
  });

  test("a laptop from before this side read things itself changes nothing by saying what it has", async () => {
    const w = await primary();
    await w.core();
    const kept = JSON.parse((await w.keys.read()) ?? "{}") as { privateJwk: JsonWebKey };
    const mine = w.stored.get("m:board") as { hash: string; at: number };
    // It lists the board among what it has, with the very content this
    // side read (so the same fingerprint), a later time, and its own
    // trouble reading it; and prompts this side has not kept a copy of.
    w.now.t += 5_000;
    const msg = {
      v: 1,
      at: w.now.t,
      config: w.edgeConfig,
      sections: [],
      have: {
        board: { hash: mine.hash, at: w.now.t, error: "the queue store is down" },
        prs: { hash: "theirs", at: w.now.t, error: null },
      },
      acks: [],
    };
    const body = new TextEncoder().encode(JSON.stringify(msg));
    const before = asked(w);
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
    const answer = (await res.json()) as { want: string[]; owns: string[] };
    // Nothing of what this side reads is asked of it (it would send the
    // whole section every time, and it would never be taken).
    expect(answer.want).toEqual([]);
    expect(answer.owns).toEqual(["board", "prs", "priorities", "asks"]);
    // Its word did not make this side read anything again ...
    expect(asked(w)).toBe(before);
    // ... nor is its trouble said of a section this side reads well.
    const { doc } = await w.core();
    expect(doc.health.board).toEqual({ ok: true, error: null });
    expect((w.stored.get("m:board") as { at: number }).at).toBe(mine.at);
  });

  test("an answer from GitHub with no issue number on it is not passed on as a task", async () => {
    const w = await primary();
    w.ghOdd.noNumber = true;
    const res = await w.act("intake.send", { kind: "bug", text: "odd" });
    const { result } = (await res.json()) as { result: { queued: boolean; note: string } };
    expect(result.queued).toBe(false);
    expect(result.note).toBe("filed, not queued: the issue could not be read from its task");
    expect(w.stored.get("held") ?? []).toEqual([]);
  });

  test("with too much already waiting, the issue is still filed and the answer says its task is not", async () => {
    const w = await primary();
    w.stored.set(
      "held",
      Array.from({ length: 100 }, (_, i) => ({
        id: `h${i}`,
        kind: "ask.send",
        args: {},
        who: OWNER,
        at: new Date(w.now.t).toISOString(),
      })),
    );
    w.edge.now = w.makeEdge();
    const res = await w.act("intake.send", { kind: "bug", text: "one more" });
    const { result } = (await res.json()) as { result: { issue: number; queued: boolean; note: string } };
    expect(result).toMatchObject({ issue: 500, queued: false });
    expect(result.note).toBe("filed, not queued: too much is already waiting for a machine");
    expect((w.stored.get("held") as unknown[]).length).toBe(100);
  });

  test("the screens that need only GitHub and the queue are read here, and the ones that need a machine are asked of it", async () => {
    const w = await primary();
    w.net.hostUp = false;
    w.net.edgeUp = false;
    const read = async (path: string) => {
      const res = await w.browse(`/console/api/${path}`);
      return { status: res.status, body: (await res.json()) as Record<string, any> };
    };
    const pr = await read("prs/55");
    expect(pr.status).toBe(200);
    expect(pr.body).toMatchObject({
      number: 55,
      title: "Fresh PR",
      state: "open",
      mergeable: true,
      body: "What it does.",
    });
    const queue = await read("queue?status=blocked");
    expect(queue.body.counts).toEqual(QUEUE_NOW);
    expect(queue.body.tasks.map((t: { id: number }) => t.id)).toEqual([21049]);
    expect((await read("priorities")).body.issues.map((i: { number: number }) => i.number)).toEqual([301]);
    expect((await read("prs")).body.map((p: { number: number }) => p.number)).toEqual([55]);
    const releases = await read("releases");
    expect(releases.body.site[0]).toMatchObject({ id: 4501, state: "success", byHand: false, commit: "0123456789ab" });
    expect(releases.body.app[0].url).toContain("pm-console-android.yml");
    expect((await read("roadmaps")).body.slice(0, 4).map((c: { id: string }) => c.id)).toEqual([
      "id",
      "fleet",
      "atlas",
      "pm-stack",
    ]);
    const roadmap = await read("roadmap?area=pm-stack");
    expect(roadmap.body.rows.map((r: { title: string; phase: string }) => [r.title, r.phase])).toEqual([
      ["Coherence", "doing"],
    ]);
    expect(roadmap.body.issues.now.map((i: { number: number }) => i.number)).toEqual([301]);
    // Looked at again within the minute, it is not asked of GitHub again:
    // what read it is kept, here as on a machine.
    const github = w.githubCalls.length;
    expect((await read("roadmap?area=pm-stack")).body.rows).toHaveLength(1);
    expect(w.githubCalls.length).toBe(github);
    expect((await read("roadmap?area=pm-stack&fresh=1")).status).toBe(200);
    expect(w.githubCalls.length).toBe(github + 3);
    expect((await read("bugs")).status).toBe(200);
    expect((await read("intake")).body.items).toEqual([]);
    // A mistake in what was asked is the caller's, said as such.
    expect((await read("roadmap?area=nonesuch")).status).toBe(400);
    expect((await read("prs/abc")).status).toBe(400);
    // The laptop is out of reach and none of that needed it.
    expect(w.hostSaw).toEqual([]);
    // What only a machine has is asked of it: answered when it is
    // there, and said to be out of reach when it is not.
    const away = await read("services");
    expect(away.status).toBe(503);
    expect(away.body.error).toContain("cannot be reached");
    w.net.hostUp = true;
    expect((await w.browse("/console/api/push")).status).toBe(200);
    await w.browse("/console/api/agents");
    expect(w.hostSaw).toEqual(["GET /console/api/agents"]);
  });

  test("a read that GitHub refuses is answered with why, not sent to the laptop", async () => {
    const w = await primary();
    w.directDown.on = true;
    const res = await w.browse("/console/api/prs/55");
    expect(res.status).toBe(502);
    expect(((await res.json()) as { error: string }).error).toContain("the direct source is down");
    expect(w.hostSaw).toEqual([]);
  });

  test("an action GitHub refuses this side's token for is done by the laptop instead, and left to it for the hour", async () => {
    const w = await primary();
    const merges = () => w.githubCalls.filter((c) => c.method === "PUT" && c.url.endsWith("/pulls/55/merge")).length;
    w.ghRefuses.push((method, url) => method === "PUT" && url.includes("/merge"));
    // Refused here with nothing done: the laptop, which has its own way in, does it.
    const res = await w.act("pr.merge", { pr: 55 });
    expect(res.status).toBe(200);
    expect(await res.json()).toMatchObject({ ok: true, result: { merged: 55 } });
    expect(w.ran).toEqual([{ kind: "pr.merge", args: { pr: 55 } }]);
    expect(merges()).toBe(1);
    expect(w.ghMerged).toEqual([]);
    // Health says which, so that it is plain what this side's token lacks.
    const health = async () =>
      ((await (await w.browse("/console/health", { token: null })).json()) as { refused: string[] }).refused;
    expect(await health()).toEqual(["pr.merge"]);
    // It is on this side's record as refused here, not as a failure of the owner's.
    const audit = w.stored.get("audit") as { action: string; phase: string; ok?: boolean; error?: string }[];
    expect(audit.filter((a) => a.action === "pr.merge" && a.phase === "done")).toEqual([
      expect.objectContaining({ ok: false, error: "refused here; left to a machine" }),
    ]);
    // The next one within the hour goes straight to the laptop: GitHub is not asked again.
    w.now.t += 30 * 60_000;
    await w.act("pr.merge", { pr: 55 });
    expect(merges()).toBe(1);
    expect(w.ran).toHaveLength(2);
    // What it was not refused for is still done here.
    expect((await w.act("issue.comment", { issue: 301, text: "still here" })).status).toBe(200);
    expect(w.ran).toHaveLength(2);
    // After the hour this side tries again; the token has been put right meanwhile.
    w.ghRefuses.length = 0;
    w.now.t += 31 * 60_000;
    expect(await health()).toEqual([]);
    const again = await w.act("pr.merge", { pr: 55 });
    expect(await again.json()).toEqual({ ok: true, result: { number: 55, merged: true, sha: "5a5a5a5a5a5a" } });
    expect(w.ran).toHaveLength(2);
  });

  test("what the laptop does to something this side reads is read again here at once, not when its interval comes round", async () => {
    const w = await primary();
    await w.core();
    await w.browse("/console/api/doc/ops");
    await settle(w);
    const lists = () => w.githubCalls.filter((c) => c.method === "GET" && /\/pulls\?/.test(c.url)).length;
    // Refused here, so the laptop merges it, and on GitHub it is gone.
    w.ghRefuses.push((method, url) => method === "PUT" && url.includes("/merge"));
    w.table["pr.merge"].run = async (a) => {
      w.ran.push({ kind: "pr.merge", args: a });
      w.ghPrs.length = 0;
      return { merged: a.pr };
    };
    const before = lists();
    expect((await w.act("pr.merge", { pr: 55 })).status).toBe(200);
    expect(w.ran.map((r) => r.kind)).toEqual(["pr.merge"]);
    // The laptop sent the list as it read it. That copy is not taken
    // (the list is this side's to read), but it says the list changed:
    // this side read it again, once, before the answer went back.
    expect(lists()).toBe(before + 1);
    expect(((await (await w.browse("/console/api/doc/ops")).json()) as { prs: unknown[] }).prs).toEqual([]);
    expect(lists()).toBe(before + 1);
    // The same for something only a machine does: the task of an issue,
    // filed by the laptop, is on the board served here at once.
    w.table["triage.enqueue"].run = async (a) => {
      w.ran.push({ kind: "triage.enqueue", args: a });
      w.queueTasks.push({ ...w.queueTasks[1], id: 21060, title: "Filed by the laptop", issue: Number(a.issue) });
      return { issue: a.issue, queued: true };
    };
    const filed = await w.act("triage.enqueue", { issue: 301, rank: "now", at: new Date(w.now.t).toISOString() });
    expect(filed.status).toBe(200);
    expect((await w.core()).doc.board.next.map((t: { id: number }) => t.id)).toContain(21060);
    // What the laptop did that touches nothing read here asks nothing of GitHub.
    const github = w.githubCalls.length;
    const queue = w.queueCalls.length;
    expect((await w.act("question.answer", { qid: "q1", answer: "yes" })).status).toBe(200);
    expect([w.githubCalls.length, w.queueCalls.length]).toEqual([github, queue]);
  });

  test("refused here with the laptop out of reach: held if it may be done late, and said to be refused if not", async () => {
    const w = await primary();
    w.net.hostUp = false;
    w.net.edgeUp = false;
    w.ghRefuses.push((method) => method !== "GET");
    const comment = await w.act("issue.comment", { issue: 301, text: "later, then" });
    expect(await comment.json()).toEqual({ ok: true, held: true, result: null });
    expect((w.stored.get("held") as { kind: string }[]).map((h) => h.kind)).toEqual(["issue.comment"]);
    const merge = await w.act("pr.merge", { pr: 55 });
    expect(merge.status).toBe(503);
    expect(((await merge.json()) as { error: string }).error).toContain("not something to do late, so it is not kept");
    expect(w.ghMerged).toEqual([]);
  });

  test("a refusal after something was written is not handed on: the laptop would do the written part again", async () => {
    const w = await primary();
    // Ranking adds a label and takes the old one off. The first is taken, the second refused.
    w.ghRefuses.push((method) => method === "DELETE");
    const res = await w.act("issue.rank", { issue: 301, rank: "next" });
    expect(res.status).toBe(502);
    expect(w.ran).toEqual([]);
    expect(w.ghLabels[301]).toEqual(["now", "next"]);
    // Nor is a failure that is not a refusal: GitHub being broken is not this side lacking the right.
    w.ghRefuses.length = 0;
    w.ghBreaks.push((method, url) => method === "PUT" && url.includes("/merge"));
    const merge = await w.act("pr.merge", { pr: 55 });
    expect(merge.status).toBe(502);
    expect(w.ran).toEqual([]);
    const health = (await (await w.browse("/console/health", { token: null })).json()) as { refused: string[] };
    expect(health.refused).toEqual([]);
    // Both are on the record of what was done, as failures.
    const acts = (await w.core()).doc.acts as { kind: string; status: string }[];
    expect(
      acts
        .filter((a) => a.status === "failed")
        .map((a) => a.kind)
        .sort(),
    ).toEqual(["issue.rank", "pr.merge"]);
    // Nor one that got no answer at all: it may have been taken.
    w.ghBreaks.length = 0;
    w.directDown.on = true;
    const silent = await w.act("deploy.start", { what: "site" });
    expect(silent.status).toBe(502);
    expect(((await silent.json()) as { error: string }).error).toContain("GitHub could not be reached");
    expect(w.ran).toEqual([]);
  });

  test("the queue store refusing this side's token is a refusal like GitHub's: the laptop moves the task", async () => {
    const w = await primary();
    const before = w.queueTasks.find((t) => t.id === 21048)?.priority;
    w.queueRefuses.on = true;
    const res = await w.act("task.move", { task: 21048, to: "first" });
    expect(await res.json()).toMatchObject({ ok: true, result: { task: 21048, priority: 10 } });
    expect(w.ran).toEqual([{ kind: "task.move", args: { task: 21048, to: "first" } }]);
    // It was tried here first (and once more, in case the token had only
    // been changed); and then the board, which the laptop said it had
    // changed, was to be read again here, and was refused the same way.
    expect(w.queueRefused.n).toBe(4);
    // Nothing was changed from here.
    expect(w.queueTasks.find((t) => t.id === 21048)?.priority).toBe(before);
    // So the board is the laptop's to read as well: the copy that came
    // with its answer is shown at once, and health says both.
    const { doc } = await w.core();
    expect(doc.board).toEqual(w.data.board);
    expect(doc.health.board).toEqual({ ok: true, error: null });
    expect(
      ((await (await w.browse("/console/health", { token: null })).json()) as { refused: string[] }).refused,
    ).toEqual(["task.move", "read.board"]);
  });

  test("a section this side's token is refused for is the laptop's to read, as before, until the token is put right", async () => {
    const w = await primary();
    const health = async () =>
      ((await (await w.browse("/console/health", { token: null })).json()) as { refused: string[] }).refused;
    const ops = async () =>
      (await (await w.browse("/console/api/doc/ops")).json()) as {
        prs: { number: number }[];
        health: Record<string, { ok: boolean; error: string | null }>;
      };
    const lists = () => w.githubCalls.filter((c) => c.method === "GET" && /\/pulls\?/.test(c.url)).length;
    await ops();
    await settle(w);
    expect((await ops()).prs.map((p) => p.number)).toEqual([55]);
    // GitHub stops letting this side's token list pull requests. The
    // list's interval passes, and the read behind the next look is refused.
    w.ghRefuses.push((method, url) => method === "GET" && /\/pulls\?/.test(url));
    w.now.t += 91_000;
    await ops();
    await settle(w);
    expect(await health()).toEqual(["read.prs"]);
    // Until the laptop's copy comes, the last one read here is shown,
    // with why it could not be read again.
    const waiting = await ops();
    expect(waiting.prs.map((p) => p.number)).toEqual([55]);
    expect(waiting.health.prs.ok).toBe(false);
    // For as long as no laptop's copy has come: it is not asked of
    // GitHub again, and it does not start to look as if all were well.
    const refusedAt = lists();
    w.now.t += 91_000;
    await ops();
    await settle(w);
    const still = await ops();
    expect(lists()).toBe(refusedAt);
    expect(still.prs.map((p) => p.number)).toEqual([55]);
    expect(still.health.prs).toEqual({ ok: false, error: expect.stringContaining("turned away") });
    // The laptop is told the list is not read here any more ...
    w.now.t += 1_000;
    expect((await w.link.now.once())?.owns).toEqual(["board", "priorities", "asks"]);
    // ... so its next report brings its own copy, which is shown as it
    // was before this side read anything.
    w.now.t += 1_000;
    await w.link.now.once();
    expect(w.reports[w.reports.length - 1].sections.map((x) => x.name)).toContain("prs");
    const theirs = await ops();
    expect(theirs.prs).toEqual(w.data.prs as never);
    expect(theirs.health.prs).toEqual({ ok: true, error: null });
    // It is not asked for here again within the hour, however often it is looked at.
    const asked = lists();
    w.now.t += 91_000;
    await ops();
    await settle(w);
    expect(lists()).toBe(asked);
    // What it was not refused for is still read here, and not reported.
    const queue = w.queueCalls.length;
    await w.core();
    await settle(w);
    expect(w.queueCalls.length).toBeGreaterThan(queue);
    await w.hostReads();
    await w.link.now.once();
    const last = w.reports[w.reports.length - 1];
    expect(Object.keys(last.have)).toContain("prs");
    for (const name of ["board", "priorities", "asks"]) expect(Object.keys(last.have)).not.toContain(name);
    // After the hour, with the token put right, it is read here again,
    // and the laptop is told so.
    w.ghRefuses.length = 0;
    w.now.t += 60 * 60_000;
    expect(await health()).toEqual([]);
    await ops();
    await settle(w);
    expect((await ops()).prs.map((p) => p.number)).toEqual([55]);
    expect(lists()).toBeGreaterThan(asked);
    await w.hostReads();
    expect((await w.link.now.once())?.owns).toEqual(["board", "prs", "priorities", "asks"]);
  });

  test("what this side is turned away for is remembered when the object starts again, and tried again when the owner says", async () => {
    const w = await primary();
    const health = async () =>
      ((await (await w.browse("/console/health", { token: null })).json()) as { refused: string[] }).refused;
    const ops = async () => (await (await w.browse("/console/api/doc/ops")).json()) as { prs: { number: number }[] };
    const lists = () => w.githubCalls.filter((c) => c.method === "GET" && /\/pulls\?/.test(c.url)).length;
    const merges = () => w.githubCalls.filter((c) => c.method === "PUT" && c.url.endsWith("/pulls/55/merge")).length;
    await ops();
    await settle(w);
    // The list of pull requests and merging are both refused here.
    w.ghRefuses.push((method, url) => method === "GET" && /\/pulls\?/.test(url));
    w.ghRefuses.push((method, url) => method === "PUT" && url.includes("/merge"));
    w.now.t += 91_000;
    await ops();
    await settle(w);
    // (Each is written down as it happens: an object that starts again
    // between the two knows of the first.)
    w.edge.now = w.makeEdge();
    expect(await health()).toEqual(["read.prs"]);
    await w.act("pr.merge", { pr: 55 });
    expect(await health()).toEqual(["pr.merge", "read.prs"]);
    w.now.t += 1_000;
    await w.link.now.once();
    w.now.t += 1_000;
    await w.link.now.once();
    expect((await ops()).prs).toEqual(w.data.prs as never);
    const asked = [lists(), merges()];
    // The object is put away and starts again, as it does whenever it
    // is idle. It does not claim the list back: the laptop would be told
    // to stop sending it, and each start would only be refused again.
    w.edge.now = w.makeEdge();
    expect(await health()).toEqual(["pr.merge", "read.prs"]);
    w.now.t += 91_000;
    expect((await ops()).prs).toEqual(w.data.prs as never);
    await settle(w);
    await w.hostReads();
    expect((await w.link.now.once())?.owns).toEqual(["board", "priorities", "asks"]);
    await w.act("pr.merge", { pr: 55 });
    expect([lists(), merges()]).toEqual(asked);
    expect(w.ran.map((r) => r.kind)).toEqual(["pr.merge", "pr.merge"]);
    // The token is put right, and the owner says to try again now.
    w.ghRefuses.length = 0;
    const retry = await w.act("edge.retry");
    expect(await retry.json()).toEqual({ ok: true, result: { tried: ["pr.merge", "read.prs"] } });
    expect(await health()).toEqual([]);
    // That was this side's own to do: the laptop was not asked.
    expect(w.ran).toHaveLength(2);
    // It holds across a restart too, and both are done here again.
    w.edge.now = w.makeEdge();
    expect(await health()).toEqual([]);
    w.now.t += 91_000;
    await ops();
    await settle(w);
    expect((await ops()).prs.map((p) => p.number)).toEqual([55]);
    expect(lists()).toBeGreaterThan(asked[0]);
    expect((await w.act("pr.merge", { pr: 55 })).status).toBe(200);
    expect(merges()).toBe(asked[1] + 1);
    expect(w.ran).toHaveLength(2);
  });

  test("what was kept of being turned away is not believed past its time, nor for longer than it could have been set", async () => {
    const w = await primary();
    const health = async () =>
      ((await (await w.browse("/console/health", { token: null })).json()) as { refused: string[] }).refused;
    w.stored.set("away", {
      kinds: { "pr.merge": w.now.t + 10 * 60_000, "ask.send": w.now.t - 1, "issue.rank": "soon" },
      reads: { prs: w.now.t + 365 * 24 * 3600_000, board: w.now.t - 1 },
    });
    w.edge.now = w.makeEdge();
    expect(await health()).toEqual(["pr.merge", "read.prs"]);
    w.now.t += 11 * 60_000;
    expect(await health()).toEqual(["read.prs"]);
    // A year was written down; an hour is the longest it can stand.
    w.now.t += 50 * 60_000;
    expect(await health()).toEqual([]);
    // Storage that cannot keep it does not stop the hand-over itself.
    w.storeFails.add("away");
    w.ghRefuses.push((method, url) => method === "PUT" && url.includes("/merge"));
    expect((await w.act("pr.merge", { pr: 55 })).status).toBe(200);
    expect(w.ran.map((r) => r.kind)).toEqual(["pr.merge"]);
    expect(await health()).toEqual(["pr.merge"]);
    w.ghRefuses.push((method, url) => method === "GET" && url.includes("/search/issues"));
    const said = async () =>
      ((await (await w.browse("/console/api/doc/ops")).json()) as { health: Record<string, { error: string | null }> })
        .health.priorities.error;
    await said();
    await settle(w);
    expect(await health()).toEqual(["pr.merge", "read.priorities"]);
    // What the section says is why it could not be read, not that
    // something could not be written down ...
    expect(await said()).toContain("Resource not accessible");
    // ... and from then on, with no copy of anybody's here, that this
    // side was turned away: not that a laptop has been slow to report.
    w.now.t += 181_000;
    await said();
    await settle(w);
    expect(await said()).toContain("turned away");
  });

  test("a section refused here from the start shows the laptop's copy without a gap", async () => {
    // The laptop has been reporting, and this side has only now been
    // given its tokens: the copies here are the laptop's.
    const w = await setup({ direct: true });
    w.edge.now = w.makeEdge(false);
    await w.hostReads();
    await w.link.now.once();
    expect((await w.core()).doc.board).toEqual(w.data.board);
    w.queueRefuses.on = true;
    w.edge.now = w.makeEdge(true);
    w.now.t += 31_000;
    // Its first look shows what is kept, and the read behind it is refused.
    const first = (await w.core()).doc;
    expect(first.board).toEqual(w.data.board);
    await settle(w);
    expect(w.queueRefused.n).toBeGreaterThan(0);
    const after = (await w.core()).doc;
    expect(after.board).toEqual(w.data.board);
    expect(after.health.board).toEqual({ ok: true, error: null });
    // Nothing of the laptop's was lost on the way, and it goes on reporting.
    w.now.t += 1_000;
    expect((await w.link.now.once())?.owns).not.toContain("board");
  });

  test("a token that may read the queue and not change it: the laptop moves the task, though this side had read it", async () => {
    const w = await primary();
    const before = w.queueTasks.find((t) => t.id === 21048)?.priority;
    w.queueReadOnly.on = true;
    const queue = w.queueCalls.length;
    const res = await w.act("task.move", { task: 21048, to: "first" });
    // It read where the task stood, here, and was then refused the
    // change. Reading wrote nothing, so the laptop does it from the start.
    expect(w.queueCalls.length).toBeGreaterThan(queue);
    expect(await res.json()).toMatchObject({ ok: true, result: { task: 21048, priority: 10 } });
    expect(w.ran).toEqual([{ kind: "task.move", args: { task: 21048, to: "first" } }]);
    expect(w.queueTasks.find((t) => t.id === 21048)?.priority).toBe(before);
    // The board is still read here: it was the change that was refused.
    expect(
      ((await (await w.browse("/console/health", { token: null })).json()) as { refused: string[] }).refused,
    ).toEqual(["task.move"]);
    expect((await w.core()).doc.board.counts).toEqual(QUEUE_NOW);
  });

  test("a task is cancelled here with the machine away: the row, its lease and the event say so", async () => {
    const w = await primary();
    w.net.hostUp = false;
    w.net.edgeUp = false;
    const res = await w.act("task.cancel", { task: 21048, reason: "superseded" });
    expect(await res.json()).toEqual({ ok: true, result: { task: 21048, action: "cancel" } });
    const task = w.queueTasks.find((t) => t.id === 21048);
    expect(task?.status).toBe("cancelled");
    expect(task?.worker).toBeNull();
    expect(task?.lease).toBeNull();
    expect(task?.result).toEqual({ cancel_reason: "superseded" });
    // The machine was never asked: nothing held, nothing run there.
    expect(w.ran).toEqual([]);
    expect(w.hostSaw).toEqual([]);
    // The cancellation is an event, in the broker's words, and the
    // update only touches a row that had not already ended.
    expect(w.queueEvents).toEqual([
      {
        worker: "pm-console",
        task: 21048,
        kind: "cancelled",
        detail: { reason: "superseded", via: "cancel_repo_task_turso" },
        created: expect.any(String),
      },
    ]);
    expect(w.queueCalls.map((c) => c.sql)).toContain(
      "UPDATE tasks SET status = 'cancelled', worker_id = NULL, lease_until = NULL, result = ?, updated_at = ? WHERE id = ? AND queue = ? AND status NOT IN ('done', 'cancelled', 'failed')",
    );
    expect(w.queueCalls.some((c) => c.sql.startsWith("DELETE FROM leases"))).toBe(true);
    // And the Machines screen says the edge does this kind, and unblocking too.
    const machines = (await (await w.browse("/console/api/machines")).json()) as MachinesAnswer;
    expect(machines.platform?.actions).toContain("task.cancel");
    expect(machines.platform?.actions).toContain("task.unblock");
  });

  test("cancelling a claimed task bound to a session closes the binding window", async () => {
    const w = await primary();
    w.net.hostUp = false;
    const res = await w.act("task.cancel", { task: 21047, reason: "no longer needed" });
    expect(await res.json()).toEqual({ ok: true, result: { task: 21047, action: "cancel" } });
    const task = w.queueTasks.find((t) => t.id === 21047);
    expect(task?.status).toBe("cancelled");
    expect(task?.worker).toBeNull();
    expect(task?.lease).toBeNull();
    expect(task?.result).toEqual({ cancel_reason: "no longer needed" });
    expect(w.ran).toEqual([]);
    expect(w.queueEvents.map((e) => e.kind)).toEqual(["cancelled", "session_unbound"]);
    expect(w.queueEvents[1]?.detail).toEqual({
      task_id: 21047,
      queue: w.edgeConfig.queue,
      pool: w.edgeConfig.queue,
      platform: null,
      lane_id: null,
      model_provider: null,
      model_id: "muse",
      model_variant: null,
      session_id: "sess-21047",
      trigger: "cancel",
    });
  });

  test("a blocked task is unblocked here with the machine away, and its dep record is named as discarded", async () => {
    const w = await primary();
    w.net.hostUp = false;
    const deps = w.queueTasks.find((t) => t.id === 21049)?.blocked_deps;
    const res = await w.act("task.unblock", { task: 21049, reason: "the owner says so" });
    expect(await res.json()).toEqual({ ok: true, result: { task: 21049, action: "unblock" } });
    const task = w.queueTasks.find((t) => t.id === 21049);
    expect(task?.status).toBe("queued");
    expect(task?.blocked_deps).toBeNull();
    expect(w.ran).toEqual([]);
    expect(w.queueEvents).toHaveLength(1);
    expect(w.queueEvents[0]).toMatchObject({ worker: "pm-console", task: 21049, kind: "dep_unblock_override" });
    expect(w.queueEvents[0]?.detail).toEqual({
      task_id: 21049,
      queue: w.edgeConfig.queue,
      pool: w.edgeConfig.queue,
      platform: null,
      lane_id: null,
      model_provider: null,
      model_id: null,
      model_variant: null,
      session_id: null,
      reason: "the owner says so",
      discarded_deps: deps,
      via: "unblock_task_turso",
    });
    expect(w.queueCalls.map((c) => c.sql)).toContain(
      "UPDATE tasks SET status = 'queued', blocked_deps = NULL, dep_next_recheck_at = NULL, dep_blocked_until = NULL, dep_recheck_seconds = NULL, updated_at = ? WHERE id = ? AND queue = ? AND status = 'blocked'",
    );
  });

  test("cancelling and unblocking refuse what the broker refuses: no such task, a task that is not blocked, a quarantined packet", async () => {
    const w = await primary();
    w.net.hostUp = false;
    const missing = await w.act("task.cancel", { task: 99999, reason: "gone" });
    expect(missing.status).toBe(404);
    expect(((await missing.json()) as { error: string }).error).toContain("no such task");
    const notBlocked = await w.act("task.unblock", { task: 21048, reason: "nothing to release" });
    expect(notBlocked.status).toBe(409);
    expect(((await notBlocked.json()) as { error: string }).error).toContain("is queued, not blocked");
    // A task the claim path quarantined: refused, and told to repair it.
    const quarantined = w.queueTasks.find((t) => t.id === 21049);
    if (!quarantined) throw new Error("fixture lost 21049");
    quarantined.last_error = "quarantined by claim path: payload missing/empty (#5444)";
    quarantined.payload = {};
    const refused = await w.act("task.unblock", { task: 21049, reason: "let it go" });
    expect(refused.status).toBe(409);
    expect(((await refused.json()) as { error: string }).error).toContain("repair the packet first");
    expect(w.queueTasks.find((t) => t.id === 21049)?.status).toBe("blocked");
    // Nothing was written for any of the three.
    expect(w.queueEvents).toEqual([]);
    expect(w.queueCalls.every((c) => /^\s*SELECT/i.test(c.sql))).toBe(true);
    expect(w.ran).toEqual([]);
  });

  test("a blocked task whose packet was repaired unblocks here, though its last_error still says quarantined", async () => {
    const w = await primary();
    w.net.hostUp = false;
    const task = w.queueTasks.find((t) => t.id === 21049);
    if (!task) throw new Error("fixture lost 21049");
    task.last_error = "quarantined by claim path: payload missing/empty (#5444)";
    const packet = triagePacket(w.edgeConfig.repo, w.edgeConfig.queue, 301, "Ranked fresh", "now", "2026-11-22T11:30:00Z");
    task.payload = packet.payload;
    task.title = packet.title;
    task.body = packet.body;
    task.priority = packet.priority;
    task.dedupe_key = packet.dedupe_key;
    task.max_attempts = 1000;
    const res = await w.act("task.unblock", { task: 21049, reason: "repaired" });
    expect(await res.json()).toEqual({ ok: true, result: { task: 21049, action: "unblock" } });
    expect(w.queueTasks.find((t) => t.id === 21049)?.status).toBe("queued");
    expect(w.queueEvents.map((e) => e.kind)).toEqual(["dep_unblock_override"]);
  });

  test("cancelling a task that is already cancelled answers done and writes nothing", async () => {
    const w = await primary();
    w.net.hostUp = false;
    const first = await w.act("task.cancel", { task: 21048, reason: "twice" });
    expect(first.status).toBe(200);
    const calls = w.queueCalls.length;
    const events = w.queueEvents.length;
    const again = await w.act("task.cancel", { task: 21048, reason: "twice" });
    expect(await again.json()).toEqual({ ok: true, result: { task: 21048, action: "cancel" } });
    expect(w.queueEvents.length).toBe(events);
    // Only reads: the row, and the board read again after.
    expect(w.queueCalls.slice(calls).every((c) => /^\s*SELECT/i.test(c.sql))).toBe(true);
    expect(w.ran).toEqual([]);
  });

  test("turned away for calling too often is not refused for the hour: the laptop does it meanwhile, and this side tries again in five minutes", async () => {
    const w = await primary();
    const health = async () =>
      ((await (await w.browse("/console/health", { token: null })).json()) as { refused: string[] }).refused;
    const merges = () => w.githubCalls.filter((c) => c.method === "PUT" && c.url.endsWith("/pulls/55/merge")).length;
    w.ghLimits.push((method, url) => method === "PUT" && url.includes("/merge"));
    expect((await w.act("pr.merge", { pr: 55 })).status).toBe(200);
    expect(w.ran).toEqual([{ kind: "pr.merge", args: { pr: 55 } }]);
    expect(merges()).toBe(1);
    expect(await health()).toEqual(["pr.merge"]);
    // Within the five minutes it goes straight to the laptop.
    w.now.t += 4 * 60_000;
    await w.act("pr.merge", { pr: 55 });
    expect(merges()).toBe(1);
    expect(w.ran).toHaveLength(2);
    // After them this side tries again, and GitHub takes it.
    w.ghLimits.length = 0;
    w.now.t += 61_000;
    expect(await health()).toEqual([]);
    const again = await w.act("pr.merge", { pr: 55 });
    expect(await again.json()).toEqual({ ok: true, result: { number: 55, merged: true, sha: "5a5a5a5a5a5a" } });
    expect(w.ran).toHaveLength(2);
    // The same for what it reads: the ranked issues are searched for,
    // and search is what runs out first.
    await w.browse("/console/api/doc/ops");
    await settle(w);
    w.ghLimits.push((method, url) => method === "GET" && url.includes("/search/issues"));
    w.now.t += 181_000;
    await w.browse("/console/api/doc/ops");
    await settle(w);
    expect(await health()).toEqual(["read.priorities"]);
    w.now.t += 1_000;
    expect((await w.link.now.once())?.owns).toEqual(["board", "prs", "asks"]);
    w.ghLimits.length = 0;
    w.now.t += 4 * 60_000;
    expect(await health()).toEqual(["read.priorities"]);
    w.now.t += 61_000;
    expect(await health()).toEqual([]);
    await w.hostReads();
    expect((await w.link.now.once())?.owns).toEqual(["board", "prs", "priorities", "asks"]);
  });

  test("a section is judged by its own sources: the queue refusing does not hand the laptop what is read from GitHub", async () => {
    const w = await primary();
    await w.core();
    await w.browse("/console/api/doc/ops");
    await settle(w);
    // GitHub is slow with the list of pull requests, and will fail on
    // its own account. While that read is under way, the queue store
    // refuses this side's token.
    let letGo = () => {};
    const wait = new Promise<void>((r) => {
      letGo = r;
    });
    const list = (method: string, url: string) => method === "GET" && /\/pulls\?/.test(url);
    w.ghHolds.push({ match: list, wait });
    w.ghBreaks.push(list);
    w.queueRefuses.on = true;
    w.now.t += 200_000;
    await w.browse("/console/api/doc/ops"); // the list starts to be read, and waits
    await w.core(); // the board and the prompts are read, and refused
    await new Promise((r) => setTimeout(r, 5));
    expect(w.queueRefused.n).toBeGreaterThan(0);
    letGo();
    await settle(w);
    const refused = ((await (await w.browse("/console/health", { token: null })).json()) as { refused: string[] })
      .refused;
    // The board and the prompts are read from the queue store, and are
    // the laptop's now. The pull requests only failed, and the refusal
    // that came while they were being read was not theirs: still read here.
    expect(refused).toEqual(["read.asks", "read.board"]);
  });

  test("a write that got no answer while another action was refused: that one is not handed on either", async () => {
    const w = await primary();
    // A merge is refused, but GitHub is slow to say so ...
    let letGo = () => {};
    const wait = new Promise<void>((r) => {
      letGo = r;
    });
    const merge = (method: string, url: string) => method === "PUT" && url.includes("/merge");
    w.ghHolds.push({ match: merge, wait });
    w.ghRefuses.push(merge);
    const merging = w.act("pr.merge", { pr: 55 });
    await new Promise((r) => setTimeout(r, 5));
    // ... and meanwhile a deploy is started, and gets no answer at all:
    // it may have been taken.
    w.ghDrops.push((method, url) => method === "POST" && url.includes("/dispatches"));
    expect((await w.act("deploy.start", { what: "site" })).status).toBe(502);
    letGo();
    // What was written while the merge ran cannot be told from the
    // merge's own, so the merge is said as it failed, not done again
    // by the laptop. (It errs on that side only.)
    expect((await merging).status).toBe(502);
    expect(w.ran).toEqual([]);
  });

  test("a read GitHub refuses this side's token for is asked of the laptop", async () => {
    const w = await primary();
    w.ghRefuses.push((method, url) => method === "GET" && url.endsWith("/pulls/55"));
    await w.browse("/console/api/prs/55");
    expect(w.hostSaw).toEqual(["GET /console/api/prs/55"]);
    // With no laptop reporting there is nobody to ask, and it says what
    // is the matter: not that a laptop could not be reached.
    w.net.hostUp = false;
    w.net.edgeUp = false;
    w.now.t += 61_000;
    const res = await w.browse("/console/api/prs/55");
    expect(res.status).toBe(503);
    expect(((await res.json()) as { error: string }).error).toContain("turned away");
    expect(w.hostSaw).toHaveLength(1);
  });

  test("a screen that says what it could not read, refused here, is the laptop's while it is there", async () => {
    const w = await primary();
    // The deploys and builds: each list says for itself that it could
    // not be read, so nothing is thrown.
    w.ghRefuses.push((method, url) => method === "GET" && url.includes("/actions/workflows/"));
    await w.browse("/console/api/releases");
    expect(w.hostSaw).toEqual(["GET /console/api/releases"]);
    // A roadmap whose issues could not be searched for, the same.
    w.ghRefuses.length = 0;
    w.ghRefuses.push((method, url) => method === "GET" && url.includes("/search/issues"));
    await w.browse("/console/api/roadmap?area=pm-stack");
    expect(w.hostSaw).toEqual(["GET /console/api/releases", "GET /console/api/roadmap"]);
    // One that was read whole is answered here, as ever.
    w.ghRefuses.length = 0;
    expect((await w.browse("/console/api/releases")).status).toBe(200);
    expect(w.hostSaw).toHaveLength(2);
    // With no laptop reporting, what could be read is shown, with what
    // could not beside it: better than being told a laptop is away.
    w.ghRefuses.push((method, url) => method === "GET" && url.includes("pm-console-android.yml"));
    w.net.hostUp = false;
    w.net.edgeUp = false;
    w.now.t += 61_000;
    const res = await w.browse("/console/api/releases");
    expect(res.status).toBe(200);
    const body = (await res.json()) as { site: { id: number }[]; app: { error: string } };
    expect(body.site[0].id).toBe(4501);
    expect(body.app.error).toContain("Resource not accessible");
    expect(w.hostSaw).toHaveLength(2);
  });

  test("health says this side does the core, and how its own check of each kind of read went", async () => {
    const w = await primary();
    const health = (await (await w.browse("/console/health", { token: null })).json()) as Record<string, unknown>;
    expect(health.directEnabled).toBe(true);
    expect(health.githubTokenConfigured).toBe(true);
    expect(health.queueTokenConfigured).toBe(true);
    expect(health.primary).toBe(true);
    expect(health.directReady).toBe(true);
    const selfCheck = health.selfCheck as { at: number; kinds: Record<string, { ok: boolean }> };
    expect(selfCheck.at).toBe(START);
    expect(Object.keys(selfCheck.kinds).sort()).toEqual(["asks", "board", "priorities", "prs"]);
    expect(Object.values(selfCheck.kinds).every((k) => k.ok)).toBe(true);
    // A second health read reuses the stored self-check: no more reads.
    const calls = asked(w);
    await w.browse("/console/health", { token: null });
    expect(asked(w)).toBe(calls);
    // Nor an hour's worth of them: a check that read everything stands for the hour.
    w.now.t += 59 * 60_000;
    await w.browse("/console/health", { token: null });
    expect(((await (await w.browse("/console/health", { token: null })).json()) as typeof health).selfCheck).toEqual(
      selfCheck,
    );
  });

  test("a check in which something could not be read is made again in five minutes, not left standing for the hour", async () => {
    const w = await primary();
    const check = async () =>
      (
        (await (await w.browse("/console/health", { token: null })).json()) as {
          selfCheck: { at: number; kinds: Record<string, { ok: boolean; error?: string }> };
        }
      ).selfCheck;
    // The hour's check comes round while GitHub is answering "one too many" for pull requests.
    w.now.t += 61 * 60_000;
    w.ghLimits.push((method, url) => method === "GET" && /\/pulls\?/.test(url));
    const failed = await check();
    expect(failed.at).toBe(w.now.t);
    expect([failed.kinds.prs.ok, failed.kinds.board.ok]).toEqual([false, true]);
    // GitHub answers again. Asked a minute later, the failed check still stands ...
    w.ghLimits.length = 0;
    w.now.t += 60_000;
    expect((await check()).at).toBe(failed.at);
    // ... and five minutes after it was made, it is made again, and reads everything.
    w.now.t += 4 * 60_000 + 1;
    const again = await check();
    expect(again.at).toBe(w.now.t);
    expect(Object.values(again.kinds).every((kind) => kind.ok)).toBe(true);
  });

  test("without the switch, or without a token, nothing is read or done here: the laptop does it, or it is held", async () => {
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    expect((await w.link.now.once())?.owns ?? []).toEqual([]);
    expect((await w.core()).doc.board).toEqual(w.data.board);
    // In reach, the laptop does it.
    expect((await w.act("issue.comment", { issue: 14, text: "now" })).status).toBe(200);
    expect(w.ran).toEqual([{ kind: "issue.comment", args: { issue: 14, text: "now" } }]);
    // Out of reach, it is held.
    w.net.hostUp = false;
    w.net.edgeUp = false;
    const held = await w.act("issue.comment", { issue: 14, text: "late" });
    expect(await held.json()).toEqual({ ok: true, held: true, result: null });
    // And what the plan's screens read is the laptop's to answer.
    expect((await w.browse("/console/api/roadmap?area=fleet")).status).toBe(503);
    expect(w.githubCalls).toEqual([]);
    expect(w.queueCalls).toEqual([]);
    // Both tokens there and the switch off is off: the switch is what says so.
    clearJwksCache();
    const unswitched = await setup({ direct: true });
    unswitched.edge.now = new Edge({
      kv: {
        get: async (k) => unswitched.stored.get(k),
        put: async (k, v) => void unswitched.stored.set(k, v),
        delete: async (k) => void unswitched.stored.delete(k),
      },
      now: () => unswitched.now.t,
      fetchImpl: asThePlatformFetch(async (url: string | URL | Request) => {
        if (String(url) === ISSUER.jwksUri)
          return { ok: true, status: 200, json: async () => ({ keys: [unswitched.access.jwk] }) };
        throw new Error(`unexpected fetch ${String(url)}`);
      }) as never,
      direct: { enabled: false, githubToken: "gh-test", queueToken: "queue-test" },
    });
    await unswitched.hostReads();
    await unswitched.link.now.once();
    expect((await unswitched.core()).doc.edge).toEqual({ held: 0, primary: false });
    expect((await unswitched.core()).doc.board).toEqual(unswitched.data.board);
    expect(unswitched.githubCalls).toEqual([]);
    // The switch on, with a token missing, is the same as off.
    clearJwksCache();
    const half = await setup({ direct: true });
    half.edge.now = new Edge({
      kv: {
        get: async (k) => half.stored.get(k),
        put: async (k, v) => void half.stored.set(k, v),
        delete: async (k) => void half.stored.delete(k),
      },
      now: () => half.now.t,
      fetchImpl: asThePlatformFetch(async (url: string | URL | Request) => {
        if (String(url) === ISSUER.jwksUri)
          return { ok: true, status: 200, json: async () => ({ keys: [half.access.jwk] }) };
        throw new Error(`unexpected fetch ${String(url)}`);
      }) as never,
      direct: { enabled: true, githubToken: "gh-test" },
    });
    await half.hostReads();
    await half.link.now.once();
    const health = (await (await half.browse("/console/health", { token: null })).json()) as Record<string, unknown>;
    expect([health.directEnabled, health.githubTokenConfigured, health.queueTokenConfigured, health.primary]).toEqual([
      true,
      true,
      false,
      false,
    ]);
    expect((await half.core()).doc.board).toEqual(half.data.board);
    expect((await half.core()).doc.edge).toEqual({ held: 0, primary: false });
  });

  test("a laptop too old to say where the queue is: the app is served from its reports, and nothing is done here", async () => {
    const w = await setup({ direct: true });
    // What an older laptop reports: the same settings, without the
    // fields acting here needs. That shape still validates, with those empty.
    const legacy = w.edgeConfig as unknown as Record<string, unknown>;
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
    await w.hostReads();
    await w.link.now.once();
    // The laptop goes out of reach: the app is still served, from its copy.
    w.net.hostUp = false;
    w.net.edgeUp = false;
    const { res, doc } = await w.core();
    expect(res.status).toBe(200);
    expect(doc.board).toEqual(w.data.board);
    expect(doc.asks).toEqual([]);
    expect(doc.edge).toEqual({ held: 0, primary: false });
    expect(w.githubCalls).toEqual([]);
    expect(w.queueCalls).toEqual([]);
    const health = (await (await w.browse("/console/health", { token: null })).json()) as Record<string, unknown>;
    expect(health.configured).toBe(true);
    expect(health.primary).toBe(false);
    expect(health.directReady).toBe(false);
    // And an action is held for the laptop, not done here.
    expect(await (await w.act("issue.comment", { issue: 14, text: "late" })).json()).toEqual({
      ok: true,
      held: true,
      result: null,
    });
  });
});

describe("telling the owner, with no machine", () => {
  type World = Awaited<ReturnType<typeof setup>>;
  async function subscribed() {
    const w = await setup({ direct: true });
    const phone = await browser();
    await w.hostReads();
    await w.link.now.once();
    // The fleet's answer on #201 is already there when the device
    // subscribes: it is not news.
    expect((await w.act("push.on", { subscription: phone.subscription, label: "Pocket" })).status).toBe(200);
    expect(w.pushes).toHaveLength(0);
    w.hostSaw.length = 0;
    return { w, phone };
  }
  const reply = (w: World, issue: number, text: string) => {
    (w.ghComments[issue] ??= []).push({
      body: text,
      created_at: new Date(w.now.t).toISOString(),
      user: { type: "User" },
    });
    w.ghIssues[issue].comments = w.ghComments[issue].length;
    w.ghIssues[issue].updated_at = new Date(w.now.t).toISOString();
  };

  test("this side wakes itself each minute, reads the prompts, and tells a subscribed device of a reply", async () => {
    const { w, phone } = await subscribed();
    // Subscribing asked for a wake: there is someone to tell.
    expect(w.alarm.at).toBe(START + 1_000);
    // The laptop goes away for good. Nobody opens the app.
    w.net.hostUp = false;
    w.net.edgeUp = false;
    w.now.t = w.alarm.at;
    await w.edge.now.alarm();
    expect(w.pushes).toHaveLength(0);
    expect(w.alarm.at).toBe(w.now.t + 60_000);
    // The fleet answers on GitHub.
    w.now.t += 30_000;
    reply(w, 202, "Checked: the queue is moving.");
    w.now.t = w.alarm.at;
    await w.edge.now.alarm();
    expect(w.pushes).toHaveLength(1);
    expect(JSON.parse(await phone.open(w.pushes[0].body))).toMatchObject({
      title: "The fleet replied on #202",
      body: "Checked: the queue is moving.",
    });
    // Told once: the next wake finds nothing new, and asks for the one after.
    w.now.t = w.alarm.at;
    await w.edge.now.alarm();
    expect(w.pushes).toHaveLength(1);
    expect(w.alarm.at).toBe(w.now.t + 60_000);
    // No request reached the laptop for any of it.
    expect(w.hostSaw.filter((s) => !s.startsWith("GET /console/api/doc/"))).toEqual([]);
  });

  test("a wake in an object that has just started reads all the same: what was kept is the starting point, and only what changed is news", async () => {
    const { w, phone } = await subscribed();
    w.net.hostUp = false;
    w.net.edgeUp = false;
    const threads = (issue: number) =>
      w.githubCalls.filter((c) => c.method === "GET" && c.url.includes(`/issues/${issue}/comments`)).length;
    expect([threads(201), threads(202)]).toEqual([1, 0]);
    // Between wakes the object is put away, and the fleet answers on #202.
    w.now.t += 30_000;
    reply(w, 202, "Checked: the queue is moving.");
    w.now.t += 30_000;
    w.edge.now = w.makeEdge();
    await w.edge.now.alarm();
    // The answer on #201 was there before: not told again. The new one is.
    expect(w.pushes).toHaveLength(1);
    expect(JSON.parse(await phone.open(w.pushes[0].body)).title).toBe("The fleet replied on #202");
    // And only the thread that changed was fetched: what the kept list
    // said of the other still stands.
    expect([threads(201), threads(202)]).toEqual([1, 1]);
    expect(w.alarm.at).toBe(w.now.t + 60_000);
    // Put away again, with nothing new: woken, it reads the list and no thread.
    w.now.t = w.alarm.at;
    w.edge.now = w.makeEdge();
    const lists = () => w.githubCalls.filter((c) => c.url.includes("/issues?labels=owner%2Fprompt")).length;
    const before = lists();
    await w.edge.now.alarm();
    expect(lists() - before).toBe(1);
    expect([threads(201), threads(202)]).toEqual([1, 1]);
    expect(w.pushes).toHaveLength(1);
  });

  test("turned away while reading for the owner, the wakes do not end: it is tried again when its time is up", async () => {
    const { w, phone } = await subscribed();
    w.net.hostUp = false;
    w.net.edgeUp = false;
    w.now.t = w.alarm.at;
    await w.edge.now.alarm();
    // GitHub turns this side away for calling too often, as it reads the prompts.
    w.ghLimits.push((method, url) => method === "GET" && /\/issues\?/.test(url));
    w.now.t = w.alarm.at;
    await w.edge.now.alarm();
    const limitedAt = w.now.t;
    expect(w.pushes).toHaveLength(0);
    // Not a minute from now, when it would only be turned away again:
    // when the five minutes are up. Nobody is looking and no laptop
    // reports, so this wake is all that will notice.
    expect(w.alarm.at).toBe(limitedAt + 5 * 60_000 + 1_000);
    // Meanwhile the fleet answers.
    w.ghLimits.length = 0;
    w.now.t += 60_000;
    reply(w, 202, "Done while you were turned away.");
    w.now.t = w.alarm.at;
    await w.edge.now.alarm();
    expect(w.pushes).toHaveLength(1);
    expect(JSON.parse(await phone.open(w.pushes[0].body))).toMatchObject({ title: "The fleet replied on #202" });
    // And it is back to a wake a minute.
    expect(w.alarm.at).toBe(w.now.t + 60_000);
  });

  test("a look at the app starts the wakes again if the object has none asked for", async () => {
    const { w } = await subscribed();
    // A new object, as after being put away, with its wake lost.
    w.edge.now = w.makeEdge();
    w.alarm.at = 0;
    w.now.t += 5_000;
    await w.core();
    expect(w.alarm.at).toBe(w.now.t + 1_000);
    // With one asked for, a look does not move it.
    w.now.t += 500;
    await w.core();
    expect(w.alarm.at).toBe(w.now.t + 500);
  });

  test("with nobody to tell, or without the switch, it does not wake to read: only to notice a laptop gone quiet", async () => {
    const w = await setup({ direct: true });
    await w.hostReads();
    await w.link.now.once();
    expect(w.alarm.at).toBe(START + SILENT_MS + 1000);
    const calls = w.githubCalls.length;
    const due = w.alarm.at;
    w.now.t = due;
    w.net.hostUp = false;
    w.net.edgeUp = false;
    await w.edge.now.alarm();
    expect(w.githubCalls.length).toBe(calls);
    // The laptop's going quiet is noted, and with that there is nothing
    // left to wake for: no further wake is asked.
    expect(w.stored.get("silent")).toBe(true);
    expect(w.alarm.at).toBe(due);
    // Without the switch, a subscribed device changes nothing about that.
    clearJwksCache();
    const off = await setup();
    const phone = await browser();
    await off.hostReads();
    await off.link.now.once();
    await off.act("push.on", { subscription: phone.subscription, label: "Pocket" });
    expect(off.alarm.at).toBe(START + SILENT_MS + 1000);
    // Nor does being turned away ask for a wake when there is nobody
    // to tell: the next look, or the laptop's next report, tries again.
    clearJwksCache();
    const alone = await setup({ direct: true });
    await alone.hostReads();
    await alone.link.now.once();
    await alone.core();
    await Promise.all(alone.behind.splice(0));
    alone.ghLimits.push((method, url) => method === "GET" && /\/issues\?/.test(url));
    alone.now.t += 46_000;
    await alone.core();
    await Promise.all(alone.behind.splice(0));
    expect(
      ((await (await alone.browse("/console/health", { token: null })).json()) as { refused: string[] }).refused,
    ).toEqual(["read.asks"]);
    alone.now.t += 1_000;
    await alone.link.now.once();
    expect(alone.alarm.at).toBe(alone.now.t + SILENT_MS + 1000);
  });

  test("the laptop going quiet is told once, in words that say this side carries on; a telling that fails is tried again in a minute", async () => {
    const { w, phone } = await subscribed();
    w.net.hostUp = false;
    w.net.edgeUp = false;
    w.pushFailures.remaining = 1;
    w.now.t = START + SILENT_MS + 1000;
    await w.edge.now.alarm();
    // It could not be told: asked for again soon, not at a time gone by.
    expect(w.stored.get("silent")).toBeUndefined();
    expect(w.alarm.at).toBe(w.now.t + 60_000);
    const failed = w.pushes.length;
    w.now.t = w.alarm.at;
    await w.edge.now.alarm();
    expect(w.stored.get("silent")).toBe(true);
    const told = JSON.parse(await phone.open(w.pushes[failed].body)) as { title: string; body: string };
    expect(told.title).toBe("The laptop has gone quiet");
    expect(told.body).toContain("Cloudflare carries on");
    // Told once; the wakes go on, for the prompts.
    const count = w.pushes.length;
    w.now.t = w.alarm.at;
    await w.edge.now.alarm();
    expect(w.pushes).toHaveLength(count);
    expect(w.alarm.at).toBe(w.now.t + 60_000);
  });

  test("a laptop's report does not make this side read GitHub again: only its own intervals and wakes do", async () => {
    const { w } = await subscribed();
    const calls = w.githubCalls.length;
    // Six reports in half a minute, each with a unit's state changed,
    // which is something the owner is told about.
    for (let i = 0; i < 6; i++) {
      w.now.t += 5_000;
      (w.data.services as unknown[]).length = 0;
      (w.data.services as unknown[]).push({ unit: `u${i}.service`, load: "loaded", active: "active", sub: "running" });
      await w.hostReads();
      await w.link.now.once();
    }
    expect(w.githubCalls.length).toBe(calls);
  });
});

test("what a machine may say of itself: a name fit to show, and lists of kinds", () => {
  expect(
    machineReport({ name: "nixos", actions: ["task.move", "ask.send", "task.move"], sections: ["board"] }),
  ).toEqual({
    name: "nixos",
    actions: ["ask.send", "task.move"],
    sections: ["board"],
  });
  expect(machineReport({ name: "Drew's laptop", actions: [], sections: [] })).toBeNull();
  expect(machineReport({ name: "amd.local", actions: [], sections: ["questionCount"] })).toEqual({
    name: "amd.local",
    actions: [],
    sections: ["questionCount"],
  });
  for (const bad of [
    null,
    "nixos",
    { name: "nixos" },
    { name: "nixos", actions: "task.move", sections: [] },
    { name: "nixos", actions: ["Task Move"], sections: [] },
    { name: "nixos", actions: [7], sections: [] },
    { name: "x".repeat(64), actions: [], sections: [] },
    { name: " nixos", actions: [], sections: [] },
    { name: "nixos", actions: Array.from({ length: 201 }, (_, i) => `a.k${i}`), sections: [] },
  ]) {
    expect([bad, machineReport(bad)]).toEqual([bad, null]);
  }
  // A name from whatever the machine is called locally.
  expect(machineName("nixos")).toBe("nixos");
  expect(machineName("  Drew's laptop ")).toBe("Drew-s laptop");
  expect(machineName("amd.lan")).toBe("amd.lan");
  expect(machineName("--weird__")).toBe("weird__");
  expect(machineName("x".repeat(100))).toBe("x".repeat(63));
  for (const none of ["", "   ", "!!!", undefined, null]) expect([none, machineName(none)]).toEqual([none, ""]);
});
