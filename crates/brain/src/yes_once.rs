use crate::app::{App, lock};
use crate::decision::normalise;

pub const YES_WORDS: [&str; 16] = ["yes", "yeah", "yep", "yup", "ok", "okay", "sure", "allow", "approve", "approved", "go", "run", "do", "fine", "alright", "confirm"];
pub const NO_WORDS: [&str; 12] = ["no", "not", "don't", "dont", "never", "stop", "wait", "cancel", "hold", "later", "nope", "isn't"];

pub fn says_yes_to(line: &str, names: &[&str]) -> bool {
    if line.trim().ends_with('?') {
        return false;
    }
    let text = normalise(line);
    let words: Vec<&str> = text.split(' ').filter(|w| !w.is_empty()).collect();
    let Some(first) = words.first() else { return false };
    if !YES_WORDS.contains(first) || words.iter().any(|w| NO_WORDS.contains(w)) {
        return false;
    }
    let padded = format!(" {text} ");
    names.iter().map(|n| normalise(n)).filter(|n| !n.is_empty()).any(|n| padded.contains(&format!(" {n} ")))
}

pub fn fresh_line(app: &App, card_at: u64, now: u64, limit_ms: u64) -> Option<(String, u64)> {
    let (last, used) = {
        let brain = lock(&app.brain);
        (brain.last_user.clone(), brain.approval_used.clone())
    };
    let (line, at) = last?;
    if at <= card_at || now.saturating_sub(at) > limit_ms {
        return None;
    }
    if used.as_ref().is_some_and(|(when, _)| *when == at) {
        return None;
    }
    Some((line, at))
}

pub fn chat_yes(app: &App, card_at: u64, now: u64, limit_ms: u64, names: &[&str]) -> Option<u64> {
    let mut brain = lock(&app.brain);
    let (line, at) = brain.last_user.clone()?;
    if at <= card_at || now.saturating_sub(at) > limit_ms || brain.approval_used.as_ref().is_some_and(|(when, _)| *when == at) {
        return None;
    }
    if !says_yes_to(&line, names) {
        return None;
    }
    brain.approval_used = Some((at, "chat yes".into()));
    Some(at)
}

pub fn use_yes(app: &App, at: u64, what: impl Into<String>) {
    lock(&app.brain).approval_used = Some((at, what.into()));
}

pub type HeldAsk = std::sync::Mutex<Option<(String, u64)>>;

pub fn approved_once(app: &App, held: &HeldAsk, key: &str, words: &[&str], what: &str) -> bool {
    let now = crate::util::now_ms();
    let limit = crate::decision::YES_MS;
    let mut slot = held.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, at)) = slot.as_ref().filter(|(k, at)| k == key && now.saturating_sub(*at) <= limit)
        && let Some(yes) = chat_yes(app, *at, now, limit, words)
    {
        *slot = None;
        use_yes(app, yes, what);
        return true;
    }
    if slot.as_ref().is_none_or(|(k, at)| k != key || now.saturating_sub(*at) > limit) {
        *slot = Some((key.to_string(), now));
        app.info(&format!("Held for you: {what}. Say \"yes {}\" to allow it once.", words.first().copied().unwrap_or("go")));
    }
    false
}

pub fn held_note(what: &str, word: &str) -> String {
    format!("Held for the user's yes: {what}. Tell the user in one sentence what waits. A plain yes does not count: the user says \"yes {word}\" in chat. Then call this tool again with the same input. Nothing ran.")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> (tempfile::TempDir, std::sync::Arc<App>) {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        (dir, app)
    }

    #[test]
    fn a_yes_counts_only_when_it_names_the_step() {
        assert!(says_yes_to("yes, click it", &["click"]));
        assert!(says_yes_to("OK run mail", &["mail"]));
        assert!(says_yes_to("yes push", &["push"]));
        assert!(!says_yes_to("yes", &["click"]));
        assert!(!says_yes_to("go ahead", &["push"]));
        assert!(!says_yes_to("yes please", &["mail"]));
        assert!(!says_yes_to("yes, change the code", &["click"]));
        assert!(!says_yes_to("click it", &["click"]));
        assert!(!says_yes_to("yes click?", &["click"]));
        assert!(!says_yes_to("yes but don't click", &["click"]));
        assert!(!says_yes_to("ok, not the click yet", &["click"]));
        assert!(!says_yes_to("yes clicker", &["click"]));
        assert!(!says_yes_to("yes click", &[""]));
    }

    #[test]
    fn a_chat_yes_is_fresh_after_the_card_and_used_once() {
        let (_dir, app) = app();
        lock(&app.brain).last_user = Some(("yes click".into(), 1000));
        assert_eq!(chat_yes(&app, 1000, 1500, 5000, &["click"]), None);
        assert_eq!(chat_yes(&app, 900, 1000 + 5001, 5000, &["click"]), None);
        assert_eq!(chat_yes(&app, 900, 1500, 5000, &["type"]), None);
        assert_eq!(chat_yes(&app, 900, 1500, 5000, &["click"]), Some(1000));
        use_yes(&app, 1000, "desktop d1");
        assert_eq!(chat_yes(&app, 900, 1500, 5000, &["click"]), None);
        lock(&app.brain).last_user = Some(("yes click".into(), 1200));
        assert_eq!(chat_yes(&app, 900, 1500, 5000, &["click"]), Some(1200));
    }

    #[test]
    fn one_yes_is_taken_by_only_one_tool() {
        let (_dir, app) = app();
        let now = crate::util::now_ms();
        let push: HeldAsk = std::sync::Mutex::new(Some(("repo dev".into(), now - 10)));
        let photo: HeldAsk = std::sync::Mutex::new(Some(("phone".into(), now - 10)));
        lock(&app.brain).last_user = Some(("yes push and photo".into(), now - 5));
        let first = std::thread::scope(|s| {
            let a = s.spawn(|| approved_once(&app, &push, "repo dev", &["push"], "push dev"));
            let b = s.spawn(|| approved_once(&app, &photo, "phone", &["photo"], "a photo"));
            [a.join().unwrap(), b.join().unwrap()]
        });
        assert_eq!(first.iter().filter(|ok| **ok).count(), 1, "{first:?}");
        assert!(!approved_once(&app, &push, "repo dev", &["push"], "push dev"));
        assert!(!approved_once(&app, &photo, "phone", &["photo"], "a photo"));
    }
}
