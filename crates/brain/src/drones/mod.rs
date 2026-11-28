//! Read-only code watchers for Nixa.
//! Every minute (or `every` minutes) they walk a JSON decision tree over every
//! open non-locked task and live non-locked terminal and report stalls to Nixa.

pub mod asks;
pub mod routes;
pub mod subjects;
#[cfg(test)]
mod tests;
pub mod tools;

use crate::agent;
use crate::app::{App, lock};
use crate::util::{clip_flat, log, now_ms};
use regex::Regex;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::UNIX_EPOCH;

pub const DRONE_MINUTE_MS: u64 = 60_000;
pub const DRONE_REMIND_MIN: u64 = 15;
pub const DRONE_EVERY_MAX: u64 = 60;
pub const DRONE_REMIND_MAX: u64 = 240;
pub const DRONE_MAX_JUMPS: usize = 100;
pub const DRONE_MAX_TOLD: usize = 8;
pub const TREE_FILE: &str = "drone-tree.json";
pub const EDITS_FILE: &str = "drone-edits.jsonl";
pub const PARKED_FILE: &str = "drone-parked.json";
pub const PARKED_NOTE: &str = " It is parked on purpose, so the drones say this once and stay quiet until its status, blocker or parking note changes.";

static ID_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9][a-zA-Z0-9._-]{0,59}$").unwrap());
static SLOT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{([a-zA-Z][a-zA-Z0-9.]*)\}").unwrap());

pub const DRONE_FACTS: &[(&str, &str)] = &[
    ("subject", "task or terminal: what this walk looks at"),
    (
        "rules.waiting",
        "number of answer rules waiting for user approval",
    ),
    ("task.id", "task number"),
    ("task.title", "task title"),
    ("task.status", "inbox, todo, doing, blocked or review"),
    ("task.assigned", "assignee of the task, or empty"),
    (
        "task.hasAssignee",
        "true when the task is assigned to someone",
    ),
    ("task.hasAgent", "true when a terminal holds the task"),
    (
        "task.agentTerminal",
        "terminal id holding the task, or empty",
    ),
    (
        "task.agentState",
        "state of that terminal: working, idle, done, blocked, error, none, closed, or empty",
    ),
    (
        "task.agentMinutesInState",
        "minutes that terminal has been in its state, or null",
    ),
    ("task.blockedOn", "user, task, other, or empty"),
    (
        "task.parked",
        "true when a blocked task is parked on purpose: its newest note about parking says parked",
    ),
    (
        "task.parkedMark",
        "changes when a parked task's status, blocker or parking note changes, or empty",
    ),
    ("task.needsUser", "true when the task waits for the user"),
    (
        "task.minutesSinceUpdate",
        "minutes since the task last changed",
    ),
    ("task.space", "space name of the task"),
    (
        "task.spaceAutoStart",
        "true when the task's space starts todo tasks by itself",
    ),
    (
        "task.spaceHasRoom",
        "true when the task's space has fewer doing tasks than its limit",
    ),
    (
        "task.closedClaims",
        "number of file claims of this task held by closed terminals",
    ),
    ("terminal.id", "terminal id"),
    ("terminal.name", "terminal display name"),
    ("terminal.agent", "agent program in the terminal"),
    ("terminal.where", "space of the terminal"),
    (
        "terminal.state",
        "working, idle, done, blocked, error or none",
    ),
    (
        "terminal.minutesInState",
        "minutes the terminal has been in its state",
    ),
    (
        "terminal.hasTask",
        "true when the terminal holds a doing or blocked task",
    ),
    (
        "terminal.hasClaim",
        "true when the terminal holds a file claim",
    ),
    ("terminal.nixaJob", "true when Nixa gave this terminal work"),
    (
        "terminal.minutesSinceSent",
        "minutes since Nixa's last message to the terminal, or null",
    ),
    (
        "terminal.startedAfterSend",
        "true when the terminal changed state after Nixa's last message",
    ),
];

fn park_word(text: &str) -> Option<bool> {
    let low = text.to_lowercase();
    let words: Vec<&str> = low.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    if words.iter().any(|w| *w == "unparked" || *w == "unpark")
        || words.windows(2).any(|p| p == ["not", "parked"])
        || words.windows(3).any(|p| p == ["no", "longer", "parked"])
    {
        return Some(false);
    }
    words.contains(&"parked").then_some(true)
}

pub fn parked_note(notes: &[(i64, &str)]) -> Option<i64> {
    notes.iter().fold(None, |start, (id, text)| match park_word(text) {
        Some(true) => start.or(Some(*id)),
        Some(false) => None,
        None => start,
    })
}

pub fn known_fact(name: &str) -> bool {
    DRONE_FACTS.iter().any(|(k, _)| *k == name)
}

pub fn facts_map() -> BTreeMap<String, String> {
    DRONE_FACTS
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

pub type FactMap = BTreeMap<String, Value>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DroneTest {
    pub fact: String,
    #[serde(
        default,
        deserialize_with = "deserialize_test_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub is: Option<Value>,
    #[serde(rename = "in", default, skip_serializing_if = "Option::is_none")]
    pub within: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub over: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub under: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<String>,
}

fn deserialize_test_value<'de, D: Deserializer<'de>>(input: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(input).map(Some)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum DroneStep {
    If {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        off: Option<bool>,
        #[serde(flatten)]
        test: DroneTest,
        then: String,
        #[serde(rename = "else")]
        otherwise: String,
    },
    Switch {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        off: Option<bool>,
        fact: String,
        cases: BTreeMap<String, String>,
        default: String,
    },
    All {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        off: Option<bool>,
        tests: Vec<DroneTest>,
        then: String,
        #[serde(rename = "else")]
        otherwise: String,
    },
    Any {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        off: Option<bool>,
        tests: Vec<DroneTest>,
        then: String,
        #[serde(rename = "else")]
        otherwise: String,
    },
    Wait {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        off: Option<bool>,
    },
    Report {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        off: Option<bool>,
        text: String,
    },
}

impl DroneStep {
    pub fn is_off(&self) -> bool {
        match self {
            DroneStep::If { off, .. }
            | DroneStep::Switch { off, .. }
            | DroneStep::All { off, .. }
            | DroneStep::Any { off, .. }
            | DroneStep::Wait { off, .. }
            | DroneStep::Report { off, .. } => off.unwrap_or(false),
        }
    }

    pub fn set_off(&mut self, value: bool) {
        let slot = match self {
            DroneStep::If { off, .. }
            | DroneStep::Switch { off, .. }
            | DroneStep::All { off, .. }
            | DroneStep::Any { off, .. }
            | DroneStep::Wait { off, .. }
            | DroneStep::Report { off, .. } => off,
        };
        *slot = value.then_some(true);
    }

    pub fn label(&self) -> Option<&str> {
        match self {
            DroneStep::If { label, .. }
            | DroneStep::Switch { label, .. }
            | DroneStep::All { label, .. }
            | DroneStep::Any { label, .. }
            | DroneStep::Wait { label, .. }
            | DroneStep::Report { label, .. } => label.as_deref(),
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            DroneStep::If { .. } => "if",
            DroneStep::Switch { .. } => "switch",
            DroneStep::All { .. } => "all",
            DroneStep::Any { .. } => "any",
            DroneStep::Wait { .. } => "wait",
            DroneStep::Report { .. } => "report",
        }
    }

    pub fn next_ids(&self) -> Vec<&str> {
        match self {
            DroneStep::If {
                then, otherwise, ..
            }
            | DroneStep::All {
                then, otherwise, ..
            }
            | DroneStep::Any {
                then, otherwise, ..
            } => vec![then.as_str(), otherwise.as_str()],
            DroneStep::Switch { cases, default, .. } => {
                let mut out: Vec<&str> = cases.values().map(String::as_str).collect();
                out.push(default.as_str());
                out
            }
            DroneStep::Wait { .. } | DroneStep::Report { .. } => Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DroneTree {
    pub version: u32,
    #[serde(default = "default_on")]
    pub on: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remind_after: Option<u64>,
    pub start: String,
    pub steps: BTreeMap<String, DroneStep>,
}

fn default_on() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq)]
pub struct DroneSubject {
    pub kind: &'static str,
    pub key: String,
    pub facts: FactMap,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DroneWalk {
    pub subject: String,
    pub path: Vec<String>,
    pub end: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DroneFinding {
    pub key: String,
    pub subject: String,
    pub step: String,
    pub text: String,
    pub first_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub told_at: Option<u64>,
    pub reminded: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DroneEdit {
    pub at: u64,
    pub who: String,
    pub op: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub before: DroneTree,
    pub after: DroneTree,
}

fn is_scalar(v: &Value) -> bool {
    matches!(
        v,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
    )
}

fn check_test(test: &DroneTest, where_at: &str) -> Result<(), String> {
    if let Some(question) = &test.ask {
        let question = question.trim();
        if question.is_empty() || question.chars().count() > asks::QUESTION_MAX {
            return Err(format!("{where_at}: ask must be a yes or no question of 1 to {} characters.", asks::QUESTION_MAX));
        }
        if test.is.is_some() || test.within.is_some() || test.over.is_some() || test.under.is_some() {
            return Err(format!("{where_at}: an ask test takes no is, in, over or under."));
        }
    }
    if !known_fact(&test.fact) {
        return Err(format!(
            "{where_at}: unknown fact {:?}. Use nixa_drones facts.",
            test.fact
        ));
    }
    if let Some(is) = &test.is
        && !is_scalar(is)
    {
        return Err(format!(
            "{where_at}: is must be a string, number, boolean or null."
        ));
    }
    if let Some(list) = &test.within
        && !list.iter().all(is_scalar)
    {
        return Err(format!("{where_at}: in must be a list of scalar values."));
    }
    if let Some(over) = test.over
        && !over.is_finite()
    {
        return Err(format!("{where_at}: over must be a finite number."));
    }
    if let Some(under) = test.under
        && !under.is_finite()
    {
        return Err(format!("{where_at}: under must be a finite number."));
    }
    Ok(())
}

pub fn check_step(id: &str, step: &DroneStep) -> Result<(), String> {
    if !ID_RE.is_match(id) {
        return Err(format!(
            "Step id {id:?} must use letters, digits, dots, hyphens or underscores."
        ));
    }
    match step {
        DroneStep::If {
            test,
            then,
            otherwise,
            ..
        } => {
            check_test(test, &format!("Step {id}"))?;
            if then.trim().is_empty() {
                return Err(format!("Step {id}: then must name a step id."));
            }
            if otherwise.trim().is_empty() {
                return Err(format!("Step {id}: else must name a step id."));
            }
        }
        DroneStep::All {
            tests,
            then,
            otherwise,
            ..
        }
        | DroneStep::Any {
            tests,
            then,
            otherwise,
            ..
        } => {
            if tests.is_empty() {
                return Err(format!(
                    "Step {id}: tests must be a list with at least one test."
                ));
            }
            for (i, t) in tests.iter().enumerate() {
                check_test(t, &format!("Step {id} test {}", i + 1))?;
            }
            if then.trim().is_empty() {
                return Err(format!("Step {id}: then must name a step id."));
            }
            if otherwise.trim().is_empty() {
                return Err(format!("Step {id}: else must name a step id."));
            }
        }
        DroneStep::Switch {
            fact,
            cases,
            default,
            ..
        } => {
            if !known_fact(fact) {
                return Err(format!(
                    "Step {id}: unknown fact {fact:?}. Use nixa_drones facts."
                ));
            }
            if cases.values().any(|v| v.trim().is_empty()) {
                return Err(format!("Step {id}: cases must map values to step ids."));
            }
            if default.trim().is_empty() {
                return Err(format!("Step {id}: default must name a step id."));
            }
        }
        DroneStep::Wait { .. } => {}
        DroneStep::Report { text, .. } => {
            if text.trim().is_empty() {
                return Err(format!("Step {id}: report text cannot be empty."));
            }
            if text.chars().count() > 400 {
                return Err(format!(
                    "Step {id}: report text is longer than 400 characters."
                ));
            }
            for caps in SLOT_RE.captures_iter(text) {
                let slot = &caps[1];
                if !known_fact(slot) {
                    return Err(format!(
                        "Step {id}: unknown slot {{{slot}}}. Use nixa_drones facts."
                    ));
                }
            }
        }
    }
    Ok(())
}

fn find_loop(
    id: &str,
    steps: &BTreeMap<String, DroneStep>,
    state: &mut HashMap<String, u8>,
    trail: &mut Vec<String>,
) -> Option<Vec<String>> {
    if state.get(id) == Some(&2) {
        return None;
    }
    if state.get(id) == Some(&1) {
        let start = trail.iter().position(|x| x == id).unwrap_or(0);
        let mut cycle = trail[start..].to_vec();
        cycle.push(id.to_string());
        return Some(cycle);
    }
    state.insert(id.to_string(), 1);
    trail.push(id.to_string());
    if let Some(step) = steps.get(id) {
        for next in step.next_ids() {
            if let Some(cycle) = find_loop(next, steps, state, trail) {
                return Some(cycle);
            }
        }
    }
    trail.pop();
    state.insert(id.to_string(), 2);
    None
}

pub fn check_tree(tree: &DroneTree) -> Result<(), String> {
    if tree.version != 1 {
        return Err("The drone tree needs version 1.".into());
    }
    if let Some(every) = tree.every
        && !(1..=DRONE_EVERY_MAX).contains(&every)
    {
        return Err(format!(
            "every must be whole minutes from 1 to {DRONE_EVERY_MAX}."
        ));
    }
    if let Some(remind) = tree.remind_after
        && !(1..=DRONE_REMIND_MAX).contains(&remind)
    {
        return Err(format!(
            "remindAfter must be whole minutes from 1 to {DRONE_REMIND_MAX}."
        ));
    }
    if !tree.steps.contains_key(&tree.start) {
        return Err(format!("Start step {:?} does not exist.", tree.start));
    }
    for (id, step) in &tree.steps {
        check_step(id, step)?;
    }
    for (id, step) in &tree.steps {
        for next in step.next_ids() {
            if !tree.steps.contains_key(next) {
                return Err(format!("Step {id} points to missing step {next:?}."));
            }
        }
    }
    let mut state = HashMap::new();
    let mut trail = Vec::new();
    for id in tree.steps.keys() {
        if let Some(cycle) = find_loop(id, &tree.steps, &mut state, &mut trail) {
            return Err(format!("The steps make a loop: {}.", cycle.join(" -> ")));
        }
    }
    Ok(())
}

fn value_eq(actual: Option<&Value>, expected: &Value) -> bool {
    match actual {
        None | Some(Value::Null) => expected.is_null(),
        Some(v) => v == expected,
    }
}

fn truthy(actual: Option<&Value>) -> bool {
    match actual {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(m)) => !m.is_empty(),
    }
}

pub fn passes(test: &DroneTest, facts: &FactMap) -> bool {
    let val = facts.get(&test.fact);
    let result = if let Some(question) = &test.ask {
        truthy(facts.get(&asks::fact_of(question)))
    } else if let Some(is) = &test.is {
        value_eq(val, is)
    } else if let Some(list) = &test.within {
        list.iter().any(|item| value_eq(val, item))
    } else if test.over.is_some() || test.under.is_some() {
        val.and_then(Value::as_f64)
            .is_some_and(|n| test.over.is_none_or(|o| n > o) && test.under.is_none_or(|u| n < u))
    } else {
        truthy(val)
    };
    if test.not.unwrap_or(false) {
        !result
    } else {
        result
    }
}

fn fact_str(val: Option<&Value>) -> String {
    match val {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

pub fn fill(text: &str, facts: &FactMap) -> String {
    SLOT_RE
        .replace_all(text, |caps: &regex::Captures| fact_str(facts.get(&caps[1])))
        .into_owned()
}

pub fn walk_subject(tree: &DroneTree, subject: &DroneSubject) -> DroneWalk {
    let mut path = Vec::new();
    let mut id = tree.start.clone();
    while path.len() < DRONE_MAX_JUMPS {
        let Some(step) = tree.steps.get(&id) else {
            return DroneWalk {
                subject: subject.key.clone(),
                path,
                end: "error".into(),
                step: None,
                text: None,
                error: Some(format!("Step {id} does not exist.")),
            };
        };
        path.push(id.clone());
        if step.is_off() {
            return DroneWalk {
                subject: subject.key.clone(),
                path,
                end: "off".into(),
                step: Some(id),
                text: None,
                error: None,
            };
        }
        match step {
            DroneStep::Wait { .. } => {
                return DroneWalk {
                    subject: subject.key.clone(),
                    path,
                    end: "wait".into(),
                    step: Some(id),
                    text: None,
                    error: None,
                };
            }
            DroneStep::Report { text, .. } => {
                return DroneWalk {
                    subject: subject.key.clone(),
                    path,
                    end: "report".into(),
                    step: Some(id),
                    text: Some(fill(text, &subject.facts)),
                    error: None,
                };
            }
            DroneStep::If {
                test,
                then,
                otherwise,
                ..
            } => {
                id = if passes(test, &subject.facts) {
                    then.clone()
                } else {
                    otherwise.clone()
                };
            }
            DroneStep::Switch {
                fact,
                cases,
                default,
                ..
            } => {
                let key = fact_str(subject.facts.get(fact));
                id = cases.get(&key).cloned().unwrap_or_else(|| default.clone());
            }
            DroneStep::All {
                tests,
                then,
                otherwise,
                ..
            } => {
                id = if tests.iter().all(|t| passes(t, &subject.facts)) {
                    then.clone()
                } else {
                    otherwise.clone()
                };
            }
            DroneStep::Any {
                tests,
                then,
                otherwise,
                ..
            } => {
                id = if tests.iter().any(|t| passes(t, &subject.facts)) {
                    then.clone()
                } else {
                    otherwise.clone()
                };
            }
        }
    }
    DroneWalk {
        subject: subject.key.clone(),
        path,
        end: "error".into(),
        step: None,
        text: None,
        error: Some(format!("The walk stopped after {DRONE_MAX_JUMPS} steps.")),
    }
}

fn t(fact: &str) -> DroneTest {
    DroneTest {
        fact: fact.into(),
        is: None,
        within: None,
        over: None,
        under: None,
        not: None,
        ask: None,
    }
}

fn t_is(fact: &str, v: Value) -> DroneTest {
    DroneTest {
        is: Some(v),
        ..t(fact)
    }
}

fn t_in(fact: &str, list: &[&str]) -> DroneTest {
    DroneTest {
        within: Some(list.iter().map(|s| json!(s)).collect()),
        ..t(fact)
    }
}

fn t_over(fact: &str, n: f64) -> DroneTest {
    DroneTest {
        over: Some(n),
        ..t(fact)
    }
}

fn t_not(fact: &str) -> DroneTest {
    DroneTest {
        not: Some(true),
        ..t(fact)
    }
}

pub fn default_drone_tree() -> DroneTree {
    let mut steps = BTreeMap::new();
    let mut subject_cases = BTreeMap::new();
    subject_cases.insert("task".into(), "t-claims".into());
    subject_cases.insert("terminal".into(), "c-question".into());
    steps.insert(
        "subject".into(),
        DroneStep::Switch {
            label: Some("Task or terminal".into()),
            off: None,
            fact: "subject".into(),
            cases: subject_cases,
            default: "fine".into(),
        },
    );
    steps.insert(
        "t-claims".into(),
        DroneStep::If {
            label: Some("File claims held by closed terminals".into()),
            off: None,
            test: t_over("task.closedClaims", 0.0),
            then: "t-claims-report".into(),
            otherwise: "a-doing".into(),
        },
    );
    steps.insert("t-claims-report".into(), DroneStep::Report {
        label: None,
        off: None,
        text: "Task #{task.id} \"{task.title}\" has {task.closedClaims} file claim(s) held by closed terminals. Free them or give the task to someone else.".into(),
    });
    steps.insert(
        "a-doing".into(),
        DroneStep::If {
            label: Some("(a) Task is doing".into()),
            off: None,
            test: t_is("task.status", json!("doing")),
            then: "a-agent".into(),
            otherwise: "b-inbox".into(),
        },
    );
    let mut agent_cases = BTreeMap::new();
    agent_cases.insert("working".into(), "fine".into());
    agent_cases.insert("idle".into(), "a-finished".into());
    agent_cases.insert("done".into(), "a-finished".into());
    agent_cases.insert("closed".into(), "a-closed".into());
    agent_cases.insert(String::new(), "a-no-agent".into());
    steps.insert(
        "a-agent".into(),
        DroneStep::Switch {
            label: Some("Its agent's state".into()),
            off: None,
            fact: "task.agentState".into(),
            cases: agent_cases,
            default: "fine".into(),
        },
    );
    steps.insert(
        "a-finished".into(),
        DroneStep::If {
            label: Some("Agent finished for 2 minutes".into()),
            off: None,
            test: t_over("task.agentMinutesInState", 1.0),
            then: "a-finished-report".into(),
            otherwise: "fine".into(),
        },
    );
    steps.insert("a-finished-report".into(), DroneStep::Report {
        label: None,
        off: None,
        text: "Task #{task.id} \"{task.title}\" is still doing, but its agent in terminal {task.agentTerminal} has finished. Check its work, then set review or give the next step.".into(),
    });
    steps.insert("a-closed".into(), DroneStep::Report {
        label: None,
        off: None,
        text: "Task #{task.id} \"{task.title}\" is doing, but its terminal {task.agentTerminal} is closed. Free the claim or give the task to someone else.".into(),
    });
    steps.insert(
        "a-no-agent".into(),
        DroneStep::If {
            label: Some("Doing with no terminal for 10 minutes".into()),
            off: None,
            test: t_over("task.minutesSinceUpdate", 9.0),
            then: "a-no-agent-report".into(),
            otherwise: "fine".into(),
        },
    );
    steps.insert("a-no-agent-report".into(), DroneStep::Report {
        label: None,
        off: None,
        text: "Task #{task.id} \"{task.title}\" is doing, but no terminal holds it. Start it or set it back to todo.".into(),
    });
    steps.insert(
        "b-inbox".into(),
        DroneStep::If {
            label: Some("(b) Task is in the inbox".into()),
            off: None,
            test: t_is("task.status", json!("inbox")),
            then: "b-inbox-nobody".into(),
            otherwise: "b-todo".into(),
        },
    );
    steps.insert(
        "b-inbox-nobody".into(),
        DroneStep::All {
            label: Some("Nobody assigned for 5 minutes".into()),
            off: None,
            tests: vec![
                t_not("task.hasAssignee"),
                t_over("task.minutesSinceUpdate", 4.0),
            ],
            then: "b-inbox-report".into(),
            otherwise: "fine".into(),
        },
    );
    steps.insert("b-inbox-report".into(), DroneStep::Report {
        label: None,
        off: None,
        text: "Task #{task.id} \"{task.title}\" is in the inbox with nobody assigned. Place it: pick an agent and a space.".into(),
    });
    steps.insert(
        "b-todo".into(),
        DroneStep::If {
            label: Some("Task is todo".into()),
            off: None,
            test: t_is("task.status", json!("todo")),
            then: "b-todo-waiting".into(),
            otherwise: "c-blocked".into(),
        },
    );
    steps.insert(
        "b-todo-waiting".into(),
        DroneStep::All {
            label: Some("Assigned, not started, space has room, 30 minutes".into()),
            off: None,
            tests: vec![
                t("task.hasAssignee"),
                t_not("task.hasAgent"),
                t("task.spaceAutoStart"),
                t("task.spaceHasRoom"),
                t_over("task.minutesSinceUpdate", 29.0),
            ],
            then: "b-todo-report".into(),
            otherwise: "fine".into(),
        },
    );
    steps.insert("b-todo-report".into(), DroneStep::Report {
        label: None,
        off: None,
        text: "Task #{task.id} \"{task.title}\" is todo for {task.minutesSinceUpdate} minutes, assigned to {task.assigned}, and no agent started it, though space {task.space} has room.".into(),
    });
    steps.insert(
        "c-blocked".into(),
        DroneStep::If {
            label: Some("(c) Task is blocked".into()),
            off: None,
            test: t_is("task.status", json!("blocked")),
            then: "c-blocked-reason".into(),
            otherwise: "d-review".into(),
        },
    );
    steps.insert(
        "c-blocked-reason".into(),
        DroneStep::If {
            label: Some("Blocked with no reason".into()),
            off: None,
            test: t_is("task.blockedOn", json!("")),
            then: "c-blocked-report".into(),
            otherwise: "c-blocked-long".into(),
        },
    );
    steps.insert(
        "c-blocked-report".into(),
        DroneStep::Report {
            label: None,
            off: None,
            text: "Task #{task.id} \"{task.title}\" is blocked but gives no reason.".into(),
        },
    );
    steps.insert(
        "c-blocked-long".into(),
        DroneStep::All {
            label: Some("Blocked on something other than the user for an hour".into()),
            off: None,
            tests: vec![t_not("task.needsUser"), t_over("task.minutesSinceUpdate", 59.0)],
            then: "c-blocked-long-report".into(),
            otherwise: "fine".into(),
        },
    );
    steps.insert("c-blocked-long-report".into(), DroneStep::Report {
        label: None,
        off: None,
        text: "Task #{task.id} \"{task.title}\" is blocked on {task.blockedOn} for {task.minutesSinceUpdate} minutes. Unblock it, give it to another agent, or ask the deep brain of space {task.space}.".into(),
    });
    steps.insert(
        "d-review".into(),
        DroneStep::If {
            label: Some("(d) Task is in review".into()),
            off: None,
            test: t_is("task.status", json!("review")),
            then: "d-review-long".into(),
            otherwise: "fine".into(),
        },
    );
    steps.insert(
        "d-review-long".into(),
        DroneStep::All {
            label: Some("In review for an hour and not waiting on the user".into()),
            off: None,
            tests: vec![t_not("task.needsUser"), t_over("task.minutesSinceUpdate", 59.0)],
            then: "d-review-report".into(),
            otherwise: "fine".into(),
        },
    );
    steps.insert("d-review-report".into(), DroneStep::Report {
        label: None,
        off: None,
        text: "Task #{task.id} \"{task.title}\" waits in review for {task.minutesSinceUpdate} minutes. Check its proof: close it when it has proof, send a fix when it fails, or give the review to another agent.".into(),
    });
    steps.insert(
        "c-question".into(),
        DroneStep::All {
            label: Some("(c) Terminal waits on a question for 5 minutes".into()),
            off: None,
            tests: vec![
                t_is("terminal.state", json!("blocked")),
                t_over("terminal.minutesInState", 4.0),
            ],
            then: "c-question-ours".into(),
            otherwise: "c-not-started".into(),
        },
    );
    steps.insert(
        "c-question-ours".into(),
        DroneStep::Any {
            label: Some("Nixa job or a task".into()),
            off: None,
            tests: vec![t("terminal.nixaJob"), t("terminal.hasTask")],
            then: "c-question-report".into(),
            otherwise: "c-not-started".into(),
        },
    );
    steps.insert("c-question-report".into(), DroneStep::Report {
        label: None,
        off: None,
        text: "Terminal {terminal.id} ({terminal.agent}) has waited on a question for {terminal.minutesInState} minutes.".into(),
    });
    steps.insert(
        "c-not-started".into(),
        DroneStep::All {
            label: Some("(c) Nixa message did not start work in 3 minutes".into()),
            off: None,
            tests: vec![
                t("terminal.nixaJob"),
                t_not("terminal.startedAfterSend"),
                t_over("terminal.minutesSinceSent", 2.0),
                t_in("terminal.state", &["idle", "done"]),
            ],
            then: "c-not-started-report".into(),
            otherwise: "d-idle".into(),
        },
    );
    steps.insert("c-not-started-report".into(), DroneStep::Report {
        label: None,
        off: None,
        text: "Nixa's last message to terminal {terminal.id} ({terminal.agent}) did not start any work in {terminal.minutesSinceSent} minutes.".into(),
    });
    steps.insert(
        "d-idle".into(),
        DroneStep::All {
            label: Some("(d) Nixa terminal finished, no task or claim, 15 minutes".into()),
            off: None,
            tests: vec![
                t("terminal.nixaJob"),
                t_not("terminal.hasTask"),
                t_not("terminal.hasClaim"),
                t_in("terminal.state", &["idle", "done"]),
                t_over("terminal.minutesInState", 14.0),
            ],
            then: "d-idle-report".into(),
            otherwise: "fine".into(),
        },
    );
    steps.insert("d-idle-report".into(), DroneStep::Report {
        label: None,
        off: None,
        text: "Terminal {terminal.id} ({terminal.agent}) finished and holds no task or file claim for {terminal.minutesInState} minutes. It can be closed.".into(),
    });
    steps.insert(
        "fine".into(),
        DroneStep::Wait {
            label: Some("Nothing to report".into()),
            off: None,
        },
    );
    DroneTree {
        version: 1,
        on: true,
        every: None,
        remind_after: None,
        start: "subject".into(),
        steps,
    }
}

pub struct DroneStore {
    pub file: PathBuf,
    pub edits_file: PathBuf,
    pub tree: DroneTree,
    loaded_mtime: u128,
    pub file_error: Option<String>,
    pub walks: Vec<DroneWalk>,
    pub last_tick: Option<u64>,
    pub findings: BTreeMap<String, DroneFinding>,
    pub parked_file: PathBuf,
    pub parked_told: BTreeMap<String, String>,
}

fn mtime_of(path: &Path) -> u128 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

impl DroneStore {
    pub fn open(dir: &Path) -> DroneStore {
        let mut store = DroneStore {
            file: dir.join(TREE_FILE),
            edits_file: dir.join(EDITS_FILE),
            tree: default_drone_tree(),
            loaded_mtime: 0,
            file_error: None,
            walks: Vec::new(),
            last_tick: None,
            findings: BTreeMap::new(),
            parked_file: dir.join(PARKED_FILE),
            parked_told: fs::read_to_string(dir.join(PARKED_FILE)).ok().and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or_default(),
        };
        store.load();
        store
    }

    pub fn every_minutes(&self) -> u64 {
        self.tree.every.unwrap_or(1)
    }

    pub fn remind_minutes(&self) -> u64 {
        self.tree.remind_after.unwrap_or(DRONE_REMIND_MIN)
    }

    fn write_tree(&mut self, next: DroneTree) -> Result<(), String> {
        if let Some(parent) = self.file.parent() {
            nixa_protocol::paths::make_private_dirs(parent)
                .map_err(|e| format!("Could not create drones folder: {e}"))?;
        }
        let body = serde_json::to_string_pretty(&next).map_err(|e| e.to_string())?;
        crate::util::atomic_write(&self.file, &format!("{body}\n"))
            .map_err(|e| format!("Could not save drone tree: {e}"))?;
        self.loaded_mtime = mtime_of(&self.file);
        self.tree = next;
        self.file_error = None;
        Ok(())
    }

    pub fn load(&mut self) {
        if !self.file.exists() {
            if let Err(error) = self.write_tree(default_drone_tree()) {
                self.file_error = Some(error);
            }
            return;
        }
        let mtime = mtime_of(&self.file);
        if mtime != 0 && mtime == self.loaded_mtime {
            return;
        }
        self.loaded_mtime = mtime;
        let raw = match fs::read_to_string(&self.file) {
            Ok(raw) => raw,
            Err(error) => {
                self.file_error = Some(format!(
                    "Could not read drone-tree.json: {error}. The last good tree is used."
                ));
                return;
            }
        };
        let parsed: DroneTree = match serde_json::from_str(&raw) {
            Ok(tree) => tree,
            Err(error) => {
                self.file_error = Some(format!(
                    "The tree file is not valid JSON: {error}. The last good tree is used."
                ));
                return;
            }
        };
        if let Err(error) = check_tree(&parsed) {
            self.file_error = Some(format!("{error} The last good tree is used."));
            return;
        }
        self.tree = parsed;
        self.file_error = None;
    }

    fn save_parked(&self) {
        let body = serde_json::to_string_pretty(&self.parked_told).unwrap_or_default();
        if let Err(error) = crate::util::atomic_write(&self.parked_file, &format!("{body}\n")) {
            log("drones", &format!("Could not save {PARKED_FILE}: {error}"));
        }
    }

    fn read_edits(&self) -> Vec<DroneEdit> {
        let Ok(raw) = fs::read_to_string(&self.edits_file) else {
            return Vec::new();
        };
        raw.lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str::<DroneEdit>(l).ok())
            .collect()
    }

    fn persist_edit(&mut self, edit: DroneEdit) -> Result<(), String> {
        let line = serde_json::to_string(&edit).map_err(|e| e.to_string())?;
        let log_error = |e| format!("Could not write drone edit log: {e}");
        let mut log = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&self.edits_file)
            .map_err(log_error)?;
        let log_len = log.metadata().map_err(log_error)?.len();
        self.write_tree(edit.after)?;
        if let Err(error) = log.write_all(format!("{line}\n").as_bytes()) {
            let tree_rollback = self.write_tree(edit.before);
            let log_rollback = log.set_len(log_len).map_err(log_error);
            return match (tree_rollback, log_rollback) {
                (Ok(()), Ok(())) => Err(log_error(error)),
                (tree, log) => Err(format!(
                    "{}. Rollback errors: {}",
                    log_error(error),
                    tree.err()
                        .into_iter()
                        .chain(log.err())
                        .collect::<Vec<_>>()
                        .join("; ")
                )),
            };
        }
        Ok(())
    }

    fn save_edit(
        &mut self,
        op: &str,
        who: &str,
        next: DroneTree,
        id: Option<String>,
        now: u64,
    ) -> Result<DroneTree, String> {
        check_tree(&next)?;
        self.persist_edit(DroneEdit {
            at: now,
            who: who.into(),
            op: op.into(),
            id,
            before: self.tree.clone(),
            after: next.clone(),
        })?;
        Ok(next)
    }

    pub fn add_step(
        &mut self,
        id: &str,
        step: DroneStep,
        who: &str,
        now: u64,
    ) -> Result<DroneTree, String> {
        self.load();
        if self.tree.steps.contains_key(id) {
            return Err(format!("Step {id} already exists. Use change instead."));
        }
        let mut next = self.tree.clone();
        next.steps.insert(id.to_string(), step);
        self.save_edit("add", who, next, Some(id.to_string()), now)
    }

    pub fn change_step(
        &mut self,
        id: &str,
        step: DroneStep,
        who: &str,
        now: u64,
    ) -> Result<DroneTree, String> {
        self.load();
        if !self.tree.steps.contains_key(id) {
            return Err(format!("Step {id} does not exist. Use add instead."));
        }
        let mut next = self.tree.clone();
        next.steps.insert(id.to_string(), step);
        self.save_edit("change", who, next, Some(id.to_string()), now)
    }

    pub fn remove_step(&mut self, id: &str, who: &str, now: u64) -> Result<DroneTree, String> {
        self.load();
        if !self.tree.steps.contains_key(id) {
            return Err(format!("Step {id} does not exist."));
        }
        if id == self.tree.start {
            return Err(format!(
                "Step {id} is the start step and cannot be removed."
            ));
        }
        let users: Vec<&str> = self
            .tree
            .steps
            .iter()
            .filter(|(_, s)| s.next_ids().contains(&id))
            .map(|(name, _)| name.as_str())
            .collect();
        if !users.is_empty() {
            return Err(format!(
                "Step {id} is used by {}. Change those first.",
                users.join(", ")
            ));
        }
        let mut next = self.tree.clone();
        next.steps.remove(id);
        self.save_edit("remove", who, next, Some(id.to_string()), now)
    }

    pub fn set_on(
        &mut self,
        on: bool,
        who: &str,
        id: Option<&str>,
        now: u64,
    ) -> Result<DroneTree, String> {
        self.load();
        let mut next = self.tree.clone();
        if let Some(id) = id {
            let step = next
                .steps
                .get_mut(id)
                .ok_or_else(|| format!("Step {id} does not exist."))?;
            step.set_off(!on);
        } else {
            next.on = on;
        }
        self.save_edit(
            if on { "on" } else { "off" },
            who,
            next,
            id.map(str::to_string),
            now,
        )
    }

    pub fn set_timing(
        &mut self,
        who: &str,
        every: Option<u64>,
        remind: Option<u64>,
        now: u64,
    ) -> Result<DroneTree, String> {
        self.load();
        if every.is_none() && remind.is_none() {
            return Err("timing needs every or remind, in minutes.".into());
        }
        let mut next = self.tree.clone();
        if let Some(every) = every {
            next.every = Some(every);
        }
        if let Some(remind) = remind {
            next.remind_after = Some(remind);
        }
        self.save_edit("timing", who, next, None, now)
    }

    pub fn undo(&mut self, who: &str, now: u64) -> Result<DroneEdit, String> {
        let edits = self.read_edits();
        let undone: HashSet<String> = edits
            .iter()
            .filter(|e| e.op == "undo")
            .filter_map(|e| e.id.clone())
            .collect();
        let index = edits
            .iter()
            .enumerate()
            .rev()
            .find(|(at, e)| e.op != "undo" && !undone.contains(&at.to_string()))
            .map(|(at, _)| at)
            .ok_or_else(|| "There is no edit to undo.".to_string())?;
        let edit = edits[index].clone();
        check_tree(&edit.before).map_err(|e| format!("The old tree is not valid: {e}"))?;
        self.load();
        self.persist_edit(DroneEdit {
            at: now,
            who: who.into(),
            op: "undo".into(),
            id: Some(index.to_string()),
            before: self.tree.clone(),
            after: edit.before.clone(),
        })?;
        Ok(edit)
    }

    pub fn history(&self, limit: usize) -> Vec<Value> {
        let edits = self.read_edits();
        let start = edits.len().saturating_sub(limit);
        edits[start..]
            .iter()
            .map(|e| {
                let mut obj = json!({"at": e.at, "who": e.who, "op": e.op});
                if let Some(id) = &e.id {
                    obj["id"] = json!(id);
                }
                obj
            })
            .collect()
    }

    pub fn dry_run(
        &mut self,
        subjects: &[DroneSubject],
        change: Option<(&str, DroneStep)>,
    ) -> Result<Vec<DroneWalk>, String> {
        self.load();
        let tree = if let Some((id, step)) = change {
            let mut next = self.tree.clone();
            next.steps.insert(id.to_string(), step);
            check_tree(&next)?;
            next
        } else {
            self.tree.clone()
        };
        if !tree.on {
            return Ok(Vec::new());
        }
        Ok(subjects.iter().map(|s| walk_subject(&tree, s)).collect())
    }

    pub fn due(&mut self, now: u64, force: bool) -> bool {
        self.load();
        if force {
            return true;
        }
        match self.last_tick {
            None => true,
            Some(last) => {
                now.saturating_sub(last) >= self.every_minutes() * DRONE_MINUTE_MS - 5_000
            }
        }
    }

    /// Walk all subjects, update findings, and return any report message ready to send.
    pub fn record_walk(
        &mut self,
        subjects: &[DroneSubject],
        now: u64,
        can_report: bool,
    ) -> (Vec<DroneWalk>, Option<String>) {
        self.load();
        let walks: Vec<DroneWalk> = if self.tree.on {
            subjects
                .iter()
                .map(|s| walk_subject(&self.tree, s))
                .collect()
        } else {
            Vec::new()
        };
        self.walks = walks.clone();
        self.last_tick = Some(now);
        let parked: HashMap<String, String> = subjects
            .iter()
            .filter(|s| s.facts.get("task.parked").and_then(Value::as_bool) == Some(true))
            .map(|s| (s.key.clone(), fact_str(s.facts.get("task.parkedMark"))))
            .collect();
        let told_before = self.parked_told.len();
        self.parked_told.retain(|k, mark| parked.get(k) == Some(mark));
        if self.parked_told.len() != told_before {
            self.save_parked();
        }
        let mut seen = HashSet::new();
        for walk in &walks {
            if walk.end == "error"
                && let Some(err) = &walk.error
            {
                log(
                    "drones",
                    &format!("walk failed for {}: {err}", walk.subject),
                );
            }
            if walk.end != "report" {
                continue;
            }
            let (Some(step), Some(text)) = (&walk.step, &walk.text) else {
                continue;
            };
            if parked.contains_key(&walk.subject) && self.parked_told.contains_key(&walk.subject) {
                continue;
            }
            let text = &if parked.contains_key(&walk.subject) { format!("{text}{PARKED_NOTE}") } else { text.clone() };
            let key = if parked.contains_key(&walk.subject) { format!("{}|{step}|parked", walk.subject) } else { format!("{}|{step}", walk.subject) };
            seen.insert(key.clone());
            if let Some(finding) = self.findings.get_mut(&key) {
                finding.text.clone_from(text);
            } else {
                self.findings.insert(
                    key.clone(),
                    DroneFinding {
                        key,
                        subject: walk.subject.clone(),
                        step: step.clone(),
                        text: text.clone(),
                        first_at: now,
                        told_at: None,
                        reminded: false,
                    },
                );
            }
        }
        self.findings.retain(|k, _| seen.contains(k));
        if !can_report {
            return (walks, None);
        }
        let remind_ms = self.remind_minutes() * DRONE_MINUTE_MS;
        let fresh_keys: Vec<String> = self
            .findings
            .values()
            .filter(|f| f.told_at.is_none())
            .map(|f| f.key.clone())
            .collect();
        let again_keys: Vec<String> = self
            .findings
            .values()
            .filter(|f| {
                f.told_at
                    .is_some_and(|at| !f.reminded && now.saturating_sub(at) >= remind_ms)
            })
            .map(|f| f.key.clone())
            .collect();
        if fresh_keys.is_empty() && again_keys.is_empty() {
            return (walks, None);
        }
        let send_fresh: Vec<String> = fresh_keys.iter().take(DRONE_MAX_TOLD).cloned().collect();
        let left = DRONE_MAX_TOLD.saturating_sub(send_fresh.len());
        let send_again: Vec<String> = again_keys.iter().take(left).cloned().collect();
        let more = fresh_keys.len() + again_keys.len() - send_fresh.len() - send_again.len();

        let mut lines = vec![
            "Event: Drone report. Drones are read-only code watchers; they changed nothing."
                .to_string(),
        ];
        if !send_fresh.is_empty() {
            lines.push("New:".into());
            for k in &send_fresh {
                if let Some(f) = self.findings.get(k) {
                    lines.push(format!("- {}", clip_flat(&f.text, 240)));
                }
            }
        }
        if !send_again.is_empty() {
            lines.push(format!(
                "Reminder, no change for {} minutes:",
                self.remind_minutes()
            ));
            for k in &send_again {
                if let Some(f) = self.findings.get(k) {
                    lines.push(format!("- {}", clip_flat(&f.text, 240)));
                }
            }
        }
        if more > 0 {
            lines.push(format!(
                "{more} more wait for the next report (nixa_drones read shows all)."
            ));
        }
        lines.push("Handle each one by the deep brain rules. nixa_drones read shows the path each drone walked.".into());

        let mut parked_changed = false;
        for k in &send_fresh {
            if let Some(f) = self.findings.get_mut(k) {
                f.told_at = Some(now);
                if let Some(mark) = parked.get(&f.subject) {
                    self.parked_told.insert(f.subject.clone(), mark.clone());
                    parked_changed = true;
                }
            }
        }
        if parked_changed {
            self.save_parked();
        }
        for k in &send_again {
            if let Some(f) = self.findings.get_mut(k) {
                f.reminded = true;
            }
        }
        (walks, Some(lines.join("\n")))
    }

    pub fn read_view(&mut self) -> Value {
        self.load();
        let results: Vec<Value> = self
            .walks
            .iter()
            .filter(|w| w.end != "wait")
            .take(60)
            .map(walk_line)
            .collect();
        let quiet = self.walks.iter().filter(|w| w.end == "wait").count();
        let findings: Vec<Value> = self
            .findings
            .values()
            .map(|f| {
                json!({
                    "text": f.text,
                    "step": f.step,
                    "subject": f.subject,
                    "at": f.first_at,
                    "told": f.told_at.is_some(),
                    "reminded": f.reminded,
                })
            })
            .collect();
        let mut out = json!({
            "on": self.tree.on,
            "every": self.every_minutes(),
            "remindAfter": self.remind_minutes(),
            "file": self.file.display().to_string(),
            "tree": self.tree,
            "lastTick": self.last_tick,
            "subjects": self.walks.len(),
            "results": results,
            "quiet": quiet,
            "findings": findings,
        });
        if let Some(err) = &self.file_error {
            out["error"] = json!(err);
        }
        out
    }

    pub fn snapshot(&mut self) -> Value {
        self.load();
        let findings: Vec<Value> = self
            .findings
            .values()
            .take(30)
            .map(|f| {
                json!({
                    "text": f.text,
                    "step": f.step,
                    "subject": f.subject,
                    "at": f.first_at,
                    "told": f.told_at.is_some(),
                    "reminded": f.reminded,
                })
            })
            .collect();
        let mut out = json!({
            "on": self.tree.on,
            "start": self.tree.start,
            "steps": self.tree.steps,
            "findings": findings,
            "lastTick": self.last_tick,
            "subjects": self.walks.len(),
            "every": self.every_minutes(),
            "remindAfter": self.remind_minutes(),
        });
        if let Some(err) = &self.file_error {
            out["error"] = json!(err);
        }
        out
    }
}

pub fn walk_line(walk: &DroneWalk) -> Value {
    let mut obj = json!({
        "subject": walk.subject,
        "end": walk.end,
        "path": walk.path.join(" > "),
    });
    if let Some(text) = &walk.text {
        obj["text"] = json!(text);
    }
    if let Some(err) = &walk.error {
        obj["error"] = json!(err);
    }
    obj
}

pub fn results_json(walks: &[DroneWalk]) -> Value {
    let non_wait: Vec<Value> = walks
        .iter()
        .filter(|w| w.end != "wait")
        .take(60)
        .map(walk_line)
        .collect();
    let quiet = walks.iter().filter(|w| w.end == "wait").count();
    json!({"walked": walks.len(), "quiet": quiet, "results": non_wait})
}

pub struct State {
    pub store: Mutex<DroneStore>,
    pub asks: Mutex<asks::Book>,
}

impl State {
    pub fn open(dir: &Path) -> State {
        State {
            store: Mutex::new(DroneStore::open(dir)),
            asks: Mutex::new(asks::Book::default()),
        }
    }
}

pub fn broadcast_changed(app: &App) {
    let snap = lock(&app.drones.store).snapshot();
    app.send_out(json!({"type": "drones_changed", "drones": snap}));
}

fn subjects_error(error: anyhow::Error) -> String {
    format!("Could not collect drone subjects: {error:#}")
}

fn run_tick(app: &Arc<App>, force: bool) -> Result<Vec<DroneWalk>, String> {
    let now = now_ms();
    if !lock(&app.drones.store).due(now, force) {
        return Ok(lock(&app.drones.store).walks.clone());
    }
    let mut list = subjects::collect(app).map_err(subjects_error)?;
    let tree = lock(&app.drones.store).tree.clone();
    asks::fill(app, &tree, &mut list, now);
    let can_report = !agent::busy(app) && agent::has_brain(app);
    let (walks, msg) = lock(&app.drones.store).record_walk(&list, now, can_report);
    if let Some(msg) = msg {
        log("drones", "reporting findings to brain");
        crate::watch::tell_event(app, &msg);
    }
    broadcast_changed(app);
    Ok(walks)
}

pub fn tick(app: &Arc<App>) {
    if crate::settings_api::nixa_on(&app.dir)
        && let Err(error) = run_tick(app, false)
    {
        log("drones", &error);
    }
}
