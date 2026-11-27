// PM-core schedule registry coverage (#16056): Surface 6 of
// doc/fleet-schedule-registry.md must list both in-process PM-core
// schedules — the quota refresh (15m) and the fleet-feed collection
// (5m) — with their startup-refresh behavior and source pointers, and
// the legacy systemd timer rows must stay until live disablement is
// verified. These tests pin the registry against the code constants so
// a future cadence change forces a same-pass registry update.
//
// Run: bun test test/pm-core-schedule-registry-16056.test.ts
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { FLEET_FEED_CADENCE_MS, QUOTA_CADENCE_MS } from "../src/pm-core-scheduler.ts";

const ROOT = join(import.meta.dir, "..", "..", "..");
const REGISTRY = readFileSync(join(ROOT, "doc", "fleet-schedule-registry.md"), "utf8");
const WIRING = readFileSync(join(ROOT, "pm-stack", "aggregator", "src", "pm-core-wiring.ts"), "utf8");
const SERVER = readFileSync(join(ROOT, "pm-stack", "aggregator", "src", "server.ts"), "utf8");

function surface6(): string {
  const start = REGISTRY.indexOf("## Surface 6");
  const end = REGISTRY.indexOf("## Surface 7");
  expect(start).toBeGreaterThanOrEqual(0);
  expect(end).toBeGreaterThan(start);
  return REGISTRY.slice(start, end);
}

function rowFor(text: string, name: string): string {
  const row = text.split("\n").find((line) => line.includes(name));
  expect(row, `Surface 6 row for ${name}`).toBeDefined();
  return row as string;
}

describe("PM-core schedule registry coverage (#16056)", () => {
  test("code cadences are the documented 15m quota / 5m fleet-feed", () => {
    expect(QUOTA_CADENCE_MS).toBe(15 * 60_000);
    expect(FLEET_FEED_CADENCE_MS).toBe(5 * 60_000);
  });

  test("Surface 6 lists the PM-core quota refresh with cadence, startup, and source pointers", () => {
    const row = rowFor(surface6(), "PM-core quota refresh");
    expect(row).toContain("15 minute");
    expect(row).toContain("immediate refresh");
    expect(row).toContain("QUOTA_CADENCE_MS");
    expect(row).toContain("pm-core-wiring.ts");
    expect(row).toContain("live aggregator process only");
  });

  test("Surface 6 lists the PM-core fleet-feed collection with cadence, startup, and source pointers", () => {
    const row = rowFor(surface6(), "PM-core fleet-feed collection");
    expect(row).toContain("5 minute");
    expect(row).toContain("immediate refresh");
    expect(row).toContain("FLEET_FEED_CADENCE_MS");
    expect(row).toContain("pm-core-wiring.ts");
    expect(row).toContain("live aggregator process only");
  });

  test("Surface 6 records coexistence with the legacy timers pending live-disablement verification", () => {
    const section = surface6();
    expect(section).toContain("quota-gatherer.timer");
    expect(section).toContain("fleet-feed-collector.timer");
    expect(section).toContain("retirement");
    const legacyQuota = rowFor(REGISTRY, "quota-gatherer.timer");
    expect(legacyQuota).toContain("retirement pending");
    const legacyFeed = rowFor(REGISTRY, "fleet-feed-collector.timer");
    expect(legacyFeed).toContain("retirement pending");
  });

  test("the wiring still refreshes both jobs immediately at startup", () => {
    expect(WIRING).toContain('runNow("quota")');
    expect(WIRING).toContain('runNow("fleet-feed")');
    expect(SERVER).toContain("startPmCoreSchedulers");
  });
});
