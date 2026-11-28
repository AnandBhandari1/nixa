use nixa_store::agents::{CustomAgent, StartProfile, start_command_in};
use std::path::Path;
use nixa_store::text::one_line;

pub const DEEP_ANSWER_RULE: &str = "When Nixa or your space's deep brain answers your question, that answer is the user's decision: act on it now. It counts for every step, and nothing waits for the user: a step that physically needs the user (a sign-in, a key, a test on the user's own device) is written as a note for the user, and you find another way (a saved key, a mock, or skip that step) and go on with the rest.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub argv: Vec<String>,
    pub send: Option<String>,
}

pub fn quote(word: &str) -> String {
    if !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:@+,".contains(c))
    {
        return word.to_string();
    }
    format!("'{}'", word.replace('\'', "'\\''"))
}

pub fn join(words: &[String]) -> String {
    words.iter().map(|w| quote(w)).collect::<Vec<_>>().join(" ")
}

pub fn shell(dir: &Path, command: &str) -> Vec<String> {
    vec!["sh".into(), "-lc".into(), crate::git_tools::gate::gated_line(dir, command)]
}

pub fn base_command(dir: &Path, agent: &str) -> Option<String> {
    start_command_in(dir, agent).map(str::to_string)
}

fn model_flag(agent: &str) -> Option<&'static str> {
    nixa_store::agents::model_flag(agent)
}

pub fn profile_words(dir: &Path, start: &StartProfile) -> Option<Vec<String>> {
    let start = &nixa_store::agents::read_profile(start)?;
    let mut words: Vec<String> = base_command(dir, &start.agent)?
        .split_whitespace()
        .map(str::to_string)
        .collect();
    if let Some(flag) = model_flag(&start.agent)
        && !start.model.is_empty()
    {
        words.push(flag.into());
        words.push(start.model.clone());
    }
    words.extend(nixa_store::agents::effort_words(&start.agent, &start.effort));
    words.extend(start.args.iter().cloned());
    nixa_store::agents::configure_start_in(dir, &mut words);
    Some(words)
}

pub fn model_note(start: &StartProfile) -> Option<String> {
    let model = !start.model.is_empty() && model_flag(&start.agent).is_none();
    let effort = !start.effort.is_empty() && nixa_store::agents::effort_words(&start.agent, &start.effort).is_empty();
    match (model, effort) {
        (true, true) => Some(format!(
            "Use the model {} with {} effort.",
            start.model, start.effort
        )),
        (true, false) => Some(format!("Use the model {}.", start.model)),
        (false, true) => Some(format!("Work with {} effort.", start.effort)),
        (false, false) => None,
    }
}

pub fn prompt_plan(words: Vec<String>, prompt: &str) -> Plan {
    let argv = words;
    if prompt.is_empty() {
        return Plan { argv, send: None };
    }
    Plan {
        argv,
        send: Some(prompt.to_string()),
    }
}

pub fn profile_rules(agent: &CustomAgent) -> String {
    let mut parts = vec![format!(
        "You are {}, a custom agent of Nixa. Your purpose: {}",
        agent.title,
        one_line(&agent.purpose)
    )];
    if !agent.instructions.is_empty() {
        parts.push(format!(
            "Your standing instructions:\n{}",
            agent.instructions
        ));
    }
    if let Some(block) = nixa_store::agents::skills_block(agent, &nixa_protocol::paths::data_dir()) {
        parts.push(block);
    }
    parts.push(
        "You report to Nixa. When you are stuck or a decision is not yours, ask Nixa your question with your recommended option, then act on the answer from Nixa or your space's deep brain.".into(),
    );
    if let Some(note) = agent.start.as_ref().and_then(model_note) {
        parts.push(note);
    }
    parts.join("\n\n")
}

pub fn profile_plan(dir: &Path, agent: &CustomAgent, message: &str) -> Option<Plan> {
    let start = agent.start.as_ref()?;
    let words = profile_words(dir, start)?;
    let mut prompt = profile_rules(agent);
    if !message.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(message);
    }
    Some(prompt_plan(words, &prompt))
}

pub fn with_instructions(agent: &CustomAgent, message: &str) -> String {
    let skills = nixa_store::agents::skills_block(agent, &nixa_protocol::paths::data_dir());
    if agent.instructions.is_empty() && skills.is_none() {
        return message.to_string();
    }
    let mut parts = Vec::new();
    if !agent.instructions.is_empty() {
        parts.push(format!("Your standing instructions as {} ({}):\n{}", agent.title, agent.id, agent.instructions));
    }
    parts.extend(skills);
    parts.push(message.to_string());
    parts.join("\n\n")
}

pub fn first_message(agent: &CustomAgent, message: &str) -> String {
    let mut parts = vec![format!(
        "You are the custom agent {} ({}). Your purpose: {}",
        agent.title,
        agent.id,
        one_line(&agent.purpose)
    )];
    if !agent.instructions.is_empty() {
        parts.push(format!(
            "Your standing instructions:\n{}",
            agent.instructions
        ));
    }
    if let Some(block) = nixa_store::agents::skills_block(agent, &nixa_protocol::paths::data_dir()) {
        parts.push(block);
    }
    parts.push(DEEP_ANSWER_RULE.into());
    parts.push(crate::status_rule::rule());
    if !message.is_empty() {
        parts.push(message.to_string());
    }
    parts.join("\n\n")
}
