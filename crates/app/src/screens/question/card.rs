use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardKind {
    Question,
    Permission,
    Plan,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardState {
    #[default]
    Waiting,
    Sending,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardOption {
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    pub id: String,
    pub kind: CardKind,
    #[serde(default)]
    pub agent: String,
    pub terminal: String,
    #[serde(default)]
    pub terminal_name: Option<String>,
    #[serde(default)]
    pub place: Option<String>,
    pub title: String,
    #[serde(default)]
    pub header: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub options: Vec<CardOption>,
    #[serde(default)]
    pub multi: bool,
    #[serde(default = "one")]
    pub step: usize,
    #[serde(default = "one")]
    pub steps: usize,
    #[serde(default)]
    pub state: CardState,
    #[serde(default)]
    pub unread: bool,
    #[serde(default)]
    pub auto: bool,
}

fn one() -> usize {
    1
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Snapshot {
    pub card: Option<Card>,
    pub waiting: usize,
    pub last_sent: Option<String>,
}

pub fn snapshot_of(message: &Value) -> Option<Snapshot> {
    if message.get("type").and_then(Value::as_str) != Some("prompts") {
        return None;
    }
    serde_json::from_value(message.clone()).ok()
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
            chars
                .next()
                .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        }
    }
}

pub fn kind_label(kind: CardKind) -> &'static str {
    match kind {
        CardKind::Question => "Question",
        CardKind::Permission => "Permission",
        CardKind::Plan => "Plan",
    }
}

pub fn kind_words(kind: CardKind) -> &'static str {
    match kind {
        CardKind::Question => "asks",
        CardKind::Permission => "wants your OK",
        CardKind::Plan => "proposes steps",
    }
}

pub fn place_words(card: &Card) -> String {
    let place = card
        .place
        .as_deref()
        .filter(|p| !p.is_empty())
        .map(|p| format!(" in {p}"))
        .unwrap_or_default();
    format!(
        "{}{place} · terminal {}",
        kind_words(card.kind),
        card.terminal
    )
}

pub fn card_agent_title(card: &Card) -> String {
    card.terminal_name.clone().filter(|name| !name.trim().is_empty()).unwrap_or_else(|| agent_title(&card.agent))
}

pub fn hint(card: &Card, name: &str, voice: bool) -> String {
    if card.unread && card.auto {
        return format!("{name} reads its screen and answers it there.");
    }
    if card.unread {
        return format!(
            "{name} cannot read the options on this screen. Go to {} and answer it there.",
            card_agent_title(card)
        );
    }
    if card.options.is_empty() {
        return "No options to click. Say or type your answer.".into();
    }
    if !voice {
        return "Click an answer.".into();
    }
    match card.kind {
        CardKind::Question => format!("Just say your answer. {name} passes it on."),
        CardKind::Permission => format!("Say yes, always or no. {name} passes it on."),
        CardKind::Plan => format!("Say yes to approve or no to reject. {name} passes it on."),
    }
}

pub fn waiting_words(waiting: usize) -> String {
    if waiting > 0 {
        format!("{waiting} more waiting")
    } else {
        "Nothing else waiting".into()
    }
}

pub fn send_words(count: usize) -> String {
    if count > 0 {
        format!("Send {count} picked")
    } else {
        "Pick first".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_the_brain_snapshot() {
        let message = json!({
            "type": "prompts",
            "card": {"id": "p2", "kind": "permission", "agent": "claude", "terminal": "m3", "place": "shop", "title": "Wants to use Bash",
                     "detail": "rm -rf build", "options": [{"label": "Yes"}, {"label": "Yes, always"}, {"label": "No"}], "multi": false,
                     "step": 1, "steps": 1, "state": "sending", "unread": false},
            "waiting": 2, "mode": "auto", "last_sent": null
        });
        let snap = snapshot_of(&message).unwrap();
        let card = snap.card.unwrap();
        assert_eq!(card.kind, CardKind::Permission);
        assert_eq!(card.state, CardState::Sending);
        assert_eq!(card.options.len(), 3);
        assert_eq!(snap.waiting, 2);
        assert_eq!(place_words(&card), "wants your OK in shop · terminal m3");
        assert!(snapshot_of(&json!({"type": "chat"})).is_none());
        let empty =
            snapshot_of(&json!({"type": "prompts", "card": null, "waiting": 0, "mode": "ask"}))
                .unwrap();
        assert!(empty.card.is_none());
    }

    #[test]
    fn words() {
        let card = Card {
            id: "p1".into(),
            kind: CardKind::Question,
            agent: "codex".into(),
            terminal: "m1".into(),
            terminal_name: None,
            place: None,
            title: "Which?".into(),
            header: None,
            detail: None,
            options: vec![],
            multi: false,
            step: 1,
            steps: 1,
            state: CardState::Waiting,
            unread: false,
            auto: false,
        };
        assert_eq!(
            hint(&card, "Nixa", true),
            "No options to click. Say or type your answer."
        );
        assert_eq!(
            hint(
                &Card {
                    unread: true,
                    ..card.clone()
                },
                "Nixa",
                true
            ),
            "Nixa cannot read the options on this screen. Go to Codex and answer it there."
        );
        assert_eq!(
            hint(
                &Card {
                    unread: true,
                    auto: true,
                    ..card.clone()
                },
                "Nixa",
                true
            ),
            "Nixa reads its screen and answers it there."
        );
        let with = Card {
            options: vec![CardOption {
                label: "a".into(),
                description: None,
            }],
            ..card
        };
        assert_eq!(hint(&with, "Nixa", false), "Click an answer.");
        assert_eq!(
            hint(&with, "Nixa", true),
            "Just say your answer. Nixa passes it on."
        );
        assert_eq!(waiting_words(0), "Nothing else waiting");
        assert_eq!(waiting_words(3), "3 more waiting");
        assert_eq!(send_words(2), "Send 2 picked");
        assert_eq!(agent_title("unknown"), "An agent");
    }
}
