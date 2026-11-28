use super::model::{Level, Notice};
use serde_json::Value;

pub const QUESTION_MAX: usize = 90;
pub const PICK_MAX: usize = 160;

pub fn clip(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let cut: String = flat.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

pub fn deep_notice(ask: &Value) -> Option<Notice> {
    let text = |key: &str| ask.get(key).and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let question = clip(&text("question"), QUESTION_MAX);
    if question.is_empty() {
        return None;
    }
    let space = text("spaceName");
    let place = if space.is_empty() { String::new() } else { format!(" of {space}") };
    match text("state").as_str() {
        "answered" => {
            let pick = clip(&text("pick"), PICK_MAX);
            let why = clip(&text("why"), PICK_MAX);
            let body = match (pick.is_empty(), why.is_empty()) {
                (true, _) => why,
                (false, true) => format!("Pick: {pick}"),
                (false, false) => format!("Pick: {pick}. Why: {why}"),
            };
            Some(Notice::new(Level::Info, format!("The deep brain{place} answered: {question}")).body(body))
        }
        "timeout" => Some(
            Notice::new(Level::Warning, format!("The deep brain{place} did not answer: {question}"))
                .body("Nixa decides it by itself."),
        ),
        _ => None,
    }
}
