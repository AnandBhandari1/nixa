use super::{self as decide, Answer, Answers};
use crate::app::{App, lock};
use crate::deep::Ask;
use crate::util::{clip, log, now_ms};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Mutex};

pub const PICK: &str = "pick";
pub const FLOW: &str = "workflow";
pub const NO_FLOW: &str = "none";
pub const KINDS: [(&str, &str); 3] = [("close", "followup"), (crate::extras::model_pick::ASK_KIND, "routing"), (crate::workflows::driver::ASK_KIND, "routing")];
const FLOW_LINE: &str = "Workflows (add";
const TRIED_MAX: usize = 500;

static TRIED: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

pub fn use_of(kind: Option<&str>) -> Option<&'static str> {
    let kind = kind?;
    KINDS.iter().find(|(k, _)| *k == kind).map(|(_, u)| *u)
}

pub fn questions(ask: &Ask, flows: &[String]) -> Vec<Value> {
    let numbers: Vec<String> = (1..=ask.options.len()).map(|n| n.to_string()).collect();
    let choices: Vec<(&str, &str)> = numbers.iter().zip(ask.options.iter()).map(|(n, o)| (n.as_str(), o.as_str())).collect();
    let mut out = vec![decide::choice(PICK, &clip(&ask.question, 600), &choices)];
    if !flows.is_empty() {
        let mut names: Vec<(&str, &str)> = vec![(NO_FLOW, "no workflow fits; use the normal flow")];
        names.extend(flows.iter().map(|f| (f.as_str(), "")));
        out.push(decide::choice(FLOW, "Which workflow fits this task, if any?", &names));
    }
    out
}

pub fn input(ask: &Ask) -> String {
    let notes = ask.notes.trim();
    if notes.is_empty() { ask.question.clone() } else { format!("{}\n{}", ask.question, crate::agent::data_block(&clip(notes, 2400))) }
}

fn choice_of(answers: &Answers, name: &str) -> Option<(String, f64)> {
    match answers.get(name) {
        Some(Answer::Choice { choice, confidence, .. }) => Some((choice.as_str().map(str::to_string).unwrap_or_else(|| choice.to_string()), *confidence)),
        _ => None,
    }
}

pub fn chosen(answers: &Answers, ask: &Ask, flows: &[String], level: f64) -> Result<String, String> {
    let (number, confidence) = choice_of(answers, PICK).ok_or("it gave no pick")?;
    if confidence < level {
        return Err(format!("it was not sure enough: pick {number} ({confidence:.2}), level {level:.2}"));
    }
    let option = number.trim().parse::<usize>().ok().and_then(|n| n.checked_sub(1)).and_then(|i| ask.options.get(i)).ok_or(format!("pick {number} is not an option"))?;
    if crate::deep::says_ask_user(option) {
        return Err("it picked the ask the user option, which stays with the deep brain".into());
    }
    let mut pick = option.clone();
    if !flows.is_empty() {
        let (flow, sure) = choice_of(answers, FLOW).ok_or("it gave no workflow answer")?;
        if sure < level {
            return Err(format!("it was not sure enough about the workflow: {flow} ({sure:.2})"));
        }
        if flow != NO_FLOW && flows.contains(&flow) {
            pick = format!("{pick} workflow {flow}");
        }
    }
    Ok(pick)
}

fn flows_for(app: &App, ask: &Ask) -> Vec<String> {
    if ask.kind.as_deref() != Some(crate::extras::model_pick::ASK_KIND) || !ask.notes.contains(FLOW_LINE) {
        return Vec::new();
    }
    crate::workflows::open_db(app).map(|conn| crate::workflows::pick::usable_names(&conn)).unwrap_or_default()
}

pub fn scan(app: &Arc<App>) {
    let open: Vec<Ask> = lock(&app.deep).open_asks().into_iter().filter(|a| use_of(a.kind.as_deref()).is_some()).cloned().collect();
    for ask in open {
        let new = {
            let mut tried = lock(&TRIED);
            if tried.len() > TRIED_MAX {
                tried.clear();
            }
            tried.insert(ask.id.clone())
        };
        if new {
            first(app, &ask);
        }
    }
}

pub fn first(app: &Arc<App>, ask: &Ask) {
    let Some(name) = use_of(ask.kind.as_deref()) else { return };
    if ask.options.len() < 2 || !crate::settings_api::nixa_on(&app.dir) {
        return;
    }
    let set = decide::settings::load(app);
    if !set.uses(name) {
        return;
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else { return };
    let app = app.clone();
    let ask = ask.clone();
    handle.spawn(async move {
        let flows = flows_for(&app, &ask);
        let result = match decide::ask(&app, &set.model, vec![decide::text(&input(&ask))], questions(&ask, &flows)).await {
            Ok(answers) => chosen(&answers, &ask, &flows, set.level).map(|pick| (pick, answers)),
            Err(e) => Err(e),
        };
        let (pick, answers) = match result {
            Ok(found) => found,
            Err(why) => {
                log("decide", &format!("{} ({}) goes to the deep brain: {why}", ask.id, ask.kind.as_deref().unwrap_or_default()));
                return;
            }
        };
        let note = answers.answers.iter().map(Answer::short).collect::<Vec<_>>().join(", ");
        let why = format!("Decided by the decision model {} ({note}).", set.model);
        let done = lock(&app.deep).model_reply(&ask.id, &pick, &why, now_ms());
        match done {
            Ok((_, actions)) => {
                log("decide", &format!("{} answered by {}: {}", ask.id, set.model, clip(&pick, 120)));
                crate::watch::run_actions(&app, actions).await;
            }
            Err(e) => log("decide", &format!("{} was not answered by the model: {e}", ask.id)),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ask(kind: &str, options: &[&str]) -> Ask {
        Ask { id: "d1".into(), question: "Close it?".into(), options: options.iter().map(|o| o.to_string()).collect(), notes: "facts".into(), at: 0, state: crate::deep::AskState::Waiting, tried: vec![], kind: Some(kind.into()), wait: None, space: "s".into(), space_name: "s".into(), want: None, brain: None, terminal: None, sent_at: None, done_at: None, nudged: false, pick: None, why: None, learn: None }
    }

    fn pick(number: &str, confidence: f64) -> Answer {
        Answer::Choice { name: PICK.into(), choice: json!(number), confidence, probabilities: Vec::new() }
    }

    #[test]
    fn only_fixed_choice_kinds_go_to_the_model() {
        assert_eq!(use_of(Some("close")), Some("followup"));
        assert_eq!(use_of(Some("model")), Some("routing"));
        assert_eq!(use_of(Some("workflow")), Some("routing"));
        assert_eq!(use_of(Some("fix")), None);
        assert_eq!(use_of(Some("blocker")), None);
        assert_eq!(use_of(None), None);
    }

    #[test]
    fn a_sure_pick_becomes_the_option_text() {
        let a = ask("close", &["close it", "keep it open"]);
        let sure = Answers { answers: vec![pick("1", 0.93)], ..Answers::default() };
        assert_eq!(chosen(&sure, &a, &[], 0.85).unwrap(), "close it");
        let unsure = Answers { answers: vec![pick("1", 0.6)], ..Answers::default() };
        assert!(chosen(&unsure, &a, &[], 0.85).is_err());
        let wrong = Answers { answers: vec![pick("7", 0.99)], ..Answers::default() };
        assert!(chosen(&wrong, &a, &[], 0.85).is_err());
        let user = ask("close", &["close it", "ask the user"]);
        let to_user = Answers { answers: vec![pick("2", 0.99)], ..Answers::default() };
        assert!(chosen(&to_user, &user, &[], 0.85).is_err());
    }

    #[test]
    fn a_workflow_is_added_only_when_sure() {
        let a = ask("model", &["row 1: Build, claude::sonnet", "row 2: Docs, codex::gpt"]);
        let flows = vec!["tdd".to_string()];
        let flow = |name: &str, c: f64| Answer::Choice { name: FLOW.into(), choice: json!(name), confidence: c, probabilities: Vec::new() };
        let with = Answers { answers: vec![pick("2", 0.9), flow("tdd", 0.9)], ..Answers::default() };
        assert_eq!(chosen(&with, &a, &flows, 0.85).unwrap(), "row 2: Docs, codex::gpt workflow tdd");
        let none = Answers { answers: vec![pick("2", 0.9), flow(NO_FLOW, 0.9)], ..Answers::default() };
        assert_eq!(chosen(&none, &a, &flows, 0.85).unwrap(), "row 2: Docs, codex::gpt");
        let unsure = Answers { answers: vec![pick("2", 0.9), flow("tdd", 0.5)], ..Answers::default() };
        assert!(chosen(&unsure, &a, &flows, 0.85).is_err());
        assert_eq!(questions(&a, &flows).len(), 2);
        assert_eq!(questions(&a, &[]).len(), 1);
    }
}
