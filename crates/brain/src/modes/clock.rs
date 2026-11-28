use crate::schedules::zone::{self, Zone};
use std::sync::Mutex;

static ZONE: Mutex<Option<(Zone, u64)>> = Mutex::new(None);

fn here() -> Zone {
    zone::cached(&ZONE, crate::util::now_ms())
}

pub fn stamp_in(place: &Zone, at: u64) -> String {
    let local = zone::local_s(place, at as i64);
    let (y, m, d) = zone::civil(local.div_euclid(86_400));
    let secs = local.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", secs / 3600, (secs % 3600) / 60)
}

pub fn clock_in(place: &Zone, at: u64) -> String {
    let secs = zone::local_s(place, at as i64).rem_euclid(86_400);
    format!("{:02}:{:02}", secs / 3600, (secs % 3600) / 60)
}

pub fn day_start_in(place: &Zone, at: u64) -> u64 {
    let local = zone::local_s(place, at as i64);
    let midnight = local - local.rem_euclid(86_400);
    zone::to_utc(place, midnight).map(|s| (s.max(0) as u64) * 1000).unwrap_or_else(|| at - at % 86_400_000)
}

pub fn stamp(at: u64) -> String {
    stamp_in(&here(), at)
}

pub fn clock(at: u64) -> String {
    clock_in(&here(), at)
}

pub fn start_of_day(at: u64) -> u64 {
    day_start_in(&here(), at)
}
