use crate::risk::{git_verbs, hard_parts, hard_stop_text, risky_text};
use crate::util::clip_flat;
use nixa_store::Connection;
use nixa_store::settings::{get_setting, set_setting_text, setting_text};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

pub const CARRY_MS: u64 = 2 * 60 * 1000;
pub const APPROVED_MS: u64 = 30 * 60 * 1000;
pub const YES_MS: u64 = 5 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Full {
    Deep,
    Nixa,
}

impl Full {
    pub fn parse(word: &str) -> Option<Full> {
        match word.trim().to_lowercase().as_str() {
            "deep" | "deep brain" => Some(Full::Deep),
            "nixa" | "assistant" | "self" => Some(Full::Nixa),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Full::Deep => "deep",
            Full::Nixa => "nixa",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Who {
    User,
    Nixa,
    Deep,
}

pub fn who_of(full: Full, deep_ready: bool) -> Who {
    match full {
        Full::Nixa => Who::Nixa,
        Full::Deep if deep_ready => Who::Deep,
        Full::Deep => Who::Nixa,
    }
}

pub const FULL_KEY: &str = "decision.full";
pub const OLD_FILE: &str = "decision";
pub const OLD_SPACES_FILE: &str = "decision-spaces.json";

pub struct Decision {
    pub db: PathBuf,
}

fn fail(error: anyhow::Error) -> String {
    format!("{error:#}")
}

fn open_db(db: &Path) -> Result<Connection, String> {
    nixa_store::open(db).map_err(fail)
}

pub fn full_of(conn: &Connection) -> Full {
    setting_text(conn, FULL_KEY, "").ok().and_then(|t| Full::parse(&t)).unwrap_or(Full::Deep)
}

impl Decision {
    pub fn open(dir: &Path) -> Decision {
        let decision = Decision { db: dir.join(nixa_store::DB_FILE) };
        match import_old(&decision.db, dir) {
            Ok(0) => {}
            Ok(count) => crate::util::log("brain", &format!("moved {count} old decision settings into the store")),
            Err(error) => crate::util::log("brain", &format!("the old decision files were not imported: {error:#}")),
        }
        decision
    }

    pub fn full(&self) -> Full {
        open_db(&self.db).map(|conn| full_of(&conn)).unwrap_or(Full::Deep)
    }

    pub fn set_full(&self, full: Full) -> Result<(), String> {
        set_setting_text(&open_db(&self.db)?, FULL_KEY, full.word()).map_err(fail)
    }

    pub fn set_choice(&self, word: &str) -> Result<Full, String> {
        let full = Full::parse(word).ok_or_else(|| format!("Who decides must be nixa or deep, not {}.", word.trim()))?;
        self.set_full(full)?;
        Ok(full)
    }

    pub fn deep_mode(&self) -> bool {
        self.full() == Full::Deep
    }

    pub fn nixa_decides(&self) -> bool {
        self.full() == Full::Nixa
    }

    pub fn who(&self, deep_ready: bool) -> Who {
        who_of(self.full(), deep_ready)
    }

    pub fn name(&self) -> String {
        full_name(self.full())
    }
}

pub fn import_old(db: &Path, dir: &Path) -> anyhow::Result<usize> {
    let file = dir.join(OLD_FILE);
    let spaces_file = dir.join(OLD_SPACES_FILE);
    if !file.exists() && !spaces_file.exists() {
        return Ok(0);
    }
    let mut moved = 0;
    if file.exists() {
        let text = std::fs::read_to_string(&file)?;
        let conn = nixa_store::open(db)?;
        if let Some(full) = Full::parse(&text)
            && get_setting(&conn, FULL_KEY)?.is_none()
        {
            set_setting_text(&conn, FULL_KEY, full.word())?;
            moved += 1;
        }
        std::fs::rename(&file, dir.join(format!("{OLD_FILE}.imported")))?;
    }
    if spaces_file.exists() {
        std::fs::rename(&spaces_file, spaces_file.with_extension("json.imported"))?;
    }
    Ok(moved)
}

pub fn full_name(full: Full) -> String {
    match full {
        Full::Deep => "The deep brain decides".into(),
        Full::Nixa => "Nixa decides".into(),
    }
}

pub fn card_why(text: &str) -> String {
    let verbs = git_verbs(text);
    if hard_stop_text(text) {
        let held: Vec<&str> = verbs
            .iter()
            .map(String::as_str)
            .filter(|v| ["push", "deploy", "publish", "production", "reset", "clean", "discard"].contains(v))
            .collect();
        let what = if held.is_empty() { "push, deploy, production or destroy".to_string() } else { held.join(" and ") };
        return format!("Why: it has a {what} step, and only you allow those.");
    }
    if !verbs.is_empty() {
        return format!("Why: it has a git step ({}), and git steps wait for your yes.", verbs.join(", "));
    }
    if risky_text(text) {
        return "Why: it is a risky step, so it waits for your yes.".into();
    }
    "Why: only you can allow this.".into()
}

static GO_AHEAD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:(?:ok|okay|yes|yeah|yep|sure|alright|all right|fine|right|so|then)\s+)*(?:please\s+)?(?:(?:just\s+)?do (?:it|that|this|so)|go ahead|go for it|go on|proceed|carry on|send it|tell (?:him|her|it|them|claude|codex|the agent) (?:to )?(?:go ahead|do it|yes))(?:\s+(?:please|now|then))?(?:\s+(?:whatever|what) (?:he|she|it|they|claude|codex|the agent) (?:is asking|asks|asked|wants|said|says|suggests|suggested)(?:\s+for)?)?(?:\s+please)?$").unwrap()
});

pub fn normalise(text: &str) -> String {
    let lowered: String = text.to_lowercase().chars().map(|c| if c.is_alphanumeric() || c == '\'' { c } else { ' ' }).collect();
    crate::util::flat(&lowered)
}

pub fn go_ahead(line: &str) -> bool {
    let text = line.trim();
    if text.ends_with('?') {
        return false;
    }
    GO_AHEAD.is_match(&normalise(text))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approval {
    pub verbs: Vec<String>,
    pub at: u64,
    #[serde(default)]
    pub scope: StepScope,
}

pub fn approval_of(text: &str, at: u64) -> Approval {
    Approval { verbs: git_verbs(text), at, scope: step_scope(text) }
}

pub fn approval_covers(approval: Option<&Approval>, text: &str, now: u64) -> bool {
    let Some(approval) = approval else { return false };
    if now.saturating_sub(approval.at) > APPROVED_MS {
        return false;
    }
    if !hard_stop_text(text) {
        return true;
    }
    let verbs = git_verbs(text);
    !verbs.is_empty() && verbs.iter().all(|v| approval.verbs.contains(v)) && scope_fits(Some(&approval.scope), &step_scope(text))
}

pub fn user_allows(last_user: Option<(&str, u64)>, text: &str, now: u64) -> bool {
    let Some((line, at)) = last_user else { return false };
    if now.saturating_sub(at) > YES_MS || line.trim().ends_with('?') {
        return false;
    }
    let given = git_verbs(line);
    let said = format!(" {} ", normalise(line));
    let hard = hard_parts(text);
    let named = |part: &String| {
        let words = normalise(part);
        let verbs = git_verbs(part);
        said.contains(&format!(" {words} ")) || (!words.contains("force") && !verbs.is_empty() && verbs.iter().all(|v| given.contains(v)))
    };
    if !hard.iter().all(named) {
        return false;
    }
    if !given.is_empty() && !scope_fits(Some(&step_scope(line)), &step_scope(text)) {
        return false;
    }
    let needed = git_verbs(text);
    (!needed.is_empty() && needed.iter().all(|v| given.contains(v))) || go_ahead(line) || !hard.is_empty()
}

const AUTOMATIC_CONTROL: &str = "Actions run without an approval pause and are recorded in the decision journal. The user can always stop the work or turn Nixa off. Validation, privacy and verified update rollback remain in force.";

pub fn full_meaning(full: Full, assistant: &str) -> String {
    match full {
        Full::Deep => format!(
            "{}. Every choice in every project is decided without waiting for approval. The deep brain decides everything in every project: agent questions, job checks, task placement, what to build and the next step. {assistant} asks it, waits for the answer and follows it, and does not ask the user to pick. If no deep brain is up, {assistant} decides by itself. {AUTOMATIC_CONTROL}",
            full_name(full)
        ),
        Full::Nixa => format!(
            "{}. Every choice in every project is decided without waiting for approval. {assistant} decides almost everything by itself: work, agent questions, and local git work such as commit, merge, rebase and stash. It picks the recommended option, does not ask the deep brain, and tells the user after. {AUTOMATIC_CONTROL}",
            full_name(full)
        ),
    }
}

pub fn local_git_note(full: Full) -> String {
    format!("{} is on: git steps are yours like any other next step, so proceed without asking for approval. Record the action and tell the user after. The user can stop work at any time.", full_name(full))
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepScope {
    pub force: bool,
    pub names: Vec<String>,
}

const FORCE_TERM: &str = r"(?:force[- ]?push\w*|--force(?:-with-lease)?|--mirror|--delete|--hard|branch\s+-D|push\b[^\n]*?\s(?:-f|-d)(?:\s|$)|push\b[^\n]*?\s\+[\w./-]+)";

static FORCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!("(?i){FORCE_TERM}")).unwrap());
static NOT_FORCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"(?i)\b(?:do not|don't|dont|never|no|not|without)\s+(?:\w+\s+){{0,3}}?{FORCE_TERM}")).unwrap());
static STEP_START: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(?:force[- ]?push|push|merge|rebase|reset|deploy|publish|tag|revert|clean)\w*").unwrap());
static PROTECTED_REF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:main|master|trunk|staging|upstream|release(?:/[\w.-]+)?)$").unwrap());
static REMOTE_REF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(?:origin|upstream)\s+(?:HEAD:)?([A-Za-z0-9._/-]+)").unwrap());
static FILE_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.[a-z]{1,5}$").unwrap());

fn ref_name(word: &str) -> String {
    let low = word.to_lowercase();
    let low = low.trim_end_matches('.');
    for prefix in ["refs/heads/", "heads/", "origin/"] {
        if let Some(rest) = low.strip_prefix(prefix) {
            return rest.to_string();
        }
    }
    low.to_string()
}

fn ref_words(text: &str) -> Vec<String> {
    let part = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/');
    let chars: Vec<char> = text.chars().collect();
    let mut words = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        if !part(chars[at]) {
            at += 1;
            continue;
        }
        let from = at;
        while at < chars.len() && part(chars[at]) {
            at += 1;
        }
        let word: String = chars[from..at].iter().collect();
        let joined = (from > 0 && matches!(chars[from - 1], '~' | ':' | '@')) || chars.get(at).is_some_and(|c| c.is_alphanumeric());
        if !joined && !word.starts_with('/') && !word.ends_with('/') && !word.contains("//") {
            words.push(word);
        }
    }
    words
}

pub fn step_scope(text: &str) -> StepScope {
    let plain = NOT_FORCE.replace_all(text, " ").to_string();
    let mut names: Vec<String> = Vec::new();
    let mut add = |name: String| {
        if !names.contains(&name) {
            names.push(name);
        }
    };
    for line in plain.split('\n') {
        let Some(start) = STEP_START.find(line) else { continue };
        let rest = &line[start.end()..];
        for word in ref_words(rest) {
            let name = ref_name(&word);
            if PROTECTED_REF.is_match(&name) || (name.contains('/') && !FILE_END.is_match(&name)) {
                add(name);
            }
        }
        for found in REMOTE_REF.captures_iter(rest) {
            let name = ref_name(&found[1]);
            if !name.is_empty() && !name.starts_with('-') {
                add(name);
            }
        }
    }
    StepScope { force: FORCE.is_match(&plain), names }
}

pub fn scope_fits(approved: Option<&StepScope>, asked: &StepScope) -> bool {
    let empty = StepScope::default();
    let given = approved.unwrap_or(&empty);
    if asked.force && !given.force {
        return false;
    }
    let specific = given.names.iter().filter(|n| !PROTECTED_REF.is_match(n)).count();
    asked.names.iter().all(|name| given.names.contains(name) || (!PROTECTED_REF.is_match(name) && specific == 0))
}

pub fn carries_yes(line: &str, message: &str, said: Option<&str>) -> bool {
    if line.trim().ends_with('?') {
        return false;
    }
    let asked = git_verbs(line);
    let needed = git_verbs(message);
    if needed.is_empty() {
        return false;
    }
    if !asked.is_empty() {
        return needed.iter().all(|v| asked.contains(v)) && scope_fits(Some(&step_scope(line)), &step_scope(message));
    }
    let Some(said) = said else { return false };
    if !go_ahead(line) || hard_stop_text(message) {
        return false;
    }
    let offered = git_verbs(said);
    needed.iter().all(|v| offered.contains(v))
}

static START_WORDS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(?:start(?:s|ed|ing)?|run(?:s|ning)?|begin|launch|kick (?:it |this |that )?off|work on|pick (?:it |this |that )?up|resume)\b").unwrap());
static NO_START: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(?:do not|don't|don\x{2019}t|dont|never|no need to|not now|not yet|wait)\b[^.!?]*\b(?:start|run|begin|launch)").unwrap());
static PLAIN_OK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:ok|okay|yes|yeah|yep|sure|alright|all right|fine|right)(?: (?:please|now|then|go|do it|go ahead|start it))*$").unwrap());

pub fn asks_start(line: &str, said: &str) -> bool {
    let text = line.trim();
    if text.is_empty() || NO_START.is_match(text) {
        return false;
    }
    if START_WORDS.is_match(text) {
        return true;
    }
    (PLAIN_OK.is_match(&normalise(text)) || go_ahead(text)) && START_WORDS.is_match(said)
}

static SEND_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?s)^Send "(.*)" to ([^"]*?)\??$"#).unwrap());
static START_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?s)^Start [^"]+ with the first message "(.*)"\??$"#).unwrap());
static YES_HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*Needs your yes:\s*").unwrap());
static TERM_IN: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"\(({})\)", nixa_protocol::id::TERM_ID_PATTERN)).unwrap());

pub fn sent_text(message: &str) -> Option<String> {
    let body = YES_HEAD.replace(message, "").trim().to_string();
    if let Some(found) = SEND_LINE.captures(&body) {
        return if found[2].contains("to answer") { None } else { Some(found[1].to_string()) };
    }
    START_LINE.captures(&body).map(|c| c[1].to_string())
}

pub fn terminal_in_message(message: &str) -> Option<String> {
    TERM_IN.captures_iter(message).filter_map(|c| c[1].parse::<nixa_protocol::TermId>().ok()).last().map(String::from)
}

pub const TALK_BYTES: u64 = 512 * 1024;
pub const TALK_MAX_BYTES: u64 = 32 * 1024 * 1024;
pub const TALK_LINES: usize = 4;
const LINE_CHARS: usize = 300;
const AGENT_CHARS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TalkWho {
    Typed,
    Answer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TalkLine {
    pub who: TalkWho,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Talk {
    pub lines: Vec<TalkLine>,
    pub agent: Option<String>,
}

static ANSWER_PART: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""="((?:[^"\\]|\\.)*)""#).unwrap());

fn answers_in(content: &Value) -> Vec<String> {
    let text = match content {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    ANSWER_PART.captures_iter(&text).map(|c| c[1].replace("\\\"", "\"")).filter(|s| !s.is_empty()).collect()
}

pub fn read_talk(jsonl: &str, max: usize) -> Talk {
    let mut lines = Vec::new();
    let mut asks: Vec<String> = Vec::new();
    let mut agent = None;
    for raw in jsonl.lines() {
        let Ok(row) = serde_json::from_str::<Value>(raw.trim()) else { continue };
        let content = &row["message"]["content"];
        let kind = row["type"].as_str().unwrap_or("");
        if kind == "assistant" {
            for part in content.as_array().into_iter().flatten() {
                if part["type"] == "tool_use" && part["name"] == "AskUserQuestion" {
                    asks.push(part["id"].as_str().unwrap_or("").to_string());
                }
                if part["type"] == "text"
                    && let Some(text) = part["text"].as_str().filter(|t| !t.trim().is_empty())
                {
                    agent = Some(text.to_string());
                }
            }
            continue;
        }
        if kind != "user" || row["isMeta"] == Value::Bool(true) {
            continue;
        }
        let parts: Vec<Value> = match content {
            Value::String(s) => vec![serde_json::json!({"type": "text", "text": s})],
            Value::Array(list) => list.clone(),
            _ => Vec::new(),
        };
        for part in parts {
            if part["type"] == "text" {
                let text = part["text"].as_str().unwrap_or("").trim();
                if !text.is_empty() && !text.starts_with('<') && !text.starts_with("[Image: source:") && !text.starts_with("[Request interrupted") {
                    lines.push(TalkLine { who: TalkWho::Typed, text: text.to_string() });
                }
            } else if part["type"] == "tool_result" && asks.iter().any(|a| part["tool_use_id"].as_str() == Some(a.as_str())) {
                let answers = answers_in(&part["content"]);
                if !answers.is_empty() {
                    lines.push(TalkLine { who: TalkWho::Answer, text: answers.join("; ") });
                }
            }
        }
    }
    let skip = lines.len().saturating_sub(max);
    Talk { lines: lines.split_off(skip), agent }
}

fn read_tail(file: &Path, size: u64, bytes: u64) -> std::io::Result<String> {
    let start = size.saturating_sub(bytes);
    let mut handle = std::fs::File::open(file)?;
    handle.seek(SeekFrom::Start(start))?;
    let mut buffer = Vec::new();
    handle.take(size - start).read_to_end(&mut buffer)?;
    Ok(String::from_utf8_lossy(&buffer).to_string())
}

pub fn read_talk_file(file: &Path, bytes: u64, max: usize) -> Option<Talk> {
    let size = std::fs::metadata(file).ok()?.len();
    let mut window = bytes.max(1);
    loop {
        let talk = read_talk(&read_tail(file, size, window).ok()?, max);
        if talk.lines.len() >= max || window >= size || window >= TALK_MAX_BYTES {
            return Some(talk);
        }
        window = window.saturating_mul(4);
    }
}

pub fn clip_ends(text: &str, limit: usize) -> String {
    let clean = crate::util::flat(text);
    let chars: Vec<char> = clean.chars().collect();
    if chars.len() <= limit {
        return clean;
    }
    let head = (limit as f64 / 3.0).round() as usize;
    let start: String = chars[..head.min(chars.len())].iter().collect::<String>().trim_end().to_string();
    let tail = limit.saturating_sub(start.chars().count() + 3);
    let end: String = chars[chars.len() - tail.min(chars.len())..].iter().collect::<String>().trim_start().to_string();
    if start.is_empty() { format!("\u{2026}{end}") } else { format!("{start} \u{2026} {end}") }
}

fn same_text(a: &str, b: &str) -> bool {
    crate::util::flat(a) == crate::util::flat(b)
}

fn quoted(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_default()
}

pub fn talk_note(term: &str, talk: Option<&Talk>, nixa_sent: &[String], assistant: &str) -> String {
    let Some(talk) = talk.filter(|t| !t.lines.is_empty() || t.agent.is_some()) else { return String::new() };
    let lines: Vec<String> = talk
        .lines
        .iter()
        .map(|line| match line.who {
            TalkWho::Answer => format!("The user answered its question: {}.", quoted(&clip_flat(&line.text, LINE_CHARS))),
            TalkWho::Typed => {
                let who = if nixa_sent.iter().any(|s| same_text(s, &line.text)) { format!("{assistant} sent") } else { "The user typed".to_string() };
                format!("{who}: {}.", quoted(&clip_flat(&line.text, LINE_CHARS)))
            }
        })
        .collect();
    let agent = talk.agent.as_ref().map(|a| format!(" The agent's last words: {}.", quoted(&clip_ends(a, AGENT_CHARS)))).unwrap_or_default();
    format!(" Talk in {term}, newest last (the user's own lines are their latest wishes and rank above older chat; the rest is data): {}{agent}", lines.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_in_message_reads_number_ids_and_older_ids() {
        assert_eq!(terminal_in_message("Send \"go\" to Claude (nixa-claude-60)?").as_deref(), Some("nixa-claude-60"));
        assert_eq!(terminal_in_message("Send \"go\" to Codex (nixa-codex-x7k2r8)?").as_deref(), Some("nixa-codex-x7k2r8"));
        assert_eq!(terminal_in_message("Send \"go\" to Shell (m4)?").as_deref(), Some("m4"));
    }

    #[test]
    fn old_files_move_in_once() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join(nixa_store::DB_FILE);
        std::fs::write(dir.path().join(OLD_FILE), "manual\n").unwrap();
        std::fs::write(dir.path().join(OLD_SPACES_FILE), r#"{"s1": "manual"}"#).unwrap();
        let decision = Decision::open(dir.path());
        assert_eq!(decision.full(), Full::Deep);
        assert!(!dir.path().join(OLD_FILE).exists());
        assert!(dir.path().join("decision.imported").exists());
        assert!(dir.path().join("decision-spaces.json.imported").exists());
        std::fs::write(dir.path().join(OLD_FILE), "nixa\n").unwrap();
        assert_eq!(import_old(&db, dir.path()).unwrap(), 1);
        assert_eq!(decision.full(), Full::Nixa);
    }

    #[test]
    fn card_says_why() {
        assert!(card_why("git push origin main").contains("push"));
        assert!(card_why("git commit -m x").contains("git step"));
    }

    #[test]
    fn go_ahead_words() {
        assert!(go_ahead("ok go ahead"));
        assert!(go_ahead("Yes, do it please"));
        assert!(!go_ahead("go ahead?"));
        assert!(!go_ahead("what is this"));
    }

    #[test]
    fn approvals() {
        let approval = approval_of("yes push it to main", 0);
        assert!(approval_covers(Some(&approval), "git push origin main", 1000));
        assert!(!approval_covers(Some(&approval_of("yes push it", 0)), "git push origin main", 1000));
        assert!(approval_covers(Some(&approval_of("yes push it", 0)), "git push origin feature-x", 1000));
        assert!(!approval_covers(Some(&approval), "git push --force origin main", 1000));
        assert!(!approval_covers(Some(&approval), "deploy now", 1000));
        assert!(!approval_covers(Some(&approval), "git push", APPROVED_MS + 1));
        assert!(user_allows(Some(("yes commit it", 0)), "git commit -m fix", 10));
        assert!(!user_allows(Some(("hello", 0)), "git commit -m fix", 10));
        assert!(user_allows(Some(("go ahead", 0)), "git commit -m fix", 10));
        assert!(!user_allows(Some(("go ahead", 0)), "git push origin main", 10));
        assert!(user_allows(Some(("push it to origin main", 0)), "git push origin main", 10));
        assert!(!user_allows(Some(("push it to origin", 0)), "git push origin main", 10));
        assert!(!user_allows(Some(("push feature-a", 0)), "git push origin main", 10));
        assert!(user_allows(Some(("push feature-a", 0)), "git push origin feature-a", 10));
        assert!(!user_allows(Some(("push feature-a", 0)), "git push --force origin main && rm -rf .", 5 * 60 * 1000 - 1));
        assert!(!user_allows(Some(("push feature-a", 0)), "git push origin main", YES_MS + 1));
        assert!(!user_allows(Some(("go ahead", 0)), "rm -rf build", 10));
        assert!(user_allows(Some(("yes run rm -rf build", 0)), "rm -rf build", 10));
        assert!(!user_allows(Some(("push it", 0)), "git push --force", 10));
        assert!(user_allows(Some(("force push it", 0)), "git push --force", 10));
        assert!(!user_allows(Some(("push it?", 0)), "git push", 10));
    }

    #[test]
    fn full_modes_nixa_and_deep() {
        let dir = tempfile::tempdir().unwrap();
        let decision = Decision::open(dir.path());
        assert_eq!(decision.full(), Full::Deep);
        assert!(decision.deep_mode());
        assert_eq!(decision.who(true), Who::Deep);
        assert_eq!(decision.set_choice("nixa").unwrap(), Full::Nixa);
        assert!(!decision.deep_mode() && decision.nixa_decides());
        assert_eq!(decision.who(true), Who::Nixa);
        assert_eq!(decision.name(), "Nixa decides");
        assert!(decision.set_choice("manual").is_err());
        assert_eq!(decision.set_choice("deep").unwrap(), Full::Deep);
        assert_eq!(decision.who(false), Who::Nixa);
        assert!(full_meaning(Full::Deep, "Nixa").contains("If no deep brain is up, Nixa decides by itself"));
        assert!(full_meaning(Full::Nixa, "Nixa").contains("does not ask the deep brain"));
        assert!(local_git_note(Full::Nixa).contains("Nixa decides"));
    }

    #[test]
    fn step_scopes() {
        assert_eq!(step_scope("git push origin main"), StepScope { force: false, names: vec!["main".into()] });
        assert_eq!(step_scope("git push origin HEAD:feature/login"), StepScope { force: false, names: vec!["feature/login".into()] });
        assert!(step_scope("git push -f origin x").force);
        assert!(step_scope("git push origin +x").force);
        assert!(step_scope("git reset --hard").force);
        assert!(!step_scope("push it, but do not force push").force);
        assert!(!step_scope("git push -fx").force);
        assert!(step_scope("merge src/main.rs fixes").names.is_empty());
        assert_eq!(step_scope("rebase onto release/2.1").names, vec!["release/2.1".to_string()]);
        assert!(step_scope("push user@main").names.is_empty());
        let given = step_scope("push feature/a");
        assert!(scope_fits(Some(&given), &step_scope("git push origin feature/a")));
        assert!(!scope_fits(Some(&given), &step_scope("git push origin feature/b")));
        assert!(!scope_fits(None, &step_scope("git push origin main")));
        assert!(scope_fits(None, &step_scope("git push origin topic")));
    }

    #[test]
    fn yes_carries_and_start_words() {
        assert!(go_ahead("tell claude to go ahead"));
        assert!(go_ahead("ok go ahead whatever it is asking for please"));
        assert!(carries_yes("yes commit it", "git commit -m fix", None));
        assert!(!carries_yes("yes commit it", "git commit && git push origin main", None));
        assert!(carries_yes("go ahead", "git commit -m fix", Some("Shall I commit the fix?")));
        assert!(!carries_yes("go ahead", "git push origin main", Some("Shall I push?")));
        assert!(!carries_yes("commit it?", "git commit", None));
        assert!(!carries_yes("ok", "run the tests", None));
        assert!(asks_start("start task 57", ""));
        assert!(!asks_start("don't start it yet", ""));
        assert!(asks_start("ok", "Shall I start task 4?"));
        assert!(!asks_start("ok", "Here is the plan."));
        assert_eq!(sent_text("Needs your yes: Send \"run tests\" to Codex (m4)?").as_deref(), Some("run tests"));
        assert_eq!(sent_text("Send \"2\" to Codex to answer its question?"), None);
        assert_eq!(sent_text("Start Claude Code with the first message \"fix it\"?").as_deref(), Some("fix it"));
        assert_eq!(terminal_in_message("Send x to a (m2) then (m1a)").as_deref(), Some("m1a"));
    }

    #[test]
    fn reads_agent_talk() {
        let rows = [
            serde_json::json!({"type": "user", "message": {"content": "Fix the login page"}}),
            serde_json::json!({"type": "user", "isMeta": true, "message": {"content": "hidden"}}),
            serde_json::json!({"type": "user", "message": {"content": [{"type": "text", "text": "<command>skip</command>"}]}}),
            serde_json::json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "q1", "name": "AskUserQuestion"}, {"type": "text", "text": "Which page?"}]}}),
            serde_json::json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "q1", "content": "User answered: \"Which page?\"=\"The admin one\""}]}}),
            serde_json::json!({"type": "user", "message": {"content": "run the tests"}}),
        ];
        let text = rows.iter().map(|r| r.to_string()).collect::<Vec<_>>().join("\n") + "\nnot json\n";
        let talk = read_talk(&text, 4);
        assert_eq!(talk.lines.iter().map(|l| (l.who, l.text.as_str())).collect::<Vec<_>>(), vec![(TalkWho::Typed, "Fix the login page"), (TalkWho::Answer, "The admin one"), (TalkWho::Typed, "run the tests")]);
        assert_eq!(talk.agent.as_deref(), Some("Which page?"));
        assert_eq!(read_talk(&text, 1).lines.len(), 1);
        let note = talk_note("m3", Some(&talk), &["run the tests".to_string()], "Nixa");
        assert!(note.contains("Talk in m3") && note.contains("The user typed: \"Fix the login page\".") && note.contains("Nixa sent: \"run the tests\"."));
        assert!(note.contains("The user answered its question: \"The admin one\"."));
        assert_eq!(talk_note("m3", None, &[], "Nixa"), "");
        let home = tempfile::tempdir().unwrap();
        let moved = home.path().join("abc-1x.jsonl");
        std::fs::write(&moved, &text).unwrap();
        let read = read_talk_file(&moved, 16, 4).unwrap();
        assert_eq!(read.lines.len(), 3);
        assert_eq!(clip_ends("abcdefghij klmnopqrst", 10), "abc \u{2026} qrst");
    }
}
