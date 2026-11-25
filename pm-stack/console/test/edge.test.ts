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
import { DIRECT_ACTIONS, queueWrites } from "../edge/src/direct.ts";
import { Edge, type KV, ownerSettingsRecord, SILENT_MS } from "../edge/src/edge.ts";
import { AUDIT_KEPT } from "../edge/src/edge-shared.ts";
import { forgetPoke, type Io, route } from "../edge/src/entry.ts";
import type { Action } from "../src/actions.ts";
import { clearJwksCache, identify } from "../src/auth.ts";
import { readConfig } from "../src/config.ts";
import { platformSettings } from "../src/settings.ts";
import { Devices, Limiter } from "../src/devices.ts";
import { Docs } from "../src/docs.ts";
import { NEEDS_LABELS } from "../src/needs.ts";
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
import { Ops } from "../src/ops.ts";
import { type Deps, handle, heldRunner, marked } from "../src/server.ts";
import { memoryStore } from "../src/store.ts";
import { EdgeLink } from "../src/sync.ts";
import { TASKS_BY_DAY_SQL } from "../src/trends.ts";
import { DONE_COUNT_SQL, DONE_KINDS_SQL, DONE_TASKS_SQL } from "../src/work.ts";
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

/** A collection kept one record per key (#14960), read back as the
 * list its index names, in order. A world no object has read yet may
 * still hold the old whole-array value, which is the list itself. */
const storedList = <T>(stored: Map<string, unknown>, name: string, entry: (id: string) => string): T[] => {
  const index = stored.get(`${name}:index`);
  if (Array.isArray(index)) {
    return index.flatMap((id) => {
      if (typeof id !== "string") return [];
      const value = stored.get(entry(id));
      return value === undefined ? [] : [value as T];
    });
  }
  return (stored.get(name) as T[] | undefined) ?? [];
};
const heldStored = (w: { stored: Map<string, unknown> }) => storedList(w.stored, "held", (id) => `held:${id}`);
const takenStored = (w: { stored: Map<string, unknown> }) => storedList(w.stored, "taken", (id) => `taken:${id}`);
/** The handed names as storage holds them (#14960). */
const handedStored = (w: { stored: Map<string, unknown> }): string[] => {
  const index = w.stored.get("handed:index");
  if (Array.isArray(index)) return index.filter((id): id is string => typeof id === "string");
  return (w.stored.get("handed") as string[] | undefined) ?? [];
};
/** The audit log as its entries are kept (#14960): one per key under
 * its number, the kept window behind the next number. */
const auditStored = (w: { stored: Map<string, unknown> }): unknown[] => {
  const next = w.stored.get("audit:next");
  if (typeof next === "number") {
    const entries: unknown[] = [];
    for (let seq = Math.max(0, next - AUDIT_KEPT); seq < next; seq++) entries.push(w.stored.get(`audit:${seq}`));
    return entries;
  }
  return (w.stored.get("audit") as unknown[] | undefined) ?? [];
};
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
  "done",
  "agents",
  "services",
  "more",
];

async function setup(options: { direct?: boolean; kinds?: readonly string[]; bridge?: boolean } = {}) {
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
  const githubCalls: { method: string; url: string; body?: string; unchanged?: true }[] = [];
  const queueCalls: { sql: string; args: (string | number)[] }[] = [];
  /** Calls the edge made of anywhere that is neither GitHub nor the
   * queue store main names, and whether each carried a token. */
  const strays: { url: string; token: boolean }[] = [];
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
  /** What each repository's list of open issues holds that waits on the
   * owner, as GitHub gives it: issues and pull requests carrying
   * `user/review` or `user/escalated`. A repository taken out of here
   * is one this token cannot see. */
  const ghNeeds: Record<
    string,
    {
      number: number;
      title: string;
      labels: string[];
      body?: string | null;
      pr?: boolean;
      updated_at?: string;
      state?: string;
    }[]
  > = Object.fromEntries(cfg.needsRepos.map((repo) => [repo, []]));
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
  /** What GitHub says with such an answer of when to ask again, if a
   * test has it say anything. */
  const ghLimitSays: { headers: Record<string, string> } = { headers: {} };
  /** GitHub tags what it answers a read with, and answers "not
   * modified" to one asked for by the tag of what it would say. On
   * for every test, as it is at GitHub; `unchanged` counts those. */
  const ghTags = { on: true, unchanged: 0 };
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
  /** What GitHub lists as closed, most lately touched first. */
  const ghClosed: Record<string, unknown>[] = [];
  /** Tasks the queue's events say were finished, newest first, as its store answers for them. */
  const queueDone: { id: number; title: string; at: string; issue: number | null; pr: number | null }[] = [];
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
    if (sql === TASKS_BY_DAY_SQL) {
      // Finished tasks by day, from the same finished tasks the Done screen is given.
      const days = new Map<string, number>();
      for (const t of queueDone) {
        if (`${t.at}Z` >= String(args[0])) days.set(t.at.slice(0, 10), (days.get(t.at.slice(0, 10)) ?? 0) + 1);
      }
      return {
        cols: cols(["day", "kind", "n"]),
        rows: rows(
          [...days.entries()].sort().map(([day, n]) => [{ value: day }, { value: "small fix" }, { value: n }]),
        ),
      };
    }
    if (sql === DONE_TASKS_SQL) {
      return {
        cols: cols(["id", "title", "at", "lane", "model", "kind", "issue", "pr", "verdict"]),
        rows: rows(
          queueDone.map((t) => [
            { value: t.id },
            { value: t.title },
            { value: t.at },
            { value: null },
            { value: null },
            { value: null },
            { value: t.issue },
            { value: t.pr },
            { value: null },
          ]),
        ),
      };
    }
    if (sql === DONE_COUNT_SQL) return { cols: cols(["n"]), rows: rows([[{ value: queueDone.length }]]) };
    if (sql === DONE_KINDS_SQL) {
      // None of the stand-in's finished tasks says what kind it is.
      return {
        cols: cols(["kind", "n"]),
        rows: rows(queueDone.length ? [[{ value: "" }, { value: queueDone.length }]] : []),
      };
    }
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
        // By the kind of reason, as the board asks: every one here has a reason written on it.
        cols: cols(["kind", "n"]),
        rows: rows([[{ value: "reason" }, { value: queueTasks.filter((t) => t.status === "blocked").length }]]),
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
    // Any repository's open issues and pull requests carrying one of
    // the labels that say something waits on the owner, a page at a time.
    const listed = /^\/repos\/([^/]+\/[^/]+)\/issues$/.exec(u.pathname);
    const wanted = u.searchParams.get("labels");
    if (method === "GET" && listed && wanted !== null && (NEEDS_LABELS as readonly string[]).includes(wanted)) {
      const repo = listed[1];
      const all = ghNeeds[repo];
      if (!all) return fail(404, { message: "Not Found" });
      const per = Number(u.searchParams.get("per_page") ?? 30);
      const page = Number(u.searchParams.get("page") ?? 1);
      return ok(
        all
          .filter((i) => i.labels.includes(wanted))
          .slice((page - 1) * per, page * per)
          .map((i) => ({
            number: i.number,
            title: i.title,
            body: i.body ?? null,
            state: i.state ?? "open",
            html_url: `https://github.com/${repo}/${i.pr ? "pull" : "issues"}/${i.number}`,
            updated_at: i.updated_at ?? "2026-11-22T12:00:00Z",
            labels: i.labels.map((name) => ({ name })),
            ...(i.pr ? { pull_request: { url: "x" } } : {}),
          })),
      );
    }
    if (method === "GET" && path === "pulls") return ok(ghPrs);
    if (method === "GET" && u.pathname === "/search/issues")
      return ok({ items: ghSearch, total_count: ghSearch.length });
    // What is closed is a list of its own.
    if (method === "GET" && path === "issues" && u.searchParams.get("state") === "closed") return ok(ghClosed);
    // Everything changed at or after a time, oldest change first, a
    // page at a time: what the history behind the Trends screen asks.
    if (method === "GET" && path === "issues" && u.searchParams.get("state") === "all" && u.searchParams.has("since")) {
      const since = u.searchParams.get("since") ?? "";
      const per = Math.min(100, Number(u.searchParams.get("per_page") ?? 30));
      const changed = (i: Record<string, unknown>) => String(i.updated_at ?? "");
      return ok(
        (ghClosed as Record<string, unknown>[])
          .map((i): Record<string, unknown> => ({ state: "closed", created_at: i.closed_at, ...i }))
          .filter((i) => changed(i) >= since)
          .sort((a, b) => changed(a).localeCompare(changed(b)) || Number(a.number) - Number(b.number))
          .slice(0, per),
      );
    }
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
      // As GitHub answers for one issue: its title with the rest.
      return ok({
        number: n,
        ...(ghIssues[n] ? { title: ghIssues[n].title } : {}),
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
      const label = decodeURIComponent(labelMatch[2]);
      // One of the labels that say something waits on the owner: it is
      // on the item where the lists above are read from. Taken off one
      // that does not carry it, GitHub says so (its own words, read
      // from it on 2026-11-24).
      if ((NEEDS_LABELS as readonly string[]).includes(label)) {
        const item = (ghNeeds["developing-today/code"] ?? []).find((i) => i.number === n);
        if (!item?.labels.includes(label)) return fail(404, { message: "Label does not exist", status: "404" });
        item.labels = item.labels.filter((l) => l !== label);
        return ok([]);
      }
      ghLabels[n] = ghLabels[n].filter((l) => l !== label);
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
  /** The bridge, as the edge reaches it over its service binding
   * (#14373): every call recorded, and the agents and the message log
   * it answers with. `bridgeRefuses` makes its next answer the plain
   * 404 of a credential the bridge no longer knows. */
  const bridgeCalls: { method: string; url: string; body?: string }[] = [];
  const bridgeAgents = [
    { name: "lane-1", groups: ["muse"], first_seen: "2026-11-24T00:00:00Z", last_seen: "2026-11-24T01:00:00Z" },
  ];
  const bridgeLog = [
    { id: 3, agent: "lane-1", direction: "inbound", kind: "message", sender: "service:pm-stack", body: "earlier" },
  ];
  // The unified views the console's Messages and Agents screens read
  // (#14374): one row per surface with its real sender, the unified
  // directory (a notes-only identity included), and credential state.
  const bridgePipeline = [
    {
      surface: "connector",
      id: 11,
      kind: "instruction",
      sender_addr: "user:drew",
      sender_label: "drew",
      receiver: "claude",
      direction: "inbound",
      read_at: null,
      body: "owner instruction",
    },
    {
      surface: "fleet",
      id: 12,
      kind: "command",
      sender_addr: "agent:lane-1",
      sender_label: "lane-1",
      receiver: "agent:lane-2",
      direction: "inbound",
      read_at: null,
      body: "agent command",
    },
    {
      surface: "notes",
      id: 13,
      kind: "note",
      sender_addr: "user:drew",
      sender_label: "drew",
      receiver: "muse/lane-1",
      direction: "inbound",
      read_at: null,
      body: "a note",
    },
  ];
  const bridgeDirectory = [
    {
      surface: "fleet",
      name: "lane-1",
      groups: ["muse"],
      presence: "last_seen",
      first_seen: "2026-11-24T00:00:00Z",
      last_seen: "2026-11-24T01:00:00Z",
      unread: 1,
    },
    {
      surface: "notes",
      name: "muse/scout-1",
      identity: "muse/scout-1",
      scopes: { can_send: true, can_inbox: true },
      presence: "none",
      first_seen: "2026-11-24T00:00:00Z",
      last_seen: null,
    },
  ];
  const bridgeCredentials = [
    {
      agent: "lane-1",
      state: "active",
      scopes: ["inbox", "send"],
      addr: null,
      credentials: 1,
      live: 1,
      created: "2026-11-24T00:00:00Z",
      revoked_at: null,
    },
  ];
  const bridgeEnrollTokens = [
    {
      agent: "new-1",
      kind: "muse",
      groups: ["muse"],
      notes_identity: null,
      created: "2026-11-24T00:00:00Z",
      expires_at: "2026-11-25T00:00:00Z",
      used_at: null,
      revoked_at: null,
      state: "pending",
    },
  ];
  const bridgeRefuses = { on: false };
  const bridgeFetch = (async (
    url: string,
    init: { method?: string; headers?: Record<string, string>; body?: string },
  ) => {
    const u = String(url);
    const method = init?.method ?? "GET";
    bridgeCalls.push({ method, url: u, body: init?.body });
    if (bridgeRefuses.on) return { ok: false, status: 404, json: async () => JSON.parse("") };
    if (method === "POST" && u.endsWith("/send")) {
      // A send's answer is its match report (#14375).
      return {
        ok: true,
        status: 201,
        json: async () => ({
          batch: "b1",
          messages: [],
          unregistered: [],
          matched: 1,
          delivered: 1,
          recipients: ["lane-1"],
          remaining: 0,
        }),
      };
    }
    if (method === "GET" && u.endsWith("/agents")) {
      return { ok: true, status: 200, json: async () => ({ agents: bridgeAgents }) };
    }
    if (method === "GET" && u.includes("/messages")) {
      return { ok: true, status: 200, json: async () => ({ messages: bridgeLog }) };
    }
    if (method === "GET" && u.includes("/pipeline")) {
      return { ok: true, status: 200, json: async () => ({ messages: bridgePipeline }) };
    }
    if (method === "GET" && u.endsWith("/directory")) {
      return { ok: true, status: 200, json: async () => ({ agents: bridgeDirectory }) };
    }
    if (method === "GET" && u.endsWith("/credentials")) {
      return { ok: true, status: 200, json: async () => ({ credentials: bridgeCredentials }) };
    }
    if (method === "GET" && u.endsWith("/enroll-tokens")) {
      return { ok: true, status: 200, json: async () => ({ enroll_tokens: bridgeEnrollTokens }) };
    }
    throw new Error(`unexpected bridge fetch ${method} ${u}`);
  }) as never;
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
      const call: (typeof githubCalls)[number] = { method, url: u, body: init?.body };
      githubCalls.push(call);
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
          headers: new Headers(ghLimitSays.headers),
          json: async () => ({ message: "API rate limit exceeded for user ID 1.", status: "403" }),
        };
      }
      if (ghBreaks.some((breaks) => breaks(method, u))) {
        return { ok: false, status: 500, json: async () => ({ message: "Server Error" }) };
      }
      const says = ghSays.find((say) => say.match(method, u));
      if (says) return { ok: false, status: says.status, json: async () => ({ message: says.message }) };
      const answer = githubAnswer(method, u, init?.body);
      if (method !== "GET" || !answer.ok || !ghTags.on) return answer;
      const etag = `W/"${Bun.hash(JSON.stringify(await answer.json())).toString(16)}"`;
      if (init?.headers?.["if-none-match"] === etag) {
        ghTags.unchanged++;
        call.unchanged = true;
        return {
          ok: false,
          status: 304,
          headers: new Headers({ etag }),
          // As a real answer with no body does.
          json: async () => JSON.parse(""),
        };
      }
      return { ...answer, headers: new Headers({ etag }) };
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
    strays.push({ url: u, token: Boolean(init?.headers?.authorization ?? init?.headers?.Authorization) });
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
  /** The platform's live connections, standing in: who is connected
   * (each by what it was sent), as the platform holds them whether or
   * not the object is in memory. */
  const live: { open: string[][]; refuses: boolean } = { open: [], refuses: false };
  const sockets = {
    accept: (_req: Request) => {
      live.open.push([]);
      // What the platform answers an upgrade with (its 101 cannot be made outside it).
      return new Response(null, { status: 200, headers: { "x-live": "accepted" } });
    },
    send: (text: string) => {
      for (const got of live.open) got.push(text);
      return live.open.length;
    },
    count: () => live.open.length,
  };
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
            ...(options.bridge ? { bridgeToken: "pm-cred-test", bridgeFetch } : {}),
            fetchImpl: asThePlatformFetch(directFetch) as typeof directFetch,
            ...(options.kinds ? { kinds: options.kinds } : {}),
          }
        : undefined,
      setAlarm: async (at) => {
        alarm.at = at;
      },
      getAlarm: async () => alarm.at || null,
      sockets,
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
    done: {
      tasks: { day: 0, kinds: [], latest: [] },
      merged: { day: 0, more: false, latest: [] },
      closed: { day: 0, more: false, latest: [] },
    },
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
        await gate.wait;
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
    "hands.done": {
      run: async (a) => {
        ran.push({ kind: "hands.done", args: a });
        return { ref: a.ref, done: true };
      },
      touch: ["census"],
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
    /** The host can be reached. When it cannot, what answers in its
     * place says so (the tunnel is down), unless `silent`. */
    hostUp: boolean;
    edgeUp: boolean;
    loseAnswer: boolean;
    giveUpEarly: boolean;
    /** Out of reach with nothing to say so: asking it only runs out of time. */
    silent: boolean;
    /** The host does the work and answers, and its answer cannot be read. */
    garble: boolean;
    /** A report of more than this many bytes does not get through: it
     * is cut off on the way ("lost", a slow connection running out of
     * time) or turned away by something in front of the edge ("refused"). */
    large: { over: number; fate: "lost" | "refused" } | null;
  } = { hostUp: true, edgeUp: true, loseAnswer: false, giveUpEarly: false, silent: false, garble: false, large: null };
  /** The size of each report the host sent, whether or not it arrived. */
  const reportSizes: number[] = [];
  /** Each report the host sent, as it read. */
  const reports: { sections: { name: string }[]; have: Record<string, unknown> }[] = [];
  /** The signed Android build on the host, if any, and how often it was read to be sent. */
  const build: {
    bytes: Uint8Array<ArrayBuffer> | null;
    at: number;
    reads: number;
    corrupt: boolean;
    unreadable: boolean;
  } = {
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
  const askHost = async (r: Request): Promise<Response | null> => {
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
    if (net.garble) {
      const said = await answer;
      return new Response("{", { status: said.status, headers: said.headers });
    }
    return answer;
  };
  const io: Io = {
    state: (r) => edge.now.fetch(r),
    // The platform's file store tags each file by its content; the icon
    // here has none, as a store that sent no tag would not.
    asset: async (path) =>
      files.has(path)
        ? new Response(files.get(path), path.startsWith("/icons/") ? {} : { headers: { ETag: `"tag-of-${path}"` } })
        : new Response("", { status: 404 }),
    host: askHost,
    // Asked to do something: a host that cannot be reached is said to
    // be, as Cloudflare says of a tunnel with no connection.
    act: async (r) => (!net.hostUp && !net.silent ? "unreached" : askHost(r)),
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
    ghLimitSays,
    ghTags,
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
    bridgeCalls,
    bridgeAgents,
    bridgeLog,
    bridgeRefuses,
    githubCalls,
    queueCalls,
    strays,
    live,
    ghIssues,
    ghComments,
    ghLabels,
    ghNeeds,
    ghClosed,
    queueDone,
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
      github: { answers: 0, unchanged: 0, since: w.now.t, kept: 0, quiet: { core: null, search: null } },
      hostSeen: null,
      hostVersion: null,
      listening: 0,
      machineSettingsDiffer: null,
      androidBuild: null,
      androidRelease: null,
      androidReleaseTrouble: null,
      answered: 0,
      deferred: 0,
      version: "test",
      woke: w.now.t,
      clock: { wanted: false, next: null },
      held: { either: 0, machine: 0, asked: 0 },
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

  // The platform's own settings are the repository's (src/settings.ts),
  // and the Worker carries them. A machine tells the ones it runs with
  // in every report; one on other code, or with another environment,
  // tells others. Which those are is said to anyone who asks, by name
  // only: no value of any setting is public.
  test("the edge says in which of the platform's settings the machine runs with others, by name only", async () => {
    const w = await setup();
    const differ = async () =>
      ((await (await w.browse("/console/health", { token: null })).json()) as Record<string, unknown>)
        .machineSettingsDiffer;
    // No machine has said anything yet: nothing to compare.
    expect(await differ()).toBeNull();
    await w.hostReads();
    await w.link.now.once();
    // The machine runs main's console with nothing set otherwise.
    expect(await differ()).toEqual([]);
    // A machine on older code: its list of repositories is the one main had then.
    const mains = [...w.edgeConfig.needsRepos];
    w.edgeConfig.needsRepos = ["developing-today/code", "developing-today/opencode"];
    w.edgeConfig.queue = "a-queue-of-its-own";
    w.now.t += 5_000;
    await w.link.now.once();
    expect(await differ()).toEqual(["queue", "needsRepos"]);
    const page = await (await w.browse("/console/health", { token: null })).text();
    expect(page).not.toContain("a-queue-of-its-own");
    expect(page).not.toContain("developing-today/opencode");
    // It is kept with the settings, across the edge's own restarts.
    w.edge.now = w.makeEdge();
    expect(await differ()).toEqual(["queue", "needsRepos"]);
    // The machine follows main again.
    w.edgeConfig.needsRepos = mains;
    w.edgeConfig.queue = platformSettings().queue;
    w.now.t += 5_000;
    await w.link.now.once();
    expect(await differ()).toEqual([]);
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
    for (const [, value] of pieces) {
      const text = typeof value === "string" ? value : (value as { t: string }).t;
      expect(text.length).toBeLessThanOrEqual(600_000);
      if (typeof value !== "string") expect(value).toMatchObject({ n: "questions" });
    }
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

  test("someone looking survives the object being put away", async () => {
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    await w.core();
    expect(w.stored.get("docAt")).toBe(w.now.t);
    // A new object over the same storage, as after being put away idle:
    // it has seen no document read, but the host is still told someone
    // is looking.
    w.edge.now = w.makeEdge();
    w.now.t += 1_000;
    expect((await w.link.now.once())?.watching).toBe(true);
    // And after long enough with no read, it is not.
    w.now.t += 91_000;
    w.edge.now = w.makeEdge();
    expect((await w.link.now.once())?.watching).toBe(false);
  });

  test("a section kept in pieces that is not one section's is no copy", async () => {
    const w = await setup();
    const big = { open: Array.from({ length: 4000 }, (_, i) => ({ qid: `q${i}`, question: "é".repeat(400) })) };
    w.data.questions = big;
    await w.hostReads();
    await w.link.now.once();
    const meta = w.stored.get("m:questions") as { hash: string; chunks: number };
    expect(meta.chunks).toBeGreaterThan(2);
    // A piece from another write under the same index is not taken for
    // this section's: the copy is asked for again.
    w.stored.set("m:questions:0", { n: "questions", h: "other", t: "x" });
    w.edge.now = w.makeEdge();
    const res = await w.browse("/console/api/doc/questions");
    expect(res.status).toBe(200);
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
    expect(heldStored(w)).toEqual([]);
  });

  test("with the host out of reach, what may be done late is held, done once when it is back, and the rest is refused", async () => {
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    w.net.hostUp = false;
    w.net.edgeUp = false;

    const held = await w.act("ask.send", { text: "Do this when you are back" });
    expect(await held.json()).toEqual({ ok: true, held: true, result: null, waits: "machine" });
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
    const ids = (heldStored(w) as { id: string }[]).map((h) => h.id);
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
    expect(await res.json()).toEqual({ ok: true, held: true, result: null, waits: "asked" });
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
    expect(await second.json()).toEqual({ ok: true, held: true, result: null, waits: "machine" });
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
          "hands.done",
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
      ["nixos", 10, SECTION_NAMES.length],
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
      { error: "That is no longer waiting: it was done or is being done, or was taken back already." },
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
      waits: "machine",
    });
    expect(await (await w.act("ask.send", { text: "kept once" }, id)).json()).toEqual({
      ok: true,
      held: true,
      result: null,
      again: true,
      waits: "machine",
    });
    expect((heldStored(w) as { id: string }[]).map((h) => h.id)).toEqual([id]);
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
    expect(await res.json()).toEqual({ ok: true, held: true, result: null, waits: "asked" });
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
    expect((heldStored(w) as unknown[]).length).toBe(100);
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
    for (let i = 0; i < pieces; i++) {
      const kept = w.stored.get(`m:questions:${i}`) as { t: string } | string | undefined;
      text += typeof kept === "string" ? kept : (kept?.t ?? "");
    }
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

  test("the object put away between requests, as it is in service: GitHub is still asked for the release now and then, not by every object that starts", async () => {
    const w = await setup();
    await w.hostReads();
    await report(w);
    /** Another object over the same storage answers, as after every idle spell. */
    const ask = async () => {
      w.edge.now = w.makeEdge();
      return ((await (await w.browse("/console/api/android")).json()) as Record<string, any>).release;
    };
    const trouble = async () => {
      w.edge.now = w.makeEdge();
      return ((await (await w.browse("/console/health", { token: null })).json()) as Record<string, unknown>)
        .androidReleaseTrouble;
    };
    // Nothing published. Asked once, however many objects are asked.
    for (let i = 0; i < 5; i++) expect(await ask()).toBeNull();
    expect(w.github.asked).toEqual(["version.json"]);
    // A release goes up, and is seen ten minutes on; then known to every object without asking.
    w.github.apk = apk(9, 450_000);
    w.github.version = await described(w.github.apk, "2026.11.23-a3a547e", 21755935);
    w.now.t += 9 * 60_000;
    expect(await ask()).toBeNull();
    w.now.t += 60_000;
    for (let i = 0; i < 5; i++) expect((await ask())?.name).toBe("2026.11.23-a3a547e");
    expect(w.github.asked).toEqual(["version.json", "version.json"]);
    // GitHub out of reach: tried each minute, not by each object, and each says why.
    w.github.down = true;
    w.now.t += 10 * 60_000;
    for (let i = 0; i < 5; i++) expect((await ask())?.name).toBe("2026.11.23-a3a547e");
    expect(w.github.asked.length).toBe(3);
    expect(await trouble()).toBe("github: connection reset");
    expect(w.github.asked.length).toBe(3);
    w.now.t += 60_000;
    expect((await ask())?.name).toBe("2026.11.23-a3a547e");
    expect(w.github.asked.length).toBe(4);
    // A description found out of step with its file is asked for again by the next object,
    // not trusted by it for ten minutes.
    w.github.down = false;
    w.now.t += 60_000;
    w.github.version = await described(apk(8, 440_000), "2026.11.22-0000000", 21000000);
    expect((await ask())?.name).toBe("2026.11.22-0000000");
    const askedBy = w.github.asked.filter((file) => file === "version.json").length;
    w.build.bytes = apk(1);
    w.build.at = w.now.t;
    await report(w);
    await report(w);
    w.edge.now = w.makeEdge();
    expect((await download(w)).bytes).toEqual(apk(1));
    expect(w.stored.has("released")).toBe(false);
    w.github.version = await described(w.github.apk, "2026.11.23-a3a547e", 21755935);
    expect((await ask())?.name).toBe("2026.11.23-a3a547e");
    expect(w.github.asked.filter((file) => file === "version.json").length).toBe(askedBy + 1);
    // What storage holds is held to what could have been put there. A time yet to come would
    // never fall due, in one object or the next: what it is stored with is asked for again at
    // once. Something that is not this at all is nothing known.
    const versionAsks = () => w.github.asked.filter((file) => file === "version.json").length;
    w.stored.set("released", { at: w.now.t + 400 * 24 * 60 * 60_000, asked: true, found: null, trouble: null });
    const before = versionAsks();
    expect((await ask())?.name).toBe("2026.11.23-a3a547e");
    expect(versionAsks()).toBe(before + 1);
    // (And what was asked for then is stored with the time it was asked at.)
    expect((w.stored.get("released") as { at: number }).at).toBe(w.now.t);
    // With GitHub out of reach, what such a record described still stands while it is asked for.
    const known = w.stored.get("released") as { found: unknown };
    w.stored.set("released", { ...known, at: w.now.t + 60_000 });
    w.github.down = true;
    expect((await ask())?.name).toBe("2026.11.23-a3a547e");
    expect(versionAsks()).toBe(before + 2);
    w.github.down = false;
    w.now.t += 60_000;
    for (const odd of [7, "x", { at: "soon", asked: true }, { at: w.now.t, asked: "yes" }, null]) {
      w.stored.set("released", odd);
      const asks = versionAsks();
      expect([odd, (await ask())?.name]).toEqual([odd, "2026.11.23-a3a547e"]);
      expect([odd, versionAsks()]).toEqual([odd, asks + 1]);
    }
    // A description in storage that is not one is no release, whatever else the record says.
    w.stored.set("released", { at: w.now.t, asked: true, found: { name: "../../etc", code: 1 }, trouble: 7 });
    expect(await ask()).toBeNull();
    expect(await trouble()).toBeNull();
    // Storage that cannot take it: GitHub is asked by each object, and the answer still given.
    w.stored.delete("released");
    w.storeFails.add("released");
    const failing = versionAsks();
    expect((await ask())?.name).toBe("2026.11.23-a3a547e");
    expect((await ask())?.name).toBe("2026.11.23-a3a547e");
    expect(versionAsks()).toBe(failing + 2);
    // One object asked twice still asks GitHub once: what it knows it has in mind as well.
    const android = async () =>
      ((await (await w.browse("/console/api/android")).json()) as Record<string, any>).release;
    expect([(await android())?.name, (await android())?.name]).toEqual(["2026.11.23-a3a547e", "2026.11.23-a3a547e"]);
    expect(versionAsks()).toBe(failing + 2);
    w.edge.now = w.makeEdge();
    expect([(await android())?.name, (await android())?.name]).toEqual(["2026.11.23-a3a547e", "2026.11.23-a3a547e"]);
    expect(versionAsks()).toBe(failing + 3);
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

  test("the key-fetch gate survives the object being put away", async () => {
    const w = await setup();
    const firstPair = await newKeyPair();
    const body = new TextEncoder().encode(JSON.stringify({ v: 1, at: w.now.t, sections: [], have: {}, acks: [] }));
    const first = new Request(`https://${HOST}/console/edge/sync`, {
      method: "POST",
      body,
      headers: {
        "x-pm-key": await keyId(firstPair.publicJwk),
        "x-pm-signature": await sign(firstPair.privateJwk, body),
      },
    });
    expect((await w.edge.now.fetch(first)).headers.get("x-pm-next")).toBe("key");
    expect(w.stored.get("keyFetch")).toBe(w.now.t);
    // A new object over the same storage, still within the thirty
    // seconds: another stranger is refused without asking the host.
    w.edge.now = w.makeEdge();
    w.now.t += 1_000;
    w.hostSaw.length = 0;
    const secondPair = await newKeyPair();
    const secondBody = new TextEncoder().encode(
      JSON.stringify({ v: 1, at: w.now.t, sections: [], have: {}, acks: [] }),
    );
    const second = new Request(`https://${HOST}/console/edge/sync`, {
      method: "POST",
      body: secondBody,
      headers: {
        "x-pm-key": await keyId(secondPair.publicJwk),
        "x-pm-signature": await sign(secondPair.privateJwk, secondBody),
      },
    });
    expect((await w.edge.now.fetch(second)).status).toBe(401);
    expect(w.hostSaw).toEqual([]);
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
    // The key-fetch gate itself is kept: asking the host once is the
    // one change, and the second stranger changes nothing further.
    const afterFetch = JSON.stringify([...w.stored.entries()]);
    w.now.t += 1_000;
    expect((await report(w, () => {}, bySomeoneElse, await keyId(stranger.publicJwk))).status).toBe(401);
    expect(w.hostSaw).toEqual(["GET /console/edge/key"]);
    expect(JSON.stringify([...w.stored.entries()])).toBe(afterFetch);
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
    expect(heldStored(w)).toEqual([]);
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

// The app is told when something changes, and reads again at once. The
// connection is the platform's (a WebSocket the Worker's object holds,
// kept by Cloudflare while the object sleeps); here it is stood in for.
// The oracle for "changed" is the document's own tag: a message means
// the document named would be answered with another tag.
describe("the app is told when something changes", () => {
  type World = Awaited<ReturnType<typeof setup>>;
  const connect = (w: World, init: { token?: string | null; upgrade?: boolean } = {}) =>
    w.browse("/console/api/events", {
      ...(init.token === undefined ? {} : { token: init.token }),
      headers: init.upgrade === false ? {} : { upgrade: "websocket" },
    });
  /** Everything this side reads itself, read once: after this a read is
   * a change only if what it reads has changed. */
  const readAll = async (w: World) => {
    for (let look = 0; look < 2; look++) {
      await w.core();
      await w.browse("/console/api/doc/ops");
      await w.browse("/console/api/doc/done");
      await Promise.all(w.behind.splice(0));
    }
  };
  const said = (w: World) => w.live.open.map((got) => got.map((text) => JSON.parse(text)));
  const tag = async (w: World, doc: string) => (await w.browse(`/console/api/doc/${doc}`)).headers.get("etag");
  const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

  test("only the owner, signed in, is connected; anything else is not a connection", async () => {
    const w = await setup({ direct: true });
    await w.hostReads();
    await w.link.now.once();
    // Not signed in: the Worker connects nobody (the request goes on to the machine, which refuses a stranger).
    expect((await connect(w, { token: null })).headers.get("x-live")).toBeNull();
    expect(
      (await connect(w, { token: await w.token({ email: "someone@else.example" }) })).headers.get("x-live"),
    ).toBeNull();
    expect(w.live.open).toHaveLength(0);
    // Signed in, but not asking to be connected.
    expect((await connect(w, { upgrade: false })).status).toBe(426);
    expect(w.live.open).toHaveLength(0);
    // The owner, asking to be connected.
    const res = await connect(w);
    expect(res.headers.get("x-live")).toBe("accepted");
    expect(w.live.open).toHaveLength(1);
    // And health says how many are connected, which is all it says of them.
    const health = (await (await w.browse("/console/health", { token: null })).json()) as { listening: number };
    expect(health.listening).toBe(1);
  });

  test("what the owner does changes the core document: everyone connected is told once, and the document has another tag", async () => {
    const w = await setup({ direct: true });
    await w.hostReads();
    await w.link.now.once();
    await w.core();
    await Promise.all(w.behind.splice(0));
    await connect(w);
    await connect(w);
    await tick();
    for (const got of w.live.open) got.length = 0;
    const before = await tag(w, "core");
    expect((await w.act("issue.comment", { issue: 14, text: "live" })).status).toBe(200);
    await tick();
    // Each connection was told that the core document changed: once, however many things changed in it.
    expect(said(w)).toEqual([[{ t: "changed", docs: ["core"] }], [{ t: "changed", docs: ["core"] }]]);
    expect(await tag(w, "core")).not.toBe(before);
  });

  test("a read that changes nothing says nothing, and one that changes a list says which document", async () => {
    const w = await setup({ direct: true });
    await w.hostReads();
    await w.link.now.once();
    await readAll(w);
    await connect(w);
    await tick();
    w.live.open[0].length = 0;
    // Past every list's age, with nothing changed at GitHub or in the queue.
    w.now.t += 10 * 60_000;
    await w.edge.now.alarm();
    await tick();
    expect(said(w)).toEqual([[]]);
    // A task is claimed: the board is another board.
    w.queueTasks[1].status = "claimed";
    w.now.t += 10 * 60_000;
    await w.edge.now.alarm();
    await tick();
    expect(said(w)).toEqual([[{ t: "changed", docs: ["core"] }]]);
  });

  test("while someone is connected the Worker keeps reading by itself, and stops asking to be woken for it when nobody is", async () => {
    const w = await setup({ direct: true });
    await w.hostReads();
    await w.link.now.once();
    await w.core();
    await Promise.all(w.behind.splice(0));
    await connect(w);
    // It asked to be woken within the half minute.
    expect(w.alarm.at).toBeGreaterThan(w.now.t);
    expect(w.alarm.at).toBeLessThanOrEqual(w.now.t + 30_000);
    // Woken past the board's age: the board is read again with nobody asking for a document.
    const reads = () => w.queueCalls.length;
    const before = reads();
    w.now.t += 2 * 60_000;
    await w.edge.now.alarm();
    expect(reads()).toBeGreaterThan(before);
    expect(w.alarm.at).toBeLessThanOrEqual(w.now.t + 30_000);
    // Everyone goes away: the next wake is not for this (the machine's silence is still watched for).
    w.live.open.length = 0;
    w.now.t += 30_000;
    await w.edge.now.alarm();
    expect(w.alarm.at === 0 || w.alarm.at > w.now.t + 30_000).toBe(true);
    const after = reads();
    w.now.t += 2 * 60_000;
    await w.edge.now.alarm();
    expect(reads()).toBe(after);
  });

  test("a machine's report that changes what only it has is passed on too", async () => {
    const w = await setup({ direct: true });
    await w.hostReads();
    await w.link.now.once();
    await w.core();
    await Promise.all(w.behind.splice(0));
    await connect(w);
    await tick();
    w.live.open[0].length = 0;
    (w.data.sessions as { waiting: unknown[] }).waiting = [{ id: "s1", title: "Log in", ref: "", why: "" }];
    w.now.t += 11_000;
    await w.hostReads();
    await w.link.now.once();
    await w.core();
    await Promise.all(w.behind.splice(0));
    await tick();
    expect(said(w)[0]).toContainEqual({ t: "changed", docs: ["core"] });
  });

  test("with nobody connected nothing is sent, and a Worker with no connections to hold says so", async () => {
    const w = await setup({ direct: true });
    await w.hostReads();
    await w.link.now.once();
    expect((await w.act("issue.comment", { issue: 14, text: "nobody listening" })).status).toBe(200);
    await tick();
    expect(w.live.open).toEqual([]);
    const health = (await (await w.browse("/console/health", { token: null })).json()) as { listening: number };
    expect(health.listening).toBe(0);
  });
});

// The repository's settings are the platform's own (#14163): the Worker
// carries them in its bundle and goes by them. A machine tells the ones
// it runs with in every report, and until this they were what the
// Worker acted by: the defaults of whatever checkout the laptop's
// console was started from, a branch nobody merged included. The oracle
// throughout is src/settings.ts, which is main's word.
describe("the platform goes by its own settings, whatever a machine says", () => {
  const OTHERS = {
    repo: "someone/else",
    queue: "another-queue",
    queueDbUrl: "https://elsewhere.example",
    queueTokenName: "another.token",
    androidPackage: "another.package",
    promptLabel: "another/label",
  };
  const health = async (w: Awaited<ReturnType<typeof setup>>) =>
    (await (await w.browse("/console/health", { token: null })).json()) as Record<string, unknown>;

  test("a machine that says another repository, queue, store address, token name, package and label: the Worker reads and acts by main's", async () => {
    const w = await setup({ direct: true });
    const mains = platformSettings();
    // The fingerprint of the certificate the Android app is signed with
    // is the deployment's, and is the machine's to tell.
    const fingerprint = Array.from({ length: 32 }, () => "AB").join(":");
    Object.assign(w.edgeConfig, OTHERS, { androidFingerprints: [fingerprint] });
    await w.hostReads();
    await w.link.now.once();
    const { res, doc } = await w.core();
    expect(res.status).toBe(200);
    // The board is the queue store's own word, read where main says it
    // is and for the queue main names: it could not have been read at
    // all from where the machine said.
    expect(doc.edge).toEqual({ held: 0, primary: true });
    expect(doc.health.board).toEqual({ ok: true, error: null });
    expect(doc.board.counts).toEqual({ claimed: 1, queued: 1, blocked: 1 });
    expect(w.queueCalls.length).toBeGreaterThan(0);
    expect(w.queueCalls.some((call) => call.args.includes(mains.queue))).toBe(true);
    expect(w.queueCalls.some((call) => call.args.includes(OTHERS.queue))).toBe(false);
    // Nothing was asked of anywhere else, with the queue's token or without it.
    expect(w.strays).toEqual([]);
    // GitHub was asked about main's repository and by main's label.
    expect(w.githubCalls.length).toBeGreaterThan(0);
    expect(w.githubCalls.filter((call) => call.url.includes(OTHERS.repo))).toEqual([]);
    expect(w.githubCalls.filter((call) => call.url.includes(encodeURIComponent(OTHERS.promptLabel)))).toEqual([]);
    expect(
      w.githubCalls.some((call) =>
        call.url.startsWith(
          `https://api.github.com/repos/${mains.repo}/issues?labels=${encodeURIComponent(mains.promptLabel)}`,
        ),
      ),
    ).toBe(true);
    // What the owner does is done there too.
    const sent = (await (await w.act("issue.comment", { issue: 14, text: "by main's settings" })).json()) as {
      ok: boolean;
    };
    expect(sent.ok).toBe(true);
    expect(w.githubCalls.at(-1)).toMatchObject({
      method: "POST",
      url: `https://api.github.com/repos/${mains.repo}/issues/14/comments`,
    });
    // The Android app is linked by the package main names, signed as the machine says.
    const links = (await (await w.browse("/.well-known/assetlinks.json", { token: null })).json()) as {
      target: { package_name: string; sha256_cert_fingerprints: string[] };
    }[];
    expect(links.map((link) => [link.target.package_name, link.target.sha256_cert_fingerprints])).toEqual([
      [mains.androidPackage, [fingerprint]],
    ]);
    // And it says, by name, where the machine differs.
    expect((await health(w)).machineSettingsDiffer).toEqual(Object.keys(OTHERS));
    expect(w.strays).toEqual([]);
  });

  test("what a machine told is kept as it told it: after a restart the Worker still goes by main's, and still says where the machine differs", async () => {
    const w = await setup({ direct: true });
    Object.assign(w.edgeConfig, OTHERS);
    await w.hostReads();
    await w.link.now.once();
    w.edge.now = w.makeEdge(true);
    w.net.hostUp = false;
    w.net.edgeUp = false;
    const { doc } = await w.core();
    expect(doc.health.board).toEqual({ ok: true, error: null });
    expect(doc.board.counts).toEqual({ claimed: 1, queued: 1, blocked: 1 });
    expect(w.strays).toEqual([]);
    expect((await health(w)).machineSettingsDiffer).toEqual(Object.keys(OTHERS));
    // Kept as told, not as gone by: what is compared after a restart is the machine's own word.
    expect(w.stored.get("cfg")).toMatchObject(OTHERS);
    // And kept once: a machine that goes on saying the same is not written down again with every report.
    w.net.hostUp = true;
    w.net.edgeUp = true;
    w.puts.length = 0;
    w.now.t += 5_000;
    await w.link.now.once();
    w.now.t += 5_000;
    await w.link.now.once();
    expect(w.puts.filter((key) => key === "cfg")).toEqual([]);
  });

  // Where things are the Worker knows by itself; who may sign in it
  // does not. With the switch on and both tokens, and no machine having
  // said, it is not doing the core, and nobody is signed in by it.
  test("with the switch and the tokens but no machine's word on who may sign in, the Worker does not do the core", async () => {
    const w = await setup({ direct: true });
    const said = await health(w);
    expect([said.configured, said.directEnabled, said.primary, said.directReady]).toEqual([false, true, false, false]);
    expect(said.machineSettingsDiffer).toBeNull();
    expect(w.githubCalls).toEqual([]);
    expect(w.queueCalls).toEqual([]);
    // The first report is what changes that.
    await w.hostReads();
    await w.link.now.once();
    expect((await health(w)).primary).toBe(true);
  });

  test("a machine that takes back what it said changes nothing the Worker does, and is no longer said to differ", async () => {
    const w = await setup({ direct: true });
    const mains = platformSettings();
    Object.assign(w.edgeConfig, OTHERS);
    await w.hostReads();
    await w.link.now.once();
    const before = (await w.core()).doc.board;
    Object.assign(
      w.edgeConfig,
      Object.fromEntries(Object.keys(OTHERS).map((name) => [name, mains[name as keyof typeof OTHERS]])),
    );
    w.now.t += 5_000;
    await w.link.now.once();
    expect((await w.core()).doc.board).toEqual(before);
    expect((await health(w)).machineSettingsDiffer).toEqual([]);
    expect(w.strays).toEqual([]);
  });

  // Not changed by this, and said so that it is not taken for done: who
  // may sign in is the deployment's, set by the script that sets the
  // hostname up, and a machine's report is still what tells it (#13495).
  test("who may sign in is still as the machine tells it", async () => {
    const w = await setup();
    await w.hostReads();
    await w.link.now.once();
    const second = "second@example.com";
    /** Who answered the Machines screen for this address: the Worker
     * itself, or the laptop it passes anyone it does not know on to
     * (which in this world still has the first owner), or nobody. */
    const served = async (email?: string) => {
      const res = await w.browse("/console/api/machines", email ? { token: await w.token({ email }) } : {});
      return res.status === 200 ? ((await res.json()) as MachinesAnswer).served : res.status;
    };
    expect([await served(), await served(second)]).toEqual(["edge", 403]);
    w.edgeConfig.owners = [second];
    w.now.t += 5_000;
    await w.link.now.once();
    expect([await served(second), await served()]).toEqual(["edge", "machine"]);
  });

  // The list of repositories is set apart (see the design record): a
  // machine takes a reply only for an item in a repository on its own
  // list, so while a machine tells a list the Worker reads by that one.
  test("for the list of repositories the Worker goes by the machine's while a machine tells one, and says that it differs", async () => {
    const w = await setup({ direct: true });
    const mains = platformSettings();
    const theirs = ["developing-today/code", "developing-today/opencode"];
    w.edgeConfig.needsRepos = theirs;
    w.ghNeeds["developing-today/opencode"] = [{ number: 5, title: "Under its old name", labels: ["user/review"] }];
    await w.hostReads();
    await w.link.now.once();
    const { doc } = await w.core();
    expect(doc.health.census).toEqual({ ok: true, error: null });
    expect(doc.census.reviews.map((item: { ref: string }) => item.ref)).toEqual(["developing-today/opencode#5"]);
    const lists = w.githubCalls.filter((call) => /labels=user%2F/.test(call.url)).map((call) => call.url);
    expect(lists.filter((url) => url.includes("/repos/developing-today/opencode/"))).toHaveLength(2);
    expect(lists.filter((url) => mains.needsRepos.slice(1).some((repo) => url.includes(`/repos/${repo}/`)))).toEqual(
      [],
    );
    expect((await health(w)).machineSettingsDiffer).toEqual(["needsRepos"]);
  });

  test("machines: a machine on other settings is said with what it says, what main says, and which the Worker goes by", async () => {
    const w = await setup({ direct: true });
    const mains = platformSettings();
    const machines = async () => (await (await w.browse("/console/api/machines")).json()) as MachinesAnswer;
    await w.hostReads();
    await w.link.now.once();
    // On main's settings there is nothing to say, and nothing is.
    expect(Object.keys((await machines()).machines[0])).not.toContain("differs");
    const theirs = ["developing-today/code", "developing-today/opencode"];
    w.edgeConfig.queue = OTHERS.queue;
    w.edgeConfig.needsRepos = theirs;
    w.now.t += 5_000;
    await w.link.now.once();
    expect((await machines()).machines[0].differs).toEqual([
      { name: "queue", machine: OTHERS.queue, platform: mains.queue, goesBy: "platform" },
      { name: "needsRepos", machine: theirs, platform: mains.needsRepos, goesBy: "machine" },
    ]);
    // What a machine says is shown as it said it, but not without end.
    w.edgeConfig.queue = "q".repeat(5_000);
    w.edgeConfig.needsRepos = Array.from({ length: 400 }, (_, i) => `o/r${i}`);
    w.now.t += 5_000;
    await w.link.now.once();
    const long = (await machines()).machines[0].differs ?? [];
    expect((long[0].machine as string).length).toBeLessThanOrEqual(200);
    expect((long[1].machine as string[]).length).toBeLessThanOrEqual(50);
    // Nobody not signed in is told any of it.
    expect((await w.browse("/console/api/machines", { token: null })).status).toBe(401);
  });
});

// The settings the owner changes from the app are kept at the edge
// under their own key, beside a machine's word and never in it, and
// are gone by above both the repository and a machine (#14954, S2 of
// doc/2026-11-25T08-48-36Z_design_pm-owner-settings-at-edge.md). The
// oracle is the precedence table of that design (§3), which
// effectiveSettings() in src/settings.ts carries.
describe("the owner's changed settings are kept at the edge and gone by", () => {
  const labelCalls = (w: Awaited<ReturnType<typeof setup>>, label: string) =>
    w.githubCalls.filter((call) => call.url.includes(`labels=${encodeURIComponent(label)}`));

  test("an owner's label is read by, above main's, and above a machine that says another", async () => {
    const w = await setup({ direct: true });
    w.edgeConfig.promptLabel = "machine/label";
    const kept = await w.edge.now.setOwnerSetting(OWNER, "promptLabel", "owner/label");
    expect(kept).toMatchObject({
      rev: 1,
      by: OWNER,
      values: { promptLabel: "owner/label" },
    });
    await w.hostReads();
    await w.link.now.once();
    const { res } = await w.core();
    expect(res.status).toBe(200);
    expect(labelCalls(w, "owner/label").length).toBeGreaterThan(0);
    expect(labelCalls(w, platformSettings().promptLabel)).toEqual([]);
    expect(labelCalls(w, "machine/label")).toEqual([]);
  });

  test("an owner's list of repositories is read by, above the list a machine tells", async () => {
    const w = await setup({ direct: true });
    w.edgeConfig.needsRepos = ["developing-today/code", "developing-today/opencode"];
    w.ghNeeds["developing-today/opencode"] = [
      {
        number: 5,
        title: "Only on the machine's list",
        labels: ["user/review"],
      },
    ];
    await w.edge.now.setOwnerSetting(OWNER, "needsRepos", ["developing-today/code"]);
    await w.hostReads();
    await w.link.now.once();
    const { doc } = await w.core();
    expect(doc.health.census).toEqual({ ok: true, error: null });
    const lists = w.githubCalls.filter((call) => /labels=user%2F/.test(call.url)).map((call) => call.url);
    expect(lists.some((url) => url.includes("/repos/developing-today/code/"))).toBe(true);
    expect(lists.filter((url) => url.includes("/repos/developing-today/opencode/"))).toEqual([]);
    expect(doc.census.reviews.map((item: { ref: string }) => item.ref)).not.toContain("developing-today/opencode#5");
  });

  test("kept beside what a machine told, not in it: a machine's next report and a restart leave it as the owner set it", async () => {
    const w = await setup({ direct: true });
    await w.hostReads();
    await w.link.now.once();
    await w.edge.now.setOwnerSetting(OWNER, "promptLabel", "owner/label");
    const kept = JSON.stringify(w.stored.get("ownerSettings"));
    expect(JSON.stringify(w.stored.get("cfg"))).not.toContain("owner/label");
    // The machine reports on other settings; its word is kept, the owner's stays.
    w.edgeConfig.promptLabel = "machine/label";
    w.now.t += 5_000;
    await w.link.now.once();
    expect((w.stored.get("cfg") as { promptLabel: string }).promptLabel).toBe("machine/label");
    expect(JSON.stringify(w.stored.get("ownerSettings"))).toBe(kept);
    // A new object reads it back and goes by it.
    w.edge.now = w.makeEdge(true);
    w.githubCalls.length = 0;
    w.net.hostUp = false;
    w.net.edgeUp = false;
    await w.core();
    expect(labelCalls(w, "owner/label").length).toBeGreaterThan(0);
    expect(labelCalls(w, "machine/label")).toEqual([]);
  });

  test("a machine's report carrying what looks like the owner's settings changes nothing", async () => {
    const w = await setup({ direct: true });
    Object.assign(w.edgeConfig, {
      ownerSettings: {
        rev: 9,
        at: START,
        by: OWNER,
        values: { promptLabel: "forged/label" },
      },
      values: { promptLabel: "forged/label" },
      rev: 9,
      by: OWNER,
    });
    await w.hostReads();
    await w.link.now.once();
    await w.core();
    expect(w.stored.has("ownerSettings")).toBe(false);
    expect(Object.keys(w.stored.get("cfg") as object)).not.toContain("ownerSettings");
    expect(Object.keys(w.stored.get("cfg") as object)).not.toContain("values");
    expect(labelCalls(w, "forged/label")).toEqual([]);
    expect(labelCalls(w, platformSettings().promptLabel).length).toBeGreaterThan(0);
    expect(auditStored(w).filter((e) => String((e as { kind?: string }).kind).startsWith("setting."))).toEqual([]);
  });

  test("each set and clear is a new revision and an audit entry that names the setting, never its value", async () => {
    const w = await setup({ direct: true });
    await w.hostReads();
    await w.link.now.once();
    const t0 = w.now.t;
    await w.edge.now.setOwnerSetting(OWNER, "promptLabel", "owner/label");
    w.now.t += 1_000;
    await w.edge.now.setOwnerSetting(OWNER, "needsRepos", ["developing-today/code", "developing-today/code"]);
    expect(w.stored.get("ownerSettings")).toEqual({
      rev: 2,
      at: w.now.t,
      by: OWNER,
      values: {
        promptLabel: "owner/label",
        needsRepos: ["developing-today/code"],
      },
    });
    w.now.t += 1_000;
    const one = await w.edge.now.clearOwnerSetting(OWNER, "promptLabel");
    expect(one).toMatchObject({
      rev: 3,
      values: { needsRepos: ["developing-today/code"] },
    });
    // Clearing what is not changed changes nothing.
    expect(await w.edge.now.clearOwnerSetting(OWNER, "promptLabel")).toEqual(one);
    w.now.t += 1_000;
    expect(await w.edge.now.clearOwnerSetting(OWNER)).toMatchObject({
      rev: 4,
      values: {},
    });
    const settings = auditStored(w).filter((e) => String((e as { kind?: string }).kind).startsWith("setting."));
    expect(settings).toEqual([
      {
        kind: "setting.set",
        name: "promptLabel",
        rev: 1,
        at: new Date(t0).toISOString(),
        by: OWNER,
      },
      {
        kind: "setting.set",
        name: "needsRepos",
        rev: 2,
        at: new Date(t0 + 1_000).toISOString(),
        by: OWNER,
      },
      {
        kind: "setting.clear",
        name: "promptLabel",
        rev: 3,
        at: new Date(t0 + 2_000).toISOString(),
        by: OWNER,
      },
      {
        kind: "setting.clear",
        name: "needsRepos",
        rev: 4,
        at: new Date(t0 + 3_000).toISOString(),
        by: OWNER,
      },
    ]);
    expect(JSON.stringify(settings)).not.toContain("owner/label");
    // Cleared, the repository speaks again at once.
    w.githubCalls.length = 0;
    await w.core();
    expect(labelCalls(w, platformSettings().promptLabel).length).toBeGreaterThan(0);
    expect(labelCalls(w, "owner/label")).toEqual([]);
  });

  test("a value not in the setting's shape is refused whole, and nothing is kept", async () => {
    const w = await setup({ direct: true });
    await expect(w.edge.now.setOwnerSetting(OWNER, "promptLabel", "")).rejects.toBeInstanceOf(OpError);
    await expect(w.edge.now.setOwnerSetting(OWNER, "needsRepos", ["not a repository"])).rejects.toBeInstanceOf(OpError);
    expect(w.stored.has("ownerSettings")).toBe(false);
    expect(auditStored(w)).toEqual([]);
  });

  test("a kept record that does not read as one is no record: the repository speaks", async () => {
    for (const bad of [
      null,
      { rev: 0, at: START, by: OWNER, values: {} },
      { rev: 1, at: START, by: "", values: {} },
      { rev: 1, at: START, by: OWNER, values: { repo: "someone/else" } },
      { rev: 1, at: START, by: OWNER, values: { promptLabel: 7 } },
      { rev: 1, at: START, by: OWNER, values: { needsRepos: ["no-slash"] } },
    ]) {
      expect(ownerSettingsRecord(bad)).toBeNull();
    }
    expect(
      ownerSettingsRecord({
        rev: 2,
        at: START,
        by: OWNER,
        values: { needsRepos: ["a/b", "a/b"] },
        extra: 1,
      }),
    ).toEqual({
      rev: 2,
      at: START,
      by: OWNER,
      values: { needsRepos: ["a/b"] },
    });
    // Kept that way, the edge goes by the repository.
    const w = await setup({ direct: true });
    w.stored.set("ownerSettings", {
      rev: 1,
      at: START,
      by: OWNER,
      values: { promptLabel: "owner/label", repo: "x/y" },
    });
    await w.hostReads();
    await w.link.now.once();
    await w.core();
    expect(labelCalls(w, "owner/label")).toEqual([]);
    expect(labelCalls(w, platformSettings().promptLabel).length).toBeGreaterThan(0);
  });
});

describe("the edge does the core itself", () => {
  type World = Awaited<ReturnType<typeof setup>>;
  /** The switch is on, the tokens are there, and a machine has said
   * where things are: from its first report on, this side does the core. */
  async function primary(kinds?: readonly string[]) {
    const w = await setup({ direct: true, ...(kinds ? { kinds } : {}) });
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

  test("a message is delivered by the edge through the bridge, with every machine away, and the agents and the log are read there", async () => {
    const w = await setup({ direct: true, bridge: true });
    await w.hostReads();
    await w.link.now.once();
    w.hostSaw.length = 0;
    // Every machine goes away: nothing can be relayed or held for one.
    w.net.hostUp = false;
    const sent = await w.act("message.send", { to: ["lane-1"], text: "from the edge" });
    expect(await sent.json()).toEqual({
      ok: true,
      result: {
        to: ["lane-1"],
        command: false,
        batch: "b1",
        matched: 1,
        delivered: 1,
        recipients: ["lane-1"],
        remaining: 0,
        unregistered: [],
      },
    });
    // The bridge was asked once, over its binding, as the PM stack.
    expect(w.bridgeCalls).toHaveLength(1);
    expect(w.bridgeCalls[0].method).toBe("POST");
    expect(w.bridgeCalls[0].url).toBe("https://bridge.internal/fleet/pm-cred-test/send");
    expect(JSON.parse(w.bridgeCalls[0].body ?? "{}")).toEqual({
      agent: "pm-stack",
      to: ["lane-1"],
      body: "from the edge",
      kind: "message",
    });
    // Nothing waits for a machine, and no machine was asked for anything.
    const machines = (await (await w.browse("/console/api/machines")).json()) as MachinesAnswer;
    expect(machines.held).toEqual([]);
    expect(w.ran).toEqual([]);
    // The agents list is the bridge's, read here: the ops document
    // shows the agent the bridge named, not the host's empty list.
    const ops = (await (await w.browse("/console/api/doc/ops")).json()) as { agents: { name: string }[] };
    expect(ops.agents.map((a) => a.name)).toEqual(["lane-1"]);
    // The message log is the bridge's too, at the address the app asks.
    const log = (await (await w.browse("/console/api/messages?limit=15")).json()) as { body: string }[];
    expect(log.map((m) => m.body)).toEqual(["earlier"]);
    expect(w.bridgeCalls.map((c) => `${c.method} ${c.url}`)).toEqual([
      "POST https://bridge.internal/fleet/pm-cred-test/send",
      "GET https://bridge.internal/fleet/pm-cred-test/agents",
      "GET https://bridge.internal/fleet/pm-cred-test/messages?limit=15&order=desc",
    ]);
    expect(w.hostSaw).not.toContain("GET /console/api/messages?limit=15");
  });

  test("the unified pipeline and directory are read at the edge, and a revoke is held for a machine, never done here", async () => {
    const w = await setup({ direct: true, bridge: true });
    await w.hostReads();
    await w.link.now.once();
    w.hostSaw.length = 0;
    w.net.hostUp = false;
    // The Messages screen's read: one feed over all three surfaces,
    // the filters the owner chose carried to the bridge, and every
    // row's real sender intact — an agent's command names the agent.
    const feed = (await (await w.browse("/console/api/pipeline?kind=command&sender=lane-1")).json()) as {
      surface: string;
      sender_addr: string;
    }[];
    expect(feed.map((m) => [m.surface, m.sender_addr])).toEqual([
      ["connector", "user:drew"],
      ["fleet", "agent:lane-1"],
      ["notes", "user:drew"],
    ]);
    // The Agents screen's read: the unified directory with credential
    // state merged in — the fleet agent active, the notes-only
    // identity present with no presence invented for it.
    const directory = (await (await w.browse("/console/api/directory")).json()) as {
      name: string;
      surface: string;
      presence: string;
      last_seen: string | null;
      credential: { state: string } | null;
    }[];
    const lane = directory.find((a) => a.name === "lane-1");
    expect(lane?.credential?.state).toBe("active");
    const scout = directory.find((a) => a.name === "muse/scout-1");
    expect(scout?.surface).toBe("notes");
    expect(scout?.presence).toBe("none");
    expect(scout?.last_seen).toBeNull();
    expect(scout?.credential?.state).toBe("active");
    expect(w.bridgeCalls.map((c) => `${c.method} ${c.url}`)).toEqual([
      "GET https://bridge.internal/fleet/pm-cred-test/pipeline?order=desc&kind=command&sender=lane-1&limit=100",
      "GET https://bridge.internal/fleet/pm-cred-test/directory",
      "GET https://bridge.internal/fleet/pm-cred-test/credentials",
    ]);
    expect(w.hostSaw).not.toContain("GET /console/api/pipeline?kind=command&sender=lane-1");
    expect(w.hostSaw).not.toContain("GET /console/api/directory");
    // The provisioning read (#14376): the minted enroll tokens'
    // state, read the same way — state only, no hash, no value.
    const enrollTokens = (await (await w.browse("/console/api/enroll-tokens")).json()) as {
      agent: string;
      kind: string;
      groups: string[];
      notes_identity: string | null;
      created: string;
      expires_at: string;
      used_at: string | null;
      revoked_at: string | null;
      state: string;
    }[];
    expect(enrollTokens).toEqual([
      {
        agent: "new-1",
        kind: "muse",
        groups: ["muse"],
        notes_identity: null,
        created: "2026-11-24T00:00:00Z",
        expires_at: "2026-11-25T00:00:00Z",
        used_at: null,
        revoked_at: null,
        state: "pending",
      },
    ]);
    expect(w.bridgeCalls.at(-1)?.method).toBe("GET");
    expect(w.bridgeCalls.at(-1)?.url).toBe("https://bridge.internal/fleet/pm-cred-test/enroll-tokens");
    expect(w.hostSaw).not.toContain("GET /console/api/enroll-tokens");
    // A revoke asked here is held for a machine — the edge's
    // credential holds the read scope only — and the bridge is not
    // asked for one.
    const callsBefore = w.bridgeCalls.length;
    const held = await w.act("agent.revoke", { surface: "fleet", name: "lane-1" });
    expect(await held.json()).toEqual({ ok: true, held: true, result: null, waits: "machine" });
    expect(w.bridgeCalls).toHaveLength(callsBefore);
    // Taking back unused enroll tokens is held the same way (#14376).
    const heldEnroll = await w.act("agent.revoke-enroll", { agent: "new-1" });
    expect(await heldEnroll.json()).toEqual({ ok: true, held: true, result: null, waits: "machine" });
    expect(w.bridgeCalls).toHaveLength(callsBefore);
    // A mint is never held: its answer is the secret itself, shown
    // once on the screen that asked — with no machine answering, it
    // is refused, and the bridge is not asked for one either.
    const mint = await w.act("agent.mint-enroll", { agent: "new-2", kind: "muse" });
    expect(((await mint.json()) as { held?: unknown }).held).not.toBe(true);
    expect(w.bridgeCalls).toHaveLength(callsBefore);
    const machines = (await (await w.browse("/console/api/machines")).json()) as MachinesAnswer;
    expect(machines.held.map((entry) => entry.kind)).toEqual(["agent.revoke", "agent.revoke-enroll"]);
  });

  test("a bridge credential the bridge no longer knows stops the send here, with the reason said", async () => {
    const w = await setup({ direct: true, bridge: true });
    await w.hostReads();
    await w.link.now.once();
    w.net.hostUp = false;
    w.bridgeRefuses.on = true;
    const sent = await w.act("message.send", { to: ["lane-1"], text: "never sent" });
    expect(sent.status).toBe(502);
    expect(((await sent.json()) as { error: string }).error).toContain("turned the PM credential away");
    // It was not handed to a machine either: the refusal is the
    // credential's, and a machine's own dispatch is another matter.
    const machines = (await (await w.browse("/console/api/machines")).json()) as MachinesAnswer;
    expect(machines.held).toEqual([]);
    expect(w.ran).toEqual([]);
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
    expect(answer?.owns).toEqual(["board", "prs", "priorities", "asks", "census", "done"]);
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
    expect((heldStored(w) as { kind: string }[]).map((h) => h.kind)).toEqual(["prompt.enqueue"]);
    // Each is on this side's record of what was done, without what was typed.
    const audit = auditStored(w) as { action: string; phase: string; ok?: boolean; args: unknown }[];
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

  test("one audit event writes one entry, not the whole log (#14960)", async () => {
    const w = await primary();
    // A log at its kept size, kept one entry per key.
    for (let seq = 0; seq < AUDIT_KEPT; seq++)
      w.stored.set(`audit:${seq}`, { action: "seed", phase: "done", ok: true, n: seq });
    w.stored.set("audit:next", AUDIT_KEPT);
    w.edge.now = w.makeEdge();
    const before = w.puts.length;
    expect((await w.act("issue.comment", { issue: 7, text: "counted" })).status).toBe(200);
    // Of the log, only the event's two entries (start and done) and the
    // count beside them were written — never the whole log at once.
    expect(w.puts.slice(before).filter((k) => k === "audit" || k.startsWith("audit:"))).toEqual([
      "audit:300",
      "audit:next",
      "audit:301",
      "audit:next",
    ]);
    // The kept size stands: the two oldest entries fell out of the window.
    expect([w.stored.has("audit:0"), w.stored.has("audit:1"), w.stored.has("audit:301")]).toEqual([false, false, true]);
    const log = auditStored(w) as { n?: number }[];
    expect(log.length).toBe(AUDIT_KEPT);
    expect(log[0].n).toBe(2);
  });

  test("one act more writes one act record, not the whole record of what was done (#14960)", async () => {
    const w = await primary();
    // The record at its kept size, kept one act per key, newest first.
    const ids: string[] = [];
    for (let n = 0; n < 30; n++) {
      const id = `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;
      ids.push(id);
      w.stored.set(`act:${id}`, {
        id,
        kind: "issue.comment",
        at: new Date(START + (30 - n) * 1000).toISOString(),
        by: "owner",
        status: "done",
        note: "",
      });
    }
    w.stored.set("acts:index", ids);
    w.edge.now = w.makeEdge();
    const before = w.puts.length;
    expect((await w.act("issue.comment", { issue: 7, text: "counted" })).status).toBe(200);
    const written = w.puts.slice(before).filter((k) => k === "acts" || k === "acts:index" || k.startsWith("act:"));
    // The whole record was never written; the index was written once.
    expect(written.filter((k) => k === "acts")).toEqual([]);
    expect(written.filter((k) => k === "acts:index")).toHaveLength(1);
    // Every act record written is the new act's own.
    const fresh = [...new Set(written.filter((k) => k.startsWith("act:")))];
    expect(fresh).toHaveLength(1);
    expect(ids).not.toContain(fresh[0].slice(4));
    // The kept size stands: the last act in the list fell out, record and all.
    expect(w.stored.has(`act:${ids[29]}`)).toBe(false);
    expect(storedList(w.stored, "acts", (id) => `act:${id}`)).toHaveLength(30);
  });

  test("taking one thing back writes the held index, not the whole list (#14960)", async () => {
    const w = await primary();
    // What is held, at its most, kept one record per key.
    const ids: string[] = [];
    for (let n = 0; n < 100; n++) {
      const id = `10000000-0000-4000-8000-${String(n).padStart(12, "0")}`;
      ids.push(id);
      w.stored.set(`held:${id}`, {
        id,
        kind: "issue.comment",
        args: { issue: 7, text: `held ${n}` },
        who: "owner",
        at: new Date(START + n * 1000).toISOString(),
        unsent: true,
      });
    }
    w.stored.set("held:index", ids);
    w.edge.now = w.makeEdge();
    const before = w.puts.length;
    expect((await w.act("held.drop", { id: ids[50] })).status).toBe(200);
    // Only the index was written; the dropped record is forgotten, the rest untouched.
    expect(w.puts.slice(before).filter((k) => k === "held" || k === "held:index" || k.startsWith("held:"))).toEqual([
      "held:index",
    ]);
    expect(w.stored.has(`held:${ids[50]}`)).toBe(false);
    expect(heldStored(w)).toHaveLength(99);
  });

  test("what an older copy kept as whole lists is read back the same, and kept the new way (#14960)", async () => {
    const w = await primary();
    // Storage as the version before this one left it: whole lists under plain names.
    for (const k of [...w.stored.keys()]) {
      if (/^(held|taken|handed|acts|act:|audit)/.test(k)) w.stored.delete(k);
    }
    const held = [
      {
        id: "20000000-0000-4000-8000-000000000001",
        kind: "issue.comment",
        args: { issue: 7, text: "kept" },
        who: "owner",
        at: new Date(START).toISOString(),
        unsent: true,
      },
    ];
    const acts = [
      {
        id: "20000000-0000-4000-8000-000000000002",
        kind: "issue.comment",
        at: new Date(START).toISOString(),
        by: "owner",
        status: "done",
        note: "",
      },
    ];
    w.stored.set("held", held);
    w.stored.set("handed", [held[0].id]);
    w.stored.set("acts", acts);
    w.stored.set("audit", [{ action: "issue.comment", phase: "done", ok: true }]);
    w.stored.set("taken", []);
    w.edge.now = w.makeEdge();
    const { doc } = await w.core();
    // Read back the same: what is held is as it was, and the act is on the record.
    expect(heldStored(w)).toEqual(held);
    expect(handedStored(w)).toEqual([held[0].id]);
    expect(doc.acts.map((a: { id: string }) => a.id)).toContain(acts[0].id);
    // Kept the new way now: the plain names are gone, the records under their own.
    expect([w.stored.has("held"), w.stored.has("handed"), w.stored.has("acts")]).toEqual([false, false, false]);
    expect(w.stored.get(`held:${held[0].id}`)).toEqual(held[0]);
    expect(w.stored.get(`act:${acts[0].id}`)).toEqual(acts[0]);
    // The audit log, too: the next event is numbered after the kept entry.
    expect((await w.act("issue.comment", { issue: 7, text: "after" })).status).toBe(200);
    expect(w.stored.has("audit")).toBe(false);
    expect(auditStored(w)[0]).toEqual({ action: "issue.comment", phase: "done", ok: true });
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
    expect((heldStored(w) as { kind: string }[]).map((h) => h.kind)).toEqual(["prompt.enqueue"]);
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
        "hands.done",
        "issue.rank",
        "task.move",
        "pr.merge",
        "deploy.start",
        "intake.send",
      ],
      sections: ["board", "prs", "priorities", "asks", "census", "done"],
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

  // "Done" on something that needed the owner's hands is the label
  // taken off and the fleet told. It was a machine's to do: with the
  // laptop away it waited. The Worker does it itself now, as it does a
  // reply (#13491). The oracle is GitHub's stand-in: what the item
  // carries and what was said on it afterwards.
  describe("what needed the owner's hands, marked done with no machine", () => {
    const REF = "developing-today/code#301";
    /** One thing only he can do, in the repository the stand-in keeps comments for. */
    const handsItem = (w: World) => {
      w.ghNeeds["developing-today/code"].push({
        number: 301,
        title: "amd will not boot",
        labels: ["user/escalated"],
        body: 'It needs a person.\n<!-- hands:v1 kind=physical action="Power the host on at its BMC." -->',
      });
      // What was read before it was there is kept for two minutes: past that.
      w.now.t += 121_000;
    };
    /** The list as GitHub has it now: the first look after a start
     * shows the copy from before and reads behind it. */
    const hands = async (w: World) => {
      await w.core();
      await settle(w);
      return (await w.core()).doc.census.hands.map((i: { ref: string }) => i.ref);
    };
    const said = (w: World) => (w.ghComments[301] ?? []).map((c) => c.body);
    const onItem = (method: string, tail: RegExp) => (m: string, url: string) => m === method && tail.test(url);

    test("the label is taken off and the fleet is told, by the Worker itself, and the item leaves the list at once", async () => {
      const w = await primary();
      handsItem(w);
      w.net.hostUp = false;
      w.net.edgeUp = false;
      expect(await hands(w)).toEqual([REF]);
      const before = w.githubCalls.length;
      const done = await w.act("hands.done", { ref: REF, note: "the BMC is on" });
      expect(await done.json()).toEqual({ ok: true, result: { ref: REF, done: true } });
      // The label first, then the comment, each once.
      expect(
        w.githubCalls
          .slice(before)
          .filter((c) => c.method !== "GET")
          .map((c) => [c.method, new URL(c.url).pathname]),
      ).toEqual([
        ["DELETE", "/repos/developing-today/code/issues/301/labels/user%2Fescalated"],
        ["POST", "/repos/developing-today/code/issues/301/comments"],
      ]);
      expect(w.ghNeeds["developing-today/code"][0].labels).toEqual([]);
      expect(said(w)).toHaveLength(1);
      expect(said(w)[0]).toContain("Done by the owner");
      expect(said(w)[0]).toContain("the BMC is on");
      // No machine was asked, and nothing waits for one.
      expect(w.ran.filter((r) => r.kind === "hands.done")).toEqual([]);
      expect(heldStored(w)).toEqual([]);
      // The list the app shows is read again before the answer: the
      // item is gone from the very next look, with nothing left to settle.
      expect((await w.core()).doc.census.hands).toEqual([]);
    });

    test("the comment fails after the label is off: it is said to have failed, no machine is given it, and trying again says it once", async () => {
      const w = await primary();
      handsItem(w);
      w.ghBreaks.push(onItem("POST", /\/issues\/301\/comments$/));
      const first = await w.act("hands.done", { ref: REF });
      expect(first.status).toBe(502);
      expect(w.ghNeeds["developing-today/code"][0].labels).toEqual([]);
      expect(said(w)).toEqual([]);
      // Something was written, so it is not a machine's to do again from the start.
      expect(w.ran.filter((r) => r.kind === "hands.done")).toEqual([]);
      expect(heldStored(w)).toEqual([]);
      w.ghBreaks.length = 0;
      const again = await w.act("hands.done", { ref: REF });
      expect(await again.json()).toEqual({ ok: true, result: { ref: REF, done: true } });
      expect(said(w)).toHaveLength(1);
    });

    test("a label an agent already took off: the fleet is still told, once", async () => {
      const w = await primary();
      handsItem(w);
      w.ghNeeds["developing-today/code"][0].labels = [];
      const done = await w.act("hands.done", { ref: REF });
      expect(await done.json()).toEqual({ ok: true, result: { ref: REF, done: true } });
      expect(said(w)).toHaveLength(1);
    });

    test("a token that may not write there: nothing is written, and it is a machine's to do for the hour, as a reply is", async () => {
      const w = await primary();
      handsItem(w);
      w.ghRefuses.push(onItem("DELETE", /\/issues\/301\/labels\//));
      const done = await w.act("hands.done", { ref: REF });
      expect(done.status).toBe(200);
      // The machine did it (the stand-in for it records what it ran).
      expect(w.ran.filter((r) => r.kind === "hands.done")).toHaveLength(1);
      expect(said(w)).toEqual([]);
      const health = (await (await w.browse("/console/health", { token: null })).json()) as { refused: string[] };
      expect(health.refused).toEqual(["hands.done"]);
    });

    test("only for an item in a repository on the list, as on a machine", async () => {
      const w = await primary();
      const before = w.githubCalls.length;
      const res = await w.act("hands.done", { ref: "someone/else#1" });
      expect(res.status).toBe(400);
      expect(w.githubCalls.slice(before).filter((c) => c.method !== "GET")).toEqual([]);
    });
  });

  /** What needs the owner, as GitHub's stand-in has it for these tests. */
  const NEEDS = (w: World) => {
    w.ghNeeds["developing-today/code"].push(
      {
        number: 11,
        title: "Sign the lease",
        labels: ["user/escalated", "kind/chore"],
        body: 'Notes.\n<!-- hands:v1 kind=signature action="Sign the lease and send it back." -->',
      },
      { number: 12, title: "Review the plan", labels: ["user/review"], pr: true },
    );
    w.ghNeeds["dezren39/darwin-nix"].push({
      number: 3,
      title: "Which host?",
      labels: ["user/escalated"],
      body: "Reason: two hosts fit",
      updated_at: "2026-11-21T08:30:00Z",
    });
  };
  const needsCalls = (w: World) => w.githubCalls.filter((c) => /\/issues\?state=open&labels=user%2F/.test(c.url));

  test("what needs the owner is read here from each repository's own list, with no machine", async () => {
    const w = await primary();
    // Nothing waits on him yet.
    expect((await w.core()).doc.census).toMatchObject({ hands: [], reviews: [], other: [] });
    await settle(w);
    const repos = Object.keys(w.ghNeeds);
    w.net.hostUp = false;
    w.net.edgeUp = false;
    NEEDS(w);
    w.now.t += 121_000;
    await w.core();
    await settle(w);
    const { doc } = await w.core();
    expect(doc.census).toEqual({
      at: null,
      complete: true,
      unread: [],
      hands: [
        {
          ref: "developing-today/code#11",
          title: "Sign the lease",
          url: "https://github.com/developing-today/code/issues/11",
          pr: false,
          updated: "2026-11-22T12:00:00",
          kind: "signature",
          action: "Sign the lease and send it back.",
        },
      ],
      reviews: [
        {
          ref: "developing-today/code#12",
          title: "Review the plan",
          url: "https://github.com/developing-today/code/pull/12",
          pr: true,
          updated: "2026-11-22T12:00:00",
        },
      ],
      other: [
        {
          ref: "dezren39/darwin-nix#3",
          title: "Which host?",
          url: "https://github.com/dezren39/darwin-nix/issues/3",
          pr: false,
          updated: "2026-11-21T08:30:00",
          why: "two hosts fit",
        },
      ],
    });
    expect(doc.health.census).toEqual({ ok: true, error: null });
    // Asked of each repository's list of each label, and never of the search.
    const asked = needsCalls(w).map((c) => new URL(c.url).pathname);
    expect(new Set(asked)).toEqual(new Set(repos.map((repo) => `/repos/${repo}/issues`)));
    expect(w.githubCalls.some((c) => c.url.includes("/search/") && c.url.includes("user%2F"))).toBe(false);
  });

  test("a repository this side's token cannot see is named and the rest is shown; with none it can see, the last good copy stays and says why", async () => {
    const w = await primary();
    w.net.hostUp = false;
    w.net.edgeUp = false;
    NEEDS(w);
    delete w.ghNeeds["developing-today-forks/opencode"];
    w.now.t += 121_000;
    await w.core();
    await settle(w);
    const partial = (await w.core()).doc;
    expect([partial.census.complete, partial.census.unread]).toEqual([false, ["developing-today-forks/opencode"]]);
    expect(partial.census.hands.map((i: { ref: string }) => i.ref)).toEqual(["developing-today/code#11"]);
    expect(partial.health.census).toEqual({ ok: true, error: null });
    // None of them now.
    for (const repo of Object.keys(w.ghNeeds)) delete w.ghNeeds[repo];
    w.now.t += 121_000;
    await w.core();
    await settle(w);
    const none = (await w.core()).doc;
    expect(none.census).toEqual(partial.census);
    expect(none.health.census.ok).toBe(false);
    expect(none.health.census.error).toContain("what needs you could not be read");
    // Not seeing a repository is not being turned away: the list is still this side's to read.
    const health = (await (await w.browse("/console/health", { token: null })).json()) as { refused: string[] };
    expect(health.refused).toEqual([]);
  });

  test("the lists that need the owner are asked for by their tags: unchanged, they cost nothing and nothing is kept again", async () => {
    const w = await primary();
    w.ghTags.on = true;
    NEEDS(w);
    const lists = Object.keys(w.ghNeeds).length * 2;
    /** One round of reading, two minutes on: the calls it made for these lists. */
    const round = async () => {
      const from = needsCalls(w).length;
      w.now.t += 121_000;
      await w.core();
      await settle(w);
      return needsCalls(w).slice(from);
    };
    const changed = (calls: typeof w.githubCalls) =>
      calls
        .filter((c) => c.unchanged !== true)
        .map(
          (c) =>
            `${new URL(c.url).pathname.split("/").slice(2, 4).join("/")} ${new URL(c.url).searchParams.get("labels")}`,
        );
    // Every list is asked for. The three that changed since the last read are read whole;
    // the rest are "not modified".
    const whole = await round();
    expect([whole.length, changed(whole)]).toEqual([
      lists,
      [
        "developing-today/code user/review",
        "developing-today/code user/escalated",
        "dezren39/darwin-nix user/escalated",
      ],
    ]);
    const first = (await w.core()).doc.census;
    expect(first.hands).toHaveLength(1);
    const written = w.puts.length;
    // Two minutes on, the same lists are asked for, and each is "not modified".
    const again = await round();
    expect([again.length, again.every((c) => c.unchanged === true)]).toEqual([lists, true]);
    expect((await w.core()).doc.census).toEqual(first);
    // What is shown did not change, so the copy kept is not written out again.
    expect(w.puts.slice(written).filter((key) => key.startsWith("m:census:"))).toEqual([]);
    // One of them changes: that list is read whole, and the others are still "not modified".
    w.ghNeeds["dezren39/darwin-nix"].push({ number: 4, title: "Another", labels: ["user/review"] });
    expect(changed(await round())).toEqual(["dezren39/darwin-nix user/review"]);
    expect((await w.core()).doc.census.reviews.map((i: { ref: string }) => i.ref)).toEqual([
      "developing-today/code#12",
      "dezren39/darwin-nix#4",
    ]);
  });

  test("a laptop that has not said which repositories to look in goes on reporting what needs the owner itself", async () => {
    const w = await primary();
    NEEDS(w);
    // Its report names no repositories (a console from before it did).
    const saidBefore = needsCalls(w).length;
    w.edgeConfig.needsRepos = [];
    (w.data.census as { reviews: unknown[] }).reviews = [
      { ref: "developing-today/code#12", title: "From the laptop", url: "u", pr: true, updated: "" },
    ];
    w.now.t += 1_000;
    await w.hostReads();
    // It is told the list is not read here, sends its own, and that is what is shown.
    expect((await w.link.now.once())?.owns).toEqual(["board", "prs", "priorities", "asks", "done"]);
    w.now.t += 121_000;
    await w.hostReads();
    await w.link.now.once();
    await w.core();
    await settle(w);
    expect((await w.core()).doc.census.reviews.map((i: { title: string }) => i.title)).toEqual(["From the laptop"]);
    // And GitHub is not asked again for lists it has no repositories to ask of.
    expect(needsCalls(w).length).toBe(saidBefore);
    expect((await w.core()).doc.health.census).toEqual({ ok: true, error: null });
  });

  test("refused every one of the lists, this side leaves what needs the owner to the laptop for the hour, as with anything it reads", async () => {
    const w = await primary();
    NEEDS(w);
    w.ghRefuses.push((method, url) => method === "GET" && /labels=user%2F/.test(url));
    w.now.t += 121_000;
    await w.core();
    await settle(w);
    const health = async () =>
      ((await (await w.browse("/console/health", { token: null })).json()) as { refused: string[] }).refused;
    expect(await health()).toEqual(["read.census"]);
    // The laptop is told the list is its own to send again, and its copy is what is shown.
    (w.data.census as { hands: unknown[] }).hands = [
      { ref: "developing-today/code#11", title: "From the laptop", url: "u", pr: false, updated: "", action: "" },
    ];
    w.now.t += 1_000;
    await w.hostReads();
    expect((await w.link.now.once())?.owns).toEqual(["board", "prs", "priorities", "asks", "done"]);
    w.now.t += 1_000;
    await w.link.now.once();
    expect((await w.core()).doc.census.hands.map((i: { title: string }) => i.title)).toEqual(["From the laptop"]);
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
    // The oracle for what the list says is the file itself, read here
    // apart from the code under test (the #14290 pattern, #14297):
    // whoever updates the repository's list — a Worker deployed, a
    // deploy workflow renamed — changes what is expected with it.
    const listed = JSON.parse(readFileSync(join(import.meta.dir, "..", "..", "platform", "workers.json"), "utf8")) as {
      checked: string;
      workers: { name: string; group: string; state: string; deploy: string; workflow: string | null }[];
    };
    const listedByName = new Map(listed.workers.map((w) => [w.name, w]));
    expect([p.malformed, p.checked]).toEqual([0, listed.checked]);
    expect(p.workers.map((x) => x.name).sort()).toEqual(listed.workers.map((w) => w.name).sort());
    // The platform first, then the rest by what they belong to.
    const own = listedByName.get("pm-console-edge");
    if (!own) throw new Error("pm-console-edge is not in the repository's list of Workers");
    expect(p.workers[0]).toMatchObject({
      name: own.name,
      group: own.group,
      state: own.state,
      deploy: own.deploy,
      workflow: own.workflow,
      // The latest run of its deploy on main, read from GitHub here.
      run: { commit: expect.stringMatching(/^[0-9a-f]{12}$/), state: "success", at: "2026-11-22T11:50:00Z" },
    });
    const byName = new Map(p.workers.map((x) => [x.name, x]));
    // One that a machine deploys by hand has no run to show; one that is not deployed, neither.
    const byHand = listed.workers.find((w) => w.deploy === "by hand");
    if (!byHand) throw new Error("the repository's list has no by-hand Worker");
    expect(byName.get(byHand.name)).toMatchObject({ deploy: "by hand", workflow: null, run: null });
    const notDeployed = listed.workers.find((w) => w.deploy === "none");
    if (!notDeployed) throw new Error("the repository's list has no undeployed Worker");
    expect(byName.get(notDeployed.name)).toMatchObject({ state: notDeployed.state, deploy: "none", run: null });
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
    const held = heldStored(w) as { kind: string; args: Record<string, unknown> }[];
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

  // A question or a standing instruction is filed here as what it is,
  // and the machine that files its task is told which: the task then
  // asks for an answer, or for the instruction to be recorded, and not
  // for "something to be done". The oracle is GitHub's stand-in (the
  // issue as filed) and what the machine was handed.
  test("a question and a standing instruction are filed here as what they are, and the machine is told which when it files the task", async () => {
    const w = await primary();
    w.net.hostUp = false;
    w.net.edgeUp = false;
    const asked = await w.act("ask.send", { text: "Why did the join slice stall?", intent: "question" });
    expect(((await asked.json()) as { result: { issue: number } }).result.issue).toBe(500);
    await w.act("ask.send", { text: "No merges on a red check.", intent: "directive" });
    await w.act("ask.send", { text: "Do the docs" });
    expect([500, 501, 502].map((n) => w.ghIssues[n].title)).toEqual([
      "Owner asks: Why did the join slice stall?",
      "Owner directs: No merges on a red check.",
      "Owner: Do the docs",
    ]);
    expect(w.ghIssues[500].body).toContain("Change nothing to answer it");
    const held = heldStored(w) as { kind: string; args: Record<string, unknown> }[];
    expect(held.map((h) => [h.kind, h.args.issue, h.args.intent])).toEqual([
      ["prompt.enqueue", 500, "question"],
      ["prompt.enqueue", 501, "directive"],
      // Something to do is held as it always was: nothing more is said of it.
      ["prompt.enqueue", 502, undefined],
    ]);
    expect(Object.keys(held[2].args).sort()).toEqual(["at", "issue", "text"]);
    // The list he sees says which each is.
    w.now.t += 46_000;
    await w.core();
    await settle(w);
    const listed = (await w.core()).doc.asks as { issue: number; intent: string }[];
    expect(
      listed
        .filter((a) => a.issue >= 500)
        .map((a) => [a.issue, a.intent])
        .sort(),
    ).toEqual([
      [500, "question"],
      [501, "directive"],
      [502, "task"],
    ]);
    // A reply on the question is held as a question too.
    await w.act("ask.reply", { issue: 500, text: "Since when?" });
    expect((heldStored(w) as typeof held).at(-1)?.args).toMatchObject({
      issue: 500,
      intent: "question",
      text: "Since when?",
    });
    // An intent that is none of the three is refused, and nothing is filed for it.
    const before = w.githubCalls.filter((c) => c.method === "POST" && c.url.endsWith("/issues")).length;
    const odd = await w.act("ask.send", { text: "x", intent: "rant" });
    expect(odd.status).toBe(400);
    expect(w.githubCalls.filter((c) => c.method === "POST" && c.url.endsWith("/issues")).length).toBe(before);
    // The machine comes back and is handed each with what it is.
    w.net.hostUp = true;
    w.net.edgeUp = true;
    w.now.t += 60_000;
    await w.link.now.once();
    expect(w.ran.filter((r) => r.kind === "prompt.enqueue").map((r) => [r.args.issue, r.args.intent])).toEqual([
      [500, "question"],
      [501, "directive"],
      [502, undefined],
      [500, "question"],
    ]);
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
    expect(((await note.json()) as { result: Record<string, unknown> }).result).toMatchObject({
      issue: 301,
      commented: true,
      queued: true,
    });
    expect((w.ghComments[301] ?? []).some((c) => c.body.startsWith("do this first\n\n---\n"))).toBe(true);
    expect(w.ghLabels[301]).toEqual(["owner/intake", "expedite"]);
    // As a session: the owner's prompt, with the routing labels added.
    const session = await w.act("intake.send", {
      kind: "note",
      area: "fleet",
      text: "pick these up first",
      mode: "session",
    });
    expect(((await session.json()) as { result: Record<string, unknown> }).result).toMatchObject({
      issue: 501,
      commented: false,
      queued: true,
    });
    expect(w.ghLabels[501]).toEqual(["owner/prompt", "expedite", "source/human", "owner/intake", "app/fleet"]);
    // Three tasks wait for a machine, each as the action that builds it there.
    const at = new Date(w.now.t).toISOString();
    expect((heldStored(w) as { kind: string; args: Record<string, unknown> }[]).map((h) => [h.kind, h.args])).toEqual([
      ["triage.enqueue", { issue: 500, title: body.title, rank: "now", at }],
      ["triage.enqueue", { issue: 301, title: "do this first", rank: "expedite", at, note: true }],
      ["prompt.enqueue", { issue: 501, text: "pick these up first\n\n(note, Fleet; from the PM app)", at }],
    ]);
    // What he typed is not on the record of what was done.
    expect(JSON.stringify(auditStored(w))).not.toContain("badge count");
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
    expect(heldStored(w)).toEqual([]);
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
    expect(answer.owns).toEqual(["board", "prs", "priorities", "asks", "census", "done"]);
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
    expect(heldStored(w)).toEqual([]);
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
    expect((heldStored(w) as unknown[]).length).toBe(100);
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
    const audit = auditStored(w) as { action: string; phase: string; ok?: boolean; error?: string }[];
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
    expect(await comment.json()).toEqual({ ok: true, held: true, result: null, waits: "either" });
    expect((heldStored(w) as { kind: string }[]).map((h) => h.kind)).toEqual(["issue.comment"]);
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

  // What was finished lately is read here like the rest of the core:
  // the queue's own events and GitHub's list of what is closed. The
  // oracle is the two stand-ins: what they hold is what is shown,
  // whatever a machine's copy says.
  describe("what was finished lately", () => {
    type Done = {
      done: {
        tasks: { day: number; kinds: { kind: string | null; n: number }[]; latest: Record<string, unknown>[] };
        merged: { day: number; more: boolean; latest: { number: number; by: string }[] };
        closed: { day: number; more: boolean; latest: { number: number; reason: string | null }[] };
      };
      health: Record<string, { ok: boolean; error: string | null }>;
    };
    const read = async (w: World) => (await (await w.browse("/console/api/doc/done")).json()) as Done;
    const CLOSED_LIST =
      "https://api.github.com/repos/developing-today/code/issues?state=closed&sort=updated&direction=desc&per_page=100";
    const finish = (w: World) => {
      w.queueDone.push({ id: 21001, title: "Fix the claim scan", at: "2026-11-22T11:00:00", issue: 5, pr: 77 });
      w.ghClosed.push(
        {
          number: 77,
          title: "fix: claim scan",
          html_url: "https://github.com/developing-today/code/pull/77",
          closed_at: "2026-11-22T11:00:00Z",
          updated_at: "2026-11-22T11:00:00Z",
          user: { login: "lane-a" },
          pull_request: { merged_at: "2026-11-22T11:00:00Z" },
        },
        {
          number: 5,
          title: "Claim scan is slow",
          html_url: "https://github.com/developing-today/code/issues/5",
          closed_at: "2026-11-22T11:05:00Z",
          updated_at: "2026-11-22T11:05:00Z",
          state_reason: "completed",
          user: { login: "owner" },
        },
      );
    };

    test("the history behind Trends is read and kept by Cloudflare: no machine is asked, and its own clock reads on", async () => {
      const w = await primary();
      finish(w);
      w.ghClosed.push({
        number: 78,
        title: "feat(pm-console): a screen of trends",
        html_url: "https://github.com/developing-today/code/pull/78",
        created_at: "2026-11-22T09:00:00Z",
        closed_at: "2026-11-22T10:00:00Z",
        updated_at: "2026-11-22T10:00:00Z",
        user: { login: "lane-b" },
        pull_request: { merged_at: "2026-11-22T10:00:00Z" },
      });
      // No machine: the laptop is out of reach.
      w.net.hostUp = false;
      const saw = w.hostSaw.length;
      const res = await w.browse("/console/api/trends?days=7");
      expect(res.status).toBe(200);
      const doc = (await res.json()) as {
        window: number;
        days: string[];
        history: { whole: boolean; from: string; covered: string | null };
        merged: { all: number[]; kinds: Record<string, number[]>; projects: Record<string, number[]> };
        features: { latest: Record<string, { number: number; title: string }[]> };
        issues: { closed: number[] };
        tasks: { all: number[] };
        partial?: boolean;
      };
      // The last day and two older stretches are read by this first ask; the rest is still to come.
      expect([doc.window, doc.days[doc.days.length - 1], doc.history.whole, doc.history.covered, doc.partial]).toEqual([
        7,
        "2026-11-22",
        false,
        "2026-11-15T12:00:00Z",
        undefined,
      ]);
      const last = (list: number[]) => list[list.length - 1];
      // Two pull requests merged that day (a fix and a feature), one issue closed, one task finished.
      expect([last(doc.merged.all), last(doc.merged.kinds.feat), last(doc.merged.kinds.fix)]).toEqual([2, 1, 1]);
      expect(last(doc.merged.projects["pm-stack"])).toBe(1);
      expect(doc.features.latest["pm-stack"]).toEqual([
        {
          number: 78,
          title: "a screen of trends",
          at: "2026-11-22T10:00:00Z",
          url: "https://github.com/developing-today/code/pull/78",
        },
      ] as never);
      expect(last(doc.issues.closed)).toBe(1);
      expect(last(doc.tasks.all)).toBe(1);
      expect(w.hostSaw.length).toBe(saw);
      // GitHub was asked for everything changed in the last day, then for two older stretches, each
      // up to where the one after it began.
      const history = () =>
        w.githubCalls.filter((c) => c.url.includes("since=")).map((c) => new URL(c.url).searchParams.get("since"));
      expect(history()).toEqual(["2026-11-21T12:00:00Z", "2026-11-19T12:00:00Z", "2026-11-15T12:00:00Z"]);
      // What it read is in the object's own storage: where it has read to, and a day.
      const kept = [...w.stored.keys()].filter((k) => k.startsWith("trend:")).sort();
      expect(kept).toEqual(["trend:day:2026-11-22", "trend:meta"]);
      const meta = () => JSON.parse(w.stored.get("trend:meta") as string) as Record<string, unknown>;
      expect([meta().from, meta().cursor, meta().live, meta().whole]).toEqual([
        "2026-08-24",
        "2026-11-22T11:05:00Z",
        true,
        false,
      ]);
      // Asked for again before then, it answers from what it has: nothing is read on behind the
      // answer, where it could be cut off part-way. Its clock does that.
      const sooner = (await (await w.browse("/console/api/trends?days=7")).json()) as typeof doc;
      await new Promise((resolve) => setTimeout(resolve, 20));
      expect([sooner.history.whole, history().length]).toEqual([false, 3]);
      // The screen was asked for, so the object's own clock reads the rest on, starting at once: a
      // look at what is new and five pages of older stretches, which is all of them.
      expect(w.alarm.at).toBe(w.now.t + 1000);
      w.now.t += 1000;
      await w.edge.now.alarm();
      expect(history().slice(3)).toEqual([
        "2026-11-22T11:05:00Z",
        "2026-11-08T12:00:00Z",
        "2026-10-23T12:00:00Z",
        "2026-09-23T12:00:00Z",
        "2026-08-24T00:00:00Z",
      ]);
      expect([meta().whole, meta().back, meta().readAt]).toEqual([true, null, new Date(w.now.t).toISOString()]);
      // Whole, it wakes for it again within half an hour (sooner if something else is due, as the
      // laptop going quiet is here); a wake before then asks GitHub nothing of it, and neither does
      // the screen asked for again.
      expect(w.alarm.at).toBeLessThanOrEqual(w.now.t + 30 * 60_000);
      w.now.t += 60_000;
      await w.edge.now.alarm();
      expect(history().length).toBe(8);
      const whole = (await (await w.browse("/console/api/trends?days=7")).json()) as typeof doc;
      expect([whole.history.whole, whole.history.covered]).toEqual([true, "2026-08-24T00:00:00Z"]);
      expect(history().length).toBe(8);
      // Something merges. Nobody opens the screen; half an hour on, the object's own clock reads on
      // from where it stopped, and the screen has it when it is next opened.
      w.ghClosed.push({
        number: 79,
        title: "feat(id): later",
        html_url: "https://github.com/developing-today/code/pull/79",
        created_at: "2026-11-22T12:10:00Z",
        closed_at: "2026-11-22T12:20:00Z",
        updated_at: "2026-11-22T12:20:00Z",
        pull_request: { merged_at: "2026-11-22T12:20:00Z" },
      });
      w.now.t += 31 * 60_000;
      await w.edge.now.alarm();
      expect(history().length).toBe(9);
      expect(history()[8]).toBe("2026-11-22T11:05:00Z");
      const day = JSON.parse(w.stored.get("trend:day:2026-11-22") as string) as { pm: Record<string, unknown[]> };
      expect(Object.keys(day.pm).sort()).toEqual(["77", "78", "79"]);
      expect(day.pm["79"]).toEqual(["feat", "id", 10, "12:20", "later"]);
      // It reads on like that only while somebody looks at the screen. A day after it was last
      // asked for, a wake reads nothing of it: an object nobody needs is not kept awake.
      w.now.t += 25 * 60 * 60_000;
      await w.edge.now.alarm();
      await w.edge.now.alarm();
      expect(history().length).toBe(9);
      // Opened again, it is read on at once (and by the clock again after).
      await w.browse("/console/api/trends?days=7");
      expect(history().length).toBe(10);
    });

    test("it comes from the queue store and GitHub, laptop or no laptop", async () => {
      const w = await primary();
      finish(w);
      // The laptop is reporting, and its copy says nothing was finished.
      const doc = await read(w);
      expect(doc.done.tasks).toEqual({
        day: 1,
        kinds: [{ kind: null, n: 1 }],
        latest: [
          {
            id: 21001,
            title: "Fix the claim scan",
            at: "2026-11-22T11:00:00",
            lane: null,
            model: null,
            kind: null,
            issue: 5,
            pr: 77,
            verdict: null,
            url: "https://github.com/developing-today/code/issues/5",
            prUrl: "https://github.com/developing-today/code/pull/77",
          },
        ],
      });
      expect([doc.done.merged.day, doc.done.merged.more, doc.done.merged.latest.map((m) => [m.number, m.by])]).toEqual([
        1,
        false,
        [[77, "lane-a"]],
      ]);
      expect(doc.done.closed.latest.map((c) => [c.number, c.reason])).toEqual([[5, "completed"]]);
      expect(doc.health.done).toEqual({ ok: true, error: null });
      // Read from the two of them, by the platform's own settings and
      // this side's own clock; the laptop was asked for nothing.
      expect(onlyPokes(w)).toBe(true);
      expect(w.githubCalls.map((c) => c.url)).toContain(CLOSED_LIST);
      const stamp = (ms: number) => new Date(ms).toISOString().replace(/\.\d+Z$/, "Z");
      expect(w.queueCalls.filter((c) => c.sql === DONE_TASKS_SQL).map((c) => c.args)).toEqual([
        [stamp(START - 7 * 24 * 3_600_000), platformSettings().queue],
      ]);
      expect(w.queueCalls.filter((c) => c.sql === DONE_COUNT_SQL).map((c) => c.args)).toEqual([
        [stamp(START - 24 * 3_600_000), platformSettings().queue],
      ]);
      // And with the laptop gone for good, it is still read, and what is new is shown.
      w.net.hostUp = false;
      w.net.edgeUp = false;
      w.queueDone.unshift({ id: 21002, title: "Another", at: "2026-11-22T12:01:00", issue: null, pr: null });
      w.now.t += 121_000;
      await read(w);
      await settle(w);
      expect((await read(w)).done.tasks.latest.map((t) => t.id)).toEqual([21002, 21001]);
    });

    test("turned away for GitHub's list, it is a machine's to read for the hour, and said so", async () => {
      const w = await primary();
      finish(w);
      const refused = async () =>
        ((await (await w.browse("/console/health", { token: null })).json()) as { refused: string[] }).refused;
      const lists = () => w.githubCalls.filter((c) => c.url === CLOSED_LIST).length;
      expect((await read(w)).done.merged.latest.map((m) => m.number)).toEqual([77]);
      await settle(w);
      w.ghRefuses.push((method, url) => method === "GET" && url.includes("state=closed"));
      w.now.t += 121_000;
      await read(w);
      await settle(w);
      expect(await refused()).toEqual(["read.done"]);
      // Until a machine's copy comes, the copy read here is shown as one
      // that could not be read again; GitHub is not asked again for it
      // within the hour, and it is said to have been turned away.
      const waiting = await read(w);
      expect(waiting.done.merged.latest.map((m) => m.number)).toEqual([77]);
      expect(waiting.health.done.ok).toBe(false);
      const asked = lists();
      w.now.t += 121_000;
      await read(w);
      await settle(w);
      const still = await read(w);
      expect(lists()).toBe(asked);
      expect(still.done.merged.latest.map((m) => m.number)).toEqual([77]);
      expect(still.health.done).toEqual({ ok: false, error: expect.stringContaining("turned away") });
      // The laptop is told it is not read here, and its next report brings its own.
      w.now.t += 1_000;
      expect((await w.link.now.once())?.owns).toEqual(["board", "prs", "priorities", "asks", "census"]);
      w.now.t += 1_000;
      await w.link.now.once();
      const theirs = await read(w);
      expect(theirs.done).toEqual(w.data.done as never);
      expect(theirs.health.done).toEqual({ ok: true, error: null });
    });
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
    expect((await w.link.now.once())?.owns).toEqual(["board", "priorities", "asks", "census", "done"]);
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
    expect((await w.link.now.once())?.owns).toEqual(["board", "prs", "priorities", "asks", "census", "done"]);
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
    expect((await w.link.now.once())?.owns).toEqual(["board", "priorities", "asks", "census", "done"]);
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
      // A year was written down for one, and the longest there can be for another.
      reads: { prs: w.now.t + 365 * 24 * 3600_000, priorities: w.now.t + 65 * 60_000, board: w.now.t - 1 },
    });
    // Each time by an object that has just started, as in service. The year is not believed
    // by any of them: one that cut it down to the longest would cut it down afresh each time,
    // and it would never come.
    const fresh = async () => {
      w.edge.now = w.makeEdge();
      return health();
    };
    expect(await fresh()).toEqual(["pr.merge", "read.priorities"]);
    w.now.t += 11 * 60_000;
    expect(await fresh()).toEqual(["read.priorities"]);
    w.now.t += 53 * 60_000;
    expect(await fresh()).toEqual(["read.priorities"]);
    w.now.t += 60_000 + 1;
    expect(await fresh()).toEqual([]);
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

  /** The document pull requests and ranked issues are in. */
  const opsDoc = async (w: World) =>
    (await (await w.browse("/console/api/doc/ops")).json()) as {
      prs: unknown;
      priorities: unknown;
      health: Record<string, { ok: boolean; error: string | null }>;
      read: Record<string, string | null>;
      good: Record<string, string | null>;
    };
  const githubSaid = async (w: World) =>
    (
      (await (await w.browse("/console/health", { token: null })).json()) as {
        github: {
          answers: number;
          unchanged: number;
          kept: number;
          quiet: { core: number | null; search: number | null };
        };
      }
    ).github;

  test('a list that has not changed is asked for by its tag: GitHub is asked as often, says "not modified", and what is shown is the same', async () => {
    const w = await primary();
    const list = () => w.githubCalls.filter((c) => /\/pulls\?/.test(c.url));
    const pulls = () => list().length;
    const unchanged = () => list().filter((c) => c.unchanged).length;
    const first = await opsDoc(w);
    await settle(w);
    expect(JSON.stringify(first.prs)).toContain("Fresh PR");
    expect([pulls(), unchanged()]).toEqual([1, 0]);
    // Past the time the list of pull requests is read again.
    w.now.t += 91_000;
    await opsDoc(w);
    await settle(w);
    const second = await opsDoc(w);
    await settle(w);
    // It was asked for once more, by its tag, and that cost nothing of the hour's allowance.
    expect([pulls(), unchanged()]).toEqual([2, 1]);
    expect(second.prs).toEqual(first.prs);
    // The list changes at GitHub: the tag no longer fits, the answer is a full one, and it is what is shown.
    w.ghPrs.push({
      ...w.ghPrs[0],
      number: 56,
      title: "Another PR",
      html_url: "https://github.com/developing-today/code/pull/56",
      head: { ref: "queue/56", sha: "5656565656565656565656565656565656565656" },
    });
    w.now.t += 91_000;
    await opsDoc(w);
    await settle(w);
    const third = await opsDoc(w);
    await settle(w);
    expect([pulls(), unchanged()]).toEqual([3, 1]);
    expect(JSON.stringify(third.prs)).toContain("Another PR");
    // Health says it from outside: every answer of GitHub's, and how many were "not modified".
    const said = await githubSaid(w);
    await settle(w);
    expect([said.unchanged, said.answers]).toEqual([w.ghTags.unchanged, w.githubCalls.length]);
    expect([said.kept > 0, said.quiet]).toEqual([true, { core: null, search: null }]);
    // A GitHub that gave no tags is the same to look at: every answer a full one.
    const plain = await primary();
    plain.ghTags.on = false;
    const a = await opsDoc(plain);
    await settle(plain);
    plain.now.t += 91_000;
    await opsDoc(plain);
    await settle(plain);
    expect((await opsDoc(plain)).prs).toEqual(a.prs);
    expect(plain.ghTags.unchanged).toBe(0);
  });

  test("the object put away between two reads of a list, as Cloudflare does with an idle one: the list is still asked for by its tag, and GitHub's wait is still kept to", async () => {
    const w = await primary();
    const list = () => w.githubCalls.filter((c) => /\/pulls\?/.test(c.url));
    const first = await opsDoc(w);
    await settle(w);
    expect([list().length, list().filter((c) => c.unchanged).length]).toEqual([1, 0]);
    const health = async () =>
      (await (await w.browse("/console/health", { token: null })).json()) as {
        woke: number;
        github: { answers: number; unchanged: number; kept: number; since: number; quiet: { core: number | null } };
      };
    const before = await health();
    await settle(w);
    // (Health's own check of each read, the first time it is asked, read the list once more.)
    const sofar = [list().length, list().filter((c) => c.unchanged).length];
    expect(sofar).toEqual([2, 1]);
    // Put away, and another started in its place a minute and a half later: nothing in
    // memory, the same storage. (Kept in memory, as this was first built, the answer was gone
    // by now every time, and every read was a full one.)
    w.now.t += 91_000;
    w.edge.now = w.makeEdge();
    await opsDoc(w);
    await settle(w);
    const second = await opsDoc(w);
    await settle(w);
    expect([list().length, list().filter((c) => c.unchanged).length]).toEqual([sofar[0] + 1, sofar[1] + 1]);
    expect(second.prs).toEqual(first.prs);
    const after = await health();
    await settle(w);
    // Another object (it says when it started), going on with the same count.
    expect([after.woke, after.woke > before.woke, after.github.since]).toEqual([w.now.t, true, before.github.since]);
    expect(after.github.answers).toBeGreaterThan(before.github.answers);
    expect(after.github.unchanged).toBeGreaterThan(before.github.unchanged);
    expect(after.github.kept).toBeGreaterThanOrEqual(before.github.kept);
    // Turned away with the hour used up, and put away again during the wait: the next object
    // asks GitHub for nothing counted by the hour either.
    w.net.hostUp = false;
    w.net.edgeUp = false;
    w.now.t += 181_000;
    const back = Math.floor((w.now.t + 40 * 60_000) / 1000) * 1000;
    w.ghLimits.push((_method, url) => !url.includes("/search/"));
    w.ghLimitSays.headers = { "x-ratelimit-remaining": "0", "x-ratelimit-reset": String(back / 1000) };
    await opsDoc(w);
    await w.core();
    await settle(w);
    const hourly = () => w.githubCalls.filter((c) => !c.url.includes("/search/")).length;
    const asked = hourly();
    w.ghLimits.length = 0;
    for (let minute = 0; minute < 36; minute += 6) {
      w.now.t += 6 * 60_000;
      w.edge.now = w.makeEdge();
      await opsDoc(w);
      await w.core();
      await settle(w);
      expect([minute, (await health()).github.quiet.core, hourly()]).toEqual([minute, back, asked]);
      await settle(w);
    }
    // The time comes: an object started after it asks again.
    w.now.t = back + 181_000;
    w.edge.now = w.makeEdge();
    await opsDoc(w);
    await w.core();
    await settle(w);
    expect(hourly()).toBeGreaterThan(asked);
    expect((await health()).github.quiet.core).toBeNull();
  });

  test("the hour's allowance used up: GitHub's own time is kept to, nothing that draws on it is asked until then, and the last good copy says why and when", async () => {
    const w = await primary();
    const first = { core: (await w.core()).doc, ops: await opsDoc(w) };
    await settle(w);
    // No machine either: what this side kept is all there is to show.
    w.net.hostUp = false;
    w.net.edgeUp = false;
    // GitHub: the account's allowance for the hour is used up, and comes back in forty minutes.
    // Searches it counts apart, and those it still answers.
    w.now.t += 181_000;
    const back = Math.floor((w.now.t + 40 * 60_000) / 1000) * 1000;
    w.ghLimits.push((_method, url) => !url.includes("/search/"));
    w.ghLimitSays.headers = { "x-ratelimit-remaining": "0", "x-ratelimit-reset": String(back / 1000) };
    const hourly = () => w.githubCalls.filter((c) => !c.url.includes("/search/")).length;
    const searches = () => w.githubCalls.length - hourly();
    const before = { hourly: hourly(), searches: searches() };
    const look = async () => {
      await w.core();
      await opsDoc(w);
      await settle(w);
      const seen = { core: (await w.core()).doc, ops: await opsDoc(w) };
      await settle(w);
      return seen;
    };
    const limited = await look();
    // It was told by what it asked at once: the first call of each of the two lists the first
    // screen reads from GitHub (the prompts, and what needs the owner). Nothing else that
    // draws on the hour was asked: not the rest of either list, not the other screen's list,
    // and not the second look.
    expect(hourly()).toBe(before.hourly + 2);
    expect([limited.ops.prs, limited.ops.priorities, limited.core.asks]).toEqual([
      first.ops.prs,
      first.ops.priorities,
      first.core.asks,
    ]);
    for (const section of [limited.ops.health.prs, limited.core.health.asks]) {
      expect(section.ok).toBe(false);
      expect(section.error).toContain(`GitHub says to ask again at ${new Date(back).toISOString()}`);
      expect(section.error).toMatch(/API rate limit exceeded/);
    }
    // How old the copy shown is, is when it was read, which the failed tries leave alone;
    // when each was last tried has moved on.
    expect([limited.ops.good.prs, limited.core.good.asks]).toEqual([first.ops.good.prs, first.core.good.asks]);
    expect(limited.ops.good.prs).toBe(first.ops.read.prs);
    expect(Date.parse(limited.ops.read.prs ?? "")).toBeGreaterThan(Date.parse(first.ops.read.prs ?? ""));
    // Ranked issues are a search: read as ever.
    expect(limited.ops.health.priorities.ok).toBe(true);
    expect(searches()).toBeGreaterThan(before.searches);
    // Twenty minutes with nobody looking: the two lists are still not this side's to read, and
    // it has not claimed them back in between only to be turned away again.
    const refused = async () =>
      ((await (await w.browse("/console/health", { token: null })).json()) as { refused: string[] }).refused;
    w.now.t += 20 * 60_000;
    expect(await refused()).toEqual(["read.asks", "read.census", "read.prs"]);
    await settle(w);
    // Something the owner does meanwhile is not sent either: it is kept, not lost.
    const tried = await w.act("issue.rank", { issue: 301, rank: "next" });
    expect(await tried.json()).toMatchObject({ ok: true, held: true });
    expect(hourly()).toBe(before.hourly + 2);
    // That kind of action is left to a machine for five minutes, as when no time is given,
    // and not for the rest of the hour: what failed beside a call held back may have
    // failed for a reason of its own. (Tried again here, it is held back again.)
    expect(await refused()).toEqual(["issue.rank", "read.asks", "read.census", "read.prs"]);
    w.now.t += 5 * 60_000 + 1_000;
    expect(await refused()).toEqual(["read.asks", "read.census", "read.prs"]);
    await settle(w);
    // For the rest of the forty: screens looked at, minutes passing, the check that health
    // makes. None of it asks GitHub for anything counted by the hour, and what is held back
    // here is not counted as an answer of GitHub's.
    for (let minute = 28; minute < 40; minute += 3) {
      w.now.t += 3 * 60_000;
      await look();
      const said = await githubSaid(w);
      await settle(w);
      expect([minute, said.quiet, said.answers]).toEqual([minute, { core: back, search: null }, w.githubCalls.length]);
      expect([minute, hourly()]).toEqual([minute, before.hourly + 2]);
    }
    // The time comes, and GitHub answers again: it is asked, and what it says is shown with nothing wrong.
    w.ghLimits.length = 0;
    w.now.t = back + 181_000;
    const after = await look();
    expect(hourly()).toBeGreaterThan(before.hourly + 2);
    expect((await githubSaid(w)).quiet).toEqual({ core: null, search: null });
    expect([after.ops.health.prs.ok, after.ops.health.priorities.ok, after.core.health.asks.ok]).toEqual([
      true,
      true,
      true,
    ]);
  });

  test("turned away with no time given, this side asks again in five minutes as before; a time that cannot be right is kept to for a little over an hour, no longer", async () => {
    const w = await primary();
    await opsDoc(w);
    await settle(w);
    w.net.hostUp = false;
    w.net.edgeUp = false;
    const pulls = () => w.githubCalls.filter((c) => /\/pulls\?/.test(c.url)).length;
    const look = async () => {
      await opsDoc(w);
      await settle(w);
    };
    // No time given: nothing else is held back, and the list is left alone for five minutes
    // and read again at its next turn after them.
    w.now.t += 91_000;
    w.ghLimits.push((method, url) => method === "GET" && /\/pulls\?/.test(url));
    await look();
    expect(pulls()).toBe(2);
    // (Health makes its own check of each read the first time it is asked: one more.)
    expect((await githubSaid(w)).quiet).toEqual({ core: null, search: null });
    await settle(w);
    const before = pulls();
    expect(before).toBe(3);
    w.now.t += 4 * 60_000;
    await look();
    expect(pulls()).toBe(before);
    w.now.t += 2 * 60_000;
    await look();
    expect(pulls()).toBe(before + 1);
    // A time a day off: kept to for a little over an hour, no longer.
    w.ghLimitSays.headers = { "retry-after": String(24 * 60 * 60) };
    w.now.t += 5 * 60_000 + 1_000;
    const told = w.now.t;
    await look();
    expect(pulls()).toBe(before + 2);
    // (Said in seconds, it is said of the caller: searches are hushed with the rest.)
    expect((await githubSaid(w)).quiet).toEqual({ core: told + 65 * 60_000, search: told + 65 * 60_000 });
    // And the owner saying "try again now" is taken at their word: the list is asked for at
    // its next turn, not an hour on.
    await w.act("edge.retry", {});
    w.ghLimits.length = 0;
    w.now.t += 91_000;
    await look();
    expect(pulls()).toBe(before + 3);
    expect((await githubSaid(w)).quiet).toEqual({ core: null, search: null });
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
    expect((await w.link.now.once())?.owns).toEqual(["board", "prs", "asks", "census", "done"]);
    w.ghLimits.length = 0;
    w.now.t += 4 * 60_000;
    expect(await health()).toEqual(["read.priorities"]);
    w.now.t += 61_000;
    expect(await health()).toEqual([]);
    await w.hostReads();
    expect((await w.link.now.once())?.owns).toEqual(["board", "prs", "priorities", "asks", "census", "done"]);
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
    expect(Object.keys(selfCheck.kinds).sort()).toEqual(["asks", "board", "census", "done", "priorities", "prs"]);
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
    expect(await held.json()).toEqual({ ok: true, held: true, result: null, waits: "machine" });
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

  // Until #14163 a laptop too old to say where the queue is left the
  // Worker serving its reports and doing nothing itself. The Worker
  // knows where things are by its own bundle now: it needs a machine to
  // have said who may sign in, and nothing else.
  test("a laptop too old to say where the queue is: the Worker does the core itself all the same, by main's settings", async () => {
    const w = await setup({ direct: true });
    // What an older laptop reports: the same settings, without the
    // fields acting here needed. That shape still validates, with those empty.
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
    // The laptop goes out of reach: the board is the queue store's own word, read here.
    w.net.hostUp = false;
    w.net.edgeUp = false;
    const { res, doc } = await w.core();
    expect(res.status).toBe(200);
    expect(doc.edge).toEqual({ held: 0, primary: true });
    expect(doc.health.board).toEqual({ ok: true, error: null });
    expect(doc.board.counts).toEqual({ claimed: 1, queued: 1, blocked: 1 });
    expect(doc.asks.map((a: { issue: number }) => a.issue)).toEqual([202, 201]);
    expect(w.strays).toEqual([]);
    const health = (await (await w.browse("/console/health", { token: null })).json()) as Record<string, unknown>;
    expect([health.configured, health.primary, health.directReady]).toEqual([true, true, true]);
    expect(health.machineSettingsDiffer).toEqual(["queueDbUrl", "queueTokenName", "promptLabel", "needsRepos"]);
    // A machine that names no repositories goes on reporting what needs
    // the owner itself: the Worker does not read that from none.
    expect(doc.census).toEqual(w.data.census);
    expect(w.githubCalls.filter((call) => /labels=user%2F/.test(call.url))).toEqual([]);
    // And an action is done here, not held for the laptop.
    expect(await (await w.act("issue.comment", { issue: 14, text: "late" })).json()).toMatchObject({ ok: true });
    expect(w.githubCalls.at(-1)).toMatchObject({ method: "POST", url: expect.stringContaining("/issues/14/comments") });
    expect(heldStored(w)).toEqual([]);
  });

  test("an older held action the laptop never took is done here first, under its own name", async () => {
    const w = await primary();
    const comment = (method: string, url: string) => method === "POST" && /\/issues\/\d+\/comments$/.test(url);
    // This side is turned away for comments, so the older one waits.
    w.ghRefuses.push(comment);
    w.net.hostUp = false;
    w.net.edgeUp = false;
    const olderId = "0f000000-0000-4000-8000-000000000001";
    expect(await (await w.act("issue.comment", { issue: 7, text: "older" }, olderId)).json()).toEqual({
      ok: true,
      held: true,
      result: null,
      waits: "either",
    });
    expect((heldStored(w) as { id: string }[]).map((h) => h.id)).toEqual([olderId]);
    // The token is put right and this side takes comments back.
    w.ghRefuses.length = 0;
    w.net.hostUp = true;
    w.net.edgeUp = true;
    expect((await w.act("edge.retry", {})).status).toBe(200);
    // A newer comment about the same issue: the older one the laptop
    // never took is done here first, under its own name, then the newer.
    const newerId = "0f000000-0000-4000-8000-000000000002";
    const second = await w.act("issue.comment", { issue: 7, text: "newer" }, newerId);
    const done = (await second.json()) as { ok?: boolean; held?: boolean };
    expect(done).toMatchObject({ ok: true });
    expect(done.held).toBeUndefined();
    const posted = w.githubCalls
      .filter((c) => comment(c.method, c.url))
      .map((c) => (JSON.parse(c.body ?? "{}") as { body?: string }).body);
    // The refused first try is recorded too: the two that count come last, in order.
    expect(posted.slice(-2)).toEqual(["older", "newer"]);
    expect(heldStored(w)).toEqual([]);
    expect(w.stored.get(`a:${olderId}`)).toMatchObject({ kind: "issue.comment", state: "done" });
    expect(w.stored.get(`a:${newerId}`)).toMatchObject({ kind: "issue.comment", state: "done" });
    expect(handedStored(w)).toEqual([]);
  });

  test("a held rank is replaced by a newer rank about the same issue: the last one wins", async () => {
    const w = await primary();
    const labels = (method: string, url: string) => method === "POST" && /\/issues\/201\/labels$/.test(url);
    // This side is turned away for labelling, so the older rank waits.
    w.ghRefuses.push(labels);
    w.net.hostUp = false;
    w.net.edgeUp = false;
    const olderId = "0f000000-0000-4000-8000-000000000011";
    expect(await (await w.act("issue.rank", { issue: 201, rank: "next" }, olderId)).json()).toEqual({
      ok: true,
      held: true,
      result: null,
      waits: "either",
    });
    expect((heldStored(w) as { id: string }[]).map((h) => h.id)).toEqual([olderId]);
    // The token is put right and this side takes ranking back.
    w.ghRefuses.length = 0;
    w.net.hostUp = true;
    w.net.edgeUp = true;
    expect((await w.act("edge.retry", {})).status).toBe(200);
    // A newer rank about the same issue: the older one is dropped as
    // replaced, and only the newer is done.
    const newerId = "0f000000-0000-4000-8000-000000000012";
    const second = await w.act("issue.rank", { issue: 201, rank: "now" }, newerId);
    expect(await second.json()).toMatchObject({ ok: true });
    expect(heldStored(w)).toEqual([]);
    expect(w.stored.get(`a:${olderId}`)).toMatchObject({ kind: "issue.rank", state: "done", status: 409 });
    expect(w.stored.get(`a:${newerId}`)).toMatchObject({ kind: "issue.rank", state: "done" });
    expect(w.ghLabels[201]).toContain("now");
    expect(w.ghLabels[201]).not.toContain("next");
  });

  test("an older held action handed to the laptop blocks a newer one about the same thing", async () => {
    const w = await primary();
    const comment = (method: string, url: string) => method === "POST" && /\/issues\/\d+\/comments$/.test(url);
    // This side is turned away for comments, so the older one waits.
    w.ghRefuses.push(comment);
    w.net.hostUp = false;
    w.net.edgeUp = false;
    const olderId = "0f000000-0000-4000-8000-000000000021";
    expect(await (await w.act("issue.comment", { issue: 7, text: "older" }, olderId)).json()).toEqual({
      ok: true,
      held: true,
      result: null,
      waits: "either",
    });
    // The token is put right and this side takes comments back.
    w.ghRefuses.length = 0;
    w.net.hostUp = true;
    w.net.edgeUp = true;
    expect((await w.act("edge.retry", {})).status).toBe(200);
    // The laptop reports and is handed the older one, which it does.
    await w.hostReads();
    const answer = await w.link.now.once();
    expect(answer?.commands.map((c) => c.id)).toEqual([olderId]);
    expect(w.ran.filter((r) => r.kind === "issue.comment")).toHaveLength(1);
    expect(handedStored(w)).toEqual([olderId]);
    // A newer comment about the same issue waits behind the handed one:
    // it is kept, not done here.
    const newerId = "0f000000-0000-4000-8000-000000000022";
    const second = await w.act("issue.comment", { issue: 7, text: "newer" }, newerId);
    expect(await second.json()).toEqual({ ok: true, held: true, result: null, waits: "machine" });
    expect((heldStored(w) as { id: string }[]).map((h) => h.id)).toEqual([olderId, newerId]);
    // Nothing was done here: the laptop did the older, the newer waits.
    // The only call this side made was the older one's refused try.
    expect(w.githubCalls.filter((c) => comment(c.method, c.url)).map((c) => c.body)).toEqual(['{"body":"older"}']);
    expect(w.ran.filter((r) => r.kind === "issue.comment")).toHaveLength(1);
  });

  test("a newer action about something else is not held behind an older one", async () => {
    const w = await primary();
    const comment = (method: string, url: string) => method === "POST" && /\/issues\/\d+\/comments$/.test(url);
    // This side is turned away for comments, so the older one waits.
    w.ghRefuses.push(comment);
    w.net.hostUp = false;
    w.net.edgeUp = false;
    const olderId = "0f000000-0000-4000-8000-000000000031";
    expect(await (await w.act("issue.comment", { issue: 7, text: "older" }, olderId)).json()).toEqual({
      ok: true,
      held: true,
      result: null,
      waits: "either",
    });
    // The token is put right and this side takes comments back.
    w.ghRefuses.length = 0;
    w.net.hostUp = true;
    w.net.edgeUp = true;
    expect((await w.act("edge.retry", {})).status).toBe(200);
    // A newer comment about another issue goes straight through.
    const newerId = "0f000000-0000-4000-8000-000000000032";
    const second = await w.act("issue.comment", { issue: 8, text: "unrelated" }, newerId);
    const done = (await second.json()) as { ok?: boolean; held?: boolean };
    expect(done).toMatchObject({ ok: true });
    expect(done.held).toBeUndefined();
    expect((heldStored(w) as { kind: string; args: Record<string, unknown> }[]).map((h) => h.args.issue)).toEqual([7]);
    const posted = w.githubCalls
      .filter((c) => comment(c.method, c.url))
      .map((c) => (JSON.parse(c.body ?? "{}") as { body?: string }).body);
    expect(posted.slice(-1)).toEqual(["unrelated"]);
    expect(w.stored.get(`a:${newerId}`)).toMatchObject({ kind: "issue.comment", state: "done" });
  });

  // ---- what is held, done here when this side can again (#13831) -----------

  const COMMENT = (method: string, url: string) => method === "POST" && /\/issues\/\d+\/comments$/.test(url);
  type HeldWorld = Awaited<ReturnType<typeof primary>>;
  /** The words of every comment asked of GitHub, in order (refused ones too). */
  const commented = (w: HeldWorld) =>
    w.githubCalls
      .filter((c) => COMMENT(c.method, c.url))
      .map((c) => (JSON.parse(c.body ?? "{}") as { body?: string }).body);
  const heldIds = (w: HeldWorld) => (heldStored(w) as { id: string }[]).map((h) => h.id);
  /** What is held, each with who it waits for, as the Machines screen is told it. */
  const heldWaits = async (w: HeldWorld) =>
    ((await (await w.browse("/console/api/machines")).json()) as MachinesAnswer).held.map((h) => [h.id, h.waits]);
  /** This side wakes itself: at the time it asked for, and (as in service) as an object that has just started. */
  const wake = async (w: HeldWorld) => {
    w.now.t = Math.max(w.now.t, w.alarm.at);
    w.edge.now = w.makeEdge();
    await w.edge.now.alarm();
    await settle(w);
  };
  /** Turned away for comments with every machine away: each comment asked for is held. */
  const heldComments = async (w: HeldWorld, comments: [id: string, issue: number, text: string][]) => {
    w.ghRefuses.push(COMMENT);
    w.net.hostUp = false;
    w.net.edgeUp = false;
    for (const [id, issue, text] of comments) {
      expect(await (await w.act("issue.comment", { issue, text }, id)).json()).toEqual({
        ok: true,
        held: true,
        result: null,
        waits: "either",
      });
      w.now.t += 1_000;
    }
  };
  const ID = (n: number) => `0f000000-0000-4000-8000-${String(100_000 + n).padStart(12, "0")}`;

  test("what is held and never reached a machine is done here when this side can again: with no machine, once, and the owner sees it done", async () => {
    const w = await primary();
    const t0 = w.now.t;
    await heldComments(w, [[ID(1), 7, "held, then done here"]]);
    expect(w.stored.get(`a:${ID(1)}`)).toMatchObject({ state: "given" });
    expect(commented(w)).toEqual(["held, then done here"]);
    // The token is put right. Nothing tells this side so: it finds out when its hour for the
    // kind is up, and it has asked to be woken then. A wake before that (to notice the laptop
    // gone quiet) asks GitHub nothing about it.
    w.ghRefuses.length = 0;
    await wake(w);
    expect(w.now.t).toBeLessThan(t0 + 60 * 60_000);
    expect([commented(w).length, heldIds(w)]).toEqual([1, [ID(1)]]);
    await wake(w);
    expect(w.now.t).toBe(t0 + 60 * 60_000);
    // Done here: asked of GitHub once more and no more, by no machine.
    expect(commented(w)).toEqual(["held, then done here", "held, then done here"]);
    expect([heldIds(w), takenStored(w), handedStored(w), w.ran]).toEqual([[], [], [], []]);
    expect(w.stored.get(`a:${ID(1)}`)).toMatchObject({ kind: "issue.comment", state: "done", status: 200 });
    const { doc } = await w.core();
    expect(doc.edge).toEqual({ held: 0, primary: true });
    expect(doc.acts.find((a: { id: string }) => a.id === ID(1))).toMatchObject({
      kind: "issue.comment",
      status: "done",
    });
    // Sent again under its name, it is answered from the record, not done again; and a
    // machine that comes back is handed nothing.
    const again = (await (await w.act("issue.comment", { issue: 7, text: "held, then done here" }, ID(1))).json()) as {
      again?: boolean;
    };
    expect([again.again, commented(w).length]).toEqual([true, 2]);
    w.net.hostUp = true;
    w.net.edgeUp = true;
    await w.hostReads();
    expect((await w.link.now.once())?.commands).toEqual([]);
    expect(w.ran).toEqual([]);
  });

  test("kept with the laptop long gone and nobody to tell, it still asks to be woken for it: nothing else would wake it", async () => {
    const w = await primary();
    w.net.hostUp = false;
    w.net.edgeUp = false;
    // The laptop went quiet, and that has been said. Nothing is left to wake this side.
    await wake(w);
    expect(w.stored.get("silent")).toBe(true);
    const idle = w.alarm.at;
    w.now.t += 3 * 60 * 60_000;
    const kept = w.now.t;
    await heldComments(w, [[ID(91), 7, "kept late at night"]]);
    expect(idle).toBeLessThan(kept);
    expect(w.alarm.at).toBe(kept + 60 * 60_000);
    w.ghRefuses.length = 0;
    await wake(w);
    expect([commented(w), heldIds(w)]).toEqual([["kept late at night", "kept late at night"], []]);
    // With nothing held, and nothing else to wake for, it asks for no wake at all.
    const after = w.alarm.at;
    await wake(w);
    expect(w.alarm.at).toBe(after);
  });

  test("turned away for calling too often, what is held is done at the time GitHub gave and not before", async () => {
    const w = await primary();
    const t0 = w.now.t;
    const back = Math.floor((t0 + 40 * 60_000) / 1000) * 1000;
    w.ghLimits.push(COMMENT);
    w.ghLimitSays.headers = { "x-ratelimit-remaining": "0", "x-ratelimit-reset": String(back / 1000) };
    w.net.hostUp = false;
    w.net.edgeUp = false;
    expect(await (await w.act("issue.comment", { issue: 7, text: "when GitHub answers" }, ID(2))).json()).toEqual({
      ok: true,
      held: true,
      result: null,
      waits: "either",
    });
    w.ghLimits.length = 0;
    const asked = w.githubCalls.length;
    // Woken before the time (for the laptop gone quiet): nothing is asked of GitHub at all.
    await wake(w);
    expect([w.now.t < back, w.githubCalls.length, heldIds(w)]).toEqual([true, asked, [ID(2)]]);
    // Woken again half a minute before the time: still nothing, and its next wake is the time
    // itself (a pass that found nothing due does not put the next one off).
    w.now.t = back - 30_000;
    w.edge.now = w.makeEdge();
    await w.edge.now.alarm();
    expect([w.githubCalls.length, w.alarm.at]).toEqual([asked, back]);
    // Its next wake is the time GitHub gave, and then it is done.
    await wake(w);
    expect(w.now.t).toBe(back);
    expect(commented(w)).toEqual(["when GitHub answers", "when GitHub answers"]);
    expect([heldIds(w), w.ran]).toEqual([[], []]);
    expect(w.stored.get(`a:${ID(2)}`)).toMatchObject({ state: "done", status: 200 });
  });

  test("done in the order they were done: two about one thing one after the other, and one about another thing not kept behind them", async () => {
    const w = await primary();
    await heldComments(w, [
      [ID(11), 7, "first about seven"],
      [ID(12), 8, "about eight"],
      [ID(13), 7, "second about seven"],
    ]);
    w.ghRefuses.length = 0;
    await wake(w);
    await wake(w);
    expect(commented(w).slice(1)).toEqual(["first about seven", "about eight", "second about seven"]);
    expect([heldIds(w), w.ran]).toEqual([[], []]);
  });

  test("one a machine has been handed is the machine's: this side does not do it, nor anything else about the same thing", async () => {
    const w = await primary();
    await heldComments(w, [
      [ID(21), 7, "handed"],
      [ID(22), 9, "about nine"],
    ]);
    // The laptop comes back and is handed both. It goes again before saying what became of them.
    w.net.hostUp = true;
    w.net.edgeUp = true;
    await w.hostReads();
    expect((await w.link.now.once())?.commands.map((c) => c.id)).toEqual([ID(21), ID(22)]);
    expect(handedStored(w)).toEqual([ID(21), ID(22)]);
    w.net.hostUp = false;
    w.net.edgeUp = false;
    // A later one about the same thing as a handed one waits behind it, held.
    w.now.t += 2 * 60 * 60_000;
    w.ghRefuses.length = 0;
    expect(await (await w.act("issue.comment", { issue: 7, text: "behind it" }, ID(23))).json()).toMatchObject({
      held: true,
    });
    const before = commented(w).length;
    const kept = (heldStored(w) as { id: string; at: string }[]).map((h) => Date.parse(h.at));
    // Woken (to notice the laptop gone quiet), none of the three is done here: two are the
    // laptop's, which may have done them, and the third waits behind one of those.
    await wake(w);
    expect([commented(w).length, heldIds(w)]).toEqual([before, [ID(21), ID(22), ID(23)]]);
    expect(takenStored(w)).toEqual([]);
    // Nor does it wake itself over them again and again: nothing here is its own to do, and
    // its next wake is the day's end of the oldest.
    expect(w.alarm.at).toBe(kept[0] + 24 * 60 * 60_000 + 1_000);
    // The laptop never comes back. At the day's end the first it was handed is dropped, and
    // said to have been; what waited behind it is then this side's, and is done here, once.
    await wake(w);
    expect(heldIds(w)).toEqual([ID(22)]);
    expect(commented(w).slice(before)).toEqual(["behind it"]);
    const { doc } = await w.core();
    // (Not "not sent": the laptop was handed it, and may have done it.)
    expect(doc.acts.find((a: { id: string }) => a.id === ID(21))).toMatchObject({
      status: "failed",
      note: expect.stringMatching(/may have got this, and did not say what became of it within a day/),
    });
    expect(doc.acts.find((a: { id: string }) => a.id === ID(23))).toMatchObject({ status: "done" });
  });

  test("a machine reporting while this side is doing one: it is not handed that one, nor one behind it about the same thing, and each is done once", async () => {
    const w = await primary();
    await heldComments(w, [
      [ID(31), 7, "A"],
      [ID(32), 7, "B"],
      [ID(33), 8, "C"],
    ]);
    w.ghRefuses.length = 0;
    w.now.t += 60 * 60_000;
    // This side wakes and begins A; GitHub is slow to answer.
    let answerNow: () => void = () => {};
    w.ghHolds.push({
      match: COMMENT,
      wait: new Promise<void>((resolve) => {
        answerNow = resolve;
      }),
    });
    const waking = w.edge.now.alarm();
    for (let turn = 0; turn < 200 && commented(w).length < 2; turn++) await new Promise((r) => setTimeout(r, 1));
    expect(commented(w)).toEqual(["A", "A"]);
    expect([heldIds(w), (takenStored(w) as { id: string }[]).map((t) => t.id)]).toEqual([[ID(32), ID(33)], [ID(31)]]);
    // The laptop is back, and reports while A is still being done.
    w.net.hostUp = true;
    w.net.edgeUp = true;
    await w.hostReads();
    const answer = await w.link.now.once();
    expect(answer?.commands.map((c) => c.id)).toEqual([ID(33)]);
    expect(handedStored(w)).toEqual([ID(33)]);
    expect(w.ran.filter((r) => r.kind === "issue.comment").map((r) => r.args.text)).toEqual(["C"]);
    // GitHub answers. A is done here, then B behind it; C was the laptop's and is not done here.
    w.ghHolds.length = 0;
    answerNow();
    await waking;
    await settle(w);
    expect(commented(w)).toEqual(["A", "A", "B"]);
    expect(w.ran.filter((r) => r.kind === "issue.comment")).toHaveLength(1);
    expect(heldIds(w)).toEqual([ID(33)]);
    // The laptop says C is done, and nothing waits.
    await w.hostReads();
    await w.link.now.once();
    expect([heldIds(w), handedStored(w)]).toEqual([[], []]);
    expect(w.ran.filter((r) => r.kind === "issue.comment")).toHaveLength(1);
    for (const n of [31, 32]) expect(w.stored.get(`a:${ID(n)}`)).toMatchObject({ state: "done", status: 200 });
  });

  test("a newer one about a thing arriving while an older held one is being done here waits for it, and is done after", async () => {
    const w = await primary();
    await heldComments(w, [[ID(41), 7, "older"]]);
    w.ghRefuses.length = 0;
    w.now.t += 60 * 60_000;
    let answerNow: () => void = () => {};
    w.ghHolds.push({
      match: (method, url) => COMMENT(method, url),
      wait: new Promise<void>((resolve) => {
        answerNow = resolve;
      }),
    });
    const waking = w.edge.now.alarm();
    for (let turn = 0; turn < 200 && commented(w).length < 2; turn++) await new Promise((r) => setTimeout(r, 1));
    // The owner comments again on the same issue. It is not asked of GitHub until the older has ended.
    w.net.edgeUp = true;
    const newer = w.act("issue.comment", { issue: 7, text: "newer" }, ID(42));
    for (let turn = 0; turn < 20; turn++) await new Promise((r) => setTimeout(r, 1));
    expect(commented(w)).toEqual(["older", "older"]);
    w.ghHolds.length = 0;
    answerNow();
    await waking;
    expect(await (await newer).json()).toMatchObject({ ok: true });
    await settle(w);
    expect(commented(w)).toEqual(["older", "older", "newer"]);
    expect(heldIds(w)).toEqual([]);
  });

  test("turned away again when it tries: what it took goes back as it was, where it was, and a machine can still be handed it", async () => {
    const w = await primary();
    await heldComments(w, [
      [ID(51), 7, "still refused"],
      [ID(52), 8, "behind"],
    ]);
    // The hour passes and the token is as it was.
    await wake(w);
    await wake(w);
    const tried = w.now.t;
    expect(commented(w)).toEqual(["still refused", "still refused"]);
    expect(heldIds(w)).toEqual([ID(51), ID(52)]);
    expect([takenStored(w), handedStored(w)]).toEqual([[], []]);
    expect(w.stored.get(`a:${ID(51)}`)).toMatchObject({ state: "given" });
    // Not tried again before the kind's next hour is up.
    expect(w.alarm.at).toBe(tried + 60 * 60_000);
    // Sent again under its name meanwhile, it is not done here: it is a machine's by name, and waits once.
    w.ghRefuses.length = 0;
    expect(await (await w.act("issue.comment", { issue: 7, text: "still refused" }, ID(51))).json()).toMatchObject({
      held: true,
      again: true,
    });
    expect(commented(w)).toHaveLength(2);
    // The laptop comes back, is handed both in order, and does each once.
    w.net.hostUp = true;
    w.net.edgeUp = true;
    await w.hostReads();
    expect((await w.link.now.once())?.commands.map((c) => c.id)).toEqual([ID(51), ID(52)]);
    expect(w.ran.filter((r) => r.kind === "issue.comment").map((r) => r.args.text)).toEqual([
      "still refused",
      "behind",
    ]);
    await w.hostReads();
    await w.link.now.once();
    expect(heldIds(w)).toEqual([]);
    // Nothing more was asked of GitHub here.
    expect(commented(w)).toHaveLength(2);
  });

  test("put away while it had one taken: an object that starts again settles each by its record, and nothing is done twice", async () => {
    const w = await primary();
    await heldComments(w, [
      [ID(61), 7, "begun, and not known to have ended"],
      [ID(62), 8, "never begun"],
      [ID(63), 9, "back in the list already"],
      [ID(64), 10, "ended"],
      [ID(65), 11, "untouched"],
    ]);
    const held = heldStored(w) as { id: string; at: string }[];
    const by = (n: number) => held.find((h) => h.id === ID(n)) as { id: string; at: string };
    // As an object put away part-way would leave things: four taken, three of them out of the list.
    // (The taken index also names two that are not records of a taken thing: one with no
    // record kept under its name, and one whose record's name is not a name.)
    w.stored.set("taken:index", [ID(61), ID(62), ID(63), ID(64), "not one", ID(66)]);
    for (const n of [61, 62, 63, 64]) w.stored.set(`taken:${ID(n)}`, by(n));
    w.stored.set(`taken:${ID(66)}`, { id: 7 });
    w.stored.set("held:index", [ID(63), ID(65)]);
    w.stored.set(`held:${ID(63)}`, by(63));
    w.stored.set(`held:${ID(65)}`, by(65));
    w.stored.set(`a:${ID(61)}`, { at: w.now.t, kind: "issue.comment", state: "running" });
    w.stored.set(`a:${ID(64)}`, { at: w.now.t, kind: "issue.comment", state: "done", status: 200, body: { ok: true } });
    // (The one never begun was put away just after being turned away again: its name's record
    // had been cleared for the try, and not yet written back.)
    w.stored.delete(`a:${ID(62)}`);
    const asked = commented(w).length;
    w.edge.now = w.makeEdge();
    const { doc } = await w.core();
    // Never begun: it waits again, in its place (before the two that were kept after it).
    expect(heldIds(w)).toEqual([ID(62), ID(63), ID(65)]);
    expect(w.stored.get(`a:${ID(62)}`)).toMatchObject({ state: "given" });
    expect(takenStored(w)).toEqual([]);
    // Begun and not known to have ended: neither side does it again, and the owner is told to look.
    expect(w.stored.get(`a:${ID(61)}`)).toMatchObject({ state: "done", status: 502 });
    expect(doc.acts.find((a: { id: string }) => a.id === ID(61))).toMatchObject({
      status: "failed",
      note: expect.stringMatching(/restarted while doing this/),
    });
    // The one that had ended is as its record says.
    expect(w.stored.get(`a:${ID(64)}`)).toMatchObject({ state: "done", status: 200 });
    expect(doc.edge).toEqual({ held: 3, primary: true });
    // The laptop comes back: it is handed the three that wait, and no other.
    w.net.hostUp = true;
    w.net.edgeUp = true;
    await w.hostReads();
    expect((await w.link.now.once())?.commands.map((c) => c.id)).toEqual([ID(62), ID(63), ID(65)]);
    expect(commented(w)).toHaveLength(asked);
    // Something in storage that is not a list of them is nothing taken.
    for (const odd of [7, "x", { id: ID(61) }, null]) {
      w.stored.set("taken", odd);
      w.edge.now = w.makeEdge();
      expect([odd, (await w.core()).res.status]).toEqual([odd, 200]);
    }
  });

  // Since "done" became this side's (#13491) no kind of today's that
  // only a machine can do is about the same thing as one done here. The
  // rule has to hold for the next that is, so this object is told not
  // to do "done": it stands for a kind only a machine can do.
  test("one that only a machine can do is a machine's: this side does not try it, and what comes after it about the same thing waits behind it", async () => {
    const w = await primary(DIRECT_ACTIONS.filter((kind) => kind !== "hands.done"));
    w.net.hostUp = false;
    w.net.edgeUp = false;
    const ref = "developing-today/code#1";
    // Marking a need done is a machine's to do here; a comment on it is something this side does.
    expect(await (await w.act("hands.done", { ref }, ID(111))).json()).toMatchObject({ held: true });
    w.now.t += 1_000;
    const asked = w.githubCalls.length;
    expect(await (await w.act("need.comment", { ref, text: "after it" }, ID(112))).json()).toMatchObject({
      held: true,
    });
    // Nothing was tried here for either, and neither was failed.
    expect(heldIds(w)).toEqual([ID(111), ID(112)]);
    // Each is said to wait for a machine: the one only it can do, and the one behind it.
    expect(await heldWaits(w)).toEqual([
      [ID(111), "machine"],
      [ID(112), "machine"],
    ]);
    expect(w.stored.get(`a:${ID(111)}`)).toMatchObject({ state: "given" });
    // Woken, this side still does neither: the comment is behind what is a machine's.
    await wake(w);
    expect([heldIds(w), w.githubCalls.length]).toEqual([[ID(111), ID(112)], asked]);
    const { doc } = await w.core();
    expect(doc.acts.filter((a: { status: string }) => a.status === "failed")).toEqual([]);
    // The laptop comes back and does both, in order.
    w.net.hostUp = true;
    w.net.edgeUp = true;
    await w.hostReads();
    expect((await w.link.now.once())?.commands.map((c) => c.id)).toEqual([ID(111), ID(112)]);
  });

  test("a pass that could do nothing is not tried again at once, until the owner says to", async () => {
    const w = await primary();
    await heldComments(w, [[ID(121), 7, "when storage is well"]]);
    w.ghRefuses.length = 0;
    w.now.t += 60 * 60_000;
    // What it takes cannot be written down: it is not taken, and still waits.
    w.storeFails.add(`taken:${ID(121)}`);
    await w.edge.now.alarm();
    expect([commented(w).length, heldIds(w)]).toEqual([1, [ID(121)]]);
    expect(((await w.core()).doc.edge as { held: number }).held).toBe(1);
    // Its next wake for it is a minute on, not the next second.
    expect(w.alarm.at).toBe(w.now.t + 60_000);
    // Storage is well again, and the owner says to try again now: it wakes at once, and does it.
    w.storeFails.clear();
    w.net.edgeUp = true;
    expect((await w.act("edge.retry", {})).status).toBe(200);
    expect(w.alarm.at).toBe(w.now.t + 1_000);
    w.now.t = w.alarm.at;
    await w.edge.now.alarm();
    expect([commented(w).length, heldIds(w)]).toEqual([2, []]);
  });

  test("a take that could not be written down does not leave the thing marked as being done: a newer one about it is not left waiting", async () => {
    const w = await primary();
    await heldComments(w, [[ID(125), 7, "older"]]);
    w.ghRefuses.length = 0;
    w.now.t += 61 * 60_000;
    w.storeFails.add(`taken:${ID(125)}`);
    await w.edge.now.alarm();
    expect([commented(w).length, heldIds(w)]).toEqual([1, [ID(125)]]);
    // Storage is well again, and the owner comments on the same issue: the older is made
    // first, then the newer. (Were the issue still marked as being worked on, this would wait
    // for an end that never comes.)
    w.storeFails.clear();
    const newer = await w.act("issue.comment", { issue: 7, text: "newer" }, ID(126));
    expect(await newer.json()).toMatchObject({ ok: true, result: expect.anything() });
    expect([commented(w), heldIds(w)]).toEqual([["older", "older", "newer"], []]);
  });

  test("a side that no longer does the core does none of what is held: it stays a machine's, and is not failed", async () => {
    const w = await primary();
    await heldComments(w, [[ID(131), 7, "for whoever does the core"]]);
    w.ghRefuses.length = 0;
    // The switch is turned off (which restarts the Worker), and the hour passes.
    w.edge.now = w.makeEdge(false);
    w.now.t += 61 * 60_000;
    await w.edge.now.alarm();
    await settle(w);
    expect([commented(w).length, heldIds(w)]).toEqual([1, [ID(131)]]);
    expect(w.stored.get(`a:${ID(131)}`)).toMatchObject({ state: "given" });
    // The laptop comes back and does it.
    w.net.hostUp = true;
    w.net.edgeUp = true;
    await w.hostReads();
    expect((await w.link.now.once())?.commands.map((c) => c.id)).toEqual([ID(131)]);
    expect(w.ran.filter((r) => r.kind === "issue.comment")).toHaveLength(1);
  });

  test("an older held one this side is turned away for again keeps a newer one about the same thing behind it, though the newer is a kind it can do", async () => {
    const w = await primary();
    const labels = (method: string, url: string) => method === "POST" && /\/issues\/\d+\/labels$/.test(url);
    w.ghRefuses.push(labels);
    w.net.hostUp = false;
    w.net.edgeUp = false;
    expect(await (await w.act("issue.rank", { issue: 201, rank: "now" }, ID(141))).json()).toMatchObject({
      held: true,
    });
    // The hour for ranking is up, GitHub still refuses it, and the owner comments on the same
    // issue. The rank is tried first, refused again, and goes back; the comment, which this
    // side could make, waits behind it and is not made out of turn.
    w.now.t += 61 * 60_000;
    const written = w.puts.length;
    expect(await (await w.act("issue.comment", { issue: 201, text: "after the rank" }, ID(142))).json()).toMatchObject({
      held: true,
    });
    expect([commented(w), heldIds(w)]).toEqual([[], [ID(141), ID(142)]]);
    expect(w.stored.get(`a:${ID(141)}`)).toMatchObject({ state: "given" });
    // Tried once, not over and over: taken, and then no longer taken.
    expect(w.puts.slice(written).filter((key) => key === "taken:index")).toHaveLength(2);
    // The laptop comes back and does both, in order.
    w.net.hostUp = true;
    w.net.edgeUp = true;
    await w.hostReads();
    expect((await w.link.now.once())?.commands.map((c) => c.id)).toEqual([ID(141), ID(142)]);
  });

  test("of two ranks held for one issue the last wins, and a comment about an issue does not replace its rank", async () => {
    const w = await primary();
    const labels = (method: string, url: string) => method === "POST" && /\/issues\/\d+\/labels$/.test(url);
    w.ghRefuses.push(labels);
    w.net.hostUp = false;
    w.net.edgeUp = false;
    for (const [id, issue, rank] of [
      [ID(71), 201, "next"],
      [ID(72), 201, "now"],
      [ID(73), 202, "priority/high"],
    ] as const) {
      expect(await (await w.act("issue.rank", { issue, rank }, id)).json()).toMatchObject({ held: true });
      w.now.t += 1_000;
    }
    w.ghRefuses.length = 0;
    // The hour for ranking is up, and before this side has woken the owner comments on #202:
    // its rank, held and older, is done first. A comment is not a newer rank.
    w.now.t += 60 * 60_000;
    w.net.edgeUp = true;
    expect(await (await w.act("issue.comment", { issue: 202, text: "and a word" }, ID(74))).json()).toMatchObject({
      ok: true,
    });
    expect(w.ghLabels[202]).toContain("priority/high");
    expect(w.stored.get(`a:${ID(73)}`)).toMatchObject({ kind: "issue.rank", state: "done", status: 200 });
    expect(commented(w)).toEqual(["and a word"]);
    // Then it wakes: of the two ranks for #201 the older is dropped as replaced, and the newer done.
    await wake(w);
    expect(w.stored.get(`a:${ID(71)}`)).toMatchObject({ kind: "issue.rank", state: "done", status: 409 });
    expect(w.stored.get(`a:${ID(72)}`)).toMatchObject({ kind: "issue.rank", state: "done", status: 200 });
    expect(w.ghLabels[201]).toContain("now");
    expect(w.ghLabels[201]).not.toContain("next");
    expect(heldIds(w)).toEqual([]);
    const { doc } = await w.core();
    expect(doc.acts.find((a: { id: string }) => a.id === ID(71))).toMatchObject({
      status: "failed",
      note: expect.stringMatching(/Replaced by a newer issue\.rank/),
    });
  });

  test("one the laptop may have done, its answer lost on the way back, is the laptop's by name: this side never does it, however long the laptop is away", async () => {
    const w = await primary();
    w.ghRefuses.push(COMMENT);
    // Turned away here, it goes to the laptop, which makes the comment. Its answer is lost on the way back.
    w.net.loseAnswer = true;
    expect(
      await (await w.act("issue.comment", { issue: 7, text: "made by the laptop" }, ID(201))).json(),
    ).toMatchObject({ held: true });
    expect(w.ran.filter((r) => r.kind === "issue.comment")).toHaveLength(1);
    // The laptop goes away. The token is put right, the hour passes, and this side wakes.
    w.net.loseAnswer = false;
    w.net.hostUp = false;
    w.net.edgeUp = false;
    w.ghRefuses.length = 0;
    const asked = commented(w).length;
    w.now.t += 61 * 60_000;
    await wakeNow(w);
    // It does not make the comment a second time, and still holds it for the laptop.
    expect([commented(w).length, heldIds(w)]).toEqual([asked, [ID(201)]]);
    expect(await heldWaits(w)).toEqual([[ID(201), "asked"]]);
    // Nor when the owner says to try again now.
    expect((await w.act("edge.retry", {})).status).toBe(200);
    w.now.t += 2_000;
    await wakeNow(w);
    expect([commented(w).length, heldIds(w)]).toEqual([asked, [ID(201)]]);
    // The laptop comes back, is handed it by name, and says what became of it: made once.
    w.net.hostUp = true;
    w.net.edgeUp = true;
    await w.hostReads();
    expect((await w.link.now.once())?.commands.map((c) => c.id)).toEqual([ID(201)]);
    await w.link.now.once();
    expect([heldIds(w), w.ran.filter((r) => r.kind === "issue.comment").length, commented(w).length]).toEqual([
      [],
      1,
      asked,
    ]);
  });

  /** Woken now, as an object that has just started. (Not at the time it asked for: with nothing
   * of its own to do, that is the day's end of what it holds.) */
  const wakeNow = async (w: HeldWorld) => {
    w.edge.now = w.makeEdge();
    await w.edge.now.alarm();
    await settle(w);
  };

  test("out of reach with nothing to say so, the laptop may have got it, and it is never done here; said to be out of reach, it is", async () => {
    // Two comments, each turned away here with the laptop out of reach: one way, then the other.
    const w = await primary();
    w.ghRefuses.push(COMMENT);
    w.net.hostUp = false;
    w.net.edgeUp = false;
    w.net.silent = true;
    expect(await (await w.act("issue.comment", { issue: 7, text: "may have arrived" }, ID(211))).json()).toEqual({
      ok: true,
      held: true,
      result: null,
      waits: "asked",
    });
    w.net.silent = false;
    expect(await (await w.act("issue.comment", { issue: 8, text: "never arrived" }, ID(212))).json()).toEqual({
      ok: true,
      held: true,
      result: null,
      waits: "either",
    });
    expect(await heldWaits(w)).toEqual([
      [ID(211), "asked"],
      [ID(212), "either"],
    ]);
    // The health answer, which anyone may read, counts them and says nothing else of them.
    const health = (await (await w.browse("/console/health", { token: null })).json()) as { held: unknown };
    expect(health.held).toEqual({ either: 1, machine: 0, asked: 1 });
    const { doc } = await w.core();
    const noted = (id: string) => doc.acts.find((a: { id: string }) => a.id === id)?.note;
    expect(noted(ID(211))).toMatch(/did not answer\. It may have got this/);
    expect(noted(ID(212))).toMatch(/Cloudflare does it as soon as it can again/);
    // So far GitHub was asked for the first, and refused it; with this side turned away by
    // then, the second was not asked for at all.
    expect(commented(w)).toEqual(["may have arrived"]);
    // GitHub takes comments again, and the hour passes.
    w.ghRefuses.length = 0;
    w.now.t += 61 * 60_000;
    await wakeNow(w);
    // Only the one that reached nobody is made here. The other is still the laptop's.
    expect([commented(w), heldIds(w)]).toEqual([["may have arrived", "never arrived"], [ID(211)]]);
  });

  test("offered again while it waits here, having reached no machine, it is not sent toward the laptop: it waits as it was, and is done once", async () => {
    const w = await primary();
    await heldComments(w, [[ID(221), 7, "offered twice"]]);
    // The device never heard that it was kept, and offers it again. By now the laptop can be
    // reached (it has not reported yet): sent on, the comment would be made there.
    w.net.hostUp = true;
    const again = await w.act("issue.comment", { issue: 7, text: "offered twice" }, ID(221));
    expect(await again.json()).toEqual({ ok: true, held: true, result: null, again: true, waits: "either" });
    expect(w.ran.filter((r) => r.kind === "issue.comment")).toEqual([]);
    expect(heldIds(w)).toEqual([ID(221)]);
    // This side makes it when it can: once.
    w.ghRefuses.length = 0;
    w.now.t += 61 * 60_000;
    await wakeNow(w);
    expect([commented(w), heldIds(w)]).toEqual([["offered twice", "offered twice"], []]);
    expect(w.ran.filter((r) => r.kind === "issue.comment")).toEqual([]);
    // Offered a third time, it is answered from what was written down.
    expect(await (await w.act("issue.comment", { issue: 7, text: "offered twice" }, ID(221))).json()).toMatchObject({
      ok: true,
      again: true,
    });
    expect(commented(w)).toHaveLength(2);
  });

  test("offered again while its first asking of the laptop is still out: word that the second never arrived is not word that the first did not", async () => {
    const w = await primary();
    w.ghRefuses.push(COMMENT);
    // Turned away here, it goes to the laptop, which is slow about it.
    let open = () => {};
    w.gate.wait = new Promise<void>((resolve) => {
      open = resolve;
    });
    const acts = () => w.hostSaw.filter((seen) => seen === "POST /console/api/act").length;
    const before = acts();
    const first = w.act("issue.comment", { issue: 7, text: "asked twice" }, ID(231));
    for (let turn = 0; turn < 200 && acts() === before; turn++) await new Promise((r) => setTimeout(r, 1));
    expect(acts()).toBe(before + 1);
    // The device gives up on that try and offers it again. By now the tunnel is down.
    w.net.hostUp = false;
    const second = await w.act("issue.comment", { issue: 7, text: "asked twice" }, ID(231));
    expect(await second.json()).toEqual({ ok: true, held: true, result: null, waits: "asked" });
    // GitHub takes comments again and the hour passes: this side still does not make it.
    w.ghRefuses.length = 0;
    const asked = commented(w).length;
    w.now.t += 61 * 60_000;
    await wakeNow(w);
    expect([commented(w).length, heldIds(w)]).toEqual([asked, [ID(231)]]);
    // The laptop finishes the first asking. It made the comment, once, and its answer settles the name.
    w.gate.wait = null;
    open();
    expect(await (await first).json()).toMatchObject({ ok: true });
    expect([heldIds(w), w.ran.filter((r) => r.kind === "issue.comment").length, commented(w).length]).toEqual([
      [],
      1,
      asked,
    ]);
  });

  test("a newer one about the same thing does not go ahead of one the laptop may have: it waits behind it, and one about something else does not", async () => {
    const w = await primary();
    w.ghRefuses.push(COMMENT);
    w.net.loseAnswer = true;
    expect(await (await w.act("issue.comment", { issue: 7, text: "the laptop's" }, ID(241))).json()).toMatchObject({
      held: true,
      waits: "asked",
    });
    w.net.loseAnswer = false;
    w.net.hostUp = false;
    w.net.edgeUp = false;
    // This side takes comments again.
    w.ghRefuses.length = 0;
    w.now.t += 61 * 60_000;
    w.edge.now = w.makeEdge();
    const asked = commented(w).length;
    const newer = await w.act("issue.comment", { issue: 7, text: "newer" }, ID(242));
    expect(await newer.json()).toEqual({ ok: true, held: true, result: null, waits: "machine" });
    expect(commented(w)).toHaveLength(asked);
    expect(await heldWaits(w)).toEqual([
      [ID(241), "asked"],
      [ID(242), "machine"],
    ]);
    // A comment on another issue is nothing to do with either: it is made at once.
    expect(await (await w.act("issue.comment", { issue: 8, text: "other" }, ID(243))).json()).toMatchObject({
      ok: true,
      result: expect.anything(),
    });
    expect(commented(w).slice(asked)).toEqual(["other"]);
    // Woken, this side still does neither of the two.
    await wakeNow(w);
    expect([commented(w).length, heldIds(w)]).toEqual([asked + 1, [ID(241), ID(242)]]);
  });

  test("offered again at the moment this side has taken it to do itself, it is not sent to the laptop as well: the answer is what came of doing it here", async () => {
    const w = await primary();
    await heldComments(w, [[ID(251), 7, "taken, then offered"]]);
    w.ghRefuses.length = 0;
    w.now.t += 61 * 60_000;
    // The laptop can be reached again (it has not reported): a name sent toward it is done there.
    w.net.hostUp = true;
    w.edge.now = w.makeEdge();
    // Its record is slow to read. The wake has taken it, and has not yet written down that it
    // has begun, when the device offers it again: by its record it is still a machine's.
    let read = () => {};
    w.storeSlow.set(
      `a:${ID(251)}`,
      new Promise<void>((resolve) => {
        read = resolve;
      }),
    );
    const waking = w.edge.now.alarm();
    const taken = () => takenStored(w).length;
    for (let turn = 0; turn < 200 && taken() === 0; turn++) await new Promise((r) => setTimeout(r, 1));
    expect(taken()).toBe(1);
    const again = w.act("issue.comment", { issue: 7, text: "taken, then offered" }, ID(251));
    for (let turn = 0; turn < 20; turn++) await new Promise((r) => setTimeout(r, 1));
    w.storeSlow.clear();
    read();
    await waking;
    expect(await (await again).json()).toMatchObject({ ok: true, again: true });
    await settle(w);
    // Made here, once, and not by the laptop.
    expect(commented(w)).toEqual(["taken, then offered", "taken, then offered"]);
    expect(w.ran.filter((r) => r.kind === "issue.comment")).toEqual([]);
    expect([heldIds(w), takenStored(w)]).toEqual([[], []]);
  });

  test("one kept before it was written down whether it reached a machine is taken to have: this side leaves it to the laptop", async () => {
    const w = await primary();
    await heldComments(w, [[ID(261), 7, "from before"]]);
    // As the version before this one stored it: with no word either way.
    const stored = heldStored(w) as Record<string, unknown>[];
    expect(stored.map((h) => h.unsent)).toEqual([true]);
    w.stored.set(`held:${ID(261)}`, stored.map(({ unsent: _unsent, ...rest }) => rest)[0]);
    w.ghRefuses.length = 0;
    w.now.t += 61 * 60_000;
    await wakeNow(w);
    expect([commented(w).length, heldIds(w)]).toEqual([1, [ID(261)]]);
    expect(await heldWaits(w)).toEqual([[ID(261), "asked"]]);
    // The laptop, back, is handed it by name.
    w.net.hostUp = true;
    w.net.edgeUp = true;
    await w.hostReads();
    expect((await w.link.now.once())?.commands.map((c) => c.id)).toEqual([ID(261)]);
  });

  test("the laptop answers for a name and its answer cannot be read: the name is the laptop's, and nothing is left waiting under it", async () => {
    const w = await primary();
    w.ghRefuses.push(COMMENT);
    w.net.loseAnswer = true;
    expect(await (await w.act("issue.comment", { issue: 7, text: "answered, unread" }, ID(271))).json()).toMatchObject({
      held: true,
      waits: "asked",
    });
    w.net.loseAnswer = false;
    // The device offers it again; the laptop says how it went, and what it says does not arrive whole.
    w.net.garble = true;
    const again = await w.act("issue.comment", { issue: 7, text: "answered, unread" }, ID(271));
    w.net.garble = false;
    expect([again.status, await again.json()]).toEqual([503, { error: "the host's answer could not be read" }]);
    expect(heldIds(w)).toEqual([]);
    // Offered once more, the laptop's own answer comes through: made once.
    expect(await (await w.act("issue.comment", { issue: 7, text: "answered, unread" }, ID(271))).json()).toMatchObject({
      ok: true,
    });
    expect(w.ran.filter((r) => r.kind === "issue.comment")).toHaveLength(1);
  });

  test("turned away again at the moment it is offered again: the device is told it is not over, and it is not sent to the laptop", async () => {
    const w = await primary();
    await heldComments(w, [[ID(255), 7, "taken, refused, offered"]]);
    // The hour passes with GitHub still refusing comments, and the laptop can be reached again.
    w.now.t += 61 * 60_000;
    w.net.hostUp = true;
    w.edge.now = w.makeEdge();
    let read = () => {};
    w.storeSlow.set(
      `a:${ID(255)}`,
      new Promise<void>((resolve) => {
        read = resolve;
      }),
    );
    const waking = w.edge.now.alarm();
    const taken = () => takenStored(w).length;
    for (let turn = 0; turn < 200 && taken() === 0; turn++) await new Promise((r) => setTimeout(r, 1));
    expect(taken()).toBe(1);
    const again = w.act("issue.comment", { issue: 7, text: "taken, refused, offered" }, ID(255));
    for (let turn = 0; turn < 20; turn++) await new Promise((r) => setTimeout(r, 1));
    w.storeSlow.clear();
    read();
    await waking;
    const answer = await again;
    expect([answer.status, await answer.json()]).toEqual([409, { error: "This is still being done.", running: true }]);
    await settle(w);
    // Refused twice by GitHub, made by nobody, and waiting again as it was.
    expect(commented(w)).toEqual(["taken, refused, offered", "taken, refused, offered"]);
    expect(w.ran.filter((r) => r.kind === "issue.comment")).toEqual([]);
    expect([heldIds(w), takenStored(w)]).toEqual([[ID(255)], []]);
    expect(await heldWaits(w)).toEqual([[ID(255), "either"]]);
  });

  test("a rank the laptop may have set is not replaced by a newer rank: the newer waits behind it, and neither is done here", async () => {
    const w = await primary();
    const labels = (method: string, url: string) => method === "POST" && /\/issues\/\d+\/labels$/.test(url);
    const labelled = () => w.githubCalls.filter((c) => labels(c.method, c.url)).length;
    w.ghRefuses.push(labels);
    // Turned away here, the rank goes to the laptop. Its answer is lost on the way back.
    w.net.loseAnswer = true;
    expect(await (await w.act("issue.rank", { issue: 201, rank: "now" }, ID(281))).json()).toMatchObject({
      held: true,
      waits: "asked",
    });
    w.net.loseAnswer = false;
    w.net.hostUp = false;
    w.net.edgeUp = false;
    // This side may label again, and the owner ranks the issue again.
    w.ghRefuses.length = 0;
    w.now.t += 61 * 60_000;
    w.edge.now = w.makeEdge();
    const asked = labelled();
    expect(await (await w.act("issue.rank", { issue: 201, rank: "next" }, ID(282))).json()).toEqual({
      ok: true,
      held: true,
      result: null,
      waits: "machine",
    });
    // Both wait, in the order they were done. GitHub was not asked, and the older was not
    // dropped as replaced: done late by the laptop, it would otherwise be the last word.
    expect([heldIds(w), labelled()]).toEqual([[ID(281), ID(282)], asked]);
    const { doc } = await w.core();
    expect(doc.acts.find((a: { id: string }) => a.id === ID(281))).toMatchObject({ status: "running" });
  });

  test("a name given to a machine before its askings were counted is taken to have set off already", async () => {
    const w = await primary();
    w.ghRefuses.push(COMMENT);
    w.net.hostUp = false;
    w.net.edgeUp = false;
    // As the version before this one wrote it down: a machine's, with no count of its askings.
    w.stored.set(`a:${ID(291)}`, { at: w.now.t, kind: "issue.comment", state: "given" });
    // Word that this asking never arrived says nothing of the one before it.
    expect(await (await w.act("issue.comment", { issue: 7, text: "from before the count" }, ID(291))).json()).toEqual({
      ok: true,
      held: true,
      result: null,
      waits: "asked",
    });
    expect(w.stored.get(`a:${ID(291)}`)).toMatchObject({ state: "given", relays: 2 });
    // A name new to this side, out of reach the same way, set off once: it reached nobody.
    expect(await (await w.act("issue.comment", { issue: 8, text: "new" }, ID(292))).json()).toEqual({
      ok: true,
      held: true,
      result: null,
      waits: "either",
    });
    expect(w.stored.get(`a:${ID(292)}`)).toMatchObject({ state: "given", relays: 1 });
  });

  test("offered again while its first asking is still out, with something else about the same thing waiting here: it is kept as the laptop's, and nothing about the thing is done here meanwhile", async () => {
    const w = await primary();
    w.ghRefuses.push(COMMENT);
    // Turned away here, the first comment goes to the laptop, which is slow about it.
    let open = () => {};
    w.gate.wait = new Promise<void>((resolve) => {
      open = resolve;
    });
    const acts = () => w.hostSaw.filter((seen) => seen === "POST /console/api/act").length;
    const before = acts();
    const first = w.act("issue.comment", { issue: 7, text: "first, slow" }, ID(301));
    for (let turn = 0; turn < 200 && acts() === before; turn++) await new Promise((r) => setTimeout(r, 1));
    expect(acts()).toBe(before + 1);
    // The tunnel drops. A second comment on the issue reaches nobody, and waits here.
    w.net.hostUp = false;
    expect(await (await w.act("issue.comment", { issue: 7, text: "second" }, ID(302))).json()).toMatchObject({
      held: true,
      waits: "either",
    });
    // The device gives up on the first and offers it again. It is kept, as the laptop's:
    // its first asking may be being done.
    expect(await (await w.act("issue.comment", { issue: 7, text: "first, slow" }, ID(301))).json()).toEqual({
      ok: true,
      held: true,
      result: null,
      waits: "asked",
    });
    expect(await heldWaits(w)).toEqual([
      [ID(302), "machine"],
      [ID(301), "asked"],
    ]);
    // GitHub takes comments again and the hour passes: this side makes neither.
    w.ghRefuses.length = 0;
    const asked = commented(w).length;
    w.now.t += 61 * 60_000;
    await wakeNow(w);
    expect([commented(w).length, heldIds(w)]).toEqual([asked, [ID(302), ID(301)]]);
    // The laptop finishes the first asking: made there, once, and the name is settled.
    w.gate.wait = null;
    open();
    expect(await (await first).json()).toMatchObject({ ok: true });
    expect([heldIds(w), w.ran.filter((r) => r.kind === "issue.comment").length]).toEqual([[ID(302)], 1]);
    // What is left reached nobody, and is this side's again: made here, once.
    expect(await heldWaits(w)).toEqual([[ID(302), "either"]]);
    w.now.t += 2_000;
    await wakeNow(w);
    expect([commented(w).slice(asked), heldIds(w)]).toEqual([["second"], []]);
    expect(w.ran.filter((r) => r.kind === "issue.comment")).toHaveLength(1);
  });

  test("with no machine ever reporting, what only a machine can do is still dropped after a day, and said to have been", async () => {
    const w = await primary();
    w.net.hostUp = false;
    w.net.edgeUp = false;
    const t0 = w.now.t;
    expect(await (await w.act("message.send", { to: ["lane-1"], text: "nobody came" }, ID(81))).json()).toMatchObject({
      held: true,
    });
    // Woken meanwhile (for the laptop gone quiet), it is still held, and not this side's to try.
    await wake(w);
    expect([w.now.t < t0 + 60 * 60_000, heldIds(w)]).toEqual([true, [ID(81)]]);
    // Its next wake is the day's end of it.
    await wake(w);
    expect(w.now.t).toBe(t0 + 24 * 60 * 60_000 + 1_000);
    expect(heldIds(w)).toEqual([]);
    const { doc } = await w.core();
    expect(doc.acts.find((a: { id: string }) => a.id === ID(81))).toMatchObject({
      status: "failed",
      note: expect.stringMatching(/more than a day/),
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
    // Health says so to anyone: the clock is wanted, and when it next wakes. It is how the
    // path notifications depend on is seen to be alive from outside.
    const clock = async () =>
      ((await (await w.browse("/console/health", { token: null })).json()) as { clock: unknown }).clock;
    expect(await clock()).toEqual({ wanted: true, next: START + 1_000 });
    // The laptop goes away for good. Nobody opens the app.
    w.net.hostUp = false;
    w.net.edgeUp = false;
    w.now.t = w.alarm.at;
    await w.edge.now.alarm();
    expect(w.pushes).toHaveLength(0);
    expect(w.alarm.at).toBe(w.now.t + 60_000);
    // (Said by an object that has just started, too: the time is the platform's to hold.)
    w.edge.now = w.makeEdge();
    expect(await clock()).toEqual({ wanted: true, next: w.now.t + 60_000 });
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
    // The other list read for the owner (what needs him) was not due this time, so it is
    // still this side's to read, and the next wake is a minute on as ever.
    expect(w.alarm.at).toBe(limitedAt + 60_000);
    // GitHub gave no time, so it is asked once for that list too, is told the same, and
    // asks for none of the rest of it.
    const asked = w.githubCalls.length;
    w.now.t = w.alarm.at;
    await w.edge.now.alarm();
    expect(w.githubCalls.length).toBe(asked + 1);
    // After that, not a minute from now, when it would only be turned away again:
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

  test("something new that only the owner can do is told to a subscribed device with no machine reporting", async () => {
    const { w, phone } = await subscribed();
    w.net.hostUp = false;
    w.net.edgeUp = false;
    // A wake reads what needs him; there is nothing yet, and nothing is said.
    w.now.t = w.alarm.at + 121_000;
    await w.edge.now.alarm();
    expect(w.pushes).toHaveLength(0);
    // An agent marks an issue as needing his hands.
    w.ghNeeds["developing-today/code"].push({
      number: 21,
      title: "Hardware key",
      labels: ["user/escalated"],
      body: '<!-- hands:v1 kind=physical action="Plug the hardware key into the build host." -->',
    });
    w.now.t += 121_000;
    w.edge.now = w.makeEdge();
    await w.edge.now.alarm();
    expect(w.pushes).toHaveLength(1);
    // What to do, on what, and a button that says it is done: as it
    // reaches the phone, after the trip through the push service's own encryption.
    expect(JSON.parse(await phone.open(w.pushes[0].body))).toMatchObject({
      title: "Something needs you",
      body: "Plug the hardware key into the build host. (developing-today/code#21)",
      actions: [
        { action: "done", title: "I did it", kind: "hands.done", args: { ref: "developing-today/code#21" } },
        { action: "open", title: "Open", url: "/console/#needs/hands" },
      ],
    });
    // Told once: a later wake that finds the same thing says nothing more.
    w.now.t += 121_000;
    await w.edge.now.alarm();
    expect(w.pushes).toHaveLength(1);
    expect(w.hostSaw).toEqual([]);
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
