use super::answer::{Note, Personal, Web, short};
use crate::app::App;
use crate::connectors::manifest::{FieldType, Permission};
use crate::tools::Args;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;

pub const WEB_NAME: &str = "web search";
pub const QUERY_MAX: usize = 240;
pub const NO_KEY: &str = "web search has no Exa key yet. Add one in Settings.";

pub fn memory(app: &App, query: &str, count: usize) -> Vec<String> {
    app.memory.search(query).into_iter().take(count).collect()
}

pub fn web_name(app: &App) -> String {
    let setup = crate::web::setup::load(app);
    if setup.on { format!("{WEB_NAME} ({})", setup.provider.label()) } else { WEB_NAME.to_string() }
}

pub fn web_ready(app: &App) -> bool {
    crate::web::setup::load(app).on
}

pub fn web_result(found: Result<String, String>) -> Web {
    match found {
        Ok(text) => Web::Text(text),
        Err(why) if why == crate::web::OFF => Web::Off(String::new()),
        Err(why) if why == crate::web::NO_EXA_KEY => Web::Off(NO_KEY.into()),
        Err(why) if why.contains("private") || why.contains("public") => Web::Off(why),
        Err(_) => Web::Failed,
    }
}

pub async fn web(app: &Arc<App>, query: &str, count: usize) -> Web {
    let mut args = Args::new();
    args.insert("query".into(), json!(short(query, QUERY_MAX)));
    args.insert("count".into(), json!(count));
    web_result(crate::web::search(app, &args).await)
}

pub const MAIL_WORDS: [&str; 3] = ["mail", "email", "inbox"];
pub const CALENDAR_WORDS: [&str; 4] = ["calendar", "event", "meeting", "agenda"];
pub const HELD: &str = "The connector call waits for the user's yes on the connector card.";

pub fn knowledge_ready(app: &App) -> bool {
    crate::knowledge::chunks(app) > 0
}

pub async fn knowledge(app: &Arc<App>, query: &str, count: usize) -> Vec<Note> {
    let wanted = crate::knowledge::index::Query { limit: count, ..Default::default() };
    crate::knowledge::search(app, &short(query, QUERY_MAX), wanted)
        .await
        .into_iter()
        .take(count)
        .map(|h| Note { text: h.text, source: h.source, path: h.path, indexed: h.indexed_at.unwrap_or_default() })
        .collect()
}

pub fn personal_pick(root: &Path, mail: bool) -> Option<(String, String, String)> {
    let words: &[&str] = if mail { &MAIL_WORDS } else { &CALENDAR_WORDS };
    for manifest in crate::connectors::call::working(root) {
        for action in &manifest.actions {
            let texts: Vec<&String> = action.properties.iter().filter(|(_, f)| f.kind == FieldType::String).map(|(k, _)| k).collect();
            if action.permission != Permission::Read || action.stop || texts.len() != 1 || action.properties.len() != 1 {
                continue;
            }
            let about = format!("{} {} {} {}", manifest.connector, manifest.description, action.name, action.description).to_lowercase();
            if words.iter().any(|w| about.split(|c: char| !c.is_alphanumeric()).any(|part| part == *w)) {
                return Some((manifest.connector.clone(), action.name.clone(), texts[0].clone()));
            }
        }
    }
    None
}

pub fn personal_ready(app: &App) -> bool {
    personal_pick(&app.connectors.root, true).is_some() || personal_pick(&app.connectors.root, false).is_some()
}

pub async fn personal(app: &Arc<App>, question: &str, mail: bool) -> Option<Result<Personal, String>> {
    let (connector, action, field) = personal_pick(&app.connectors.root, mail)?;
    let mut args = Args::new();
    args.insert("connector".into(), json!(connector));
    args.insert("action".into(), json!(action));
    args.insert("input".into(), json!({ field: short(question, QUERY_MAX) }));
    let found = crate::connectors::run_call(app, &args).await;
    Some(match found {
        Ok(text) if serde_json::from_str::<Value>(&text).is_ok_and(|v| v["outcome"] == "held") => Err(HELD.into()),
        Ok(text) => Ok(Personal { name: connector, text, observed: super::lookup::stamp(app) }),
        Err(why) => Err(why),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install(root: &Path, name: &str, manifest: Value) {
        let folder = root.join(format!("connector-{name}"));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("connector.json"), manifest.to_string()).unwrap();
    }

    fn one(name: &str, description: &str, permission: &str, properties: Value) -> Value {
        json!({"connector": name, "version": 1, "description": description, "hosts": ["localhost"],
            "actions": [{"name": "find", "description": "Find by words", "permission": permission, "reach": "outside",
                "inputSchema": {"type": "object", "properties": properties}, "request": {"url": "https://localhost/find"}}]})
    }

    #[test]
    fn personal_lookups_use_one_read_action_with_one_text_input() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert_eq!(personal_pick(root, true), None);
        install(root, "notes", one("notes", "My notes", "read", json!({"q": {"type": "string"}})));
        install(root, "sendmail", one("sendmail", "Send email", "write", json!({"q": {"type": "string"}})));
        install(root, "mailtwo", one("mailtwo", "Search email", "read", json!({"q": {"type": "string"}, "box": {"type": "string"}})));
        assert_eq!(personal_pick(root, true), None);
        install(root, "inbox", one("inbox", "Search email", "read", json!({"words": {"type": "string"}})));
        install(root, "cal", one("cal", "Calendar events", "read", json!({"q": {"type": "string"}})));
        assert_eq!(personal_pick(root, true), Some(("inbox".into(), "find".into(), "words".into())));
        assert_eq!(personal_pick(root, false), Some(("cal".into(), "find".into(), "q".into())));
    }
}
