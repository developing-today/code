// Probe matrix for the #5762 home-resolution contract on the aggregator's
// module-level defaults (#8220 slice 2), alongside home-resolution-8220.test.ts
// for config.ts: HOME unset and HOME empty are each HomeUnresolvedError naming
// the setting; HOME set gives the same path as before; an override never
// needs HOME.
import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { join } from "node:path";
import { opencodeDbDefault } from "../src/analytics-store.ts";
import { defaultDecisionEventsFile, defaultDecisionOutboxFile } from "../src/decision-events.ts";
import { defaultDecisionsFile } from "../src/decisions.ts";
import { HomeUnresolvedError } from "../src/home.ts";
import { defaultHostHealthFile } from "../src/host-health.ts";
import { defaultLatestPath } from "../src/meta-renewal.ts";
import { defaultNeedsDrewFile } from "../src/needs-drew.ts";
import { defaultReportRecordsFile } from "../src/report-records.ts";
import { defaultStackHealthFile } from "../src/stack-health.ts";

const STATE = ".local/state/pm-aggregator";

// [setting the error names, override variable or null, default under HOME, resolver]
const DEFAULTS: [string, string | null, string, (env: Record<string, string | undefined>) => string][] = [
  ["PM_AGG_DECISION_EVENTS_FILE", "PM_AGG_DECISION_EVENTS_FILE", `${STATE}/decision-events.jsonl`, (env) =>
    defaultDecisionEventsFile(env)],
  ["PM_AGG_DECISION_OUTBOX_FILE", "PM_AGG_DECISION_OUTBOX_FILE", `${STATE}/decision-events-outbox.jsonl`, (env) =>
    defaultDecisionOutboxFile(env)],
  ["PM_AGG_DECISIONS_FILE", "PM_AGG_DECISIONS_FILE", `${STATE}/decisions.json`, (env) => defaultDecisionsFile(env)],
  ["PM_AGG_HOST_HEALTH_FILE", "PM_AGG_HOST_HEALTH_FILE", `${STATE}/host-health.json`, (env) =>
    defaultHostHealthFile(env)],
  ["PM_AGG_NEEDS_DREW_FILE", "PM_AGG_NEEDS_DREW_FILE", `${STATE}/needs-drew.json`, (env) => defaultNeedsDrewFile(env)],
  ["PM_AGG_REPORT_RECORDS_FILE", "PM_AGG_REPORT_RECORDS_FILE", `${STATE}/report-records.jsonl`, (env) =>
    defaultReportRecordsFile(env)],
  ["PM_AGG_STACK_HEALTH_FILE", "PM_AGG_STACK_HEALTH_FILE", `${STATE}/stack-health.json`, (env) =>
    defaultStackHealthFile(env)],
  ["the OpenCode database path", null, ".local/share/opencode/opencode.db", () => opencodeDbDefault()],
  ["the meta quota export's latest.json", null, "atlas/meta-quota-export/latest.json", () => defaultLatestPath()],
];

let savedHome: string | undefined;
beforeEach(() => {
  savedHome = process.env.HOME;
});
afterEach(() => {
  if (savedHome === undefined) delete process.env.HOME;
  else process.env.HOME = savedHome;
});

describe("aggregator module defaults under the home-resolution contract (#8220)", () => {
  test("HOME set: every default path is the same as before", () => {
    process.env.HOME = "/home/probe";
    for (const [setting, , path, resolve] of DEFAULTS) {
      expect([setting, resolve({})]).toEqual([setting, join("/home/probe", path)]);
    }
  });

  for (const [label, value] of [
    ["unset", undefined],
    ["empty", ""],
  ] as const) {
    test(`HOME ${label}: each default is HomeUnresolvedError naming its setting`, () => {
      if (value === undefined) delete process.env.HOME;
      else process.env.HOME = value;
      for (const [setting, , , resolve] of DEFAULTS) {
        let caught: unknown;
        try {
          resolve({});
        } catch (error) {
          caught = error;
        }
        expect([setting, caught instanceof HomeUnresolvedError]).toEqual([setting, true]);
        expect((caught as HomeUnresolvedError).setting).toBe(setting);
      }
    });

    test(`HOME ${label}: an override never needs HOME`, () => {
      if (value === undefined) delete process.env.HOME;
      else process.env.HOME = value;
      for (const [setting, override, , resolve] of DEFAULTS) {
        if (override === null) continue;
        expect([setting, resolve({ [override]: "/override/path" })]).toEqual([setting, "/override/path"]);
      }
    });
  }

  test("a described setting's message says to set HOME, an override variable's names it too", () => {
    delete process.env.HOME;
    expect(() => opencodeDbDefault()).toThrow("set HOME (home-resolution");
    expect(() => defaultDecisionsFile({})).toThrow("set HOME or set PM_AGG_DECISIONS_FILE");
  });
});
