//! The §5.4 time grammar: RFC3339 timestamps and calendar durations.
//!
//! Rules that matter (spec §5.4):
//! - Lifetimes are **Gregorian calendar arithmetic** from the start
//!   date — `20y` is twenty calendar years, never a fixed count of
//!   seconds. Leap-day behaviour is explicit: Feb 29 + 1y lands on
//!   Feb 28 of the following year (clamp, never overflow into March).
//! - Exact instants serialize as precise RFC3339 UTC
//!   (`YYYY-MM-DDTHH:MM:SSZ`).
//! - `--not-after` (exact) and `--valid-for` (duration) are mutually
//!   exclusive; the resolved pair is always reported back as explicit
//!   RFC3339 timestamps in manifests.

use crate::error::{CertError, Result};
use serde::{Deserialize, Serialize};
use std::fmt;
use time::format_description::well_known::Rfc3339 as Rfc3339Format;
use time::{Date, Duration as TimeDuration, Month, OffsetDateTime, Time};

/// An instant, serialized canonically as RFC3339 UTC seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Rfc3339(pub OffsetDateTime);

impl Rfc3339 {
    pub fn now() -> Self {
        Rfc3339(OffsetDateTime::now_utc())
    }

    pub fn parse(input: &str) -> Result<Self> {
        OffsetDateTime::parse(input.trim(), &Rfc3339Format)
            .map(Rfc3339)
            .map_err(|e| CertError::usage(format!("invalid RFC3339 timestamp '{input}': {e}")))
    }

    pub fn to_string_canonical(&self) -> String {
        // OffsetDateTime::format with the well-known Rfc3339 description
        // emits e.g. 2026-11-23T14:00:00Z for UTC values.
        self.0
            .format(&Rfc3339Format)
            .unwrap_or_else(|_| "invalid".to_string())
    }
}

impl fmt::Display for Rfc3339 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string_canonical())
    }
}

impl Serialize for Rfc3339 {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string_canonical())
    }
}

impl<'de> Deserialize<'de> for Rfc3339 {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        Rfc3339::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// A calendar-aware duration: year/month parts are Gregorian calendar
/// arithmetic; the rest are exact clock quantities (spec §5.4).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarDuration {
    pub years: i64,
    pub months: i64,
    pub weeks: i64,
    pub days: i64,
    pub hours: i64,
    pub minutes: i64,
    pub seconds: i64,
}

impl CalendarDuration {
    pub const fn years(n: i64) -> Self {
        CalendarDuration {
            years: n,
            months: 0,
            weeks: 0,
            days: 0,
            hours: 0,
            minutes: 0,
            seconds: 0,
        }
    }
    pub const fn days(n: i64) -> Self {
        CalendarDuration {
            years: 0,
            months: 0,
            weeks: 0,
            days: n,
            hours: 0,
            minutes: 0,
            seconds: 0,
        }
    }

    pub fn is_zero(&self) -> bool {
        *self == CalendarDuration::default()
    }

    /// Parse the duration grammar: a sequence of `<number><unit>`
    /// terms, separated by spaces or commas — `20y`, `1y`,
    /// `15 years 7 days`, `90d`, `15m`, `15 minutes`, `2w`, `12h`.
    /// A bare number or an unknown unit is an error, never a guess
    /// (spec §5.4: never use an ambiguous conversion silently).
    /// `m` means minutes (spec §24.1(2)'s "15 minutes" short-lived
    /// identities); months are `mo`/`month`/`months`.
    pub fn parse(input: &str) -> Result<Self> {
        let text = input.trim().to_ascii_lowercase();
        if text.is_empty() {
            return Err(CertError::usage("empty duration"));
        }
        let mut out = CalendarDuration::default();
        let mut rest = text.as_str();
        let mut terms = 0usize;
        while !rest.is_empty() {
            rest = rest.trim_start_matches([' ', ',', '+']);
            if rest.is_empty() {
                break;
            }
            let num_end = rest.find(|c: char| !c.is_ascii_digit()).ok_or_else(|| {
                CertError::usage(format!(
                    "duration '{input}': bare number without a unit is ambiguous; \
                         add a unit (y, mo, w, d, h, m, s)"
                ))
            })?;
            if num_end == 0 {
                return Err(CertError::usage(format!(
                    "duration '{input}': expected a number at '{rest}'"
                )));
            }
            let n: i64 = rest[..num_end].parse().map_err(|_| {
                CertError::usage(format!("duration '{input}': bad number in '{rest}'"))
            })?;
            rest = rest[num_end..].trim_start();
            let unit_end = rest
                .find(|c: char| !c.is_ascii_alphabetic())
                .unwrap_or(rest.len());
            if unit_end == 0 {
                return Err(CertError::usage(format!(
                    "duration '{input}': missing unit after {n}"
                )));
            }
            let unit = &rest[..unit_end];
            rest = &rest[unit_end..];
            match unit {
                "y" | "yr" | "yrs" | "year" | "years" => out.years += n,
                "mo" | "month" | "months" => out.months += n,
                "w" | "week" | "weeks" => out.weeks += n,
                "d" | "day" | "days" => out.days += n,
                "h" | "hr" | "hour" | "hours" => out.hours += n,
                "m" | "min" | "mins" | "minute" | "minutes" => out.minutes += n,
                "s" | "sec" | "secs" | "second" | "seconds" => out.seconds += n,
                other => {
                    return Err(CertError::usage(format!(
                        "duration '{input}': unknown unit '{other}' \
                         (known: y, mo, w, d, h, m, s — spelled-out forms accepted)"
                    )));
                }
            }
            terms += 1;
        }
        if terms == 0 || out.is_zero() {
            return Err(CertError::usage(format!(
                "duration '{input}': no positive terms"
            )));
        }
        if out.years < 0
            || out.months < 0
            || out.weeks < 0
            || out.days < 0
            || out.hours < 0
            || out.minutes < 0
            || out.seconds < 0
        {
            return Err(CertError::usage(format!(
                "duration '{input}': negative terms are not allowed"
            )));
        }
        Ok(out)
    }

    /// Add this duration to an instant using Gregorian calendar
    /// arithmetic for the year/month parts (clamping the day of month
    /// when the target month is shorter — Feb 29 + 1y = Feb 28), then
    /// exact arithmetic for weeks/days and clock parts. The
    /// time-of-day is preserved across the calendar parts.
    pub fn add_to(&self, start: Rfc3339) -> Result<Rfc3339> {
        let dt = start.0;
        let date = dt.date();
        let time_of_day: Time = dt.time();

        // Calendar years, then months, with day clamping.
        let mut year = date.year() as i64 + self.years;
        let mut month_idx = (date.month() as i64 - 1) + self.months;
        year += month_idx.div_euclid(12);
        month_idx = month_idx.rem_euclid(12);
        let month_num = (month_idx + 1) as u8;
        let month = Month::try_from(month_num)
            .map_err(|_| CertError::engine("month arithmetic overflow"))?;
        let year_i32 =
            i32::try_from(year).map_err(|_| CertError::engine("year arithmetic overflow"))?;
        let max_day = days_in_month(year_i32, month);
        let day = date.day().min(max_day);
        let new_date = Date::from_calendar_date(year_i32, month, day)
            .map_err(|e| CertError::engine(format!("date arithmetic failed: {e}")))?;
        let mut out = OffsetDateTime::new_utc(new_date, time_of_day);

        let exact = TimeDuration::weeks(self.weeks)
            + TimeDuration::days(self.days)
            + TimeDuration::hours(self.hours)
            + TimeDuration::minutes(self.minutes)
            + TimeDuration::seconds(self.seconds);
        out += exact;
        Ok(Rfc3339(out))
    }
}

fn days_in_month(year: i32, month: Month) -> u8 {
    match month {
        Month::January
        | Month::March
        | Month::May
        | Month::July
        | Month::August
        | Month::October
        | Month::December => 31,
        Month::April | Month::June | Month::September | Month::November => 30,
        Month::February => {
            if time::util::is_leap_year(year) {
                29
            } else {
                28
            }
        }
    }
}

/// Default lifetimes (spec §5.4 / §24.1(2)).
pub const ROOT_DEFAULT_LIFETIME_YEARS: i64 = 20;
pub const ISSUER_DEFAULT_LIFETIME_YEARS: i64 = 5;
pub const LEAF_DEFAULT_LIFETIME_DAYS: i64 = 90;

/// Resolve a validity window from the flag grammar: `--not-before`
/// (default: now), then exactly one of `--not-after` (exact) or
/// `--valid-for` (duration from not-before), else the caller's
/// default lifetime. Both given is a usage error (spec §6.3 mutual
/// exclusion).
pub fn resolve_validity(
    not_before: Option<Rfc3339>,
    valid_for: Option<CalendarDuration>,
    not_after: Option<Rfc3339>,
    default_lifetime: CalendarDuration,
) -> Result<(Rfc3339, Rfc3339)> {
    if valid_for.is_some() && not_after.is_some() {
        return Err(CertError::usage(
            "--valid-for and --not-after are mutually exclusive (spec §6.3)",
        ));
    }
    let nb = not_before.unwrap_or_else(Rfc3339::now);
    let na = match (valid_for, not_after) {
        (_, Some(exact)) => exact,
        (Some(dur), None) => dur.add_to(nb)?,
        (None, None) => default_lifetime.add_to(nb)?,
    };
    if na <= nb {
        return Err(CertError::usage(format!(
            "not-after ({na}) must be strictly after not-before ({nb})"
        )));
    }
    Ok((nb, na))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> Rfc3339 {
        Rfc3339::parse(s).unwrap()
    }

    #[test]
    fn grammar_terms() {
        assert_eq!(CalendarDuration::parse("20y").unwrap().years, 20);
        assert_eq!(CalendarDuration::parse("90d").unwrap().days, 90);
        assert_eq!(CalendarDuration::parse("15m").unwrap().minutes, 15);
        let d = CalendarDuration::parse("15 years 7 days").unwrap();
        assert_eq!((d.years, d.days), (15, 7));
        let d = CalendarDuration::parse("1y 6mo").unwrap();
        assert_eq!((d.years, d.months), (1, 6));
        assert!(
            CalendarDuration::parse("20").is_err(),
            "bare number rejected"
        );
        assert!(CalendarDuration::parse("3 fortnights").is_err());
        assert!(CalendarDuration::parse("").is_err());
        assert!(CalendarDuration::parse("0d").is_err());
    }

    #[test]
    fn gregorian_year_arithmetic_and_leap_clamp() {
        // 20 calendar years from a fixed start.
        let end = CalendarDuration::years(20)
            .add_to(ts("2026-07-07T12:00:00Z"))
            .unwrap();
        assert_eq!(end.to_string_canonical(), "2046-07-07T12:00:00Z");
        // Leap day + 1y clamps to Feb 28, never Mar 1.
        let end = CalendarDuration::years(1)
            .add_to(ts("2024-02-29T00:00:00Z"))
            .unwrap();
        assert_eq!(end.to_string_canonical(), "2025-02-28T00:00:00Z");
        // Leap day + 4y lands back on Feb 29.
        let end = CalendarDuration::years(4)
            .add_to(ts("2024-02-29T00:00:00Z"))
            .unwrap();
        assert_eq!(end.to_string_canonical(), "2028-02-29T00:00:00Z");
        // Month arithmetic clamps too: Jan 31 + 1mo = Feb 28/29.
        let end = CalendarDuration::parse("1mo")
            .unwrap()
            .add_to(ts("2025-01-31T00:00:00Z"))
            .unwrap();
        assert_eq!(end.to_string_canonical(), "2025-02-28T00:00:00Z");
        // Mixed calendar + exact parts.
        let end = CalendarDuration::parse("15 years 7 days")
            .unwrap()
            .add_to(ts("2026-07-07T12:00:00Z"))
            .unwrap();
        assert_eq!(end.to_string_canonical(), "2041-07-14T12:00:00Z");
    }

    #[test]
    fn rfc3339_round_trip() {
        let t = ts("2050-07-07T12:00:00Z");
        assert_eq!(t.to_string_canonical(), "2050-07-07T12:00:00Z");
        assert!(Rfc3339::parse("2050-07-07").is_err());
        assert!(Rfc3339::parse("not a date").is_err());
    }

    #[test]
    fn validity_resolution_rules() {
        let nb = ts("2026-01-01T00:00:00Z");
        // Default lifetime applies.
        let (b, a) = resolve_validity(Some(nb), None, None, CalendarDuration::years(20)).unwrap();
        assert_eq!(a.to_string_canonical(), "2046-01-01T00:00:00Z");
        assert_eq!(b, nb);
        // Exact not-after wins; both flags together are an error.
        let exact = ts("2032-07-07T12:00:00Z");
        let (_, a) =
            resolve_validity(Some(nb), None, Some(exact), CalendarDuration::years(5)).unwrap();
        assert_eq!(a, exact);
        assert!(
            resolve_validity(
                Some(nb),
                Some(CalendarDuration::days(90)),
                Some(exact),
                CalendarDuration::years(5)
            )
            .is_err()
        );
        // not-after <= not-before is an error.
        assert!(resolve_validity(Some(nb), None, Some(nb), CalendarDuration::years(5)).is_err());
    }
}
