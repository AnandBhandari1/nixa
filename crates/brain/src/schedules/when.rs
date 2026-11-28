use super::zone::{DAY_S, Zone, civil, local_s, to_utc, weekday};
use nixa_store::schedules::{LastRun, Repeat, RunResult, Schedule, valid_time};
use nixa_store::time::parse_day;

pub const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

fn hour_minute(time: &str) -> Option<(i64, i64)> {
    if !valid_time(time) {
        return None;
    }
    Some((time[0..2].parse().ok()?, time[3..5].parse().ok()?))
}

pub fn next_run(
    zone: &Zone,
    repeat: Repeat,
    time: &str,
    date: &str,
    days: &[i64],
    after_ms: i64,
) -> Option<i64> {
    let (hours, minutes) = hour_minute(time)?;
    let local_after = local_s(zone, after_ms);
    let later = |local: i64| {
        to_utc(zone, local)
            .map(|s| s * 1000)
            .filter(|ms| *ms > after_ms)
    };
    if repeat == Repeat::Hourly {
        let hour = local_after.div_euclid(3600) * 3600;
        return (0..3).find_map(|step| later(hour + step * 3600 + minutes * 60));
    }
    let list: Vec<i64> = if repeat == Repeat::Once {
        let (y, m, d) = parse_day(date)?;
        vec![super::zone::days_from_civil(y, m, d)]
    } else {
        let today = local_after.div_euclid(DAY_S);
        (0..15).map(|step| today + step).collect()
    };
    for day in list {
        let wd = weekday(day);
        if repeat == Repeat::Weekdays && (wd == 0 || wd == 6) {
            continue;
        }
        if repeat == Repeat::Weekly && !days.contains(&wd) {
            continue;
        }
        if let Some(ms) = later(day * DAY_S + hours * 3600 + minutes * 60) {
            return Some(ms);
        }
    }
    None
}

pub fn schedule_next(zone: &Zone, s: &Schedule, after_ms: i64) -> Option<i64> {
    next_run(zone, s.repeat, &s.time, &s.date, &s.days, after_ms)
}

pub fn missed_runs(zone: &Zone, s: &Schedule, due: i64, now: i64) -> u32 {
    let mut count = 0;
    let mut next = Some(due);
    while let Some(at) = next {
        if at > now || count >= 100 {
            break;
        }
        count += 1;
        next = schedule_next(zone, s, at);
    }
    count
}

fn parts(zone: &Zone, ms: i64) -> (i64, i64, i64, i64, i64) {
    let local = local_s(zone, ms);
    let (y, m, d) = civil(local.div_euclid(DAY_S));
    let secs = local.rem_euclid(DAY_S);
    (y, m, d, secs / 3600, secs % 3600 / 60)
}

pub fn clock(zone: &Zone, ms: i64) -> String {
    let (_, _, _, h, min) = parts(zone, ms);
    format!("{h:02}:{min:02}")
}

pub fn date_time(zone: &Zone, ms: i64) -> String {
    let (y, m, d, h, min) = parts(zone, ms);
    let wd = weekday(local_s(zone, ms).div_euclid(DAY_S)) as usize;
    format!("{} {y:04}-{m:02}-{d:02} {h:02}:{min:02}", DAYS[wd])
}

pub fn stamp(zone: &Zone, ms: i64) -> String {
    let (y, m, d, h, min) = parts(zone, ms);
    format!("{y:04}-{m:02}-{d:02}-{h:02}{min:02}")
}

pub fn maybe(zone: &Zone, ms: Option<i64>) -> String {
    ms.map(|at| date_time(zone, at))
        .unwrap_or_else(|| "—".into())
}

pub fn when_text(s: &Schedule) -> String {
    match s.repeat {
        Repeat::Once => format!("Once on {} at {}", s.date, s.time),
        Repeat::Hourly => format!("Every hour at :{}", s.time.get(3..5).unwrap_or("00")),
        Repeat::Daily => format!("Daily at {}", s.time),
        Repeat::Weekdays => format!("Weekdays at {}", s.time),
        Repeat::Weekly => {
            let names: Vec<&str> = s
                .days
                .iter()
                .filter_map(|d| usize::try_from(*d).ok().and_then(|d| DAYS.get(d).copied()))
                .collect();
            format!("{} at {}", names.join(", "), s.time)
        }
    }
}

pub fn last_text(zone: &Zone, run: Option<&LastRun>, folder: &str) -> String {
    let Some(run) = run else {
        return "Last: never".into();
    };
    if run.result == RunResult::Failed {
        return format!("Last: failed {}. {}", date_time(zone, run.at), run.error)
            .trim_end()
            .to_string();
    }
    if !run.team_run.is_empty() {
        return format!(
            "Last: team run {} started {}. The Team panel shows its steps.",
            run.team_run,
            date_time(zone, run.at)
        );
    }
    let report = if run.report.is_empty() {
        String::new()
    } else {
        let short = run
            .report
            .strip_prefix(&format!("{folder}/"))
            .unwrap_or(&run.report);
        format!(" Report: {short}.")
    };
    let Some(done) = run.done_at else {
        return format!(
            "Last: started {} in terminal {}, still running.{report}",
            date_time(zone, run.at),
            run.terminal
        );
    };
    let follow = if run.follow_terminal.is_empty() {
        String::new()
    } else {
        format!(" Follow-up agent in terminal {}.", run.follow_terminal)
    };
    format!(
        "Last: {}, done {}, {} open tasks.{report}{follow}",
        date_time(zone, run.at),
        date_time(zone, done),
        run.open_tasks.unwrap_or(0)
    )
}
