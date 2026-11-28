use gpui::{App, AppContext, Entity, Global};
use serde_json::Value;

pub const MAX_CHARS: usize = 80;
pub const THINKING: &str = "thinking";

#[derive(Default)]
pub struct Progress {
    pub text: String,
}

struct Hub(Entity<Progress>);

impl Global for Hub {}

pub fn hub(cx: &mut App) -> Entity<Progress> {
    if let Some(h) = cx.try_global::<Hub>() {
        return h.0.clone();
    }
    let h = cx.new(|_| Progress::default());
    cx.set_global(Hub(h.clone()));
    h
}

pub fn current(cx: &App) -> String {
    cx.try_global::<Hub>().map(|h| h.0.read(cx).text.clone()).unwrap_or_default()
}

fn clean(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= MAX_CHARS {
        return flat;
    }
    let cut: String = flat.chars().take(MAX_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
}

pub fn next(text: &str, msg: &Value) -> Option<String> {
    match msg.get("type").and_then(Value::as_str)? {
        "progress" => Some(clean(msg.get("text").and_then(Value::as_str).unwrap_or_default())),
        "talk" => msg.get("progress").map(|p| clean(p.as_str().unwrap_or_default())),
        "busy" => (msg.get("busy").and_then(Value::as_bool) == Some(false)).then(String::new),
        "reply" | "chat_cleaned" | "interrupted" => Some(String::new()),
        _ => None,
    }
    .filter(|fresh| fresh != text)
}

pub fn feed(msg: &Value, cx: &mut App) -> bool {
    let h = hub(cx);
    h.update(cx, |h, cx| match next(&h.text, msg) {
        Some(fresh) => {
            h.text = fresh;
            cx.notify();
            true
        }
        None => false,
    })
}

pub fn word(base: &str, progress: &str) -> String {
    if base == THINKING && !progress.is_empty() {
        format!("{THINKING} · {progress}")
    } else {
        base.to_string()
    }
}

pub fn status_word(base: String, cx: &App) -> String {
    word(&base, &current(cx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use serde_json::json;

    #[test]
    fn progress_lines_come_and_go() {
        assert_eq!(next("", &json!({"type": "progress", "text": "  Reading   the tests  "})).as_deref(), Some("Reading the tests"));
        assert_eq!(next("Reading the tests", &json!({"type": "progress", "text": "Reading the tests"})), None);
        assert_eq!(next("a", &json!({"type": "talk", "progress": "Checking m3"})).as_deref(), Some("Checking m3"));
        assert_eq!(next("a", &json!({"type": "talk", "progress": null})).as_deref(), Some(""));
        assert_eq!(next("a", &json!({"type": "talk", "presence": {}})), None);
        assert_eq!(next("a", &json!({"type": "busy", "busy": true})), None);
        assert_eq!(next("a", &json!({"type": "busy", "busy": false})).as_deref(), Some(""));
        assert_eq!(next("a", &json!({"type": "reply", "text": "done"})).as_deref(), Some(""));
        assert_eq!(next("a", &json!({"type": "chat"})), None);
        let long = "x".repeat(200);
        assert_eq!(next("", &json!({"type": "progress", "text": long})).map(|s| s.chars().count()), Some(MAX_CHARS));
        assert_eq!(word("thinking", "Reading"), "thinking · Reading");
        assert_eq!(word("thinking", ""), "thinking");
        assert_eq!(word("speaking", "Reading"), "speaking");
    }

    #[gpui::test]
    fn the_hub_keeps_the_last_line(cx: &mut TestAppContext) {
        cx.update(|cx| {
            assert_eq!(current(cx), "");
            assert!(feed(&json!({"type": "progress", "text": "Looking at terminal 2"}), cx));
            assert!(!feed(&json!({"type": "progress", "text": "Looking at terminal 2"}), cx));
            assert_eq!(status_word("thinking".into(), cx), "thinking · Looking at terminal 2");
            assert!(feed(&json!({"type": "busy", "busy": false}), cx));
            assert_eq!(status_word("thinking".into(), cx), "thinking");
        });
    }
}
