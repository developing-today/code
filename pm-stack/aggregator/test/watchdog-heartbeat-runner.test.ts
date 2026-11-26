import { describe, expect, test } from "bun:test";
import { chmod, mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { watchdogHeartbeatLine, WATCHDOG_HEARTBEAT_INTERVAL_MS } from "../src/watchdog-heartbeat.ts";
import { SloJournal } from "../src/slo-journal.ts";

const script = join(import.meta.dir, "../scripts/check-watchdog-heartbeat.ts");

async function fakePostAlertCommand(dir: string): Promise<{ path: string; capture: string }> {
  const path = join(dir, "bin", "bun");
  const capture = join(dir, "post-alert-args.txt");
  await mkdir(join(dir, "bin"), { recursive: true });
  await Bun.write(
    path,
    '#!/bin/sh\nprintf "%s\\n" "$*" > "$WATCHDOG_TEST_POST_ALERT_ARGS"\nexit "${WATCHDOG_TEST_POST_ALERT_EXIT:-0}"\n',
  );
  await chmod(path, 0o755);
  return { path, capture };
}

async function runScript(args: string[], env: Record<string, string>) {
  const proc = Bun.spawn([process.execPath, "run", script, ...args], {
    env: { ...process.env, ...env },
    stdout: "pipe",
    stderr: "pipe",
  });
  const [code, stdout, stderr] = await Promise.all([
    proc.exited,
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
  ]);
  return { code, stdout, stderr };
}

describe("watchdog heartbeat runner", () => {
  test("rejects unknown and valueless arguments before spawning the alert command", async () => {
    const dir = await mkdtemp(join(tmpdir(), "watchdog-heartbeat-runner-"));
    try {
      const fake = await fakePostAlertCommand(dir);
      const env = {
        PATH: `${join(dir, "bin")}:${process.env.PATH ?? ""}`,
        WATCHDOG_TEST_POST_ALERT_ARGS: fake.capture,
      };
      const unknown = await runScript(["--unknown"], env);
      const valueless = await runScript(["--journal", "--unknown"], env);
      const duplicate = await runScript(["--journal", "one", "--journal", "two"], env);
      const joined = await runScript(["--journal=/tmp/journal"], env);
      expect(unknown.code).toBe(2);
      expect(unknown.stderr).toContain("unknown argument");
      expect(valueless.code).toBe(2);
      expect(valueless.stderr).toContain("requires a value");
      expect(duplicate.code).toBe(2);
      expect(duplicate.stderr).toContain("duplicate --journal");
      expect(joined.code).toBe(2);
      expect(joined.stderr).toContain("unknown argument");
      await expect(Bun.file(fake.capture).exists()).resolves.toBe(false);

      const missingJournal = await runScript(["--journal", join(dir, "missing.jsonl")], env);
      expect(missingJournal.code).toBe(0);
      expect(missingJournal.stdout).toContain("unknown: no valid watchdog heartbeat or journal activity");
      expect(missingJournal.stderr).toBe("");
      await expect(Bun.file(fake.capture).exists()).resolves.toBe(false);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("posts orchestrator alerts through the alert CLI and emits pages independently", async () => {
    const dir = await mkdtemp(join(tmpdir(), "watchdog-heartbeat-runner-"));
    try {
      const fake = await fakePostAlertCommand(dir);
      const file = join(dir, "journal.jsonl");
      const journal = new SloJournal(file);
      const argsFile = fake.capture;
      const env = {
        PATH: `${join(dir, "bin")}:${process.env.PATH ?? ""}`,
        WATCHDOG_TEST_POST_ALERT_ARGS: argsFile,
      };

      await journal.append([watchdogHeartbeatLine(Date.now() - 2 * WATCHDOG_HEARTBEAT_INTERVAL_MS)]);
      const orchestrator = await runScript(["--journal", file], env);
      expect(orchestrator.code).toBe(0);
      expect(await Bun.file(argsFile).text()).toContain("--severity warning");
      const orchestratorArgs = await Bun.file(argsFile).text();
      expect(orchestratorArgs).toContain("watchdog-heartbeat-");
      expect(orchestratorArgs).toContain("-needs-orchestrator");

      await rm(argsFile);
      const pageFile = join(dir, "page-journal.jsonl");
      await new SloJournal(pageFile).append([watchdogHeartbeatLine(Date.now() - 3 * WATCHDOG_HEARTBEAT_INTERVAL_MS)]);
      const page = await runScript(["--journal", pageFile], env);
      expect(page.code).toBe(0);
      expect(page.stdout).toContain("PAGE_DREW: PM-stack watchdog heartbeat missing");
      expect(page.stdout).toContain("3 five-minute cycles");
      await expect(Bun.file(argsFile).exists()).resolves.toBe(false);
      const incidentKey = (args: string) => args.match(/watchdog-heartbeat-[^ ]+/)?.[0];

      const recoveredEpisodeFile = join(dir, "recovered-episode-journal.jsonl");
      await new SloJournal(recoveredEpisodeFile).append([
        watchdogHeartbeatLine(Date.now() - 2 * WATCHDOG_HEARTBEAT_INTERVAL_MS - 60_000),
      ]);
      const laterOutage = await runScript(["--journal", recoveredEpisodeFile], env);
      expect(laterOutage.code).toBe(0);
      expect(incidentKey(await Bun.file(argsFile).text())).not.toBe(incidentKey(orchestratorArgs));
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("first run on a pre-monitoring journal persists activation instead of paging", async () => {
    const dir = await mkdtemp(join(tmpdir(), "watchdog-heartbeat-runner-"));
    try {
      const fake = await fakePostAlertCommand(dir);
      const env = {
        PATH: `${join(dir, "bin")}:${process.env.PATH ?? ""}`,
        WATCHDOG_TEST_POST_ALERT_ARGS: fake.capture,
      };
      const file = join(dir, "journal.jsonl");
      const stateFile = `${file}.checker-state.json`;
      // Journal activity from three days ago, no heartbeat ever: the
      // journal predates heartbeat monitoring, so the first checker run
      // must open the startup grace (persist activation), not page.
      await writeFile(
        file,
        `${JSON.stringify({ facet: "feed-age", atMs: Date.now() - 3 * 24 * 60 * 60_000, bad: false, value: 1 })}\n`,
      );
      const before = Date.now();
      const first = await runScript(["--journal", file], env);
      const after = Date.now();
      expect(first.code).toBe(0);
      expect(first.stdout).not.toContain("PAGE_DREW");
      expect(first.stdout).not.toContain("NEEDS_ORCHESTRATOR");
      expect(first.stderr).toBe("");
      await expect(Bun.file(fake.capture).exists()).resolves.toBe(false);
      const state = JSON.parse(await Bun.file(stateFile).text()) as { activationAtMs: number };
      expect(state.activationAtMs).toBeGreaterThanOrEqual(before);
      expect(state.activationAtMs).toBeLessThanOrEqual(after);

      // With the persisted activation aged past three cycles, the same
      // journal pages on the independent stdout channel.
      const pageActivation = Date.now() - 3 * WATCHDOG_HEARTBEAT_INTERVAL_MS;
      await writeFile(stateFile, `${JSON.stringify({ activationAtMs: pageActivation })}\n`);
      const page = await runScript(["--journal", file], env);
      expect(page.code).toBe(0);
      expect(page.stdout).toContain("PAGE_DREW: PM-stack watchdog heartbeat missing");
      expect(page.stdout).toContain("No watchdog heartbeat has ever been recorded");
      await expect(Bun.file(fake.capture).exists()).resolves.toBe(false);

      // At two cycles the escalation is both recorded through the alert
      // CLI (keyed by the activation reference) and published on stdout.
      const orchestratorActivation = Date.now() - 2 * WATCHDOG_HEARTBEAT_INTERVAL_MS;
      await writeFile(stateFile, `${JSON.stringify({ activationAtMs: orchestratorActivation })}\n`);
      const orchestrator = await runScript(["--journal", file], env);
      expect(orchestrator.code).toBe(0);
      expect(orchestrator.stdout).toContain("NEEDS_ORCHESTRATOR: PM-stack watchdog heartbeat missing");
      expect(orchestrator.stdout).toContain("2 five-minute cycles");
      expect(await Bun.file(fake.capture).text()).toContain(
        `watchdog-heartbeat-${orchestratorActivation}-needs-orchestrator`,
      );
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("surfaces alert-delivery failures with a nonzero exit", async () => {
    const dir = await mkdtemp(join(tmpdir(), "watchdog-heartbeat-runner-"));
    try {
      const fake = await fakePostAlertCommand(dir);
      const file = join(dir, "journal.jsonl");
      await new SloJournal(file).append([watchdogHeartbeatLine(Date.now() - 2 * WATCHDOG_HEARTBEAT_INTERVAL_MS)]);
      const result = await runScript(["--journal", file], {
        PATH: `${join(dir, "bin")}:${process.env.PATH ?? ""}`,
        WATCHDOG_TEST_POST_ALERT_ARGS: fake.capture,
        WATCHDOG_TEST_POST_ALERT_EXIT: "7",
      });
      expect(result.code).toBe(1);
      expect(result.stderr).toContain("post-alert exited with status 7");
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("publishes NEEDS_ORCHESTRATOR on stdout even when the aggregator alert intake is down", async () => {
    const dir = await mkdtemp(join(tmpdir(), "watchdog-heartbeat-runner-"));
    try {
      const fake = await fakePostAlertCommand(dir);
      const file = join(dir, "journal.jsonl");
      await new SloJournal(file).append([watchdogHeartbeatLine(Date.now() - 2 * WATCHDOG_HEARTBEAT_INTERVAL_MS)]);
      const result = await runScript(["--journal", file], {
        PATH: `${join(dir, "bin")}:${process.env.PATH ?? ""}`,
        WATCHDOG_TEST_POST_ALERT_ARGS: fake.capture,
        WATCHDOG_TEST_POST_ALERT_EXIT: "7",
      });
      expect(result.stdout).toContain("NEEDS_ORCHESTRATOR: PM-stack watchdog heartbeat missing");
      expect(result.stdout).toContain("2 five-minute cycles");
      expect(result.code).toBe(1);
      expect(result.stderr).toContain("post-alert exited with status 7");
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});
