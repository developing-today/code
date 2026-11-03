import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { spawn, type Subprocess } from "bun";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

// #1145 follow-up: buildHealth assembled the pending comment counts and
// /health rendered them, but the /api/health handler never serialized
// view.comments — the live API omitted the counts the acceptance asks
// the "health view/API" to carry. This test boots a live-source server
// against a seeded comments store and asserts on the /api/health body.

interface HealthBody {
  status: string;
  version?: string;
  comments?: { pendingMessages: number; pendingGithub: number; total: number };
}

let proc: Subprocess | undefined;
let health: HealthBody | undefined;

const comment = (over: Record<string, unknown>) => ({
  id: "dc-test-1",
  objectRef: "gh:developing-today/code#1145",
  kind: "note",
  author: { kind: "drew", id: "drew", label: "Drew" },
  body: "seeded for api-health-comments test",
  createdAt: "2026-10-29T00:00:00.000Z",
  ...over,
});

beforeAll(async () => {
  const dir = await mkdtemp(join(tmpdir(), "pm-api-health-comments-"));
  const now = new Date().toISOString();
  const feed = {
    version: 1,
    generatedAt: now,
    quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
    attention: [],
    orchestration: {
      version: 1,
      generatedAt: now,
      runId: "orch-api-health-comments-test",
      provenance: { sources: ["api-health-comments test"], runId: "orch-api-health-comments-test" },
      tiers: { needsDrew: [], needsOrchestrator: [] },
      dependencies: [],
      goals: [],
      actions: [],
    },
    registry: [],
  };
  const feedFile = join(dir, "fleet-feed.json");
  await writeFile(feedFile, JSON.stringify(feed));
  const commentsFile = join(dir, "comments.json");
  await writeFile(
    commentsFile,
    JSON.stringify({
      version: 1,
      comments: [
        comment({ id: "dc-test-msg-new", kind: "message", triageState: "new" }),
        comment({ id: "dc-test-msg-done", kind: "message", triageState: "triaged" }),
        comment({
          id: "dc-test-gh-pending",
          kind: "github",
          githubRef: "developing-today/code#1145",
          githubState: "pending",
        }),
        comment({
          id: "dc-test-gh-posted",
          kind: "github",
          githubRef: "developing-today/code#1145",
          githubState: "posted",
          githubUrl: "https://github.com/developing-today/code/issues/1145#issuecomment-1",
        }),
      ],
    }),
  );
  const port = 5900 + Math.floor(Math.random() * 800);
  proc = spawn({
    cmd: ["bun", "run", "src/server.ts"],
    cwd: join(import.meta.dir, ".."),
    env: {
      ...Bun.env,
      PM_AGG_PORT: String(port),
      PM_AGG_SOURCE: "live",
      PM_AGG_BUILD_VERSION: "test-build-1150",
      PM_AGG_FEED_FILE: feedFile,
      PM_AGG_REGISTRY_FILE: join(dir, "registry.json"),
      PM_AGG_COMMENTS_FILE: commentsFile,
      PM_AGG_SNAPSHOTS_DIR: join(dir, "snapshots"),
      PM_AGG_OPENCHAMBER_STATE: join(dir, "no-state.json"),
      PM_AGG_QUOTA_VIEW: join(dir, "no-quota-view.md"),
      PM_AGG_QUOTA_REGISTRY: join(dir, "no-quota-registry.md"),
      PM_PROJECTOR_URL: "off",
      KANEO_ADMIN_EMAIL: "",
      KANEO_ADMIN_PASSWORD: "",
      GRIST_API_KEY: "",
    },
    stdout: "pipe",
    stderr: "inherit",
  });
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline && !health) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/api/health`);
      if (res.ok) health = (await res.json()) as HealthBody;
    } catch {
      /* not up yet */
    }
    if (!health) await Bun.sleep(150);
  }
  expect(health).toBeDefined();
});

afterAll(() => {
  proc?.kill();
});

describe("/api/health build version (#1150)", () => {
  test("the API body carries the deployed build marker from PM_AGG_BUILD_VERSION", () => {
    expect(health?.version).toBe("test-build-1150");
  });
});

describe("/api/health comment counts (#1145)", () => {
  test("the API body carries pending message/github counts from the store", () => {
    expect(health?.status).toBe("ok");
    expect(health?.comments).toEqual({ pendingMessages: 1, pendingGithub: 1, total: 4 });
  });
});
