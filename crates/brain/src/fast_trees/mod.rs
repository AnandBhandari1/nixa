mod check;
mod files;
mod phrase;
mod rules;
mod tree;

#[cfg(test)]
mod tests;

pub use check::{SAMPLE, report};
pub use files::{FILE, FastTrees, Reload, TREES_DIR, good_tree_name};
pub use phrase::{Hit, PhraseIndex, clean_phrase, words_of};
pub use rules::{AnswerRule, Compiled, DecisionRow, FastDecision, Input, Keyword, MatchView, ROUTING, Rules, SelfCheck, compile, required_action, rules_of, seed, seed_value};
pub use tree::{Chart, MAX_DEPTH, Run, Step, Tree, chart_of, compile_tree};

use crate::app::{App, lock};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const WATCH_MS: u64 = 2000;
pub const DEEP_HINT: &str = " This looks like a hard decision: ask the deep brain with nixa_deep_ask before you act, and tell the user in one short line that you asked.";

pub struct State {
    pub trees: Mutex<FastTrees>,
}

impl State {
    pub fn open(dir: &Path) -> State {
        let trees = FastTrees::open(dir);
        if let Some(problem) = &trees.problem {
            crate::util::log("fast_trees", problem);
        }
        State { trees: Mutex::new(trees) }
    }
}

pub fn first_problem(state: &State) -> Option<String> {
    lock(&state.trees).problem.clone()
}

pub fn tick(state: &State) -> Option<String> {
    lock(&state.trees).reload().problem
}

pub fn decide(state: &State, text: &str, input: &Input) -> Result<FastDecision, String> {
    lock(&state.trees).decide(text, input)
}

pub async fn run(app: Arc<App>) {
    if let Some(problem) = first_problem(&app.fast_trees) {
        app.info(&problem);
    }
    loop {
        tokio::time::sleep(Duration::from_millis(WATCH_MS)).await;
        if let Some(problem) = tick(&app.fast_trees) {
            app.info(&problem);
        }
    }
}

pub fn chat_note(app: &App, text: &str) -> Option<String> {
    let card_open = !lock(&app.prompts.book).items.is_empty();
    let ask_deep = app.decision.full() == crate::decision::Full::Deep;
    note(&app.fast_trees, text, card_open, ask_deep)
}

pub fn automatic_decision(mut decision: FastDecision) -> FastDecision {
    decision.requires_approval = false;
    if decision.action == "approval" {
        decision.action = "build".into();
    }
    decision
}

pub fn note_of(decision: &FastDecision, ask_deep: bool) -> String {
    let json = serde_json::to_string(decision).unwrap_or_default();
    let hint = if ask_deep && decision.tier == "premium" { DEEP_HINT } else { "" };
    format!("Note: Fast local decision (compiled keyword, table and tree lookup): {json}. There is no approval gate, so take the step yourself and record it in the decision journal. The user's own stop and hold words still apply.{hint}")
}

pub fn note(state: &State, text: &str, card_open: bool, ask_deep: bool) -> Option<String> {
    let input = Input { card_open, ..Input::default() };
    match decide(state, text, &input) {
        Ok(decision) => Some(note_of(&automatic_decision(decision), ask_deep)),
        Err(error) => {
            crate::util::log("fast_trees", &format!("no fast decision: {error}"));
            None
        }
    }
}
