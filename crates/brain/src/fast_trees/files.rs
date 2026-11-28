use super::rules::{Compiled, FastDecision, Input, SelfCheck, compile, rules_of, seed, seed_value};
use crate::util::atomic_write;
use regex::Regex;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

pub const FILE: &str = "fast-decisions.json";
pub const TREES_DIR: &str = "decision-trees";

static TREE_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z0-9][a-z0-9-]{0,59}$").unwrap());

pub fn good_tree_name(name: &str) -> bool {
    TREE_NAME.is_match(name)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reload {
    pub changed: bool,
    pub problem: Option<String>,
}

pub struct Snapshot {
    pub raw: String,
    pub source: String,
    pub trees: Vec<(String, PathBuf, String)>,
}

fn pretty(value: &Value) -> String {
    format!("{}\n", serde_json::to_string_pretty(value).unwrap_or_default())
}

fn private_dir(dir: &Path) -> Result<(), String> {
    nixa_protocol::paths::ensure_private_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))
}

pub fn tree_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(list) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<PathBuf> = list.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    files.sort();
    files
}

fn seed_trees(base: &Value, dir: &Path) -> Result<(), String> {
    let embedded = match base.get("trees") {
        Some(Value::Object(map)) => map.clone(),
        _ => seed_value().get("trees").and_then(Value::as_object).cloned().unwrap_or_default(),
    };
    if embedded.is_empty() {
        return Ok(());
    }
    private_dir(dir)?;
    for (name, chart) in embedded {
        let target = dir.join(format!("{name}.json"));
        if good_tree_name(&name) && !target.exists() {
            atomic_write(&target, &pretty(&chart)).map_err(|e| format!("{}: {e}", target.display()))?;
        }
    }
    Ok(())
}

pub fn ensure(file: &Path, dir: &Path) -> Result<(), String> {
    if !file.exists() {
        let rules = seed_value();
        atomic_write(file, &pretty(&rules)).map_err(|e| format!("{}: {e}", file.display()))?;
        return seed_trees(&rules, dir);
    }
    if let Ok(text) = std::fs::read_to_string(file)
        && let Ok(base) = serde_json::from_str::<Value>(&text)
    {
        seed_trees(&base, dir)?;
    }
    for path in std::iter::once(file.to_path_buf()).chain(tree_files(dir)) {
        rename_capability(&path)?;
    }
    Ok(())
}

const OLD_CAPABILITY: &str = "\"cheap-drone\"";
const NEW_CAPABILITY: &str = "\"cheap\"";

fn rename_capability(path: &Path) -> Result<(), String> {
    match std::fs::read_to_string(path) {
        Ok(text) if text.contains(OLD_CAPABILITY) => {
            atomic_write(path, &text.replace(OLD_CAPABILITY, NEW_CAPABILITY)).map_err(|e| format!("{}: {e}", path.display()))
        }
        _ => Ok(()),
    }
}

pub fn snapshot(file: &Path, dir: Option<&Path>) -> Result<Snapshot, String> {
    let source = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let mut trees = Vec::new();
    if let Some(dir) = dir {
        for path in tree_files(dir) {
            let entry = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let name = entry.trim_end_matches(".json").to_string();
            if !good_tree_name(&name) {
                return Err(format!("Invalid decision tree file name {entry}."));
            }
            let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            trees.push((name, path, text));
        }
    }
    let pairs: Vec<(&String, &String)> = trees.iter().map(|(n, _, t)| (n, t)).collect();
    let raw = serde_json::to_string(&(&source, pairs)).unwrap_or_default();
    Ok(Snapshot { raw, source, trees })
}

pub fn combined(snap: &Snapshot) -> Result<Value, String> {
    let mut base: Value = serde_json::from_str(&snap.source).map_err(|e| format!("Invalid fast-decisions.json: {e}"))?;
    let Some(root) = base.as_object_mut() else { return Err("Invalid fast-decisions.json: root must be a JSON object.".into()) };
    let mut trees: Map<String, Value> = root.get("trees").and_then(Value::as_object).cloned().unwrap_or_default();
    for (name, _, text) in &snap.trees {
        let chart: Value = serde_json::from_str(text).map_err(|e| format!("Invalid decision tree {name}.json: {e}"))?;
        trees.insert(name.clone(), chart);
    }
    root.insert("trees".into(), Value::Object(trees));
    Ok(base)
}

pub fn compile_value(value: &Value) -> Result<Compiled, String> {
    compile(&rules_of(value)?)
}

pub struct FastTrees {
    pub file: PathBuf,
    pub trees_dir: PathBuf,
    pub problem: Option<String>,
    compiled: Compiled,
    last_attempt: String,
}

impl FastTrees {
    pub fn open(dir: &Path) -> FastTrees {
        FastTrees::at(&dir.join(FILE), &dir.join(TREES_DIR))
    }

    pub fn at(file: &Path, trees_dir: &Path) -> FastTrees {
        let built_in = || compile(&seed()).expect("the built-in fast decision rules compile");
        let mut problem = ensure(file, trees_dir).err();
        let first = snapshot(file, Some(trees_dir)).and_then(|s| combined(&s)).and_then(|v| compile_value(&v));
        let compiled = match first {
            Ok(c) => c,
            Err(_) => {
                let backup = PathBuf::from(format!("{}.bak", file.display()));
                std::fs::read_to_string(&backup)
                    .ok()
                    .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                    .and_then(|v| compile_value(&v).ok())
                    .unwrap_or_else(built_in)
            }
        };
        let mut trees = FastTrees { file: file.to_path_buf(), trees_dir: trees_dir.to_path_buf(), problem: None, compiled, last_attempt: String::new() };
        let first = trees.reload();
        problem = first.problem.or(problem);
        trees.problem = problem;
        trees
    }

    pub fn reload(&mut self) -> Reload {
        let snap = match snapshot(&self.file, Some(&self.trees_dir)) {
            Ok(s) => s,
            Err(e) => {
                if self.last_attempt == e {
                    return Reload { changed: false, problem: None };
                }
                self.last_attempt = e.clone();
                let problem = kept(&e);
                self.problem = Some(problem.clone());
                return Reload { changed: false, problem: Some(problem) };
            }
        };
        if snap.raw == self.last_attempt {
            return Reload { changed: false, problem: None };
        }
        self.last_attempt = snap.raw.clone();
        match combined(&snap).and_then(|v| compile_value(&v)) {
            Ok(compiled) => {
                self.compiled = compiled;
                self.problem = None;
                Reload { changed: true, problem: None }
            }
            Err(e) => {
                let problem = kept(&e);
                self.problem = Some(problem.clone());
                Reload { changed: false, problem: Some(problem) }
            }
        }
    }

    pub fn rules(&self) -> Result<Value, String> {
        ensure(&self.file, &self.trees_dir)?;
        combined(&snapshot(&self.file, Some(&self.trees_dir))?)
    }

    pub fn update(&mut self, change: impl FnOnce(&mut Value)) -> Result<Value, String> {
        let mut value = self.rules()?;
        change(&mut value);
        let compiled = compile_value(&value)?;
        atomic_write(&self.file, &pretty(&value)).map_err(|e| format!("{}: {e}", self.file.display()))?;
        private_dir(&self.trees_dir)?;
        if let Some(trees) = value.get("trees").and_then(Value::as_object) {
            for (name, chart) in trees {
                if good_tree_name(name) {
                    let target = self.trees_dir.join(format!("{name}.json"));
                    atomic_write(&target, &pretty(chart)).map_err(|e| format!("{}: {e}", target.display()))?;
                }
            }
        }
        self.last_attempt = snapshot(&self.file, Some(&self.trees_dir))?.raw;
        self.compiled = compiled;
        self.problem = None;
        Ok(value)
    }

    pub fn decide(&self, text: &str, input: &Input) -> Result<FastDecision, String> {
        self.compiled.decide(text, input)
    }

    pub fn self_check(&self) -> SelfCheck {
        self.compiled.self_check()
    }

    pub fn compiled(&self) -> &Compiled {
        &self.compiled
    }

    pub fn tree_files(&self) -> Vec<PathBuf> {
        tree_files(&self.trees_dir)
    }
}

fn kept(error: &str) -> String {
    format!("Fast decisions kept the last good rules: {error}")
}
