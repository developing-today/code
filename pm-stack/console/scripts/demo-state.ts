// A made-up host, to look at the interface without a fleet: the real
// server code answers, only the sources are canned, and every action
// succeeds. demo.ts serves it directly; edge-demo.ts puts the edge in
// front of it.
import { appendFileSync } from "node:fs";
import { actions as realActions } from "../src/actions.ts";
import { readConfig } from "../src/config.ts";
import { Devices, Limiter } from "../src/devices.ts";
import { Docs } from "../src/docs.ts";
import { fileStore } from "../src/file-store.ts";
import { Named } from "../src/named.ts";
import { type Fetch, Ops, type Run } from "../src/ops.ts";
import { Plan } from "../src/plan.ts";
import { Push } from "../src/push.ts";
import type { Deps } from "../src/server.ts";
import { memoryStore } from "../src/store.ts";
import { Work } from "../src/work.ts";

const iso = (minutesAgo: number) => new Date(Date.now() - minutesAgo * 60_000).toISOString().slice(0, 19);
const task = (id: number, title: string, extra: Record<string, unknown>) => ({
  id,
  title,
  priority: 30,
  status: "queued",
  attempts: 1,
  claimed: "",
  lease: "",
  created: iso(300),
  worker: null,
  lane: null,
  model: null,
  kind: "small_fix",
  issue: 9000 + id,
  pr: null,
  step: "",
  goal: "The fix lands with a test that fails without it",
  url: `https://github.com/developing-today/code/issues/${9000 + id}`,
  ...extra,
});
const question = (set: string, n: number, extra: Record<string, unknown> = {}) => ({
  set,
  setTitle: `Pending questions — ${set}`,
  created: iso(60 * 24 * n),
  ledger: `ledger-${set}-${n}`,
  item: `PM-stack S${n} attachments (#1003${n}): where do attachments live`,
  links: [`https://github.com/developing-today/code/issues/1003${n}`],
  qid: `q${n}`,
  q: `For S${n} attachments and multimedia, build new storage, or reuse a mechanism outside pm-stack that you name?`,
  context: "Attachments today are pasted links. The dashboard has no storage of its own, and three pages want one.",
  detail:
    "Building new keeps the stack self-contained. Reusing means one fewer store to run, but ties the release to another project.",
  multi: false,
  recommended: ["opt-1"],
  options: [
    { id: "opt-1", label: "Build new attachment storage", tags: ["recommended", "simpler"] },
    { id: "opt-2", label: "Reuse a mechanism outside pm-stack (name it in your answer)", tags: [] },
    { id: "opt-3", label: "Build new now; migrate storage refs later", tags: ["reversible"] },
  ],
  ...extra,
});

const state = {
  board: {
    counts: { claimed: 3, queued: 4, blocked: 474, parked: 58 },
    now: [
      task(1, "Fix #9075: session state — session id in env and a resume path", {
        status: "claimed",
        claimed: iso(14),
        worker: "repo-worker-code",
        lane: "oc-code-02",
        model: "workhorse",
        attempts: 8,
        step: "Write the failing test for resume after a crash, then wire the env var through the launcher.",
      }),
      task(2, "Implement mcpx Roc script support after the Python parity gate", {
        status: "claimed",
        claimed: iso(41),
        worker: "repo-worker-code",
        lane: "code-h",
        priority: 20,
        step: "Resume only when a compatible docs build is available.",
      }),
      task(3, "Fix #2723 phase 3: PendingHold post and void lifecycle", {
        status: "claimed",
        claimed: iso(3),
        worker: "repo-worker-code",
        lane: "code-j",
        goal: "Posting and voiding a hold are covered by the ledger property tests",
      }),
    ],
    next: [
      task(4, "Owner prompt developing-today/code#12501: make the join-at-admission slice the top priority", {
        priority: 10,
        kind: "triage",
      }),
      task(5, "Triage developing-today/code#12416: link from the dashboard to the console", {
        priority: 30,
        kind: "triage",
      }),
      task(6, "Merge-watch developing-today/code#12240: recheck fix PR for issue #12230", {
        priority: 30,
        kind: "pr_shepherd",
      }),
      task(7, "Fix #11350: toolchain file drift between crates", { priority: 50 }),
    ],
    blockedWhy: [
      { why: "waits on #9106 (open, all PRs merged)", n: 212 },
      { why: "no reason recorded", n: 140 },
      { why: '{"kind":"time","until":"2026-11-19T14:02:00Z"}', n: 4 },
    ],
  },
  sessions: {
    moving: [
      {
        id: "s1",
        title: "F12.11 binary frames: fuzz the decoder",
        agent: "build",
        model: "gpt-5.2-codex",
        where: "git/code-j",
        last: iso(1),
        tools: 2,
        messages: 141,
      },
      {
        id: "s2",
        title: "Triage sweep: opencode",
        agent: "triage",
        model: "workhorse",
        where: "git/opencode",
        last: iso(6),
        tools: 0,
        messages: 37,
      },
    ],
    waiting: [
      {
        id: "porkbun",
        title: "Porkbun: log into the main account and enter the password in the staged modal",
        ref: "gh:developing-today/code#902",
        why: "",
      },
    ],
  },
  census: {
    at: iso(1),
    complete: true,
    hands: [
      {
        ref: "developing-today/code#12417",
        title: "Google sign-in for pm.security.cab: create the OAuth client",
        url: "https://github.com/developing-today/code/issues/12417",
        pr: false,
        updated: iso(50),
        kind: "interactive-login",
        action:
          "Create a Google OAuth web client with the redirect URI in the issue and save its ID and secret in Bitwarden.",
      },
      {
        ref: "developing-today/code#1080",
        title: "epic(cluster): hardware inventory and first-node selection",
        url: "https://github.com/developing-today/code/issues/1080",
        pr: false,
        updated: iso(60 * 24 * 9),
        kind: "physical",
        action: "Power on one Threadripper host and make its BMC reachable to the worker.",
      },
    ],
    reviews: [
      {
        ref: "developing-today/code#24",
        title: "Dependency Dashboard",
        url: "https://github.com/developing-today/code/issues/24",
        pr: false,
        updated: iso(60 * 30),
      },
    ],
    other: [
      {
        ref: "developing-today/code#1086",
        title: "epic(cluster): TALM and Talos bootstrap",
        url: "https://github.com/developing-today/code/issues/1086",
        pr: false,
        updated: iso(60 * 24 * 20),
        why: "",
      },
    ],
  },
  asks: [
    {
      issue: 12501,
      url: "https://github.com/developing-today/code/issues/12501",
      text: "Make the join-at-admission slice the top priority and tell me what is in its way.",
      created: iso(30),
      updated: iso(4),
      status: "answered",
      worker: null,
      replies: [
        {
          you: false,
          at: iso(4),
          text: "Done. #12279 is ranked expedite and its task is first in the queue. In its way: #12230 (36 test files in no registry row) fails the lint the slice's PR will run. I queued that ahead of it.",
        },
      ],
    },
    {
      issue: 12499,
      url: "https://github.com/developing-today/code/issues/12499",
      text: "What is blocked and why? One paragraph.",
      created: iso(200),
      updated: iso(180),
      status: "done",
      worker: null,
      replies: [
        {
          you: false,
          at: iso(180),
          text: "474 tasks are blocked. 212 wait on #9106, which is open although every PR for it is merged; closing it frees them.",
        },
      ],
    },
    {
      issue: 12502,
      url: "https://github.com/developing-today/code/issues/12502",
      text: "Add a dark and light theme switch to the app.",
      created: iso(2),
      updated: iso(2),
      status: "queued",
      worker: null,
      replies: [],
    },
  ],
  questionCount: { total: 6, sets: 2 },
  questions: {
    total: 6,
    sets: 2,
    open: [
      question("2026-11-17", 7),
      question("2026-11-17", 8, {
        q: "Which checks must pass before a batch merges?",
        multi: true,
        recommended: ["opt-1", "opt-3"],
        options: [
          { id: "opt-1", label: "Unit tests of the crates the batch touches", tags: ["recommended"] },
          { id: "opt-2", label: "The whole workspace", tags: ["safer"] },
          { id: "opt-3", label: "The module reach check", tags: ["recommended"] },
        ],
      }),
      question("2026-11-17", 9, { recommended: [], context: "", detail: "" }),
      question("2026-11-16", 4),
      question("2026-11-16", 5),
      question("2026-11-16", 6),
    ],
  },
  prs: [
    {
      number: 12240,
      title: "ci(id): register the 36 unwired suites",
      draft: false,
      url: "https://github.com/developing-today/code/pull/12240",
      created: iso(60 * 20),
      author: "dezren39",
      branch: "task/suite-registry",
      head: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    },
    {
      number: 12241,
      title: "feat(id): F12.11 binary frames, encoder",
      draft: false,
      url: "https://github.com/developing-today/code/pull/12241",
      created: iso(60 * 19),
      author: "dezren39",
      branch: "task/f12-11-frames",
      head: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    },
    {
      number: 12390,
      title: "wip: talos bootstrap notes",
      draft: true,
      url: "https://github.com/developing-today/code/pull/12390",
      created: iso(60 * 3),
      author: "dezren39",
      branch: "wip/talos",
      head: "cccccccccccccccccccccccccccccccccccccccc",
    },
  ],
  done: {
    tasks: {
      day: 41,
      kinds: [
        { kind: "second_pass", n: 31 },
        { kind: "small_fix", n: 6 },
        { kind: "triage", n: 3 },
        { kind: "feature", n: 1 },
      ],
      latest: [
        {
          id: 21640,
          title: "Console: say why tasks are blocked, by the kind of reason",
          at: iso(50),
          lane: "code-a",
          model: "sonnet",
          kind: "small_fix",
          issue: 14212,
          pr: 14210,
          verdict: "merged_verified",
          url: "https://github.com/developing-today/code/issues/14212",
          prUrl: "https://github.com/developing-today/code/pull/14210",
        },
        {
          id: 21633,
          title: "Triage: duplicate of the queue census report",
          at: iso(60 * 5),
          lane: "code-c",
          model: null,
          kind: "triage",
          issue: 14101,
          pr: null,
          verdict: "closed_verified",
          url: "https://github.com/developing-today/code/issues/14101",
          prUrl: null,
        },
        {
          id: 21611,
          title: "id: relay fallback when both peers are behind symmetric NAT",
          at: iso(60 * 9),
          lane: "code-f",
          model: "opus",
          kind: "feature",
          issue: 13990,
          pr: 14150,
          verdict: "open_draft_with_followup",
          url: "https://github.com/developing-today/code/issues/13990",
          prUrl: "https://github.com/developing-today/code/pull/14150",
        },
        {
          id: 21570,
          title: "Fleet: seat guard restarts cleanly after a failed probe",
          at: iso(60 * 31),
          lane: "code-b",
          model: "sonnet",
          kind: "small_fix",
          issue: null,
          pr: 14102,
          verdict: "merged_verified",
          url: null,
          prUrl: "https://github.com/developing-today/code/pull/14102",
        },
      ],
    },
    merged: {
      day: 2,
      more: false,
      latest: [
        {
          number: 14217,
          title: "feat(pm-console): the app is told of changes as they happen",
          url: "https://github.com/developing-today/code/pull/14217",
          at: iso(35),
          by: "dezren39",
        },
        {
          number: 14210,
          title: "fix(pm-console): what the live pages got wrong",
          url: "https://github.com/developing-today/code/pull/14210",
          at: iso(50),
          by: "dezren39",
        },
        {
          number: 14102,
          title: "fix(fleet): seat guard restarts cleanly after a failed probe",
          url: "https://github.com/developing-today/code/pull/14102",
          at: iso(60 * 31),
          by: "dezren39",
        },
      ],
    },
    closed: {
      day: 1,
      more: false,
      latest: [
        {
          number: 14101,
          title: "Queue census counts a parked task twice",
          url: "https://github.com/developing-today/code/issues/14101",
          at: iso(60 * 5),
          reason: "not_planned",
        },
        {
          number: 13820,
          title: "Console: Needs me shows 394",
          url: "https://github.com/developing-today/code/issues/13820",
          at: iso(60 * 40),
          reason: "completed",
        },
      ],
    },
  },
  priorities: {
    truncated: false,
    issues: [
      {
        number: 4000,
        title: "id: Phase 12 exit",
        url: "https://github.com/developing-today/code/issues/4000",
        updated: iso(90),
        comments: 14,
        labels: ["expedite"],
        rank: "expedite",
        order: 0,
      },
      {
        number: 12279,
        title: "id: join at admission (slice 3)",
        url: "https://github.com/developing-today/code/issues/12279",
        updated: iso(300),
        comments: 2,
        labels: ["now"],
        rank: "now",
        order: 2,
      },
    ],
  },
  agents: [
    { name: "conductor", groups: ["opencode"], last_seen: iso(2), unread_commands: 0, unread_messages: 1 },
    { name: "muse", groups: ["research"], last_seen: iso(400), unread_commands: 2, unread_messages: 0 },
  ],
  services: [
    { unit: "pm-aggregator.service", load: "loaded", active: "active", sub: "running", description: "PM dashboard" },
    {
      unit: "opencode-go-seating-guard.service",
      load: "loaded",
      active: "failed",
      sub: "failed",
      description: "OpenCode Go seating guard",
    },
    {
      unit: "queue-broker.service",
      load: "loaded",
      active: "active",
      sub: "running",
      description: "Repo queue broker",
    },
  ],
  more: [
    {
      group: "Overview",
      pages: [
        {
          title: "Overview",
          route: "/?dashboard",
          blurb: "The stack front door: needs-you, projects, quotas and health at a glance.",
        },
        { title: "Today", route: "/today", blurb: "What changed today." },
      ],
    },
    { group: "Work", pages: [{ title: "Roadmap", route: "/roadmap", blurb: "Phases and what is in each." }] },
  ],
};

/** The commands the console runs, answered from nothing: every one
 * succeeds with an empty list, and the sign-in script remembers whether
 * it was handed a Google client. */
let demoGoogle = false;
const demoRun: Run = async (argv) => {
  if (argv.includes("--google-stdin")) {
    demoGoogle = true;
    return { code: 0, stdout: "changed: access: add Google sign-in\n", stderr: "" };
  }
  if (argv.includes("--status")) {
    return {
      code: 0,
      stdout: `${JSON.stringify({ google: demoGoogle, pin: true, team: "your-team.cloudflareaccess.com" })}\n`,
      stderr: "",
    };
  }
  // The Releases screen: GitHub's record of the site's deploys and the
  // app's builds, and a start that is taken and does nothing here.
  const path = argv.find((a) => a.startsWith("repos/")) ?? "";
  if (path.includes("/actions/workflows/") && path.includes("/runs")) {
    const site = path.includes("pm-console-edge-deploy.yml");
    const run = (
      id: number,
      sha: string,
      status: string,
      conclusion: string | null,
      event: string,
      minutes: number,
    ) => ({
      id,
      head_sha: sha,
      status,
      conclusion,
      event,
      created_at: iso(minutes),
      html_url: `https://github.com/developing-today/code/actions/runs/${id}`,
    });
    const runs = site
      ? [
          run(45120541236, "ca5c04493289cde76fa0d7f287bbd46e0395c491", "completed", "success", "push", 9),
          run(45119509023, "326ac803beddb586530c6e48b76ce1db630e8aca", "completed", "success", "push", 18),
          run(
            45098200037,
            "2bec21c17fae00000000000000000000000000aa",
            "completed",
            "success",
            "workflow_dispatch",
            235,
          ),
        ]
      : [run(45118133709, "0f311ae99323955fbe908d8db814dc8de41d56f0", "completed", "success", "push", 31)];
    return { code: 0, stdout: JSON.stringify({ workflow_runs: runs }), stderr: "" };
  }
  if (path.endsWith("/dispatches")) return { code: 0, stdout: "", stderr: "" };
  // The plan's screens: the roadmap files are the repository's own, read
  // from this checkout; the issues under them are made up.
  const answer = (out: unknown) => ({ code: 0, stdout: JSON.stringify(out), stderr: "" });
  // So is the list of Workers the Platform screen is drawn from.
  const contents = /\/contents\/(planning\/roadmaps\/[a-z-]+\.md|pm-stack\/platform\/workers\.json)$/.exec(path);
  if (contents) {
    const text = await Bun.file(`${import.meta.dir}/../../../${contents[1]}`).text();
    return answer({ content: Buffer.from(text, "utf8").toString("base64") });
  }
  const planIssue = (number: number, title: string, labels: string[], state = "open", minutes = 90) => ({
    number,
    title,
    html_url: `https://github.com/developing-today/code/issues/${number}`,
    state,
    updated_at: iso(minutes),
    comments: number % 5,
    labels: labels.map((name) => ({ name })),
  });
  const q = argv.find((a) => a.startsWith("q="));
  if (argv.includes("search/issues") && q) {
    if (q.includes("kind/bug")) {
      return answer({
        total_count: q.includes("is:closed") ? 516 : 3,
        items: q.includes("is:closed")
          ? [
              planIssue(
                12931,
                "The Android release tracked an older build",
                ["kind/bug", "app/pm-stack"],
                "closed",
                600,
              ),
            ]
          : [
              planIssue(12520, "flake.lock diverges after a rebuild marker commit", ["kind/bug", "next"], "open", 2000),
              planIssue(12980, "Deploy again does nothing on a phone with no signal", [
                "kind/bug",
                "app/pm-stack",
                "now",
              ]),
              planIssue(12811, "A claimed task shows as queued for a minute", ["kind/bug", "app/fleet"], "open", 300),
            ],
      });
    }
    const of = /"(app\/[a-z-]+|area\/[a-z-]+|roadmap\/[a-z-]+)"/.exec(q)?.[1] ?? "app/id";
    const n = 12000 + (of.length % 7) * 100;
    if (q.includes("is:closed")) {
      return answer({
        total_count: 146,
        items: [planIssue(n + 1, "The piece that shipped last week", [of], "closed", 3000)],
      });
    }
    return answer({
      total_count: 4,
      items: [
        planIssue(n + 2, "The piece being built now", [of, "queue/claimed", "now"], "open", 12),
        planIssue(n + 3, "The piece that is up next", [of, "now"], "open", 240),
        planIssue(n + 4, "A piece for the coming weeks", [of, "next"], "open", 900),
        planIssue(n + 5, "epic: the long road for this capability", [of, "meta/epic"], "open", 5000),
      ],
    });
  }
  if (path.includes("/issues?labels=owner%2Fintake")) {
    return answer([
      planIssue(
        12980,
        "Bug (PM stack): Deploy again does nothing on a phone with no signal",
        ["kind/bug", "owner/intake", "app/pm-stack", "now", "queue/claimed"],
        "open",
        20,
      ),
      {
        ...planIssue(
          12975,
          'id roadmap: move "Native TUI client" to "Planned: short term"',
          ["owner/intake", "app/id", "next"],
          "open",
          200,
        ),
        body: 'sooner than later\n\n---\n<!-- pm-intake {"v":1,"id":null,"kind":"change","area":"id","mode":"issue"} -->',
      },
      planIssue(
        12960,
        "Owner: make lanes pick up owner notes first",
        ["owner/prompt", "owner/intake", "app/fleet", "expedite"],
        "closed",
        700,
      ),
    ]);
  }
  if (argv.includes("POST")) {
    if (path.endsWith("/comments")) {
      return answer({ html_url: "https://github.com/developing-today/code/issues/12980#issuecomment-1" });
    }
    if (path.endsWith("/issues")) {
      return answer({
        number: 13050,
        html_url: "https://github.com/developing-today/code/issues/13050",
        title: "Filed from the demo",
      });
    }
  }
  return { code: 0, stdout: "[]", stderr: "" };
};

/** The queue store, standing in for the real one, so that the Queue
 * screen has something to read and nothing here goes out to the
 * network. It answers the two reads that screen makes; a task that was
 * unblocked or cancelled stays as it was, since nothing here changes. */
const queueTasks = [
  {
    id: 21043,
    status: "blocked",
    priority: 20,
    title: "Fix #12538: modeline verbs agree with the queue's states",
    dedupe_key: "issue-12538",
    updated: iso(14),
    why: "waiting on #12530 to merge",
  },
  {
    id: 20977,
    status: "blocked",
    priority: 30,
    title: "Fix #12279: every test file sits in a registry row",
    dedupe_key: "issue-12279",
    updated: iso(95),
    why: "36 test files are in no registry row; the lint this slice's pull request runs fails on them",
  },
  {
    id: 20412,
    status: "blocked",
    priority: 50,
    title: "Review #12403: quota forecast accuracy ledger",
    dedupe_key: "review-12403",
    updated: iso(400),
    why: "the author's session ended before answering the review",
  },
  {
    id: 19850,
    status: "blocked",
    priority: null,
    title: "Fix #11871: watchdog registry row for the new claimant",
    dedupe_key: "issue-11871",
    updated: iso(2900),
    why: "",
  },
  {
    id: 21101,
    status: "queued",
    priority: 10,
    title: "Fix #12600: the session id reaches the resume path",
    dedupe_key: "issue-12600",
    updated: iso(3),
    why: "",
  },
  {
    id: 21099,
    status: "queued",
    priority: 30,
    title: "Fix #12588: stale claims are released after a restart",
    dedupe_key: "issue-12588",
    updated: iso(22),
    why: "",
  },
  {
    id: 21050,
    status: "claimed",
    priority: 20,
    title: "Fix #9075: session state — session id in env and a resume path",
    dedupe_key: "issue-9075",
    updated: iso(1),
    why: "",
  },
  {
    id: 18002,
    status: "parked",
    priority: 70,
    title: "Fix #10210: tidy the exporter's flags",
    dedupe_key: "issue-10210",
    updated: iso(9000),
    why: "parked until the exporter is rewritten",
  },
];
const demoQueue: Fetch = async (_url, init) => {
  const requests =
    (
      JSON.parse(init.body ?? "{}") as {
        requests?: { type: string; stmt?: { sql: string; args: { value: string }[] } }[];
      }
    ).requests ?? [];
  const cell = (value: string | number | null) =>
    value === null ? { type: "null" } : { type: typeof value === "number" ? "integer" : "text", value };
  const results = requests
    .filter((r) => r.type === "execute")
    .map((r) => {
      const sql = r.stmt?.sql ?? "";
      const args = (r.stmt?.args ?? []).map((a) => a.value);
      let cols: string[] = [];
      let rows: (string | number | null)[][] = [];
      if (/count\(\*\) AS n FROM tasks/.test(sql)) {
        cols = ["status", "n"];
        rows = Object.entries(state.board.counts);
      } else if (/^SELECT id, status, priority, title/.test(sql)) {
        const [, status, like] = args;
        const want = String(like ?? "%%")
          .slice(1, -1)
          .replace(/\\([\\%_])/g, "$1")
          .toLowerCase();
        cols = ["id", "status", "priority", "title", "dedupe_key", "updated", "why"];
        rows = queueTasks
          .filter((t) => t.status === status && t.title.toLowerCase().includes(want))
          .map((t) => [t.id, t.status, t.priority, t.title, t.dedupe_key, t.updated.slice(0, 16), t.why]);
      }
      return {
        type: "ok",
        response: {
          type: "execute",
          result: {
            cols: cols.map((name) => ({ name })),
            rows: rows.map((row) => row.map(cell)),
            affected_row_count: 0,
          },
        },
      };
    });
  return { ok: true, status: 200, json: async () => ({ results }) };
};

const cfg = readConfig({ HOME: "/tmp", PM_CONSOLE_DASHBOARD_URL: "" });
const docs = new Docs(
  Object.fromEntries(Object.entries(state).map(([name, data]) => [name, { ttlMs: 3_600_000, load: async () => data }])),
);
// The offline check's witness: when PM_CONSOLE_DEMO_ACT_LOG names a
// file, every canned action the demo host runs is appended there as one
// JSON line (kind, args, time), so a driver outside the browser can
// count what arrived and when. Unset, the demo writes nothing. A log
// that cannot be written is reported on stderr and the action still
// runs: the check then fails at its own count, not inside the demo.
const done = (kind: string, result: unknown) => ({
  run: async (args: unknown) => {
    const actLog = process.env.PM_CONSOLE_DEMO_ACT_LOG;
    if (actLog) {
      try {
        appendFileSync(actLog, `${JSON.stringify({ at: new Date().toISOString(), kind, args })}\n`);
      } catch (error) {
        console.error(`demo act log ${actLog} not writable: ${error}`);
      }
    }
    return result;
  },
  touch: [] as string[],
  audit: () => ({}),
});
// Linking a phone is the real thing, kept in a file under /tmp.
const devices = new Devices(fileStore("/tmp/pm-console-demo-devices.json"));
const phones = { devices, link: new Limiter(30, 60_000), refused: new Limiter(60, 60_000) };
// The plan (roadmaps, bugs, intake) is the real thing too, over the
// stand-in for GitHub above: what the demo shows is what the code does.
const demoOps = new Ops(cfg, demoRun, demoQueue);
const demoWork = new Work(demoOps);
const demoPlan = new Plan(demoOps, demoWork);
const real = realActions(demoOps, demoWork, undefined, devices, demoPlan);
const table = new Proxy({} as Record<string, ReturnType<typeof done>>, {
  get: (_t, kind) =>
    typeof kind === "string" && (kind.startsWith("device.") || kind === "sso.google" || kind === "intake.send")
      ? real[kind]
      : done(
          typeof kind === "string" ? kind : String(kind),
          kind === "ask.send"
            ? { issue: 12503, queued: true }
            : kind === "questions.accept"
              ? { applied: 2, refused: [] }
              : { ok: true },
        ),
  has: () => true,
  // The kinds a real machine has, for where what it can do is listed.
  ownKeys: () => Object.keys(real),
  getOwnPropertyDescriptor: () => ({ configurable: true, enumerable: true, value: done("?", {}) }),
});

/** The made-up host: canned state, and actions that all succeed. */
export const demoDeps: Deps = {
  cfg,
  ops: demoOps,
  plan: demoPlan,
  docs,
  phones,
  push: new Push(fileStore("/tmp/pm-console-demo-push.json"), "mailto:you@example.com", async () => ({
    status: 201,
  })),
  actions: table,
  // What was done under each name, as the real host keeps it: an action
  // a device sends twice is done once here too.
  named: new Named(memoryStore()),
  // What this machine is called where machines are listed.
  self: { name: "framework", version: "0123abcd4567" },
  identify: async () => ({ via: "demo", email: "you@example.com" }),
  audit: async () => {},
};
export { docs as demoDocs };
