use std::time::{SystemTime, UNIX_EPOCH};

pub const DAY: i64 = 86_400;

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn iso(secs: i64) -> String {
    let (y, m, d) = civil_from_days(secs.div_euclid(DAY));
    let rest = secs.rem_euclid(DAY);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

pub fn now_iso() -> String {
    iso(now())
}

fn num(text: &str) -> Option<i64> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn zone(text: &str) -> Option<i64> {
    let t = text.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("z") || t.eq_ignore_ascii_case("utc") || t.eq_ignore_ascii_case("gmt") {
        return Some(0);
    }
    let sign = match t.as_bytes()[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let digits: String = t[1..].chars().filter(|c| *c != ':').collect();
    if digits.len() != 4 {
        return None;
    }
    let h = num(&digits[..2])?;
    let m = num(&digits[2..])?;
    Some(sign * (h * 3600 + m * 60))
}

pub fn parse(text: &str) -> Option<i64> {
    let t = text.trim();
    if t.len() < 16 {
        return None;
    }
    let date = &t[..10];
    let sep = t.as_bytes()[10];
    if sep != b'T' && sep != b' ' {
        return None;
    }
    let parts: Vec<&str> = date.split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    let (y, mo, d) = (num(parts[0])?, num(parts[1])?, num(parts[2])?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let rest = &t[11..];
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == ':' || c == '.'))
        .unwrap_or(rest.len());
    let clock = rest[..end].split('.').next().unwrap_or("");
    let bits: Vec<&str> = clock.split(':').collect();
    if bits.len() < 2 || bits.len() > 3 {
        return None;
    }
    let h = num(bits[0])?;
    let mi = num(bits[1])?;
    let s = if bits.len() == 3 { num(bits[2])? } else { 0 };
    let offset = zone(&rest[end..])?;
    Some(days_from_civil(y, mo, d) * DAY + h * 3600 + mi * 60 + s - offset)
}

pub fn parse_expiry(raw: &str) -> Option<String> {
    parse(raw).map(iso)
}

pub fn format_date(text: &str) -> String {
    match parse(text) {
        Some(secs) => {
            let (y, m, d) = civil_from_days(secs.div_euclid(DAY));
            format!("{d} {} {y}", MONTHS[(m - 1) as usize])
        }
        None => String::new(),
    }
}

pub fn days_left(text: &str, now: i64) -> Option<i64> {
    let at = parse(text)?;
    Some((at - now + DAY - 1).div_euclid(DAY))
}

pub fn is_past(text: Option<&str>, now: i64) -> bool {
    text.and_then(parse).is_some_and(|at| at <= now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_round_trips() {
        let secs = 1_795_340_590;
        assert_eq!(parse(&iso(secs)), Some(secs));
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(parse("2026-11-22T07:43:10.123Z"), parse("2026-11-22T07:43:10Z"));
    }

    #[test]
    fn expiry_reads_github_forms() {
        assert_eq!(
            parse_expiry("2026-12-01 10:00:00 UTC").as_deref(),
            Some("2026-12-01T10:00:00Z")
        );
        assert_eq!(
            parse_expiry("2026-12-01 18:00:00 +0800").as_deref(),
            Some("2026-12-01T10:00:00Z")
        );
        assert_eq!(parse_expiry(""), None);
        assert_eq!(parse_expiry("soon"), None);
    }

    #[test]
    fn dates_and_days() {
        assert_eq!(format_date("2026-12-01T10:00:00Z"), "1 Dec 2026");
        let now = parse("2026-11-22T00:00:00Z").unwrap();
        assert_eq!(days_left("2026-11-25T00:00:00Z", now), Some(3));
        assert_eq!(days_left("2026-11-24T12:00:00Z", now), Some(3));
        assert!(is_past(Some("2026-11-21T00:00:00Z"), now));
        assert!(!is_past(None, now));
    }
}
