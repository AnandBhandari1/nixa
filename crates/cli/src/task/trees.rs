use super::print;
use crate::args::usage;
use crate::ctx::{self, Ctx};
use crate::http;
use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value, json};
use std::io::Write;

pub const NAME_MAX: usize = 60;
pub const BOX_MAX: usize = 30;
pub const WATCH_TREE: &str = "watch-agents";
pub const WORDS: &str = "list, read, box, walk, put, create, add, change, remove, meta, delete or undo";

pub fn good_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name.len() <= NAME_MAX
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub fn good_box(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && id.len() <= BOX_MAX
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn need_name(name: Option<&String>, what: &str) -> Result<String> {
    let name = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).ok_or_else(|| usage(format!("nixa task trees {what} needs a tree name. Run nixa task trees list")))?;
    if !good_name(&name) {
        return Err(usage(format!("{name} is not a tree name: small letters, numbers and dashes, up to {NAME_MAX}")));
    }
    Ok(name)
}

fn need_box(id: Option<&String>, what: &str) -> Result<String> {
    let id = id.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).ok_or_else(|| usage(format!("nixa task trees {what} needs a box id")))?;
    if !good_box(&id) {
        return Err(usage(format!("{id} is not a box id")));
    }
    Ok(id)
}

fn json_of(text: &str, what: &str) -> Result<Value> {
    serde_json::from_str(text).map_err(|e| anyhow!("{what} is not valid JSON: {e}"))
}

fn json_file(path: &str) -> Result<Value> {
    let text = std::fs::read_to_string(path).map_err(|e| anyhow!("cannot read {path}: {e}"))?;
    json_of(&text, path)
}

fn read(name: &str) -> Result<Value> {
    let reply = http::window_call("GET", &format!("/trees/{name}"), None)?;
    reply.get("tree").cloned().filter(Value::is_object).ok_or_else(|| anyhow!("nixa-brain sent no tree {name}."))
}

fn save(name: &str, tree: &Value) -> Result<Value> {
    let reply = http::window_call("PUT", &format!("/trees/{name}"), Some(&json!({ "tree": tree })))?;
    Ok(reply.get("tree").cloned().unwrap_or(Value::Null))
}

fn text_at<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

pub fn tree_line(info: &Value) -> String {
    let boxes = info.get("boxes").and_then(Value::as_u64).unwrap_or(0);
    let mut line = format!("{}: {} ({boxes} boxes)", text_at(info, "name"), text_at(info, "title"));
    if !text_at(info, "when").is_empty() {
        line.push_str(&format!(", when {}", text_at(info, "when")));
    }
    if !text_at(info, "broken").is_empty() {
        line.push_str(&format!(", broken: {}", text_at(info, "broken")));
    }
    line
}

pub fn box_lines(tree: &Value, id: &str) -> Result<Vec<String>> {
    let boxes = tree.get("boxes").and_then(Value::as_object).ok_or_else(|| anyhow!("The tree has no boxes."))?;
    let one = boxes.get(id).ok_or_else(|| anyhow!("The tree has no box {id}."))?;
    let mut lines = vec![format!("{id}: {}", text_at(one, "text"))];
    match one.get("next").and_then(Value::as_object).filter(|n| !n.is_empty()) {
        Some(next) => {
            for (word, to) in next {
                let to = to.as_str().unwrap_or("");
                let after = boxes.get(to).map(|b| text_at(b, "text")).unwrap_or("");
                lines.push(format!("  {word} -> {to}: {after}"));
            }
        }
        None => lines.push("  (end)".to_string()),
    }
    Ok(lines)
}

fn boxes_mut(tree: &mut Value) -> Result<&mut Map<String, Value>> {
    tree.get_mut("boxes").and_then(Value::as_object_mut).ok_or_else(|| anyhow!("The tree has no boxes."))
}

pub fn next_box_id(tree: &Value) -> String {
    let top = tree
        .get("boxes")
        .and_then(Value::as_object)
        .and_then(|boxes| boxes.keys().filter_map(|k| k.strip_prefix('r')).filter_map(|n| n.parse::<u64>().ok()).max())
        .unwrap_or(0);
    format!("r{}", top + 1)
}

pub fn add_box(tree: &mut Value, text: &str, link: Option<(&str, &str)>) -> Result<String> {
    let text = text.trim();
    if text.is_empty() {
        bail!("A new box needs --text.");
    }
    let id = next_box_id(tree);
    let boxes = boxes_mut(tree)?;
    if let Some((from, word)) = link {
        let word = word.trim();
        if word.is_empty() {
            bail!("--from needs --word, the answer that leads to the new box.");
        }
        let parent = boxes.get_mut(from).and_then(Value::as_object_mut).ok_or_else(|| anyhow!("The tree has no box {from}."))?;
        let next = parent.entry("next").or_insert(json!({}));
        if next.is_null() {
            *next = json!({});
        }
        let next = next.as_object_mut().ok_or_else(|| anyhow!("Box {from} has a broken next list."))?;
        if next.contains_key(word) {
            bail!("Box {from} already has the word {word}.");
        }
        next.insert(word.to_string(), json!(id));
    }
    boxes.insert(id.clone(), json!({ "text": text }));
    Ok(id)
}

pub fn change_box(tree: &mut Value, id: &str, text: Option<&str>, data: Option<Value>) -> Result<()> {
    let boxes = boxes_mut(tree)?;
    let Some(old) = boxes.get_mut(id) else {
        bail!("The tree has no box {id}.");
    };
    match (text, data) {
        (_, Some(data)) => {
            if !data.is_object() {
                bail!("--data must be a JSON object, for example {{\"text\":\"...\",\"next\":{{\"Yes\":\"r2\"}}}}.");
            }
            *old = data;
        }
        (Some(text), None) => {
            let text = text.trim();
            if text.is_empty() {
                bail!("--text must not be empty.");
            }
            let row = old.as_object_mut().ok_or_else(|| anyhow!("Box {id} is broken."))?;
            row.insert("text".into(), json!(text));
        }
        (None, None) => bail!("trees change needs --text or --data."),
    }
    Ok(())
}

pub fn remove_box(tree: &mut Value, id: &str) -> Result<()> {
    if tree.get("start").and_then(Value::as_str) == Some(id) {
        bail!("Box {id} is the start box. Set another start with meta --start first.");
    }
    let boxes = boxes_mut(tree)?;
    if boxes.remove(id).is_none() {
        bail!("The tree has no box {id}.");
    }
    for other in boxes.values_mut() {
        if let Some(next) = other.get_mut("next").and_then(Value::as_object_mut) {
            let gone: Vec<String> = next.iter().filter(|(_, to)| to.as_str() == Some(id)).map(|(word, _)| word.clone()).collect();
            for word in gone {
                next.remove(&word);
            }
        }
    }
    Ok(())
}

pub fn set_meta(tree: &mut Value, title: Option<&str>, when: Option<&str>, start: Option<&str>) -> Result<()> {
    if title.is_none() && when.is_none() && start.is_none() {
        bail!("trees meta needs --title, --when or --start.");
    }
    if let Some(start) = start
        && tree.get("boxes").and_then(|b| b.get(start)).is_none()
    {
        bail!("The tree has no box {start}.");
    }
    let row = tree.as_object_mut().ok_or_else(|| anyhow!("The tree is not a JSON object."))?;
    for (key, value) in [("title", title), ("when", when), ("start", start)] {
        if let Some(value) = value {
            row.insert(key.to_string(), json!(value.trim()));
        }
    }
    Ok(())
}

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--file", "--data", "--text", "--from", "--word", "--title", "--when", "--start"], false)?;
    let what = p.pos.first().map(|w| w.trim().to_lowercase()).unwrap_or_else(|| "list".into());
    let file = p.value(&["--file"]);
    let data = p.value(&["--data"]);
    let text = p.value(&["--text"]);
    let from = p.value(&["--from"]);
    let word = p.value(&["--word"]);
    let title = p.value(&["--title"]);
    let when = p.value(&["--when"]);
    let start = p.value(&["--start"]);
    p.finish(3, false)?;
    let name = p.pos.get(1);
    let id = p.pos.get(2);
    match what.as_str() {
        "list" | "ls" => {
            let reply = http::get("/trees")?;
            let list = reply.get("trees").and_then(Value::as_array).cloned().unwrap_or_default();
            let text = if list.is_empty() { "No rule trees.".to_string() } else { list.iter().map(tree_line).collect::<Vec<_>>().join("\n") };
            print(&ctx, out, &list, &text)
        }
        "walk" | "dry" => {
            if name.is_some_and(|n| n.trim() != WATCH_TREE) {
                return Err(usage(format!("nixa task trees walk tests only the watch tree {WATCH_TREE}")));
            }
            super::walk::run(&ctx, file, out)
        }
        "read" => {
            let name = need_name(name, "read")?;
            let tree = read(&name)?;
            print(&ctx, out, &tree, &serde_json::to_string_pretty(&tree)?)
        }
        "box" => {
            let name = need_name(name, "box")?;
            let tree = read(&name)?;
            let id = match id {
                Some(id) => need_box(Some(id), "box")?,
                None => text_at(&tree, "start").to_string(),
            };
            let lines = box_lines(&tree, &id)?;
            print(&ctx, out, &tree["boxes"][&id], &lines.join("\n"))
        }
        "put" | "create" | "add" | "change" | "remove" | "meta" | "delete" | "undo" => {
            let (name, line) = match what.as_str() {
                "put" => {
                    let name = need_name(name, "put")?;
                    let path = file.ok_or_else(|| usage("nixa task trees put needs --file with the tree JSON"))?;
                    save(&name, &json_file(&path)?)?;
                    let line = format!("Saved rule tree {name} from {path}.");
                    (name, line)
                }
                "create" => {
                    let tree = match (data, file) {
                        (Some(data), _) => json_of(&data, "--data")?,
                        (None, Some(path)) => json_file(&path)?,
                        (None, None) => return Err(usage("nixa task trees create needs --data or --file with the tree JSON")),
                    };
                    let name = need_name(tree.get("name").and_then(Value::as_str).map(str::to_string).as_ref(), "create")?;
                    save(&name, &tree)?;
                    let line = format!("Created rule tree {name}.");
                    (name, line)
                }
                "add" => {
                    let name = need_name(name, "add")?;
                    let mut tree = read(&name)?;
                    let link = match (&from, &word) {
                        (Some(from), Some(word)) => Some((from.as_str(), word.as_str())),
                        (Some(_), None) => return Err(usage("--from needs --word")),
                        (None, Some(_)) => return Err(usage("--word needs --from")),
                        (None, None) => None,
                    };
                    let made = add_box(&mut tree, text.as_deref().unwrap_or_default(), link)?;
                    save(&name, &tree)?;
                    let line = format!("Added box {made} to rule tree {name}.");
                    (name, line)
                }
                "change" => {
                    let name = need_name(name, "change")?;
                    let id = need_box(id, "change")?;
                    let mut tree = read(&name)?;
                    let data = data.as_deref().map(|d| json_of(d, "--data")).transpose()?;
                    change_box(&mut tree, &id, text.as_deref(), data)?;
                    save(&name, &tree)?;
                    let line = format!("Changed box {id} of rule tree {name}.");
                    (name, line)
                }
                "remove" => {
                    let name = need_name(name, "remove")?;
                    let id = need_box(id, "remove")?;
                    let mut tree = read(&name)?;
                    remove_box(&mut tree, &id)?;
                    save(&name, &tree)?;
                    let line = format!("Removed box {id} from rule tree {name}.");
                    (name, line)
                }
                "meta" => {
                    let name = need_name(name, "meta")?;
                    let mut tree = read(&name)?;
                    set_meta(&mut tree, title.as_deref(), when.as_deref(), start.as_deref())?;
                    save(&name, &tree)?;
                    let line = format!("Changed the title, when line or start of rule tree {name}.");
                    (name, line)
                }
                "delete" => {
                    let name = need_name(name, "delete")?;
                    http::window_call("DELETE", &format!("/trees/{name}"), None)?;
                    let line = format!("Deleted rule tree {name}. Undo puts it back.");
                    (name, line)
                }
                _ => {
                    let name = need_name(name, "undo")?;
                    http::window_call("POST", &format!("/trees/{name}/undo"), Some(&json!({})))?;
                    let line = format!("Put back the earlier copy of rule tree {name}.");
                    (name, line)
                }
            };
            print(&ctx, out, &json!({ "tree": name, "done": what }), &line)
        }
        other => Err(usage(format!("nixa task trees {other} is not known. Use {WORDS}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        json!({
            "version": 1,
            "name": "watch-agents",
            "title": "Watch",
            "when": "every few minutes",
            "start": "r1",
            "boxes": {
                "r1": { "text": "Is the agent stuck?", "next": { "Yes": "r2", "No": "r3" } },
                "r2": { "text": "Ask it" },
                "r3": { "text": "Wait" }
            }
        })
    }

    #[test]
    fn names_and_box_ids() {
        assert!(good_name("watch-agents"));
        assert!(!good_name("Watch"));
        assert!(!good_name("../x"));
        assert!(!good_name(""));
        assert!(good_box("r12"));
        assert!(good_box("B_2-x"));
        assert!(!good_box("r 1"));
        assert!(!good_box("/r1"));
    }

    #[test]
    fn boxes_are_added_linked_and_shown() {
        let mut tree = sample();
        assert_eq!(next_box_id(&tree), "r4");
        let made = add_box(&mut tree, " Ping the user ", Some(("r2", "Still stuck"))).unwrap();
        assert_eq!(made, "r4");
        assert_eq!(tree["boxes"]["r2"]["next"]["Still stuck"], json!("r4"));
        assert_eq!(tree["boxes"]["r4"]["text"], json!("Ping the user"));
        assert!(add_box(&mut tree, "x", Some(("r1", "Yes"))).is_err());
        assert!(add_box(&mut tree, "x", Some(("r9", "Yes"))).is_err());
        assert!(add_box(&mut tree, " ", None).is_err());
        let lines = box_lines(&tree, "r1").unwrap();
        assert_eq!(lines[0], "r1: Is the agent stuck?");
        assert!(lines.contains(&"  Yes -> r2: Ask it".to_string()));
        assert_eq!(box_lines(&tree, "r3").unwrap()[1], "  (end)");
        assert!(box_lines(&tree, "r9").is_err());
    }

    #[test]
    fn boxes_change_and_go() {
        let mut tree = sample();
        change_box(&mut tree, "r3", Some(" Wait more "), None).unwrap();
        assert_eq!(tree["boxes"]["r3"]["text"], json!("Wait more"));
        change_box(&mut tree, "r3", None, Some(json!({ "text": "New", "next": { "Go": "r2" } }))).unwrap();
        assert_eq!(tree["boxes"]["r3"]["next"]["Go"], json!("r2"));
        assert!(change_box(&mut tree, "r3", None, Some(json!("text"))).is_err());
        assert!(change_box(&mut tree, "r3", None, None).is_err());
        assert!(remove_box(&mut tree, "r1").unwrap_err().to_string().contains("start box"));
        remove_box(&mut tree, "r2").unwrap();
        assert!(tree["boxes"].get("r2").is_none());
        assert!(tree["boxes"]["r1"]["next"].get("Yes").is_none());
        assert!(tree["boxes"]["r3"]["next"].get("Go").is_none());
        assert!(remove_box(&mut tree, "r2").is_err());
    }

    #[test]
    fn meta_and_lines() {
        let mut tree = sample();
        set_meta(&mut tree, Some(" Watch them "), None, Some("r3")).unwrap();
        assert_eq!((tree["title"].clone(), tree["start"].clone()), (json!("Watch them"), json!("r3")));
        assert!(set_meta(&mut tree, None, None, Some("r9")).is_err());
        assert!(set_meta(&mut tree, None, None, None).is_err());
        let info = json!({ "name": "watch-agents", "title": "Watch", "when": "often", "boxes": 3 });
        assert_eq!(tree_line(&info), "watch-agents: Watch (3 boxes), when often");
    }
}
