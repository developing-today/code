import { describe, expect, test } from "bun:test";
import { chmod, mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { watchdogHeartbeatLine, WATCHDOG_HEARTBEAT_INTERVAL_MS } from "../src/watchdog-heartbeat.ts";
import { SloJournal } from "../src/slo-journal.ts";

const script = join(import.meta.dir, "../scripts/check-watchdog-heartbeat.ts");

async function fakePostAlertCommand(dir: string): Promise<{ path: string; capture: string }> {
  const path = join(dir, "bin", "bun");
  const capture = join(dir, "post-alert-args.txt");
  await mkdir(join(dir, "bin"), { recursive: true });
  await Bun.write(path, '#!/bin/sh\nprintf "%s\\n" "$*" > "$WATCHDOG_TEST_POST_ALERT_ARGS"\nexit "${WATCHDOG_TEST_POST_ALERT_EXIT:-0}"\n');
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
      expect(missingJournal.code).toBe(1);
      expect(missingJournal.stderr).toContain("no valid watchdog heartbeat");
      await expect(Bun.file(fake.capture).exists()).resolves.toBe(false);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("posts orchestrator and page alerts through the existing alert CLI", async () => {
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
      expect(await Bun.file(argsFile).text()).toContain("watchdog-heartbeat-needs-orchestrator");

      const pageFile = join(dir, "page-journal.jsonl");
      await new SloJournal(pageFile).append([watchdogHeartbeatLine(Date.now() - 3 * WATCHDOG_HEARTBEAT_INTERVAL_MS)]);
      const page = await runScript(["--journal", pageFile], env);
      expect(page.code).toBe(0);
      expect(await Bun.file(argsFile).text()).toContain("--severity critical");
      expect(await Bun.file(argsFile).text()).toContain("watchdog-heartbeat-page-drew");
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
});
