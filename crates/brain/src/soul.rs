use crate::app::{App, lock};
use crate::memory::quote_in;
use crate::tools::{Args, ToolResult, need, schema, spec, text};
use crate::turns::ToolSpec;
use crate::util::{atomic_write, short_hash};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

pub const FILE: &str = "rules.json";
pub const MAX_CHARS: usize = 64 * 1024;
pub const MAX_TITLE: usize = 200;
pub const DEFAULT_TITLE: &str = "Nixa chat rules";
pub const CONFLICT: &str = "The rules.json file changed since you read it. Call read_soul again, apply the user's change to the new JSON, then call update_soul again.";
pub const NAMES: [&str; 2] = ["read_soul", "update_soul"];
pub const PROMPT_HEAD: &str = "Standing rules (rules.json, written by the user; they always apply):";

static HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s{0,3}(#{1,6})\s+(.+?)\s*#*\s*$").unwrap());
static BULLET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*[-*+]\s+(.+)$").unwrap());

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub title: String,
    pub rules: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RulesDoc {
    pub version: u32,
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SoulText {
    pub text: String,
    pub hash: String,
}

pub struct Soul {
    pub file: PathBuf,
    last_good: Mutex<Option<SoulText>>,
}

pub fn tidy(text: &str) -> String {
    let lf = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut rest = lf.as_str();
    while let Some(at) = rest.find('\n') {
        if !rest[..at].trim_matches([' ', '\t']).is_empty() {
            break;
        }
        rest = &rest[at + 1..];
    }
    rest.trim_end().to_string()
}

pub fn hash(text: &str) -> String {
    short_hash(text)
}

fn rule_list(value: &Value, place: &str) -> Result<Vec<String>, String> {
    let Some(list) = value.as_array() else {
        return Err(format!("{place} must be an array of plain rule strings."));
    };
    let mut out = Vec::new();
    for (index, entry) in list.iter().enumerate() {
        match entry.as_str().map(str::trim) {
            Some(rule) if !rule.is_empty() => out.push(rule.to_string()),
            _ => return Err(format!("{place}[{index}] must be a non-empty string.")),
        }
    }
    Ok(out)
}

fn short_title(value: &str) -> bool {
    !value.is_empty() && value.chars().count() <= MAX_TITLE
}

pub fn check_doc(raw: &Value) -> Result<RulesDoc, String> {
    let Some(map) = raw.as_object() else {
        return Err("rules.json must be a JSON object with rules or sections.".into());
    };
    if let Some(version) = map.get("version")
        && version.as_u64() != Some(1)
    {
        return Err("rules.json version must be 1.".into());
    }
    let title = match map.get("title") {
        None => DEFAULT_TITLE.to_string(),
        Some(value) => match value.as_str().map(str::trim) {
            Some(title) if short_title(title) => title.to_string(),
            _ => {
                return Err(format!(
                    "rules.json title must be a non-empty string up to {MAX_TITLE} characters."
                ));
            }
        },
    };
    let rules = match map.get("rules") {
        None => Vec::new(),
        Some(value) => rule_list(value, "rules")?,
    };
    let mut sections = Vec::new();
    if let Some(value) = map.get("sections") {
        let Some(list) = value.as_array() else {
            return Err("sections must be an array of { title, rules } objects.".into());
        };
        for (index, item) in list.iter().enumerate() {
            let Some(item) = item.as_object() else {
                return Err(format!(
                    "sections[{index}] must be an object with title and rules."
                ));
            };
            let heading = item
                .get("title")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            if !short_title(heading) {
                return Err(format!(
                    "sections[{index}].title must be a non-empty string up to {MAX_TITLE} characters."
                ));
            }
            let list = rule_list(
                item.get("rules").unwrap_or(&Value::Null),
                &format!("sections[{index}].rules"),
            )?;
            if list.is_empty() {
                return Err(format!(
                    "sections[{index}].rules must contain at least one rule."
                ));
            }
            sections.push(Section {
                title: heading.to_string(),
                rules: list,
            });
        }
    }
    if rules.is_empty() && sections.iter().all(|s| s.rules.is_empty()) {
        return Err("rules.json must contain at least one rule in rules or sections.".into());
    }
    Ok(RulesDoc {
        version: 1,
        title,
        rules,
        sections,
    })
}

pub fn parse_rules(text: &str) -> Result<RulesDoc, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| format!("rules.json is not valid JSON: {e}"))?;
    check_doc(&value)
}

pub fn format_doc(doc: &RulesDoc) -> Result<String, String> {
    let value = serde_json::to_value(doc).map_err(|e| e.to_string())?;
    let checked = check_doc(&value)?;
    serde_json::to_string_pretty(&checked).map_err(|e| e.to_string())
}

pub fn markdown_to_doc(markdown: &str, default_title: &str) -> RulesDoc {
    let mut title = default_title.to_string();
    let mut title_set = false;
    let mut top: Vec<String> = Vec::new();
    let mut sections: Vec<Section> = Vec::new();
    for raw in tidy(markdown).lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(found) = HEADING.captures(line) {
            let level = found[1].len();
            let name = found[2].trim().to_string();
            if level == 1 && !title_set && sections.is_empty() && top.is_empty() {
                title = name;
                title_set = true;
                continue;
            }
            if sections.last().is_some_and(|s| s.rules.is_empty()) {
                sections.pop();
            }
            sections.push(Section {
                title: name,
                rules: Vec::new(),
            });
            continue;
        }
        let entry = BULLET
            .captures(line)
            .map(|c| c[1].trim().to_string())
            .unwrap_or_else(|| line.to_string());
        if entry.is_empty() {
            continue;
        }
        match sections.last_mut() {
            Some(section) => section.rules.push(entry),
            None => top.push(entry),
        }
    }
    sections.retain(|s| !s.rules.is_empty());
    if sections.is_empty() && top.is_empty() {
        top.push("The user edits this rules.json file.".into());
    }
    RulesDoc {
        version: 1,
        title,
        rules: top,
        sections,
    }
}

pub fn write_private(file: &Path, text: &str) -> std::io::Result<()> {
    if file.exists() {
        let backup = backup_of(file);
        if std::fs::copy(file, &backup).is_ok() {
            std::fs::set_permissions(&backup, std::fs::Permissions::from_mode(0o600)).ok();
        }
    }
    atomic_write(file, text)
}

pub fn backup_of(file: &Path) -> PathBuf {
    let mut name = file
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".bak");
    file.with_file_name(name)
}

fn valid(raw: &str) -> Option<SoulText> {
    let clean = tidy(raw);
    if clean.is_empty() || parse_rules(&clean).is_err() {
        return None;
    }
    Some(SoulText {
        hash: hash(&clean),
        text: clean,
    })
}

impl Soul {
    pub fn open(dir: &Path) -> Soul {
        let file = dir.join(FILE);
        let seed = valid(&std::fs::read_to_string(&file).unwrap_or_default()).or_else(|| {
            std::fs::read_to_string(backup_of(&file))
                .ok()
                .and_then(|raw| valid(&raw))
        });
        Soul {
            file,
            last_good: Mutex::new(seed),
        }
    }

    fn raw(&self) -> String {
        tidy(&std::fs::read_to_string(&self.file).unwrap_or_default())
    }

    pub fn last_good(&self) -> Option<SoulText> {
        lock(&self.last_good).clone()
    }

    pub fn read(&self) -> SoulText {
        let text = self.raw();
        if let Some(good) = valid(&text) {
            *lock(&self.last_good) = Some(good);
        }
        SoulText {
            hash: hash(&text),
            text,
        }
    }

    pub fn check(&self) -> Result<SoulText, String> {
        let raw = self.raw();
        if raw.is_empty() {
            return Err("rules.json is empty. Kept the last good rules.".into());
        }
        match parse_rules(&raw) {
            Ok(_) => {
                let good = SoulText {
                    hash: hash(&raw),
                    text: raw,
                };
                *lock(&self.last_good) = Some(good.clone());
                Ok(good)
            }
            Err(error) => Err(format!("rules.json kept the last good rules: {error}")),
        }
    }

    pub fn update(&self, text: &str, expected: &str) -> Result<String, String> {
        let raw = tidy(text);
        if raw.is_empty() {
            return Err("The new rules JSON is empty. Nothing was saved.".into());
        }
        let too_long =
            format!("The rules are longer than {MAX_CHARS} characters. Nothing was saved.");
        if raw.len() > MAX_CHARS {
            return Err(too_long);
        }
        let next = format_doc(&parse_rules(&raw)?)?;
        if next.len() > MAX_CHARS {
            return Err(too_long);
        }
        if self.read().hash != expected {
            return Err(CONFLICT.into());
        }
        write_private(&self.file, &format!("{next}\n"))
            .map_err(|e| format!("rules.json was not saved: {e}"))?;
        let saved = SoulText {
            hash: hash(&next),
            text: next,
        };
        let out = saved.hash.clone();
        *lock(&self.last_good) = Some(saved);
        Ok(out)
    }

    pub fn for_prompt(&self) -> String {
        match self.check() {
            Ok(good) => good.text,
            Err(_) => self.last_good().map(|g| g.text).unwrap_or_default(),
        }
    }

    pub fn prompt_hash(&self) -> String {
        match self.check() {
            Ok(good) => good.hash,
            Err(_) => self.last_good().map(|g| g.hash).unwrap_or_else(|| hash("")),
        }
    }

    pub fn problem(&self) -> Option<String> {
        if self.file.exists() {
            self.check().err()
        } else {
            None
        }
    }

    pub fn prompt(&self) -> String {
        let text = self.for_prompt();
        if text.is_empty() {
            String::new()
        } else {
            format!("{PROMPT_HEAD}\n{text}")
        }
    }
}

pub fn specs(name: &str) -> Vec<ToolSpec> {
    vec![
        spec(
            "read_soul",
            &format!(
                "Read {name}'s standing instructions (rules.json) with its hash. Call it right before update_soul."
            ),
            schema(json!({}), &[]),
        ),
        spec(
            "update_soul",
            &format!(
                "Save {name}'s standing instructions (rules.json, plain JSON). Memory notes stay unchanged. You may change it yourself (for example the Weekly review lines) without a quote; it goes into the decision journal. When the user asked, put their own words in quote. Call read_soul first, keep every other rule, then send the complete JSON text, the hash and what changes."
            ),
            schema(
                json!({
                    "text": {"type": "string", "description": "The complete new rules.json text (valid JSON)"},
                    "expectedHash": {"type": "string", "description": "The hash from read_soul"},
                    "change": {"type": "string", "description": "What changes, in a few plain words"},
                    "quote": {"type": "string", "description": "The user's own words that asked for this change, when the user asked."}
                }),
                &["text", "expectedHash", "change"],
            ),
        ),
    ]
}

pub fn read_text(soul: &Soul) -> String {
    let current = soul.read();
    if current.text.is_empty() {
        format!("The rules are empty. hash: {}", current.hash)
    } else {
        format!("hash: {}\n\n{}", current.hash, current.text)
    }
}

pub fn call(app: &Arc<App>, name: &str, args: &Args) -> Option<ToolResult> {
    let soul = Soul::open(&app.dir);
    match name {
        "read_soul" => Some(Ok(read_text(&soul))),
        "update_soul" => Some(update_tool(app, &soul, args)),
        _ => None,
    }
}

fn update_tool(app: &Arc<App>, soul: &Soul, args: &Args) -> ToolResult {
    let quote = text(args, "quote").unwrap_or_default();
    let asked = quote_in(&quote, &lock(&app.chat).user_lines(12));
    let body = args.get("text").and_then(Value::as_str).unwrap_or_default();
    let expected = need(args, "expectedHash")?;
    let change = need(args, "change")?;
    if body.trim().is_empty() {
        return Err(
            "Send the complete new rules JSON, the hash from read_soul, and what changes.".into(),
        );
    }
    let hash = soul.update(body, &expected)?;
    app.send_out(json!({"type": "content_changed", "document": "rules", "hash": hash}));
    app.info(&format!(
        "Standing instructions changed: {}",
        crate::util::clip_flat(&change, 160)
    ));
    if !asked {
        let what = format!("changed standing instructions: {}", crate::util::clip_flat(&change, 160));
        crate::modes::note(app, "auto-yes", &what, None, Some("Nixa changes its standing instructions by itself"));
    }
    Ok("Saved. These rules replace the earlier ones from now on.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nixa-soul-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn tidy_drops_blank_head_and_tail() {
        assert_eq!(
            tidy("\r\n  \n\tline one\r\nline two  \n\n"),
            "\tline one\nline two"
        );
        assert_eq!(tidy("   "), "");
        assert_eq!(tidy("a\rb"), "a\nb");
    }

    #[test]
    fn checks_the_rules_shape() {
        let doc = parse_rules(
            r#"{"rules": [" be short "], "sections": [{"title": "Git", "rules": ["no push"]}]}"#,
        )
        .unwrap();
        assert_eq!(doc.title, DEFAULT_TITLE);
        assert_eq!(doc.rules, vec!["be short"]);
        assert_eq!(doc.sections[0].rules, vec!["no push"]);
        assert!(parse_rules("[1]").unwrap_err().contains("JSON object"));
        assert!(
            parse_rules(r#"{"version": 2, "rules": ["a"]}"#)
                .unwrap_err()
                .contains("version must be 1")
        );
        assert!(
            parse_rules(r#"{"rules": []}"#)
                .unwrap_err()
                .contains("at least one rule")
        );
        assert!(
            parse_rules(r#"{"rules": [""]}"#)
                .unwrap_err()
                .contains("rules[0]")
        );
        assert!(
            parse_rules(r#"{"title": "", "rules": ["a"]}"#)
                .unwrap_err()
                .contains("title")
        );
        assert!(
            parse_rules(r#"{"sections": [{"title": "x", "rules": []}]}"#)
                .unwrap_err()
                .contains("sections[0].rules")
        );
        assert!(
            parse_rules("{nope")
                .unwrap_err()
                .starts_with("rules.json is not valid JSON")
        );
        let text = format_doc(&doc).unwrap();
        assert!(text.starts_with("{\n  \"version\": 1,\n  \"title\""));
        assert!(
            !format_doc(&RulesDoc {
                version: 1,
                title: "t".into(),
                rules: vec![],
                sections: vec![]
            })
            .is_ok()
        );
    }

    #[test]
    fn markdown_becomes_rules() {
        let doc = markdown_to_doc(
            "# My rules\n\n- top one\nplain line\n## Empty\n## Git\n* no push\n+ ask first\n",
            DEFAULT_TITLE,
        );
        assert_eq!(doc.title, "My rules");
        assert_eq!(doc.rules, vec!["top one", "plain line"]);
        assert_eq!(doc.sections.len(), 1);
        assert_eq!(doc.sections[0].title, "Git");
        assert_eq!(doc.sections[0].rules, vec!["no push", "ask first"]);
        let empty = markdown_to_doc("\n\n", "T");
        assert_eq!(empty.rules, vec!["The user edits this rules.json file."]);
        assert!(format_doc(&doc).is_ok());
    }

    #[test]
    fn update_needs_the_hash_and_keeps_a_private_backup() {
        let dir = temp("update");
        let soul = Soul::open(&dir);
        let first = soul.read();
        assert_eq!(first.text, "");
        assert!(soul.problem().is_none());
        assert!(soul.update("", &first.hash).unwrap_err().contains("empty"));
        assert_eq!(
            soul.update(r#"{"rules": ["a"]}"#, "wrong").unwrap_err(),
            CONFLICT
        );
        let hash = soul.update(r#"{"rules": ["a"]}"#, &first.hash).unwrap();
        let now = soul.read();
        assert_eq!(now.hash, hash);
        assert!(now.text.contains("\"version\": 1"));
        assert_eq!(
            std::fs::read_to_string(dir.join(FILE)).unwrap(),
            format!("{}\n", now.text)
        );
        let second = soul.update(r#"{"rules": ["b"]}"#, &hash).unwrap();
        assert_ne!(second, hash);
        let backup = backup_of(&dir.join(FILE));
        assert!(std::fs::read_to_string(&backup).unwrap().contains("\"a\""));
        let mode = std::fs::metadata(&backup).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let mode = std::fs::metadata(dir.join(FILE))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        assert!(
            soul.update(&"x".repeat(MAX_CHARS + 1), &second)
                .unwrap_err()
                .contains("longer")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_broken_file_keeps_the_last_good_rules() {
        let dir = temp("broken");
        std::fs::write(dir.join(FILE), r#"{"rules": ["keep me"]}"#).unwrap();
        let soul = Soul::open(&dir);
        assert!(soul.for_prompt().contains("keep me"));
        let good_hash = soul.prompt_hash();
        std::fs::write(dir.join(FILE), "{broken").unwrap();
        assert!(
            soul.problem()
                .unwrap()
                .starts_with("rules.json kept the last good rules")
        );
        assert!(soul.for_prompt().contains("keep me"));
        assert_eq!(soul.prompt_hash(), good_hash);
        assert!(soul.prompt().starts_with(PROMPT_HEAD));
        assert_eq!(soul.read().text, "{broken");
        let fresh = Soul::open(&dir);
        assert!(fresh.last_good().is_none());
        std::fs::write(backup_of(&dir.join(FILE)), r#"{"rules": ["from backup"]}"#).unwrap();
        let from_backup = Soul::open(&dir);
        assert!(from_backup.for_prompt().contains("from backup"));
        std::fs::write(dir.join(FILE), "").unwrap();
        assert!(from_backup.problem().unwrap().contains("empty"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_text_shows_hash() {
        let dir = temp("readtext");
        let soul = Soul::open(&dir);
        assert!(read_text(&soul).starts_with("The rules are empty. hash: "));
        std::fs::write(dir.join(FILE), r#"{"rules": ["a"]}"#).unwrap();
        let shown = read_text(&soul);
        assert!(shown.starts_with("hash: ") && shown.ends_with(r#"{"rules": ["a"]}"#));
        assert_eq!(
            specs("Nixa")
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            NAMES.to_vec()
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn update_works_without_a_quote() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let soul = Soul::open(&app.dir);
        let args = json!({"text": r#"{"rules": ["be short"]}"#, "expectedHash": soul.read().hash, "change": "add be short"});
        let out = call(&app, "update_soul", args.as_object().unwrap()).unwrap().unwrap();
        assert!(out.starts_with("Saved."), "{out}");
        let conn = crate::modes::store(&app).unwrap();
        let rows = crate::modes::journal::last_for(&conn, None, 20).unwrap();
        assert!(rows.iter().any(|row| row.choice.contains("changed standing instructions")), "{rows:?}");
    }
}
