use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;

pub const FILE: &str = "keywords.json";
pub const ALIAS_MAX_WORDS: usize = 4;
pub const RISK_DOES: &str = "Runs without a card and goes in the decision journal.";

pub const EFFECTS: [&str; 37] = [
    "plan-topic", "plan-write", "act", "review-now", "explain", "debate", "mode-plan", "mode-build", "mode-review",
    "yourself", "team", "new-agent", "quick", "one-step", "full", "this-repo", "everywhere", "only-this",
    "urgent", "later", "at-time", "every", "test-first", "second-opinion", "deeper", "cheap", "best-model", "backup-model",
    "stop", "hold", "go-on", "silent", "undo", "repeat", "status", "remember", "take-over",
];

pub const LIVE_EFFECTS: [&str; 5] = ["plan-topic", "act", "review-now", "hold", "go-on"];


pub const RISK_WORDS: [(&str, &[&str]); 6] = [
    ("commit", &["commit it", "local commit"]),
    ("push", &["push it", "force push"]),
    ("deploy", &["redeploy", "release", "ship it", "go live"]),
    ("publish", &["post it", "post the comment"]),
    ("production", &["prod", "prod database", "live server", "live database"]),
    ("discard", &["discard the changes", "throw away the changes", "reset hard", "delete the branch", "clean", "restore", "wipe"]),
];

const ONLY_YES_NO: [&str; 17] = ["yes", "yeah", "yep", "yup", "ok", "okay", "sure", "fine", "right", "no", "nope", "nah", "uh", "um", "hmm", "so", "well"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub id: String,
    pub word: String,
    pub aliases: Vec<String>,
    pub effect: String,
    pub does: String,
    pub must_not: String,
    pub on: bool,
    pub built_in: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Loaded {
    pub rows: Vec<Row>,
    pub error: Option<String>,
    pub wrote: bool,
}

fn row(id: &str, word: &str, aliases: &[&str], effect: &str, does: &str, must_not: &str) -> Row {
    Row {
        id: id.into(),
        word: word.into(),
        aliases: aliases.iter().map(|a| a.to_string()).collect(),
        effect: effect.into(),
        does: does.into(),
        must_not: must_not.into(),
        on: true,
        built_in: true,
    }
}

pub fn defaults() -> Vec<Row> {
    vec![
        row("discuss", "discuss", &["lets discuss", "let us discuss", "can we discuss", "talk it through", "talk about", "just talk", "think with me", "do not assign", "dont assign", "dont start anyone", "do not start anyone", "plan only", "just plan"], "plan-topic",
            "Talks it through with you. The work goes on, and each step goes in the decision journal.",
            "Never starts an agent, sends work, starts a task, a team run or a schedule, or hands a task over."),
        row("assign", "assign", &["assign it", "give it to", "go ahead", "build", "build it", "do it", "start it", "send it", "send this", "start the work"], "act",
            "Only when something is held: ends the talk and runs this one job, with the plan agreed in the chat.",
            "Never a yes for push, deploy, publish, production or a destroying step. Does nothing when nothing is held."),
        row("review", "review", &["review the agents", "review terminal", "check the agents", "check terminal", "check what they did", "check the finished ones"], "review-now",
            "Checks the agents that finished now, or only the one named, then closes the job or sends one fix.",
            "Never starts a new agent, task or team run. Never commits when it closes a job."),
        row("hold", "hold", &["hold everything", "pause", "pause everything", "stop everything", "stop all agents", "do nothing until i say"], "hold",
            "No new work starts until you say go on, or for 30 minutes. Agent questions, checks and fixes still run.",
            "Never interrupts or closes a running agent. Never counts as a no for an open card."),
        row("go-on", "go on", &["resume", "carry on", "start again", "continue", "unpause", "back to work"], "go-on",
            "Only when a hold is on: ends the hold, then the waiting checks and agent questions go on.",
            "Never gives a yes for a card or a held push that waited."),
        row("explain", "explain", &["why did you", "tell me why", "what did you decide"], "explain",
            "Answers from the decision journal or from knowledge.", "Does no work action for this line."),
        row("debate", "debate", &["debate this", "let them debate", "debate with"], "debate",
            "Runs a read-only debate between the two agents you name.", "Never starts a debate by itself. The agents never change files or run git."),
        row("team", "team", &["use the team", "team run", "feature team", "pr team"], "team",
            "Runs that team with the space, repo and branch picked from lists.", "Never commits, pushes or posts on GitHub."),
        row("new-agent", "new agent", &["fresh agent", "new terminal", "start a new one"], "new-agent",
            "Starts a new agent instead of reusing an open one.", "Never starts the same work twice."),
        row("full", "full build", &["all the way", "end to end", "the whole thing", "finish it"], "full",
            "Builds with the checks, up to the round limit.", "Never commits or pushes."),
        row("tonight", "tonight", &["at night", "overnight"], "at-time",
            "Saves a schedule that runs once at that time.", "A scheduled run never carries a yes."),
        row("every", "every day", &["daily", "every morning", "weekly", "hourly", "weekdays"], "every",
            "Saves a repeat schedule.", "A scheduled run never carries a yes."),
        row("backup-model", "backup model", &["use the backup", "the other model"], "backup-model",
            "Starts the member on its backup model.", "Never gives a free model a git write or a risky job."),
        row("stop", "stop", &["quiet", "cancel", "never mind", "enough", "silence", "shut up"], "stop",
            "Stops speech and the current brain turn at once. Drops an open card without a yes.", "Never holds the agents. Never undoes anything."),
        row("silent", "quiet time", &["stay quiet", "quiet for"], "silent",
            "Quiet time, 1 hour by default. Lines wait in the chat, then a summary.", "Never hides a card."),
        row("repeat", "say that again", &["repeat", "what did you say", "i cannot hear you"], "repeat",
            "Says the last words again.", "Does nothing else."),
        row("status", "status", &["whats going on", "where are we", "what are you working on", "what is waiting"], "status",
            "Says what is going on from the jobs and the waiting list.", "Does nothing else."),
        row("remember", "remember", &["remember that", "keep in mind", "from now on", "forget"], "remember",
            "Saves a memory note, or an answer rule for from now on.", "A note or a rule is never a yes for push, deploy, publish or production."),
    ]
}

fn risk_in_alias() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\b(?:push|deploy|redeploy|publish|prod|production|release|live|commit|merge|rebase|reset|revert|force|discard|delete|clean|restore|wipe|ship)\b").expect("the risk word pattern is valid")
    })
}

pub fn phrase(text: &str) -> String {
    let lower = text.to_lowercase().replace(['\'', '\u{2019}'], "");
    let mut out = String::new();
    let mut gap = false;
    for c in lower.chars() {
        if c.is_alphanumeric() {
            if gap && !out.is_empty() {
                out.push(' ');
            }
            gap = false;
            out.push(c);
        } else {
            gap = true;
        }
    }
    out
}

fn text_field(item: &serde_json::Map<String, Value>, key: &str, max: usize) -> Option<String> {
    let text = item.get(key)?.as_str()?.trim();
    (!text.is_empty() && text.chars().count() <= max).then(|| text.to_string())
}

pub fn check(input: &Value) -> Result<Vec<Row>, String> {
    let Some(list) = input.as_array() else { return Err("The file must hold a list of keyword rows.".into()) };
    let mut rows = Vec::new();
    let mut used: HashMap<String, String> = HashMap::new();
    let mut ids: HashSet<String> = HashSet::new();
    for (index, raw) in list.iter().enumerate() {
        let at = format!("Row {}", index + 1);
        let Some(item) = raw.as_object() else { return Err(format!("{at} is not an object.")) };
        let (Some(id), Some(word)) = (text_field(item, "id", 40), text_field(item, "word", 60)) else {
            return Err(format!("{at} needs an id and a word."));
        };
        if !ids.insert(id.clone()) {
            return Err(format!("{at}: the id {id} is used twice."));
        }
        let effect = item.get("effect").and_then(Value::as_str).unwrap_or_default().to_string();
        if !EFFECTS.contains(&effect.as_str()) {
            return Err(format!("{at} ({word}): the effect must be one of the fixed list."));
        }
        let Some(aliases) = item.get("aliases").and_then(Value::as_array) else {
            return Err(format!("{at} ({word}): aliases must be a list of phrases."));
        };
        let Some(aliases) = aliases.iter().map(|a| a.as_str().map(str::to_string)).collect::<Option<Vec<String>>>() else {
            return Err(format!("{at} ({word}): aliases must be a list of phrases."));
        };
        let built_in = item.get("builtIn").and_then(Value::as_bool) == Some(true);
        let phrases: Vec<String> = std::iter::once(word.clone()).chain(aliases.iter().cloned()).map(|p| phrase(&p)).filter(|p| !p.is_empty()).collect();
        for said in phrases {
            let size = said.split(' ').count();
            if size > ALIAS_MAX_WORDS && !(built_in && size <= ALIAS_MAX_WORDS + 1) {
                return Err(format!("{at} ({word}): \"{said}\" is longer than {ALIAS_MAX_WORDS} words."));
            }
            if risk_in_alias().is_match(&said) {
                return Err(format!("{at} ({word}): \"{said}\" has a risk word. Risk words have their own fixed handling, so they cannot be keywords."));
            }
            if ONLY_YES_NO.contains(&said.as_str()) {
                return Err(format!("{at} ({word}): \"{said}\" is only a yes, no or filler word."));
            }
            if let Some(before) = used.get(&said)
                && *before != id
            {
                return Err(format!("{at} ({word}): \"{said}\" is already used by {before}."));
            }
            used.insert(said, id.clone());
        }
        let mut seen = HashSet::new();
        let aliases: Vec<String> = aliases.iter().map(|a| a.trim().to_string()).filter(|a| !a.is_empty() && seen.insert(a.clone())).collect();
        rows.push(Row {
            id,
            word,
            aliases,
            effect,
            does: text_field(item, "does", 300).unwrap_or_default(),
            must_not: text_field(item, "mustNot", 300).unwrap_or_default(),
            on: item.get("on").and_then(Value::as_bool) != Some(false),
            built_in,
        });
    }
    Ok(rows)
}

pub fn with_built_ins(mut rows: Vec<Row>) -> Vec<Row> {
    let have: HashSet<String> = rows.iter().map(|r| r.id.clone()).collect();
    rows.extend(defaults().into_iter().filter(|r| !have.contains(&r.id)));
    rows
}

pub fn load(file: &Path, last: &[Row]) -> Loaded {
    let last = if last.is_empty() { defaults() } else { last.to_vec() };
    if !file.exists() {
        let text = serde_json::to_string_pretty(&defaults()).unwrap_or_else(|_| "[]".into()) + "\n";
        if let Some(dir) = file.parent()
            && let Err(e) = nixa_protocol::paths::ensure_private_dir(dir)
        {
            return Loaded { rows: last, error: Some(format!("Could not write {}: {e}", file.display())), wrote: false };
        }
        return match crate::util::atomic_write(file, &text) {
            Ok(()) => Loaded { rows: defaults(), error: None, wrote: true },
            Err(e) => Loaded { rows: last, error: Some(format!("Could not write {}: {e}", file.display())), wrote: false },
        };
    }
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) => return Loaded { rows: last, error: Some(format!("Could not read {}: {e}", file.display())), wrote: false },
    };
    let value: Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(e) => return Loaded { rows: last, error: Some(format!("The keyword file is not valid JSON: {e}")), wrote: false },
    };
    match check(&value) {
        Ok(rows) => Loaded { rows: with_built_ins(rows), error: None, wrote: false },
        Err(error) => Loaded { rows: last, error: Some(error), wrote: false },
    }
}

pub fn view(rows: &[Row], error: Option<&str>) -> Value {
    let keywords: Vec<Value> = rows
        .iter()
        .map(|r| json!({"word": r.word, "aliases": r.aliases, "does": r.does, "mustNot": r.must_not, "on": r.on, "live": LIVE_EFFECTS.contains(&r.effect.as_str())}))
        .collect();
    let risk: Vec<Value> = RISK_WORDS.iter().map(|(word, aliases)| json!({"word": word, "aliases": aliases})).collect();
    let mut out = json!({"keywords": keywords, "risk": risk, "riskDoes": RISK_DOES});
    if let (Some(error), Some(map)) = (error, out.as_object_mut()) {
        map.insert("error".into(), json!(error));
    }
    out
}
