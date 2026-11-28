use crate::app::lock;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

pub const ZONE_DIRS: [&str; 2] = ["/var/db/timezone/zoneinfo", "/usr/share/zoneinfo"];
pub const LOCAL_FILE: &str = "/etc/localtime";
pub const DAY_S: i64 = 86_400;
const RELOAD_MS: u64 = 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Day {
    Month { month: i64, week: i64, weekday: i64 },
    Julian(i64),
    Zero(i64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    pub day: Day,
    pub time: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    pub std: i64,
    pub dst: Option<(i64, Change, Change)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zone {
    pub name: String,
    pub times: Vec<i64>,
    pub offsets: Vec<i64>,
    pub first: i64,
    pub rule: Option<Rule>,
}

pub fn utc() -> Zone {
    Zone {
        name: "UTC".into(),
        times: Vec::new(),
        offsets: Vec::new(),
        first: 0,
        rule: None,
    }
}

fn be32(b: &[u8], at: usize) -> Option<i64> {
    let s = b.get(at..at + 4)?;
    Some(i32::from_be_bytes([s[0], s[1], s[2], s[3]]) as i64)
}

fn be64(b: &[u8], at: usize) -> Option<i64> {
    let s = b.get(at..at + 8)?;
    Some(i64::from_be_bytes([
        s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
    ]))
}

fn counts(b: &[u8], at: usize) -> Option<[usize; 6]> {
    if b.get(at..at + 4)? != b"TZif" {
        return None;
    }
    let mut out = [0usize; 6];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = usize::try_from(be32(b, at + 20 + i * 4)?).ok()?;
    }
    Some(out)
}

fn block_len(c: &[usize; 6], wide: bool) -> usize {
    let [isut, isstd, leap, time, kinds, chars] = *c;
    let t = if wide { 8 } else { 4 };
    time * t + time + kinds * 6 + chars + leap * (t + 4) + isstd + isut
}

pub fn parse_tzif(name: &str, b: &[u8]) -> Option<Zone> {
    let first = counts(b, 0)?;
    let version = *b.get(4)?;
    let (c, start, wide) = if version >= b'2' {
        let second = 44 + block_len(&first, false);
        (counts(b, second)?, second + 44, true)
    } else {
        (first, 44, false)
    };
    let [_, _, _, time_n, kind_n, _] = c;
    if kind_n == 0 {
        return None;
    }
    let t = if wide { 8 } else { 4 };
    let mut times = Vec::with_capacity(time_n);
    for i in 0..time_n {
        times.push(if wide {
            be64(b, start + i * t)?
        } else {
            be32(b, start + i * t)?
        });
    }
    let idx_at = start + time_n * t;
    let kinds_at = idx_at + time_n;
    let kind_offset = |k: usize| -> Option<i64> {
        if k >= kind_n {
            return None;
        }
        be32(b, kinds_at + k * 6)
    };
    let mut offsets = Vec::with_capacity(time_n);
    for i in 0..time_n {
        offsets.push(kind_offset(*b.get(idx_at + i)? as usize)?);
    }
    let rule = if wide {
        let end = start + block_len(&c, true);
        let tail = b.get(end..).unwrap_or_default();
        let text = String::from_utf8_lossy(tail);
        let footer = text
            .trim_matches('\n')
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
        if footer.is_empty() {
            None
        } else {
            parse_rule(&footer)
        }
    } else {
        None
    };
    Some(Zone {
        name: name.to_string(),
        times,
        offsets,
        first: kind_offset(0)?,
        rule,
    })
}

fn take_name(s: &str) -> Option<(&str, &str)> {
    if let Some(rest) = s.strip_prefix('<') {
        let end = rest.find('>')?;
        return Some((&rest[..end], &rest[end + 1..]));
    }
    let end = s
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(s.len());
    (end >= 3).then(|| (&s[..end], &s[end..]))
}

fn take_clock(s: &str) -> Option<(i64, &str)> {
    let (sign, body) = match s.as_bytes().first()? {
        b'-' => (-1, &s[1..]),
        b'+' => (1, &s[1..]),
        _ => (1, s),
    };
    let end = body
        .find(|c: char| !(c.is_ascii_digit() || c == ':'))
        .unwrap_or(body.len());
    let parts: Vec<&str> = body[..end].split(':').collect();
    if parts.is_empty() || parts.len() > 3 || parts[0].is_empty() {
        return None;
    }
    let mut secs = 0;
    for (i, part) in parts.iter().enumerate() {
        let n: i64 = part.parse().ok()?;
        secs += n * [3600, 60, 1][i];
    }
    Some((sign * secs, &body[end..]))
}

fn take_change(s: &str) -> Option<(Change, &str)> {
    let end = s.find([',', '/']).unwrap_or(s.len());
    let spec = &s[..end];
    let day = if let Some(m) = spec.strip_prefix('M') {
        let parts: Vec<i64> = m
            .split('.')
            .map(|p| p.parse().ok())
            .collect::<Option<_>>()?;
        let [month, week, weekday] = parts[..] else {
            return None;
        };
        if !(1..=12).contains(&month) || !(1..=5).contains(&week) || !(0..=6).contains(&weekday) {
            return None;
        }
        Day::Month {
            month,
            week,
            weekday,
        }
    } else if let Some(j) = spec.strip_prefix('J') {
        Day::Julian(j.parse().ok().filter(|n| (1..=365).contains(n))?)
    } else {
        Day::Zero(spec.parse().ok().filter(|n| (0..=365).contains(n))?)
    };
    let mut rest = &s[end..];
    let mut time = 7200;
    if let Some(r) = rest.strip_prefix('/') {
        let (t, after) = take_clock(r)?;
        time = t;
        rest = after;
    }
    Some((Change { day, time }, rest))
}

pub fn parse_rule(text: &str) -> Option<Rule> {
    let (_, rest) = take_name(text.trim())?;
    let (west, rest) = take_clock(rest)?;
    let std = -west;
    if rest.is_empty() {
        return Some(Rule { std, dst: None });
    }
    let (_, mut rest) = take_name(rest)?;
    let mut dst = std + 3600;
    if !rest.starts_with(',') && !rest.is_empty() {
        let (w, after) = take_clock(rest)?;
        dst = -w;
        rest = after;
    }
    let rest = rest.strip_prefix(',')?;
    let (start, rest) = take_change(rest)?;
    let rest = rest.strip_prefix(',')?;
    let (end, rest) = take_change(rest)?;
    rest.is_empty().then_some(Rule {
        std,
        dst: Some((dst, start, end)),
    })
}

pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    nixa_store::time::days_from_civil(y, m, d)
}

pub fn civil(days: i64) -> (i64, i64, i64) {
    nixa_store::time::civil_from_days(days)
}

pub fn weekday(days: i64) -> i64 {
    nixa_store::time::weekday_of_days(days)
}

fn leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn month_days(y: i64, m: i64) -> i64 {
    match m {
        2 if leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn change_day(year: i64, day: Day) -> i64 {
    let jan1 = days_from_civil(year, 1, 1);
    match day {
        Day::Zero(n) => jan1 + n,
        Day::Julian(n) => jan1 + n - 1 + i64::from(leap(year) && n >= 60),
        Day::Month {
            month,
            week,
            weekday: wd,
        } => {
            let first = days_from_civil(year, month, 1);
            let mut d = first + (wd - weekday(first)).rem_euclid(7) + (week - 1) * 7;
            while d >= first + month_days(year, month) {
                d -= 7;
            }
            d
        }
    }
}

fn rule_offset(rule: &Rule, utc_s: i64) -> i64 {
    let Some((dst, start, end)) = rule.dst else {
        return rule.std;
    };
    let year = civil((utc_s + rule.std).div_euclid(DAY_S)).0;
    let on = change_day(year, start.day) * DAY_S + start.time - rule.std;
    let off = change_day(year, end.day) * DAY_S + end.time - dst;
    let summer = if on < off {
        utc_s >= on && utc_s < off
    } else {
        !(utc_s >= off && utc_s < on)
    };
    if summer { dst } else { rule.std }
}

pub fn offset_at(zone: &Zone, utc_s: i64) -> i64 {
    if zone.times.is_empty() || utc_s < zone.times[0] {
        return match (&zone.rule, zone.times.is_empty()) {
            (Some(rule), true) => rule_offset(rule, utc_s),
            _ => zone.first,
        };
    }
    let last = zone.times.len() - 1;
    if utc_s >= zone.times[last]
        && let Some(rule) = &zone.rule
    {
        return rule_offset(rule, utc_s);
    }
    let at = zone.times.partition_point(|t| *t <= utc_s);
    zone.offsets[at.saturating_sub(1)]
}

pub fn to_utc(zone: &Zone, local_s: i64) -> Option<i64> {
    let mut found: Option<i64> = None;
    for guess in [
        offset_at(zone, local_s - DAY_S),
        offset_at(zone, local_s + DAY_S),
        offset_at(zone, local_s),
    ] {
        let utc = local_s - guess;
        if offset_at(zone, utc) == guess {
            found = Some(found.map_or(utc, |f| f.min(utc)));
        }
    }
    found
}

pub fn local_s(zone: &Zone, utc_ms: i64) -> i64 {
    let s = utc_ms.div_euclid(1000);
    s + offset_at(zone, s)
}

pub fn read_file(name: &str, path: &Path) -> Option<Zone> {
    parse_tzif(name, &fs::read(path).ok()?)
}

pub fn named(name: &str) -> Option<Zone> {
    let name = name.trim().trim_start_matches(':');
    if name.is_empty() || name.contains("..") {
        return None;
    }
    if name.starts_with('/') {
        return read_file(name, Path::new(name));
    }
    ZONE_DIRS
        .iter()
        .find_map(|dir| read_file(name, &Path::new(dir).join(name)))
}

pub fn from_env(tz: Option<&str>) -> Zone {
    if let Some(tz) = tz.map(str::trim).filter(|t| !t.is_empty()) {
        if let Some(zone) = named(tz) {
            return zone;
        }
        if let Some(rule) = parse_rule(tz.trim_start_matches(':')) {
            return Zone {
                name: tz.to_string(),
                times: Vec::new(),
                offsets: Vec::new(),
                first: rule.std,
                rule: Some(rule),
            };
        }
    }
    let link = fs::read_link(LOCAL_FILE)
        .ok()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let name = ZONE_DIRS
        .iter()
        .find_map(|dir| link.strip_prefix(&format!("{dir}/")))
        .unwrap_or("local")
        .to_string();
    read_file(&name, Path::new(LOCAL_FILE)).unwrap_or_else(utc)
}

pub fn local() -> Zone {
    from_env(std::env::var("TZ").ok().as_deref())
}

pub fn cached(slot: &Mutex<Option<(Zone, u64)>>, now: u64) -> Zone {
    let mut guard = lock(slot);
    if let Some((zone, at)) = guard.as_ref()
        && now.saturating_sub(*at) < RELOAD_MS
    {
        return zone.clone();
    }
    let zone = local();
    *guard = Some((zone.clone(), now));
    zone
}
