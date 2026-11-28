use crate::util::{append_line, atomic_write, now_ms};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

pub const WATCH_TREE: &str = "watch-agents";
pub const MAX_BOXES: usize = 200;
pub const MAX_TEXT: usize = 1000;
pub const MAX_WORD: usize = 60;

pub const WATCH_FACTS: [&str; 16] = [
    "subject",
    "terminal.id",
    "terminal.name",
    "terminal.agent",
    "terminal.space",
    "terminal.status",
    "terminal.note",
    "terminal.minutesInStatus",
    "terminal.running",
    "terminal.exitCode",
    "terminal.isChild",
    "terminal.children",
    "terminal.nixaJob",
    "terminal.minutesSinceSent",
    "terminal.draft",
    "terminal.keep",
];

static TREE_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z0-9][a-z0-9-]{0,59}$").unwrap());
static BOX_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9_-]{0,29}$").unwrap());
static FACT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z][A-Za-z0-9.]{0,60}$").unwrap());
pub static SLOT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{([a-zA-Z][a-zA-Z0-9.]*)\}").unwrap());
static BOX_NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^r(\d+)$").unwrap());

pub fn good_name(name: &str) -> bool {
    TREE_NAME.is_match(name)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Condition {
    pub fact: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is: Option<Value>,
    #[serde(default, rename = "in", skip_serializing_if = "Option::is_none")]
    pub within: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gt: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gte: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lt: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lte: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Check {
    All { all: Vec<Condition> },
    Any { any: Vec<Condition> },
    Cases {
        fact: String,
        cases: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        otherwise: Option<String>,
    },
    One(Condition),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleBox {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<Check>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub report: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleTree {
    pub version: u32,
    pub name: String,
    pub title: String,
    pub when: String,
    pub start: String,
    pub boxes: BTreeMap<String, RuleBox>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TreeInfo {
    pub name: String,
    pub title: String,
    pub when: String,
    pub boxes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broken: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NextLine {
    pub word: String,
    pub to: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BoxView {
    pub tree: String,
    pub id: String,
    pub text: String,
    pub next: Vec<NextLine>,
    pub end: bool,
}

fn text_of(raw: &Map<String, Value>, key: &str) -> String {
    raw.get(key).and_then(Value::as_str).map(str::trim).unwrap_or("").to_string()
}

fn known_fact(fact: &str) -> Result<String, String> {
    if WATCH_FACTS.contains(&fact) {
        Ok(fact.to_string())
    } else {
        Err(format!("Unknown fact \"{fact}\". Facts: {}.", WATCH_FACTS.join(", ")))
    }
}

fn check_condition(raw: &Value) -> Result<Condition, String> {
    let bad = "A condition is { fact, is | in | gt | gte | lt | lte }, for example { \"fact\": \"terminal.minutesInStatus\", \"gt\": 30 }.";
    let row = raw.as_object().ok_or(bad)?;
    let fact = row.get("fact").and_then(Value::as_str).map(str::trim).filter(|f| FACT.is_match(f)).ok_or(bad)?;
    let ops: Vec<&str> = ["is", "in", "gt", "gte", "lt", "lte"].into_iter().filter(|k| row.get(*k).is_some_and(|v| !v.is_null())).collect();
    if ops.len() != 1 {
        return Err("A condition needs exactly one of is, in, gt, gte, lt, lte.".into());
    }
    let key = ops[0];
    let value = &row[key];
    let mut out = Condition { fact: known_fact(fact)?, is: None, within: None, gt: None, gte: None, lt: None, lte: None };
    match key {
        "is" => out.is = Some(value.clone()),
        "in" => out.within = Some(value.as_array().cloned().ok_or("in takes a list of values.")?),
        _ => {
            let number = value.as_f64().ok_or(format!("{key} takes a number."))?;
            match key {
                "gt" => out.gt = Some(number),
                "gte" => out.gte = Some(number),
                "lt" => out.lt = Some(number),
                _ => out.lte = Some(number),
            }
        }
    }
    Ok(out)
}

pub fn check_check(raw: &Value) -> Result<Check, String> {
    let row = raw.as_object().ok_or("A check is one condition, { all: [...] }, { any: [...] } or { fact, cases }.")?;
    if let Some(all) = row.get("all").and_then(Value::as_array) {
        return Ok(Check::All { all: all.iter().map(check_condition).collect::<Result<_, _>>()? });
    }
    if let Some(any) = row.get("any").and_then(Value::as_array) {
        return Ok(Check::Any { any: any.iter().map(check_condition).collect::<Result<_, _>>()? });
    }
    if let Some(cases) = row.get("cases").and_then(Value::as_object) {
        let mut out = BTreeMap::new();
        for (value, word) in cases {
            let word = word.as_str().map(str::trim).filter(|w| !w.is_empty()).ok_or(format!("Case \"{value}\" must name a line."))?;
            out.insert(value.clone(), word.to_string());
        }
        let fact = text_of(row, "fact");
        if !FACT.is_match(&fact) {
            return Err("A cases check needs fact, for example terminal.status.".into());
        }
        let otherwise = row.get("otherwise").and_then(Value::as_str).map(|s| s.trim().to_string());
        return Ok(Check::Cases { fact: known_fact(&fact)?, cases: out, otherwise });
    }
    Ok(Check::One(check_condition(raw)?))
}

pub fn check_box(raw: &Value) -> Result<RuleBox, String> {
    let row = raw.as_object().ok_or("A box is { text, next }: text is one plain English rule, next maps a word to the next box id.")?;
    let text = text_of(row, "text");
    if text.is_empty() {
        return Err("A box needs text: one plain English rule.".into());
    }
    if text.chars().count() > MAX_TEXT {
        return Err(format!("Box text is at most {MAX_TEXT} characters."));
    }
    let check = match row.get("check") {
        Some(v) if !v.is_null() => Some(check_check(v)?),
        _ => None,
    };
    let report = row.get("report").and_then(Value::as_bool) == Some(true);
    if report {
        for slot in SLOT.captures_iter(&text) {
            if &slot[1] != "item" {
                known_fact(&slot[1])?;
            }
        }
    }
    let next = match row.get("next") {
        None | Some(Value::Null) => None,
        Some(Value::Object(map)) => {
            let mut out = BTreeMap::new();
            for (word, to) in map {
                let label = if word.trim().is_empty() { "then".to_string() } else { word.trim().to_string() };
                if label.chars().count() > MAX_WORD {
                    return Err(format!("A line word is at most {MAX_WORD} characters."));
                }
                let to = to.as_str().map(str::trim).filter(|t| BOX_ID.is_match(t)).ok_or(format!("Line \"{label}\" must point to a box id."))?;
                out.insert(label, to.to_string());
            }
            (!out.is_empty()).then_some(out)
        }
        Some(_) => return Err("next maps a word to a box id, for example { \"Yes\": \"r2\", \"No\": \"r3\" }.".into()),
    };
    Ok(RuleBox { text, next, check, report })
}

pub fn check_tree(raw: &Value, file_name: Option<&str>) -> Result<RuleTree, String> {
    let row = raw.as_object().ok_or("A tree is { name, title, when, start, boxes }.")?;
    if row.get("version").is_some_and(|v| !v.is_null() && v.as_u64() != Some(1)) {
        return Err("Tree version must be 1.".into());
    }
    let given = text_of(row, "name");
    let name = if given.is_empty() { file_name.unwrap_or("").trim().to_string() } else { given.clone() };
    if !good_name(&name) {
        return Err("Tree name: small letters, numbers and dashes, up to 60.".into());
    }
    if let Some(file) = file_name
        && !given.is_empty() && given != file {
            return Err(format!("Tree file {file}.json has name \"{given}\", which must match the filename."));
        }
    let title = Some(text_of(row, "title")).filter(|t| !t.is_empty()).unwrap_or_else(|| name.clone());
    let when = text_of(row, "when");
    if when.is_empty() {
        return Err("A tree needs when: one plain line that says when to use it.".into());
    }
    let raw_boxes = row.get("boxes").and_then(Value::as_object).ok_or("A tree needs boxes: { \"r1\": { text, next } }.")?;
    if raw_boxes.is_empty() {
        return Err("A tree needs at least one box.".into());
    }
    if raw_boxes.len() > MAX_BOXES {
        return Err(format!("A tree has at most {MAX_BOXES} boxes."));
    }
    let mut boxes = BTreeMap::new();
    let mut first = String::new();
    for (id, value) in raw_boxes {
        if !BOX_ID.is_match(id) {
            return Err(format!("Box id {id}: letters, numbers, dash or underscore, up to 30."));
        }
        if first.is_empty() {
            first = id.clone();
        }
        boxes.insert(id.clone(), check_box(value).map_err(|e| format!("Box {id}: {e}"))?);
    }
    let start = Some(text_of(row, "start")).filter(|s| !s.is_empty()).unwrap_or(first);
    if !boxes.contains_key(&start) {
        return Err(format!("start {start} is not a box of this tree."));
    }
    for (id, rule) in &boxes {
        for (word, to) in rule.next.iter().flatten() {
            if !boxes.contains_key(to) {
                return Err(format!("Box {id} line \"{word}\" points to {to}, which is not a box of this tree."));
            }
        }
    }
    if boxes.values().any(|b| b.check.is_some()) {
        let mut reached: HashSet<String> = HashSet::from([start.clone()]);
        let mut queue = vec![start.clone()];
        while let Some(id) = queue.pop() {
            for to in boxes.get(&id).and_then(|b| b.next.as_ref()).into_iter().flat_map(|n| n.values()) {
                if reached.insert(to.clone()) {
                    queue.push(to.clone());
                }
            }
        }
        let lost: Vec<&String> = boxes.iter().filter(|(id, b)| b.report && !reached.contains(*id)).map(|(id, _)| id).collect();
        if !lost.is_empty() {
            let names: Vec<&str> = lost.iter().map(|s| s.as_str()).collect();
            return Err(format!("Report box {} can never be reached: no line leads to it from {start}.", names.join(", ")));
        }
    }
    Ok(RuleTree { version: 1, name, title, when, start, boxes })
}

pub fn next_box_id(tree: &RuleTree) -> String {
    let top = tree.boxes.keys().filter_map(|id| BOX_NUMBER.captures(id).and_then(|c| c[1].parse::<u64>().ok())).max().unwrap_or(0);
    format!("r{}", top + 1)
}

pub fn box_view(tree: &RuleTree, id: &str) -> Result<BoxView, String> {
    let rule = tree.boxes.get(id).ok_or_else(|| {
        let ids: Vec<&str> = tree.boxes.keys().map(String::as_str).collect();
        format!("Tree {} has no box {id}. Boxes: {}.", tree.name, ids.join(", "))
    })?;
    let next: Vec<NextLine> = rule
        .next
        .iter()
        .flatten()
        .map(|(word, to)| NextLine { word: word.clone(), to: to.clone(), text: tree.boxes.get(to).map(|b| b.text.clone()).unwrap_or_default() })
        .collect();
    Ok(BoxView { tree: tree.name.clone(), id: id.to_string(), text: rule.text.clone(), end: next.is_empty(), next })
}

pub fn default_watch_tree() -> Value {
    json!({
        "version": 1,
        "name": WATCH_TREE,
        "title": "Watch the agents",
        "when": "Every watch tick, for each terminal Nixa knows.",
        "start": "r1",
        "boxes": {
            "r1": {"text": "Is this a terminal?", "check": {"fact": "subject", "is": "terminal"}, "next": {"Yes": "r2", "No": "r9"}},
            "r2": {"text": "What status did the program report?", "check": {"fact": "terminal.status", "cases": {"blocked": "blocked", "error": "error", "working": "working", "done": "done"}, "otherwise": "other"}, "next": {"blocked": "r3", "error": "r4", "working": "r5", "done": "r7", "other": "r9"}},
            "r3": {"text": "Report: {item} ({terminal.name}) is blocked and waits for an answer: {terminal.note}.", "report": true},
            "r4": {"text": "Report: {item} ({terminal.name}) reported an error: {terminal.note}.", "report": true},
            "r5": {"text": "Has it worked for more than 30 minutes?", "check": {"fact": "terminal.minutesInStatus", "gt": 30}, "next": {"Yes": "r6", "No": "r9"}},
            "r6": {"text": "Report: {item} ({terminal.name}) has been working for {terminal.minutesInStatus} minutes. Check if it is stuck.", "report": true},
            "r7": {"text": "Did Nixa send it a job?", "check": {"fact": "terminal.nixaJob", "is": true}, "next": {"Yes": "r8", "No": "r9"}},
            "r8": {"text": "Report: {item} ({terminal.name}) is done with the job Nixa sent. Check the work.", "report": true},
            "r9": {"text": "Nothing to do."}
        }
    })
}

pub const QUESTION_TREE: &str = "agent-question";
pub const FINISHED_TREE: &str = "agent-finished";
pub const PROMPT_MAX: usize = 24_000;
pub const NOTE_MAX: usize = 12_000;

pub fn default_question_tree() -> Value {
    json!({
        "version": 1,
        "name": QUESTION_TREE,
        "title": "Agent asks a question",
        "when": "An agent asks a question, asks for a permission or shows a plan, and waits for an answer.",
        "start": "r1",
        "boxes": {
            "r1": {"text": "Did the user already answer it on the agent's screen, or say they will answer it?", "next": {"Yes": "r2", "No": "r8"}},
            "r2": {"text": "Leave it. Drop your own question about it."},
            "r8": {"text": "Read its screen with nixa_term_read and weigh every option against its job and the context. Never pick option 1 or the one marked Recommended without thinking.", "next": {"then": "r9"}},
            "r9": {"text": "Is the best answer clear?", "next": {"Yes": "r10", "No": "r11"}},
            "r10": {"text": "Answer it with nixa_answer_prompt, decide true and a short reason. Allow once, never always. Then tell the user in one short sentence what you chose and why."},
            "r11": {"text": "Ask the deep brain with nixa_deep_ask: the question, the options and short facts. When its answer comes, answer the agent with nixa_answer_prompt and say what you chose. When no deep brain is up, pick the safer option that can be undone and say so."}
        }
    })
}

pub fn default_finished_tree() -> Value {
    json!({
        "version": 1,
        "name": FINISHED_TREE,
        "title": "Agent finished its work",
        "when": "An agent finished its work or stopped, or an event or a watch finding says an agent is done.",
        "start": "r1",
        "boxes": {
            "r1": {"text": "Did Nixa or the user give it this work as a job?", "next": {"Yes": "r2", "No": "r10"}},
            "r2": {"text": "Read its reply, its screen and the files it changed. Is the job met and proven: checks passed, tests ran and passed, or no test was needed?", "next": {"Yes": "r13", "No": "r6"}},
            "r4": {"text": "Does it suggest or offer a next step that fits its work?", "next": {"Yes": "r5", "No": "r9"}},
            "r5": {"text": "Tell it to go ahead with nixa_term_send. Do not wait for the user. Then say in one sentence what you told it."},
            "r6": {"text": "Can the agent fix what is missing or failed, and is your fix different from the messages you already sent for this job?", "next": {"Yes": "r7", "No": "r16"}},
            "r7": {"text": "Send one clear fix with nixa_term_send. Name the failing check and its last lines. Ask it to finish the missing part, run the tests and show the result. Then say in one sentence what you asked."},
            "r9": {"text": "Close it. Say in one sentence that it is done and checked, with the proof. Send nothing to the agent."},
            "r10": {"text": "Read enough of its screen and its work first. Does it ask a question or offer a next step?", "next": {"Yes": "r11", "No": "r12"}},
            "r11": {"text": "Follow the tree agent-question (Agent asks a question) from its start box."},
            "r12": {"text": "Things it lists for the user to do are not questions to you. Ask nixa_deep_ask for a way to do each one without the user (another agent, an emulator, a saved login), and tell the user only what truly needs their hands. When it is complete, say in one or two short sentences what it finished."},
            "r13": {"text": "Is the next step a git write, a deploy, production, or a choice such as what to build, a name or money?", "next": {"Yes": "r14", "No": "r4"}},
            "r14": {"text": "Do not wait for the user. Decide it with nixa_deep_ask, or with your own best pick when no deep brain is up. For a commit or a push use nixa_git_commit or nixa_git_push. Record the choice, then go on.", "next": {"then": "r5"}},
            "r16": {"text": "Do not hand the job to the user and do not wait. Ask nixa_deep_ask for a different fix and send it, or give the work to another agent with nixa_term_start. Record what you chose in one short line, then go on."}
        }
    })
}

pub fn trees_json(trees: &[RuleTree], max: usize) -> String {
    let mut shown = Vec::new();
    let mut left = Vec::new();
    let mut used = 2;
    for tree in trees {
        let text = serde_json::to_string(tree).unwrap_or_default();
        if used + text.len() < max {
            used += text.len() + 1;
            shown.push(text);
        } else {
            left.push(tree.name.clone());
        }
    }
    let mut out = format!("[{}]", shown.join(","));
    if !left.is_empty() {
        out.push_str(&format!("\nNot shown here because they are too big: {}. Read them with nixa_trees read when you need them.", left.join(", ")));
    }
    out
}

pub fn carry_note(carry: &str, name: &str, note: &str) -> String {
    let own = format!("Note: Rule tree {name} ");
    let old = format!("Note: the user changed rule tree {name}.");
    let mut parts: Vec<&str> = carry.split("\n\n").filter(|p| !p.is_empty() && !p.starts_with(&own) && !p.starts_with(&old)).collect();
    parts.push(note);
    parts.join("\n\n")
}

pub fn change_note(store: &mut TreeStore, name: &str, by: &str, max: usize) -> String {
    match store.read(name) {
        Ok(tree) => {
            let text = serde_json::to_string(&tree).unwrap_or_default();
            if text.len() <= max {
                format!("Note: Rule tree {name} was changed {by}. Its JSON now: {text}")
            } else {
                format!("Note: Rule tree {name} was changed {by}. It is too big to show here. Read it with nixa_trees read when you need it.")
            }
        }
        Err(_) if store.names().iter().any(|n| n == name) => format!("Note: Rule tree {name} was changed {by}, but the new file is broken, so its last good copy stays."),
        Err(_) => format!("Note: Rule tree {name} was removed {by}."),
    }
}

pub struct TreeStore {
    pub dir: PathBuf,
    good: HashMap<String, RuleTree>,
    seen: HashMap<String, String>,
    errors: HashMap<String, String>,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize)]
pub struct SyncReport {
    pub changed: Vec<String>,
    pub removed: Vec<String>,
    pub problems: Vec<(String, String)>,
}

fn parse_file(name: &str, raw: &str) -> Result<RuleTree, String> {
    let value: Value = serde_json::from_str(raw).map_err(|e| format!("Invalid JSON in {name}.json: {e}"))?;
    check_tree(&value, Some(name))
}

impl TreeStore {
    pub fn open(dir: &Path) -> TreeStore {
        let mut store = TreeStore { dir: dir.to_path_buf(), good: HashMap::new(), seen: HashMap::new(), errors: HashMap::new() };
        store.sync();
        store
    }

    fn file_of(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.json"))
    }

    fn history(&self) -> PathBuf {
        self.dir.join(".history")
    }

    fn keep(&self, name: &str, suffix: &str) -> std::io::Result<()> {
        let file = self.file_of(name);
        if let Ok(text) = std::fs::read_to_string(&file) {
            atomic_write(&self.history().join(format!("{name}-{}.{suffix}", now_ms())), &text)?;
        }
        Ok(())
    }

    fn log_edit(&self, who: &str, tree: &str, op: &str, id: Option<&str>) {
        let line = json!({"at": now_ms(), "who": who, "tree": tree, "op": op, "id": id}).to_string();
        let _ = append_line(&self.dir.join("edits.jsonl"), &line);
    }

    pub fn names(&self) -> Vec<String> {
        let Ok(read) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        let mut names: Vec<String> = read
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".json")).map(str::to_string))
            .filter(|n| good_name(n))
            .collect();
        names.sort();
        names
    }

    pub fn read(&mut self, name: &str) -> Result<RuleTree, String> {
        let file = self.file_of(name);
        if !good_name(name) || !file.exists() {
            return Err(format!("No tree named {name}. Trees: {}.", { let n = self.names(); if n.is_empty() { "none".to_string() } else { n.join(", ") } }));
        }
        let raw = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
        let tree = parse_file(name, &raw)?;
        self.good.insert(name.to_string(), tree.clone());
        self.seen.insert(name.to_string(), raw);
        self.errors.remove(name);
        Ok(tree)
    }

    pub fn all(&mut self) -> Vec<RuleTree> {
        let names = self.names();
        names.iter().filter_map(|n| self.read(n).ok().or_else(|| self.good.get(n).cloned())).collect()
    }

    pub fn list(&mut self) -> Vec<TreeInfo> {
        let names = self.names();
        names
            .into_iter()
            .map(|name| match self.read(&name) {
                Ok(tree) => TreeInfo { name, title: tree.title, when: tree.when, boxes: tree.boxes.len(), broken: None },
                Err(error) => TreeInfo { title: name.clone(), name, when: String::new(), boxes: 0, broken: Some(error) },
            })
            .collect()
    }

    pub fn save(&mut self, tree: &RuleTree, who: &str, op: &str, id: Option<&str>) -> Result<RuleTree, String> {
        let value = serde_json::to_value(tree).map_err(|e| e.to_string())?;
        let checked = check_tree(&value, None)?;
        let written = format!("{}\n", serde_json::to_string_pretty(&checked).map_err(|e| e.to_string())?);
        self.keep(&checked.name, "json").map_err(|e| e.to_string())?;
        atomic_write(&self.file_of(&checked.name), &written).map_err(|e| e.to_string())?;
        self.good.insert(checked.name.clone(), checked.clone());
        self.seen.insert(checked.name.clone(), written);
        self.errors.remove(&checked.name);
        self.log_edit(who, &checked.name, op, id);
        Ok(checked)
    }

    pub fn create(&mut self, raw: &Value, who: &str) -> Result<RuleTree, String> {
        let tree = check_tree(raw, None)?;
        if self.file_of(&tree.name).exists() {
            return Err(format!("A tree named {} already exists.", tree.name));
        }
        self.save(&tree, who, "create", None)
    }

    pub fn put(&mut self, name: &str, raw: &Value, who: &str) -> Result<RuleTree, String> {
        if !good_name(name) {
            return Err("Tree name: small letters, numbers and dashes, up to 60.".into());
        }
        let tree = check_tree(raw, Some(name))?;
        let op = if self.file_of(name).exists() { "put" } else { "create" };
        self.save(&tree, who, op, None)
    }

    pub fn add_box(&mut self, name: &str, text: &str, who: &str, from: Option<&str>, word: Option<&str>) -> Result<(RuleTree, String), String> {
        let mut tree = self.read(name)?;
        let id = next_box_id(&tree);
        let rule = check_box(&json!({"text": text}))?;
        tree.boxes.insert(id.clone(), rule);
        if let Some(from) = from {
            let parent = tree.boxes.get_mut(from).ok_or(format!("Tree {name} has no box {from}."))?;
            let label = word.map(str::trim).filter(|w| !w.is_empty()).unwrap_or("then").to_string();
            parent.next.get_or_insert_with(BTreeMap::new).insert(label, id.clone());
        }
        let saved = self.save(&tree, who, "add", Some(&id))?;
        Ok((saved, id))
    }

    pub fn set_box(&mut self, name: &str, id: &str, raw: &Value, who: &str) -> Result<RuleTree, String> {
        let mut tree = self.read(name)?;
        if !tree.boxes.contains_key(id) {
            return Err(format!("Tree {name} has no box {id}. Use add for a new box."));
        }
        tree.boxes.insert(id.to_string(), check_box(raw)?);
        self.save(&tree, who, "change", Some(id))
    }

    pub fn remove_box(&mut self, name: &str, id: &str, who: &str) -> Result<RuleTree, String> {
        let mut tree = self.read(name)?;
        if !tree.boxes.contains_key(id) {
            return Err(format!("Tree {name} has no box {id}."));
        }
        if tree.start == id {
            return Err("The top box cannot be removed. Change start first.".into());
        }
        tree.boxes.remove(id);
        for rule in tree.boxes.values_mut() {
            if let Some(next) = rule.next.as_mut() {
                next.retain(|_, to| to != id);
                if next.is_empty() {
                    rule.next = None;
                }
            }
        }
        self.save(&tree, who, "remove", Some(id))
    }

    pub fn set_meta(&mut self, name: &str, title: Option<&str>, when: Option<&str>, start: Option<&str>, who: &str) -> Result<RuleTree, String> {
        let mut tree = self.read(name)?;
        if let Some(t) = title.map(str::trim).filter(|t| !t.is_empty()) {
            tree.title = t.to_string();
        }
        if let Some(w) = when.map(str::trim).filter(|w| !w.is_empty()) {
            tree.when = w.to_string();
        }
        if let Some(s) = start.map(str::trim).filter(|s| !s.is_empty()) {
            tree.start = s.to_string();
        }
        self.save(&tree, who, "meta", None)
    }

    pub fn remove_tree(&mut self, name: &str, who: &str) -> Result<(), String> {
        self.read(name)?;
        self.keep(name, "removed").map_err(|e| e.to_string())?;
        let file = self.file_of(name);
        std::fs::rename(&file, file.with_extension("json.removed")).map_err(|e| e.to_string())?;
        self.good.remove(name);
        self.seen.remove(name);
        self.errors.remove(name);
        self.log_edit(who, name, "remove tree", None);
        Ok(())
    }

    pub fn undo(&mut self, name: &str, who: &str) -> Result<RuleTree, String> {
        let prefix = format!("{name}-");
        let mut saved: Vec<(u64, String)> = std::fs::read_dir(self.history())
            .map(|read| {
                read.filter_map(|e| e.ok())
                    .filter_map(|e| e.file_name().to_str().map(str::to_string))
                    .filter_map(|f| f.strip_prefix(&prefix).and_then(|r| r.strip_suffix(".json")).and_then(|n| n.parse::<u64>().ok()).map(|n| (n, f)))
                    .collect()
            })
            .unwrap_or_default();
        saved.sort();
        let (_, last) = saved.pop().ok_or(format!("Tree {name} has no earlier version."))?;
        let path = self.history().join(&last);
        let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        parse_file(name, &text)?;
        atomic_write(&self.file_of(name), &text).map_err(|e| e.to_string())?;
        std::fs::rename(&path, path.with_extension("undone")).map_err(|e| e.to_string())?;
        self.log_edit(who, name, "undo", None);
        self.read(name)
    }

    pub fn view(&mut self, name: &str, id: Option<&str>) -> Result<BoxView, String> {
        let tree = self.read(name)?;
        let id = id.filter(|i| !i.is_empty()).unwrap_or(&tree.start).to_string();
        box_view(&tree, &id)
    }

    pub fn sync(&mut self) -> SyncReport {
        let current: HashSet<String> = self.names().into_iter().collect();
        let mut report = SyncReport::default();
        let old: Vec<String> = self.seen.keys().cloned().collect();
        for name in old {
            if !current.contains(&name) {
                self.seen.remove(&name);
                self.good.remove(&name);
                self.errors.remove(&name);
                report.removed.push(name);
            }
        }
        let mut names: Vec<&String> = current.iter().collect();
        names.sort();
        for name in names {
            let Ok(raw) = std::fs::read_to_string(self.file_of(name)) else { continue };
            if self.seen.get(name) == Some(&raw) {
                continue;
            }
            self.seen.insert(name.clone(), raw.clone());
            match parse_file(name, &raw) {
                Ok(tree) => {
                    self.good.insert(name.clone(), tree);
                    self.errors.remove(name);
                    report.changed.push(name.clone());
                }
                Err(error) => {
                    if self.errors.get(name) != Some(&error) {
                        self.errors.insert(name.clone(), error.clone());
                        report.problems.push((name.clone(), error));
                    }
                }
            }
        }
        report
    }

    pub fn ensure_watch_tree(&mut self) {
        if !self.file_of(WATCH_TREE).exists() {
            let _ = self.create(&default_watch_tree(), "nixa");
        }
    }

    fn seed(&mut self, name: &str, raw: &Value) {
        let file = self.file_of(name);
        if !file.exists() && !file.with_extension("json.removed").exists() {
            let _ = self.create(raw, "nixa");
            return;
        }
        let old = std::fs::read_to_string(&file).unwrap_or_default();
        if old.contains("Decision manual") {
            let _ = self.put(name, raw, "nixa");
        }
    }

    pub fn ensure_first_trees(&mut self) {
        self.ensure_watch_tree();
        self.seed(QUESTION_TREE, &default_question_tree());
        self.seed(FINISHED_TREE, &default_finished_tree());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_trees() {
        let tree = check_tree(&default_watch_tree(), Some(WATCH_TREE)).unwrap();
        assert_eq!(tree.start, "r1");
        assert!(check_tree(&json!({"name": "Bad Name", "when": "x", "boxes": {"r1": {"text": "a"}}}), None).is_err());
        assert!(check_tree(&json!({"name": "a", "boxes": {"r1": {"text": "a"}}}), None).is_err());
        let lost = json!({"name": "a", "when": "w", "boxes": {"r1": {"text": "q", "check": {"fact": "subject", "is": "x"}}, "r2": {"text": "Report: x", "report": true}}});
        assert!(check_tree(&lost, None).unwrap_err().contains("never be reached"));
        let bad_fact = json!({"name": "a", "when": "w", "boxes": {"r1": {"text": "q", "check": {"fact": "nope", "is": 1}}}});
        assert!(check_tree(&bad_fact, None).unwrap_err().contains("Unknown fact"));
        let bad_link = json!({"name": "a", "when": "w", "boxes": {"r1": {"text": "q", "next": {"Yes": "r5"}}}});
        assert!(check_tree(&bad_link, None).is_err());
    }

    #[test]
    fn old_manual_trees_move_to_the_new_default() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = TreeStore::open(dir.path());
        let mut old = default_question_tree();
        old["boxes"]["r1"]["next"]["No"] = json!("r3");
        old["boxes"]["r3"] = json!({"text": "Is Decision manual on?", "next": {"Yes": "r2", "No": "r8"}});
        store.create(&old, "nixa").unwrap();
        store.ensure_first_trees();
        let text = std::fs::read_to_string(store.file_of(QUESTION_TREE)).unwrap();
        assert!(!text.contains("Decision manual"), "{text}");
        assert!(!text.contains("Decision automatic"), "{text}");
        assert!(check_tree(&default_finished_tree(), Some(FINISHED_TREE)).is_ok());
    }

    #[test]
    fn store_edits_and_undo() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = TreeStore::open(dir.path());
        store.ensure_watch_tree();
        assert_eq!(store.names(), vec![WATCH_TREE.to_string()]);
        let made = store.create(&json!({"name": "answers", "when": "An agent asks", "boxes": {"r1": {"text": "Is it a test run?"}}}), "sam").unwrap();
        assert_eq!(made.start, "r1");
        let (tree, id) = store.add_box("answers", "Say yes.", "nixa", Some("r1"), Some("Yes")).unwrap();
        assert_eq!(id, "r2");
        assert_eq!(tree.boxes["r1"].next.as_ref().unwrap()["Yes"], "r2");
        let view = store.view("answers", None).unwrap();
        assert_eq!(view.next[0].text, "Say yes.");
        let back = store.undo("answers", "sam").unwrap();
        assert_eq!(back.boxes.len(), 1);
        let tree = store.remove_box("answers", "r1", "x");
        assert!(tree.is_err());
        store.remove_tree("answers", "sam").unwrap();
        assert!(store.read("answers").is_err());
    }

    #[test]
    fn first_trees_are_seeded_once() {
        for raw in [default_question_tree(), default_finished_tree()] {
            let tree = check_tree(&raw, None).unwrap();
            assert_eq!(tree.start, "r1");
            assert!(tree.boxes.values().all(|b| b.text.chars().count() <= MAX_TEXT));
        }
        let dir = tempfile::tempdir().unwrap();
        let mut store = TreeStore::open(dir.path());
        store.ensure_first_trees();
        assert_eq!(store.names(), vec![FINISHED_TREE.to_string(), QUESTION_TREE.to_string(), WATCH_TREE.to_string()]);
        assert_eq!(store.read(QUESTION_TREE).unwrap().title, "Agent asks a question");
        let finished = store.read(FINISHED_TREE).unwrap();
        assert_eq!(finished.title, "Agent finished its work");
        assert_eq!(finished.boxes["r6"].next.as_ref().unwrap()["No"], "r16");
        assert!(!finished.boxes.contains_key("r15"));
        assert!(finished.boxes["r16"].text.contains("nixa_deep_ask") && !finished.boxes["r16"].text.contains("Tell the user"));
        store.remove_tree(QUESTION_TREE, "sam").unwrap();
        store.ensure_first_trees();
        assert!(!store.names().contains(&QUESTION_TREE.to_string()));
    }

    #[test]
    fn trees_json_keeps_a_size_cap() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = TreeStore::open(dir.path());
        store.ensure_first_trees();
        let all = store.all();
        let full = trees_json(&all, PROMPT_MAX);
        let parsed: Vec<RuleTree> = serde_json::from_str(&full).unwrap();
        assert_eq!(parsed.len(), 3);
        let small = trees_json(&all, 3000);
        let (json_part, rest) = small.split_once('\n').unwrap();
        assert!(json_part.len() <= 3000);
        assert!(rest.contains("Not shown here"));
        assert_eq!(trees_json(&[], 10), "[]");
        let note = change_note(&mut store, WATCH_TREE, "by the user", NOTE_MAX);
        assert!(note.starts_with("Note: Rule tree watch-agents was changed by the user. Its JSON now: {"));
        assert!(change_note(&mut store, WATCH_TREE, "on disk", 10).contains("too big"));
        store.remove_tree(WATCH_TREE, "sam").unwrap();
        assert_eq!(change_note(&mut store, WATCH_TREE, "on disk", NOTE_MAX), "Note: Rule tree watch-agents was removed on disk.");
        std::fs::write(dir.path().join("bad.json"), "{").unwrap();
        assert!(change_note(&mut store, "bad", "on disk", NOTE_MAX).contains("broken"));
        let first = carry_note("", WATCH_TREE, "Note: Rule tree watch-agents was changed by the user. Its JSON now: {}");
        let second = carry_note(&format!("Other note\n\n{first}"), WATCH_TREE, "Note: Rule tree watch-agents was removed on disk.");
        assert_eq!(second, "Other note\n\nNote: Rule tree watch-agents was removed on disk.");
        assert_eq!(carry_note("Note: the user changed rule tree a. Read it again before you use it.", "a", "Note: Rule tree a was removed on disk."), "Note: Rule tree a was removed on disk.");
    }

    #[test]
    fn sync_reports_broken_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = TreeStore::open(dir.path());
        std::fs::write(dir.path().join("bad.json"), "{").unwrap();
        let report = store.sync();
        assert_eq!(report.problems.len(), 1);
        assert!(store.sync().problems.is_empty());
        assert!(store.list()[0].broken.is_some());
    }
}
