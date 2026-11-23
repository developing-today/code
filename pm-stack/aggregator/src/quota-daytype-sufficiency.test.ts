// Tests for the C4.3 day-type sample-sufficiency re-measure
// (#13259). Every test names its oracle: the C4 design §5 gate
// (≥8 independent samples per cell; below it the state is
// adjustment_unavailable), the Stage-3 design §2 arithmetic it
// re-measures (33 days of history ⇒ ~4–5 samples of each weekday
// per window, too thin), the design §2 rename join as carried by
// the S2 harness, and the America/Chicago calendar the design §4
// renders in.
import { describe, expect, test } from "bun:test";
import {
  DAY_TYPE_SAMPLE_GATE,
  dayTypeParts,
  measureDayTypeSufficiency,
  type DayTypeCellMeasurement,
} from "./quota-daytype-sufficiency.ts";
import type { SnapshotRecord } from "./model.ts";

function rec(serviceId: string, metricId: string, isoTs: string, value: number = 50): SnapshotRecord {
  return {
    ts: isoTs,
    serviceId,
    metricId,
    value,
    unit: "pct",
    limit: 100,
    resetsAt: null,
    source: "api",
    observedBy: "test",
  };
}

/** 15:00Z is 09:00–10:00 in Chicago (CST/CDT) — a morning reading. */
function morningOn(date: string): string {
  return `${date}T15:00:00.000Z`;
}

function cell(
  m: ReturnType<typeof measureDayTypeSufficiency>,
  pool: string,
  window: string,
  grain: string,
  name: string,
): DayTypeCellMeasurement {
  const pw = m.poolWindows.find((p) => p.pool === pool && p.window === window);
  expect(pw).toBeDefined();
  const c = pw?.cells.find((x) => x.grain === grain && x.cell === name);
  expect(c).toBeDefined();
  return c as DayTypeCellMeasurement;
}

describe("the gate (oracle: C4 design §5)", () => {
  test("the gate is 8 independent samples", () => {
    expect(DAY_TYPE_SAMPLE_GATE).toBe(8);
  });

  test("a cell with exactly 8 independent samples is admitted; 7 is not", () => {
    // Eight Fridays: 2026-10-30, 11-06, 11-13, 11-20, 11-27,
    // 12-04, 12-11, 12-18 (weekly cadence from Friday 2026-10-30).
    const fridays = [
      "2026-10-30",
      "2026-11-06",
      "2026-11-13",
      "2026-11-20",
      "2026-11-27",
      "2026-12-04",
      "2026-12-11",
      "2026-12-18",
    ];
    const eight = measureDayTypeSufficiency(fridays.map((d) => rec("meta", "5h", morningOn(d))));
    expect(cell(eight, "meta", "5h", "weekday", "Fri").samples).toBe(8);
    expect(cell(eight, "meta", "5h", "weekday", "Fri").state).toBe("admitted");
    const seven = measureDayTypeSufficiency(
      fridays.slice(0, 7).map((d) => rec("meta", "5h", morningOn(d))),
    );
    expect(cell(seven, "meta", "5h", "weekday", "Fri").samples).toBe(7);
    expect(cell(seven, "meta", "5h", "weekday", "Fri").state).toBe("adjustment_unavailable");
  });

  test("readings are not samples: two readings on one date are one independent sample", () => {
    const m = measureDayTypeSufficiency([
      rec("meta", "5h", "2026-11-06T15:00:00.000Z"),
      rec("meta", "5h", "2026-11-06T16:00:00.000Z"),
    ]);
    const fri = cell(m, "meta", "5h", "weekday", "Fri");
    expect(fri.samples).toBe(1);
    expect(fri.readings).toBe(2);
  });
});

describe("the Stage-3 deferral, reproduced (oracle: Stage-3 design §2)", () => {
  test("33 days of dense history gives ~4–5 samples per weekday and admits no weekday cell", () => {
    // 2026-10-26 (Mon) .. 2026-11-27 (Fri) inclusive is 33 days —
    // the span Stage-3 §2 measured. Four readings a day stands in
    // for the gatherer's dense sampling; samples count dates. The
    // hours are 06:00–15:00 Chicago, so every reading falls on its
    // UTC date in the Chicago calendar too.
    const records: SnapshotRecord[] = [];
    const start = Date.parse("2026-10-26T00:00:00.000Z");
    for (let d = 0; d < 33; d++) {
      for (const h of [12, 15, 18, 21]) {
        records.push(rec("meta", "5h", new Date(start + d * 86_400_000 + h * 3_600_000).toISOString()));
      }
    }
    const m = measureDayTypeSufficiency(records);
    for (const wd of ["Mon", "Tue", "Wed", "Thu", "Fri"]) {
      expect(cell(m, "meta", "5h", "weekday", wd).samples).toBe(5);
    }
    for (const wd of ["Sat", "Sun"]) {
      expect(cell(m, "meta", "5h", "weekday", wd).samples).toBe(4);
    }
    const weekdayCells = m.poolWindows[0]?.cells.filter((c) => c.grain === "weekday") ?? [];
    expect(weekdayCells.every((c) => c.state === "adjustment_unavailable")).toBe(true);
    // The coarser grains the design names do cross on the same
    // history: weekends pool Sat+Sun dates, dayparts pool all days.
    expect(cell(m, "meta", "5h", "dayclass", "weekend").samples).toBe(8);
    expect(cell(m, "meta", "5h", "dayclass", "weekend").state).toBe("admitted");
    expect(cell(m, "meta", "5h", "daypart", "morning").samples).toBe(33);
    expect(cell(m, "meta", "5h", "daypart", "morning").state).toBe("admitted");
    // The finest grain stays under: no weekday-daypart cell can
    // hold more samples than its weekday.
    const fine = m.poolWindows[0]?.cells.filter((c) => c.grain === "weekday-daypart") ?? [];
    expect(fine.every((c) => c.state === "adjustment_unavailable")).toBe(true);
  });
});

describe("the calendar (oracle: C4 design §4 — America/Chicago wall clock)", () => {
  test("a reading at 05:00Z in November is the previous Chicago evening", () => {
    // 2026-11-23T05:00Z is Sunday 2026-11-22 23:00 CST — a Sunday
    // evening sample, not a Monday night one.
    const parts = dayTypeParts(Date.parse("2026-11-23T05:00:00.000Z"));
    expect(parts).toEqual({ date: "2026-11-22", weekday: "Sun", daypart: "evening" });
    const m = measureDayTypeSufficiency([rec("deepseek", "5h", "2026-11-23T05:00:00.000Z")]);
    expect(cell(m, "deepseek", "5h", "weekday", "Sun").samples).toBe(1);
    expect(cell(m, "deepseek", "5h", "weekday", "Mon").samples).toBe(0);
    expect(cell(m, "deepseek", "5h", "weekday-daypart", "Sun evening").samples).toBe(1);
  });

  test("dayparts are quarter-day blocks: night 00–06, morning 06–12, afternoon 12–18, evening 18–24 Chicago", () => {
    // Chicago is UTC-6 on 2026-11-22 (CST): local hour = UTC hour - 6.
    const at = (utcHour: number) => dayTypeParts(Date.parse(`2026-11-22T${String(utcHour).padStart(2, "0")}:00:00.000Z`)).daypart;
    expect(at(6)).toBe("night"); // 00:00 local
    expect(at(11)).toBe("night"); // 05:00 local
    expect(at(12)).toBe("morning"); // 06:00 local
    expect(at(17)).toBe("morning"); // 11:00 local
    expect(at(18)).toBe("afternoon"); // 12:00 local
    expect(at(23)).toBe("afternoon"); // 17:00 local
    expect(at(0)).toBe("evening"); // 18:00 local, previous UTC day
    expect(at(5)).toBe("evening"); // 23:00 local
  });
});

describe("the join (oracle: Stage-3 design §2 + the S2 harness's own rules)", () => {
  test("renamed service ids fold into one pool row", () => {
    const m = measureDayTypeSufficiency([
      rec("antigravity", "5h", morningOn("2026-11-06")),
      rec("_antigravity", "5h", morningOn("2026-11-13")),
      rec("openai.alternate", "5h", morningOn("2026-11-06")),
      rec("openai", "5h", morningOn("2026-11-13")),
    ]);
    // The rename join (S2's SERVICE_ID_RENAME_ALIASES): the
    // underscore spelling folds into antigravity; the alternate
    // account's spelling joins its own pool (openai.alt), and the
    // bare spelling joins the main account's pool (openai.main).
    const pools = m.poolWindows.map((p) => p.pool).sort();
    expect(pools).toEqual(["antigravity", "openai.alt", "openai.main"]);
    expect(cell(m, "antigravity", "5h", "weekday", "Fri").samples).toBe(2);
    expect(cell(m, "openai.alt", "5h", "weekday", "Fri").samples).toBe(1);
    expect(cell(m, "openai.main", "5h", "weekday", "Fri").samples).toBe(1);
    const ag = m.poolWindows.find((p) => p.pool === "antigravity");
    expect(ag?.serviceIds).toEqual(["_antigravity", "antigravity"]);
  });

  test("window spellings fold: 7d and weekly are one window; a ':5h' suffix counts for 5h", () => {
    const m = measureDayTypeSufficiency([
      rec("meta", "7d", morningOn("2026-11-06")),
      rec("meta", "weekly", morningOn("2026-11-13")),
      rec("meta", "door:5h", morningOn("2026-11-06")),
    ]);
    expect(cell(m, "meta", "weekly", "weekday", "Fri").samples).toBe(2);
    expect(cell(m, "meta", "5h", "weekday", "Fri").samples).toBe(1);
  });

  test("non-percent records and unparseable timestamps are not measured", () => {
    const estimate: SnapshotRecord = {
      ...rec("meta", "5h", morningOn("2026-11-06")),
      value: { low: 1, high: 2 },
      unit: "usd-est",
    };
    const m = measureDayTypeSufficiency([
      estimate,
      { ...rec("meta", "5h", morningOn("2026-11-06")), ts: "not-a-date" },
      rec("meta", "5h", morningOn("2026-11-06")),
    ]);
    expect(m.store.pctRecords).toBe(1);
    expect(cell(m, "meta", "5h", "weekday", "Fri").samples).toBe(1);
  });

  test("as-of excludes later readings, so a run over a fixed store copy is reproducible", () => {
    const records = [
      rec("meta", "5h", morningOn("2026-11-06")),
      rec("meta", "5h", morningOn("2026-11-13")),
    ];
    const m = measureDayTypeSufficiency(records, { asOfMs: Date.parse("2026-11-10T00:00:00.000Z") });
    expect(cell(m, "meta", "5h", "weekday", "Fri").samples).toBe(1);
    expect(m.store.toIso).toBe("2026-11-06T15:00:00.000Z");
  });
});
