use super::deliver::Permission;
use super::dialog::{Question, QuestionOption};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

pub const DEDUPE_MS: u64 = 5000;
pub const DETAIL_CHARS: usize = 500;
pub const ANSWERED_KEEP: usize = 20;
pub const SCREEN_PREFIX: &str = "screen:";
pub const RECENT_KEY_MS: u64 = 8_000;
pub const RECENT_ANY_MS: u64 = 4_000;
pub const RECENT_KEEP: usize = 20;
const INPUT_KEYS: [&str; 5] = ["command", "file_path", "path", "url", "pattern"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptKind {
    Question,
    Permission,
    Plan,
}

impl PromptKind {
    pub fn parse(word: &str) -> Option<PromptKind> {
        match word.trim() {
            "question" => Some(PromptKind::Question),
            "permission" => Some(PromptKind::Permission),
            "plan" => Some(PromptKind::Plan),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Choice {
    Yes,
    Always,
    No,
}

impl Choice {
    pub fn parse(word: &str) -> Option<Choice> {
        match word.trim().to_lowercase().as_str() {
            "yes" => Some(Choice::Yes),
            "always" => Some(Choice::Always),
            "no" => Some(Choice::No),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Choice::Yes => "yes",
            Choice::Always => "always",
            Choice::No => "no",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NewPrompt {
    pub kind: String,
    pub terminal: String,
    pub agent: String,
    pub terminal_name: Option<String>,
    pub cwd: Option<String>,
    pub place: Option<String>,
    pub unread: bool,
    pub plain: bool,
    pub text: Option<String>,
    pub tool_name: Option<String>,
    pub tool_input: Option<Value>,
    pub detail: Option<String>,
    pub screen_key: Option<String>,
    pub prompt_id: Option<String>,
    pub questions: Vec<Question>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Prompt {
    pub id: String,
    pub kind: PromptKind,
    pub agent: String,
    pub terminal: String,
    pub terminal_name: Option<String>,
    pub cwd: Option<String>,
    pub place: Option<String>,
    pub unread: bool,
    pub plain: bool,
    pub text: Option<String>,
    pub tool_name: Option<String>,
    pub detail: Option<String>,
    pub screen_key: Option<String>,
    pub prompt_id: Option<String>,
    pub questions: Vec<Question>,
    pub question_index: usize,
    pub created_at: u64,
    pub asked_at: u64,
}

impl Prompt {
    pub fn question(&self) -> Option<&Question> {
        self.questions.get(self.question_index)
    }

    pub fn from_screen(&self) -> bool {
        self.prompt_id.as_deref().is_some_and(|p| p.starts_with(SCREEN_PREFIX))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Answer {
    pub choice: Option<Choice>,
    pub options: Vec<usize>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Type(String),
    Permission(Permission),
    Plan { approve: bool, text: Option<String> },
    Option(usize),
    Options(Vec<usize>),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Answered {
    pub id: String,
    pub terminal: String,
    pub asked: String,
    pub said: String,
    pub at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recent {
    pub terminal: String,
    pub key: Option<String>,
    pub at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeepWait {
    pub ask: String,
    pub prompt: String,
    pub terminal: String,
    pub sig: String,
    pub at: u64,
}

#[derive(Debug, Default)]
pub struct Book {
    pub items: Vec<Prompt>,
    pub counter: u64,
    pub sending: Option<String>,
    pub last_sent: Option<String>,
    pub untold: HashSet<String>,
    pub told_auto: HashMap<String, u64>,
    pub answered: Vec<Answered>,
    pub used_said_at: u64,
    pub recent: Vec<Recent>,
    pub missing: HashMap<String, u64>,
    pub deep: Vec<DeepWait>,
}

pub fn permission_options() -> Vec<QuestionOption> {
    ["Yes", "Yes, always", "No"].iter().map(|l| QuestionOption { label: l.to_string(), description: None }).collect()
}

pub fn plan_options() -> Vec<QuestionOption> {
    ["Approve", "Reject"].iter().map(|l| QuestionOption { label: l.to_string(), description: None }).collect()
}

pub fn plain_options() -> Vec<QuestionOption> {
    ["Yes", "No"].iter().map(|l| QuestionOption { label: l.to_string(), description: None }).collect()
}

pub fn input_detail(input: Option<&Value>) -> Option<String> {
    let input = input?.as_object()?;
    let parts: Vec<&str> = INPUT_KEYS.iter().filter_map(|k| input.get(*k).and_then(Value::as_str)).map(str::trim).filter(|s| !s.is_empty()).collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

pub fn short(text: &str) -> String {
    if text.chars().count() > DETAIL_CHARS {
        format!("{}…", text.chars().take(DETAIL_CHARS).collect::<String>())
    } else {
        text.to_string()
    }
}

pub fn option_labels(prompt: &Prompt) -> Vec<String> {
    match prompt.kind {
        PromptKind::Permission => vec!["yes".into(), "yes, always".into(), "no".into()],
        PromptKind::Plan => vec!["yes, approve".into(), "no, reject".into()],
        PromptKind::Question => prompt.question().map(|q| q.options.iter().map(|o| o.label.clone()).collect()).unwrap_or_default(),
    }
}

pub fn answer_from_click(prompt: &Prompt, indexes: &[usize]) -> Answer {
    let first = indexes.first().copied();
    match prompt.kind {
        PromptKind::Permission => Answer { choice: Some(match first { Some(0) => Choice::Yes, Some(1) => Choice::Always, _ => Choice::No }), ..Answer::default() },
        PromptKind::Plan => Answer { choice: Some(if first == Some(0) { Choice::Yes } else { Choice::No }), ..Answer::default() },
        PromptKind::Question => Answer { options: indexes.iter().map(|i| i + 1).collect(), ..Answer::default() },
    }
}

static YES_LABEL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(yes|ok|okay|allow|approve|accept|proceed|sure)\b").expect("yes label"));
static NO_LABEL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(no|deny|reject|cancel|skip)\b").expect("no label"));

fn find_label(labels: &[String], choice: Choice) -> Option<usize> {
    let want: &Regex = if choice == Choice::No { &NO_LABEL } else { &YES_LABEL };
    labels.iter().position(|l| want.is_match(l))
}

pub fn plan(prompt: &Prompt, answer: &Answer) -> Result<(Step, String), String> {
    let labels = option_labels(prompt);
    let text = answer.text.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
    if prompt.plain {
        let picked = (answer.options.len() == 1).then(|| answer.options[0].checked_sub(1).and_then(|i| labels.get(i).cloned())).flatten();
        let word = match answer.choice {
            Some(Choice::No) => Some("No".to_string()),
            Some(_) => Some("Yes".to_string()),
            None => None,
        };
        let said = text.or(picked).or(word).ok_or("This agent asks in plain text. Answer with text, for example yes or no.")?;
        return Ok((Step::Type(said.clone()), said));
    }
    match prompt.kind {
        PromptKind::Permission => {
            let choice = answer.choice.ok_or("A permission needs choice yes, always or no.")?;
            let permission = match choice {
                Choice::Yes => Permission::Allow,
                Choice::Always => Permission::Always,
                Choice::No => Permission::Deny,
            };
            Ok((Step::Permission(permission), choice.word().to_string()))
        }
        PromptKind::Plan => {
            if answer.choice.is_none() && text.is_none() {
                return Err("A plan needs choice yes or no.".into());
            }
            let approve = matches!(answer.choice, Some(Choice::Yes | Choice::Always));
            let said = if approve { "approved" } else { "rejected" };
            Ok((Step::Plan { approve, text: if approve { None } else { text } }, said.into()))
        }
        PromptKind::Question => {
            let multi = prompt.question().is_some_and(|q| q.multi_select);
            if !answer.options.is_empty() {
                if answer.options.iter().any(|&n| n == 0 || n > labels.len()) {
                    return Err(format!("Options are 1 to {}.", labels.len()));
                }
                if answer.options.len() > 1 && !multi {
                    return Err("This question takes only one option.".into());
                }
                let indexes: Vec<usize> = answer.options.iter().map(|n| n - 1).collect();
                let said = indexes.iter().map(|&i| labels[i].clone()).collect::<Vec<_>>().join(" and ");
                let step = if indexes.len() == 1 && !multi { Step::Option(indexes[0]) } else { Step::Options(indexes) };
                return Ok((step, said));
            }
            if let Some(text) = text {
                return Ok((Step::Text(text.clone()), text));
            }
            if let Some(choice) = answer.choice {
                let want = if choice == Choice::Always { Choice::Yes } else { choice };
                let index = find_label(&labels, want).ok_or("This question has no yes or no option. Pick an option number or give text.")?;
                return Ok((Step::Option(index), labels[index].clone()));
            }
            Err("Give a choice, option numbers or text.".into())
        }
    }
}

impl Book {
    fn is_duplicate(&self, event: &NewPrompt, kind: PromptKind, at: u64) -> bool {
        let screen = event.prompt_id.as_deref().is_some_and(|p| p.starts_with(SCREEN_PREFIX));
        self.items.iter().any(|item| {
            if event.prompt_id.is_some() && item.prompt_id == event.prompt_id {
                return true;
            }
            if item.terminal != event.terminal {
                return false;
            }
            if item.kind == kind && item.text.as_deref().unwrap_or("") == event.text.as_deref().unwrap_or("") && at.saturating_sub(item.created_at) < DEDUPE_MS {
                return true;
            }
            screen
        })
    }

    pub fn add(&mut self, event: NewPrompt, at: u64) -> Option<Prompt> {
        let kind = PromptKind::parse(&event.kind)?;
        if event.terminal.trim().is_empty() || self.is_duplicate(&event, kind, at) {
            return None;
        }
        if !event.prompt_id.as_deref().is_some_and(|p| p.starts_with(SCREEN_PREFIX)) {
            self.items.retain(|item| item.terminal != event.terminal);
        }
        self.counter += 1;
        let detail = event.detail.clone().filter(|d| !d.trim().is_empty()).or_else(|| input_detail(event.tool_input.as_ref()));
        let prompt = Prompt {
            id: format!("p{}", self.counter),
            kind,
            agent: if event.agent.trim().is_empty() { "unknown".into() } else { event.agent.trim().to_lowercase() },
            terminal: event.terminal,
            terminal_name: event.terminal_name,
            cwd: event.cwd,
            place: event.place,
            unread: event.unread,
            plain: event.plain,
            text: event.text,
            tool_name: event.tool_name,
            detail,
            screen_key: event.screen_key,
            prompt_id: event.prompt_id,
            questions: event.questions,
            question_index: 0,
            created_at: at,
            asked_at: at,
        };
        self.items.push(prompt.clone());
        Some(prompt)
    }

    pub fn get(&self, id: &str) -> Option<&Prompt> {
        let id = id.trim().to_lowercase();
        self.items.iter().find(|item| item.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Prompt> {
        let id = id.trim().to_lowercase();
        self.items.iter_mut().find(|item| item.id == id)
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|item| item.id != id);
        self.untold.remove(id);
        self.told_auto.remove(id);
        let left: HashSet<String> = self.items.iter().map(|p| p.terminal.clone()).collect();
        self.missing.retain(|terminal, _| left.contains(terminal));
        before != self.items.len()
    }

    pub fn by_terminal(&self, terminal: &str) -> Vec<Prompt> {
        self.items.iter().filter(|item| item.terminal == terminal).cloned().collect()
    }

    pub fn note_sent(&mut self, terminal: &str, key: Option<String>, at: u64) {
        self.recent.push(Recent { terminal: terminal.to_string(), key, at });
        let extra = self.recent.len().saturating_sub(RECENT_KEEP);
        self.recent.drain(..extra);
    }

    pub fn sent_lately(&self, terminal: &str, key: Option<&str>, now: u64) -> bool {
        self.recent.iter().any(|r| {
            let age = now.saturating_sub(r.at);
            r.terminal == terminal
                && match (r.key.as_deref(), key) {
                    (Some(old), Some(new)) => old == new && age < RECENT_KEY_MS,
                    _ => age < RECENT_ANY_MS,
                }
        })
    }

    pub fn note_answered(&mut self, prompt: &Prompt, said: &str, at: u64) {
        let asked = prompt.question().map(|q| q.question.clone()).or_else(|| prompt.text.clone()).unwrap_or_default();
        self.answered.push(Answered { id: prompt.id.clone(), terminal: prompt.terminal.clone(), asked, said: said.to_string(), at });
        let extra = self.answered.len().saturating_sub(ANSWERED_KEEP);
        self.answered.drain(..extra);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(kind: &str, terminal: &str, text: &str) -> NewPrompt {
        NewPrompt { kind: kind.into(), terminal: terminal.into(), agent: "Claude".into(), text: Some(text.into()), ..NewPrompt::default() }
    }

    fn question(options: &[&str], multi: bool) -> Question {
        Question { question: "Which one?".into(), header: None, options: options.iter().map(|l| QuestionOption { label: l.to_string(), description: None }).collect(), multi_select: multi }
    }

    #[test]
    fn adds_and_dedupes() {
        let mut book = Book::default();
        let p = book.add(event("permission", "m1", "rm -rf build"), 1000).unwrap();
        assert_eq!(p.id, "p1");
        assert_eq!(p.agent, "claude");
        assert!(book.add(event("permission", "m1", "rm -rf build"), 2000).is_none());
        assert!(book.add(event("chat", "m1", "x"), 2000).is_none());
        let again = book.add(event("permission", "m1", "rm -rf build"), 9000).unwrap();
        assert_eq!(again.id, "p2");
        assert_eq!(book.items.len(), 1);
        let mut screen = event("question", "m1", "x");
        screen.prompt_id = Some("screen:m1:1".into());
        assert!(book.add(screen.clone(), 9500).is_none());
        screen.terminal = "m2".into();
        assert!(book.add(screen.clone(), 9500).is_some());
        assert!(book.add(screen, 9600).is_none());
        assert_eq!(book.items.len(), 2);
        assert!(book.get(" P2 ").is_some());
        assert!(book.remove("p2"));
        assert!(!book.remove("p2"));
    }

    #[test]
    fn detail_comes_from_tool_input() {
        let mut book = Book::default();
        let mut e = event("permission", "m1", "");
        e.tool_input = Some(json!({"command": "ls -la", "path": "/tmp", "other": 3}));
        assert_eq!(book.add(e, 1).unwrap().detail.as_deref(), Some("ls -la · /tmp"));
        assert_eq!(input_detail(Some(&json!({"x": 1}))), None);
    }

    fn prompt(kind: PromptKind, q: Option<Question>, plain: bool) -> Prompt {
        Prompt {
            id: "p1".into(), kind, agent: "claude".into(), terminal: "m1".into(), terminal_name: None, cwd: None, place: None, unread: false, plain,
            text: Some("text".into()), tool_name: None, detail: None, screen_key: None, prompt_id: None,
            questions: q.into_iter().collect(), question_index: 0, created_at: 0, asked_at: 0,
        }
    }

    #[test]
    fn plans_permissions_and_plans() {
        let p = prompt(PromptKind::Permission, None, false);
        assert_eq!(plan(&p, &answer_from_click(&p, &[1])).unwrap(), (Step::Permission(Permission::Always), "always".into()));
        assert_eq!(plan(&p, &answer_from_click(&p, &[2])).unwrap().0, Step::Permission(Permission::Deny));
        assert!(plan(&p, &Answer::default()).is_err());
        let plan_prompt = prompt(PromptKind::Plan, None, false);
        assert_eq!(plan(&plan_prompt, &answer_from_click(&plan_prompt, &[0])).unwrap(), (Step::Plan { approve: true, text: None }, "approved".into()));
        let no = Answer { choice: Some(Choice::No), text: Some("smaller steps".into()), ..Answer::default() };
        assert_eq!(plan(&plan_prompt, &no).unwrap().0, Step::Plan { approve: false, text: Some("smaller steps".into()) });
    }

    #[test]
    fn plans_questions() {
        let single = prompt(PromptKind::Question, Some(question(&["SQLite", "Postgres"], false)), false);
        assert_eq!(plan(&single, &answer_from_click(&single, &[1])).unwrap(), (Step::Option(1), "Postgres".into()));
        assert!(plan(&single, &Answer { options: vec![1, 2], ..Answer::default() }).unwrap_err().contains("only one"));
        assert!(plan(&single, &Answer { options: vec![3], ..Answer::default() }).unwrap_err().contains("1 to 2"));
        assert_eq!(plan(&single, &Answer { text: Some("MySQL".into()), ..Answer::default() }).unwrap().0, Step::Text("MySQL".into()));
        assert!(plan(&single, &Answer { choice: Some(Choice::Yes), ..Answer::default() }).is_err());
        let multi = prompt(PromptKind::Question, Some(question(&["Tests", "Docs", "Lint"], true)), false);
        assert_eq!(plan(&multi, &answer_from_click(&multi, &[0, 2])).unwrap(), (Step::Options(vec![0, 2]), "Tests and Lint".into()));
        assert_eq!(plan(&multi, &answer_from_click(&multi, &[1])).unwrap().0, Step::Options(vec![1]));
        let yes_no = prompt(PromptKind::Question, Some(question(&["Yes, go", "No"], false)), false);
        assert_eq!(plan(&yes_no, &Answer { choice: Some(Choice::Always), ..Answer::default() }).unwrap().0, Step::Option(0));
        assert_eq!(plan(&yes_no, &Answer { choice: Some(Choice::No), ..Answer::default() }).unwrap().0, Step::Option(1));
    }

    #[test]
    fn plans_plain_questions() {
        let plain = prompt(PromptKind::Question, Some(Question { question: "Run it?".into(), options: plain_options(), ..Question::default() }), true);
        assert_eq!(plan(&plain, &answer_from_click(&plain, &[1])).unwrap(), (Step::Type("No".into()), "No".into()));
        assert_eq!(plan(&plain, &Answer { choice: Some(Choice::Yes), ..Answer::default() }).unwrap().0, Step::Type("Yes".into()));
        assert_eq!(plan(&plain, &Answer { text: Some("only the fast ones".into()), ..Answer::default() }).unwrap().0, Step::Type("only the fast ones".into()));
        assert!(plan(&plain, &Answer::default()).is_err());
    }

    #[test]
    fn a_sent_dialog_is_not_read_again_as_new() {
        let mut book = Book::default();
        book.note_sent("m1", Some("Which db?|SQLite|Postgres".into()), 1000);
        assert!(book.sent_lately("m1", Some("Which db?|SQLite|Postgres"), 5000));
        assert!(!book.sent_lately("m1", Some("Which db?|SQLite|Postgres"), 1000 + RECENT_KEY_MS));
        assert!(!book.sent_lately("m1", Some("Name?|A|B"), 2000));
        assert!(book.sent_lately("m1", None, 2000));
        assert!(!book.sent_lately("m1", None, 1000 + RECENT_ANY_MS));
        assert!(!book.sent_lately("m2", Some("Which db?|SQLite|Postgres"), 2000));
        book.note_sent("m2", None, 3000);
        assert!(book.sent_lately("m2", Some("x"), 4000));
        for at in 0..30 {
            book.note_sent("m3", None, at);
        }
        assert_eq!(book.recent.len(), RECENT_KEEP);
    }

    #[test]
    fn keeps_the_last_answers() {
        let mut book = Book::default();
        let p = prompt(PromptKind::Question, Some(question(&["a"], false)), false);
        for at in 0..30 {
            book.note_answered(&p, "a", at);
        }
        assert_eq!(book.answered.len(), ANSWERED_KEEP);
        assert_eq!(book.answered[0].at, 10);
        assert_eq!(book.answered[0].asked, "Which one?");
    }
}
