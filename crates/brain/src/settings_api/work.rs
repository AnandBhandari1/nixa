use serde::Serialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TopicKind {
    Discuss,
    Plan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Topic {
    pub kind: TopicKind,
    pub text: String,
    pub at: u64,
    pub alias: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hold {
    pub at: u64,
    pub alias: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Work {
    pub topic: Option<Topic>,
    pub hold: Option<Hold>,
}

fn topic_word(kind: TopicKind) -> &'static str {
    match kind {
        TopicKind::Discuss => "Discuss",
        TopicKind::Plan => "Plan",
    }
}

pub fn note(work: &Work) -> String {
    let topic = work
        .topic
        .as_ref()
        .map(|t| {
            let about = if t.text.is_empty() { String::new() } else { format!(" for \"{}\"", t.text) };
            format!(" Keyword {} is on{about}: talk it through with the user. It holds no step: work goes on and each step goes in the decision journal.", topic_word(t.kind).to_lowercase())
        })
        .unwrap_or_default();
    let hold = if work.hold.is_some() {
        format!(
            " Hold is on: no new work starts, agent questions, checks and fixes still run, and the hold ends by itself {} minutes after it began.",
            crate::modes::work::HOLD_MS / 60_000
        )
    } else {
        String::new()
    };
    format!("Note:{topic}{hold} Nixa does the work: assign it, run it, check it and close it. Do not wait for the user.")
}

pub fn clear(work: &mut Work, what: &str) -> Result<Option<String>, String> {
    match what {
        "hold" => Ok(work.hold.take().map(|_| "Hold is off. Work runs again.".to_string())),
        "topic" => Ok(work.topic.take().map(|t| format!("{} is off.", topic_word(t.kind)))),
        _ => Err("keyword_clear needs what: topic or hold.".into()),
    }
}

pub fn view(work: &Work, keywords: Value) -> Value {
    let mut out = json!({});
    if let Some(map) = out.as_object_mut() {
        if let Some(topic) = &work.topic {
            map.insert("topic".into(), json!({"kind": topic.kind, "text": topic.text, "at": topic.at}));
        }
        if let Some(hold) = &work.hold {
            map.insert("hold".into(), json!({"at": hold.at}));
        }
        if let Value::Object(more) = keywords {
            map.extend(more);
        }
    }
    out
}
