// Bitwarden SM collector (#1111): the count-only read. Fixtures carry
// sentinel names/keys/values so the tests can prove none of them ever
// reach a count, a formatted bullet, an updated tracker view, or an
// error — only array lengths cross the boundary.
import { describe, expect, test } from "bun:test";
import {
  BwsCountError,
  applyBitwardenRead,
  collectBitwardenCounts,
  countBwsListOutput,
  countsFromBwsOutputs,
  formatBitwardenUsageValue,
} from "../src/bitwarden-counts.ts";
import { extractSaasServices } from "../src/saas-extract.ts";
import { recordsFromSaasServices } from "../src/snapshot-records.ts";

const SENTINELS = ["SENTINEL-secret-name", "SENTINEL-key", "SENTINEL-value", "SENTINEL-project-name"];

const SECRET_LIST_JSON = JSON.stringify([
  { id: "00000000-0000-0000-0000-000000000001", key: "SENTINEL-key", name: "SENTINEL-secret-name", value: "SENTINEL-value" },
  { id: "00000000-0000-0000-0000-000000000002", key: "SENTINEL-key", name: "SENTINEL-secret-name", value: "SENTINEL-value" },
  { id: "00000000-0000-0000-0000-000000000003", key: "SENTINEL-key", name: "SENTINEL-secret-name", value: "SENTINEL-value" },
]);
const PROJECT_LIST_JSON = JSON.stringify([{ id: "00000000-0000-0000-0000-00000000000p", name: "SENTINEL-project-name" }]);

const VIEW = `# Part (b) — Subscriptions & SaaS/infrastructure

### Bitwarden
- **Plan / price / allowance:** Secrets Manager; plan/price unknown.
- **Usage / dates:** Count not recorded yet.
- **PAYG estimate:** not applicable — flat account-tooling plan.
- **Last checked:** tracker.

### Vercel
- **Usage / dates:** unknown.
`;

describe("bitwarden count-only collector", () => {
  test("counts are array lengths only", () => {
    expect(countBwsListOutput(SECRET_LIST_JSON, "secret")).toBe(3);
    expect(countBwsListOutput(PROJECT_LIST_JSON, "project")).toBe(1);
    expect(countsFromBwsOutputs(SECRET_LIST_JSON, PROJECT_LIST_JSON)).toEqual({ secrets: 3, projects: 1 });
    expect(countBwsListOutput("[]", "secret")).toBe(0);
  });

  test("malformed or non-array bws output fails closed, echoing nothing", () => {
    for (const bad of ["not json", "{}", '"SENTINEL-value"', "42"]) {
      let err: unknown = null;
      try {
        countBwsListOutput(bad, "secret");
      } catch (e) {
        err = e;
      }
      expect(err).toBeInstanceOf(BwsCountError);
      for (const s of SENTINELS) expect(String((err as Error).message)).not.toContain(s);
    }
  });

  test("collectBitwardenCounts drives the runner with the two list commands", () => {
    const calls: string[][] = [];
    const counts = collectBitwardenCounts((args) => {
      calls.push(args);
      return args[0] === "secret" ? SECRET_LIST_JSON : PROJECT_LIST_JSON;
    });
    expect(calls).toEqual([
      ["secret", "list"],
      ["project", "list"],
    ]);
    expect(counts).toEqual({ secrets: 3, projects: 1 });
  });

  test("a failing runner is a collector error, never a partial count", () => {
    expect(() =>
      collectBitwardenCounts(() => {
        throw new Error("spawn failed");
      }),
    ).toThrow(BwsCountError);
  });

  test("formatted read matches the extractor's phrasing, singular and plural", () => {
    expect(formatBitwardenUsageValue({ secrets: 3, projects: 1 }, "2026-11-24")).toBe(
      "3 secrets, 1 project (bws count-only read 2026-11-24)",
    );
    expect(formatBitwardenUsageValue({ secrets: 1, projects: 2 }, "2026-11-24")).toBe(
      "1 secret, 2 projects (bws count-only read 2026-11-24)",
    );
  });

  test("an impossible read date is rejected", () => {
    expect(() => formatBitwardenUsageValue({ secrets: 1, projects: 1 }, "2026-02-30")).toThrow(BwsCountError);
    expect(() => applyBitwardenRead(VIEW, { secrets: 1, projects: 1 }, "24 Nov 2026")).toThrow(BwsCountError);
  });

  test("applyBitwardenRead records the read and the hub extracts + snapshots it", () => {
    const updated = applyBitwardenRead(VIEW, { secrets: 3, projects: 1 }, "2026-11-24");
    expect(updated).toContain("- **Usage / dates:** 3 secrets, 1 project (bws count-only read 2026-11-24).");
    expect(updated).toContain("- **Last checked:** bws count-only read 2026-11-24.");
    // The neighbouring section is untouched.
    expect(updated).toContain("### Vercel\n- **Usage / dates:** unknown.");

    const bw = new Map(extractSaasServices(updated).map((s) => [s.id, s])).get("bitwarden")!;
    const m = new Map(bw.metrics.map((x) => [x.id, x]));
    expect(m.get("secrets-count")?.used).toBe(3);
    expect(m.get("projects-count")?.used).toBe(1);
    expect(bw.measuredAt).toBe("2026-11-24");

    const records = recordsFromSaasServices([bw], "test");
    expect(records.map((r) => [r.serviceId, r.metricId, r.value])).toEqual([
      ["bitwarden", "secrets-count", 3],
      ["bitwarden", "projects-count", 1],
    ]);
  });

  test("no fixture name, key, or value reaches the updated view or the bullets", () => {
    const counts = countsFromBwsOutputs(SECRET_LIST_JSON, PROJECT_LIST_JSON);
    const updated = applyBitwardenRead(VIEW, counts, "2026-11-24");
    const formatted = formatBitwardenUsageValue(counts, "2026-11-24");
    for (const s of SENTINELS) {
      expect(updated).not.toContain(s);
      expect(formatted).not.toContain(s);
    }
  });

  test("applyBitwardenRead fails closed without a section or bullets", () => {
    expect(() => applyBitwardenRead("# no sections\n", { secrets: 1, projects: 1 }, "2026-11-24")).toThrow(
      BwsCountError,
    );
    const noBullets = "### Bitwarden\n- **Plan / price / allowance:** Secrets Manager.\n";
    expect(() => applyBitwardenRead(noBullets, { secrets: 1, projects: 1 }, "2026-11-24")).toThrow(BwsCountError);
  });
});
