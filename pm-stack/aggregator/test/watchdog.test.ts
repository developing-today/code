import { describe, expect, test } from "bun:test";
import { buildAttention } from "../src/derive.ts";
import { attentionSections, tierItemObjectRef } from "../src/render.ts";
import type { SectionDiagnosis } from "../src/feed.ts";
import type { OrchestrationSection, TierItem } from "../src/model.ts";
import {
  watchdogTierItems,
  WATCHDOG_FEED_STALE_MS,
  WATCHDOG_REGISTRY_STALE_MS,
  type WatchdogInputs,
} from "../src/watchdog.ts";

// #1148: the staleness watchdog's derivation — current diagnoses in,
// exactly one NEEDS ORCHESTRATOR tier item per breached input out.
// Pure-function tests: no service is ever touched, every diagnosis is
// a fixture, and recovery is proven by deriving again over a healthy
// fixture and getting nothing back.

const NOW = Date.parse("2026-10-29T12:00:00Z");
const iso = (ms: number) => new Date(ms).toISOString();

function healthy(over: Partial<WatchdogInputs> = {}): WatchdogInputs {
  return {
    now: NOW,
    feedAgeMs: 4 * 60_000,
    feedGeneratedAt: iso(NOW - 4 * 60_000),
    orchestration: { state: "live", generatedAt: iso(NOW - 30 * 60_000), ageMs: 30 * 60_000, runId: "orch-1" },
    registryAgeMs: 5 * 60_000,
    probes: [
      { id: "kaneo", name: "Kaneo", ok: true, detail: "HTTP 200", probedAt: iso(NOW) },
      { id: "grist", name: "Grist", ok: true, detail: "HTTP 200", probedAt: iso(NOW) },
      { id: "pad", name: "Pad", ok: true, detail: "HTTP 200", probedAt: iso(NOW) },
    ],
    ...over,
  };
}

describe("watchdog derivation (#1148)", () => {
  test("healthy inputs produce no items", () => {
    expect(watchdogTierItems(healthy())).toEqual([]);
  });

  test("a stale feed produces exactly one item, stable across repeated derivations", () => {
    const inputs = healthy({ feedAgeMs: WATCHDOG_FEED_STALE_MS + 5 * 60_000, feedGeneratedAt: iso(NOW - (WATCHDOG_FEED_STALE_MS + 5 * 60_000)) });
    const first = watchdogTierItems(inputs);
    expect(first).toHaveLength(1);
    expect(first[0]!.id).toBe("watchdog:fleet-feed");
    expect(first[0]!.source).toBe("watchdog");
    expect(first[0]!.title).toContain("fleet feed is stale");
    const second = watchdogTierItems(inputs);
    expect(second.map((i) => i.id)).toEqual(first.map((i) => i.id));
    expect(second[0]!.since).toBe(first[0]!.since);
  });

  test("an unreadable feed produces the absent item", () => {
    const items = watchdogTierItems(healthy({ feedAgeMs: null, feedGeneratedAt: null }));
    expect(items.map((i) => i.id)).toEqual(["watchdog:fleet-feed"]);
    expect(items[0]!.state).toContain("absent");
  });

  test("a failed Kaneo probe produces exactly one item; other services and ok/null probes produce none", () => {
    const items = watchdogTierItems(
      healthy({
        probes: [
          { id: "kaneo", name: "Kaneo", ok: false, detail: "probe failed: connection refused", probedAt: iso(NOW - 30_000) },
          { id: "grist", name: "Grist", ok: true, detail: "HTTP 200", probedAt: iso(NOW) },
          { id: "pad", name: "Pad", ok: null, detail: "off by config", probedAt: iso(NOW) },
          { id: "ready-for-agent", name: "ready-for-agent", ok: false, detail: "HTTP 500", probedAt: iso(NOW) },
        ],
      }),
    );
    expect(items.map((i) => i.id)).toEqual(["watchdog:probe-kaneo"]);
    expect(items[0]!.title).toContain("Kaneo");
  });

  test("orchestration stale / invalid / absent each produce exactly one item; live produces none", () => {
    const stale: SectionDiagnosis = { state: "stale", generatedAt: iso(NOW - 120 * 60_000), ageMs: 120 * 60_000, runId: "orch-0" };
    expect(watchdogTierItems(healthy({ orchestration: stale })).map((i) => i.id)).toEqual(["watchdog:orchestration"]);
    const invalid: SectionDiagnosis = { state: "invalid", failedChecks: ["tiers"] };
    const inv = watchdogTierItems(healthy({ orchestration: invalid }));
    expect(inv.map((i) => i.id)).toEqual(["watchdog:orchestration"]);
    expect(inv[0]!.reason).toContain("tiers");
    const absent: SectionDiagnosis = { state: "absent" };
    expect(watchdogTierItems(healthy({ orchestration: absent })).map((i) => i.id)).toEqual(["watchdog:orchestration"]);
  });

  test("registry facet stale past 1h produces its own item, distinct from the feed item", () => {
    const onlyRegistry = watchdogTierItems(healthy({ registryAgeMs: WATCHDOG_REGISTRY_STALE_MS + 60_000 }));
    expect(onlyRegistry.map((i) => i.id)).toEqual(["watchdog:registry"]);
    const both = watchdogTierItems(
      healthy({
        feedAgeMs: WATCHDOG_REGISTRY_STALE_MS + 60_000,
        feedGeneratedAt: iso(NOW - (WATCHDOG_REGISTRY_STALE_MS + 60_000)),
        registryAgeMs: WATCHDOG_REGISTRY_STALE_MS + 60_000,
      }),
    );
    expect(both.map((i) => i.id)).toEqual(["watchdog:fleet-feed", "watchdog:registry"]);
  });

  test("recovery clears the item: a breach followed by a healthy derivation yields none", () => {
    const breached = watchdogTierItems(healthy({ feedAgeMs: 60 * 60_000, feedGeneratedAt: iso(NOW - 60 * 60_000) }));
    expect(breached.length).toBeGreaterThan(0);
    expect(watchdogTierItems(healthy())).toEqual([]);
  });

  test("a future-dated feed produces exactly one fleet-feed item despite the clamped zero age (Codex P2, PR #2431)", () => {
    // buildHealth clamps a future generatedAt's age to 0 and diagnoses
    // the failure itself; the watchdog must honor that diagnosis, not
    // wait for the future timestamp + 15m.
    const items = watchdogTierItems(healthy({ feedAgeMs: 0, feedGeneratedAt: iso(NOW + 24 * 60 * 60_000) }));
    expect(items.map((i) => i.id)).toEqual(["watchdog:fleet-feed"]);
    expect(items[0]!.state).toContain("future-dated");
  });

  test("each watchdog item keys an independent object ref, not the shared issue ref (Codex P1, PR #2431)", () => {
    const items = watchdogTierItems(
      healthy({
        feedAgeMs: WATCHDOG_FEED_STALE_MS + 60_000,
        feedGeneratedAt: iso(NOW - (WATCHDOG_FEED_STALE_MS + 60_000)),
        probes: [{ id: "kaneo", name: "Kaneo", ok: false, detail: "HTTP 503", probedAt: iso(NOW) }],
      }),
    );
    expect(items.length).toBeGreaterThanOrEqual(2);
    const refs = items.map((i) => tierItemObjectRef(i));
    expect(new Set(refs).size).toBe(refs.length);
    for (const r of refs) expect(r).not.toContain("1148");
  });

  test("watchdog items merge into buildAttention's orchestrator extras and never the drew tier", () => {
    const items = watchdogTierItems(healthy({ feedAgeMs: 60 * 60_000, feedGeneratedAt: iso(NOW - 60 * 60_000) }));
    const view = buildAttention([], null, NOW, undefined, items);
    expect(view.drewExtra).toEqual([]);
    expect(view.orchestratorExtra.map((i) => i.id)).toEqual(items.map((i) => i.id));
    // The JSON surfaces publish the derived items from view.watchdog
    // (Codex P2, PR #2431): it carries exactly the merged watchdog set.
    expect(view.watchdog.map((i) => i.id)).toEqual(items.map((i) => i.id));
    // Merging is idempotent by id: passing the same items twice over
    // (two derivations concatenated) still yields one card per input.
    const view2 = buildAttention([], null, NOW, undefined, [...items, ...items].filter((v, idx, a) => a.indexOf(v) === idx || true));
    expect(new Set(view2.orchestratorExtra.map((i) => i.id)).size).toBe(view2.orchestratorExtra.length);
  });

  test("watchdog cards stay actionable while the tier source is absent; published cards stay gated (Codex P2, PR #2431)", () => {
    const published: TierItem = {
      id: "pub-1",
      kind: "task",
      title: "Published orchestrator item XYZ",
      source: "tracked",
      reason: "Published by the orchestrator.",
      since: iso(NOW - 60_000),
      lastSeenAt: iso(NOW - 60_000),
    };
    const section: OrchestrationSection = {
      version: 1,
      generatedAt: iso(NOW),
      runId: "orch-test",
      provenance: { sources: ["fixture"] },
      tiers: { needsDrew: [], needsOrchestrator: [published] },
      dependencies: [],
      goals: [],
      actions: [],
    };
    const watchdog = watchdogTierItems(healthy({ orchestration: { state: "absent" } }));
    expect(watchdog.map((i) => i.id)).toEqual(["watchdog:orchestration"]);
    const view = buildAttention([], section, NOW, { state: "absent" }, watchdog);
    const { queues } = attentionSections(view, [], undefined, { identities: [], actionsByObject: new Map() });
    const pubAt = queues.indexOf("Published orchestrator item XYZ");
    const wdAt = queues.indexOf("Watchdog: orchestration section is absent");
    expect(pubAt).toBeGreaterThanOrEqual(0);
    expect(wdAt).toBeGreaterThan(pubAt);
    // The published card sits behind the section gate (its controls are
    // in a disabled fieldset); the watchdog card — a live aggregator
    // observation — does not.
    expect(queues.slice(pubAt, wdAt)).toContain("<fieldset disabled>");
    expect(queues.slice(wdAt)).not.toContain("<fieldset disabled>");
  });
});
