use crate::app::{App, lock};
use crate::util::{log, now_ms};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

pub const FUN_KIND: &str = "office_fun";
pub const FUN_EVENT: &str = "office_fun_line";
pub const GATE_KIND: &str = "office_fun_gate";
pub const LIMIT_MS: u64 = 2500;
pub const KEEP: usize = 20;
pub const TEXT_CHARS: usize = 110;
pub const NAME_CHARS: usize = 40;
pub const SAME_SHARE: f32 = 0.7;
pub const STALE_MS: u64 = 60_000;
pub const DOWN_MS: u64 = 60_000;
pub const LOG_EVERY_MS: u64 = 60_000;
pub const SHORT_CHARS: usize = 40;
pub const KNOWN_MAX: usize = 40;

pub const SYSTEM: &str = "You rewrite short chat lines between coworkers in a friendly office.
Keep the exact meaning: the same task, the same result, the same names, numbers, file names, task titles and commands. Copy those exactly.
Each line stays in the speaker's own voice, talking to the listener with I and you. The speaker never says their own name and never starts with a name and a colon.
Use plain simple English with natural, varied words. Each line is at most 18 words.
Add a small, light touch of humour when the talk is relaxed.
If the talk is about a failure, an error, a block, a question for the user, an approval or a need for the user, set serious to true and write it plain with no jokes.
Never reuse an earlier version that is listed.
Answer with JSON only.";

const SERIOUS: [&str; 33] = [
    "fail", "error", "block", "stuck", "crash", "broke", "denied", "refused", "reject", "approv", "permission", "confirm",
    "question", "help", "urgent", "cannot", "can't", "unable", "the user", "need you", "needs you", "wrong", "problem", "warning",
    "not pass", "timed out", "timeout", "missing", "waiting for", "need your", "not found", "stopped", "killed",
];

const NEGATIONS: [&str; 8] = ["not", "no", "never", "cannot", "can't", "didn't", "won't", "failed"];

const STILL_NEGATIVE: [&str; 9] = ["not", "no", "never", "cannot", "failed", "fail", "unable", "nothing", "none"];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pair {
    pub from: String,
    pub to: String,
    pub first: String,
    pub second: String,
    pub known: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Said {
    pub text: String,
    pub changed: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outcome {
    pub first: Said,
    pub second: Said,
    pub serious: bool,
}

#[derive(Debug, Default)]
pub struct Fun {
    pub on: bool,
    pub office: bool,
    pub seen_at: u64,
    pub rewrites: u64,
    pub fallbacks: u64,
    pub last: Option<(String, String)>,
    pub last_reason: Option<String>,
    pub memory: HashMap<String, Vec<(String, String)>>,
    pub banter: Vec<String>,
    pub down_until: u64,
    pub logged: Option<(String, u64)>,
}

pub fn gate_open(on: bool, office: bool) -> bool {
    on && office
}

pub fn live(fun: &Fun, now: u64) -> bool {
    gate_open(fun.on, fun.office) && fun.seen_at > 0 && now.saturating_sub(fun.seen_at) < STALE_MS
}

pub fn resting(fun: &Fun, now: u64) -> bool {
    now < fun.down_until
}

pub fn note_reply(fun: &mut Fun, reply: &Result<Value, String>, now: u64) {
    match reply {
        Err(e) if e.contains("not reachable") => fun.down_until = now + DOWN_MS,
        Ok(_) => fun.down_until = 0,
        Err(_) => {}
    }
}

pub fn should_log(fun: &mut Fun, reason: &str, now: u64) -> bool {
    let fresh = match &fun.logged {
        Some((was, at)) => was != reason || now.saturating_sub(*at) >= LOG_EVERY_MS,
        None => true,
    };
    if fresh {
        fun.logged = Some((reason.to_string(), now));
    }
    fresh
}

pub fn serious(text: &str) -> bool {
    let low = text.to_lowercase();
    SERIOUS.iter().any(|k| low.contains(k))
}

pub fn norm(text: &str) -> String {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn similar(a: &str, b: &str) -> bool {
    let (a, b) = (norm(a), norm(b));
    if a == b {
        return true;
    }
    let wa: std::collections::HashSet<&str> = a.split(' ').collect();
    let wb: std::collections::HashSet<&str> = b.split(' ').collect();
    let all = wa.union(&wb).count();
    all > 0 && wa.intersection(&wb).count() as f32 / all as f32 >= SAME_SHARE
}

fn bare(word: &str) -> &str {
    word.trim_matches(|c: char| matches!(c, '.' | ',' | '!' | '?' | ';' | ':' | '(' | ')' | '"' | '\'' | '“' | '”' | '‘' | '’'))
}

fn spans(text: &str, mark: char) -> Vec<String> {
    text.split(mark).skip(1).step_by(2).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()
}

pub fn keep_of(text: &str, names: &[String]) -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut push = |t: String| {
        if !t.is_empty() && !keep.contains(&t) {
            keep.push(t);
        }
    };
    for mark in ['"', '`'] {
        for s in spans(text, mark) {
            push(s);
        }
    }
    let low = text.to_lowercase();
    for k in names.iter().filter(|k| k.contains(char::is_whitespace)) {
        if low.contains(&k.trim().to_lowercase()) {
            push(k.trim().to_string());
        }
    }
    let mut start = true;
    for raw in text.split_whitespace() {
        let word = bare(raw);
        let inner = word.trim_end_matches(['.', '/']);
        let filey = inner.contains(['/', '_', '.', ':', '\\']) || word.starts_with(['#', '-', '@', '$']);
        let digit = word.chars().any(|c| c.is_ascii_digit());
        let named = names.iter().any(|n| !n.is_empty() && n.eq_ignore_ascii_case(word));
        let capital = !start && word != "I" && word.chars().next().is_some_and(char::is_uppercase);
        if digit || filey || named || capital {
            push(word.to_string());
        }
        start = raw.ends_with(['.', '!', '?', ':']);
    }
    keep
}

pub fn numbers(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).map(str::to_string).collect()
}

fn words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            let w = bare(w).to_lowercase();
            w.strip_suffix("'s").or_else(|| w.strip_suffix("’s")).map(str::to_string).unwrap_or(w)
        })
        .filter(|w| !w.is_empty())
        .collect()
}

fn negative(word: &str, list: &[&str]) -> bool {
    let word = word.replace('’', "'");
    list.contains(&word.as_str()) || word.ends_with("n't")
}

pub fn guard(original: &str, rewrite: &str, names: &[String]) -> Result<(), String> {
    let low = rewrite.to_lowercase();
    let said = words(rewrite);
    for token in keep_of(original, names) {
        let found = if token.contains(char::is_whitespace) {
            low.contains(&token.to_lowercase())
        } else if token.chars().all(char::is_alphabetic) {
            said.contains(&token.to_lowercase())
        } else {
            rewrite.contains(&token)
        };
        if !found {
            return Err(format!("the rewrite lost \"{token}\""));
        }
    }
    if words(original).iter().any(|w| negative(w, &NEGATIONS)) && !said.iter().any(|w| negative(w, &STILL_NEGATIVE)) {
        return Err("the rewrite lost the \"not\"".into());
    }
    let had = numbers(original);
    if let Some(extra) = numbers(rewrite).into_iter().find(|n| !had.contains(n)) {
        return Err(format!("the rewrite added the number {extra}"));
    }
    Ok(())
}

pub fn earlier(memory: &HashMap<String, Vec<(String, String)>>, who: &str, original: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (was, made) in memory.get(&who.to_lowercase()).into_iter().flatten() {
        if similar(was, original) && !out.contains(made) {
            out.push(made.clone());
        }
    }
    out
}

pub fn remember(memory: &mut HashMap<String, Vec<(String, String)>>, who: &str, original: &str, made: &str) {
    let list = memory.entry(who.to_lowercase()).or_default();
    list.push((original.to_string(), made.to_string()));
    let extra = list.len().saturating_sub(KEEP);
    list.drain(..extra);
}

pub fn judge(original: &str, reply: Option<&str>, names: &[String], before: &[String]) -> Said {
    let keep = |reason: &str| Said { text: original.to_string(), changed: false, reason: Some(reason.to_string()) };
    if original.trim().is_empty() {
        return Said::default();
    }
    let Some(text) = reply.map(|t| t.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|t| !t.is_empty()) else {
        return keep("the model wrote no line");
    };
    if text.chars().count() > TEXT_CHARS.min((original.chars().count() * 2).max(SHORT_CHARS)) {
        return keep("the rewrite was too long");
    }
    if let Err(reason) = guard(original, &text, names) {
        return keep(&reason);
    }
    if before.iter().any(|b| norm(b) == norm(&text)) {
        return keep("the rewrite repeated an earlier one");
    }
    let changed = norm(&text) != norm(original) || text != original;
    Said { text, changed, reason: None }
}

pub fn settle(pair: &Pair, reply: Result<Value, String>, before: (&[String], &[String]), keyword: bool) -> Outcome {
    let mut names = vec![pair.from.clone(), pair.to.clone()];
    names.extend(pair.known.iter().cloned());
    let flagged = matches!(&reply, Ok(raw) if raw.get("serious").and_then(Value::as_bool) == Some(true));
    let reply = if flagged && !keyword { Err("the model marked the line serious".to_string()) } else { reply };
    match reply {
        Ok(raw) => {
            let line = |k: &str| raw.get(k).and_then(Value::as_str).map(str::to_string);
            let (first, second) = (line("first"), line("second"));
            Outcome {
                first: judge(&pair.first, first.as_deref(), &names, before.0),
                second: judge(&pair.second, second.as_deref(), &names, before.1),
                serious: keyword || flagged,
            }
        }
        Err(reason) => {
            let keep = |t: &str| Said {
                text: t.to_string(),
                changed: false,
                reason: (!t.trim().is_empty()).then(|| reason.clone()),
            };
            Outcome { first: keep(&pair.first), second: keep(&pair.second), serious: keyword || flagged }
        }
    }
}

pub fn user_message(pair: &Pair, before: (&[String], &[String]), keyword: bool) -> String {
    let mut lines = vec![format!("{} says to {}: \"{}\"", pair.from, pair.to, pair.first)];
    if !pair.second.trim().is_empty() {
        lines.push(format!("{} answers: \"{}\"", pair.to, pair.second));
    }
    for (label, list) in [("the first line", before.0), ("the answer", before.1)] {
        if !list.is_empty() {
            lines.push(format!("Earlier versions of {label} (write a different one):"));
            lines.extend(list.iter().map(|b| format!("- {b}")));
        }
    }
    if keyword {
        lines.push("This talk is serious: no jokes, only plain natural words.".into());
    }
    lines.push(if pair.second.trim().is_empty() { "Rewrite the line as first. Leave second empty.".into() } else { "Rewrite both lines.".into() });
    lines.join("\n")
}

pub fn body(model: &str, pair: &Pair, before: (&[String], &[String]), keyword: bool) -> Value {
    json!({
        "model": model,
        "stream": false,
        "think": false,
        "keep_alive": "2m",
        "format": {
            "type": "object",
            "properties": {"first": {"type": "string"}, "second": {"type": "string"}, "serious": {"type": "boolean"}},
            "required": ["first", "second", "serious"]
        },
        "options": {"temperature": if keyword { 0.5 } else { 0.95 }, "num_predict": 120},
        "messages": [
            {"role": "system", "content": SYSTEM},
            {"role": "user", "content": user_message(pair, before, keyword)}
        ]
    })
}

pub fn content_of(reply: &Value) -> Result<Value, String> {
    let content = reply.get("message").and_then(|m| m.get("content")).and_then(Value::as_str).ok_or("Ollama sent no message.")?;
    serde_json::from_str(content.trim()).map_err(|_| "the model did not write valid JSON".to_string())
}

pub async fn ask(http: &reqwest::Client, url: &str, body: &Value, limit: Duration) -> Result<Value, String> {
    let call = http.post(format!("{url}/api/chat")).timeout(limit).json(body).send();
    let reply = tokio::time::timeout(limit, call)
        .await
        .map_err(|_| "Ollama took too long".to_string())?
        .map_err(|e| if e.is_timeout() { "Ollama took too long".to_string() } else { format!("Ollama is not reachable at {url}") })?;
    if !reply.status().is_success() {
        return Err(format!("Ollama answered {}", reply.status().as_u16()));
    }
    let value: Value = reply.json().await.map_err(|_| "Ollama sent a broken reply".to_string())?;
    content_of(&value)
}

pub fn record(fun: &mut Fun, pair: &Pair, out: &Outcome) {
    for (who, original, said) in [(&pair.from, &pair.first, &out.first), (&pair.to, &pair.second, &out.second)] {
        if original.trim().is_empty() {
            continue;
        }
        if said.changed {
            fun.rewrites += 1;
            remember(&mut fun.memory, who, original, &said.text);
            fun.last = Some((original.clone(), said.text.clone()));
        } else if let Some(reason) = &said.reason {
            fun.fallbacks += 1;
            fun.last_reason = Some(reason.clone());
        }
    }
}

pub async fn call(app: &App, body: &Value, limit: Duration) -> Result<Value, String> {
    let now = now_ms();
    if resting(&lock(&app.soap.fun), now) {
        return Err("Ollama was not reachable a moment ago".into());
    }
    let url = super::url(app);
    super::allowed_url(app, &url)?;
    let reply = ask(&app.http, &url, body, limit).await;
    note_reply(&mut lock(&app.soap.fun), &reply, now_ms());
    reply
}

pub async fn rewrite(app: &App, pair: &Pair, limit: Duration) -> Outcome {
    let keyword = serious(&pair.first) || serious(&pair.second);
    let (one, two) = {
        let fun = lock(&app.soap.fun);
        (earlier(&fun.memory, &pair.from, &pair.first), earlier(&fun.memory, &pair.to, &pair.second))
    };
    let reply = call(app, &body(&super::model(app), pair, (&one, &two), keyword), limit).await;
    let out = settle(pair, reply, (&one, &two), keyword);
    record(&mut lock(&app.soap.fun), pair, &out);
    out
}

fn text_of(value: &Value, key: &str, max: usize) -> String {
    super::scene::cut(value.get(key).and_then(Value::as_str).unwrap_or(""), max)
}

pub fn pair_of(value: &Value) -> Option<Pair> {
    let pair = Pair {
        from: text_of(value, "from_name", NAME_CHARS),
        to: text_of(value, "to_name", NAME_CHARS),
        first: text_of(value, "first", TEXT_CHARS * 2),
        second: text_of(value, "second", TEXT_CHARS * 2),
        known: value
            .get("known")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|k| super::scene::cut(k, TEXT_CHARS))
            .filter(|k| !k.is_empty())
            .take(KNOWN_MAX)
            .collect(),
    };
    (!pair.first.is_empty()).then_some(pair)
}

pub fn said_json(s: &Said) -> Value {
    json!({"text": s.text, "changed": s.changed, "reason": s.reason})
}

pub fn event(id: u64, out: &Outcome) -> Value {
    json!({"type": FUN_EVENT, "id": id, "first": out.first.text, "second": out.second.text, "serious": out.serious,
        "changed": out.first.changed || out.second.changed})
}

pub fn status(fun: &Fun, now: u64) -> Value {
    json!({
        "on": live(fun, now),
        "switch": fun.on,
        "office": fun.office,
        "seen_secs": (fun.seen_at > 0).then(|| now.saturating_sub(fun.seen_at) / 1000),
        "rewrites": fun.rewrites,
        "fallbacks": fun.fallbacks,
        "last": fun.last.as_ref().map(|(a, b)| json!({"original": a, "text": b})),
        "last_reason": fun.last_reason,
    })
}

fn set_gate(app: &Arc<App>, value: &Value) {
    let flag = |k: &str| value.get(k).and_then(Value::as_bool) == Some(true);
    let opened = {
        let mut fun = lock(&app.soap.fun);
        let was = gate_open(fun.on, fun.office);
        fun.on = flag("on");
        fun.office = flag("office");
        fun.seen_at = now_ms();
        !was && gate_open(fun.on, fun.office)
    };
    if opened && !cfg!(test) {
        warm(app);
    }
}

fn warm(app: &Arc<App>) {
    let url = super::url(app);
    if super::allowed_url(app, &url).is_err() {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        let body = json!({"model": super::model(&app), "keep_alive": "2m", "messages": []});
        let _ = app.http.post(format!("{url}/api/chat")).timeout(Duration::from_secs(30)).json(&body).send().await;
    });
}

fn start(app: &Arc<App>, value: &Value) {
    let id = value.get("id").and_then(Value::as_u64).unwrap_or(0);
    let Some(pair) = pair_of(value) else { return };
    let flag = |k: &str| value.get(k).and_then(Value::as_bool) == Some(true);
    if !gate_open(flag("on"), flag("office")) {
        let out = settle(&pair, Err("the fun layer is off".into()), (&[], &[]), false);
        app.send_out(event(id, &out));
        return;
    }
    set_gate(app, value);
    let app = app.clone();
    tokio::spawn(async move {
        let out = rewrite(&app, &pair, Duration::from_millis(LIMIT_MS)).await;
        if let Some(reason) = out.first.reason.as_ref().or(out.second.reason.as_ref())
            && should_log(&mut lock(&app.soap.fun), reason, now_ms())
        {
            log("soap", &format!("office fun kept the real line: {reason}"));
        }
        app.send_out(event(id, &out));
    });
}

pub fn ws(app: &Arc<App>, kind: &str, value: &Value) -> bool {
    match kind {
        FUN_KIND => start(app, value),
        GATE_KIND => set_gate(app, value),
        _ => return false,
    }
    true
}
