pub mod routes;
pub mod run;
pub mod tool;

mod groups {
    pub mod chat_brain;
    pub mod desktop_browser_connectors_extras;
    pub mod memory_deep_trees;
    pub mod schedules_modes;
    pub mod settings_privacy;
    pub mod tasks_work;
    pub mod teams_agents_models;
    pub mod voice_talk_phone;
}

#[cfg(test)]
mod tests;

use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Read,
    Write,
}

impl Kind {
    pub fn word(self) -> &'static str {
        match self {
            Kind::Read => "read",
            Kind::Write => "write",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Body {
    #[default]
    Raw,
    Payload(&'static str),
    Field(&'static str, &'static str),
}

#[derive(Clone, Debug, Default)]
pub struct Action {
    pub name: &'static str,
    pub method: &'static str,
    pub path: &'static str,
    pub body: Body,
    pub kind: Kind,
    pub help: &'static str,
    pub inputs: Value,
    pub card: Option<&'static str>,
}

impl Action {
    pub fn view(&self) -> Value {
        json!({
            "name": self.name,
            "method": self.method,
            "path": self.path,
            "kind": self.kind.word(),
            "help": self.help,
            "inputs": self.inputs,
        })
    }

    pub fn params(&self) -> Vec<&'static str> {
        params_of(self.path)
    }
}

pub fn params_of(path: &'static str) -> Vec<&'static str> {
    path.split('/').filter_map(|part| part.strip_prefix('{').and_then(|p| p.strip_suffix('}'))).collect()
}

pub fn inputs(props: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": props, "required": required})
}

pub const SECRET_INPUTS: [&str; 7] = ["apiKey", "api_key", "token", "secret", "password", "exaKey", "keyValue"];
pub const SECRET_BODIES: [&str; 2] = ["web_key", "secret"];

pub type Group = (&'static str, fn() -> Vec<Action>);

pub const GROUPS: [Group; 8] = [
    ("chat_brain", groups::chat_brain::list),
    ("memory_deep_trees", groups::memory_deep_trees::list),
    ("settings_privacy", groups::settings_privacy::list),
    ("schedules_modes", groups::schedules_modes::list),
    ("tasks_work", groups::tasks_work::list),
    ("teams_agents_models", groups::teams_agents_models::list),
    ("voice_talk_phone", groups::voice_talk_phone::list),
    ("desktop_browser_connectors_extras", groups::desktop_browser_connectors_extras::list),
];

pub const INTERNAL: &[(&str, &str)] = &[
    ("/health", "app and CLI alive check"),
    ("/ws", "window live event socket"),
    ("/chat/completions", "test server in agent recovery tests"),
    ("/v1/chat/completions", "test server in usage event tests"),
    ("/v1/models", "test server in agent tests"),
    ("/tools", "the tool door itself"),
    ("/tool", "the tool door itself"),
    ("/agent/tools", "tool door for coding agents"),
    ("/agent/tool", "tool door for coding agents"),
    ("/actions", "the action list itself"),
    ("/actions/run", "the action runner itself"),
    ("/announce/numbers", "window sends tab numbers map"),
    ("/extras/world", "brain context text, already in prompt"),
    ("/schedules/picks", "UI picker lists repos in a folder"),
    ("/privacy/ask", "window checks a typed question"),
    ("/prompts/event", "agent hooks report a new prompt"),
    ("/talk/ask", "opens a yes card for a caller"),
    ("/talk/confirm", "window and nixa chat cards answer the open yes card, the brain must not approve its own card"),
    ("/deep/join", "nixa deep join, needs the deep brain agent's own terminal"),
    ("/deep/leave", "nixa deep leave, needs the deep brain agent's own terminal"),
    ("/deep/{id}/reply", "nixa deep decide, needs the deep brain agent's own terminal"),
    ("/talk/hear", "mic or typed speech into the talk flow"),
    ("/tasks/changed", "CLI tells the brain a task changed"),
    ("/universe/flow", "terminal agents report a message they sent"),
    ("/agents/leave", "window reads the agents on leave"),
    ("/agents/limit", "nixa agent limit, needs the agent's own terminal"),
    ("/agents/handover", "nixa agent handover, needs the agent's own terminal"),
    ("/brain/ollama", "window Ollama button makes or picks the Ollama brain"),
    ("/office/soap", "window Office view reads the soap state, window key only"),
    ("/office/soap/model", "window Office view picks the soap model, window key only"),
    ("/office/soap/test", "window Office view tests a soap scene, window key only"),
    ("/office/fun/test", "window Office view tests a fun line, window key only"),
    ("/office/talk", "window Office view reads the voice state, window key only"),
];

pub fn all() -> Vec<Action> {
    GROUPS.iter().flat_map(|(_, list)| list()).collect()
}

pub fn find(name: &str) -> Option<Action> {
    all().into_iter().find(|a| a.name == name)
}

pub fn matching(word: Option<&str>) -> Vec<Action> {
    let want = word.map(|w| w.trim().to_lowercase()).filter(|w| !w.is_empty());
    all()
        .into_iter()
        .filter(|a| want.as_ref().is_none_or(|w| format!("{} {} {}", a.name, a.path, a.help).to_lowercase().contains(w.as_str())))
        .collect()
}
