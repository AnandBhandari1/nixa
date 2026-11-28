use super::commit::locked_text;
use super::policy::{clip, plural};
use super::repo::base_name;
use crate::app::App;
use crate::project_tools::find::{ProjectSpace, spaces_of};
use crate::review_flow::check_tool::{changed_in, plan_checks, seconds_of};
use crate::review_flow::checks::{Planned, RunOptions, argv_of, find_program, plan, run_plan, short_checks, typed_plan};
use crate::tools::{Args, strings};
use regex::Regex;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::{LazyLock, Mutex};

pub const COMMANDS_FILE: &str = "docs/commands.md";
pub const SETTING_PREFIX: &str = "space_checks.";
pub const APPROVED_PREFIX: &str = "space_checks_ok.";
pub const PACKAGES: &str = "package scripts";
pub const MAX_FOUND: usize = 12;
pub const MAX_RUN: usize = 4;
pub const COMMAND_CHARS: usize = 300;
const FILE_BYTES: u64 = 1_000_000;
const SECTION_WORDS: [&str; 5] = ["check", "test", "lint", "typecheck", "verif"];

static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s{0,3}#{1,6}\s+(.+?)\s*#*\s*$").expect("heading"));
static BULLET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(?:[-*+]|\d+[.)])\s+(.+)$").expect("bullet"));
static TICKS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`([^`]+)`").expect("ticks"));
static ENV_SET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*=\S*$").expect("env"));
static PROGRAM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_.+/-]+$").expect("program"));

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub source: String,
    pub from_file: bool,
    pub planned: Vec<Planned>,
    pub skipped: Vec<Value>,
    pub none: String,
}

pub fn check_heading(text: &str) -> bool {
    text.to_lowercase().split(|c: char| !c.is_alphanumeric()).any(|w| SECTION_WORDS.iter().any(|s| w.starts_with(s)))
}

fn strip_note(text: &str) -> String {
    let text = text.trim();
    let text = text.strip_prefix("$ ").unwrap_or(text).trim();
    if text.ends_with(')')
        && let Some(at) = text.rfind(" (")
    {
        return text[..at].trim().to_string();
    }
    text.to_string()
}

pub fn candidates(item: &str) -> Vec<String> {
    let spans: Vec<String> = TICKS.captures_iter(item).map(|c| strip_note(&c[1])).collect();
    if !spans.is_empty() {
        return spans;
    }
    let text = item.split_once(": ").map(|(_, rest)| rest).unwrap_or(item);
    vec![strip_note(text)]
}

pub fn usable(command: &str, root: &Path) -> Result<(), &'static str> {
    let text = command.trim();
    if text.is_empty() || text.chars().count() > COMMAND_CHARS {
        return Err("it is empty or too long");
    }
    if text.contains(['<', '>', '\n', '`', '…']) || text.contains("...") {
        return Err("it has a placeholder or a redirect");
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.iter().any(|w| w.ends_with(':')) {
        return Err("it reads like a note, not a command");
    }
    argv_of(text)?;
    let program = words.iter().find(|w| !ENV_SET.is_match(w)).ok_or("it has no program")?;
    if !PROGRAM.is_match(program) {
        return Err("it does not start with a program");
    }
    let installed = if program.contains('/') {
        let path = Path::new(program);
        if path.is_absolute() { path.is_file() } else { root.join(path).is_file() }
    } else {
        Path::new(&find_program(program)).is_absolute()
    };
    if !installed {
        return Err("its program is not installed");
    }
    Ok(())
}

fn keep(command: String, root: &Path, found: &mut Vec<String>, skipped: &mut Vec<Value>) {
    if command.is_empty() || found.contains(&command) {
        return;
    }
    match usable(&command, root) {
        Ok(()) if found.len() < MAX_FOUND => found.push(command),
        Ok(()) => {}
        Err(why) if skipped.len() < MAX_FOUND => skipped.push(json!({"command": clip(&command, 120), "why": why})),
        Err(_) => {}
    }
}

pub fn commands_in(text: &str, root: &Path) -> (Vec<String>, Vec<Value>) {
    let mut found = Vec::new();
    let mut skipped = Vec::new();
    let mut section = false;
    let mut fence = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with("```") || line.starts_with("~~~") {
            fence = !fence;
            continue;
        }
        let picks = if fence {
            if !section || line.is_empty() || line.starts_with('#') {
                continue;
            }
            vec![strip_note(line)]
        } else if let Some(head) = HEADING.captures(raw) {
            section = check_heading(&head[1]);
            continue;
        } else if !section {
            continue;
        } else if let Some(bullet) = BULLET.captures(raw) {
            candidates(&bullet[1])
        } else {
            continue;
        };
        for pick in picks {
            keep(pick, root, &mut found, &mut skipped);
        }
    }
    (found, skipped)
}

pub fn setting_key(space_id: &str) -> String {
    format!("{SETTING_PREFIX}{space_id}")
}

pub fn setting_commands(value: Option<&Value>) -> Vec<String> {
    let lines: Vec<String> = match value {
        Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        Some(Value::String(text)) => text.lines().map(str::to_string).collect(),
        _ => Vec::new(),
    };
    let mut out: Vec<String> = Vec::new();
    for line in lines.iter().map(|l| l.trim()).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        if line.chars().count() <= COMMAND_CHARS && !out.iter().any(|o| o == line) && out.len() < MAX_FOUND {
            out.push(line.to_string());
        }
    }
    out
}

fn read_small(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > FILE_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

pub fn typed_plans(root: &Path, commands: &[String], skipped: &mut Vec<Value>) -> Vec<Planned> {
    let mut out = Vec::new();
    for command in commands {
        match typed_plan(root, command) {
            Ok(planned) => out.push(planned),
            Err(why) => skipped.push(json!({"command": clip(command, 120), "why": why})),
        }
    }
    out
}

pub fn space_of<'a>(spaces: &'a [ProjectSpace], root: &str) -> Option<&'a ProjectSpace> {
    let real = std::fs::canonicalize(root).ok();
    spaces
        .iter()
        .filter(|s| !s.folder.is_empty())
        .filter(|s| crate::spaces::folder_in(root, &s.folder) || std::fs::canonicalize(&s.folder).is_ok_and(|f| real.as_ref().is_some_and(|r| r.starts_with(&f))))
        .max_by_key(|s| s.folder.len())
}

pub async fn find_checks(db: &Path, root: &Path, space: Option<&str>) -> Found {
    if let Some(id) = space {
        let saved = nixa_store::open(db).ok().and_then(|conn| nixa_store::settings::get_setting(&conn, &setting_key(id)).ok().flatten());
        let commands = setting_commands(saved.as_ref());
        if !commands.is_empty() {
            let mut skipped = Vec::new();
            let planned = typed_plans(root, &commands, &mut skipped);
            return Found { source: "space settings".into(), from_file: false, planned, skipped, ..Found::default() };
        }
    }
    let mut skipped = Vec::new();
    if let Some(text) = read_small(&root.join(COMMANDS_FILE)) {
        let (commands, left) = commands_in(&text, root);
        skipped.extend(left);
        if !commands.is_empty() {
            let planned = typed_plans(root, &commands, &mut skipped);
            return Found { source: COMMANDS_FILE.into(), from_file: true, planned, skipped, none: String::new() };
        }
    }
    if let Some(rules) = crate::knowledge::rules::read(root).filter(|r| !r.checks.is_empty()) {
        let mut commands = Vec::new();
        for item in &rules.checks {
            for pick in candidates(item) {
                keep(pick, root, &mut commands, &mut skipped);
            }
        }
        if !commands.is_empty() {
            let planned = typed_plans(root, &commands, &mut skipped);
            return Found { source: rules.file.clone(), from_file: true, planned, skipped, none: String::new() };
        }
    }
    let changed = changed_in(root).await.unwrap_or_default();
    let packages = plan_checks(root, &changed);
    let options = RunOptions { packages: Some(packages.packages.clone()), ..RunOptions::default() };
    match plan(root, &changed, &options) {
        Ok(planned) if !packages.packages.is_empty() => Found { source: PACKAGES.into(), from_file: false, planned, skipped, none: String::new() },
        _ => Found {
            source: String::new(),
            from_file: false,
            planned: Vec::new(),
            skipped,
            none: format!("no check commands in the space settings, {COMMANDS_FILE} or docs/nixa.md, and no check, typecheck, lint or test script or Cargo.toml"),
        },
    }
}

pub fn listed(planned: &[Planned]) -> Vec<Value> {
    planned.iter().enumerate().map(|(i, p)| json!({"n": i + 1, "command": shown_command(p)})).collect()
}

pub fn shown_command(p: &Planned) -> String {
    if p.typed { p.script.clone() } else { format!("{} in {}", p.name, p.dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()) }
}

pub fn pick_only(planned: Vec<Planned>, only: &[String]) -> Result<Vec<Planned>, String> {
    let wanted: Vec<&str> = only.iter().map(|o| o.trim()).filter(|o| !o.is_empty()).collect();
    if wanted.is_empty() {
        return Ok(planned);
    }
    let mut out: Vec<Planned> = Vec::new();
    for want in wanted {
        let found = want
            .trim_start_matches('#')
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .and_then(|i| planned.get(i))
            .or_else(|| planned.iter().find(|p| p.script == want || p.name == want || shown_command(p) == want));
        match found {
            Some(p) if !out.contains(p) => out.push(p.clone()),
            Some(_) => {}
            None => {
                let names: Vec<String> = listed(&planned).iter().map(|v| format!("{}. {}", v["n"], v["command"].as_str().unwrap_or_default())).collect();
                return Err(format!("\"{want}\" is not one of the check commands. They are: {}. Nothing was run.", clip(&names.join("; "), 600)));
            }
        }
    }
    Ok(out)
}


static HELD: Mutex<Option<(String, u64)>> = Mutex::new(None);

pub fn approved_key(root: &str) -> String {
    format!("{APPROVED_PREFIX}{root}")
}

fn repo_commands(found: &Found) -> Value {
    json!(found.planned.iter().map(|p| p.script.clone()).collect::<Vec<_>>())
}

fn approved(db: &Path, root: &str, commands: &Value) -> bool {
    nixa_store::open(db).ok().and_then(|conn| nixa_store::settings::get_setting(&conn, &approved_key(root)).ok().flatten()).is_some_and(|saved| saved == *commands)
}

fn ask_repo_checks(app: &App, root: &str, repo: &str, commands: &Value) -> Option<Value> {
    if approved(&app.db_file, root, commands) {
        return None;
    }
    let key = format!("{root}\n{commands}");
    let now = crate::util::now_ms();
    let limit = crate::decision::YES_MS;
    let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, at)) = held.as_ref().filter(|(k, at)| *k == key && now.saturating_sub(*at) <= limit)
        && let Some(yes) = crate::yes_once::chat_yes(app, *at, now, limit, &["checks", "check"])
    {
        *held = None;
        crate::yes_once::use_yes(app, yes, format!("checks {repo}"));
        if let Ok(conn) = nixa_store::open(&app.db_file) {
            let _ = nixa_store::settings::set_setting(&conn, &approved_key(root), commands);
        }
        return None;
    }
    if held.as_ref().is_none_or(|(k, at)| *k != key || now.saturating_sub(*at) > limit) {
        *held = Some((key, now));
        app.info(&format!("Held for you: the check commands from the files of {repo}. Say \"yes checks\" to run them."));
    }
    Some(json!("These check commands come from the repo's own files and were not run before. Show them to the user. A plain yes does not count: the user says \"yes checks\" in chat. Then call this tool again with the same input. Nothing ran."))
}

pub async fn run_checks(app: &App, root: &str, args: &Args) -> Result<Value, String> {
    let repo = base_name(root);
    let spaces = spaces_of(&app.db_file);
    let space = space_of(&spaces, root).map(|s| s.id.clone());
    let locked = locked_text(root, &spaces).is_some();
    if locked {
        crate::modes::note(app, "auto-yes", &format!("run the checks of {repo} in a locked space"), None, Some("Nixa runs read-only checks in a locked space; the lock only stops changes"));
    }
    let found = find_checks(&app.db_file, Path::new(root), space.as_deref()).await;
    if found.from_file && !found.planned.is_empty() {
        let commands = repo_commands(&found);
        if locked {
            return Ok(json!({"repo": repo, "path": root, "ran": false, "source": found.source, "found": listed(&found.planned), "none": "this space is locked, so check commands from the repo's own files never run here. Nothing ran."}));
        }
        if let Some(note) = ask_repo_checks(app, root, &repo, &commands) {
            return Ok(json!({"repo": repo, "path": root, "ran": false, "held": true, "source": found.source, "found": listed(&found.planned), "note": note}));
        }
    }
    let mut out = json!({"repo": repo, "path": root, "ran": false});
    if !found.skipped.is_empty() {
        out["skipped"] = json!(found.skipped);
    }
    if found.planned.is_empty() {
        out["none"] = json!(found.none);
        return Ok(out);
    }
    out["source"] = json!(found.source);
    out["found"] = json!(listed(&found.planned));
    let picked = pick_only(found.planned, &strings(args, "only"))?;
    let allowed = picked;
    let not_run: Vec<String> = allowed.iter().skip(MAX_RUN).map(shown_command).collect();
    let run: Vec<Planned> = allowed.into_iter().take(MAX_RUN).collect();
    if !not_run.is_empty() {
        out["not_run"] = json!(not_run);
        out["note"] = json!(format!("Only {MAX_RUN} check commands run at once. Run again with only (numbers from found) for the rest."));
    }
    if run.is_empty() {
        out["none"] = json!("every picked check command was refused");
        return Ok(out);
    }
    let options = RunOptions { total_ms: Some(seconds_of(args.get("seconds"))), ..RunOptions::default() };
    let outcome = run_plan(Path::new(root), run, &options).await;
    let failed = outcome.results.iter().filter(|r| !r.ok).count();
    let summary = short_checks(&outcome);
    crate::util::log("git", &format!("checks in {root} from {}: {} run, {failed} failed", found.source, outcome.results.len()));
    app.info(&format!("Nixa ran {} in {repo}: {}.", plural(outcome.results.len(), "check"), if failed == 0 { "all passed".to_string() } else { format!("{failed} failed") }));
    out["ran"] = json!(true);
    out["failed"] = json!(outcome.failed);
    out["passed"] = json!(outcome.results.len() - failed);
    out["failed_count"] = json!(failed);
    out["summary"] = json!(summary);
    out["results"] = json!(outcome.results);
    Ok(out)
}
