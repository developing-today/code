export type ReportPeriodKind = "daily" | "weekly" | "monthly" | "quarterly" | "annual";

export interface ReportPeriod {
  id: string;
  kind: ReportPeriodKind;
  timezone: "America/Chicago";
  periodStart: string;
  periodEnd: string;
  softCloseAt: string;
}

export type DailyReportPeriod = ReportPeriod & { kind: "daily" };

const TIME_ZONE = "America/Chicago" as const;
const localFormatter = new Intl.DateTimeFormat("en-US", {
  timeZone: TIME_ZONE,
  year: "numeric",
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
  hourCycle: "h23",
});

function localWallTimeToUtc(date: string, hour: number): string {
  const [year, month, day] = date.split("-").map(Number) as [number, number, number];
  const wanted = Date.UTC(year, month - 1, day, hour);
  let guess = wanted;

  // Solve local wall time = UTC instant + zone offset. Iterating also
  // handles offset changes at midnight without assuming a fixed UTC offset.
  for (let i = 0; i < 4; i++) {
    const parts = Object.fromEntries(localFormatter.formatToParts(new Date(guess)).map((p) => [p.type, p.value]));
    const represented = Date.UTC(
      Number(parts.year),
      Number(parts.month) - 1,
      Number(parts.day),
      Number(parts.hour),
      Number(parts.minute),
      Number(parts.second),
    );
    const nextGuess = wanted - (represented - guess);
    if (nextGuess === guess) break;
    guess = nextGuess;
  }
  return new Date(guess).toISOString();
}

function nextCalendarDate(date: string): string {
  const [year, month, day] = date.split("-").map(Number) as [number, number, number];
  return new Date(Date.UTC(year, month - 1, day + 1)).toISOString().slice(0, 10);
}

function calendarDate(date: string): Date {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) throw new Error("date must be a valid YYYY-MM-DD calendar date");
  const [year, month, day] = date.split("-").map(Number) as [number, number, number];
  const parsed = new Date(Date.UTC(year, month - 1, day));
  if (parsed.toISOString().slice(0, 10) !== date) throw new Error("date must be a valid YYYY-MM-DD calendar date");
  return parsed;
}

function period(kind: ReportPeriodKind, identity: string, start: string, end: string, softCloseDate: string, softCloseHour: number): ReportPeriod {
  return {
    id: `${kind}:${identity}:${TIME_ZONE}`,
    kind,
    timezone: TIME_ZONE,
    periodStart: localWallTimeToUtc(start, 0),
    periodEnd: localWallTimeToUtc(end, 0),
    softCloseAt: localWallTimeToUtc(softCloseDate, softCloseHour),
  };
}

/** Resolve a daily period from an explicit America/Chicago calendar date. */
export function dailyReportPeriod(date: string): DailyReportPeriod {
  calendarDate(date);
  const nextDate = nextCalendarDate(date);
  return period("daily", date, date, nextDate, nextDate, 8) as DailyReportPeriod;
}

/** Resolve a weekly period from its explicit Monday calendar date. */
export function weeklyReportPeriod(monday: string): ReportPeriod & { kind: "weekly" } {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(monday)) throw new Error("weekly period identity must be a Monday YYYY-MM-DD date");
  const start = calendarDate(monday);
  if (start.getUTCDay() !== 1) throw new Error("weekly period date must be a Monday");
  const end = new Date(start.getTime() + 7 * 86_400_000).toISOString().slice(0, 10);
  const close = new Date(start.getTime() + 9 * 86_400_000).toISOString().slice(0, 10);
  return period("weekly", monday, monday, end, close, 0) as ReportPeriod & { kind: "weekly" };
}

/** Resolve a monthly period from its explicit YYYY-MM identity. */
export function monthlyReportPeriod(month: string): ReportPeriod & { kind: "monthly" } {
  if (!/^\d{4}-\d{2}$/.test(month)) throw new Error("month must be a valid YYYY-MM calendar month");
  const [year, monthNumber] = month.split("-").map(Number) as [number, number];
  if (monthNumber < 1 || monthNumber > 12) throw new Error("month must be a valid YYYY-MM calendar month");
  const start = `${month}-01`;
  const end = new Date(Date.UTC(year, monthNumber, 1)).toISOString().slice(0, 10);
  const close = new Date(Date.UTC(year, monthNumber, 8)).toISOString().slice(0, 10);
  return period("monthly", month, start, end, close, 0) as ReportPeriod & { kind: "monthly" };
}

/** Resolve a quarterly period from an explicit YYYY-QN identity. */
export function quarterlyReportPeriod(quarter: string): ReportPeriod & { kind: "quarterly" } {
  const match = /^(\d{4})-Q([1-4])$/.exec(quarter);
  if (!match) throw new Error("quarter must be a valid YYYY-QN calendar quarter");
  const year = Number(match[1]);
  const firstMonth = (Number(match[2]) - 1) * 3;
  const start = new Date(Date.UTC(year, firstMonth, 1)).toISOString().slice(0, 10);
  const end = new Date(Date.UTC(year, firstMonth + 3, 1)).toISOString().slice(0, 10);
  const close = new Date(Date.UTC(year, firstMonth + 3, 8)).toISOString().slice(0, 10);
  return period("quarterly", quarter, start, end, close, 0) as ReportPeriod & { kind: "quarterly" };
}

/** Resolve an annual period from its explicit YYYY identity. */
export function annualReportPeriod(year: string): ReportPeriod & { kind: "annual" } {
  if (!/^\d{4}$/.test(year)) throw new Error("year must be a valid YYYY calendar year");
  const start = `${year}-01-01`;
  const nextYear = Number(year) + 1;
  const end = `${nextYear}-01-01`;
  const close = `${nextYear}-01-08`;
  return period("annual", year, start, end, close, 0) as ReportPeriod & { kind: "annual" };
}

/** Resolve a canonical C7 period identity; reject implicit or malformed identities. */
export function reportPeriod(kind: ReportPeriodKind, identity: string): ReportPeriod {
  switch (kind) {
    case "daily": return dailyReportPeriod(identity);
    case "weekly": return weeklyReportPeriod(identity);
    case "monthly": return monthlyReportPeriod(identity);
    case "quarterly": return quarterlyReportPeriod(identity);
    case "annual": return annualReportPeriod(identity);
  }
}
