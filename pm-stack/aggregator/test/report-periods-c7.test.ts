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

  test("generated dates preserve Chicago-midnight adjacency and next-day soft close", () => {
    // Deterministic generated input over 1900–2100; the oracle is Intl's
    // America/Chicago wall clock, independent of the conversion under test.
    const chicago = new Intl.DateTimeFormat("en-CA", {
      timeZone: "America/Chicago",
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      hourCycle: "h23",
    });
    const parts = (instant: string) => Object.fromEntries(
      chicago.formatToParts(new Date(instant)).map(({ type, value }) => [type, value]),
    );
    const minDay = Date.UTC(1900, 0, 1);
    const dayCount = (Date.UTC(2100, 0, 1) - minDay) / 86_400_000;
    let state = 0x7c7c2026;
    for (let i = 0; i < 512; i++) {
      state = (Math.imul(state, 1_664_525) + 1_013_904_223) >>> 0;
      const date = new Date(minDay + (state % dayCount) * 86_400_000).toISOString().slice(0, 10);
      const nextDate = new Date(Date.parse(`${date}T00:00:00.000Z`) + 86_400_000).toISOString().slice(0, 10);
      const period = dailyReportPeriod(date);
      const next = dailyReportPeriod(nextDate);

      expect(parts(period.periodStart)).toMatchObject({ year: date.slice(0, 4), month: date.slice(5, 7), day: date.slice(8, 10), hour: "00", minute: "00" });
      expect(parts(period.periodEnd)).toMatchObject({ year: nextDate.slice(0, 4), month: nextDate.slice(5, 7), day: nextDate.slice(8, 10), hour: "00", minute: "00" });
      expect(period.periodEnd).toBe(next.periodStart);
      expect(parts(period.softCloseAt)).toMatchObject({ year: nextDate.slice(0, 4), month: nextDate.slice(5, 7), day: nextDate.slice(8, 10), hour: "08", minute: "00" });
    }
  });

  test("rejects invalid calendar dates instead of normalizing them", () => {
    expect(() => dailyReportPeriod("2026-02-30")).toThrow(/valid YYYY-MM-DD/);
    expect(() => dailyReportPeriod("2026-2-03")).toThrow(/valid YYYY-MM-DD/);
    expect(dailyReportPeriod("2024-02-29").periodEnd).toBe("2024-03-01T06:00:00.000Z");
  });
});
