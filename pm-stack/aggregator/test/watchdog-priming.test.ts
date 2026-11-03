import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { spawn, type Subprocess } from "bun";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

// #1148 / Codex P2 on PR #2431: the registry watchdog item must not
// depend on an earlier registry read. provenanceDetail().registry.ageMs
// used to stay undefined until some request called source.registry(),
// so a fresh process whose FIRST request was /api/attention.json
// omitted watchdog:registry even with a two-hour-old feed. This test
// boots a live-source server against a 2h-old feed and asserts on the
// FIRST response the process ever serves.

interface AttentionEntry {
  id?: string;
}

let proc: Subprocess | undefined;
let firstAttention: AttentionEntry[] | undefined;

beforeAll(async () => {
  const dir = await mkdtemp(join(tmpdir(), "pm-watchdog-"));
  const twoHoursAgo = new Date(Date.now() - 2 * 60 * 60_000).toISOString();
  const feed = {
    version: 1,
    generatedAt: twoHoursAgo,
    quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
    attention: [],
    orchestration: {
      version: 1,
      generatedAt: twoHoursAgo,
      runId: "orch-watchdog-test",
      provenance: { sources: ["watchdog priming test"], runId: "orch-watchdog-test" },
      tiers: { needsDrew: [], needsOrchestrator: [] },
      dependencies: [],
      goals: [],
      actions: [],
    },
    registry: [{ id: "ses-watchdog-1", source: "opencode-sandbox", label: "Watchdog probe session" }],
  };
  const feedFile = join(dir, "fleet-feed.json");
  await writeFile(feedFile, JSON.stringify(feed));
  const port = 5100 + Math.floor(Math.random() * 800);
  proc = spawn({
    cmd: ["bun", "run", "src/server.ts"],
    cwd: join(import.meta.dir, ".."),
    env: {
      ...Bun.env,
      PM_AGG_PORT: String(port),
      PM_AGG_SOURCE: "live",
      PM_AGG_FEED_FILE: feedFile,
      PM_AGG_REGISTRY_FILE: join(dir, "registry.json"),
      PM_AGG_COMMENTS_FILE: join(dir, "comments.json"),
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
  // Poll until the server answers; the first successful body IS the
  // assertion target — nothing else touches this process beforehand,
  // and in particular no request ever exercises source.registry().
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline && !firstAttention) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/api/attention.json`);
      if (res.ok) firstAttention = (await res.json()) as AttentionEntry[];
    } catch {
      /* not up yet */
    }
    if (!firstAttention) await Bun.sleep(150);
  }
  expect(firstAttention).toBeDefined();
});

afterAll(() => {
  proc?.kill();
});

describe("watchdog registry derivation needs no registry priming (#1148, Codex P2 PR #2431)", () => {
  test("the first-ever /api/attention.json already carries exactly one watchdog:registry item", () => {
    const ids = (firstAttention ?? []).map((e) => e.id);
    expect(ids.filter((id) => id === "watchdog:registry")).toHaveLength(1);
    // The 2h-old feed breaches the 15m feed threshold too.
    expect(ids.filter((id) => id === "watchdog:fleet-feed")).toHaveLength(1);
  });
});
