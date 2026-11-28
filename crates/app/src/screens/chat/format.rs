use super::kinds::{is_sent, item_label};
use super::model::{Item, Snapshot, Usage};

pub const CONFIRM_CHARS: usize = 240;
pub const WHY_CHARS: usize = 160;
const DAY_MS: i64 = 86_400_000;
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub fn local_offset_min() -> i32 {
    std::process::Command::new("/bin/date")
        .arg("+%z")
        .output()
        .ok()
        .and_then(|o| parse_offset(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or(0)
}

pub fn parse_offset(text: &str) -> Option<i32> {
    let text = text.trim();
    let (sign, rest) = match text.as_bytes().first()? {
        b'+' => (1, &text[1..]),
        b'-' => (-1, &text[1..]),
        _ => return None,
    };
    if rest.len() != 4 || !rest.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let hours: i32 = rest[..2].parse().ok()?;
    let minutes: i32 = rest[2..].parse().ok()?;
    Some(sign * (hours * 60 + minutes))
}

fn local_ms(at: u64, offset_min: i32) -> i64 {
    at as i64 + offset_min as i64 * 60_000
}

fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

pub fn clock(at: u64, offset_min: i32) -> String {
    let ms = local_ms(at, offset_min).rem_euclid(DAY_MS);
    let minutes = ms / 60_000;
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

pub fn stamp(at: u64, now: u64, offset_min: i32) -> String {
    let day = local_ms(at, offset_min).div_euclid(DAY_MS);
    let today = local_ms(now, offset_min).div_euclid(DAY_MS);
    if day == today {
        return clock(at, offset_min);
    }
    let (_, month, date) = civil(day);
    format!(
        "{date} {} {}",
        MONTHS[(month - 1) as usize],
        clock(at, offset_min)
    )
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn sender(item: &Item) -> String {
    match item.who.as_str() {
        "user" => "You".into(),
        "agent" if is_sent(item) => item
            .to
            .clone()
            .or_else(|| item.label.clone())
            .map(|l| format!("to {l}"))
            .unwrap_or_default(),
        "agent" => item.label.clone().unwrap_or_default(),
        _ => String::new(),
    }
}

pub fn foot_text(item: &Item, name: &str, now: u64, offset_min: i32) -> String {
    let meta = if item.who == "nixa" {
        item.meta.clone().unwrap_or_default()
    } else {
        String::new()
    };
    let quiet = if item.quiet && item.who == "user" {
        format!("not for {name}")
    } else {
        String::new()
    };
    [
        item_label(item, name),
        sender(item),
        stamp(item.at, now, offset_min),
        meta,
        quiet,
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join(" · ")
}

fn scaled(value: f64, unit: &str) -> String {
    if value < 10.0 {
        let tenth = (value * 10.0).round() / 10.0;
        if tenth.fract() == 0.0 {
            format!("{}{unit}", tenth as u64)
        } else {
            format!("{tenth}{unit}")
        }
    } else {
        format!("{}{unit}", value.round() as u64)
    }
}

pub fn tokens(count: u64) -> String {
    if count < 1000 {
        count.to_string()
    } else if count < 999_500 {
        scaled(count as f64 / 1000.0, "k")
    } else {
        scaled(count as f64 / 1_000_000.0, "M")
    }
}

pub fn turn(ms: u64, count: u64) -> String {
    let seconds = if ms < 10_000 {
        format!("{:.1}", ms as f64 / 1000.0)
    } else {
        (((ms as f64) / 1000.0).round() as u64).to_string()
    };
    format!("{seconds} s, {}", tokens(count))
}

fn plural(n: u64) -> &'static str {
    if n == 1 { "" } else { "s" }
}

pub fn usage_words(usage: &Usage) -> String {
    let mut words = format!(
        "today {} · {} turn{}",
        tokens(usage.tokens),
        usage.turns,
        plural(usage.turns)
    );
    if let Some(pct) = usage.context {
        words.push_str(&format!(" · context {pct}%"));
    }
    words
}

pub fn usage_tip(usage: &Usage) -> String {
    let mut lines = vec![format!(
        "Today: {} tokens, {} cached, {} turn{}.",
        tokens(usage.tokens),
        tokens(usage.cached),
        usage.turns,
        plural(usage.turns)
    )];
    if let Some((ms, count)) = usage.last {
        lines.push(format!("Last turn: {}", turn(ms, count)));
    }
    if !usage.model.is_empty() || usage.input + usage.output > 0 {
        let model = if usage.model.is_empty() {
            String::new()
        } else {
            format!(" on {}", usage.model)
        };
        lines.push(format!(
            "Last reply{model}: {} in, {} out.",
            tokens(usage.input),
            tokens(usage.output)
        ));
    }
    if let Some(pct) = usage.context {
        lines.push(format!("Context window: {pct}% used."));
    }
    lines.join("\n")
}

pub fn clip(text: &str, max: usize) -> String {
    if text.chars().count() > max {
        format!("{}…", text.chars().take(max).collect::<String>())
    } else {
        text.to_string()
    }
}

pub fn brain_label(snap: &Snapshot) -> String {
    match &snap.brain {
        Some(b) if !b.name.is_empty() => b.name.clone(),
        Some(b) if !b.kind.is_empty() => b.kind.clone(),
        _ => "No brain".into(),
    }
}

pub fn short_model(snap: &Snapshot) -> String {
    let Some(brain) = &snap.brain else {
        return "Pick a brain".into();
    };
    if !brain.model_name.is_empty() {
        return brain.model_name.clone();
    }
    if !brain.model.is_empty() {
        return brain
            .model
            .rsplit('/')
            .next()
            .unwrap_or(&brain.model)
            .to_string();
    }
    if brain.kind.starts_with("api:") || ["openai", "anthropic", "ollama"].contains(&brain.kind.as_str()) {
        return "Pick a model".into();
    }
    brain_label(snap)
}

pub fn model_name(snap: &Snapshot) -> String {
    let model = snap
        .brain
        .as_ref()
        .map(|b| {
            if b.model_name.is_empty() {
                b.model.clone()
            } else {
                b.model_name.clone()
            }
        })
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| "unknown".into());
    format!("{} · {model}", brain_label(snap))
}

pub fn hint(snap: &Snapshot) -> String {
    let name = snap.name();
    if snap.key_listen == "starting" {
        return "Starting the mic. Talk after the tink sound.".into();
    }
    if snap.key_listen == "on" {
        return format!("{name} is listening. Tap Right Command again to send.");
    }
    let hotkey = snap.listen.as_deref().is_some_and(|l| l != "always" && l != "wake");
    if snap.mic_off && !hotkey {
        return if snap.voice_off {
            "Mic and voice are off. Type to chat. Replies show here.".into()
        } else {
            format!("Mic is off. Type to chat. {name} still speaks.")
        };
    }
    if snap.mic_ready != Some(true) && !hotkey {
        return "Mic is not ready. You can type.".into();
    }
    let brain_ready = snap.brain_ready.unwrap_or(snap.brain.is_some());
    if !brain_ready && snap.brain.is_none() {
        return "No brain is set yet. Click the model chip to pick one.".into();
    }
    if !brain_ready {
        return format!(
            "{} brain is not ready. {name} tries again by itself.",
            brain_label(snap)
        );
    }
    if hotkey {
        return format!("Tap Right Command (or F9) to talk to {name}. Tap it again to send. Or type.");
    }
    if snap.voice_off {
        return "Voice is off. Talk or type. Replies show here.".into();
    }
    if snap.echo_cancel {
        return format!("Talk any time. {name} stops when you speak.");
    }
    format!("{name} stops listening while it speaks.")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Blue,
    Red,
    Green,
    Yellow,
    Magenta,
    Cyan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Ring,
    Spin,
    Pulse,
    Hollow,
}

pub fn status(snap: &Snapshot, mobile: bool, sending: bool) -> (String, Tone, Mark) {
    match snap.state.as_str() {
        "quiet" => return ("voice is off".into(), Tone::Yellow, Mark::Hollow),
        "paused" => return ("paused".into(), Tone::Magenta, Mark::Hollow),
        "off" => return ("off".into(), Tone::Red, Mark::Hollow),
        _ => {}
    }
    if sending {
        return ("sending".into(), Tone::Magenta, Mark::Spin);
    }
    if snap.state == "speaking" {
        return ("speaking".into(), Tone::Green, Mark::Pulse);
    }
    if snap.state == "thinking" || snap.busy {
        let word = if snap.progress.is_empty() {
            "thinking".to_string()
        } else {
            format!("thinking · {}", snap.progress)
        };
        return (word, Tone::Magenta, Mark::Spin);
    }
    if mobile {
        return if snap.hearing {
            ("hearing you on the phone".into(), Tone::Blue, Mark::Pulse)
        } else {
            ("phone mode · Mac mic off".into(), Tone::Blue, Mark::Hollow)
        };
    }
    if snap.hearing {
        return ("listening".into(), Tone::Cyan, Mark::Ring);
    }
    if snap.listen.as_deref().is_some_and(|l| l != "always" && l != "wake") {
        if snap.key_listen == "on" {
            return ("listening · Right ⌘ to send".into(), Tone::Cyan, Mark::Ring);
        }
        let word = if snap.voice_off { "Right ⌘ ready · voice off" } else { "Right ⌘ ready" };
        return (word.into(), Tone::Cyan, Mark::Hollow);
    }
    if snap.mic_off {
        let word = if snap.voice_off {
            "mic and voice off"
        } else {
            "mic off"
        };
        return (word.into(), Tone::Yellow, Mark::Hollow);
    }
    let word = if snap.mic_ready == Some(true) {
        "mic waits for quiet"
    } else {
        "mic is off"
    };
    (word.into(), Tone::Magenta, Mark::Hollow)
}

pub fn busy(snap: &Snapshot) -> bool {
    snap.busy || snap.state == "speaking" || snap.state == "thinking"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::chat::model::Brain;

    const DAY: u64 = 86_400_000;

    #[test]
    fn offsets_are_read() {
        assert_eq!(parse_offset("+0800\n"), Some(480));
        assert_eq!(parse_offset("-0530"), Some(-330));
        assert_eq!(parse_offset("0800"), None);
        assert_eq!(parse_offset("+8"), None);
    }

    #[test]
    fn clock_and_stamp() {
        let at = 20_000 * DAY + 13 * 3_600_000 + 5 * 60_000;
        assert_eq!(clock(at, 0), "13:05");
        assert_eq!(clock(at, 480), "21:05");
        assert_eq!(clock(at, -840), "23:05");
        assert_eq!(stamp(at, at + 60_000, 0), "13:05");
        assert_eq!(stamp(at, at + DAY, 0), "4 Oct 13:05");
        assert_eq!(stamp(at, at + 12 * 3_600_000, 660), "00:05");
        assert_eq!(stamp(at, at + 12 * 3_600_000, 0), "4 Oct 13:05");
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(19_723), (2024, 1, 1));
    }

    #[test]
    fn tokens_are_short() {
        assert_eq!(tokens(999), "999");
        assert_eq!(tokens(1000), "1k");
        assert_eq!(tokens(1234), "1.2k");
        assert_eq!(tokens(12_345), "12k");
        assert_eq!(tokens(999_499), "999k");
        assert_eq!(tokens(999_500), "1M");
        assert_eq!(tokens(2_340_000), "2.3M");
        assert_eq!(turn(2_345, 1500), "2.3 s, 1.5k");
        assert_eq!(turn(12_600, 10), "13 s, 10");
    }

    #[test]
    fn usage_lines() {
        let one = Usage {
            tokens: 1200,
            cached: 0,
            turns: 1,
            last: None,
            ..Usage::default()
        };
        assert_eq!(usage_words(&one), "today 1.2k · 1 turn");
        let many = Usage {
            tokens: 5,
            cached: 3,
            turns: 2,
            last: Some((1000, 20)),
            ..Usage::default()
        };
        assert_eq!(usage_words(&many), "today 5 · 2 turns");
        assert_eq!(
            usage_tip(&many),
            "Today: 5 tokens, 3 cached, 2 turns.\nLast turn: 1.0 s, 20"
        );
    }

    #[test]
    fn clip_shortens_confirmation_text() {
        assert_eq!(clip("abcdef", 3), "abc…");
        assert_eq!(clip("abc", 3), "abc");
    }

    #[test]
    fn foot_has_kind_sender_and_time() {
        let at = 20_000 * DAY;
        let user = Item {
            who: "user".into(),
            typed: true,
            at,
            quiet: true,
            ..Item::default()
        };
        assert_eq!(
            foot_text(&user, "Nixa", at, 0),
            "Typed · You · 00:00 · not for Nixa"
        );
        let nixa = Item {
            who: "nixa".into(),
            at,
            meta: Some("2.1 s".into()),
            ..Item::default()
        };
        assert_eq!(foot_text(&nixa, "Juno", at, 0), "Juno · 00:00 · 2.1 s");
        let sent = Item {
            who: "agent".into(),
            at,
            label: Some("Claude".into()),
            kind: Some("sent".into()),
            ..Item::default()
        };
        assert_eq!(foot_text(&sent, "Nixa", at, 0), "Sent · to Claude · 00:00");
        let agent = Item {
            who: "agent".into(),
            at,
            label: Some("Codex".into()),
            ..Item::default()
        };
        assert_eq!(foot_text(&agent, "Nixa", at, 0), "Agent · Codex · 00:00");
    }

    #[test]
    fn hints_follow_the_old_order() {
        let mut snap = Snapshot::default();
        assert_eq!(hint(&snap), "Mic is not ready. You can type.");
        snap.mic_ready = Some(true);
        assert_eq!(
            hint(&snap),
            "No brain is set yet. Click the model chip to pick one."
        );
        snap.brain = Some(Brain {
            name: "Claude".into(),
            ..Brain::default()
        });
        snap.brain_ready = Some(false);
        assert_eq!(
            hint(&snap),
            "Claude brain is not ready. Nixa tries again by itself."
        );
        snap.brain_ready = None;
        snap.brain = Some(Brain {
            name: "Claude".into(),
            ..Brain::default()
        });
        assert_eq!(hint(&snap), "Nixa stops listening while it speaks.");
        snap.listen = Some("hotkey".into());
        assert_eq!(hint(&snap), "Tap Right Command (or F9) to talk to Nixa. Tap it again to send. Or type.");
        snap.mic_off = true;
        assert_eq!(hint(&snap), "Tap Right Command (or F9) to talk to Nixa. Tap it again to send. Or type.");
        snap.mic_off = false;
        snap.listen = Some("always".into());
        snap.voice_off = true;
        assert_eq!(
            hint(&snap),
            "Voice is off. Talk or type. Replies show here."
        );
        snap.mic_off = true;
        assert_eq!(
            hint(&snap),
            "Mic and voice are off. Type to chat. Replies show here."
        );
        snap.key_listen = "on".into();
        assert_eq!(hint(&snap), "Nixa is listening. Tap Right Command again to send.");
    }

    #[test]
    fn status_words() {
        let mut snap = Snapshot::default();
        assert_eq!(status(&snap, false, false).0, "mic is off");
        snap.busy = true;
        assert_eq!(
            status(&snap, false, false),
            ("thinking".into(), Tone::Magenta, Mark::Spin)
        );
        snap.progress = "tool".into();
        assert_eq!(status(&snap, false, false).0, "thinking · tool");
        snap.state = "speaking".into();
        assert_eq!(status(&snap, false, false).2, Mark::Pulse);
        snap.state = "off".into();
        assert_eq!(status(&snap, false, false).1, Tone::Red);
        snap.state.clear();
        snap.busy = false;
        snap.hearing = true;
        assert_eq!(status(&snap, false, false).2, Mark::Ring);
        snap.hearing = false;
        snap.mic_off = true;
        assert_eq!(status(&snap, false, false).0, "mic off");
        snap.listen = Some("hotkey".into());
        assert_eq!(status(&snap, false, false), ("Right ⌘ ready".into(), Tone::Cyan, Mark::Hollow));
        snap.key_listen = "on".into();
        assert_eq!(status(&snap, false, false).0, "listening · Right ⌘ to send");
        snap.key_listen.clear();
        snap.listen = None;
        assert_eq!(
            status(&snap, true, false),
            ("phone mode · Mac mic off".into(), Tone::Blue, Mark::Hollow)
        );
        snap.hearing = true;
        assert_eq!(status(&snap, true, false).0, "hearing you on the phone");
        snap.state = "speaking".into();
        assert_eq!(
            status(&snap, true, true),
            ("sending".into(), Tone::Magenta, Mark::Spin)
        );
    }

    #[test]
    fn model_chip_words() {
        let mut snap = Snapshot::default();
        assert_eq!(short_model(&snap), "Pick a brain");
        snap.brain = Some(Brain {
            name: "OpenRouter".into(),
            kind: "openai".into(),
            ..Brain::default()
        });
        assert_eq!(short_model(&snap), "Pick a model");
        snap.brain.as_mut().unwrap().model = "anthropic/claude-x".into();
        assert_eq!(short_model(&snap), "claude-x");
        assert_eq!(model_name(&snap), "OpenRouter · anthropic/claude-x");
    }
}
