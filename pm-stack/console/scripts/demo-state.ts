// A made-up host, to look at the interface without a fleet: the real
// server code answers, only the sources are canned, and every action
// succeeds. demo.ts serves it directly; edge-demo.ts puts the edge in
// front of it.
import { actions as realActions } from "../src/actions.ts";
import { readConfig } from "../src/config.ts";
import { Devices, Limiter } from "../src/devices.ts";
import { Docs } from "../src/docs.ts";
import { fileStore } from "../src/file-store.ts";
import { Ops } from "../src/ops.ts";
import { Push } from "../src/push.ts";
import type { Deps } from "../src/server.ts";

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
    },
    {
      number: 12241,
      title: "feat(id): F12.11 binary frames, encoder",
      draft: false,
      url: "https://github.com/developing-today/code/pull/12241",
      created: iso(60 * 19),
      author: "dezren39",
      branch: "task/f12-11-frames",
    },
    {
      number: 12390,
      title: "wip: talos bootstrap notes",
      draft: true,
      url: "https://github.com/developing-today/code/pull/12390",
      created: iso(60 * 3),
      author: "dezren39",
      branch: "wip/talos",
    },
  ],
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

const cfg = readConfig({ HOME: "/tmp", PM_CONSOLE_DASHBOARD_URL: "" });
const docs = new Docs(
  Object.fromEntries(Object.entries(state).map(([name, data]) => [name, { ttlMs: 3_600_000, load: async () => data }])),
);
const done = (result: unknown) => ({ run: async () => result, touch: [] as string[], audit: () => ({}) });
// Linking a phone is the real thing, kept in a file under /tmp.
const devices = new Devices(fileStore("/tmp/pm-console-demo-devices.json"));
const phones = { devices, link: new Limiter(30, 60_000), refused: new Limiter(60, 60_000) };
const real = realActions(
  new Ops(cfg, async () => ({ code: 0, stdout: "[]", stderr: "" })),
  undefined as never,
  undefined,
  devices,
);
const table = new Proxy({} as Record<string, ReturnType<typeof done>>, {
  get: (_t, kind) =>
    typeof kind === "string" && kind.startsWith("device.")
      ? real[kind]
      : done(
          kind === "ask.send"
            ? { issue: 12503, queued: true }
            : kind === "questions.accept"
              ? { applied: 2, refused: [] }
              : { ok: true },
        ),
  has: () => true,
  getOwnPropertyDescriptor: () => ({ configurable: true, enumerable: true, value: done({}) }),
});

/** The made-up host: canned state, and actions that all succeed. */
export const demoDeps: Deps = {
  cfg,
  ops: new Ops(cfg, async () => ({ code: 0, stdout: "[]", stderr: "" })),
  docs,
  phones,
  push: new Push(fileStore("/tmp/pm-console-demo-push.json"), "mailto:you@example.com", async () => ({
    status: 201,
  })),
  actions: table,
  identify: async () => ({ via: "demo", email: "you@example.com" }),
  audit: async () => {},
};
export { docs as demoDocs };
