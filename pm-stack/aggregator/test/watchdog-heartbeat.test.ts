import { describe, expect, test } from "bun:test";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { checkWatchdogHeartbeatJournal } from "../src/watchdog-heartbeat-checker.ts";
import { watchdogHeartbeatLine, WATCHDOG_HEARTBEAT_INTERVAL_MS } from "../src/watchdog-heartbeat.ts";
import { SloJournal } from "../src/slo-journal.ts";

describe("watchdog heartbeat", () => {
  test("the independent checker raises the two- and three-missed-cycle alerts", async () => {
    const dir = await mkdtemp(join(tmpdir(), "watchdog-heartbeat-"));
    try {
      const file = join(dir, "slo-journal.jsonl");
      const startedAt = 1_000_000;
      const journal = new SloJournal(file);
      await journal.append([watchdogHeartbeatLine(startedAt)]);
      const deliveries: string[] = [];
      const sink = {
        needsOrchestrator: () => { deliveries.push("needs-orchestrator"); },
        pageDrew: () => { deliveries.push("page-drew"); },
      };

      expect(await checkWatchdogHeartbeatJournal(file, startedAt + 2 * WATCHDOG_HEARTBEAT_INTERVAL_MS - 1)).toEqual({
        alert: "none",
        missedCycles: 1,
        lastHeartbeatAtMs: startedAt,
      });
      expect(await checkWatchdogHeartbeatJournal(file, startedAt + 2 * WATCHDOG_HEARTBEAT_INTERVAL_MS, sink)).toEqual({
        alert: "needs-orchestrator",
        missedCycles: 2,
        lastHeartbeatAtMs: startedAt,
      });
      expect(await checkWatchdogHeartbeatJournal(file, startedAt + 3 * WATCHDOG_HEARTBEAT_INTERVAL_MS, sink)).toEqual({
        alert: "page-drew",
        missedCycles: 3,
        lastHeartbeatAtMs: startedAt,
      });
      expect(deliveries).toEqual(["needs-orchestrator", "page-drew"]);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("heartbeat writes append without changing earlier journal history", async () => {
    const dir = await mkdtemp(join(tmpdir(), "watchdog-heartbeat-"));
    try {
      const file = join(dir, "slo-journal.jsonl");
      const journal = new SloJournal(file);
      const first = watchdogHeartbeatLine(1_000_000);
      const second = watchdogHeartbeatLine(1_000_000 + WATCHDOG_HEARTBEAT_INTERVAL_MS);
      await journal.append([first]);
      const history = await readFile(file, "utf8");
      await journal.append([second]);
      const afterSecondAppend = await readFile(file, "utf8");

      expect(afterSecondAppend.startsWith(history)).toBe(true);
      expect(afterSecondAppend.trimEnd().split("\n")).toEqual([JSON.stringify(first), JSON.stringify(second)]);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("missing or torn journal content does not fabricate a healthy heartbeat", async () => {
    const dir = await mkdtemp(join(tmpdir(), "watchdog-heartbeat-"));
    try {
      const file = join(dir, "slo-journal.jsonl");
      expect(await checkWatchdogHeartbeatJournal(file, 2_000_000)).toEqual({
        alert: "unknown",
        missedCycles: 0,
        lastHeartbeatAtMs: null,
      });
      await writeFile(file, '{"facet":"watchdog-heartbeat","atMs":1000000}\n{"facet":"watchdog-heartbeat"', "utf8");
      expect(await checkWatchdogHeartbeatJournal(file, 1_000_000 + 3 * WATCHDOG_HEARTBEAT_INTERVAL_MS)).toEqual({
        alert: "page-drew",
        missedCycles: 3,
        lastHeartbeatAtMs: 1_000_000,
      });
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});
