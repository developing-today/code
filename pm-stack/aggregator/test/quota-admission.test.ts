import { describe, expect, test } from "bun:test";
import { buildQuotaAdmission, type AdmissionInput, type Observation } from "../src/quota-admission.ts";

const now = 10_000_000;
const scope = { provider: "go", account: "a", bucket: "plan" };
const read = (patch: Partial<Observation> = {}): Observation => ({
  ...scope,
  kind: "monthly",
  period: "p1",
  id: "new",
  source: "fixture",
  measuredAt: now,
  resetsAt: now + 86_400_000,
  percent: 50,
  state: "ok",
  denominator: "100",
  ...patch,
});
function fixture(): AdmissionInput {
  return {
    generation: "fixture-v1",
    bindings: [
      {
        ...scope,
        aliases: ["cred-a", "cred-b"],
        windows: [
          {
            kind: "monthly",
            period: "p1",
            resetsAt: now + 86_400_000,
            freshnessMs: 1_200_000,
            collectionMs: 900_000,
          },
        ],
      },
    ],
    observations: [read()],
    stops: [],
    priorWalls: [],
  };
}
const account = (f: AdmissionInput) => buildQuotaAdmission(f, now).accounts[0];
const verdict = (f: AdmissionInput) => account(f).verdict;

describe("quota admission: oracle = #10323 decision table and exact-scope invariants", () => {
  test("pure, deterministic, versioned; aliases share one allowance", () => {
    const f = fixture();
    const before = JSON.stringify(f);
    expect(verdict(f)).toBe("eligible");
    expect(account(f).aliases).toEqual(["cred-a", "cred-b"]);
    expect(buildQuotaAdmission(f, now).version).toBe(1);
    expect(JSON.stringify(f)).toBe(before);
  });
  test("independent sibling never supplies missing account evidence", () => {
    const f = fixture();
    f.observations = [read({ account: "other" })];
    expect(verdict(f)).toBe("unknown");
    f.bindings = [...f.bindings, { ...f.bindings[0], account: "other", aliases: ["cred-c"] }];
    expect(buildQuotaAdmission(f, now).accounts.map((a) => a.verdict)).toEqual(["unknown", "eligible"]);
  });
  test("ambiguous aliases, duplicate bindings and absent required windows fail closed", () => {
    for (const change of [
      (f: AdmissionInput) => {
        f.bindings[0].windows = [];
      },
      (f: AdmissionInput) => {
        f.bindings = [...f.bindings, { ...f.bindings[0] }];
      },
      (f: AdmissionInput) => {
        f.bindings = [...f.bindings, { ...f.bindings[0], account: "other" }];
      },
    ]) {
      const f = fixture();
      change(f);
      expect(verdict(f)).toBe("unknown");
    }
  });
  test("short window defeats monthly headroom; missing short window is unknown", () => {
    const f = fixture();
    f.bindings[0].windows = [...f.bindings[0].windows, { ...f.bindings[0].windows[0], kind: "weekly" }];
    expect(verdict(f)).toBe("unknown");
    f.observations = [...f.observations, read({ id: "weekly", kind: "weekly", percent: 100 })];
    expect(verdict(f)).toBe("walled");
  });
  test("window freshness is independent; no future tolerance or retained freshness", () => {
    for (const patch of [
      { measuredAt: now - 1_200_001 },
      { measuredAt: now + 1 },
      { measuredAt: NaN },
      { retained: true },
      { state: "error" as const },
    ]) {
      const f = fixture();
      f.observations = [read(patch)];
      expect(verdict(f)).toBe("unknown");
    }
    const f = fixture();
    f.observations = [read({ measuredAt: now - 1_200_000 })];
    expect(verdict(f)).toBe("eligible");
  });
  test("60-minute ceiling and malformed percent never clamp to a wall", () => {
    for (const percent of [-1, 101, NaN, Infinity]) {
      const f = fixture();
      f.observations = [read({ percent })];
      expect(verdict(f)).toBe("unknown");
    }
    const f = fixture();
    f.bindings[0].windows[0].freshnessMs = 86_400_000;
    f.observations = [read({ measuredAt: now - 3_600_001 })];
    expect(verdict(f)).toBe("unknown");
  });
  test("boundary corpus: reserve policy is opt-in, not a wall", () => {
    for (const [percent, expected] of [
      [96.999, "eligible"],
      [97, "pace_hold"],
      [99.499, "pace_hold"],
      [99.5, "pace_hold"],
      [100, "walled"],
    ] as const) {
      const f = fixture();
      f.bindings[0].windows[0].reservePct = 97;
      f.observations = [read({ percent })];
      expect(verdict(f)).toBe(expected);
    }
    const f = fixture();
    f.observations = [read({ percent: 99.5 })];
    expect(verdict(f)).toBe("eligible");
  });
  test("sustained wall requires distinct, adjacent, separated provider observations", () => {
    const f = fixture();
    const high = read({ percent: 99.5 });
    f.observations = [high, { ...high }];
    expect(verdict(f)).toBe("eligible");
    f.observations = [high, read({ id: "old", percent: 99.5, measuredAt: now - 900_000 })];
    expect(verdict(f)).toBe("walled");
    f.observations = [...f.observations, read({ id: "middle", percent: 60, measuredAt: now - 1 })];
    expect(verdict(f)).toBe("eligible");
  });
  test("corrupt IDs, simultaneous contradictions and reset mismatches are unknown", () => {
    for (const other of [
      read({ percent: 70 }),
      read({ id: "same-time", percent: 70 }),
      read({ id: "old", measuredAt: now - 1, resetsAt: now + 1 }),
    ]) {
      const f = fixture();
      f.observations = [read(), other];
      expect(verdict(f)).toBe("unknown");
    }
  });
  test("prior same-period wall survives missing reads; reset alone never reopens", () => {
    const f = fixture();
    f.observations = [];
    f.priorWalls = [{ ...scope, kind: "monthly", period: "p1", resetsAt: now + 86_400_000, evidenceId: "wall" }];
    expect(verdict(f)).toBe("walled");
    f.observations = [read({ percent: 1 })];
    expect(verdict(f)).toBe("walled"); // fresh headroom is not verified recovery
    expect(buildQuotaAdmission(f, now + 86_400_000).accounts[0].verdict).toBe("unknown");
    f.bindings[0].windows[0].period = "p2";
    expect(verdict(f)).toBe("unknown");
  });
  test("pace needs continuity and predicted exhaustion, not just a fast rate", () => {
    const f = fixture();
    f.bindings[0].windows[0].pacePpHour = 1;
    f.observations = [read({ percent: 90 }), read({ id: "old", percent: 88, measuredAt: now - 3_600_000 })];
    expect(verdict(f)).toBe("pace_hold");
    expect(account(f).windows[0].ratePpHour).toBe(2);
    for (const patch of [{ denominator: "200" }, { percent: 95 }, { period: "p0" }]) {
      f.observations = [read({ percent: 90 }), read({ id: "old", percent: 88, measuredAt: now - 3_600_000, ...patch })];
      expect(account(f).windows[0].pace).toBe("unknown");
      expect(verdict(f)).toBe("eligible");
    }
    f.observations = [read({ percent: 50 }), read({ id: "old", percent: 48, measuredAt: now - 3_600_000 })];
    expect(verdict(f)).toBe("eligible"); // wall would be 25h away, reset is 24h
  });
  test("stop precedence and explicit release; corrupt release cannot clear stop", () => {
    const f = fixture();
    f.stops = [{ ...scope, id: "stop", reason: "operator_hold", effectiveAt: now - 10 }];
    expect(verdict(f)).toBe("stopped");
    f.stops[0].releasedAt = NaN;
    expect(verdict(f)).toBe("stopped");
    f.stops[0].releasedAt = now;
    expect(verdict(f)).toBe("eligible");
    f.stops = [{ ...scope, id: "future", reason: "operator_hold", effectiveAt: now + 10 }];
    expect(verdict(f)).toBe("eligible");
  });
  test("metamorphic oracle: history/alias ordering and duplicate reads cannot change verdict", () => {
    for (let percent = 0; percent <= 100; percent++) {
      const f = fixture();
      f.observations = [
        read({ percent }),
        read({ id: "old", percent: Math.max(0, percent - 1), measuredAt: now - 900_000 }),
      ];
      const a = buildQuotaAdmission(f, now);
      f.observations = [...f.observations].reverse().concat(f.observations[0]);
      f.bindings[0].aliases = [...f.bindings[0].aliases].reverse();
      expect(buildQuotaAdmission(f, now)).toEqual(a);
    }
  });
  test("same account alias spans buckets: seat intersects every bucket", () => {
    const f = fixture();
    f.bindings = [...f.bindings, { ...f.bindings[0], bucket: "shared-weekly" }];
    f.observations = [...f.observations, read({ bucket: "shared-weekly", percent: 100 })];
    const result = buildQuotaAdmission(f, now);
    expect(result.accounts.map((a) => a.verdict)).toEqual(["eligible", "walled"]);
    expect(result.seats.every((s) => s.account === "a" && s.verdict === "walled")).toBe(true);
    expect(result.seats[0].buckets).toEqual(["plan", "shared-weekly"]);
    f.observations = [read()];
    expect(buildQuotaAdmission(f, now).seats[0].verdict).toBe("unknown");
    f.bindings = [...f.bindings, { ...f.bindings[0], account: "other" }];
    expect(buildQuotaAdmission(f, now).seats[0].account).toBeNull();
    expect(buildQuotaAdmission(f, now).seats[0].verdict).toBe("unknown");
  });
  test("slightly falling high readings sustain a wall but never infer a negative pace", () => {
    const f = fixture();
    f.observations = [read({ percent: 99.5 }), read({ id: "old", percent: 99.6, measuredAt: now - 900_000 })];
    expect(verdict(f)).toBe("walled");
    expect(account(f).windows[0].ratePpHour).toBeNull();
  });
  test("contradictory previous timestamp invalidates sustained and pace evidence", () => {
    for (const patch of [{ percent: 70 }, { state: "error" as const }, { retained: true }]) {
      const f = fixture();
      f.observations = [
        read({ percent: 99.5 }),
        read({ id: "old-a", percent: 99.5, measuredAt: now - 900_000 }),
        read({ id: "old-b", percent: 99.5, measuredAt: now - 900_000, ...patch }),
      ];
      expect(verdict(f)).toBe("eligible");
      expect(account(f).windows[0].pace).toBe("unknown");
      const expected = buildQuotaAdmission(f, now);
      f.observations = [...f.observations].reverse();
      expect(buildQuotaAdmission(f, now)).toEqual(expected);
    }
  });
  test("projection exposes measured evidence and active stop references", () => {
    const f = fixture();
    f.stops = [{ ...scope, id: "stop", reason: "operator_hold", effectiveAt: now - 10 }];
    expect(account(f).windows[0]).toMatchObject({ source: "fixture", percent: 50, measuredAt: now });
    expect(account(f).activeStops).toEqual([{ id: "stop", reason: "operator_hold", effectiveAt: now - 10 }]);
  });
});
