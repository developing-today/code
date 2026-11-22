import { describe, expect, test } from "bun:test";
import { dailyReportPeriod } from "../src/report-periods.ts";

describe("C7 daily report periods", () => {
  test("uses explicit Chicago calendar dates and a stable period identity", () => {
    const period = dailyReportPeriod("2026-11-21");
    expect(period).toEqual({
      id: "daily:2026-11-21:America/Chicago",
      kind: "daily",
      timezone: "America/Chicago",
      periodStart: "2026-11-21T06:00:00.000Z",
      periodEnd: "2026-11-22T06:00:00.000Z",
      softCloseAt: "2026-11-22T14:00:00.000Z",
    });
    expect(dailyReportPeriod("2026-11-21").id).toBe(period.id);
  });

  test("uses calendar boundaries across daylight-saving transitions", () => {
    const spring = dailyReportPeriod("2026-03-08");
    expect(spring.periodStart).toBe("2026-03-08T06:00:00.000Z");
    expect(spring.periodEnd).toBe("2026-03-09T05:00:00.000Z");
    expect(spring.softCloseAt).toBe("2026-03-09T13:00:00.000Z");

    const fall = dailyReportPeriod("2026-11-01");
    expect(fall.periodStart).toBe("2026-11-01T05:00:00.000Z");
    expect(fall.periodEnd).toBe("2026-11-02T06:00:00.000Z");
    expect(fall.softCloseAt).toBe("2026-11-02T14:00:00.000Z");
  });

  test("rejects invalid calendar dates instead of normalizing them", () => {
    expect(() => dailyReportPeriod("2026-02-30")).toThrow(/valid YYYY-MM-DD/);
    expect(() => dailyReportPeriod("2026-2-03")).toThrow(/valid YYYY-MM-DD/);
    expect(dailyReportPeriod("2024-02-29").periodEnd).toBe("2024-03-01T06:00:00.000Z");
  });
});
