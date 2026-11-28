use nixa_store::agents::REVIEWER_ROLE;
use nixa_store::review::{HandoffMove, HandoffState, MAX_HANDOFFS, PICKUP_ROLE, custom_ref, in_chain};
use serde::{Deserialize, Serialize};

pub const HANDOFF_START_KEY: &str = "handoff_start:";
pub const HANDOFF_PREFIX: &str = "Hand-off from ";
pub const REPORT_LIMIT: usize = 2500;
pub const FIX_ROUNDS: u32 = 2;
pub const CUSTOM_ROLE: &str = "custom-agent";

const FIX_AFTER: [(&str, &str); 4] = [
    ("code-reviewer", "developer"),
    ("security-reviewer", "senior-developer"),
    ("architect", "developer"),
    ("qa-tester", "developer"),
];

const ESCALATE_TO: [(&str, &str); 2] = [("junior-developer", "developer"), ("developer", "senior-developer")];

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Person {
    pub id: String,
    pub name: String,
    pub role: String,
    pub kind: String,
    pub status: String,
}

pub fn handoff_start_key(task: i64) -> String {
    format!("{HANDOFF_START_KEY}{task}")
}

pub fn fix_after(role: &str) -> Option<&'static str> {
    FIX_AFTER.iter().find(|(from, _)| *from == role).map(|(_, to)| *to)
}

pub fn escalate_to(role: &str) -> Option<&'static str> {
    ESCALATE_TO.iter().find(|(from, _)| *from == role).map(|(_, to)| *to)
}

pub fn is_review_role(role: &str) -> bool {
    fix_after(role).is_some()
}

pub fn member_for<'a>(people: &'a [Person], wanted: &str) -> Option<&'a Person> {
    let usable: Vec<&Person> = people.iter().filter(|p| p.kind != "person" && p.status != "bench").collect();
    let key = wanted.trim().to_lowercase();
    if key.is_empty() {
        return None;
    }
    usable
        .iter()
        .find(|p| p.id.to_lowercase() == key)
        .or_else(|| usable.iter().find(|p| p.role.to_lowercase() == key))
        .or_else(|| usable.iter().find(|p| p.name.to_lowercase() == key))
        .copied()
}

pub fn custom_person(id: &str, title: &str, role: &str, reviewer: bool) -> Person {
    let role = if reviewer || role == REVIEWER_ROLE { PICKUP_ROLE } else { CUSTOM_ROLE };
    Person { id: custom_ref(id), name: title.to_string(), role: role.to_string(), kind: "agent".into(), status: String::new() }
}

fn found(findings: Option<u32>) -> String {
    match findings {
        None => "did not say how many problems it found (no FINDINGS line), so Nixa treats it as problems found".into(),
        Some(1) => "found 1 problem".into(),
        Some(n) => format!("found {n} problems"),
    }
}

fn first_of<'a>(choices: &[&'a str]) -> &'a str {
    choices.iter().copied().find(|c| !c.trim().is_empty()).unwrap_or("")
}

fn pass(to: &str, why: String, state: HandoffState) -> HandoffMove {
    HandoffMove::Pass { to: to.to_string(), why, state }
}

fn to_user(why: String) -> HandoffMove {
    HandoffMove::User { why }
}

pub fn after_step(member: &Person, then: &str, findings: Option<u32>, state: &HandoffState) -> HandoffMove {
    let too_many = state.steps >= MAX_HANDOFFS;
    let name = &member.name;
    let steps = state.steps;
    if is_review_role(&member.role) {
        if findings == Some(0) {
            return HandoffMove::Done;
        }
        let seen = found(findings);
        let default_fixer = fix_after(&member.role).unwrap_or("developer");
        if state.recheck {
            let rounds = state.rounds.max(1);
            if rounds >= FIX_ROUNDS {
                return to_user(format!("The re-check by {name} {seen} after {rounds} fix rounds."));
            }
            if too_many {
                return to_user(format!("The re-check by {name} {seen} after the fix, and this task has already passed between members {steps} times."));
            }
            let fixer = first_of(&[&state.fixer, then, default_fixer]);
            let up = if state.escalated { None } else { escalate_to(fixer) };
            let to = up.unwrap_or(fixer);
            let mut next = state.clone();
            next.steps += 1;
            next.fixer = to.to_string();
            next.recheck = false;
            next.rounds = rounds + 1;
            next.escalated = state.escalated || up.is_some();
            next.fails = 0;
            let level = if up.is_some() { ", one level up" } else { "" };
            return pass(to, format!("The re-check by {name} {seen} after the fix. Nixa gives it one more fix round{level}."), next);
        }
        if too_many {
            return to_user(format!("The review by {name} {seen}, and this task has already passed between members {steps} times."));
        }
        let to = first_of(&[then, &state.back, default_fixer]);
        let mut next = state.clone();
        next.steps += 1;
        next.reviewer = member.id.clone();
        next.fixer = to.to_string();
        next.recheck = false;
        return pass(to, format!("The review by {name} {seen}."), next);
    }
    if !state.reviewer.is_empty() && !state.recheck && state.reviewer != member.id {
        if too_many {
            return to_user(format!("{name} finished the fix, and this task has already passed between members {steps} times."));
        }
        let mut next = state.clone();
        next.steps += 1;
        next.recheck = true;
        next.fails = 0;
        return pass(&state.reviewer, format!("{name} finished the fix. The reviewer checks it once."), next);
    }
    if !then.trim().is_empty() {
        if too_many {
            return to_user(format!("{name} finished, and this task has already passed between members {steps} times."));
        }
        let mut next = state.clone();
        next.steps += 1;
        next.fails = 0;
        return pass(then.trim(), format!("{name} finished this step."), next);
    }
    HandoffMove::Done
}

pub fn escalation(member: &Person, state: &HandoffState, why: &str) -> Option<HandoffMove> {
    let to = escalate_to(&member.role)?;
    if !in_chain(state) || state.escalated || state.recheck {
        return None;
    }
    let name = &member.name;
    if state.steps >= MAX_HANDOFFS {
        return Some(to_user(format!("{name} {why}, and this task has already passed between members {} times.", state.steps)));
    }
    let mut next = state.clone();
    next.steps += 1;
    next.fixer = to.to_string();
    next.escalated = true;
    next.fails = 0;
    Some(pass(to, format!("{name} {why}."), next))
}

pub fn reviewer_lines(member: Option<&Person>, state: &HandoffState) -> Vec<String> {
    if !member.is_some_and(|m| is_review_role(&m.role)) {
        return Vec::new();
    }
    let first = if state.recheck {
        "- You only review. This is the re-check of a fix: check that each finding from the earlier review is fixed, and look for new problems the fix made."
    } else {
        "- You only review. Do not change code. Write each finding as a numbered line with the file and what is wrong."
    };
    vec![
        first.to_string(),
        "- End your report with one line: FINDINGS: <number> (FINDINGS: 0 when you found nothing). Put the report in your review note. Nixa passes the findings to the next member to fix.".to_string(),
    ]
}

pub fn fixer_lines(member: Option<&Person>, state: &HandoffState, id: i64) -> Vec<String> {
    let Some(member) = member else {
        return Vec::new();
    };
    if !in_chain(state) || is_review_role(&member.role) {
        return Vec::new();
    }
    let mut out = vec!["- Fix the findings from the earlier step below. Then run the checks and set review with a note that says what you fixed.".to_string()];
    if let Some(up) = escalate_to(&member.role)
        && !state.escalated
        && !state.recheck
    {
        out.push(format!(
            "- If you are stuck on the code itself, run: $NIXA_BIN task status {id} blocked --on other --note \"why\". Nixa then passes the task to the {}.",
            up.replace('-', " ")
        ));
    }
    out
}
