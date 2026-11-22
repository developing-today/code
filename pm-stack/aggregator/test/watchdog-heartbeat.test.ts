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

  test("generated histories select the newest valid beat and obey threshold oracle", async () => {
    const dir = await mkdtemp(join(tmpdir(), "watchdog-heartbeat-property-"));
    try {
      const file = join(dir, "slo-journal.jsonl");
      // Deterministic PRNG keeps counterexamples reproducible while
      // generating varied orderings, malformed lines, and unrelated records.
      let seed = 0x6d2b79f5;
      const random = (): number => {
        seed = Math.imul(seed ^ (seed >>> 15), seed | 1);
        seed ^= seed + Math.imul(seed ^ (seed >>> 7), seed | 61);
        return ((seed ^ (seed >>> 14)) >>> 0) / 0x1_0000_0000;
      };
      for (let example = 0; example < 500; example++) {
        const beats = Array.from({ length: 1 + Math.floor(random() * 20) }, () => Math.floor(random() * 10_000_000));
        const shuffled = [...beats];
        for (let i = shuffled.length - 1; i > 0; i--) {
          const j = Math.floor(random() * (i + 1));
          [shuffled[i], shuffled[j]] = [shuffled[j]!, shuffled[i]!];
        }
        const lines = shuffled.map((atMs) => JSON.stringify({ facet: "watchdog-heartbeat", atMs }));
        lines.splice(Math.floor(random() * (lines.length + 1)), 0, "not-json", JSON.stringify({ facet: "other", atMs: 99_000_000 }));
        await writeFile(file, `${lines.join("\n")}\n`, "utf8");

        const newest = Math.max(...beats);
        const missed = Math.floor(random() * 5);
        const now = newest + missed * WATCHDOG_HEARTBEAT_INTERVAL_MS;
        const expectedAlert = missed >= 3 ? "page-drew" : missed >= 2 ? "needs-orchestrator" : "none";
        expect(await checkWatchdogHeartbeatJournal(file, now)).toEqual({
          alert: expectedAlert,
          missedCycles: missed,
          lastHeartbeatAtMs: newest,
        });
      }
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});
