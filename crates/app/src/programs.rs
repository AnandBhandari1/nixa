use nixa_store::agents::CHECKS_ON_KEY;
use serde_json::{Map, Value, json};
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub const FILE: &str = "programs.json";

pub const ICONS: [&str; 13] = [
    "agent",
    "claude",
    "codex",
    "cursor",
    "gemini",
    "pi",
    "antigravity",
    "opencode",
    "omo",
    "crush",
    "jcode",
    "hermes",
    "dim",
];

pub const BUILT_IN: &[(&str, &str, &str, &[&str])] = &[
    ("claude", "Claude Code", "claude", &["claude"]),
    ("claude-continue", "Claude (continue)", "claude", &["claude", "--continue"]),
    ("codex", "Codex", "codex", &["codex"]),
    ("codex-continue", "Codex (continue)", "codex", &["codex", "resume", "--last"]),
    ("cursor", "Cursor", "cursor", &["cursor-agent"]),
    ("gemini", "Gemini CLI", "antigravity", &["gemini"]),
    ("opencode", "OpenCode", "opencode", &["opencode"]),
    ("omp", "Oh My Pi", "pi", &["omp"]),
    ("antigravity", "Antigravity", "antigravity", &["agy"]),
    ("crush", "Crush", "crush", &["crush"]),
    ("omo", "OmO", "omo", &["omo"]),
    ("jcode", "Jcode", "jcode", &["jcode"]),
    ("dim", "DimCode", "dim", &["dima"]),
];

use nixa_protocol::paths::{HOME_TOOL_DIRS as EXTRA_DIRS, SYSTEM_TOOL_DIRS as SYSTEM_DIRS};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub key: String,
    pub name: String,
    pub icon: Option<String>,
    pub run: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fields {
    pub name: Option<String>,
    pub icon: Option<String>,
    pub run: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub key: String,
    pub text: String,
}

pub fn path_in(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

pub fn default_path() -> PathBuf {
    path_in(&nixa_protocol::paths::data_dir())
}

pub fn icon_path(name: &str) -> String {
    format!("icons/{name}.svg")
}

pub fn icon_name(path: &str) -> String {
    let base = path.rsplit('/').next().unwrap_or(path);
    base.strip_suffix(".svg").unwrap_or(base).to_string()
}

pub fn key_of(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
        .to_lowercase()
}

pub fn split_command(line: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut has = false;
    let mut quote: Option<char> = None;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        match quote {
            Some('\'') => {
                if ch == '\'' {
                    quote = None;
                } else {
                    cur.push(ch);
                }
            }
            Some(_) => {
                if ch == '"' {
                    quote = None;
                } else if ch == '\\'
                    && i + 1 < chars.len()
                    && matches!(chars[i + 1], '"' | '\\' | '$' | '\x60')
                {
                    i += 1;
                    cur.push(chars[i]);
                } else {
                    cur.push(ch);
                }
            }
            None => {
                if ch == '\'' || ch == '"' {
                    quote = Some(ch);
                    has = true;
                } else if ch == '\\' && i + 1 < chars.len() {
                    i += 1;
                    cur.push(chars[i]);
                    has = true;
                } else if ch.is_whitespace() {
                    if has {
                        out.push(std::mem::take(&mut cur));
                    }
                    cur.clear();
                    has = false;
                } else {
                    cur.push(ch);
                    has = true;
                }
            }
        }
        i += 1;
    }
    if let Some(q) = quote {
        let which = if q == '\'' { "single" } else { "double" };
        return Err(format!("A {which} quote is not closed."));
    }
    if has {
        out.push(cur);
    }
    Ok(out)
}

fn plain_arg(arg: &str) -> bool {
    !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./~^-".contains(c))
}

pub fn join_command(run: &[String]) -> String {
    run.iter()
        .map(|arg| {
            if plain_arg(arg) {
                arg.clone()
            } else {
                format!("'{}'", arg.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse(text: Option<&str>) -> Result<Map<String, Value>, String> {
    let Some(text) = text.filter(|t| !t.trim().is_empty()) else {
        let mut data = Map::new();
        data.insert("programs".into(), json!({}));
        return Ok(data);
    };
    let value: Value =
        serde_json::from_str(text).map_err(|e| format!("programs.json is not valid JSON: {e}"))?;
    let Value::Object(mut data) = value else {
        return Err("programs.json is not a JSON object.".into());
    };
    match data.get("programs") {
        None => {
            data.insert("programs".into(), json!({}));
        }
        Some(Value::Object(_)) => {}
        Some(_) => return Err("\"programs\" in programs.json is not an object.".into()),
    }
    Ok(data)
}

fn programs_of(data: &mut Map<String, Value>) -> &mut Map<String, Value> {
    if !matches!(data.get("programs"), Some(Value::Object(_))) {
        data.insert("programs".into(), json!({}));
    }
    match data.get_mut("programs") {
        Some(Value::Object(map)) => map,
        _ => unreachable!(),
    }
}

fn print(data: Map<String, Value>, old: Option<&str>) -> String {
    let mut text = serde_json::to_string_pretty(&Value::Object(data)).unwrap_or_default();
    if old.is_some_and(|o| o.ends_with('\n')) {
        text.push('\n');
    }
    text
}

pub fn user_keys(text: Option<&str>) -> Vec<String> {
    match parse(text) {
        Ok(mut data) => programs_of(&mut data).keys().cloned().collect(),
        Err(_) => Vec::new(),
    }
}

pub fn add_entry(
    text: Option<&str>,
    name: &str,
    icon: Option<&str>,
    run: &[String],
    taken: &[String],
) -> Result<Change, String> {
    let mut data = parse(text)?;
    let key = key_of(name);
    if key.is_empty() {
        return Err("Type a name.".into());
    }
    if key == "__proto__" {
        return Err("Pick another name.".into());
    }
    if run.is_empty() {
        return Err("Type the command.".into());
    }
    let programs = programs_of(&mut data);
    if programs.contains_key(&key) || taken.iter().any(|t| t == &key) {
        return Err(format!(
            "\"{key}\" is already in the list. Pick another name."
        ));
    }
    programs.insert(
        key.clone(),
        json!({"name": name.trim(), "icon": icon, "run": run}),
    );
    Ok(Change {
        key,
        text: print(data, text),
    })
}

pub fn edit_entry(
    text: Option<&str>,
    key: &str,
    values: &Fields,
    shown: &Fields,
) -> Result<Change, String> {
    let mut data = parse(text)?;
    if values.run.as_ref().is_none_or(|r| r.is_empty()) {
        return Err("Type the command.".into());
    }
    let programs = programs_of(&mut data);
    let Some(old) = programs.get(key) else {
        return Err(format!("\"{key}\" is no longer in programs.json."));
    };
    let mut entry = old.as_object().cloned().unwrap_or_default();
    let pairs = [
        (
            "name",
            values.name.as_ref().map(|v| json!(v)),
            shown.name.as_ref().map(|v| json!(v)),
        ),
        (
            "icon",
            values.icon.as_ref().map(|v| json!(v)),
            shown.icon.as_ref().map(|v| json!(v)),
        ),
        (
            "run",
            values.run.as_ref().map(|v| json!(v)),
            shown.run.as_ref().map(|v| json!(v)),
        ),
    ];
    for (field, value, seen) in pairs {
        let Some(value) = value else { continue };
        if entry.contains_key(field) || Some(&value) != seen.as_ref() {
            entry.insert(field.into(), value);
        }
    }
    programs.insert(key.into(), Value::Object(entry));
    Ok(Change {
        key: key.into(),
        text: print(data, text),
    })
}

pub fn remove_entry(text: Option<&str>, key: &str) -> Result<Option<Change>, String> {
    let mut data = parse(text)?;
    if programs_of(&mut data).remove(key).is_none() {
        return Ok(None);
    }
    Ok(Some(Change {
        key: key.into(),
        text: print(data, text),
    }))
}

fn words(value: Option<&Value>) -> Option<Vec<String>> {
    let list: Vec<String> = value?
        .as_array()?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    (!list.is_empty()).then_some(list)
}

pub fn known(text: Option<&str>) -> Vec<Program> {
    let mut list: Vec<Program> = BUILT_IN
        .iter()
        .map(|(key, name, icon, run)| Program {
            key: key.to_string(),
            name: name.to_string(),
            icon: Some(icon_path(icon)),
            run: run.iter().map(|s| s.to_string()).collect(),
        })
        .collect();
    let Ok(mut data) = parse(text) else {
        return list;
    };
    for (key, entry) in programs_of(&mut data).iter() {
        let name = entry
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string);
        let icon = entry.get("icon");
        let run = words(entry.get("run"));
        if let Some(found) = list.iter_mut().find(|p| &p.key == key) {
            if let Some(name) = name.filter(|n| !n.trim().is_empty()) {
                found.name = name;
            }
            if let Some(icon) = icon {
                found.icon = icon.as_str().map(str::to_string);
            }
            if let Some(run) = run {
                found.run = run;
            }
            continue;
        }
        let Some(run) = run else { continue };
        list.push(Program {
            key: key.clone(),
            name: name
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| key.clone()),
            icon: icon.and_then(Value::as_str).map(str::to_string),
            run,
        });
    }
    list
}

#[cfg(test)]
pub fn normal_checks(text: Option<&str>) -> Vec<String> {
    let Ok(data) = parse(text) else {
        return Vec::new();
    };
    words(data.get(CHECKS_ON_KEY)).unwrap_or_default()
}

pub fn set_normal_checks(text: Option<&str>, key: &str, on: bool) -> Result<Change, String> {
    let mut data = parse(text)?;
    let mut list = words(data.get(CHECKS_ON_KEY)).unwrap_or_default();
    list.retain(|k| k != key);
    if on {
        list.push(key.to_string());
    }
    if list.is_empty() {
        data.remove(CHECKS_ON_KEY);
    } else {
        data.insert(CHECKS_ON_KEY.into(), json!(list));
    }
    Ok(Change {
        key: key.into(),
        text: print(data, text),
    })
}

pub fn search_dirs(path_env: Option<&str>, home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = path_env
        .unwrap_or_default()
        .split(':')
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .collect();
    if let Some(home) = home {
        dirs.extend(EXTRA_DIRS.iter().map(|d| home.join(d)));
        dirs.extend(nixa_protocol::paths::dima_node_bin(home));
    }
    dirs.extend(SYSTEM_DIRS.iter().map(PathBuf::from));
    let mut seen = Vec::new();
    dirs.retain(|d| {
        let fresh = !seen.contains(d);
        if fresh {
            seen.push(d.clone());
        }
        fresh
    });
    dirs
}

pub fn default_dirs() -> Vec<PathBuf> {
    let path = std::env::var("PATH").ok();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    search_dirs(path.as_deref(), home.as_deref())
}

pub fn program_word(run: &[String]) -> Option<&str> {
    let mut words = run.iter().map(String::as_str);
    let first = words.next()?;
    if first != "env" {
        return Some(first);
    }
    words.find(|w| !w.contains('=') && !w.starts_with('-'))
}

pub(crate) fn runnable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

pub fn installed(run: &[String], dirs: &[PathBuf]) -> bool {
    let Some(word) = program_word(run) else {
        return false;
    };
    if word.contains('/') {
        return runnable(Path::new(word));
    }
    dirs.iter().any(|d| runnable(&d.join(word)))
}

pub fn read(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn write_if_same(path: &Path, text: &str, old: Option<&str>) -> Result<(), String> {
    let now = read(path).map_err(|e| format!("Cannot read programs.json: {e}"))?;
    if now.as_deref() != old {
        return Err("programs.json was changed by something else. Try again.".into());
    }
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)
        .map_err(|e| format!("Cannot make the folder for programs.json: {e}"))?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp = dir.join(format!(".{FILE}.{}.{nanos}.tmp", std::process::id()));
    fs::write(&temp, text).map_err(|e| format!("Cannot write programs.json: {e}"))?;
    fs::rename(&temp, path).map_err(|e| {
        let _ = fs::remove_file(&temp);
        format!("Cannot save programs.json: {e}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn scratch(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "nixa-programs-{name}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn split_follows_shell_quotes() {
        assert_eq!(
            split_command("omp --model zai/glm-5.3").unwrap(),
            words(&["omp", "--model", "zai/glm-5.3"])
        );
        assert_eq!(
            split_command(r#"a 'b c' "d \"e\" \$f" g\ h ''"#).unwrap(),
            words(&["a", "b c", "d \"e\" $f", "g h", ""])
        );
        assert_eq!(split_command("   ").unwrap(), Vec::<String>::new());
        assert_eq!(
            split_command("a 'b").unwrap_err(),
            "A single quote is not closed."
        );
        assert_eq!(
            split_command("a \"b").unwrap_err(),
            "A double quote is not closed."
        );
        assert_eq!(split_command(r#""a\nb""#).unwrap(), words(&["a\\nb"]));
    }

    #[test]
    fn join_quotes_only_when_needed_and_round_trips() {
        let run = words(&["env", "A=1", "my agent", "it's", "", "--x=a/b"]);
        let line = join_command(&run);
        assert_eq!(line, "env A=1 'my agent' 'it'\\''s' '' --x=a/b");
        assert_eq!(split_command(&line).unwrap(), run);
    }

    #[test]
    fn keys_and_icons() {
        assert_eq!(key_of("  My   Agent "), "my-agent");
        assert_eq!(icon_path("claude"), "icons/claude.svg");
        assert_eq!(icon_name("icons/claude.svg"), "claude");
        assert_eq!(icon_name("pi"), "pi");
    }

    #[test]
    fn add_edit_remove() {
        let added = add_entry(
            None,
            "My Agent",
            Some("icons/pi.svg"),
            &words(&["omp"]),
            &[],
        )
        .unwrap();
        assert_eq!(added.key, "my-agent");
        assert_eq!(user_keys(Some(&added.text)), words(&["my-agent"]));
        let again = add_entry(Some(&added.text), "my agent", None, &words(&["x"]), &[]);
        assert_eq!(
            again.unwrap_err(),
            "\"my-agent\" is already in the list. Pick another name."
        );
        let taken = add_entry(None, "Codex", None, &words(&["x"]), &words(&["codex"]));
        assert!(taken.is_err());
        assert_eq!(
            add_entry(None, "  ", None, &words(&["x"]), &[]).unwrap_err(),
            "Type a name."
        );
        assert_eq!(
            add_entry(None, "a", None, &[], &[]).unwrap_err(),
            "Type the command."
        );

        let shown = Fields {
            name: Some("My Agent".into()),
            icon: Some("icons/pi.svg".into()),
            run: Some(words(&["omp"])),
        };
        let values = Fields {
            name: Some("Better".into()),
            icon: Some("icons/pi.svg".into()),
            run: Some(words(&["omp", "--fast"])),
        };
        let edited = edit_entry(Some(&added.text), "my-agent", &values, &shown).unwrap();
        let list = known(Some(&edited.text));
        let mine = list.iter().find(|p| p.key == "my-agent").unwrap();
        assert_eq!(mine.name, "Better");
        assert_eq!(mine.run, words(&["omp", "--fast"]));
        assert_eq!(mine.icon.as_deref(), Some("icons/pi.svg"));
        assert!(edit_entry(Some(&edited.text), "gone", &values, &shown).is_err());

        let removed = remove_entry(Some(&edited.text), "my-agent")
            .unwrap()
            .unwrap();
        assert!(user_keys(Some(&removed.text)).is_empty());
        assert_eq!(remove_entry(Some(&removed.text), "my-agent").unwrap(), None);
    }

    #[test]
    fn edit_keeps_built_in_values_out_of_an_override() {
        let text = r#"{"programs": {"codex": {"run": ["codex", "--search"]}}}"#;
        let shown = Fields {
            name: Some("Codex".into()),
            icon: Some("icons/codex.svg".into()),
            run: Some(words(&["codex", "--search"])),
        };
        let values = Fields {
            run: Some(words(&["codex"])),
            ..shown.clone()
        };
        let edited = edit_entry(Some(text), "codex", &values, &shown).unwrap();
        let data: Value = serde_json::from_str(&edited.text).unwrap();
        assert_eq!(data["programs"]["codex"], json!({"run": ["codex"]}));
    }

    #[test]
    fn checks_on_is_saved_separately_from_legacy_checks_off() {
        assert!(normal_checks(None).is_empty());
        let on = set_normal_checks(None, "claude", true).unwrap();
        assert_eq!(normal_checks(Some(&on.text)), words(&["claude"]));
        let two = set_normal_checks(Some(&on.text), "codex", true).unwrap();
        let again = set_normal_checks(Some(&two.text), "claude", true).unwrap();
        assert_eq!(normal_checks(Some(&again.text)), words(&["codex", "claude"]));
        let off = set_normal_checks(Some(&again.text), "codex", false).unwrap();
        assert_eq!(normal_checks(Some(&off.text)), words(&["claude"]));
        let none = set_normal_checks(Some(&off.text), "claude", false).unwrap();
        let data: Value = serde_json::from_str(&none.text).unwrap();
        assert!(data.get(CHECKS_ON_KEY).is_none());
        let legacy = r#"{"programs":{"mine":{"run":["m"]}},"checksOff":["claude","codex"]}"#;
        assert!(normal_checks(Some(legacy)).is_empty());
        let kept = set_normal_checks(Some(legacy), "claude", true).unwrap();
        assert_eq!(normal_checks(Some(&kept.text)), words(&["claude"]));
        let data: Value = serde_json::from_str(&kept.text).unwrap();
        assert_eq!(data["checksOff"], json!(["claude", "codex"]));
        let cleared = set_normal_checks(Some(&kept.text), "claude", false).unwrap();
        let data: Value = serde_json::from_str(&cleared.text).unwrap();
        assert_eq!(data, serde_json::from_str::<Value>(legacy).unwrap());
        assert!(set_normal_checks(Some("{nope"), "claude", true).is_err());
        assert!(normal_checks(Some("{nope")).is_empty());
    }

    #[test]
    fn bad_files_are_reported() {
        assert!(add_entry(Some("[1]"), "a", None, &words(&["a"]), &[]).is_err());
        assert!(add_entry(Some("{\"programs\": 3}"), "a", None, &words(&["a"]), &[]).is_err());
        assert!(add_entry(Some("{nope"), "a", None, &words(&["a"]), &[]).is_err());
        assert!(user_keys(Some("{nope")).is_empty());
        let kept = add_entry(Some("{\"other\": 1}\n"), "a", None, &words(&["a"]), &[]).unwrap();
        assert!(kept.text.ends_with('\n'));
        assert!(kept.text.contains("\"other\": 1"));
    }

    #[test]
    fn known_merges_the_file_over_the_built_in_list() {
        let text = r#"{"programs": {
            "claude": {"name": "Claude", "run": ["claude", "--fast"]},
            "mine": {"name": "Mine", "icon": null, "run": ["mine"]},
            "broken": {"name": "No run"}
        }}"#;
        let list = known(Some(text));
        let claude = list.iter().find(|p| p.key == "claude").unwrap();
        assert_eq!(claude.name, "Claude");
        assert_eq!(claude.run, words(&["claude", "--fast"]));
        assert_eq!(claude.icon.as_deref(), Some("icons/claude.svg"));
        let mine = list.iter().find(|p| p.key == "mine").unwrap();
        assert_eq!(mine.icon, None);
        assert!(!list.iter().any(|p| p.key == "broken"));
        assert_eq!(known(None).len(), BUILT_IN.len());
    }

    #[test]
    fn installed_looks_in_the_folders() {
        let dir = scratch("bin");
        let bin = dir.join("fake-agent");
        fs::write(&bin, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        let plain = dir.join("not-run");
        fs::write(&plain, "x").unwrap();
        let dirs = vec![dir.clone()];
        assert!(installed(&words(&["fake-agent", "--x"]), &dirs));
        assert!(installed(&words(&["env", "A=1", "fake-agent"]), &dirs));
        assert!(!installed(&words(&["not-run"]), &dirs));
        assert!(!installed(&words(&["nothing-here"]), &dirs));
        assert!(installed(&[bin.display().to_string()], &[]));
        assert!(!installed(&[], &dirs));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn search_dirs_add_the_usual_places_once() {
        let dirs = search_dirs(Some("/a:/opt/homebrew/bin::/a"), Some(Path::new("/h")));
        assert_eq!(dirs[0], PathBuf::from("/a"));
        assert!(dirs.contains(&PathBuf::from("/h/.local/bin")));
        assert_eq!(
            dirs.iter().filter(|d| d.ends_with("homebrew/bin")).count(),
            1
        );
        assert_eq!(
            dirs.iter().filter(|d| *d == &PathBuf::from("/a")).count(),
            1
        );
    }

    #[test]
    fn write_checks_the_old_text() {
        let dir = scratch("write");
        let path = path_in(&dir.join("deep"));
        write_if_same(&path, "one", None).unwrap();
        assert_eq!(read(&path).unwrap().as_deref(), Some("one"));
        assert!(write_if_same(&path, "two", None).is_err());
        write_if_same(&path, "two", Some("one")).unwrap();
        assert_eq!(read(&path).unwrap().as_deref(), Some("two"));
        fs::remove_dir_all(&dir).ok();
    }
}
