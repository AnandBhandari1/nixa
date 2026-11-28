use crate::schedules::zone::{self, Zone};
use serde_json::{Value, json};

pub const LINE_MAX: usize = 140;
const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

pub const LEAVE_LINES: [&str; 6] = [
    "{name} went on leave till {when}, enjoying the beach. Asking {to}.",
    "{name} is on holiday till {when}, phone switched off. {to}, over to you.",
    "{name} took a very long tea break till {when}. {to} picks up the work.",
    "{name} is out of office till {when}. The auto-reply says: ask {to}.",
    "{name} ran out of coffee and coupons, back {when}. {to} takes the desk.",
    "{name} went home early, back {when}. {to}, your turn.",
];

pub const ALONE_LINES: [&str; 3] = [
    "{name} went on leave till {when}. Nobody else is free, so the work waits on the desk.",
    "{name} is on holiday till {when}. The office is empty, the task waits.",
    "{name} took the day off till {when}. The task sits in the inbox for now.",
];

pub const BACK_LINES: [&str; 4] = [
    "{name} is back from leave, tanned and ready.",
    "{name} is back at the desk after the holiday.",
    "{name} came back with snacks for everyone.",
    "{name} is back, coffee in hand.",
];

pub enum Moved {
    Nothing,
    Moved { title: String, to_title: String },
    Waiting { title: String },
    Failed { title: String, error: String },
}

pub fn day_part(place: &Zone, until: u64, now: u64) -> (String, bool) {
    let local = zone::local_s(place, until as i64);
    let today = zone::local_s(place, now as i64).div_euclid(86_400);
    let day = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400);
    let clock = format!("{:02}:{:02}", secs / 3600, (secs % 3600) / 60);
    if day == today {
        (clock, true)
    } else {
        let name = DAYS[zone::weekday(day).rem_euclid(7) as usize];
        (format!("{} {clock}", &name[..3]), false)
    }
}

pub fn back_part(place: &Zone, until: u64, known: bool, now: u64) -> String {
    if !known {
        return "back time not known, Nixa checks again in 1 hour".into();
    }
    format!("back at {}", day_part(place, until, now).0)
}

pub fn when_word(place: &Zone, until: u64, known: bool, now: u64) -> String {
    if !known {
        return "later".into();
    }
    let (text, today) = day_part(place, until, now);
    if today {
        return text;
    }
    let local = zone::local_s(place, until as i64);
    DAYS[zone::weekday(local.div_euclid(86_400)).rem_euclid(7) as usize].to_string()
}

pub fn chat_line(engine: &str, back: &str, moved: &Moved) -> String {
    let head = format!("{engine} hit its usage limit ({back}).");
    match moved {
        Moved::Nothing => head,
        Moved::Moved { title, to_title } => format!("{head} Task {title} moved to {to_title}."),
        Moved::Waiting { title } => format!("{head} No other agent is free, so task {title} waits."),
        Moved::Failed { title, error } => format!("{head} Task {title} was not moved: {error}."),
    }
}

pub fn handoff_note(name: &str, engine: &str, terminal: &str, back: &str, said: &str, branch: &str, folder: &str) -> String {
    let mut out = format!("{}{name} ({engine}) in terminal {terminal}: it hit its usage limit ({back}), so Nixa moved the task.", crate::queue::prompt::HANDOFF_PREFIX);
    if !said.trim().is_empty() {
        out.push_str(&format!(" It said last: \"{}\".", crate::util::clip_flat(said, 300)));
    }
    if !branch.trim().is_empty() {
        out.push_str(&format!(" Branch: {branch}."));
    }
    if !folder.trim().is_empty() {
        out.push_str(&format!(" Folder: {folder}."));
    }
    out
}

pub fn seed(text: &str) -> usize {
    text.bytes().fold(7usize, |a, b| a.wrapping_mul(31).wrapping_add(b as usize))
}

fn fill(template: &str, name: &str, to: &str, when: &str) -> String {
    template.replace("{name}", name).replace("{to}", to).replace("{when}", when)
}

pub fn fallback_leave(name: &str, to: &str, when: &str, seed: usize) -> String {
    if to.trim().is_empty() {
        fill(ALONE_LINES[seed % ALONE_LINES.len()], name, to, when)
    } else {
        fill(LEAVE_LINES[seed % LEAVE_LINES.len()], name, to, when)
    }
}

pub fn fallback_back(name: &str, seed: usize) -> String {
    fill(BACK_LINES[seed % BACK_LINES.len()], name, "", "")
}

pub fn good_line(line: &str, names: &[&str]) -> Option<String> {
    let line = crate::util::flat(line);
    if line.is_empty() || line.chars().count() > LINE_MAX {
        return None;
    }
    let low = line.to_lowercase();
    names.iter().filter(|n| !n.trim().is_empty()).all(|n| low.contains(&n.to_lowercase())).then_some(line)
}

pub fn ask_body(model: &str, ask: &str) -> Value {
    json!({
        "model": model,
        "stream": false,
        "think": false,
        "keep_alive": "2m",
        "format": {"type": "object", "properties": {"line": {"type": "string"}}, "required": ["line"]},
        "options": {"temperature": 0.9, "num_predict": 80},
        "messages": [
            {"role": "system", "content": "You write one short, light, friendly office joke line in plain simple English, at most 18 words. An AI coworker ran out of its usage limit, so the joke says they went on leave or holiday. Keep every name exactly. No numbers that were not given. Answer with JSON only."},
            {"role": "user", "content": ask}
        ]
    })
}

pub fn leave_ask(name: &str, to: &str, when: &str) -> String {
    if to.trim().is_empty() {
        format!("{name} is on leave till {when}. Nobody else is free, so the work waits. Write the line.")
    } else {
        format!("{name} is on leave till {when}. {to} takes over the work. Write the line, it must name {name} and {to}.")
    }
}

pub fn back_ask(name: &str) -> String {
    format!("{name} is back from leave and ready to work again. Write the line, it must name {name}.")
}
