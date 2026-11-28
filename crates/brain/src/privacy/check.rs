use crate::trees::RuleTree;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

pub const TREE: &str = "message-check";
pub const TIMEOUT_MS: u64 = 8000;
pub const ITEM_CHARS: usize = 200;
pub const LOG_FILE: &str = "message-checks.jsonl";
pub const ON_KEY: &str = "message_check.on";
pub const BRAIN_KEY: &str = "message_check.brain";
pub const MODEL_KEY: &str = "message_check.model";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Setting {
    pub on: bool,
    pub brain: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Item {
    pub text: String,
    pub quote: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Answer {
    pub yes: bool,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Question {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Step {
    pub key: String,
    pub result: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Status {
    #[serde(rename = "done")]
    Done,
    #[serde(rename = "no model")]
    NoModel,
    #[serde(rename = "timed out")]
    TimedOut,
    #[serde(rename = "bad reply")]
    BadReply,
    #[serde(rename = "failed")]
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Record {
    pub at: u64,
    pub ms: u64,
    pub status: Status,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<u64>,
    pub answers: BTreeMap<String, Answer>,
    pub steps: Vec<Step>,
    pub note: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Record {
    pub fn new(at: u64, ms: u64, status: Status, text: &str, chat_id: Option<u64>) -> Record {
        Record { at, ms, status, text: text.to_string(), chat_id, answers: BTreeMap::new(), steps: Vec::new(), note: String::new(), reason: None }
    }
}

fn step_box(text: &str, next: Option<&str>) -> Value {
    match next {
        Some(to) => json!({"text": text, "next": {"then": to}}),
        None => json!({"text": text}),
    }
}

pub fn default_tree() -> Value {
    json!({
        "version": 1,
        "name": TREE,
        "title": "Message check",
        "when": "Run on every message from the owner before Nixa's brain answers. Each box is one yes or no question.",
        "start": "task",
        "boxes": {
            "task": step_box("Does the message ask for a new job for an agent, a task in a space, or a thing Nixa should remember or check later? List each one: text is the job in a short line, quote is the exact words.", Some("deep")),
            "deep": step_box("Is there a choice for the deep brain: what to build, a design, a name, the next step, or which agent or model should do a job? List each choice: text is the choice, quote is the exact words.", Some("answer")),
            "answer": step_box("Does the message ask a question that Nixa should answer now? Quote the exact words.", Some("wait")),
            "wait": step_box("Does the message say wait, discuss, hold or stop? These never become actions. Quote the exact words.", Some("correction")),
            "correction": step_box("Does the message correct an earlier ask, such as no, not that, or change it? Quote the exact words. An earlier task is updated, not added again.", Some("own-step")),
            "own-step": step_box("Does the message ask for a git, push, deploy, publish or production step? Those stay with the owner: Nixa does not do them. Quote the exact words.", Some("setting")),
            "setting": step_box("Does the message change a Nixa setting or a Nixa rule? Quote the exact words.", Some("space")),
            "space": step_box("Does the message name which space or project the work belongs to? Quote the exact words.", None),
        }
    })
}

pub fn questions(tree: &RuleTree) -> Vec<Question> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut id = Some(tree.start.clone());
    while let Some(current) = id {
        let Some(rule) = tree.boxes.get(&current) else { break };
        if !seen.insert(current.clone()) {
            break;
        }
        out.push(Question { id: current.clone(), text: rule.text.clone() });
        id = rule.next.as_ref().and_then(|n| n.values().next().cloned());
    }
    out
}

pub fn prompt(questions: &[Question], owner: &str) -> String {
    let mut lines = vec![
        format!("You check one message that {owner} sent to Nixa. Answer every question below."),
        "Reply with only one JSON object. No code fences, no other words.".to_string(),
        "Its keys are exactly these ids:".to_string(),
    ];
    lines.extend(questions.iter().map(|q| format!("- {}: {}", q.id, q.text)));
    lines.push(r#"Each value is {"yes": true or false, "items": [{"text": "short line", "quote": "exact words from the message"}]}."#.to_string());
    lines.push("Use items only when yes is true. Each quote must be copied exactly from the message.".to_string());
    lines.join("\n")
}

fn norm(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

pub fn message_of(text: &str) -> String {
    text.trim().chars().take(ITEM_CHARS * 10).collect()
}

pub fn parse_reply(reply: &str, questions: &[Question], message: &str) -> Result<BTreeMap<String, Answer>, String> {
    let (Some(start), Some(end)) = (reply.find('{'), reply.rfind('}')) else { return Err("the reply has no JSON object".into()) };
    if end < start {
        return Err("the reply has no JSON object".into());
    }
    let parsed: Value = serde_json::from_str(&reply[start..=end]).map_err(|_| "the reply is not valid JSON".to_string())?;
    let raw = parsed.as_object().ok_or("the reply is not a JSON object")?;
    let haystack = norm(message);
    let mut answers = BTreeMap::new();
    for question in questions {
        let value = raw.get(&question.id);
        let Some(yes) = value.and_then(|v| v.get("yes")).and_then(Value::as_bool) else {
            return Err(format!("answer {} is missing or not yes/no", question.id));
        };
        let mut items = Vec::new();
        if yes {
            for item in value.and_then(|v| v.get("items")).and_then(Value::as_array).into_iter().flatten() {
                let text: String = item.get("text").and_then(Value::as_str).map(str::trim).unwrap_or("").chars().take(ITEM_CHARS).collect();
                let quote = item.get("quote").and_then(Value::as_str).map(str::trim).unwrap_or("").to_string();
                if text.is_empty() || quote.is_empty() || !haystack.contains(&norm(&quote)) {
                    continue;
                }
                items.push(Item { text, quote });
            }
        }
        answers.insert(question.id.clone(), Answer { yes: yes && !items.is_empty(), items });
    }
    Ok(answers)
}

fn texts(answer: &Answer) -> String {
    let joined = answer.items.iter().map(|i| i.text.as_str()).collect::<Vec<_>>().join("; ");
    if joined.is_empty() { "yes".into() } else { joined }
}

pub fn act(answers: &BTreeMap<String, Answer>, questions: &[Question], chat_id: Option<u64>, owner: &str, deep_ask: &mut dyn FnMut(&Item) -> String) -> (Vec<Step>, String) {
    let mut steps = Vec::new();
    let mut left = Vec::new();
    let step = |key: &str, result: String| Step { key: key.to_string(), result };
    for question in questions {
        let Some(answer) = answers.get(&question.id).filter(|a| a.yes) else { continue };
        match question.id.as_str() {
            "task" => {
                steps.push(step("task", "not done by the check: the brain adds the tasks".into()));
                let chat = chat_id.map(|id| format!(", and name chat item {id} in the description so {owner}'s words can be found")).unwrap_or_default();
                left.push(format!("task: add each one with nixa_task (action add){chat}: {}", texts(answer)));
            }
            "deep" => {
                for item in &answer.items {
                    steps.push(step("deep", deep_ask(item)));
                }
            }
            "wait" => steps.push(step("wait", format!("no action: {owner} said wait, discuss or stop"))),
            "correction" => {
                steps.push(step("correction", "not done by the check: the brain updates the earlier item".into()));
                left.push("an earlier ask is corrected: update its task with nixa_task, do not add it again".into());
            }
            "own-step" => {
                steps.push(step("own-step", format!("not done: git, push, deploy, publish and production stay with {owner}")));
                left.push(format!("a git, push, deploy or production step: {owner}'s own, do not do it"));
            }
            other => {
                steps.push(step(other, format!("not done by the check: {other} is for the brain to act on")));
                left.push(format!("{other}: {}", texts(answer)));
            }
        }
    }
    if steps.is_empty() {
        return (steps, String::new());
    }
    let mut lines = vec!["Note: Nixa's message check ran on this message before this reply. Its steps are done; do not do them again:".to_string()];
    lines.extend(steps.iter().filter(|s| !s.result.starts_with("not ")).map(|s| format!("- {}: {}", s.key, s.result)));
    if !left.is_empty() {
        lines.push("Still for you to act on, in this reply only:".into());
        lines.extend(left.iter().map(|l| format!("- {l}")));
    }
    (steps, lines.join("\n"))
}

fn count_of(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

pub fn summary(steps: &[Step]) -> String {
    let done = |key: &str| steps.iter().filter(|s| s.key == key && !s.result.starts_with("not ")).count();
    let mut parts = Vec::new();
    let deep = done("deep");
    if deep > 0 {
        parts.push(count_of(deep, "deep brain ask", "deep brain asks"));
    }
    if steps.iter().any(|s| s.key == "wait") {
        parts.push("wait (no action)".to_string());
    }
    if parts.is_empty() { "nothing to do".into() } else { parts.join(", ") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> RuleTree {
        crate::trees::check_tree(&default_tree(), None).unwrap()
    }

    #[test]
    fn the_default_tree_is_valid_and_walks_in_order() {
        let ids: Vec<String> = questions(&tree()).into_iter().map(|q| q.id).collect();
        assert_eq!(ids, ["task", "deep", "answer", "wait", "correction", "own-step", "setting", "space"]);
        let text = prompt(&questions(&tree()), "Sam");
        assert!(text.starts_with("You check one message that Sam sent to Nixa."));
        assert!(text.contains("- own-step: Does the message ask for a git"));
    }

    #[test]
    fn replies_are_checked_and_quotes_must_be_real() {
        let qs = questions(&tree());
        let message = "Please add a task to fix the  login page, and push it";
        let mut body = serde_json::Map::new();
        for q in &qs {
            body.insert(q.id.clone(), json!({"yes": false, "items": []}));
        }
        body.insert("task".into(), json!({"yes": true, "items": [{"text": "Fix the login page", "quote": "fix the login page"}, {"text": "Made up", "quote": "not in the message"}]}));
        body.insert("own-step".into(), json!({"yes": true, "items": [{"text": "push", "quote": "Push it"}]}));
        body.insert("deep".into(), json!({"yes": true, "items": [{"text": "x", "quote": "invented"}]}));
        let reply = format!("Sure: {} done", Value::Object(body.clone()));
        let answers = parse_reply(&reply, &qs, message).unwrap();
        assert!(answers["task"].yes);
        assert_eq!(answers["task"].items.len(), 1);
        assert!(!answers["deep"].yes);
        assert!(answers["own-step"].yes);
        let mut asked = Vec::new();
        let (steps, note) = act(&answers, &qs, Some(7), "Sam", &mut |item| {
            asked.push(item.text.clone());
            "asked d1".into()
        });
        assert!(asked.is_empty());
        assert_eq!(steps.iter().map(|s| s.key.as_str()).collect::<Vec<_>>(), ["task", "own-step"]);
        assert!(note.contains("- task: add each one with nixa_task (action add), and name chat item 7 in the description so Sam's words can be found: Fix the login page"));
        assert!(note.contains("Sam's own, do not do it"));
        assert_eq!(summary(&steps), "nothing to do");
        body.remove("space");
        assert_eq!(parse_reply(&Value::Object(body).to_string(), &qs, message), Err("answer space is missing or not yes/no".into()));
        assert_eq!(parse_reply("no json", &qs, message), Err("the reply has no JSON object".into()));
        assert_eq!(parse_reply("{bad}", &qs, message), Err("the reply is not valid JSON".into()));
    }

    #[test]
    fn deep_asks_and_waits_are_done_steps() {
        let qs = questions(&tree());
        let mut answers = BTreeMap::new();
        answers.insert("deep".to_string(), Answer { yes: true, items: vec![Item { text: "Pick a name".into(), quote: "name".into() }, Item { text: "Pick a model".into(), quote: "model".into() }] });
        answers.insert("wait".to_string(), Answer { yes: true, items: vec![Item { text: "wait".into(), quote: "wait".into() }] });
        let mut n = 0;
        let (steps, note) = act(&answers, &qs, None, "Sam", &mut |_| {
            n += 1;
            format!("asked d{n}")
        });
        assert_eq!(summary(&steps), "2 deep brain asks, wait (no action)");
        assert!(note.contains("- deep: asked d1\n- deep: asked d2\n"));
        assert!(!note.contains("Still for you"));
        let (none, empty) = act(&BTreeMap::new(), &qs, None, "Sam", &mut |_| String::new());
        assert!(none.is_empty() && empty.is_empty());
    }
}
