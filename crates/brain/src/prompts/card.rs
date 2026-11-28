use super::book::{Prompt, PromptKind, permission_options, plan_options, short};
use super::dialog::QuestionOption;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CardState {
    Waiting,
    Sending,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Card {
    pub id: String,
    pub kind: PromptKind,
    pub agent: String,
    pub terminal: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub options: Vec<QuestionOption>,
    pub multi: bool,
    pub step: usize,
    pub steps: usize,
    pub state: CardState,
    pub unread: bool,
    pub auto: bool,
}

pub fn agent_title(agent: &str) -> String {
    match agent {
        "" | "unknown" => "An agent".into(),
        "agy" | "antigravity" => "Antigravity".into(),
        "omp" => "Oh My Pi".into(),
        "opencode" => "OpenCode".into(),
        "codex" => "Codex".into(),
        "claude" => "Claude".into(),
        "cursor" | "cursor-agent" => "Cursor".into(),
        "kimi" => "Kimi".into(),
        other => {
            let mut chars = other.chars();
            chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
        }
    }
}

pub fn place_of(prompt: &Prompt) -> Option<String> {
    prompt.place.clone().filter(|p| !p.trim().is_empty()).or_else(|| {
        prompt.cwd.as_deref().and_then(|c| std::path::Path::new(c).file_name()).map(|n| n.to_string_lossy().to_string())
    })
}

pub fn unread_answer(prompt: &Prompt) -> String {
    let agent = agent_title(&prompt.agent);
    format!(
        "{agent} is waiting on its screen in terminal {}, and the options cannot be read from here. Do not hand it to the user: read the screen with nixa_term_read and answer it there with nixa_term_keys or nixa_term_send, or ask nixa_deep_ask when unsure.",
        prompt.terminal
    )
}

pub fn card_of(prompt: &Prompt, sending: Option<&str>, auto: bool) -> Card {
    let text = prompt.text.as_deref().unwrap_or("").trim().to_string();
    let state = if sending == Some(prompt.id.as_str()) { CardState::Sending } else { CardState::Waiting };
    if prompt.unread {
        return Card {
            id: prompt.id.clone(),
            kind: prompt.kind,
            agent: prompt.agent.clone(),
            terminal: prompt.terminal.clone(),
            terminal_name: prompt.terminal_name.clone(),
            place: place_of(prompt),
            title: match (text.is_empty(), auto) {
                (true, true) => "Being answered on its screen".into(),
                (true, false) => "Waiting for you on its screen".into(),
                (false, _) => short(&text),
            },
            header: None,
            detail: None,
            options: Vec::new(),
            multi: false,
            step: 1,
            steps: 1,
            state,
            unread: true,
            auto,
        };
    }
    let q = prompt.question();
    let title = match prompt.kind {
        PromptKind::Question => q.map(|q| q.question.clone()).filter(|t| !t.trim().is_empty()).unwrap_or_else(|| if text.is_empty() { "Question".into() } else { text.clone() }),
        PromptKind::Permission => format!("Wants to use {}", prompt.tool_name.as_deref().unwrap_or("a tool")),
        PromptKind::Plan if auto => "Plan ready, being decided".into(),
        PromptKind::Plan => "Plan ready to approve".into(),
    };
    let detail = match prompt.kind {
        PromptKind::Question => None,
        _ => {
            let joined = [prompt.detail.clone().unwrap_or_default(), text.clone()].into_iter().filter(|s| !s.trim().is_empty()).collect::<Vec<_>>().join(" — ");
            (!joined.is_empty()).then(|| short(&joined))
        }
    };
    let options = match prompt.kind {
        PromptKind::Question => q.map(|q| q.options.clone()).unwrap_or_default(),
        PromptKind::Permission => permission_options(),
        PromptKind::Plan => plan_options(),
    };
    Card {
        id: prompt.id.clone(),
        kind: prompt.kind,
        agent: prompt.agent.clone(),
        terminal: prompt.terminal.clone(),
        terminal_name: prompt.terminal_name.clone(),
        place: place_of(prompt),
        title,
        header: q.and_then(|q| q.header.clone()),
        detail,
        options,
        multi: q.is_some_and(|q| q.multi_select),
        step: prompt.question_index + 1,
        steps: prompt.questions.len().max(1),
        state,
        unread: false,
        auto,
    }
}

fn flat(text: String) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn describe(prompt: &Prompt) -> String {
    let place = place_of(prompt).map(|p| format!(" in {p}")).unwrap_or_default();
    let agent = if matches!(prompt.agent.as_str(), "" | "unknown") {
        prompt.terminal_name.clone().unwrap_or_else(|| agent_title(&prompt.agent))
    } else {
        agent_title(&prompt.agent)
    };
    let head = format!("Prompt {}: {agent}{place} (terminal {})", prompt.id, prompt.terminal);
    let text = short(prompt.text.as_deref().unwrap_or(""));
    let text = if text.trim().is_empty() { String::new() } else { crate::agent::data_block(&text) };
    if prompt.plain {
        let asked = if text.is_empty() { "\"\"".to_string() } else { text };
        return flat(format!("{head} asks in plain text, with no menu: {asked}. The answer is typed into it. Answer with nixa_answer_prompt text: \"yes\", \"no\" or other words."));
    }
    if prompt.unread {
        let what = if text.is_empty() { String::new() } else { format!(": {text}") };
        return flat(format!("{head} is waiting on its screen{what}. Its options cannot be read. Do not call nixa_answer_prompt for it. Read its screen with nixa_term_read and answer it there with nixa_term_keys or nixa_term_send, or ask nixa_deep_ask when unsure; do not hand it to the user."));
    }
    match prompt.kind {
        PromptKind::Permission => {
            let detail = prompt.detail.as_deref().map(|d| format!(": {}.", crate::agent::data_block(&short(d)))).unwrap_or_else(|| ".".into());
            let tool = crate::agent::data_block(prompt.tool_name.as_deref().unwrap_or("a tool"));
            flat(format!("{head} asks permission to use {tool}{detail} {text} Answers: yes, always, or no."))
        }
        PromptKind::Plan => flat(format!("{head} has a plan ready and asks to approve it. {text} Answers: yes or no, with optional feedback.")),
        PromptKind::Question => {
            let total = prompt.questions.len().max(1);
            let step = if total > 1 { format!(" (question {} of {total})", prompt.question_index + 1) } else { String::new() };
            let Some(q) = prompt.question() else {
                let asked = if text.is_empty() { "a question".to_string() } else { text };
                return format!("{head} asks{step}: {asked}. Answer with text.");
            };
            let options = q
                .options
                .iter()
                .enumerate()
                .map(|(i, o)| format!("{}. {}{}", i + 1, o.label, o.description.as_deref().map(|d| format!(" ({d})")).unwrap_or_default()))
                .collect::<Vec<_>>()
                .join("; ");
            let options = if options.is_empty() { String::new() } else { format!(" Options: {}.", crate::agent::data_block(&options)) };
            let multi = if q.multi_select { " More than one option may be picked." } else { "" };
            format!("{} asks{step}: {}{options}{multi} A typed text answer is also possible.", flat(head), crate::agent::data_block(&q.question))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompts::dialog::Question;

    fn prompt(kind: PromptKind) -> Prompt {
        Prompt {
            id: "p3".into(), kind, agent: "codex".into(), terminal: "m4".into(), terminal_name: None, cwd: Some("/Users/x/Projects/shop".into()), place: None,
            unread: false, plain: false, text: Some("It wants to run tests".into()), tool_name: Some("Bash".into()), detail: Some("npm test".into()),
            screen_key: None, prompt_id: None, questions: Vec::new(), question_index: 0, created_at: 0, asked_at: 0,
        }
    }

    #[test]
    fn permission_cards() {
        let p = prompt(PromptKind::Permission);
        let card = card_of(&p, Some("p3"), false);
        assert_eq!(card.title, "Wants to use Bash");
        assert_eq!(card.detail.as_deref(), Some("npm test — It wants to run tests"));
        assert_eq!(card.options.len(), 3);
        assert_eq!(card.state, CardState::Sending);
        assert_eq!(card.place.as_deref(), Some("shop"));
        let text = describe(&p);
        let b = |s: &str| flat(crate::agent::data_block(s));
        assert!(text.starts_with(&format!("Prompt p3: Codex in shop (terminal m4) asks permission to use {}: {}.", b("Bash"), b("npm test"))), "{text}");
        assert!(text.ends_with("Answers: yes, always, or no."));
    }

    #[test]
    fn question_cards() {
        let mut p = prompt(PromptKind::Question);
        p.questions = vec![
            Question { question: "Which db?".into(), header: Some("Storage".into()), options: vec![QuestionOption { label: "SQLite".into(), description: Some("local".into()) }, QuestionOption { label: "Postgres".into(), description: None }], multi_select: true },
            Question { question: "Name?".into(), ..Question::default() },
        ];
        let card = card_of(&p, None, false);
        assert_eq!(card.title, "Which db?");
        assert_eq!((card.step, card.steps, card.multi), (1, 2, true));
        assert_eq!(card.header.as_deref(), Some("Storage"));
        assert!(card.detail.is_none());
        let text = describe(&p);
        let b = crate::agent::data_block;
        assert!(text.contains(&format!("asks (question 1 of 2): {} Options: {}. More than one option may be picked.", b("Which db?"), b("1. SQLite (local); 2. Postgres"))), "{text}");
    }

    #[test]
    fn unread_and_plain() {
        let mut p = prompt(PromptKind::Question);
        p.unread = true;
        let card = card_of(&p, None, false);
        assert!(card.unread && card.options.is_empty());
        assert_eq!(card.title, "It wants to run tests");
        assert!(describe(&p).contains("Do not call nixa_answer_prompt"));
        assert!(describe(&p).contains("Read its screen"));
        assert!(unread_answer(&p).starts_with("Codex is waiting"));
        assert!(unread_answer(&p).contains("nixa_term_read"));
        p.text = None;
        assert_eq!(card_of(&p, None, true).title, "Being answered on its screen");
        assert!(card_of(&p, None, true).auto && !card_of(&p, None, false).auto);
        assert_eq!(card_of(&p, None, false).title, "Waiting for you on its screen");
        p.text = Some("It wants to run tests".into());
        p.unread = false;
        p.plain = true;
        assert!(describe(&p).contains(&format!("asks in plain text, with no menu: {}", flat(crate::agent::data_block("It wants to run tests")))));
        assert!(!describe(&p).contains("wait for their answer"));
    }

    #[test]
    fn agent_titles() {
        assert_eq!(agent_title("unknown"), "An agent");
        assert_eq!(agent_title("omp"), "Oh My Pi");
        assert_eq!(agent_title("fake_agent"), "Fake_agent");
    }
}
