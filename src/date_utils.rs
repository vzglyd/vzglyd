#![allow(dead_code)]

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub fn utc_hhmm_from_unix(epoch_secs: u64) -> String {
    let seconds_today = epoch_secs % 86_400;
    let hours = seconds_today / 3_600;
    let minutes = (seconds_today / 60) % 60;
    format!("{hours:02}:{minutes:02}")
}

pub fn utc_ymdhms_from_unix(epoch_secs: u64) -> (i32, u8, u8, u8, u8, u8) {
    let days = (epoch_secs / 86_400) as i64;
    let (year, month, day) = civil_from_days(days);
    let seconds_today = epoch_secs % 86_400;
    let hour = (seconds_today / 3_600) as u8;
    let minute = ((seconds_today / 60) % 60) as u8;
    let second = (seconds_today % 60) as u8;
    (year, month, day, hour, minute, second)
}

pub fn utc_datetime_label(epoch_secs: u64) -> String {
    let (year, month, day, hour, minute, second) = utc_ymdhms_from_unix(epoch_secs);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
}

pub fn utc_month_day_from_unix(epoch_secs: u64) -> (i32, u8, u8) {
    let days = (epoch_secs / 86_400) as i64;
    civil_from_days(days)
}

pub fn weekday_abbrev_from_iso(date: &str) -> Option<&'static str> {
    let (year, month, day) = parse_ymd_prefix(date)?;
    Some(weekday_abbrev(year, month, day))
}

pub fn month_day_label(epoch_secs: u64) -> String {
    let (_year, month, day) = utc_month_day_from_unix(epoch_secs);
    format!("{day:02} {}", MONTHS[month.saturating_sub(1) as usize])
}

pub fn parse_ymd_prefix(input: &str) -> Option<(i32, u8, u8)> {
    let prefix = input.get(..10)?;
    let mut parts = prefix.split('-');
    let year = parts.next()?.parse().ok()?;
    let month = parts.next()?.parse().ok()?;
    let day = parts.next()?.parse().ok()?;
    Some((year, month, day))
}

pub fn weekday_abbrev(year: i32, month: u8, day: u8) -> &'static str {
    let offsets = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let mut year = year;
    if month < 3 {
        year -= 1;
    }
    let weekday = (year + year / 4 - year / 100
        + year / 400
        + offsets[month.saturating_sub(1) as usize]
        + i32::from(day))
        % 7;
    WEEKDAYS[weekday as usize]
}

fn civil_from_days(days: i64) -> (i32, u8, u8) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };
    (year as i32, month as u8, day as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weekday_abbrev_matches_known_dates() {
        assert_eq!(weekday_abbrev_from_iso("2026-03-19T00:00:00Z"), Some("Thu"));
        assert_eq!(weekday_abbrev_from_iso("2026-03-20"), Some("Fri"));
    }

    #[test]
    fn utc_hhmm_formats_epoch_time() {
        assert_eq!(utc_hhmm_from_unix(0), "00:00");
        assert_eq!(utc_hhmm_from_unix(43_200 + 2_040), "12:34");
    }

    #[test]
    fn utc_datetime_label_formats_full_timestamp() {
        assert_eq!(utc_datetime_label(0), "1970-01-01 00:00:00");
        assert_eq!(utc_datetime_label(131_696), "1970-01-02 12:34:56");
    }
}
