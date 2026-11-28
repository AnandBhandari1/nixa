use crate::schedules::zone::{DAY_S, Zone, civil, days_from_civil, local_s, offset_at, to_utc};
use regex::Regex;
use std::sync::LazyLock;

pub const MINUTE_MS: i64 = 60_000;
pub const MAX_DAYS: i64 = 3660;
pub const HELP: &str = "Use at 5, tomorrow at 9 am, in 20 minutes, or a full time like 2026-12-01T09:00+01:00.";

static RELATIVE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^in (\d+|an?) (minute|min|hour|day)s?$").expect("relative time"));
static ISO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})(?::(\d{2})(?:\.\d+)?)?(Z|[+-]\d{2}:\d{2})$").expect("iso time")
});
static CLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:(tomorrow|today)\s+)?(?:at\s+)?(\d{1,2})(?::(\d{2}))?\s*(am|pm|a\.m\.?|p\.m\.?)?$").expect("clock time")
});

fn local_to_ms(zone: &Zone, local: i64) -> i64 {
    to_utc(zone, local).unwrap_or_else(|| local - offset_at(zone, local)) * 1000
}

fn relative(input: &str, now: i64) -> Option<Result<i64, String>> {
    let c = RELATIVE.captures(input)?;
    let count: i64 = match &c[1] {
        n if n.eq_ignore_ascii_case("a") || n.eq_ignore_ascii_case("an") => 1,
        n => n.parse().unwrap_or(0),
    };
    let unit = match c[2].to_lowercase().as_str() {
        "minute" | "min" => MINUTE_MS,
        "hour" => 60 * MINUTE_MS,
        _ => DAY_S * 1000,
    };
    if count <= 0 {
        return Some(Err("Give a future reminder time.".into()));
    }
    Some(match count.checked_mul(unit).and_then(|ms| now.checked_add(ms)) {
        Some(at) if at - now <= MAX_DAYS * DAY_S * 1000 => Ok(at),
        _ => Err("That reminder is too far away.".into()),
    })
}

fn iso(input: &str, now: i64) -> Option<Result<i64, String>> {
    let c = ISO.captures(input)?;
    let n = |i: usize| c.get(i).map(|m| m.as_str().parse::<i64>().unwrap_or(-1)).unwrap_or(0);
    let (y, m, d, h, min, s) = (n(1), n(2), n(3), n(4), n(5), n(6));
    let days = days_from_civil(y, m, d);
    if civil(days) != (y, m, d) || !(0..24).contains(&h) || !(0..60).contains(&min) || !(0..60).contains(&s) {
        return Some(Err("That is not a real date and time.".into()));
    }
    let zone = &c[7];
    let offset = if zone == "Z" {
        0
    } else {
        let sign = if zone.starts_with('-') { -1 } else { 1 };
        let hours: i64 = zone[1..3].parse().unwrap_or(0);
        let minutes: i64 = zone[4..6].parse().unwrap_or(0);
        sign * (hours * 3600 + minutes * 60)
    };
    let at = (days * DAY_S + h * 3600 + min * 60 + s - offset) * 1000;
    Some(if at <= now { Err("Give a future reminder time.".into()) } else { Ok(at) })
}

pub fn reminder_at(input: &str, now: i64, zone: &Zone) -> Result<i64, String> {
    let input = input.trim().trim_end_matches(['.', '!']);
    if let Some(found) = relative(input, now).or_else(|| iso(input, now)) {
        return found;
    }
    let Some(c) = CLOCK.captures(input) else {
        return Err(HELP.into());
    };
    let day_word = c.get(1).map(|m| m.as_str().to_lowercase());
    let hour: i64 = c[2].parse().unwrap_or(99);
    let minute: i64 = c.get(3).map(|m| m.as_str().parse().unwrap_or(99)).unwrap_or(0);
    let period = c.get(4).map(|m| m.as_str().to_lowercase().replace('.', ""));
    if hour > 23 || minute > 59 || (period.is_some() && !(1..=12).contains(&hour)) {
        return Err("That is not a valid clock time.".into());
    }
    let hour24 = match period.as_deref() {
        Some("pm") => hour % 12 + 12,
        Some(_) => hour % 12,
        None => hour,
    };
    let today = local_s(zone, now).div_euclid(DAY_S);
    let mut local = today * DAY_S + hour24 * 3600 + minute * 60;
    match day_word.as_deref() {
        Some("tomorrow") => local += DAY_S,
        _ if period.is_none() && (1..=12).contains(&hour) => {
            while local_to_ms(zone, local) <= now {
                local += 12 * 3600;
            }
            if day_word.as_deref() == Some("today") && local.div_euclid(DAY_S) != today {
                return Err("Give a future reminder time.".into());
            }
        }
        _ => {}
    }
    if local_to_ms(zone, local) <= now && day_word.is_none() {
        local += DAY_S;
    }
    let at = local_to_ms(zone, local);
    if at <= now {
        return Err("Give a future reminder time.".into());
    }
    Ok(at)
}

pub fn to_minute(at: i64) -> i64 {
    (at + MINUTE_MS - 1).div_euclid(MINUTE_MS) * MINUTE_MS
}

pub fn local_day_time(zone: &Zone, at: i64) -> (String, String) {
    let local = local_s(zone, at);
    let (y, m, d) = civil(local.div_euclid(DAY_S));
    let secs = local.rem_euclid(DAY_S);
    (format!("{y:04}-{m:02}-{d:02}"), format!("{:02}:{:02}", secs / 3600, secs % 3600 / 60))
}
