const DEFAULT_RETENTION_SECONDS = 604800;
const MAX_ALARM_TIMESTAMP = 8_640_000_000_000_000;

/** RETENTION_SECONDS is positive whole decimal seconds; invalid or unsupported values use seven days. */
export function retentionMilliseconds(value: string | undefined, now: number): number {
  const defaultMilliseconds = DEFAULT_RETENTION_SECONDS * 1000;
  if (value === undefined || !/^\d+$/.test(value)) return defaultMilliseconds;

  const seconds = Number(value);
  const milliseconds = seconds * 1000;
  const alarmTimestamp = now + milliseconds;
  if (
    !Number.isSafeInteger(seconds) ||
    seconds <= 0 ||
    !Number.isSafeInteger(milliseconds) ||
    !Number.isSafeInteger(alarmTimestamp) ||
    alarmTimestamp > MAX_ALARM_TIMESTAMP
  ) {
    return defaultMilliseconds;
  }
  return milliseconds;
}
