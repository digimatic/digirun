//! Timestamps and dates as they appear in the downloaded files.
//!
//! A timestamp is kept exactly as parsed (date, time of day, and the offset it
//! carried) so `isoformat()` reproduces what Python wrote to `segments.csv`.
//! Comparisons are by instant, which is also how Python compares aware datetimes.

use std::fmt;

/// A civil date (no time, no zone).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

/// A timestamp with the UTC offset it was read with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateTime {
    pub date: Date,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub micro: u32,
    /// Seconds east of UTC.
    pub offset: i32,
}

impl Date {
    pub fn isoformat(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// Parse `YYYY-MM-DD`.
    pub fn from_iso(value: &str) -> Option<Date> {
        let mut parts = value.split('-');
        let (y, m, d) = match (parts.next(), parts.next(), parts.next()) {
            (Some(y), Some(m), Some(d)) => (y, m, d),
            _ => return None,
        };
        let year = y.parse::<i32>().ok()?;
        let month = m.parse::<u32>().ok()?;
        let day = d.parse::<u32>().ok()?;
        if month == 0 || month > 12 || day == 0 || day > 31 {
            return None;
        }
        Some(Date { year, month, day })
    }

    /// Today's date in UTC.
    pub fn today() -> Date {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        civil_from_days(secs.div_euclid(86400))
    }

    /// The date `days` days earlier.
    pub fn minus_days(&self, days: i64) -> Date {
        civil_from_days(days_from_civil(self.year, self.month, self.day) - days)
    }

    /// `(iso_year, iso_week)`, matching `date.isocalendar()`.
    pub fn isocalendar(&self) -> (i32, u32) {
        let days = days_from_civil(self.year, self.month, self.day);
        // Jan 4 is always in ISO week 1 of its year.
        let jan4 = Date {
            year: self.year,
            month: 1,
            day: 4,
        };
        let start = days_from_civil(jan4.year, 1, 4) - (weekday(jan4) - 1);
        let diff = days - start;
        if diff >= 0 {
            return (self.year, (diff / 7 + 1) as u32);
        }
        let prev = Date {
            year: self.year - 1,
            month: 1,
            day: 4,
        };
        let prev_start = days_from_civil(prev.year, 1, 4) - (weekday(prev) - 1);
        (self.year - 1, ((days - prev_start) / 7 + 1) as u32)
    }
}

impl DateTime {
    /// Parse an ISO timestamp such as `2026-08-07T14:26:09.000Z` or
    /// `2026-08-07T14:26:09+02:00`. Returns None when the value is not a time.
    pub fn from_iso(value: &str) -> Option<DateTime> {
        let value = value.trim();
        if value.is_empty() {
            return None;
        }
        let date_end = value.find(['T', ' '])?;
        let date = Date::from_iso(&value[..date_end])?;
        let rest = &value[date_end + 1..];

        // The zone comes after the clock, so split it off before parsing HH:MM:SS.
        let zone_at = rest.find(['Z', '+', '-']);
        let (clock, offset) = match zone_at {
            Some(index) => (&rest[..index], parse_offset(&rest[index..])?),
            None => (rest, 0),
        };

        // HH:MM:SS, seconds optional in ISO but always present in these files.
        let mut clock_parts = clock.split(':');
        let (h, m, s) = match (clock_parts.next(), clock_parts.next(), clock_parts.next()) {
            (Some(h), Some(m), Some(s)) => (h, m, s),
            (Some(h), Some(m), None) => (h, m, "0"),
            _ => return None,
        };
        let hour = h.parse::<u32>().ok()?;
        let minute = m.parse::<u32>().ok()?;
        let (second, micro) = split_seconds(s)?;
        if hour > 23 || minute > 59 || second > 59 {
            return None;
        }

        Some(DateTime {
            date,
            hour,
            minute,
            second,
            micro,
            offset,
        })
    }

    /// Microseconds since the Unix epoch, so timestamps compare by instant.
    pub fn instant(&self) -> i64 {
        let days = days_from_civil(self.date.year, self.date.month, self.date.day);
        let local =
            ((days * 24 + self.hour as i64) * 60 + self.minute as i64) * 60 + self.second as i64;
        (local - self.offset as i64) * 1_000_000 + self.micro as i64
    }

    /// Seconds since the previous timestamp, as Python's `total_seconds()`.
    pub fn seconds_since(&self, other: &DateTime) -> f64 {
        (self.instant() - other.instant()) as f64 / 1_000_000.0
    }

    /// Python's `datetime.isoformat()`.
    pub fn isoformat(&self) -> String {
        let base = format!(
            "{}T{:02}:{:02}:{:02}",
            self.date.isoformat(),
            self.hour,
            self.minute,
            self.second
        );
        let frac = if self.micro != 0 {
            format!(".{:06}", self.micro)
        } else {
            String::new()
        };
        let sign = if self.offset < 0 { '-' } else { '+' };
        let abs = self.offset.abs();
        format!(
            "{}{}{}{:02}:{:02}",
            base,
            frac,
            sign,
            abs / 3600,
            (abs % 3600) / 60
        )
    }
}

fn split_seconds(value: &str) -> Option<(u32, u32)> {
    let second = value.split(['.', ',']).next()?.parse::<u32>().ok()?;
    let micro = match value.split_once(['.', ',']) {
        Some((_, frac)) => {
            let digits = frac.chars().take(6).collect::<String>();
            digits.parse::<u32>().ok()?
        }
        None => 0,
    };
    Some((second, micro))
}

fn parse_offset(value: &str) -> Option<i32> {
    let bytes = value.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    if bytes[0] == b'Z' {
        return Some(0);
    }
    let sign: i32 = match bytes[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let mut parts = value[1..].split(':');
    let hours = parts.next()?.parse::<i32>().ok()?;
    let minutes = parts.next().unwrap_or("0").parse::<i32>().ok()?;
    Some(sign * (hours * 3600 + minutes * 60))
}

/// Days from 1970-01-01 (Howard Hinnant's `days_from_civil`).
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year } as i64;
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy =
        (153 * (if month > 2 { month - 3 } else { month + 9 }) as i64 + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// The date `days` after 1970-01-01 (Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> Date {
    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = (yoe + era * 400 + if month <= 2 { 1 } else { 0 }) as i32;
    Date { year, month, day }
}

/// ISO weekday: 1 = Monday .. 7 = Sunday.
fn weekday(date: Date) -> i64 {
    let days = days_from_civil(date.year, date.month, date.day);
    // days_from_civil(1970-01-01) is 0 and that day was a Thursday, so +3 makes Monday 0.
    let rem = ((days + 3) % 7 + 7) % 7;
    rem + 1
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.isoformat())
    }
}
