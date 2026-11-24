import { describe, expect, test } from "bun:test";
import { chmod, mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { checkWatchdogHeartbeatJournal } from "../src/watchdog-heartbeat-checker.ts";
import { watchdogHeartbeatLine, WATCHDOG_HEARTBEAT_INTERVAL_MS } from "../src/watchdog-heartbeat.ts";
import {
  NEEDS_ORCHESTRATOR_CONDITION,
  PM_STACK_WATCHDOG_NAME,
  publishToOrchestratorFeed,
  readOrchestratorFeed,
} from "../src/watchdog-orchestrator-feed.ts";
import { SloJournal } from "../src/slo-journal.ts";

// Oracle (#14410 acceptance): the independent checker's own journal
// derivation is the oracle — alert "needs-orchestrator" iff >= 2 missed
// five-minute cycles. The feed must contain exactly one item per outage
// iff that alert fired, and nothing otherwise. No aggregator is started
// in this file: publication is a local file append, so every test here
// is also the "aggregator down" case.
describe("watchdog orchestrator feed (#14410)", () => {
  test("two-cycle NEEDS_ORCHESTRATOR reaches the orchestrator feed with the aggregator down; single cycle and recovery publish nothing", async () => {
    const dir = await mkdtemp(join(tmpdir(), "watchdog-orchestrator-feed-"));
    try {
      const journalFile = join(dir, "slo-journal.jsonl");
      const feedFile = join(dir, "orchestrator-feed.jsonl");
      const journal = new SloJournal(journalFile);
      const startedAt = 1_000_000;
      await journal.append([watchdogHeartbeatLine(startedAt)]);

      // Single missed cycle: pending only, nothing published.
      const oneCycle = await checkWatchdogHeartbeatJournal(journalFile, startedAt + WATCHDOG_HEARTBEAT_INTERVAL_MS);
      expect(oneCycle.alert).toBe("none");
      expect(await publishToOrchestratorFeed(oneCycle, feedFile, startedAt + WATCHDOG_HEARTBEAT_INTERVAL_MS)).toEqual({
        published: false,
        item: null,
      });
      expect(await readOrchestratorFeed(feedFile)).toEqual([]);

      // Second consecutive missed cycle: exactly one item, naming the
      // watchdog, the condition, and the cycles observed.
      const twoCyclesAt = startedAt + 2 * WATCHDOG_HEARTBEAT_INTERVAL_MS;
      const twoCycles = await checkWatchdogHeartbeatJournal(journalFile, twoCyclesAt);
      expect(twoCycles.alert).toBe("needs-orchestrator");
      const publication = await publishToOrchestratorFeed(twoCycles, feedFile, twoCyclesAt);
      expect(publication.published).toBe(true);
      const feed = await readOrchestratorFeed(feedFile);
      expect(feed).toHaveLength(1);
      expect(feed[0]).toMatchObject({
        kind: "watchdog-escalation",
        id: `watchdog-heartbeat-${startedAt}-needs-orchestrator`,
        watchdog: PM_STACK_WATCHDOG_NAME,
        alert: "needs-orchestrator",
        condition: NEEDS_ORCHESTRATOR_CONDITION,
        cyclesObserved: 2,
        outageSinceMs: startedAt,
        lastHeartbeatAtMs: startedAt,
        publishedAtMs: twoCyclesAt,
      });
      expect(feed[0]!.title).toContain("NEEDS ORCHESTRATOR");
      expect(feed[0]!.title).toContain("watchdog");
      expect(feed[0]!.body).toContain("2 five-minute cycles");

      // A repeat checker run for the same outage publishes nothing more.
      const repeat = await publishToOrchestratorFeed(twoCycles, feedFile, twoCyclesAt + 1_000);
      expect(repeat.published).toBe(false);
      expect(await readOrchestratorFeed(feedFile)).toHaveLength(1);

      // Recovery: a fresh heartbeat clears the pending/published state
      // without publishing — the next check is healthy and adds nothing.
      const recoveredAt = twoCyclesAt + WATCHDOG_HEARTBEAT_INTERVAL_MS;
      await journal.append([watchdogHeartbeatLine(recoveredAt)]);
      const recovered = await checkWatchdogHeartbeatJournal(journalFile, recoveredAt);
      expect(recovered.alert).toBe("none");
      expect(await publishToOrchestratorFeed(recovered, feedFile, recoveredAt)).toEqual({
        published: false,
        item: null,
      });
      expect(await readOrchestratorFeed(feedFile)).toHaveLength(1);

      // A later outage is a new item (recovery really cleared state):
      // two missed cycles after the recovery heartbeat publish again.
      const laterOutageAt = recoveredAt + 2 * WATCHDOG_HEARTBEAT_INTERVAL_MS;
      const laterOutage = await checkWatchdogHeartbeatJournal(journalFile, laterOutageAt);
      expect(laterOutage.alert).toBe("needs-orchestrator");
      expect((await publishToOrchestratorFeed(laterOutage, feedFile, laterOutageAt)).published).toBe(true);
      const finalFeed = await readOrchestratorFeed(feedFile);
      expect(finalFeed).toHaveLength(2);
      expect(finalFeed[1]).toMatchObject({
        id: `watchdog-heartbeat-${recoveredAt}-needs-orchestrator`,
        cyclesObserved: 2,
        outageSinceMs: recoveredAt,
      });
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("checker script publishes to the feed file on the independent path, exactly once across runs", async () => {
    const dir = await mkdtemp(join(tmpdir(), "watchdog-orchestrator-feed-runner-"));
    try {
      // Fake `bun` intercepts only the post-alert subprocess (the
      // aggregator-dependent ledger post); the feed append is the
      // checker's own file write and needs no aggregator at all.
      const bin = join(dir, "bin");
      await mkdir(bin, { recursive: true });
      const bunPath = join(bin, "bun");
      await Bun.write(bunPath, '#!/bin/sh\nexit 0\n');
      await chmod(bunPath, 0o755);

      const journalFile = join(dir, "journal.jsonl");
      const feedFile = join(dir, "feed.jsonl");
      await new SloJournal(journalFile).append([
        watchdogHeartbeatLine(Date.now() - 2 * WATCHDOG_HEARTBEAT_INTERVAL_MS),
      ]);
      const script = join(import.meta.dir, "../scripts/check-watchdog-heartbeat.ts");
      const env = { ...process.env, PATH: `${bin}:${process.env.PATH ?? ""}` };
      for (let run = 0; run < 2; run++) {
        const proc = Bun.spawn(
          [process.execPath, "run", script, "--journal", journalFile, "--orchestrator-feed", feedFile],
          { env, stdout: "pipe", stderr: "pipe" },
        );
        const [code, stdout] = await Promise.all([proc.exited, new Response(proc.stdout).text()]);
        expect(code).toBe(0);
        expect(stdout).toContain("NEEDS_ORCHESTRATOR: PM-stack watchdog heartbeat missing");
      }
      const feed = await readOrchestratorFeed(feedFile);
      expect(feed).toHaveLength(1);
      expect(feed[0]).toMatchObject({
        watchdog: PM_STACK_WATCHDOG_NAME,
        condition: NEEDS_ORCHESTRATOR_CONDITION,
        cyclesObserved: 2,
      });

      // Strict arg contract still holds for the new flag.
      const bad = Bun.spawn([process.execPath, "run", script, "--orchestrator-feed"], {
        env,
        stdout: "pipe",
        stderr: "pipe",
      });
      const [badCode, badStderr] = await Promise.all([bad.exited, new Response(bad.stderr).text()]);
      expect(badCode).toBe(2);
      expect(badStderr).toContain("requires a value");
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});
