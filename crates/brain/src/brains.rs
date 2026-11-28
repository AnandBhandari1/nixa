use crate::provider::{Kind, Model};
use crate::util::{now_ms, read_json, short_hash, write_json};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const CHOICE_PREFIX: &str = "api:";
pub const NAME_MAX: usize = 30;
const RESERVED: [&str; 5] = ["codex", "opencode", "agy", "cursor", "kimi"];
pub const EFFORTS_FILE: &str = "brain-efforts.json";
pub const EFFORT_ORDER: [&str; 7] = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedBrain {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub base_url: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub key_env: Option<String>,
    #[serde(default)]
    pub added_at: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BrainInput {
    pub name: String,
    pub kind: String,
    pub base_url: String,
    pub api_key: String,
    pub key_env: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    pub name: String,
    pub kind: Kind,
    pub base_url: String,
    pub key_env: Option<String>,
    pub model: String,
}

pub fn choice_of(id: &str) -> String {
    format!("{CHOICE_PREFIX}{id}")
}

pub fn id_of_choice(choice: &str) -> Option<&str> {
    let id = choice.strip_prefix(CHOICE_PREFIX)?;
    (id.len() == 8 && id.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())).then_some(id)
}

pub fn is_local_url(url: &str) -> bool {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or("");
    let host_port = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = if host_port.starts_with('[') {
        host_port.split(']').next().map(|h| format!("{h}]")).unwrap_or_default()
    } else {
        host_port.split(':').next().unwrap_or("").to_string()
    };
    matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]")
}

pub fn check_input(id: Option<&str>, input: &BrainInput, saved: &[SavedBrain], has_key: bool) -> Result<Checked, String> {
    let before = match id {
        Some(id) => Some(saved.iter().find(|b| b.id == id).ok_or("This brain is not saved any more.")?),
        None => None,
    };
    let name = crate::util::flat(&input.name);
    if name.is_empty() {
        return Err("Give the brain a name.".into());
    }
    if name.chars().count() > NAME_MAX {
        return Err(format!("Keep the name under {NAME_MAX} letters."));
    }
    if RESERVED.contains(&name.to_lowercase().as_str()) {
        return Err(format!("{name} is already a built-in brain. Pick another name."));
    }
    if saved.iter().any(|b| Some(b.id.as_str()) != id && b.name.to_lowercase() == name.to_lowercase()) {
        return Err(format!("A brain named {name} is already saved."));
    }
    let kind = Kind::parse(input.kind.trim()).ok_or("Pick OpenAI, Anthropic or Ollama.")?;
    let typed = input.base_url.trim().trim_end_matches('/');
    let base_url = if typed.is_empty() { kind.default_url().to_string() } else { typed.to_string() };
    let (scheme, rest) = base_url.split_once("://").ok_or("The URL is not valid. It must start with https:// or http://.")?;
    if scheme != "https" && scheme != "http" {
        return Err("The URL must start with https:// or http://.".into());
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() {
        return Err("The URL is not valid. It must start with https:// or http://.".into());
    }
    if authority.contains('@') {
        return Err("Do not put a name or password in the URL. Use the key box.".into());
    }
    if scheme == "http" && !is_local_url(&base_url) {
        return Err("Use https:// for a server that is not on this Mac.".into());
    }
    let typed_key = input.api_key.trim();
    if typed_key.chars().any(char::is_whitespace) {
        return Err("The key has a space in it. Paste it again.".into());
    }
    if typed_key.is_empty() && !has_key && !is_local_url(&base_url) {
        return Err("Paste the API key, or set its env var.".into());
    }
    let key_env = Some(input.key_env.trim().to_string()).filter(|s| !s.is_empty());
    if let Some(name) = &key_env
        && !key_env_ok(name) {
            return Err(format!("The env var name uses capital letters, numbers and _ only, ends with {KEY_ENV_END} and has at most {KEY_ENV_MAX} letters."));
        }
    let key_env = key_env.or_else(|| before.and_then(|b| b.key_env.clone()));
    if before.is_some_and(|b| moved(b, kind, &base_url, key_env.as_deref())) && typed_key.is_empty() && (kind != Kind::Ollama || !is_local_url(&base_url)) {
        return Err("The server or the key env var changed. Paste the API key again.".into());
    }
    let same_server = before.is_some_and(|b| b.kind == kind && b.base_url == base_url);
    let model = if !input.model.trim().is_empty() {
        input.model.trim().to_string()
    } else if same_server {
        before.map(|b| b.model.clone()).unwrap_or_default()
    } else {
        String::new()
    };
    Ok(Checked { name, kind, base_url, key_env, model })
}

pub const KEY_ENV_END: &str = "_API_KEY";
pub const KEY_ENV_MAX: usize = 64;

pub fn key_env_ok(name: &str) -> bool {
    name.len() <= KEY_ENV_MAX
        && name.len() > KEY_ENV_END.len()
        && name.ends_with(KEY_ENV_END)
        && name.starts_with(|c: char| c.is_ascii_uppercase())
        && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn moved(before: &SavedBrain, kind: Kind, base_url: &str, key_env: Option<&str>) -> bool {
    before.kind != kind || before.base_url != base_url || before.key_env.as_deref() != key_env
}

pub fn server_moved(before: &SavedBrain, checked: &Checked) -> bool {
    moved(before, checked.kind, &checked.base_url, checked.key_env.as_deref())
}

pub struct BrainStore {
    pub file: PathBuf,
}

static NEXT: AtomicU64 = AtomicU64::new(0);

pub fn new_id() -> String {
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    short_hash(&format!("{}:{}:{n}", now_ms(), std::process::id()))[..8].to_string()
}

impl BrainStore {
    pub fn open(file: &Path) -> BrainStore {
        BrainStore { file: file.to_path_buf() }
    }

    pub fn list(&self) -> Vec<SavedBrain> {
        let rows: Vec<serde_json::Value> = read_json(&self.file).unwrap_or_default();
        let mut list: Vec<SavedBrain> = rows.into_iter().filter_map(|row| serde_json::from_value(row).ok()).collect();
        list.sort_by(|a, b| a.added_at.cmp(&b.added_at).then(a.id.cmp(&b.id)));
        list
    }

    pub fn get(&self, id: &str) -> Option<SavedBrain> {
        self.list().into_iter().find(|b| b.id == id)
    }

    fn save(&self, list: &[SavedBrain]) -> std::io::Result<()> {
        write_json(&self.file, &list)
    }

    pub fn add(&self, checked: Checked) -> std::io::Result<SavedBrain> {
        let mut list = self.list();
        let brain = SavedBrain {
            id: new_id(),
            name: checked.name,
            kind: checked.kind,
            base_url: checked.base_url,
            model: checked.model,
            effort: None,
            key_env: checked.key_env,
            added_at: now_ms(),
        };
        list.push(brain.clone());
        self.save(&list)?;
        Ok(brain)
    }

    pub fn update(&self, id: &str, change: impl FnOnce(&mut SavedBrain)) -> std::io::Result<Option<SavedBrain>> {
        let mut list = self.list();
        let Some(brain) = list.iter_mut().find(|b| b.id == id) else { return Ok(None) };
        change(brain);
        let out = brain.clone();
        self.save(&list)?;
        Ok(Some(out))
    }

    pub fn remove(&self, id: &str) -> std::io::Result<bool> {
        let mut list = self.list();
        let before = list.len();
        list.retain(|b| b.id != id);
        if list.len() == before {
            return Ok(false);
        }
        self.save(&list)?;
        Ok(true)
    }
}

pub fn highest_effort(efforts: &[String]) -> Option<String> {
    let known = efforts.iter().filter_map(|e| EFFORT_ORDER.iter().position(|o| o == e).map(|at| (at, e))).max_by_key(|(at, _)| *at);
    known.map(|(_, e)| e.clone()).or_else(|| efforts.last().cloned())
}

pub fn supported_effort(model: Option<&Model>, wanted: Option<&str>) -> Option<String> {
    let wanted = wanted.filter(|w| !w.is_empty())?;
    model.filter(|m| m.efforts.iter().any(|e| e == wanted)).map(|_| wanted.to_string())
}

pub struct EffortStore {
    pub file: PathBuf,
}

impl EffortStore {
    pub fn open(file: &Path) -> EffortStore {
        EffortStore { file: file.to_path_buf() }
    }

    pub fn all(&self) -> BTreeMap<String, BTreeMap<String, String>> {
        read_json(&self.file).unwrap_or_default()
    }

    pub fn requested(&self, brain: &str, model: &str, fallback: Option<&str>) -> Option<String> {
        if model.is_empty() {
            return None;
        }
        let saved = self.all().get(brain).and_then(|m| m.get(model)).cloned();
        match saved {
            Some(effort) => Some(effort).filter(|e| !e.is_empty()),
            None => fallback.filter(|e| !e.is_empty()).map(str::to_string),
        }
    }

    pub fn save(&self, brain: &str, model: &Model, wanted: Option<&str>) -> Result<(), String> {
        let wanted = wanted.filter(|w| !w.is_empty());
        if let Some(effort) = wanted
            && !model.efforts.iter().any(|e| e == effort) {
                return Err(format!("{} does not offer effort {effort}. Load its model list again.", model.name));
            }
        let mut all = self.all();
        all.entry(brain.to_string()).or_default().insert(model.id.clone(), wanted.unwrap_or("").to_string());
        write_json(&self.file, &all).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, kind: &str, url: &str, key: &str) -> BrainInput {
        BrainInput { name: name.into(), kind: kind.into(), base_url: url.into(), api_key: key.into(), ..BrainInput::default() }
    }

    #[test]
    fn list_skips_rows_it_cannot_read() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("brains.json");
        let good = r#"{"id":"0a0b0c0d","name":"A","kind":"openai","baseUrl":"https://example.com"}"#;
        std::fs::write(&file, format!(r#"[{good},{{"id":"0a0b0c0e","name":"B","kind":"future","baseUrl":"x"}}]"#)).unwrap();
        let list = BrainStore::open(&file).list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "A");
    }

    #[test]
    fn checks_input() {
        assert!(check_input(None, &input("", "openai", "", "k"), &[], false).is_err());
        assert!(check_input(None, &input("codex", "openai", "", "k"), &[], false).is_err());
        assert!(check_input(None, &input("A", "gemini", "", "k"), &[], false).is_err());
        assert!(check_input(None, &input("A", "openai", "http://example.com", "k"), &[], false).is_err());
        assert!(check_input(None, &input("A", "openai", "https://u:p@example.com", "k"), &[], false).is_err());
        assert!(check_input(None, &input("A", "openai", "https://example.com", ""), &[], false).is_err());
        let local = check_input(None, &input("Local", "openai", "http://localhost:11434/v1/", ""), &[], false).unwrap();
        assert_eq!(local.base_url, "http://localhost:11434/v1");
        let official = check_input(None, &input("Claude", "anthropic", "", "k"), &[], false).unwrap();
        assert_eq!(official.base_url, "https://api.anthropic.com");
    }

    #[test]
    fn checks_ollama_without_a_key() {
        let made = check_input(None, &input("Ollama", "ollama", "", ""), &[], false).unwrap();
        assert_eq!((made.kind, made.base_url.as_str()), (Kind::Ollama, "http://127.0.0.1:11434/v1"));
        assert!(check_input(None, &input("Far", "ollama", "http://example.com/v1", ""), &[], false).is_err());
        assert!(check_input(None, &input("Far", "ollama", "https://example.com/v1", ""), &[], false).is_err());
    }

    #[test]
    fn store_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = BrainStore::open(&dir.path().join("brains.json"));
        let checked = check_input(None, &input("Claude", "anthropic", "", "k"), &[], false).unwrap();
        let brain = store.add(checked).unwrap();
        assert_eq!(id_of_choice(&choice_of(&brain.id)), Some(brain.id.as_str()));
        assert!(check_input(None, &input("claude", "openai", "", "k"), &store.list(), false).is_err());
        store.update(&brain.id, |b| b.model = "m".into()).unwrap();
        assert_eq!(store.get(&brain.id).unwrap().model, "m");
        let text = std::fs::read_to_string(&store.file).unwrap();
        assert!(!text.contains("apiKey"));
        assert!(store.remove(&brain.id).unwrap());
        assert!(store.list().is_empty());
    }

    #[test]
    fn efforts_per_brain_and_model() {
        let dir = tempfile::tempdir().unwrap();
        let store = EffortStore::open(&dir.path().join(EFFORTS_FILE));
        let model = Model { id: "m1".into(), name: "M1".into(), description: String::new(), efforts: vec!["low".into(), "high".into()], max_tokens: None };
        assert_eq!(store.requested("codex", "m1", Some("low")), Some("low".into()));
        assert!(store.save("codex", &model, Some("max")).is_err());
        store.save("codex", &model, Some("high")).unwrap();
        assert_eq!(store.requested("codex", "m1", Some("low")), Some("high".into()));
        assert_eq!(store.requested("agy", "m1", None), None);
        store.save("codex", &model, None).unwrap();
        assert_eq!(store.requested("codex", "m1", Some("low")), None);
        assert_eq!(store.requested("codex", "", Some("low")), None);
        assert_eq!(supported_effort(Some(&model), Some("high")), Some("high".into()));
        assert_eq!(supported_effort(Some(&model), Some("max")), None);
        assert_eq!(supported_effort(None, Some("high")), None);
        assert_eq!(highest_effort(&["high".into(), "low".into(), "xhigh".into()]), Some("xhigh".into()));
        assert_eq!(highest_effort(&["odd".into()]), Some("odd".into()));
        assert_eq!(highest_effort(&[]), None);
    }

    #[test]
    fn local_hosts() {
        assert!(is_local_url("http://127.0.0.1:8080/v1"));
        assert!(is_local_url("http://[::1]:8080"));
        assert!(!is_local_url("http://localhost.evil.com"));
    }
}
