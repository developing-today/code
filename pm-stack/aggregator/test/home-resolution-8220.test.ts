// Probe matrix for the #5762 home-resolution contract on the aggregator's
// config (#8220): HOME unset and HOME empty are each HomeUnresolvedError naming
// the setting that needed HOME; HOME set leaves every derived path unchanged;
// an explicit override never needs HOME.
import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { HomeUnresolvedError, requireHome } from "../src/home.ts";

// [config key, override variable, path under HOME] for every top-level
// HOME-derived default; notifyRelay.stateFile and the fleet ledger are below.
const HOME_DEFAULTS: [string, string, string][] = [
  ["codeCheckout", "PM_AGG_CODE_CHECKOUT", "git/code"],
  ["nixCheckout", "PM_AGG_NIX_CHECKOUT", "code/dezren39-nix"],
  ["identityRegistryFile", "PM_AGG_IDENTITY_REGISTRY_FILE", ".local/state/pm-aggregator/identity-registry.json"],
  ["breakGlassFile", "PM_AGG_BREAK_GLASS_FILE", ".local/state/pm-aggregator/break-glass-tokens.json"],
  ["outageQueueFile", "PM_AGG_OUTAGE_QUEUE_FILE", ".local/state/pm-aggregator/outage-queue.json"],
  ["freshnessFile", "PM_AGG_FRESHNESS_FILE", ".local/state/pm-aggregator/freshness.json"],
  ["queueBrokerTokenFile", "PM_AGG_QUEUE_BROKER_TOKEN_FILE", ".config/queue-broker/token"],
  ["activityFile", "PM_AGG_ACTIVITY_FILE", ".local/state/pm-aggregator/activity-ingestion.json"],
  ["outcomeFile", "PM_AGG_OUTCOME_FILE", ".local/state/pm-aggregator/outcome-facts.jsonl"],
  ["outcomeBindingsFile", "PM_AGG_OUTCOME_BINDINGS_FILE", ".local/state/pm-aggregator/outcome-bindings.jsonl"],
  ["notificationsFile", "PM_AGG_NOTIFICATIONS_FILE", ".local/state/pm-aggregator/notifications.jsonl"],
  ["needsDrewFile", "PM_AGG_NEEDS_DREW_FILE", ".local/state/pm-aggregator/needs-drew.json"],
  ["decisionsFile", "PM_AGG_DECISIONS_FILE", ".local/state/pm-aggregator/decisions.json"],
  ["decisionsEventsFile", "PM_AGG_DECISION_EVENTS_FILE", ".local/state/pm-aggregator/decision-events.jsonl"],
  ["decisionsOutboxFile", "PM_AGG_DECISION_OUTBOX_FILE", ".local/state/pm-aggregator/decision-events-outbox.jsonl"],
  ["stackHealthFile", "PM_AGG_STACK_HEALTH_FILE", ".local/state/pm-aggregator/stack-health.json"],
  ["hostHealthFile", "PM_AGG_HOST_HEALTH_FILE", ".local/state/pm-aggregator/host-health.json"],
  ["lastGoodDir", "PM_AGG_LAST_GOOD_DIR", ".local/state/pm-aggregator/last-good-pages"],
  [
    "quotaViewFile",
    "PM_AGG_QUOTA_VIEW",
    "workspace/goals/stewardship-commission-recurring-review-and-dispatch/hidden_files/visibility/quota-view.md",
  ],
  [
    "freeServicesTrackerFile",
    "PM_AGG_FREE_SERVICES_TRACKER",
    "hatch-personal/workspace/ai-subscriptions/free-services-to-check.md",
  ],
  ["cloudflareExportFile", "PM_AGG_CLOUDFLARE_EXPORT", "atlas/cloudflare-export/latest.json"],
  [
    "quotaRegistryFile",
    "PM_AGG_QUOTA_REGISTRY",
    "workspace/goals/keep-the-four-repos-healthy/hidden_files/provider-quota-registry.md",
  ],
  ["opencodeConfigFile", "PM_AGG_OPENCODE_CONFIG_FILE", "git/code/.opencode/opencode.jsonc"],
  ["itemHistoryFile", "PM_AGG_ITEM_HISTORY_FILE", ".local/state/pm-aggregator/item-history.jsonl"],
  ["sloJournalFile", "PM_AGG_SLO_JOURNAL_FILE", ".local/state/pm-aggregator/slo-journal.jsonl"],
  ["openchamberStateFile", "PM_AGG_OPENCHAMBER_STATE", ".config/openchamber/managed-opencode/state.json"],
  ["opencodeServerStateFile", "PM_AGG_OPENCODE_SERVER_STATE", ".local/state/opencode/service-prod.json"],
  ["mcpxCallLedgerDir", "PM_AGG_MCPX_CALL_LEDGER_DIR", ".local/state/mcpx/call-ledger"],
];
const NOTIFY_RELAY = ["PM_AGG_NOTIFY_RELAY_STATE_FILE", ".local/state/pm-aggregator/notify-relay.json"] as const;
const FLEET_LEDGER = "PM_AGG_FLEET_INVOCATION_LEDGER_FILE";
const ALL_OVERRIDES = [...HOME_DEFAULTS.map(([, name]) => name), NOTIFY_RELAY[0], FLEET_LEDGER];

let savedHome: string | undefined;
beforeEach(() => {
  savedHome = process.env.HOME;
});
afterEach(() => {
  if (savedHome === undefined) delete process.env.HOME;
  else process.env.HOME = savedHome;
});

function setHome(value: string | undefined): void {
  if (value === undefined) delete process.env.HOME;
  else process.env.HOME = value;
}

function overrides(): Record<string, string> {
  return Object.fromEntries(ALL_OVERRIDES.map((name) => [name, `/override/${name}`]));
}

describe("requireHome (#8220)", () => {
  test("HOME unset is a named error carrying the setting", () => {
    let caught: unknown;
    try {
      requireHome("PM_AGG_FRESHNESS_FILE", {});
    } catch (error) {
      caught = error;
    }
    expect(caught).toBeInstanceOf(HomeUnresolvedError);
    expect((caught as HomeUnresolvedError).name).toBe("HomeUnresolvedError");
    expect((caught as HomeUnresolvedError).setting).toBe("PM_AGG_FRESHNESS_FILE");
    expect((caught as Error).message).toContain("HOME is unset");
  });

  test("HOME empty or blank is a named error", () => {
    expect(() => requireHome("X", { HOME: "" })).toThrow(HomeUnresolvedError);
    expect(() => requireHome("X", { HOME: "   " })).toThrow("HOME is empty");
  });

  test("HOME set is returned unchanged", () => {
    expect(requireHome("X", { HOME: "/home/probe" })).toBe("/home/probe");
  });
});

describe("loadConfig HOME-derived defaults (#8220)", () => {
  test("HOME set: every default path is the same as before", () => {
    setHome("/home/probe");
    const cfg = loadConfig({ PM_AGG_DEV: "1" }) as unknown as Record<string, unknown>;
    for (const [key, , path] of HOME_DEFAULTS) {
      expect([key, cfg[key]]).toEqual([key, join("/home/probe", path)]);
    }
    expect(loadConfig({ PM_AGG_DEV: "1" }).notifyRelay.stateFile).toBe(join("/home/probe", NOTIFY_RELAY[1]));
    expect(loadConfig({ PM_AGG_DEV: "1" }).fleetInvocationLedgerFile).toBe(
      "/home/probe/.local/state/fleet/invocations.jsonl",
    );
  });

  for (const [label, value] of [
    ["unset", undefined],
    ["empty", ""],
  ] as const) {
    test(`HOME ${label}: a needed default is HomeUnresolvedError naming its setting`, () => {
      setHome(value);
      for (const [, name] of HOME_DEFAULTS) {
        const env: Record<string, string> = { ...overrides(), PM_AGG_DEV: "1" };
        delete env[name];
        let caught: unknown;
        try {
          loadConfig(env);
        } catch (error) {
          caught = error;
        }
        expect(caught).toBeInstanceOf(HomeUnresolvedError);
        expect((caught as HomeUnresolvedError).setting).toBe(name);
      }
      const env: Record<string, string> = { ...overrides(), PM_AGG_DEV: "1" };
      delete env[NOTIFY_RELAY[0]];
      expect(() => loadConfig(env)).toThrow(HomeUnresolvedError);
    });

    test(`HOME ${label}: explicit overrides never need HOME`, () => {
      setHome(value);
      const cfg = loadConfig({ ...overrides(), PM_AGG_DEV: "1" }) as unknown as Record<string, unknown>;
      for (const [key, name] of HOME_DEFAULTS) expect(cfg[key]).toBe(`/override/${name}`);
    });

    test(`HOME ${label}: the fleet ledger default uses XDG_STATE_HOME when set, else is a named error`, () => {
      setHome(value);
      const env: Record<string, string> = { ...overrides(), PM_AGG_DEV: "1" };
      delete env[FLEET_LEDGER];
      expect(() => loadConfig(env)).toThrow(HomeUnresolvedError);
      expect(loadConfig({ ...env, XDG_STATE_HOME: "/xdg/state" }).fleetInvocationLedgerFile).toBe(
        "/xdg/state/fleet/invocations.jsonl",
      );
    });
  }
});
