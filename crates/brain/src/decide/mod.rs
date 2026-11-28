use crate::app::{App, lock};
use crate::util::{clip_flat, log, read_json, write_json};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

pub mod asks;
pub mod routes;
pub mod settings;
pub mod topic;

#[cfg(test)]
mod tests;

pub const ROUTE: &str = "/api/experimental/generate/decisions";
pub const PROVIDER: &str = "opencode-go";
pub const DEFAULT_MODEL: &str = "jev-1.14";
pub const MODELS: [&str; 3] = ["jev-1.14", "gpt-6-luna-decision", "gemini-decision-1"];
pub const LIMIT_MS: u64 = 20_000;
pub const PRIVATE: &str = "The decision model is not used while Private and confidential is on. Nothing was sent; ask the usual way.";
pub const USAGE_FILE: &str = "decide-usage.json";
const WARM_TRIES: u32 = 5;
const WARM_WAIT_MS: u64 = 2_000;

#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    Predicate { name: String, probability: f64 },
    Choice { name: String, choice: Value, confidence: f64, probabilities: Vec<(Value, f64)> },
    Score { name: String, score: f64, confidence: f64, probabilities: Vec<(String, f64)> },
    Failed { name: String, reason: String, message: String },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Answers {
    pub answers: Vec<Answer>,
    pub cost: f64,
    pub input_tokens: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub day: String,
    pub calls: u64,
    pub failed: u64,
    pub cost: f64,
}

pub fn text(text: &str) -> Value {
    json!({"type": "text", "text": text})
}

pub fn data(value: Value) -> Value {
    json!({"type": "json", "value": value})
}

pub fn picture(picture: &nixa_desktop::picture::Picture, name: &str) -> Value {
    json!({"type": "file", "data": picture.data, "mime": picture.media_type, "name": name})
}

pub fn predicate(name: &str, instructions: &str) -> Value {
    json!({"type": "predicate", "name": name, "instructions": instructions})
}

pub fn choice(name: &str, instructions: &str, choices: &[(&str, &str)]) -> Value {
    let choices: Vec<Value> = choices.iter().map(|(value, about)| if about.is_empty() { json!({"value": value}) } else { json!({"value": value, "description": about}) }).collect();
    json!({"type": "choice", "name": name, "instructions": instructions, "choices": choices})
}

pub fn score(name: &str, instructions: &str, levels: &[(&str, &str)]) -> Value {
    let levels: Vec<Value> = levels.iter().map(|(label, about)| if about.is_empty() { json!({"label": label}) } else { json!({"label": label, "description": about}) }).collect();
    json!({"type": "score", "name": name, "instructions": instructions, "levels": levels})
}

pub fn request(model: &str, input: Vec<Value>, questions: Vec<Value>) -> Value {
    json!({"model": {"id": model, "providerID": PROVIDER}, "input": input, "questions": questions})
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

fn name_of(v: &Value) -> String {
    v["name"].as_str().unwrap_or_default().to_string()
}

pub fn read_answer(v: &Value) -> Answer {
    let name = name_of(v);
    match v["type"].as_str().unwrap_or_default() {
        "predicate" => Answer::Predicate { name, probability: num(&v["probability"]) },
        "choice" => Answer::Choice {
            name,
            choice: v["choice"].clone(),
            confidence: num(&v["confidence"]),
            probabilities: v["probabilities"].as_array().map(|a| a.iter().map(|p| (p["value"].clone(), num(&p["probability"]))).collect()).unwrap_or_default(),
        },
        "score" => Answer::Score {
            name,
            score: num(&v["score"]),
            confidence: num(&v["confidence"]),
            probabilities: v["probabilities"].as_array().map(|a| a.iter().map(|p| (p["label"].as_str().unwrap_or_default().to_string(), num(&p["probability"]))).collect()).unwrap_or_default(),
        },
        "error" => Answer::Failed { name, reason: v["reason"].as_str().unwrap_or("error").to_string(), message: v["message"].as_str().unwrap_or_default().to_string() },
        other => Answer::Failed { name, reason: "unknown".into(), message: format!("unknown answer type {other:?}") },
    }
}

pub fn parse(body: &Value) -> Result<Answers, String> {
    let data = &body["data"];
    let Some(list) = data["answers"].as_array() else { return Err(format!("The decision reply had no answers: {}", clip_flat(&body.to_string(), 200))) };
    Ok(Answers { answers: list.iter().map(read_answer).collect(), cost: num(&data["cost"]), input_tokens: data["tokens"]["input"].as_f64().unwrap_or(0.0) as u64 })
}

impl Answer {
    pub fn name(&self) -> &str {
        match self {
            Answer::Predicate { name, .. } | Answer::Choice { name, .. } | Answer::Score { name, .. } | Answer::Failed { name, .. } => name,
        }
    }

    pub fn sure(&self) -> f64 {
        match self {
            Answer::Predicate { probability, .. } => probability.max(1.0 - probability),
            Answer::Choice { confidence, .. } | Answer::Score { confidence, .. } => *confidence,
            Answer::Failed { .. } => 0.0,
        }
    }

    pub fn short(&self) -> String {
        match self {
            Answer::Predicate { name, probability } => format!("{name}={probability:.2}"),
            Answer::Choice { name, choice, confidence, .. } => format!("{name}={} ({confidence:.2})", choice.as_str().map(str::to_string).unwrap_or_else(|| choice.to_string())),
            Answer::Score { name, score, confidence, .. } => format!("{name}={score:.2} ({confidence:.2})"),
            Answer::Failed { name, reason, .. } => format!("{name} failed: {reason}"),
        }
    }
}

impl Answers {
    pub fn get(&self, name: &str) -> Option<&Answer> {
        self.answers.iter().find(|a| a.name() == name)
    }

    pub fn all_sure(&self, level: f64) -> bool {
        !self.answers.is_empty() && self.answers.iter().all(|a| a.sure() >= level)
    }
}

pub fn today() -> String {
    crate::extras::usage::day_key(crate::util::now_ms())
}

pub fn count(mut usage: Usage, day: &str, ok: bool, cost: f64) -> Usage {
    if usage.day != day {
        usage = Usage { day: day.to_string(), ..Usage::default() };
    }
    usage.calls += 1;
    if !ok {
        usage.failed += 1;
    }
    usage.cost += cost;
    usage
}

pub fn usage(app: &App) -> Usage {
    let usage: Usage = read_json(&app.dir.join(USAGE_FILE)).unwrap_or_default();
    if usage.day == today() { usage } else { Usage { day: today(), ..Usage::default() } }
}

fn note(app: &App, ok: bool, cost: f64) {
    let file = app.dir.join(USAGE_FILE);
    let next = count(read_json(&file).unwrap_or_default(), &today(), ok, cost);
    if let Err(e) = write_json(&file, &next) {
        log("decide", &format!("the decision usage was not saved: {e}"));
    }
}

pub fn server(app: &App) -> Option<(String, String)> {
    lock(&app.cli.live).opencode.clone()
}

pub fn server_text(app: &App) -> &'static str {
    if server(app).is_some() {
        "the OpenCode chat brain server"
    } else if crate::cli_brains::decide_server::running(app).is_some() {
        "a small OpenCode server for decisions"
    } else {
        "starts when needed"
    }
}

async fn pick_server(app: &Arc<App>) -> Result<(String, String), String> {
    match server(app) {
        Some(server) => Ok(server),
        None => crate::cli_brains::decide_server::get(app).await,
    }
}

pub async fn ask(app: &Arc<App>, model: &str, input: Vec<Value>, questions: Vec<Value>) -> Result<Answers, String> {
    if crate::privacy::privacy_on(app) {
        return Err(PRIVATE.into());
    }
    let (base, password) = pick_server(app).await?;
    if crate::privacy::privacy_on(app) {
        return Err(PRIVATE.into());
    }
    let names: Vec<String> = questions.iter().map(name_of).collect();
    let body = request(model, input, questions);
    let mut out = send(&base, &password, &body).await;
    for _ in 0..WARM_TRIES {
        if !out.as_ref().err().is_some_and(|e| not_loaded(e)) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(WARM_WAIT_MS)).await;
        if crate::privacy::privacy_on(app) {
            return Err(PRIVATE.into());
        }
        out = send(&base, &password, &body).await;
    }
    match &out {
        Ok(answers) => {
            note(app, true, answers.cost);
            let tops: Vec<String> = answers.answers.iter().map(Answer::short).collect();
            log("decide", &format!("{model} {}: {} cost ${:.5}", names.join(","), tops.join(", "), answers.cost));
        }
        Err(e) => {
            note(app, false, 0.0);
            log("decide", &format!("{model} {} failed: {e}", names.join(",")));
        }
    }
    out
}

pub fn not_loaded(error: &str) -> bool {
    error.contains("ModelNotFoundError")
}

async fn send(base: &str, password: &str, body: &Value) -> Result<Answers, String> {
    let http = reqwest::Client::builder().connect_timeout(Duration::from_secs(5)).build().map_err(|e| e.to_string())?;
    let response = http.post(format!("{base}{ROUTE}")).basic_auth("opencode", Some(password)).timeout(Duration::from_millis(LIMIT_MS)).json(body).send().await.map_err(|e| if e.is_timeout() { "The decision model did not answer in 20 s.".to_string() } else { format!("The decision call failed: {e}") })?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("The decision call failed: {} {}", status.as_u16(), clip_flat(&text, 200)));
    }
    let value: Value = serde_json::from_str(&text).map_err(|e| format!("The decision reply was not JSON: {e}"))?;
    parse(&value)
}
