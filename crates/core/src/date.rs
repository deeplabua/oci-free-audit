//! Calendar dates in UTC — just enough for billing months, without a date crate.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

/// A UTC calendar date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    /// Year, e.g. 2026.
    pub year: i32,
    /// Month, 1–12.
    pub month: u32,
    /// Day of month, 1–31.
    pub day: u32,
}

impl Date {
    /// A date; `None` if it does not exist.
    pub fn new(year: i32, month: u32, day: u32) -> Option<Self> {
        let valid = (1..=12).contains(&month) && day >= 1 && day <= days_in_month(year, month);
        valid.then_some(Self { year, month, day })
    }

    /// Today in UTC.
    pub fn today() -> Self {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        Self::from_days((secs / 86_400) as i64)
    }

    /// Parse `YYYY-MM-DD`, or the date part of an RFC 3339 timestamp.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.get(..10)?;
        let mut parts = text.split('-');
        let year = parts.next()?.parse().ok()?;
        let month = parts.next()?.parse().ok()?;
        let day = parts.next()?.parse().ok()?;
        Self::new(year, month, day)
    }

    /// The first day of this date's month.
    pub fn first_of_month(self) -> Self {
        Self { day: 1, ..self }
    }

    /// The first day of the previous month.
    pub fn previous_month(self) -> Self {
        if self.month == 1 {
            Self {
                year: self.year - 1,
                month: 12,
                day: 1,
            }
        } else {
            Self {
                month: self.month - 1,
                day: 1,
                ..self
            }
        }
    }

    /// `YYYY-MM`, the billing month this date falls in.
    pub fn month_key(self) -> String {
        format!("{:04}-{:02}", self.year, self.month)
    }

    /// `"November 2026"`.
    pub fn month_name(self) -> String {
        const NAMES: [&str; 12] = [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ];
        format!("{} {}", NAMES[(self.month - 1) as usize], self.year)
    }

    /// Midnight UTC as RFC 3339, the form the Usage API wants.
    pub fn rfc3339(self) -> String {
        format!("{self}T00:00:00Z")
    }

    /// Days since 1970-01-01.
    pub fn to_days(self) -> i64 {
        // Howard Hinnant's days_from_civil.
        let y = i64::from(self.year) - i64::from(self.month <= 2);
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let m = i64::from(self.month);
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(self.day) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    /// The date `days` after 1970-01-01.
    pub fn from_days(days: i64) -> Self {
        // Howard Hinnant's civil_from_days.
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
        Self { year, month, day }
    }
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

impl serde::Serialize for Date {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_days() {
        for (days, text) in [
            (0, "1970-01-01"),
            (20_402, "2025-11-10"),
            (20_767, "2026-11-10"),
        ] {
            let d = Date::from_days(days);
            assert_eq!(d.to_string(), text);
            assert_eq!(d.to_days(), days);
        }
    }

    #[test]
    fn parses_and_validates() {
        assert_eq!(Date::parse("2026-11-10T13:00:00Z"), Date::new(2026, 11, 10));
        assert!(Date::parse("2026-02-30").is_none());
        assert!(Date::parse("2024-02-29").is_some());
        assert!(Date::parse("nope").is_none());
    }

    #[test]
    fn months() {
        let d = Date::new(2026, 1, 15).unwrap();
        assert_eq!(d.previous_month().to_string(), "2025-12-01");
        assert_eq!(d.first_of_month().rfc3339(), "2026-01-01T00:00:00Z");
        assert_eq!(d.month_key(), "2026-01");
        assert_eq!(
            Date::new(2026, 11, 3).unwrap().month_name(),
            "November 2026"
        );
    }
}
