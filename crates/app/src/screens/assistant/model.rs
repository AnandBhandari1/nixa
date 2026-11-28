use serde_json::{Value, json};

pub const PLACEHOLDER: &str = "Stock price, weather, or remind me at 5 to call home";
pub const INTRO: &str = "Ask about current facts or your own things. Nixa names the source and tells you when live data is not available.";
pub const NOTICE: &str = "Answers also show in Nixa's chat and voice. Reminders stay in Tasks. Schedules stay in Settings, Schedules.";
pub const EXAMPLES: &str = "Examples: What is the stock price of AAPL? What is the news in AI today? Remind me in 20 minutes to stretch.";
pub const BRIEFING_HINT: &str = "A spoken summary once a day: weather, news, your open tasks and uncommitted work. Place and topics are optional.";
pub const NO_REMINDERS: &str = "No reminders. Ask: remind me in 20 minutes to stretch.";
pub const LOADING: &str = "Connecting to Nixa…";
pub const TIME_HINT: &str = "08:00";
pub const PLACE_HINT: &str = "City for weather (optional)";
pub const TOPICS_HINT: &str = "News topics, comma-separated";
pub const ASKING: &str = "Asking Nixa…";
pub const SAVING: &str = "Saving…";

pub fn cancel_body(id: &str) -> Value {
    json!({"action": "remind_cancel", "id": id})
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Briefing {
    pub id: String,
    pub time: String,
    pub enabled: bool,
    pub place: String,
    pub topics: Vec<String>,
    pub next: String,
    pub last: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reminder {
    pub id: String,
    pub task: i64,
    pub title: String,
    pub due: String,
    pub waiting: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sources {
    pub web: bool,
    pub web_name: String,
    pub knowledge: bool,
    pub connectors: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct State {
    pub briefing: Option<Briefing>,
    pub reminders: Vec<Reminder>,
    pub sources: Sources,
    pub zone: String,
}

fn word(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn yes(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

pub fn parse(view: &Value) -> Option<State> {
    if !view.is_object() {
        return None;
    }
    let briefing = view.get("briefing").filter(|b| b.is_object()).map(|b| Briefing {
        id: word(b, "id"),
        time: word(b, "time"),
        enabled: yes(b, "enabled"),
        place: word(b, "place"),
        topics: b.get("topics").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default(),
        next: word(b, "next"),
        last: word(b, "last"),
    });
    let reminders = view
        .get("reminders")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .map(|r| Reminder { id: word(r, "id"), task: r.get("task").and_then(Value::as_i64).unwrap_or(0), title: word(r, "title"), due: word(r, "due"), waiting: yes(r, "waiting") })
                .collect()
        })
        .unwrap_or_default();
    let s = view.get("sources").cloned().unwrap_or(Value::Null);
    let sources = Sources { web: yes(&s, "web"), web_name: word(&s, "web_name"), knowledge: yes(&s, "knowledge"), connectors: yes(&s, "connectors") };
    Some(State { briefing, reminders, sources, zone: word(view, "zone") })
}

pub fn briefing_line(b: Option<&Briefing>) -> String {
    let Some(b) = b else {
        return "Not set up. Nothing runs until you save it.".into();
    };
    let place = if b.place.is_empty() { "no weather place".to_string() } else { format!("weather for {}", b.place) };
    let news = if b.topics.is_empty() { "no news topics".to_string() } else { format!("news about {}", b.topics.join(", ")) };
    let state = if b.enabled { format!("On, next {}", b.next) } else { "Paused".to_string() };
    format!("Daily at {}, {place}, {news}. {state}.", b.time)
}

pub fn sources_line(s: &Sources) -> String {
    let web = if s.web_name.is_empty() { "Web search" } else { s.web_name.as_str() };
    let on = |v: bool| if v { "on" } else { "off" };
    let mut web = web.to_string();
    if let Some(first) = web.get(..1) {
        web = format!("{}{}", first.to_uppercase(), &web[1..]);
    }
    format!(
        "{web}: {}. Local knowledge: {}. Email and calendar: {}.",
        on(s.web),
        on(s.knowledge),
        if s.connectors { "connected" } else { "no connector" }
    )
}

pub fn reminder_line(r: &Reminder) -> String {
    let wait = if r.waiting { "" } else { " (schedule paused)" };
    format!("#{} {} · due {}{wait}", r.task, r.title, r.due)
}

pub fn topics_of(text: &str) -> Vec<String> {
    text.split(',').map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect()
}

pub fn ask_body(question: &str) -> Value {
    json!({"action": "ask", "question": question.trim()})
}

pub fn briefing_body(time: &str, place: &str, topics: &str, enabled: bool) -> Value {
    let time = if time.trim().is_empty() { TIME_HINT } else { time.trim() };
    json!({"action": "briefing", "time": time, "place": place.trim(), "topics": topics_of(topics), "enabled": enabled})
}

pub fn on_body(enabled: bool) -> Value {
    json!({"action": "briefing_on", "enabled": enabled})
}

pub fn view_body() -> Value {
    json!({"action": "view"})
}
