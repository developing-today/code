export interface DailyReportPeriod {
  id: string;
  kind: "daily";
  timezone: "America/Chicago";
  periodStart: string;
  periodEnd: string;
  softCloseAt: string;
}

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

/** Resolve a daily period from an explicit America/Chicago calendar date. */
export function dailyReportPeriod(date: string): DailyReportPeriod {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) throw new Error("date must be a valid YYYY-MM-DD calendar date");
  const [year, month, day] = date.split("-").map(Number) as [number, number, number];
  const check = new Date(Date.UTC(year, month - 1, day));
  if (check.toISOString().slice(0, 10) !== date) throw new Error("date must be a valid YYYY-MM-DD calendar date");

  const nextDate = nextCalendarDate(date);
  return {
    id: `daily:${date}:${TIME_ZONE}`,
    kind: "daily",
    timezone: TIME_ZONE,
    periodStart: localWallTimeToUtc(date, 0),
    periodEnd: localWallTimeToUtc(nextDate, 0),
    softCloseAt: localWallTimeToUtc(nextDate, 8),
  };
}
