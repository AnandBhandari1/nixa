use super::{KEYS, QUIET_MAX_MINUTES, READ_NOTE, applies, brains, choices, label, raw, read, shown};
use crate::app::{App, lock};
use crate::tools::{Args, ToolResult, need};
use crate::util::now_ms;
use serde_json::json;

pub(super) struct Section {
    pub id: &'static str,
    title: &'static str,
    keys: &'static [&'static str],
}

const fn section(id: &'static str, title: &'static str, keys: &'static [&'static str]) -> Section {
    Section { id, title, keys }
}

pub(super) const SECTIONS: [Section; 20] = [
    section("general", "General", &["nixa", "headphones", "privacy", "git_voice", "agent_voice"]),
    section("chat", "Chat brain", &["brain", "model", "effort"]),
    section("repeat_messages", "Repeat messages", &[]),
    section("memory", "Memory", &[]),
    section("knowledge", "Knowledge", &[]),
    section("sound", "Mic and speaker", &["mic", "voice", "listen", "quiet"]),
    section("voices", "Voices", &[]),
    section("phone", "Phone", &[]),
    section("privacy", "Privacy", &[]),
    section("nixa_screen", "Nixa screen", &[]),
    section("spaces", "Spaces", &[]),
    section("agents", "Agents and team", &[]),
    section("workflows", "Workflows", &[]),
    section("rules", "Rules", &[]),
    section("schedules", "Schedules", &[]),
    section("git", "Git accounts", &[]),
    section("web_search", "Web search", &[]),
    section("connectors", "Connectors", &[]),
    section("look", "Look", &[]),
    section("app", "App", &[]),
];

const CHOICE_LIMIT: usize = 50;

pub(super) fn call(app: &App, args: &Args) -> ToolResult {
    let selected = args.contains_key("section").then(|| need(args, "section")).transpose()?;
    let key = args.contains_key("key").then(|| need(args, "key")).transpose()?;
    let selected = selected.as_deref().map(|wanted| {
        SECTIONS.iter().find(|section| section.id == wanted)
            .ok_or_else(|| format!("Unknown settings section {wanted}. Use describe to list sections."))
    }).transpose()?;
    if let Some(key) = key.as_deref() {
        if !KEYS.contains(&key) {
            return Err(format!("Unknown setting {key}. Use describe to list supported settings."));
        }
        if let Some(section) = selected
            && !section.keys.contains(&key)
        {
            return Err(format!("{key} is not a supported setting in {}.", section.title));
        }
    }

    let now = read(app);
    let models = lock(&app.brain).models.clone();
    let saved = brains(app);
    let at = now_ms();
    let mut settings = Vec::new();
    let mut sections = Vec::new();
    for section in &SECTIONS {
        if selected.is_some_and(|selected| section.id != selected.id)
            || key.as_deref().is_some_and(|key| !section.keys.contains(&key))
        {
            continue;
        }
        sections.push(json!({"id": section.id, "title": section.title, "keys": section.keys,
            "availability": if section.keys.is_empty() { "unavailable_in_this_tool" } else { "supported" }}));
        for &setting in section.keys {
            if key.as_deref().is_some_and(|key| setting != key) {
                continue;
            }
            let availability = match setting {
                "brain" if saved.is_empty() => "no_choices_configured",
                "model" | "effort" if now.brain.is_none() => "no_brain_selected",
                "effort" if now.model.is_none() => "no_model_selected",
                "model" | "effort" if models.is_empty() => "model_choices_unobserved",
                _ => "available",
            };
            let mut allowed = choices(setting, &saved, &models, now.model.as_deref());
            let count = allowed.len();
            allowed.truncate(CHOICE_LIMIT);
            let mut row = json!({"section": section.id, "key": setting, "label": label(setting),
                "value": shown(&now, setting, at), "current": raw(&now, setting),
                "applies": applies(setting), "availability": availability});
            if setting == "quiet" {
                row["input"] = json!({"minutes": {"type": "number", "minimum": 0, "maximum": QUIET_MAX_MINUTES}});
                row["scope"] = json!("current session");
            } else {
                row["choices"] = json!(allowed);
                row["choicesTotal"] = json!(count);
            }
            if setting == "effort" {
                row["defaultInput"] = json!("");
            }
            settings.push(row);
        }
    }
    Ok(json!({"sections": sections, "settings": settings, "complete": false, "note": READ_NOTE,
        "source": "Local settings and previously loaded model choices. Device and provider readiness are not checked."}).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn every_supported_key_belongs_to_one_section() {
        let mut keys: Vec<_> = SECTIONS.iter().flat_map(|section| section.keys.iter().copied()).collect();
        keys.sort_unstable();
        let mut wanted = KEYS;
        wanted.sort_unstable();
        assert_eq!(keys, wanted);
    }

    #[tokio::test]
    async fn describe_is_a_local_read_and_marks_unsupported_sections() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        app.save_settings(|s| s.choice = Some("api:01234567".into())).unwrap();
        let args = json!({"action": "describe", "section": "chat", "key": "model"});
        let out = super::super::call(&app, args.as_object().unwrap()).await.unwrap();
        let out: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(out["sections"][0]["title"], "Chat brain");
        assert_eq!(out["settings"][0]["availability"], "model_choices_unobserved");
        assert_eq!(out["settings"][0]["choices"], json!([]));
        assert!(lock(&app.brain).models.is_empty());

        let phone = json!({"action": "describe", "section": "phone"});
        let out = super::super::call(&app, phone.as_object().unwrap()).await.unwrap();
        let out: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(out["sections"][0]["availability"], "unavailable_in_this_tool");
        assert_eq!(out["settings"], json!([]));
        let mixed = json!({"action": "describe", "section": "phone", "key": "model"});
        assert!(super::super::call(&app, mixed.as_object().unwrap()).await.is_err());
    }
}
