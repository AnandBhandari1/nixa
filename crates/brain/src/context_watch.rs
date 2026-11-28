use crate::app::App;
use crate::engine;
use nixa_protocol::Status;
use regex::Regex;
use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

pub const WARN_PCT: u8 = 70;
pub const REARM_PCT: u8 = 50;
pub const READ_LINES: usize = 15;

static LEFT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(\d{1,3})%\s+context\s+left\b|context\s+left\s+until\s+auto-compact:\s*(\d{1,3})%").expect("context left pattern")
});
static USED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bcontext(?:\s+window)?:?\s*(\d{1,3})%\s*(?:used|full)\b|\b(\d{1,3})%\s+(?:of\s+)?context\s+used\b").expect("context used pattern")
});

fn number(c: &regex::Captures) -> Option<u8> {
    c.iter().skip(1).flatten().next().and_then(|m| m.as_str().parse::<u8>().ok()).filter(|n| *n <= 100)
}

pub fn used_pct(screen: &str) -> Option<u8> {
    let left = LEFT.captures_iter(screen).filter_map(|c| Some((c.get(0)?.start(), 100 - number(&c)?)));
    let used = USED.captures_iter(screen).filter_map(|c| Some((c.get(0)?.start(), number(&c)?)));
    left.chain(used).max_by_key(|(at, _)| *at).map(|(_, pct)| pct)
}

pub fn decide(warned: &mut HashSet<String>, id: &str, used: Option<u8>, blocked: bool) -> bool {
    match used {
        Some(pct) if pct < REARM_PCT => {
            warned.remove(id);
            false
        }
        Some(pct) if pct >= WARN_PCT && !blocked => warned.insert(id.to_string()),
        _ => false,
    }
}

pub fn warn_text(pct: u8) -> String {
    format!("Nixa: your context is {pct}% full. Finish the current step, then run /compact, or write a short handover (what is done, what is left, which files) and report it to your parent.")
}

pub fn deep_warn_text(pct: u8, seat: &str) -> String {
    format!("Nixa: your context is {pct}% full. First finish the deep brain request you are on and answer it with nixa deep decide. Then run /compact. Then run nixa deep join --as {seat} again, so Nixa sends you the full facts again.")
}

pub fn text_for(pct: u8, deep_seat: Option<&str>) -> String {
    match deep_seat {
        Some(seat) => deep_warn_text(pct, seat),
        None => warn_text(pct),
    }
}

pub async fn tick(app: &Arc<App>, warned: &mut HashSet<String>) {
    let terms: Vec<_> = app.terminals().into_iter().filter(|s| s.info.running).map(|s| s.info).collect();
    warned.retain(|id| terms.iter().any(|t| t.id.as_str() == id));
    for info in terms {
        let key = info.id.to_string();
        let Ok(screen) = engine::read(&info.id, Some(READ_LINES), false).await else { continue };
        let used = used_pct(&screen);
        if decide(warned, &key, used, info.status == Some(Status::Blocked))
            && let Some(pct) = used
        {
            let seat = deep_seat(app, &key);
            crate::util::log("context", &format!("{key} is at {pct}% context, told it to compact"));
            if let Err(error) = engine::send(&info.id, &text_for(pct, seat.as_deref()), true, false).await {
                crate::util::log("context", &format!("could not warn {key}: {error}"));
                warned.remove(&key);
            }
        }
    }
}

fn deep_seat(app: &App, id: &str) -> Option<String> {
    crate::app::lock(&app.deep).by_terminal(id).map(|s| s.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_context_line_of_each_agent() {
        assert_eq!(used_pct("› Ask Codex to do anything\n\n  26% context left · ? for shortcuts"), Some(74));
        assert_eq!(used_pct("Context left until auto-compact: 28%"), Some(72));
        assert_eq!(used_pct("gemini-2.5-pro (31% context left)"), Some(69));
        assert_eq!(used_pct("Context: 81% used"), Some(81));
        assert_eq!(used_pct("90% context left\nlater\n20% context left"), Some(80));
        assert_eq!(used_pct("all tests pass, 100% coverage"), None);
        assert_eq!(used_pct("140% context left"), None);
    }

    #[test]
    fn warns_once_at_seventy_and_again_after_a_compact() {
        let mut warned = HashSet::new();
        assert!(!decide(&mut warned, "m2", Some(69), false));
        assert!(!decide(&mut warned, "m2", None, false));
        assert!(!decide(&mut warned, "m2", Some(72), true));
        assert!(decide(&mut warned, "m2", Some(70), false));
        assert!(!decide(&mut warned, "m2", Some(74), false));
        assert!(!decide(&mut warned, "m2", Some(55), false));
        assert!(!decide(&mut warned, "m2", Some(80), false));
        assert!(!decide(&mut warned, "m2", Some(40), false));
        assert!(decide(&mut warned, "m2", Some(75), false));
        assert!(decide(&mut warned, "m3", Some(90), false));
    }

    #[test]
    fn a_deep_seat_is_told_to_finish_compact_and_join_again() {
        let text = text_for(72, Some("claude"));
        assert!(text.starts_with("Nixa: your context is 72% full."));
        assert!(text.contains("finish the deep brain request") && text.contains("nixa deep decide"));
        let compact = text.find("/compact").unwrap();
        let join = text.find("nixa deep join --as claude").unwrap();
        assert!(text.find("nixa deep decide").unwrap() < compact && compact < join);
        assert_eq!(text_for(72, None), warn_text(72));
    }

    #[test]
    fn a_deep_seat_terminal_is_found_and_join_resets_its_facts() {
        let dir = tempfile::tempdir().unwrap();
        let mut deep = crate::deep::Deep::open(dir.path());
        let space = crate::spaces::Space { id: "s1".into(), name: "shop".into(), folder: "/tmp".into() };
        deep.join("m9", "Claude", &space, Some("claude"), 1000).unwrap();
        let seat = deep.by_terminal("m9").map(|s| s.id.clone());
        assert_eq!(seat.as_deref(), Some("claude"));
        assert!(deep.by_terminal("m8").is_none());
        let facts = dir.path().join("facts-sent.json");
        std::fs::write(&facts, r#"{"m9": ["Decision: automatic"]}"#).unwrap();
        let mut again = crate::deep::Deep::open(dir.path());
        again.join("m9", "Claude", &space, Some("claude"), 2000).unwrap();
        let left: std::collections::BTreeMap<String, Vec<String>> = crate::util::read_json(&facts).unwrap();
        assert!(!left.contains_key("m9"));
    }

    #[test]
    fn the_warning_says_compact_or_hand_over() {
        let text = warn_text(74);
        assert!(text.starts_with("Nixa: your context is 74% full."));
        assert!(text.contains("/compact") && text.contains("handover"));
    }
}
