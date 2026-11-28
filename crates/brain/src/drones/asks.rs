use super::{DroneStep, DroneSubject, DroneTree, FactMap};
use crate::app::{App, lock};
use crate::decide::{self, Answer, settings};
use crate::util::{clip, log, short_hash};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

pub const QUESTION_MAX: usize = 300;
pub const FRESH_MS: u64 = 10 * 60 * 1000;
pub const NEW_PER_TICK: usize = 6;
pub const SCREEN_LINES: usize = 40;

#[derive(Default)]
pub struct Book {
    pub answers: HashMap<String, (bool, u64)>,
    pub asking: HashSet<String>,
}

pub fn fact_of(question: &str) -> String {
    format!("ask.{}", short_hash(question.trim()))
}

pub fn key_of(subject: &str, question: &str) -> String {
    format!("{subject}\n{}", question.trim())
}

pub fn questions(tree: &DroneTree) -> Vec<String> {
    let mut out = BTreeSet::new();
    for step in tree.steps.values().filter(|s| !s.is_off()) {
        let tests = match step {
            DroneStep::If { test, .. } => vec![test],
            DroneStep::All { tests, .. } | DroneStep::Any { tests, .. } => tests.iter().collect(),
            _ => Vec::new(),
        };
        out.extend(tests.into_iter().filter_map(|t| t.ask.clone()));
    }
    out.into_iter().collect()
}

pub fn facts_text(facts: &FactMap) -> String {
    facts.iter().filter(|(k, _)| !k.starts_with("ask.")).map(|(k, v)| format!("{k}: {}", v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))).collect::<Vec<_>>().join("\n")
}

pub fn input(facts: &FactMap, screen: &str) -> String {
    let mut text = format!("Facts about one task or terminal that Nixa watches:\n{}", crate::agent::data_block(&facts_text(facts)));
    if !screen.trim().is_empty() {
        let lines: Vec<&str> = screen.lines().filter(|l| !l.trim().is_empty()).collect();
        let tail = lines[lines.len().saturating_sub(SCREEN_LINES)..].join("\n");
        text.push_str(&format!("\nLast lines of the terminal screen:\n{}", crate::agent::data_block(&clip(&tail, 3000))));
    }
    text
}

pub fn fill(app: &Arc<App>, tree: &DroneTree, list: &mut [DroneSubject], now: u64) {
    let questions = questions(tree);
    if questions.is_empty() {
        return;
    }
    let set = settings::load(app);
    let mut start = Vec::new();
    {
        let mut book = lock(&app.drones.asks);
        book.answers.retain(|_, (_, at)| now.saturating_sub(*at) < FRESH_MS * 3);
        for subject in list.iter_mut() {
            for question in &questions {
                let key = key_of(&subject.key, question);
                let fresh = book.answers.get(&key).filter(|(_, at)| now.saturating_sub(*at) < FRESH_MS).map(|(yes, _)| *yes);
                if let Some((yes, _)) = book.answers.get(&key) {
                    subject.facts.insert(fact_of(question), json!(*yes));
                }
                if fresh.is_none() && set.uses("drones") && start.len() < NEW_PER_TICK && book.asking.insert(key.clone()) {
                    start.push((key, subject.facts.clone(), question.clone()));
                }
            }
        }
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        let mut book = lock(&app.drones.asks);
        for (key, _, _) in &start {
            book.asking.remove(key);
        }
        return;
    };
    for (key, facts, question) in start {
        let app = app.clone();
        let model = set.model.clone();
        let level = set.level;
        handle.spawn(async move {
            let yes = ask_one(&app, &model, level, &facts, &question).await;
            let mut book = lock(&app.drones.asks);
            book.asking.remove(&key);
            match yes {
                Ok(yes) => {
                    book.answers.insert(key, (yes, crate::util::now_ms()));
                }
                Err(e) => log("drones", &format!("ask step \"{}\" took the no path: {e}", clip(&question, 80))),
            }
        });
    }
}

pub fn yes_of(probability: f64, level: f64) -> bool {
    probability >= level
}

async fn ask_one(app: &Arc<App>, model: &str, level: f64, facts: &FactMap, question: &str) -> Result<bool, String> {
    let screen = match facts.get("terminal.id").and_then(Value::as_str) {
        Some(id) if facts.get("subject").and_then(Value::as_str) == Some("terminal") => app.prompts.link().read(id).await.unwrap_or_default(),
        _ => String::new(),
    };
    let answers = decide::ask(app, model, vec![decide::text(&input(facts, &screen))], vec![decide::predicate("ask", question)]).await?;
    match answers.get("ask") {
        Some(Answer::Predicate { probability, .. }) => Ok(yes_of(*probability, level)),
        _ => Err("no answer".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drones::{DroneTest, passes};
    use std::collections::BTreeMap;

    fn ask_test(question: &str) -> DroneTest {
        DroneTest { fact: "subject".into(), is: None, within: None, over: None, under: None, not: None, ask: Some(question.into()) }
    }

    #[test]
    fn an_ask_test_reads_its_saved_answer() {
        let test = ask_test("The agent is stuck in a loop.");
        let mut facts = BTreeMap::new();
        assert!(!passes(&test, &facts));
        facts.insert(fact_of("The agent is stuck in a loop."), json!(true));
        assert!(passes(&test, &facts));
        let not = DroneTest { not: Some(true), ..test };
        assert!(!passes(&not, &facts));
    }

    #[test]
    fn questions_come_from_live_steps_only() {
        let mut steps = BTreeMap::new();
        steps.insert("a".to_string(), DroneStep::If { label: None, off: None, test: ask_test("Q1?"), then: "w".into(), otherwise: "w".into() });
        steps.insert("b".to_string(), DroneStep::Any { label: None, off: Some(true), tests: vec![ask_test("Q2?")], then: "w".into(), otherwise: "w".into() });
        steps.insert("w".to_string(), DroneStep::Wait { label: None, off: None });
        let tree = DroneTree { version: 1, on: true, every: None, remind_after: None, start: "a".into(), steps };
        assert_eq!(questions(&tree), vec!["Q1?".to_string()]);
    }

    #[test]
    fn the_input_has_facts_and_the_screen_tail() {
        let mut facts = BTreeMap::new();
        facts.insert("subject".to_string(), json!("terminal"));
        facts.insert("terminal.minutesInState".to_string(), json!(12));
        facts.insert(fact_of("x"), json!(true));
        let text = input(&facts, "a\n\nb\n");
        let b = crate::agent::data_block;
        assert_eq!(text, format!("Facts about one task or terminal that Nixa watches:\n{}\nLast lines of the terminal screen:\n{}", b("subject: terminal\nterminal.minutesInState: 12"), b("a\nb")));
        assert!(yes_of(0.9, 0.85) && !yes_of(0.5, 0.85));
    }
}
