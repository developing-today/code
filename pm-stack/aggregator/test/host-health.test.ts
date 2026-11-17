import { describe, expect, test } from "bun:test";
import {
  normalizeHostHealthSnapshot,
  HOST_HEALTH_BACKUP_STALE_MS,
  HOST_HEALTH_STALE_MS,
  type HostHealthSnapshot,
} from "../src/host-health.ts";
import { watchdogTierItems, type WatchdogInputs } from "../src/watchdog.ts";
import type { ProjectAudit } from "../src/model.ts";

// #10141: silent-failure surfacing. The host-health collector writes a
// snapshot; the watchdog derives exactly one NEEDS ORCHESTRATOR item
// per breached host fact, and recovery (a healthy next snapshot)
// clears them. Pure fixtures only — no service is ever touched.

const NOW = Date.parse("2026-11-14T18:00:00Z");
const iso = (ms: number) => new Date(ms).toISOString();

const freshProjectAudit: ProjectAudit = {
  title: "Developing Today",
  number: 4,
  url: "https://github.com/orgs/developing-today/projects/4",
  generatedAt: iso(NOW),
  totalItems: 1,
  fields: [],
  views: [],
  counts: { status: { Todo: 1 }, area: { "(unset)": 1 }, repo: { "developing-today/code": 1 } },
};

function healthyUnit(over: Record<string, unknown> = {}) {
  return {
    activeState: "inactive",
    subState: "dead",
    result: "success",
    execMainStatus: 0,
    stateChangedAt: iso(NOW - 60_000),
    lastSuccessAt: iso(NOW - 60_000),
    ...over,
  };
}

function healthySnapshot(over: Partial<HostHealthSnapshot> = {}): HostHealthSnapshot {
  return {
    generatedAt: iso(NOW - 60_000),
    claimantMode: { state: "live", detail: "mode file content: live" },
    units: {
      "pm-git-sync": healthyUnit(),
      "pm-projector-reconcile": healthyUnit(),
      "pm-kaneo-sync-guard": healthyUnit(),
      "pm-dashboard-backup": healthyUnit({ lastSuccessAt: iso(NOW - 6 * 3600_000) }),
    },
    timers: {
      "pm-git-sync.timer": { active: true, activeState: "active" },
      "pm-projector-reconcile.timer": { active: true, activeState: "active" },
      "pm-kaneo-sync-guard.timer": { active: true, activeState: "active" },
      "pm-dashboard-backup.timer": { active: true, activeState: "active" },
    },
    ...over,
  };
}

function inputs(hostHealth: HostHealthSnapshot | null | undefined): WatchdogInputs {
  return {
    now: NOW,
    feedAgeMs: 60_000,
    feedGeneratedAt: iso(NOW - 60_000),
    orchestration: { state: "live", generatedAt: iso(NOW - 30 * 60_000), ageMs: 30 * 60_000, runId: "orch-1" },
    registryAgeMs: 5 * 60_000,
    projectAudit: freshProjectAudit,
    probes: [],
    ...(hostHealth !== undefined ? { hostHealth } : {}),
  };
}

describe("host-health watchdog derivation (#10141)", () => {
  test("no hostHealth input (fixtures mode) produces no host items", () => {
    expect(watchdogTierItems(inputs(undefined))).toEqual([]);
  });

  test("a healthy snapshot produces no items", () => {
    expect(watchdogTierItems(inputs(healthySnapshot()))).toEqual([]);
  });

  test("an absent snapshot is itself one visible item", () => {
    const items = watchdogTierItems(inputs(null));
    expect(items.map((i) => i.id)).toEqual(["watchdog:host-health"]);
    expect(items[0]!.state).toContain("absent");
    expect(items[0]!.ref).toBe("developing-today/code#10141");
  });

  test("a stale snapshot breaches, and its unit facts still derive", () => {
    const snap = healthySnapshot({ generatedAt: iso(NOW - HOST_HEALTH_STALE_MS - 60_000) });
    const items = watchdogTierItems(inputs(snap));
    expect(items.map((i) => i.id)).toEqual(["watchdog:host-health"]);
    expect(items[0]!.state).toContain("stale");
  });

  test("a missing claimant mode file produces exactly one item; shadow-by-file and live do not", () => {
    const missing = healthySnapshot({
      claimantMode: { state: "missing", detail: "mode file does not exist" },
    });
    const items = watchdogTierItems(inputs(missing));
    expect(items.map((i) => i.id)).toEqual(["watchdog:claimant-mode"]);
    expect(items[0]!.title).toContain("missing");
    const shadow = healthySnapshot({ claimantMode: { state: "shadow" } });
    expect(watchdogTierItems(inputs(shadow))).toEqual([]);
  });

  test("a failed pm-git-sync produces one item and recovery clears it", () => {
    const snap = healthySnapshot();
    snap.units["pm-git-sync"] = healthyUnit({
      activeState: "failed",
      result: "exit-code",
      execMainStatus: 1,
      lastSuccessAt: null,
    });
    const items = watchdogTierItems(inputs(snap));
    expect(items.map((i) => i.id)).toEqual(["watchdog:unit-pm-git-sync"]);
    expect(items[0]!.title).toContain("pm-git-sync is failing");
    expect(watchdogTierItems(inputs(healthySnapshot()))).toEqual([]);
  });

  test("pm-projector-reconcile stuck in activating past the bound breaches; a fresh activation does not", () => {
    const stuck = healthySnapshot();
    stuck.units["pm-projector-reconcile"] = healthyUnit({
      activeState: "activating",
      result: null,
      stateChangedAt: iso(NOW - 11 * 60_000),
      lastSuccessAt: iso(NOW - 15 * 60_000),
    });
    const items = watchdogTierItems(inputs(stuck));
    expect(items.map((i) => i.id)).toEqual(["watchdog:unit-pm-projector-reconcile"]);
    expect(items[0]!.state).toContain("stuck");
    const fresh = healthySnapshot();
    fresh.units["pm-projector-reconcile"] = healthyUnit({
      activeState: "activating",
      result: null,
      stateChangedAt: iso(NOW - 30_000),
      lastSuccessAt: iso(NOW - 15 * 60_000),
    });
    expect(watchdogTierItems(inputs(fresh))).toEqual([]);
  });

  test("a kaneo-sync-guard failure (drift exit) breaches", () => {
    const snap = healthySnapshot();
    snap.units["pm-kaneo-sync-guard"] = healthyUnit({
      activeState: "failed",
      result: "exit-code",
      execMainStatus: 1,
      lastSuccessAt: null,
    });
    const items = watchdogTierItems(inputs(snap));
    expect(items.map((i) => i.id)).toEqual(["watchdog:unit-pm-kaneo-sync-guard"]);
  });

  test("backup staleness past 36h breaches even when the unit last exited cleanly long ago", () => {
    const snap = healthySnapshot();
    snap.units["pm-dashboard-backup"] = healthyUnit({
      lastSuccessAt: iso(NOW - HOST_HEALTH_BACKUP_STALE_MS - 3600_000),
    });
    const items = watchdogTierItems(inputs(snap));
    expect(items.map((i) => i.id)).toEqual(["watchdog:unit-pm-dashboard-backup"]);
    expect(items[0]!.state).toContain("stale");
  });

  test("an inactive unit timer breaches", () => {
    const snap = healthySnapshot();
    snap.timers["pm-git-sync.timer"] = { active: false, activeState: "inactive" };
    const items = watchdogTierItems(inputs(snap));
    expect(items.map((i) => i.id)).toEqual(["watchdog:unit-pm-git-sync"]);
    expect(items[0]!.state).toContain("timer inactive");
  });
});

describe("host-health snapshot normalization (#10141)", () => {
  test("a well-formed collector snapshot round-trips", () => {
    const snap = normalizeHostHealthSnapshot(JSON.parse(JSON.stringify(healthySnapshot())));
    expect(snap).not.toBeNull();
    expect(snap!.claimantMode.state).toBe("live");
    expect(snap!.units["pm-git-sync"]!.result).toBe("success");
  });

  test("absent, corrupt, or shapeless input reads as null, never as health", () => {
    expect(normalizeHostHealthSnapshot(null)).toBeNull();
    expect(normalizeHostHealthSnapshot({})).toBeNull();
    expect(
      normalizeHostHealthSnapshot({ generatedAt: "not-a-date", claimantMode: { state: "live" }, units: {} }),
    ).toBeNull();
    expect(
      normalizeHostHealthSnapshot({ generatedAt: iso(NOW), claimantMode: { state: "banana" }, units: {} }),
    ).toBeNull();
  });
});
