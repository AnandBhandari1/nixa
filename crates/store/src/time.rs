use std::time::{SystemTime, UNIX_EPOCH};

pub const MINUTE_MS: i64 = 60_000;
pub const HOUR_MS: i64 = 60 * MINUTE_MS;
pub const DAY_MS: i64 = 24 * HOUR_MS;

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

pub fn weekday_of_days(days: i64) -> i64 {
    (days + 4).rem_euclid(7)
}

pub fn day_text(ms: i64) -> String {
    let (y, m, d) = civil_from_days(ms.div_euclid(DAY_MS));
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn today() -> String {
    day_text(now_ms())
}

pub fn parse_day(text: &str) -> Option<(i64, i64, i64)> {
    let bytes = text.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit()).then(|| s.parse::<i64>().ok()).flatten();
    let year = digits(&text[0..4])?;
    let month = digits(&text[5..7])?;
    let day = digits(&text[8..10])?;
    if !(1..=12).contains(&month) || day < 1 {
        return None;
    }
    let back = civil_from_days(days_from_civil(year, month, day));
    (back == (year, month, day)).then_some((year, month, day))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_round_trip() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(days_from_civil(2026, 11, 22)), (2026, 11, 22));
        assert_eq!(weekday_of_days(days_from_civil(2026, 11, 22)), 0);
        assert_eq!(day_text(0), "1970-01-01");
    }

    #[test]
    fn parses_real_days_only() {
        assert_eq!(parse_day("2024-02-29"), Some((2024, 2, 29)));
        assert_eq!(parse_day("2025-02-29"), None);
        assert_eq!(parse_day("2025-13-01"), None);
        assert_eq!(parse_day("2025-1-01"), None);
    }
}
