use gpui::Hsla;
use nixa_ui::Theme;
use serde::Deserialize;
use serde_json::Value;

const DAY_MS: i64 = 86_400_000;

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BrainContext {
    pub level: String,
    pub used: u64,
    pub budget: u64,
    pub left: f64,
    pub estimate: bool,
    pub compact_at: u64,
    pub compacting: bool,
    pub count: u64,
    pub last_at: Option<u64>,
    pub last_before: Option<u64>,
}

pub fn keep_known(old: Option<BrainContext>, new: Option<BrainContext>) -> Option<BrainContext> {
    let new_known = new.as_ref().is_some_and(|c| !unknown(c) || c.compacting);
    let old_known = old.as_ref().is_some_and(|c| !unknown(c));
    if new_known || !old_known { new } else { old }
}

pub fn from_pct(old: Option<BrainContext>, used_pct: u8) -> BrainContext {
    let mut c = old.unwrap_or_default();
    let used = used_pct.min(100);
    c.left = 100.0 - used as f64;
    c.level = if used >= 90 {
        "full"
    } else if used >= 75 {
        "large"
    } else {
        "ok"
    }
    .to_string();
    c.compacting = false;
    if c.budget > 0 {
        c.used = c.budget * used as u64 / 100;
    }
    c
}

pub fn parse(value: &Value) -> Option<BrainContext> {
    let view = value.get("view").unwrap_or(value);
    if !view.is_object() {
        return None;
    }
    serde_json::from_value(view.clone()).ok()
}

fn unknown(c: &BrainContext) -> bool {
    c.level.is_empty() || c.level == "unknown"
}

pub fn token_words(count: u64) -> String {
    let scaled = |value: f64, unit: &str| {
        if value < 10.0 {
            let tenths = (value * 10.0).round() / 10.0;
            if tenths.fract() == 0.0 {
                format!("{}{unit}", tenths as u64)
            } else {
                format!("{tenths:.1}{unit}")
            }
        } else {
            format!("{}{unit}", value.round() as u64)
        }
    };
    if count < 1000 {
        count.to_string()
    } else if count < 999_500 {
        scaled(count as f64 / 1000.0, "k")
    } else {
        scaled(count as f64 / 1_000_000.0, "M")
    }
}

pub fn clock(at: u64, offset_min: i32) -> String {
    let local = (at as i64 + offset_min as i64 * 60_000).rem_euclid(DAY_MS);
    let minutes = local / 60_000;
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

pub fn words(context: Option<&BrainContext>) -> String {
    let Some(c) = context else {
        return String::new();
    };
    if c.compacting {
        return "compacting".into();
    }
    if unknown(c) || !c.left.is_finite() {
        return String::new();
    }
    format!("{}%", c.left.round() as i64)
}

pub fn tip(context: Option<&BrainContext>, name: &str, offset_min: i32) -> String {
    let Some(c) = context else {
        return String::new();
    };
    let last = match c.last_at.filter(|at| *at > 0) {
        Some(at) if c.count > 0 => {
            let times = if c.count == 1 { "time" } else { "times" };
            let before = c
                .last_before
                .filter(|b| *b > 0)
                .map(|b| format!(" (it was {})", token_words(b)))
                .unwrap_or_default();
            format!(
                " Compacted {} {times} in this chat, last at {}{before}.",
                c.count,
                clock(at, offset_min)
            )
        }
        _ => " Not compacted yet.".into(),
    };
    if c.compacting {
        return format!(
            "Brain context: {name} is compacting it now. The older chat is folded into a summary and the recent turns are kept.{last}"
        );
    }
    if unknown(c) {
        return format!(
            "Brain context: not measured yet. It shows after the next brain reply.{last}"
        );
    }
    let at = if c.compact_at > 0 { c.compact_at } else { 75 };
    format!(
        "Brain context: {}{} of {} used, {}% left. {name} compacts it by itself after a brain reply at {at}% used.{last}",
        if c.estimate { "about " } else { "" },
        token_words(c.used),
        token_words(c.budget),
        c.left.round() as i64
    )
}

pub fn color(context: Option<&BrainContext>, t: &Theme) -> Hsla {
    match context {
        None => t.muted,
        Some(c) if c.compacting => t.blue,
        Some(c) if c.level == "full" => t.red,
        Some(c) if c.level == "large" => t.yellow,
        Some(_) => t.muted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn view(v: Value) -> BrainContext {
        parse(&json!({"ok": true, "view": v})).unwrap()
    }

    #[test]
    fn tokens_read_short() {
        assert_eq!(token_words(950), "950");
        assert_eq!(token_words(1000), "1k");
        assert_eq!(token_words(1450), "1.5k");
        assert_eq!(token_words(12_400), "12k");
        assert_eq!(token_words(999_499), "999k");
        assert_eq!(token_words(999_500), "1M");
        assert_eq!(token_words(2_340_000), "2.3M");
    }

    #[test]
    fn clock_uses_the_offset() {
        assert_eq!(clock(0, 0), "00:00");
        assert_eq!(clock(0, 480), "08:00");
        assert_eq!(clock(90 * 60_000, -120), "23:30");
    }

    #[test]
    fn words_show_percent_or_compacting() {
        assert_eq!(words(None), "");
        assert_eq!(
            words(Some(&view(json!({"level": "unknown", "left": 100})))),
            ""
        );
        assert_eq!(
            words(Some(&view(json!({"level": "ok", "left": 62.4})))),
            "62%"
        );
        assert_eq!(
            words(Some(&view(json!({"level": "ok", "compacting": true})))),
            "compacting"
        );
    }

    #[test]
    fn tips_say_what_happened() {
        let c = view(
            json!({"level": "large", "used": 81_000, "budget": 100_000, "left": 19, "estimate": true, "compactAt": 75, "count": 2, "lastAt": 3_600_000, "lastBefore": 90_000}),
        );
        assert_eq!(
            tip(Some(&c), "Nixa", 0),
            "Brain context: about 81k of 100k used, 19% left. Nixa compacts it by itself after a brain reply at 75% used. Compacted 2 times in this chat, last at 01:00 (it was 90k)."
        );
        let first = view(json!({"level": "unknown"}));
        assert_eq!(
            tip(Some(&first), "Nixa", 0),
            "Brain context: not measured yet. It shows after the next brain reply. Not compacted yet."
        );
        let busy = view(json!({"level": "ok", "compacting": true, "count": 1, "lastAt": 60_000}));
        assert!(
            tip(Some(&busy), "Nixa", 0).starts_with("Brain context: Nixa is compacting it now.")
        );
        assert!(
            tip(Some(&busy), "Nixa", 0).ends_with("Compacted 1 time in this chat, last at 00:01.")
        );
        assert_eq!(tip(None, "Nixa", 0), "");
    }

    #[test]
    fn colors_follow_the_level() {
        let t = nixa_ui::theme::nixa_dark();
        assert_eq!(color(None, &t), t.muted);
        assert_eq!(color(Some(&view(json!({"level": "full"}))), &t), t.red);
        assert_eq!(color(Some(&view(json!({"level": "large"}))), &t), t.yellow);
        assert_eq!(
            color(
                Some(&view(json!({"level": "full", "compacting": true}))),
                &t
            ),
            t.blue
        );
        assert_eq!(color(Some(&view(json!({"level": "ok"}))), &t), t.muted);
    }

    #[test]
    fn parse_takes_the_view_or_the_object() {
        assert_eq!(parse(&json!({"level": "ok", "left": 5})).unwrap().left, 5.0);
        assert!(parse(&json!("x")).is_none());
    }
}
