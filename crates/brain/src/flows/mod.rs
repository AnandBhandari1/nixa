pub mod mermaid;
#[cfg(test)]
mod tests;

use crate::app::{App, lock};
use crate::drones::{DroneStep, DroneTree};
use crate::tools::{Args, ToolResult, schema, spec, text};
use crate::trees::{RuleBox, RuleTree};
use crate::turns::ToolSpec;
use crate::util::now_ms;
use crate::workflows::model::Workflow;
use mermaid::Parsed;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub const NAME: &str = "nixa_flowman";
pub const ACTIONS: [&str; 8] = ["list", "show", "export", "open", "apply", "blueprint", "active", "run"];
const RUN_SUBS: [&str; 7] = ["status", "active", "list", "read", "validate", "write", "render"];
const FLOWMAN_WAIT: Duration = Duration::from_secs(60);
const WHO: &str = "FlowMan";

pub fn specs() -> Vec<ToolSpec> {
    vec![spec(
        NAME,
        "Show and edit Nixa's flows as Mermaid in the FlowMan app. Flows: tree/NAME (decision trees), drones (the drone tree), workflow/NAME. list shows them. show prints the Mermaid. export writes the .mmd files. open writes one and opens it in FlowMan. apply reads the saved .mmd back into Nixa (trees: box text, new boxes, removed boxes and arrows; drones and workflows: box text only). blueprint lists or opens the .mmd code diagrams in DIR/docs/diagrams. active reads the diagram open in FlowMan now. run runs one flowman CLI command (status, active, list, read, validate, write, render).",
        schema(
            json!({
                "action": {"type": "string", "enum": ACTIONS},
                "flow": {"type": "string", "description": "tree/NAME, drones or workflow/NAME."},
                "dir": {"type": "string", "description": "Project folder for blueprint."},
                "file": {"type": "string", "description": "Diagram file name for blueprint, for example 01-architecture.mmd."},
                "args": {"type": "array", "items": {"type": "string"}, "description": "flowman CLI words for run, for example [\"validate\", \"/path/a.mmd\"]."},
                "source": {"type": "string", "description": "Mermaid text for run write (sent as --from -)."}
            }),
            &["action"],
        ),
    )]
}

pub fn flows_dir(app: &App) -> PathBuf {
    app.dir.join("flows")
}

#[derive(Debug, Clone, PartialEq)]
pub enum Flow {
    Tree(String),
    Drones,
    Workflow(String),
}

impl Flow {
    pub fn parse(raw: &str) -> Result<Flow, String> {
        let raw = raw.trim();
        if raw == "drones" || raw == "drone" {
            return Ok(Flow::Drones);
        }
        match raw.split_once('/') {
            Some(("tree", name)) if !name.is_empty() => Ok(Flow::Tree(name.to_string())),
            Some(("workflow", name)) if !name.is_empty() => Ok(Flow::Workflow(name.to_string())),
            _ => Err(format!("Unknown flow {raw}. Use tree/NAME, drones or workflow/NAME. Run list to see them.")),
        }
    }

    pub fn key(&self) -> String {
        match self {
            Flow::Tree(n) => format!("tree/{n}"),
            Flow::Drones => "drones".into(),
            Flow::Workflow(n) => format!("workflow/{n}"),
        }
    }

    pub fn file_name(&self) -> String {
        match self {
            Flow::Tree(n) => format!("tree-{n}.mmd"),
            Flow::Drones => "drones.mmd".into(),
            Flow::Workflow(n) => format!("workflow-{n}.mmd"),
        }
    }
}

fn safe_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn workflows(app: &App) -> Result<Vec<(String, String)>, String> {
    let conn = crate::workflows::open_db(app)?;
    let saved = nixa_store::workflows::list_workflows(&conn).map_err(|e| format!("{e:#}"))?;
    Ok(saved.into_iter().map(|w| (w.name, w.body)).collect())
}

fn workflow(app: &App, name: &str) -> Result<Workflow, String> {
    let conn = crate::workflows::open_db(app)?;
    let saved = nixa_store::workflows::get_workflow(&conn, name).map_err(|e| format!("{e:#}"))?.ok_or_else(|| format!("No workflow named {name}."))?;
    crate::workflows::model::parse_saved(name, &saved.body)
}

pub fn all_flows(app: &App) -> Vec<Flow> {
    let mut out: Vec<Flow> = lock(&app.trees).names().into_iter().map(Flow::Tree).collect();
    out.push(Flow::Drones);
    out.extend(workflows(app).unwrap_or_default().into_iter().map(|(n, _)| Flow::Workflow(n)));
    out
}

fn drone_tree(app: &App) -> DroneTree {
    let mut store = lock(&app.drones.store);
    store.load();
    store.tree.clone()
}

pub fn mmd_of(app: &App, flow: &Flow) -> Result<String, String> {
    match flow {
        Flow::Tree(name) => Ok(mermaid::tree_mmd(&lock(&app.trees).read(name)?)),
        Flow::Drones => Ok(mermaid::drone_mmd(&drone_tree(app))),
        Flow::Workflow(name) => Ok(mermaid::workflow_mmd(&workflow(app, name)?)),
    }
}

pub fn flowman_bin() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(Path::new(&home).join(".local/bin"));
    }
    dirs.push(PathBuf::from("/opt/homebrew/bin"));
    dirs.push(PathBuf::from("/usr/local/bin"));
    dirs.into_iter().map(|d| d.join("flowman")).find(|p| p.is_file())
}

const NO_FLOWMAN: &str = "The flowman command was not found. Install FlowMan and its CLI, then try again.";

pub async fn flowman(args: &[String], input: Option<&str>) -> Result<String, String> {
    use tokio::io::AsyncWriteExt;
    let bin = flowman_bin().ok_or(NO_FLOWMAN)?;
    let mut cmd = tokio::process::Command::new(bin);
    cmd.args(args).stdin(if input.is_some() { std::process::Stdio::piped() } else { std::process::Stdio::null() }).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| format!("flowman did not start: {e}"))?;
    if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin.write_all(text.as_bytes()).await.map_err(|e| format!("flowman input failed: {e}"))?;
    }
    let out = tokio::time::timeout(FLOWMAN_WAIT, child.wait_with_output()).await.map_err(|_| "flowman took more than 60 seconds.".to_string())?.map_err(|e| e.to_string())?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)).trim().to_string();
    if out.status.success() { Ok(text) } else { Err(if text.is_empty() { "flowman failed.".into() } else { text }) }
}

async fn write_mmd(app: &App, flow: &Flow) -> Result<PathBuf, String> {
    let text = mmd_of(app, flow)?;
    let dir = flows_dir(app);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(flow.file_name());
    if file.exists() && flowman_bin().is_some() {
        let path = file.to_string_lossy().to_string();
        if flowman(&["write".into(), path, "--from".into(), "-".into()], Some(&text)).await.is_ok() {
            return Ok(file);
        }
    }
    std::fs::write(&file, text).map_err(|e| e.to_string())?;
    Ok(file)
}

fn set_drone_label(step: &mut DroneStep, value: String) {
    match step {
        DroneStep::Report { label: None, text, .. } => *text = value,
        DroneStep::If { label, .. }
        | DroneStep::Switch { label, .. }
        | DroneStep::All { label, .. }
        | DroneStep::Any { label, .. }
        | DroneStep::Wait { label, .. }
        | DroneStep::Report { label, .. } => *label = Some(value),
    }
}

fn arrow_set(p: &Parsed) -> BTreeSet<(String, String, String)> {
    p.edges.iter().cloned().collect()
}

fn same_shape(now: &Parsed, edited: &Parsed, what: &str) -> Result<(), String> {
    let new_boxes: Vec<&String> = edited.nodes.keys().filter(|k| !now.nodes.contains_key(*k)).collect();
    let gone: Vec<&String> = now.nodes.keys().filter(|k| !edited.nodes.contains_key(*k)).collect();
    if !new_boxes.is_empty() || !gone.is_empty() || arrow_set(now) != arrow_set(edited) {
        return Err(format!("For {what}, FlowMan can change box text only. Boxes or arrows were added, removed or moved. Change those with {}, then export again.", if what == "drones" { "nixa drones" } else { "the Workflows window" }));
    }
    Ok(())
}

pub fn apply_tree(mut tree: RuleTree, edited: &Parsed) -> Result<(RuleTree, Vec<String>), String> {
    let mut notes = Vec::new();
    if !edited.nodes.contains_key(&tree.start) {
        return Err(format!("The start box {} is missing. Keep it.", tree.start));
    }
    let gone: Vec<String> = tree.boxes.keys().filter(|k| !edited.nodes.contains_key(*k)).cloned().collect();
    for id in gone {
        tree.boxes.remove(&id);
        notes.push(format!("removed box {id}"));
    }
    for (id, label) in &edited.nodes {
        match tree.boxes.get_mut(id) {
            Some(b) if b.text != *label => {
                b.text = label.clone();
                notes.push(format!("new text for {id}"));
            }
            Some(_) => {}
            None => {
                tree.boxes.insert(id.clone(), RuleBox { text: label.clone(), next: None, check: None, report: false });
                notes.push(format!("added box {id}"));
            }
        }
    }
    let mut links: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (from, word, to) in &edited.edges {
        if word.is_empty() {
            return Err(format!("The arrow from {from} to {to} has no word. Give every arrow a word, for example Yes or No."));
        }
        if links.entry(from.clone()).or_default().insert(word.clone(), to.clone()).is_some() {
            return Err(format!("Box {from} has two arrows with the word {word}."));
        }
    }
    for (id, b) in tree.boxes.iter_mut() {
        let next = links.remove(id);
        if b.next != next {
            b.next = next;
            notes.push(format!("new arrows from {id}"));
        }
    }
    Ok((tree, notes))
}

pub fn apply_drones(mut tree: DroneTree, edited: &Parsed) -> Result<(DroneTree, Vec<(String, DroneStep)>), String> {
    let now = mermaid::parse(&mermaid::drone_mmd(&tree))?;
    same_shape(&now, edited, "drones")?;
    let mut changed = Vec::new();
    for (id, label) in &edited.nodes {
        if let Some(step) = tree.steps.get_mut(id)
            && mermaid::drone_text(step) != *label
        {
            set_drone_label(step, label.clone());
            changed.push((id.clone(), step.clone()));
        }
    }
    Ok((tree, changed))
}

pub fn apply_workflow(mut flow: Workflow, edited: &Parsed) -> Result<(Workflow, Vec<String>), String> {
    let now = mermaid::parse(&mermaid::workflow_mmd(&flow))?;
    same_shape(&now, edited, "workflows")?;
    let mut notes = Vec::new();
    for (id, label) in &edited.nodes {
        if let Some(n) = flow.nodes.get_mut(id)
            && n.title != *label
        {
            n.title = label.clone();
            notes.push(format!("new title for {id}"));
        }
    }
    Ok((flow, notes))
}

fn apply(app: &Arc<App>, flow: &Flow, who: &str) -> Result<String, String> {
    let file = flows_dir(app).join(flow.file_name());
    let source = std::fs::read_to_string(&file).map_err(|_| format!("{} does not exist. Run export or open first.", file.display()))?;
    let edited = mermaid::parse(&source)?;
    let notes = match flow {
        Flow::Tree(name) => {
            let tree = lock(&app.trees).read(name)?;
            let (next, notes) = apply_tree(tree, &edited)?;
            if !notes.is_empty() {
                lock(&app.trees).save(&next, who, "flowman", None)?;
                crate::tree_api::changed(app, name, &format!("by {who}"));
            }
            notes
        }
        Flow::Drones => {
            let (_, steps) = apply_drones(drone_tree(app), &edited)?;
            let now = now_ms();
            let mut notes = Vec::new();
            for (id, step) in steps {
                lock(&app.drones.store).change_step(&id, step, who, now)?;
                notes.push(format!("new text for {id}"));
            }
            if !notes.is_empty() {
                crate::drones::broadcast_changed(app);
            }
            notes
        }
        Flow::Workflow(name) => {
            let (next, notes) = apply_workflow(workflow(app, name)?, &edited)?;
            if !notes.is_empty() {
                let raw = crate::workflows::model::to_json(&next);
                crate::workflows::model::check_workflow(&raw, Some(name))?;
                let conn = crate::workflows::open_db(app)?;
                nixa_store::workflows::put_workflow(&conn, name, &raw.to_string()).map_err(|e| format!("{e:#}"))?;
                crate::workflows::changed(app, name);
            }
            notes
        }
    };
    Ok(if notes.is_empty() { format!("{} has no changes to apply.", flow.key()) } else { format!("Applied {} to Nixa: {}.", flow.key(), notes.join(", ")) })
}

fn blueprint_dir(dir: &str) -> PathBuf {
    Path::new(dir).join("docs").join("diagrams")
}

async fn blueprint(args: &Args) -> ToolResult {
    let dir = text(args, "dir").ok_or("Give dir, the project folder.")?;
    let folder = blueprint_dir(&dir);
    let mut files: Vec<String> = std::fs::read_dir(&folder)
        .map_err(|_| format!("{} does not exist. Agents keep code diagrams there as NN-topic.mmd.", folder.display()))?
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| n.ends_with(".mmd"))
        .collect();
    files.sort();
    match text(args, "file") {
        None => Ok(if files.is_empty() { format!("No .mmd diagrams in {}.", folder.display()) } else { format!("Diagrams in {}:\n{}", folder.display(), files.join("\n")) }),
        Some(file) => {
            if !files.contains(&file) {
                return Err(format!("No diagram {file} in {}.", folder.display()));
            }
            let path = folder.join(&file).to_string_lossy().to_string();
            flowman(&["open".into(), path.clone()], None).await?;
            Ok(format!("Opened {path} in FlowMan."))
        }
    }
}

fn flow_dirs(app: &App) -> Vec<PathBuf> {
    use crate::project_tools::policy;
    let home = policy::home().and_then(|h| std::fs::canonicalize(h).ok());
    let mut dirs: Vec<PathBuf> = Vec::new();
    for space in crate::project_tools::find::spaces_of(&app.db_file) {
        dirs.push(PathBuf::from(space.folder));
        dirs.extend(space.repos.into_iter().map(PathBuf::from));
    }
    let mut out: Vec<PathBuf> = dirs
        .into_iter()
        .filter(|d| d.is_absolute())
        .filter_map(|d| std::fs::canonicalize(d).ok())
        .filter(|d| d.parent().is_some() && !home.as_ref().is_some_and(|h| h.starts_with(d)) && !policy::protected_path(d))
        .collect();
    out.extend(std::fs::canonicalize(flows_dir(app)).ok());
    out
}

pub fn allowed_path(raw: &str, dirs: &[PathBuf], exts: &[&str]) -> Result<PathBuf, String> {
    let refused = || format!("{raw}: FlowMan here works only on .mmd files inside an open space or the Nixa flows folder.");
    let path = PathBuf::from(crate::project_tools::find::expand_home(raw.trim()));
    if !path.is_absolute() || path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(refused());
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if !exts.is_empty() && !exts.contains(&ext.as_str()) {
        return Err(refused());
    }
    let real = match std::fs::canonicalize(&path) {
        Ok(real) => real,
        Err(_) if std::fs::symlink_metadata(&path).is_ok() => return Err(refused()),
        Err(_) => {
            let name = path.file_name().ok_or_else(refused)?;
            std::fs::canonicalize(path.parent().ok_or_else(refused)?).map_err(|_| refused())?.join(name)
        }
    };
    let real_ext = real.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if !exts.is_empty() && !exts.contains(&real_ext.as_str()) {
        return Err(refused());
    }
    use crate::project_tools::policy::{secret_part, sensitive_file};
    let Some(dir) = dirs.iter().find(|d| real.starts_with(d)) else { return Err(refused()) };
    let inside = real.strip_prefix(dir).unwrap_or(Path::new(""));
    if sensitive_file(&path) || sensitive_file(&real) || inside.components().any(|c| secret_part(&c.as_os_str().to_string_lossy())) {
        return Err(refused());
    }
    Ok(real)
}

pub fn check_run_words(words: &[String], dirs: &[PathBuf]) -> Result<Vec<String>, String> {
    let sub = words.first().map(String::as_str).unwrap_or_default();
    let mut out = vec![sub.to_string()];
    let mut rest = words.iter().skip(1);
    let shown = |p: PathBuf| p.display().to_string();
    while let Some(word) = rest.next() {
        match word.as_str() {
            "--from" => {
                if rest.next().map(String::as_str) != Some("-") {
                    return Err("write takes the Mermaid text in source (--from -), not from a file.".into());
                }
                out.extend(["--from".to_string(), "-".to_string()]);
            }
            "--out" if sub == "render" => {
                let file = allowed_path(rest.next().ok_or("Give the --out file.")?, dirs, &["svg", "png"])?;
                out.extend(["--out".to_string(), shown(file)]);
            }
            "--root" if sub == "list" => {
                let dir = allowed_path(rest.next().ok_or("Give the --root folder.")?, dirs, &[])?;
                if !dir.is_dir() {
                    return Err(format!("{} is not a folder.", dir.display()));
                }
                out.extend(["--root".to_string(), shown(dir)]);
            }
            flag if flag.starts_with('-') => return Err(format!("{flag} is not allowed here.")),
            file if matches!(sub, "read" | "validate" | "write" | "render") => {
                out.push(shown(allowed_path(file, dirs, &["mmd"])?));
            }
            other => return Err(format!("{sub} takes no {other} here.")),
        }
    }
    if sub == "list" && !out.iter().any(|w| w == "--root") {
        return Err("list needs --root, a folder inside an open space or the Nixa flows folder.".into());
    }
    Ok(out)
}

async fn run(app: &App, args: &Args) -> ToolResult {
    let words: Vec<String> = args.get("args").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    let sub = words.first().cloned().ok_or("Give args, for example [\"status\"].")?;
    if !RUN_SUBS.contains(&sub.as_str()) {
        return Err(format!("run allows only: {}. Use open or blueprint to open a window.", RUN_SUBS.join(", ")));
    }
    let source = text(args, "source");
    let mut words = check_run_words(&words, &flow_dirs(app))?;
    if sub == "write" && source.is_some() && !words.iter().any(|w| w == "--from") {
        words.extend(["--from".to_string(), "-".to_string()]);
    }
    flowman(&words, source.as_deref()).await
}

pub async fn run_action(app: &Arc<App>, action: &str, args: &Args, who: &str) -> ToolResult {
    let flow = || text(args, "flow").ok_or_else(|| "Give flow: tree/NAME, drones or workflow/NAME.".to_string()).and_then(|f| Flow::parse(&f)).and_then(|f| match &f {
        Flow::Tree(n) | Flow::Workflow(n) if !safe_name(n) => Err(format!("Bad flow name {n}.")),
        _ => Ok(f),
    });
    match action {
        "list" => {
            let lines: Vec<String> = all_flows(app).iter().map(|f| format!("{}  ({})", f.key(), flows_dir(app).join(f.file_name()).display())).collect();
            Ok(format!("Nixa flows for FlowMan:\n{}\nFlowMan CLI: {}", lines.join("\n"), flowman_bin().map(|p| p.display().to_string()).unwrap_or_else(|| "not found".into())))
        }
        "show" => mmd_of(app, &flow()?),
        "export" => {
            let list = match text(args, "flow") {
                Some(_) => vec![flow()?],
                None => all_flows(app),
            };
            let mut done = Vec::new();
            for f in &list {
                done.push(write_mmd(app, f).await?.display().to_string());
            }
            Ok(format!("Wrote {} Mermaid files:\n{}", done.len(), done.join("\n")))
        }
        "open" => {
            let file = write_mmd(app, &flow()?).await?;
            flowman(&["open".into(), file.to_string_lossy().to_string()], None).await?;
            Ok(format!("Opened {} in FlowMan. Edit and save there, then run apply with the same flow.", file.display()))
        }
        "apply" => apply(app, &flow()?, who),
        "blueprint" => blueprint(args).await,
        "active" => flowman(&["active".into()], None).await,
        "run" => run(app, args).await,
        other => Err(format!("Unknown action {other}. Use one of: {}.", ACTIONS.join(", "))),
    }
}

pub async fn call(app: &Arc<App>, args: &Args) -> ToolResult {
    let action = text(args, "action").unwrap_or_else(|| "list".into());
    let who = format!("{} brain via {WHO}", app.name());
    run_action(app, &action, args, &who).await
}
