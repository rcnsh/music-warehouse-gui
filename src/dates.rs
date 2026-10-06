//! Calendar ranges for the Overview, resolved in the system timezone.
//!
//! The Worker buckets days itself from `from`/`to`/`tz`, so the client only
//! decides which local dates to ask for, and never converts timestamps into
//! days on its own.

use chrono::{DateTime, Datelike, Days, Local, NaiveDate, TimeZone};

/// The Worker refuses ranges over 3660 days. Ranges are checked in UTC
/// milliseconds there, so a span that gains an hour across DST could tip a
/// 3660-day request over; one day of headroom avoids that edge.
pub const MAX_RANGE_DAYS: u64 = 3659;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangePreset {
    Last7Days,
    Last30Days,
    LastYear,
    Custom,
}

impl RangePreset {
    pub const ALL: [RangePreset; 4] = [
        Self::Last7Days,
        Self::Last30Days,
        Self::LastYear,
        Self::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Last7Days => "7d",
            Self::Last30Days => "30d",
            Self::LastYear => "1y",
            Self::Custom => "Custom",
        }
    }

    /// Inclusive day count ending today, or `None` for a custom range.
    fn days(self) -> Option<u64> {
        match self {
            Self::Last7Days => Some(7),
            Self::Last30Days => Some(30),
            Self::LastYear => Some(365),
            Self::Custom => None,
        }
    }

    /// The range ending on `today`, counting today as the last day.
    pub fn resolve(self, today: NaiveDate) -> Option<DateRange> {
        let days = self.days()?;
        let from = today.checked_sub_days(Days::new(days - 1))?;
        DateRange::new(from, today).ok()
    }
}

/// An inclusive range of local calendar days.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateRange {
    from: NaiveDate,
    to: NaiveDate,
}

impl DateRange {
    pub fn new(from: NaiveDate, to: NaiveDate) -> Result<Self, String> {
        if to < from {
            return Err("The end date is before the start date.".into());
        }
        let range = Self { from, to };
        if range.day_count() > MAX_RANGE_DAYS {
            return Err(format!(
                "That range is {} days; the Worker allows at most {MAX_RANGE_DAYS}.",
                range.day_count()
            ));
        }
        Ok(range)
    }

    #[cfg(test)]
    pub fn from(&self) -> NaiveDate {
        self.from
    }

    #[cfg(test)]
    pub fn to(&self) -> NaiveDate {
        self.to
    }

    pub fn day_count(&self) -> u64 {
        (self.to - self.from).num_days() as u64 + 1
    }

    pub fn query_from(&self) -> String {
        self.from.format("%Y-%m-%d").to_string()
    }

    pub fn query_to(&self) -> String {
        self.to.format("%Y-%m-%d").to_string()
    }

    pub fn describe(&self) -> String {
        if self.from.year() == self.to.year() {
            format!(
                "{} – {}",
                self.from.format("%-d %b"),
                self.to.format("%-d %b %Y")
            )
        } else {
            format!(
                "{} – {}",
                self.from.format("%-d %b %Y"),
                self.to.format("%-d %b %Y")
            )
        }
    }
}

/// Parses the custom-range inputs. Future end dates are refused because the
/// Worker would only pad them with zero days.
pub fn parse_custom(from: &str, to: &str, today: NaiveDate) -> Result<DateRange, String> {
    let parse = |label: &str, value: &str| {
        NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d")
            .map_err(|_| format!("{label} must be a date like 2026-01-31."))
    };
    let from = parse("Start", from)?;
    let to = parse("End", to)?;
    if to > today {
        return Err("The end date is in the future.".into());
    }
    DateRange::new(from, to)
}

/// Parses the History "go to date" box. A future day is clamped to today:
/// asking for tomorrow most likely means "the newest plays", not an error.
pub fn parse_jump(text: &str, today: NaiveDate) -> Result<NaiveDate, String> {
    let day = NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d")
        .map_err(|_| "Enter a date like 2024-05-31.".to_owned())?;
    Ok(day.min(today))
}

/// IANA name of the system zone, sent as `tz` so the Worker's day buckets
/// match the clock on this Mac.
pub fn system_timezone() -> String {
    iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".to_owned())
}

pub fn today_local() -> NaiveDate {
    Local::now().date_naive()
}

/// UTC milliseconds at local midnight ending `day`: the exclusive `before`
/// bound that lists that day's plays first. `None` only if the zone has no
/// such midnight at all; across a DST gap the earliest valid instant is used.
pub fn end_of_local_day_ms<Tz: TimeZone>(day: NaiveDate, tz: &Tz) -> Option<i64> {
    let next = day.succ_opt()?.and_hms_opt(0, 0, 0)?;
    tz.from_local_datetime(&next)
        .earliest()
        .map(|t| t.timestamp_millis())
}

/// Formats a play's UTC timestamp in the given zone for the history table.
pub fn format_played_at<Tz: TimeZone>(utc_ms: i64, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    match DateTime::from_timestamp_millis(utc_ms) {
        Some(utc) => utc
            .with_timezone(tz)
            .format("%a %-d %b %Y  %H:%M")
            .to_string(),
        None => "—".to_owned(),
    }
}

/// Bar label for a `YYYY-MM-DD` day from `/api/daily`. Labels double as the
/// chart's band keys, so they must stay unique: short ranges can drop the
/// year, long ones keep the full date.
pub fn day_label(day: &str, range_days: u64) -> String {
    match NaiveDate::parse_from_str(day, "%Y-%m-%d") {
        Ok(date) if range_days <= 31 => date.format("%a %-d %b").to_string(),
        Ok(date) => date.format("%-d %b %Y").to_string(),
        Err(_) => day.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn presets_end_today_and_count_today() {
        let today = d(2026, 10, 7);
        let week = RangePreset::Last7Days.resolve(today).unwrap();
        assert_eq!((week.from(), week.to()), (d(2026, 10, 1), today));
        assert_eq!(week.day_count(), 7);

        let month = RangePreset::Last30Days.resolve(today).unwrap();
        assert_eq!(month.from(), d(2026, 9, 8));
        assert_eq!(month.day_count(), 30);

        let year = RangePreset::LastYear.resolve(today).unwrap();
        assert_eq!(year.from(), d(2025, 10, 8));
        assert_eq!(year.day_count(), 365);

        assert_eq!(RangePreset::Custom.resolve(today), None);
    }

    #[test]
    fn presets_cross_month_and_leap_day() {
        let year = RangePreset::LastYear.resolve(d(2028, 3, 1)).unwrap();
        // The window contains 29 Feb 2028, so it starts a day later than the
        // same date a year back.
        assert_eq!(year.from(), d(2027, 3, 3));
        assert_eq!(year.day_count(), 365);
        let week = RangePreset::Last7Days.resolve(d(2026, 3, 2)).unwrap();
        assert_eq!(week.from(), d(2026, 2, 24));
    }

    #[test]
    fn range_validation() {
        assert!(DateRange::new(d(2026, 1, 2), d(2026, 1, 1)).is_err());
        assert_eq!(
            DateRange::new(d(2026, 1, 1), d(2026, 1, 1))
                .unwrap()
                .day_count(),
            1
        );
        let max_ok = d(2026, 1, 1)
            .checked_add_days(Days::new(MAX_RANGE_DAYS - 1))
            .unwrap();
        assert!(DateRange::new(d(2026, 1, 1), max_ok).is_ok());
        assert!(DateRange::new(d(2026, 1, 1), max_ok.succ_opt().unwrap()).is_err());
    }

    #[test]
    fn custom_parsing() {
        let today = d(2026, 10, 7);
        let range = parse_custom(" 2026-09-01 ", "2026-09-30", today).unwrap();
        assert_eq!(range.query_from(), "2026-09-01");
        assert_eq!(range.query_to(), "2026-09-30");
        assert!(parse_custom("2026-9-1x", "2026-09-30", today).is_err());
        assert!(parse_custom("2026-09-30", "2026-09-01", today).is_err());
        assert!(parse_custom("2026-10-01", "2026-10-08", today).is_err());
        assert!(parse_custom("2026-10-07", "2026-10-07", today).is_ok());
    }

    #[test]
    fn describe_collapses_shared_year() {
        let same = DateRange::new(d(2026, 9, 8), d(2026, 10, 7)).unwrap();
        assert_eq!(same.describe(), "8 Sep – 7 Oct 2026");
        let across = DateRange::new(d(2025, 10, 8), d(2026, 10, 7)).unwrap();
        assert_eq!(across.describe(), "8 Oct 2025 – 7 Oct 2026");
    }

    #[test]
    fn played_at_uses_the_given_zone() {
        // 1791312948427 ms is 2026-10-06T18:55:48Z.
        let utc = FixedOffset::east_opt(0).unwrap();
        assert_eq!(
            format_played_at(1_791_312_948_427, &utc),
            "Tue 6 Oct 2026  18:55"
        );
        // A zone ahead of UTC pushes it past midnight into the next day.
        let tokyo = FixedOffset::east_opt(9 * 3600).unwrap();
        assert_eq!(
            format_played_at(1_791_312_948_427, &tokyo),
            "Wed 7 Oct 2026  03:55"
        );
    }

    #[test]
    fn jump_dates_parse_and_clamp_to_today() {
        let today = d(2026, 10, 7);
        assert_eq!(parse_jump(" 2024-05-31 ", today), Ok(d(2024, 5, 31)));
        assert_eq!(parse_jump("2027-01-01", today), Ok(today));
        assert!(parse_jump("31/05/2024", today).is_err());
        assert!(parse_jump("", today).is_err());
    }

    #[test]
    fn end_of_day_is_next_local_midnight() {
        let utc = FixedOffset::east_opt(0).unwrap();
        // 2026-10-07T00:00:00Z
        assert_eq!(
            end_of_local_day_ms(d(2026, 10, 6), &utc),
            Some(1_791_331_200_000)
        );
        // Singapore midnight is 16:00 UTC the previous day.
        let sgt = FixedOffset::east_opt(8 * 3600).unwrap();
        assert_eq!(
            end_of_local_day_ms(d(2026, 10, 6), &sgt),
            Some(1_791_331_200_000 - 8 * 3_600_000)
        );
    }

    #[test]
    fn day_labels_stay_unique_across_years() {
        assert_eq!(day_label("2026-10-06", 7), "Tue 6 Oct");
        assert_eq!(day_label("2026-10-06", 365), "6 Oct 2026");
        assert_ne!(day_label("2025-10-06", 400), day_label("2026-10-06", 400));
        assert_eq!(day_label("garbage", 7), "garbage");
    }
}
