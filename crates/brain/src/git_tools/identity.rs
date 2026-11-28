use super::run::{READ_MS, git, git_args};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const DEFAULT_HOST: &str = "github.com";
pub const MAX_REPOS: usize = 20;
pub const PROFILE_KEY: &str = "nixa.profile";
pub const GIT_FOLDER: &str = "git";
pub const PROFILES_FILE: &str = "profiles.json";
pub const HELPER_NAME: &str = "git-credential-nixa";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub label: String,
    pub name: String,
    pub email: String,
    pub accounts: BTreeMap<String, String>,
    pub folders: Vec<String>,
    pub owners: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct RepoConfig {
    pub root: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub profile: Option<String>,
    pub helpers: Vec<String>,
    pub username: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Identity {
    pub host: String,
    pub repos: Vec<RepoConfig>,
    pub profile: Option<Profile>,
    pub account: Option<String>,
    pub repaired: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Prep {
    pub folder: String,
    pub profiles: Vec<Profile>,
    pub helper: String,
    pub host: String,
    pub repair: bool,
    pub home: Option<PathBuf>,
}

pub fn git_dir(data: &Path) -> PathBuf {
    data.join(GIT_FOLDER)
}

pub fn profiles_path(data: &Path) -> PathBuf {
    git_dir(data).join(PROFILES_FILE)
}

pub fn helper_path(data: &Path) -> PathBuf {
    git_dir(data).join("bin").join(HELPER_NAME)
}

fn text(value: Option<&Value>) -> String {
    value.and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string()
}

fn list(value: Option<&Value>) -> Vec<String> {
    value.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()).unwrap_or_default()
}

pub fn parse_profiles(source: &str) -> Vec<Profile> {
    let Ok(raw) = serde_json::from_str::<Value>(source) else { return Vec::new() };
    let items = raw.get("profiles").and_then(Value::as_array).or_else(|| raw.as_array()).cloned().unwrap_or_default();
    let mut out = Vec::new();
    for item in items {
        if !item.is_object() {
            continue;
        }
        let id = text(item.get("id"));
        if id.is_empty() {
            continue;
        }
        let mut accounts = BTreeMap::new();
        if let Some(map) = item.get("accounts").and_then(Value::as_object) {
            for (host, user) in map {
                let user = user.as_str().map(str::trim).unwrap_or_default();
                if !host.trim().is_empty() && !user.is_empty() {
                    accounts.insert(host.trim().to_lowercase(), user.to_string());
                }
            }
        }
        let label = text(item.get("label"));
        out.push(Profile { label: if label.is_empty() { id.clone() } else { label }, id, name: text(item.get("name")), email: text(item.get("email")), accounts, folders: list(item.get("folders")), owners: list(item.get("owners")) });
    }
    out
}

pub fn read_profiles(path: &Path) -> Vec<Profile> {
    std::fs::read_to_string(path).map(|s| parse_profiles(&s)).unwrap_or_default()
}

fn same(a: &str, b: &str) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => Path::new(a) == Path::new(b),
    }
}

pub async fn git_repos(folder: &str) -> Vec<String> {
    let top = git(folder, &["rev-parse", "--show-toplevel"], READ_MS).await;
    if top.ok() && same(top.stdout.trim(), folder) {
        return vec![folder.to_string()];
    }
    let mut names: Vec<String> = match std::fs::read_dir(folder) {
        Ok(read) => read
            .filter_map(|e| e.ok())
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.') && e.file_type().map(|t| t.is_dir() || t.is_symlink()).unwrap_or(false))
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect(),
        Err(_) => return Vec::new(),
    };
    names.sort();
    let mut out = Vec::new();
    for name in names {
        let child = Path::new(folder).join(&name).display().to_string();
        if !Path::new(&child).join(".git").exists() {
            continue;
        }
        let found = git(&child, &["rev-parse", "--show-toplevel"], READ_MS).await;
        if found.ok() && same(found.stdout.trim(), &child) {
            out.push(child);
        }
        if out.len() >= MAX_REPOS {
            break;
        }
    }
    out
}

pub fn parse_config(root: &str, output: &str, host: &str) -> RepoConfig {
    let mut config = RepoConfig { root: root.to_string(), ..RepoConfig::default() };
    let base = format!("credential.https://{host}.");
    for entry in output.split('\0').filter(|e| !e.is_empty()) {
        let (key, value) = entry.split_once('\n').unwrap_or((entry, ""));
        let key = key.to_lowercase();
        if key == "user.name" {
            config.name = Some(value.to_string());
        } else if key == "user.email" {
            config.email = Some(value.to_string());
        } else if key == PROFILE_KEY {
            config.profile = Some(value.to_string());
        } else if key == format!("{base}helper") {
            config.helpers.push(value.to_string());
        } else if key == format!("{base}username") {
            config.username = Some(value.to_string());
        }
    }
    config
}

pub async fn read_repo_config(root: &str, host: &str) -> RepoConfig {
    let out = git(root, &["config", "--local", "--list", "-z"], READ_MS).await;
    parse_config(root, if out.ok() { &out.stdout } else { "" }, host)
}

fn expand(path: &str, home: Option<&Path>) -> PathBuf {
    match home {
        Some(h) if path == "~" => h.to_path_buf(),
        Some(h) if path.starts_with("~/") => h.join(&path[2..]),
        _ => PathBuf::from(path),
    }
}

fn in_folder(root: &str, folder: &str, home: Option<&Path>) -> bool {
    let base = expand(folder.trim_end_matches('/'), home);
    Path::new(root).starts_with(&base)
}

pub fn pick_profile<'a>(profiles: &'a [Profile], config: &RepoConfig, home: Option<&Path>) -> Option<&'a Profile> {
    if let Some(id) = &config.profile
        && let Some(own) = profiles.iter().find(|p| &p.id == id)
    {
        return Some(own);
    }
    profiles.iter().find(|p| p.folders.iter().any(|f| in_folder(&config.root, f, home)))
}

fn words(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

pub fn helper_value(helper: &str) -> String {
    format!("!'{}'", helper.replace('\'', "'\\''"))
}

pub fn repair_plan(config: &RepoConfig, profile: &Profile, helper: &str, host: &str, helper_there: bool) -> Vec<Vec<String>> {
    let helper = helper_value(helper);
    let helper = helper.as_str();
    let mut plan = Vec::new();
    if !profile.name.is_empty() && config.name.as_deref() != Some(profile.name.as_str()) {
        plan.push(words(&["config", "--local", "user.name", &profile.name]));
    }
    if !profile.email.is_empty() && config.email.as_deref() != Some(profile.email.as_str()) {
        plan.push(words(&["config", "--local", "user.email", &profile.email]));
    }
    if config.profile.as_deref() != Some(profile.id.as_str()) {
        plan.push(words(&["config", "--local", PROFILE_KEY, &profile.id]));
    }
    let Some(account) = profile.accounts.get(host) else { return plan };
    let base = format!("credential.https://{host}");
    let reset = config.helpers.iter().position(|h| h.is_empty());
    let mine = config.helpers.iter().position(|h| h == helper);
    let bound = matches!((reset, mine), (Some(r), Some(m)) if m > r) && config.helpers.iter().all(|h| h.is_empty() || h == helper) && helper_there;
    if !bound {
        let key = format!("{base}.helper");
        plan.push(words(&["config", "--local", "--unset-all", &key]));
        plan.push(words(&["config", "--local", "--add", &key, ""]));
        plan.push(words(&["config", "--local", "--add", &key, helper]));
    }
    if config.username.as_deref() != Some(account.as_str()) {
        plan.push(words(&["config", "--local", &format!("{base}.username"), account]));
    }
    plan
}

pub async fn apply_plan(root: &str, plan: &[Vec<String>]) -> Result<(), String> {
    for args in plan {
        let out = git_args(root, args, READ_MS).await;
        let unset_none = args.iter().any(|a| a == "--unset-all") && out.code == 5;
        if !out.ok() && !unset_none {
            let why = out.stderr.trim();
            return Err(if why.is_empty() { format!("git {} failed", args.join(" ")) } else { why.to_string() });
        }
    }
    Ok(())
}

fn short_name(root: &str) -> String {
    root.trim_end_matches('/').rsplit('/').next().unwrap_or(root).to_string()
}

pub async fn prepare(input: &Prep) -> Identity {
    let host = if input.host.is_empty() { DEFAULT_HOST.to_string() } else { input.host.clone() };
    let mut out = Identity { host: host.clone(), ..Identity::default() };
    if input.profiles.is_empty() {
        return out;
    }
    let roots = git_repos(&input.folder).await;
    let helper_there = Path::new(&input.helper).exists();
    let mut picked: Vec<Profile> = Vec::new();
    for root in roots {
        let config = read_repo_config(&root, &host).await;
        out.repos.push(config.clone());
        let Some(profile) = pick_profile(&input.profiles, &config, input.home.as_deref()).cloned() else {
            out.notes.push(format!("{} has no git profile. Pick one in the Git panel.", short_name(&root)));
            continue;
        };
        picked.push(profile.clone());
        let plan = repair_plan(&config, &profile, &input.helper, &host, helper_there);
        if plan.is_empty() {
            continue;
        }
        if !input.repair {
            out.notes.push(format!("{}: the git identity is not set right ({}).", short_name(&root), super::policy::plural(plan.len(), "setting")));
            continue;
        }
        match apply_plan(&root, &plan).await {
            Ok(()) => {
                out.repaired.push(root.clone());
                let by_folder = if config.profile.is_some() { "" } else { " (picked by folder)" };
                let helper_note = if !config.helpers.is_empty() && !config.helpers.contains(&helper_value(&input.helper)) { ", with the Nixa credential helper" } else { "" };
                out.notes.push(format!("{}: set the git identity to the {} profile{by_folder}{helper_note}.", short_name(&root), profile.label));
            }
            Err(e) => out.notes.push(format!("{}: could not set the git identity: {e}", short_name(&root))),
        }
    }
    let used: BTreeSet<String> = picked.iter().map(|p| p.id.clone()).collect();
    if used.len() > 1 {
        out.notes.push(format!("The repos in this folder use different git profiles ({}). Each repo keeps its own account.", used.into_iter().collect::<Vec<_>>().join(", ")));
        return out;
    }
    if let Some(profile) = picked.into_iter().next() {
        out.account = profile.accounts.get(&host).cloned();
        out.profile = Some(profile);
    }
    out
}

pub fn prep_for(data: &Path, folder: &str, repair: bool) -> Prep {
    Prep {
        folder: folder.to_string(),
        profiles: read_profiles(&profiles_path(data)),
        helper: helper_path(data).display().to_string(),
        host: DEFAULT_HOST.into(),
        repair,
        home: crate::project_tools::policy::home(),
    }
}
