use super::fun::{self, Fun, norm};
use crate::app::{App, lock};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

pub const BANTER_KIND: &str = "office_banter";
pub const BANTER_EVENT: &str = "office_banter_line";
pub const LIMIT_MS: u64 = 4000;
pub const KEEP: usize = 20;
pub const TASK_CHARS: usize = 60;

pub const SYSTEM: &str = "You write two short lines of friendly office banter while people work.
The first person makes a light, kind joke about a coworker and the real task title given.
The second person answers in a few words.
Use plain simple English. Each line is at most 14 words. Keep it office safe and kind.
Use the coworker's name. Nobody says their own name. Copy the task title exactly if you use it. Never add numbers.
Never reuse a line that is listed.
Answer with JSON only.";

pub const LINES: [(&str, &str); 8] = [
    ("{owner} is wrestling \"{task}\" again.", "Brave soul. I will bring tea."),
    ("\"{task}\" is putting up a real fight, {owner}.", "My money is on {owner}."),
    ("{owner} and \"{task}\" are deep in talks.", "Let us not disturb the talks."),
    ("Has anyone seen {owner}? Oh, still on \"{task}\".", "Send snacks, not questions."),
    ("I think \"{task}\" just blinked first, {owner}.", "Ha! One more push."),
    ("{owner} has that \"{task}\" look on the face.", "Best to wave from far away."),
    ("Quiet please, {owner} is taming \"{task}\".", "Taming is the right word."),
    ("{owner} versus \"{task}\", round two.", "I will keep the score."),
];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Banter {
    pub who: String,
    pub name: String,
    pub to: String,
    pub to_name: String,
    pub owner: String,
    pub task: String,
}

fn text(value: &Value, key: &str, max: usize) -> String {
    super::scene::cut(value.get(key).and_then(Value::as_str).unwrap_or(""), max)
}

pub fn banter_of(value: &Value) -> Option<Banter> {
    let b = Banter {
        who: text(value, "who", fun::NAME_CHARS),
        name: text(value, "name", fun::NAME_CHARS),
        to: text(value, "to", fun::NAME_CHARS),
        to_name: text(value, "to_name", fun::NAME_CHARS),
        owner: text(value, "owner", fun::NAME_CHARS),
        task: text(value, "task", TASK_CHARS),
    };
    let ok = !b.who.is_empty() && !b.to.is_empty() && b.who != b.to && !b.owner.is_empty() && !b.task.is_empty();
    ok.then_some(b)
}

pub fn fill(line: &str, b: &Banter) -> String {
    line.replace("{owner}", &b.owner).replace("{task}", &b.task)
}

pub fn choose(b: &Banter, recent: &[String], seed: u64) -> (String, String) {
    let n = LINES.len();
    let fresh = |i: usize| !recent.iter().any(|r| norm(r) == norm(&fill(LINES[i].0, b)));
    let start = (seed % n as u64) as usize;
    let at = (0..n).map(|k| (start + k) % n).find(|&i| fresh(i)).unwrap_or(start);
    (fill(LINES[at].0, b), fill(LINES[at].1, b))
}

pub fn user_message(b: &Banter, recent: &[String]) -> String {
    let mut lines = vec![
        format!("{} says it to {}.", b.name, b.to_name),
        format!("{} is busy working on \"{}\".", b.owner, b.task),
    ];
    if !recent.is_empty() {
        lines.push("Lines used before (do not reuse):".into());
        lines.extend(recent.iter().map(|r| format!("- {r}")));
    }
    lines.push("Write first and second.".into());
    lines.join("\n")
}

pub fn body(model: &str, b: &Banter, recent: &[String]) -> Value {
    json!({
        "model": model,
        "stream": false,
        "think": false,
        "keep_alive": "2m",
        "format": {
            "type": "object",
            "properties": {"first": {"type": "string"}, "second": {"type": "string"}},
            "required": ["first", "second"]
        },
        "options": {"temperature": 1.0, "num_predict": 100},
        "messages": [
            {"role": "system", "content": SYSTEM},
            {"role": "user", "content": user_message(b, recent)}
        ]
    })
}

pub fn check(raw: &Value, b: &Banter, recent: &[String]) -> Option<(String, String)> {
    let line = |k: &str| raw.get(k).and_then(Value::as_str).map(|t| t.split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default();
    let names = [b.name.as_str(), b.to_name.as_str(), b.owner.as_str()];
    let first = crate::soap::scene::no_own_name(&line("first"), &b.name, &names);
    let second = crate::soap::scene::no_own_name(&line("second"), &b.to_name, &names);
    let short = |t: &str| !t.is_empty() && t.chars().count() <= fun::TEXT_CHARS;
    let named = first.to_lowercase().contains(&b.owner.to_lowercase());
    let had = fun::numbers(&b.task);
    let numbers = [&first, &second].iter().flat_map(|t| fun::numbers(t)).any(|n| !had.contains(&n));
    let used = recent.iter().any(|r| norm(r) == norm(&first));
    (short(&first) && short(&second) && named && !numbers && !used && on_task(b, &first, &second)).then_some((first, second))
}

pub fn on_task(b: &Banter, first: &str, second: &str) -> bool {
    let task = b.task.to_lowercase();
    if [first, second].iter().any(|t| t.to_lowercase().contains(&task)) {
        return true;
    }
    let known = [b.owner.clone(), b.name.clone(), b.to_name.clone()];
    let quoted = [first, second].iter().any(|t| t.contains(['"', '“', '”', '`']));
    let titled = [first, second]
        .iter()
        .flat_map(|t| fun::keep_of(t, &known))
        .any(|t| !known.iter().any(|k| k.eq_ignore_ascii_case(&t)) && !fun::numbers(&t).iter().any(|n| fun::numbers(&b.task).contains(n)));
    !quoted && !titled
}

pub fn note(fun: &mut Fun, first: &str) {
    fun.banter.push(first.to_string());
    let extra = fun.banter.len().saturating_sub(KEEP);
    fun.banter.drain(..extra);
}

pub async fn make(app: &App, b: &Banter, limit: Duration) -> (String, String) {
    let recent = lock(&app.soap.fun).banter.clone();
    let got = fun::call(app, &body(&super::model(app), b, &recent), limit).await.ok();
    let lines = got.and_then(|raw| check(&raw, b, &recent)).unwrap_or_else(|| choose(b, &recent, crate::util::now_ms() / 1000));
    note(&mut lock(&app.soap.fun), &lines.0);
    lines
}

pub fn event(b: &Banter, lines: &(String, String)) -> Value {
    json!({"type": BANTER_EVENT, "who": b.who, "to": b.to, "first": lines.0, "second": lines.1})
}

fn start(app: &Arc<App>, value: &Value) {
    let Some(b) = banter_of(value) else { return };
    let flag = |k: &str| value.get(k).and_then(Value::as_bool) == Some(true);
    if !fun::gate_open(flag("on"), flag("office")) {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        let lines = make(&app, &b, Duration::from_millis(LIMIT_MS)).await;
        app.send_out(event(&b, &lines));
    });
}

pub fn ws(app: &Arc<App>, kind: &str, value: &Value) -> bool {
    if kind != BANTER_KIND {
        return false;
    }
    start(app, value);
    true
}
