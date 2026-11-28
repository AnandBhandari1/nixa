use crate::accounts::helper::{self, looks_like_token};
use crate::accounts::profiles::{Profile, normalize_profile, parse_remote_url, stamp_repo};
use crate::accounts::run::{self, Opts, Output};
use crate::accounts::{Account, Env, Reply, api_base, api_get, normalize_host, same_host};
use crate::keychain;
use regex::Regex;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub const GITHUB: &str = "github.com";
pub const CLONE_TIMEOUT_SEC: u64 = 3600;
const MAX_OTHER_ACCOUNTS: usize = 4;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Input {
    pub ok: bool,
    pub url: String,
    pub protocol: String,
    pub host: Option<String>,
    pub owner: Option<String>,
    pub repo: String,
    pub user: Option<String>,
    pub display: String,
    pub problem: Option<String>,
    pub short: bool,
}

fn bad(problem: &str) -> Input {
    Input {
        problem: Some(problem.into()),
        ..Input::default()
    }
}

fn good(url: String, protocol: &str, host: Option<String>, owner: Option<String>, repo: String, user: Option<String>) -> Input {
    let display = match &owner {
        Some(o) => format!("{o}/{repo}"),
        None => repo.clone(),
    };
    Input {
        ok: true,
        url,
        protocol: protocol.into(),
        host,
        owner,
        repo,
        user,
        display,
        problem: None,
        short: false,
    }
}

fn re(p: &str) -> Regex {
    Regex::new(p).expect("clone pattern")
}

fn strip_git(name: &str) -> String {
    re(r"(?i)\.git$").replace(name, "").into_owned()
}

fn good_repo(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && re(r"^[A-Za-z0-9._-]+$").is_match(name)
}

fn owner_ok(name: &str) -> bool {
    re(r"^[A-Za-z0-9][A-Za-z0-9-]{0,38}$").is_match(name)
}

fn from_https(raw: &str) -> Input {
    let parts = parse_remote_url(raw);
    let Some(host) = parts.host.clone() else {
        return bad("That link does not look right.");
    };
    let after = re(r"(?i)^[a-z]+://").replace(raw, "");
    if after.split('/').next().unwrap_or("").contains(['?', '#']) {
        return bad("That link does not look right. Check the part before the first /.");
    }
    if parts.token_in_url {
        return bad("Take the password or token out of the link. Pick an account below instead.");
    }
    let scheme = raw[..raw.find(':').unwrap_or(0)].to_lowercase();
    let path = re(r"(?i)^[a-z]+://[^/]*").replace(raw, "").into_owned();
    let path = path.split(['?', '#']).next().unwrap_or("").trim_matches('/').to_string();
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if host == GITHUB {
        if segments.len() < 2 || !owner_ok(segments[0]) || !good_repo(&strip_git(segments[1])) {
            return bad("A GitHub link needs an owner and a repo name, like github.com/owner/repo.");
        }
        let repo = strip_git(segments[1]);
        let url = format!("https://{GITHUB}/{}/{repo}.git", segments[0]);
        return good(url, "https", Some(host), Some(segments[0].into()), repo, parts.user);
    }
    let repo = strip_git(segments.last().copied().unwrap_or(""));
    if !good_repo(&repo) {
        return bad("The link needs a repo name at the end.");
    }
    let owner = (segments.len() > 1).then(|| segments[..segments.len() - 1].join("/"));
    let url = format!("{scheme}://{host}/{}", segments.join("/"));
    good(url, &scheme, Some(host), owner, repo, parts.user)
}

fn ssh_parts(raw: &str) -> (Option<String>, Option<String>) {
    if let Some(m) = re(r"(?i)^[a-z][a-z0-9+.-]*://(?:([^@/]*)@)?(\[[^\]]+\]|[^/:@]+)").captures(raw) {
        return (m.get(1).map(|x| x.as_str().to_string()), Some(m[2].to_string()));
    }
    if let Some(m) = re(r"^(?:([^@/:]+)@)?([^/:@]+):").captures(raw) {
        return (m.get(1).map(|x| x.as_str().to_string()), Some(m[2].to_string()));
    }
    (None, None)
}

fn from_ssh(raw: &str) -> Input {
    let parts = parse_remote_url(raw);
    let (user, host) = ssh_parts(raw);
    let u = user.clone().unwrap_or_default();
    if parts.token_in_url || u.contains(':') || looks_like_token(&u) {
        return bad("Take the password or token out of the link. SSH links sign in with your SSH keys.");
    }
    let h = host.unwrap_or_default();
    if h.starts_with('-') || u.starts_with('-') {
        return bad("The host or user name in the link cannot start with a dash.");
    }
    let repo = strip_git(parts.repo.as_deref().unwrap_or(""));
    let host_ok = re(r"^(?:\[[0-9A-Fa-f:.]+\]|[A-Za-z0-9_][A-Za-z0-9._-]*)$").is_match(&h);
    let user_ok = user.is_none() || re(r"^[A-Za-z0-9_][A-Za-z0-9._-]*$").is_match(&u);
    let Some(phost) = parts.host.clone().filter(|_| good_repo(&repo) && host_ok && user_ok) else {
        return bad("That SSH link does not look right. It looks like git@github.com:owner/repo.git.");
    };
    let protocol = if parts.protocol.as_deref() == Some("git") { "git" } else { "ssh" };
    let phost = re(r"(?i)^(github\.com)[-_].+$").replace(&phost, "$1").into_owned();
    good(raw.trim_end_matches('/').into(), protocol, Some(phost), parts.owner, repo, None)
}

fn from_local(raw: &str) -> Input {
    let path = re(r"(?i)^file://").replace(raw, "").trim_end_matches('/').to_string();
    let repo = strip_git(path.rsplit('/').next().unwrap_or(""));
    if !good_repo(&repo) {
        return bad("That folder name does not look right.");
    }
    let url = if re(r"(?i)^file://").is_match(raw) { raw.trim_end_matches('/').to_string() } else { path };
    good(url, "local", None, None, repo, None)
}

pub fn parse_clone_input(text: &str) -> Input {
    let raw = text.trim();
    if raw.is_empty() {
        return bad("Type a repo link, or owner/name.");
    }
    if raw.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return bad("The link has a space or a sign that is not allowed.");
    }
    if raw.starts_with('-') {
        return bad("The link cannot start with a dash.");
    }
    if re(r"^[A-Za-z0-9+.-]+::").is_match(raw) {
        return bad("Links with :: can run programs, so they are not allowed. Use an https or SSH link.");
    }
    if re(r"(?i)^https?://").is_match(raw) {
        return from_https(raw);
    }
    if re(r"(?i)^(ssh|git\+ssh|ssh\+git|git)://").is_match(raw) {
        return from_ssh(raw);
    }
    if re(r"(?i)^file://").is_match(raw) || raw.starts_with('/') || raw.starts_with("~/") {
        return from_local(raw);
    }
    if re(r"(?i)^[a-z][a-z0-9+.-]*://").is_match(raw) {
        return bad("Use an https or SSH link.");
    }
    if let Some(m) = re(r"^(?:[^@/:]+@)?[^/:@]+:(.?)").captures(raw)
        && &m[1] != "/"
    {
        return from_ssh(raw);
    }
    let trimmed = raw.trim_end_matches('/');
    let segments: Vec<&str> = trimmed.split('/').collect();
    if segments.len() >= 2 && re(r"^[A-Za-z0-9.-]+\.[A-Za-z]{2,}(?::\d+)?$").is_match(segments[0]) {
        return from_https(&format!("https://{raw}"));
    }
    if segments.len() == 2 && owner_ok(segments[0]) && good_repo(&strip_git(segments[1])) {
        let repo = strip_git(segments[1]);
        let url = format!("https://{GITHUB}/{}/{repo}.git", segments[0]);
        let mut out = good(url, "https", Some(GITHUB.into()), Some(segments[0].into()), repo, None);
        out.short = true;
        return out;
    }
    bad("Type a link like https://github.com/owner/repo, or just owner/repo.")
}

pub fn repo_name_of(text: &str) -> String {
    let p = parse_clone_input(text);
    if p.ok { p.repo } else { String::new() }
}

pub fn expand_home(path: &str, home: &Path) -> String {
    let p = path.trim();
    let base = home.display().to_string().trim_end_matches('/').to_string();
    if p == "~" {
        return base;
    }
    match p.strip_prefix("~/") {
        Some(rest) if !base.is_empty() => format!("{base}/{rest}"),
        _ => p.to_string(),
    }
}

pub fn default_folder(home: &Path, url: &str, into: Option<&str>) -> String {
    let name = repo_name_of(url);
    let parent = match into {
        Some(i) if !i.trim().is_empty() => expand_home(i, home).trim_end_matches('/').to_string(),
        _ => format!("{}/Projects", home.display().to_string().trim_end_matches('/')),
    };
    if name.is_empty() || !parent.starts_with('/') {
        return String::new();
    }
    format!("{parent}/{name}")
}

pub fn folder_problem(folder: &str, home: &Path) -> Option<&'static str> {
    if folder.trim().is_empty() {
        return Some("Type the folder to clone into.");
    }
    if !folder.starts_with('/') {
        return Some("Use a full folder path, like ~/Projects/name.");
    }
    if folder.contains(['\0', '\n', '\r']) {
        return Some("The folder path has a sign that is not allowed.");
    }
    let clean = folder.trim_end_matches('/');
    let parts: Vec<&str> = clean.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() < 2 {
        return Some("Pick a folder inside another folder, like ~/Projects/name.");
    }
    if clean.split('/').any(|s| s == "." || s == "..") {
        return Some("The folder path cannot have . or .. in it.");
    }
    if clean == home.display().to_string().trim_end_matches('/') {
        return Some("Pick a new folder, not your home folder.");
    }
    None
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepoItem {
    pub full_name: String,
    pub owner: String,
    pub name: String,
    pub private: bool,
    pub fork: bool,
    pub archived: bool,
    pub description: String,
    pub pushed_at: Option<String>,
    pub clone_url: String,
    pub ssh_url: String,
    pub default_branch: String,
}

fn text(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

pub fn parse_repo_list(body: &str) -> Option<Vec<RepoItem>> {
    let data: Value = serde_json::from_str(body).ok()?;
    let list = data.as_array()?;
    Some(
        list.iter()
            .filter(|r| r.is_object() && r["full_name"].as_str().is_some_and(|n| !n.is_empty()))
            .map(|r| {
                let full = text(&r["full_name"]);
                RepoItem {
                    owner: r["owner"]["login"]
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| full.split('/').next().unwrap_or("").to_string()),
                    full_name: full,
                    name: text(&r["name"]),
                    private: r["private"].as_bool() == Some(true),
                    fork: r["fork"].as_bool() == Some(true),
                    archived: r["archived"].as_bool() == Some(true),
                    description: text(&r["description"]),
                    pushed_at: r["pushed_at"].as_str().map(str::to_string),
                    clone_url: text(&r["clone_url"]),
                    ssh_url: text(&r["ssh_url"]),
                    default_branch: text(&r["default_branch"]),
                }
            })
            .collect(),
    )
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Problem {
    pub kind: String,
    pub title: String,
    pub hint: String,
    pub detail: String,
}

impl Problem {
    pub fn text(&self) -> String {
        crate::accounts::repo::sentence(&[&self.title, &self.hint, &self.detail])
    }
}

fn problem(kind: &str, title: String, hint: &str, detail: &str) -> Problem {
    Problem {
        kind: kind.into(),
        title,
        hint: hint.into(),
        detail: detail.into(),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepoPage {
    pub repos: Vec<RepoItem>,
    pub page: u32,
    pub more: bool,
}

pub fn list_account_repos(env: &Env, host: &str, username: &str, page: u32) -> Result<RepoPage, Problem> {
    let host = Some(normalize_host(host)).filter(|h| !h.is_empty()).unwrap_or(GITHUB.into());
    let page = page.max(1);
    if username.is_empty() {
        return Err(problem("auth", "Pick an account first".into(), "", ""));
    }
    let url = format!("{}/user/repos?per_page=100&sort=pushed&page={page}", api_base(&host));
    let reply: Reply = api_get(env, &host, username, &url);
    if reply.missing {
        return Err(problem("auth", format!("No token for @{username}"), "Reconnect this account in Settings, Git accounts.", ""));
    }
    if let Some(e) = &reply.error {
        return Err(problem("network", format!("Cannot reach {host}"), "Check the network and try again.", e));
    }
    match reply.http.status {
        200 => {}
        401 => {
            return Err(problem(
                "auth",
                format!("{host} refused the token for @{username}"),
                "Reconnect this account in Settings, Git accounts.",
                "",
            ));
        }
        403 => {
            let said = serde_json::from_str::<Value>(&reply.http.body)
                .ok()
                .and_then(|v| v["message"].as_str().map(str::to_string))
                .unwrap_or_default();
            return Err(problem(
                "auth",
                format!("{host} blocked the repo list (403)"),
                "The token may need the repo scope, or you hit the rate limit. Try again later.",
                &said,
            ));
        }
        code => {
            let shown = if code == 0 { "none".to_string() } else { code.to_string() };
            return Err(problem("other", format!("{host} answered with code {shown}"), "Try again in a moment.", ""));
        }
    }
    let repos = parse_repo_list(&reply.http.body)
        .ok_or_else(|| problem("other", format!("{host} sent a list that could not be read"), "Try again in a moment.", ""))?;
    let more = repos.len() >= 100;
    Ok(RepoPage { repos, page, more })
}

pub fn filter_repos<'a>(repos: &'a [RepoItem], query: &str) -> Vec<&'a RepoItem> {
    let q = query.trim().to_lowercase();
    repos
        .iter()
        .filter(|r| q.is_empty() || r.full_name.to_lowercase().contains(&q) || r.description.to_lowercase().contains(&q))
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Folder {
    Missing,
    File,
    Full,
    Empty,
}

pub fn folder_state(path: &Path) -> Result<Folder, String> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Folder::Missing),
        Err(e) => Err(format!("cannot check the folder: {e}")),
        Ok(m) if !m.is_dir() => Ok(Folder::File),
        Ok(_) => {
            let mut items = std::fs::read_dir(path).map_err(|e| format!("cannot check the folder: {e}"))?;
            Ok(if items.next().is_some() { Folder::Full } else { Folder::Empty })
        }
    }
}

fn clean_up(path: &Path, before: Folder, out: &Output) -> bool {
    if !out.cancelled && !out.timed_out {
        return false;
    }
    if !matches!(before, Folder::Missing | Folder::Empty) {
        return false;
    }
    if re(r"(?i)already exists and is not an empty directory|Clone succeeded").is_match(&out.said()) {
        return false;
    }
    if !path.is_dir() {
        return false;
    }
    if path.join(".git").is_dir() {
        if before == Folder::Empty {
            let Ok(items) = std::fs::read_dir(path) else {
                return false;
            };
            for item in items.flatten() {
                let p = item.path();
                let _ = if p.is_dir() && !p.is_symlink() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
            }
            return true;
        }
        return std::fs::remove_dir_all(path).is_ok();
    }
    before == Folder::Missing && std::fs::remove_dir(path).is_ok()
}

pub fn clone_args(url: &str, folder: &str, account: Option<(&str, &str, &Path)>) -> Result<Vec<String>, String> {
    let mut args = match account {
        Some((host, user, helper_path)) => helper::account_args(host, user, helper_path)?,
        None => Vec::new(),
    };
    args.extend(run::strings(&["clone", "--progress", "--", url, folder]));
    Ok(args)
}

#[derive(Clone, Debug, Default)]
pub struct CloneOpts {
    pub url: String,
    pub folder: String,
    pub account: Option<(String, String)>,
    pub profile: Option<Profile>,
    pub accounts: Vec<Account>,
    pub try_others: bool,
    pub cancel: Option<Arc<AtomicBool>>,
    pub timeout: Option<Duration>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Used {
    pub host: String,
    pub username: String,
    pub fallback: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cloned {
    pub ok: bool,
    pub kind: String,
    pub title: String,
    pub hint: String,
    pub detail: String,
    pub folder: String,
    pub repo: String,
    pub kept: bool,
    pub used: Option<Used>,
    pub tried: Vec<String>,
    pub bound: Vec<(String, String)>,
    pub warnings: Vec<String>,
}

impl Cloned {
    pub fn text(&self) -> String {
        crate::accounts::repo::sentence(&[&self.title, &self.hint, &self.detail])
    }
}

fn refused(kind: &str, title: &str, hint: &str, folder: &str) -> Cloned {
    Cloned {
        kind: kind.into(),
        title: title.into(),
        hint: hint.into(),
        folder: folder.into(),
        ..Cloned::default()
    }
}

fn from_said(text: &str, folder: &str) -> Cloned {
    let s = run::describe(text);
    Cloned {
        kind: s.kind,
        title: s.title,
        hint: s.hint,
        detail: s.detail,
        folder: folder.into(),
        ..Cloned::default()
    }
}

pub fn auth_failed(out: &Output) -> bool {
    !out.cancelled
        && !out.timed_out
        && !out.failed_to_start
        && out.code.is_some_and(|c| c != 0)
        && ["auth", "notFound"].contains(&run::classify(&out.said()))
}

fn run_clone(env: &Env, args: &[String], opts: &CloneOpts) -> Output {
    let mut all = env.git_env.clone();
    all.extend(run::GIT_ENV.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    all.push(("GIT_SSH_COMMAND".into(), run::SSH_BATCH.into()));
    run::run(
        &env.git,
        args,
        &Opts {
            cwd: Some("/".into()),
            env: all,
            stdin: None,
            timeout: Some(opts.timeout.unwrap_or(Duration::from_secs(CLONE_TIMEOUT_SEC))),
            cancel: opts.cancel.clone(),
        },
    )
}

pub fn other_accounts(accounts: &[Account], host: &str, tried: Option<&str>) -> Vec<Account> {
    let mut list: Vec<Account> = accounts
        .iter()
        .filter(|a| {
            !a.username.is_empty()
                && same_host(&a.host, host)
                && !tried.is_some_and(|t| t.eq_ignore_ascii_case(&a.username))
                && a.status != "revoked"
                && a.status != "expired"
        })
        .cloned()
        .collect();
    list.sort_by_key(|a| !a.default);
    list.truncate(MAX_OTHER_ACCOUNTS);
    list
}

fn set_up(env: &Env, folder: &Path, profile: Option<&Profile>, used: Option<&Used>, host: Option<&str>) -> (Vec<(String, String)>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut bound = Vec::new();
    let bind = |bound: &mut Vec<(String, String)>, warnings: &mut Vec<String>, h: &str, u: &Used| {
        match helper::bind_repo(env, folder, h, &u.username) {
            Ok(_) => bound.push((h.to_string(), u.username.clone())),
            Err(e) => warnings.push(format!("Cloned, but the account was not set for this repo: {e}")),
        }
    };
    match profile {
        Some(p) => match stamp_repo(env, folder, p, false) {
            Ok(done) => {
                bound = done.bound;
                let want = host.and_then(|h| normalize_profile(p).accounts.get(h).cloned());
                match (used, host, want) {
                    (Some(u), _, Some(w)) if !w.eq_ignore_ascii_case(&u.username) => warnings.push(format!(
                        "It was cloned with @{}, but {} uses @{w}. Pushes will use @{w}.",
                        u.username,
                        if p.label.is_empty() { "the profile" } else { &p.label }
                    )),
                    (Some(u), Some(h), None) => bind(&mut bound, &mut warnings, h, u),
                    _ => {}
                }
            }
            Err(e) => warnings.push(format!("Cloned, but the profile was not set: {e}")),
        },
        None => {
            if let (Some(u), Some(h)) = (used, host) {
                bind(&mut bound, &mut warnings, h, u);
            }
        }
    }
    (bound, warnings)
}

pub fn clone_repo(env: &Env, opts: &CloneOpts) -> Cloned {
    let parsed = parse_clone_input(&opts.url);
    if !parsed.ok {
        return refused("invalid", "Check the repo link", parsed.problem.as_deref().unwrap_or(""), "");
    }
    let folder = expand_home(&opts.folder, &env.home).trim_end_matches('/').to_string();
    if let Some(p) = folder_problem(&folder, &env.home) {
        return refused("invalid", "Check the folder", p, &folder);
    }
    let url = if parsed.protocol == "local" && !parsed.url.to_lowercase().starts_with("file:") {
        expand_home(&parsed.url, &env.home)
    } else {
        parsed.url.clone()
    };
    let path = Path::new(&folder);
    let before = match folder_state(path) {
        Ok(s) => s,
        Err(e) => return from_said(&e, &folder),
    };
    match before {
        Folder::Full => return refused("exists", "That folder is not empty", "Pick a new folder name, or empty that folder first.", &folder),
        Folder::File => return refused("exists", "A file with that name is already there", "Pick another folder name.", &folder),
        _ => {}
    }
    let host = (parsed.protocol == "https").then(|| parsed.host.clone()).flatten();
    let chosen = match (&host, &opts.account) {
        (Some(h), Some((ah, au))) if !au.is_empty() && same_host(if ah.is_empty() { h } else { ah }, h) => Some(au.clone()),
        _ => None,
    };
    let helper_path = match (&chosen, opts.try_others && host.is_some()) {
        (None, false) => None,
        _ => match helper::ensure_helper(env) {
            Ok(p) => Some(p),
            Err(e) => return from_said(&e, &folder),
        },
    };
    let make_args = |user: Option<&str>| {
        let account = match (user, host.as_deref(), helper_path.as_deref()) {
            (Some(u), Some(h), Some(p)) => Some((h, u, p)),
            _ => None,
        };
        clone_args(&url, &folder, account)
    };
    let args = match make_args(chosen.as_deref()) {
        Ok(a) => a,
        Err(e) => return refused("invalid", "Check the account", &e, &folder),
    };
    let mut out = run_clone(env, &args, opts);
    let mut used = chosen.as_ref().zip(host.as_ref()).map(|(u, h)| Used {
        host: h.clone(),
        username: u.clone(),
        fallback: false,
    });
    let mut tried = Vec::new();
    if let Some(h) = host.as_deref().filter(|_| opts.try_others && auth_failed(&out)) {
        for a in other_accounts(&opts.accounts, h, chosen.as_deref()) {
            if run::is_cancelled(&opts.cancel) {
                break;
            }
            if !keychain::has_token(env, h, &a.username).unwrap_or(false) {
                continue;
            }
            if helper_path.is_none() {
                break;
            }
            let Ok(retry) = make_args(Some(&a.username)) else {
                continue;
            };
            let _ = clean_up(path, before, &Output { cancelled: true, ..out.clone() });
            let next = run_clone(env, &retry, opts);
            tried.push(a.username.clone());
            let stop = next.cancelled || next.timed_out || next.failed_to_start || !auth_failed(&next);
            if stop && next.ok() {
                used = Some(Used {
                    host: h.into(),
                    username: a.username.clone(),
                    fallback: true,
                });
            }
            out = next;
            if stop {
                break;
            }
        }
    }
    if out.cancelled || out.timed_out {
        let cleaned = clean_up(path, before, &out);
        if out.timed_out {
            let mut c = from_said("timed out waiting", &folder);
            c.detail.clear();
            return c;
        }
        let hint = if cleaned {
            "The clone was stopped. The half-made folder was removed."
        } else {
            "The clone was stopped."
        };
        return refused("cancelled", "Stopped", hint, &folder);
    }
    let said = out.said();
    if !out.ok() && re(r"(?i)Clone succeeded, but checkout failed").is_match(&said) {
        let (bound, warnings) = set_up(env, path, opts.profile.as_ref(), used.as_ref(), host.as_deref());
        return Cloned {
            kind: "partial".into(),
            title: "Downloaded, but the files were not written".into(),
            hint: "The repo is kept in its folder. Read the message below, fix the problem, then open the folder.".into(),
            detail: run::describe(&said).detail,
            folder,
            repo: parsed.repo,
            kept: true,
            used,
            tried,
            bound,
            warnings,
            ..Cloned::default()
        };
    }
    if !out.ok() && re(r"(?i)already exists and is not an empty directory").is_match(&said) {
        return refused(
            "exists",
            "That folder is not empty now",
            "Files came into it just before the clone started. Nothing in it was removed. Pick a new folder name.",
            &folder,
        );
    }
    if !out.ok() {
        let mut c = from_said(&said, &folder);
        if !tried.is_empty() {
            let names: Vec<String> = tried.iter().map(|u| format!("@{u}")).collect();
            let who = if tried.len() == 1 { "It" } else { "They" };
            c.hint = format!("{} Also tried {}. {who} could not sign in either.", c.hint, names.join(", "))
                .trim()
                .to_string();
        }
        c.tried = tried;
        return c;
    }
    let (bound, warnings) = set_up(env, path, opts.profile.as_ref(), used.as_ref(), host.as_deref());
    Cloned {
        ok: true,
        title: format!("Cloned {}", parsed.display),
        folder,
        repo: parsed.repo,
        used,
        tried,
        bound,
        warnings,
        ..Cloned::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::testkit;

    #[test]
    fn inputs_are_read_safely() {
        let p = parse_clone_input("owner/app");
        assert!(p.ok && p.short);
        assert_eq!(p.url, "https://github.com/owner/app.git");
        let p = parse_clone_input("https://github.com/o/r/tree/main");
        assert_eq!(p.url, "https://github.com/o/r.git");
        assert_eq!(p.display, "o/r");
        let p = parse_clone_input("gitlab.com/group/sub/app.git");
        assert_eq!(p.host.as_deref(), Some("gitlab.com"));
        assert_eq!(p.owner.as_deref(), Some("group/sub"));
        assert_eq!(parse_clone_input("git@github.com:o/r.git").protocol, "ssh");
        assert_eq!(parse_clone_input("~/code/thing.git").repo, "thing");
        assert!(parse_clone_input("ext::sh -c x").problem.is_some());
        assert!(parse_clone_input("-oProxy").problem.unwrap().contains("dash"));
        assert!(parse_clone_input("https://u:p@github.com/o/r").problem.unwrap().contains("token"));
        assert!(parse_clone_input("ftp://x/y").problem.unwrap().contains("https or SSH"));
        assert!(parse_clone_input("a b").problem.is_some());
        assert!(!parse_clone_input("").ok);
        let home = Path::new("/home/me");
        assert_eq!(default_folder(home, "o/app", None), "/home/me/Projects/app");
        assert_eq!(default_folder(home, "o/app", Some("~/src")), "/home/me/src/app");
        assert_eq!(folder_problem("/home/me", home), Some("Pick a new folder, not your home folder."));
        assert_eq!(folder_problem("/x", home), Some("Pick a folder inside another folder, like ~/Projects/name."));
        assert_eq!(folder_problem("/a/../b", home), Some("The folder path cannot have . or .. in it."));
        assert_eq!(folder_problem("rel/x", home), Some("Use a full folder path, like ~/Projects/name."));
        assert_eq!(folder_problem("/a/b", home), None);
    }

    #[test]
    fn repo_lists_are_read() {
        let body = r#"[{"full_name":"me/app","name":"app","owner":{"login":"me"},"private":true,"clone_url":"https://github.com/me/app.git"},{"name":"x"}]"#;
        let list = parse_repo_list(body).unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].private);
        assert_eq!(filter_repos(&list, "APP").len(), 1);
        assert!(parse_repo_list("{}").is_none());
    }

    #[test]
    fn account_repos_come_from_the_api_with_the_keychain_token() {
        let kit = testkit::kit();
        let env = &kit.env;
        keychain::save_token(env, "github.com", "ana", "tok_list_token").unwrap();
        kit.curl_reply("tok_list_token.repos", "HTTP/2 200\r\n\r\n[{\"full_name\":\"ana/app\",\"name\":\"app\"}]");
        let page = list_account_repos(env, "github.com", "ana", 1).unwrap();
        assert_eq!(page.repos[0].full_name, "ana/app");
        assert!(!page.more);
        assert!(!kit.curl_args().contains("tok_list_token"));
        assert_eq!(list_account_repos(env, "github.com", "nobody", 1).unwrap_err().kind, "auth");
    }

    fn source(kit: &testkit::Kit) -> String {
        let src = kit.repo("src");
        kit.git(&src, &["config", "user.name", "T"]);
        kit.git(&src, &["config", "user.email", "t@x.org"]);
        std::fs::write(src.join("a.txt"), "hi").unwrap();
        kit.git(&src, &["add", "a.txt"]);
        kit.git(&src, &["commit", "-q", "-m", "one"]);
        src.display().to_string()
    }

    #[test]
    fn local_clone_works_and_stamps_the_profile() {
        let kit = testkit::kit();
        let env = &kit.env;
        let url = source(&kit);
        let folder = kit.root().join("out/app");
        let profile = Profile {
            id: "work".into(),
            label: "Work".into(),
            name: "Ana".into(),
            email: "ana@x.org".into(),
            ..Profile::default()
        };
        let opts = CloneOpts {
            url: url.clone(),
            folder: folder.display().to_string(),
            profile: Some(profile),
            ..CloneOpts::default()
        };
        let done = clone_repo(env, &opts);
        assert!(done.ok, "{done:?}");
        assert!(folder.join("a.txt").exists());
        assert_eq!(kit.git(&folder, &["config", "--local", "user.email"]), "ana@x.org");
        let again = clone_repo(env, &opts);
        assert_eq!(again.kind, "exists");
        let bad = CloneOpts { url: "ext::x".into(), ..opts.clone() };
        assert_eq!(clone_repo(env, &bad).kind, "invalid");
    }

    #[test]
    fn cancel_removes_the_half_made_folder() {
        let kit = testkit::kit();
        let env = &kit.env;
        let url = source(&kit);
        let folder = kit.root().join("gone");
        std::fs::create_dir_all(folder.join(".git")).unwrap();
        let out = Output { cancelled: true, ..Output::default() };
        assert!(clean_up(&folder, Folder::Missing, &out));
        assert!(!folder.exists());
        let flag = run::cancel_flag();
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
        let opts = CloneOpts {
            url,
            folder: folder.display().to_string(),
            cancel: Some(flag),
            ..CloneOpts::default()
        };
        let done = clone_repo(env, &opts);
        assert_eq!(done.kind, "cancelled");
        assert!(!folder.exists());
    }

    #[test]
    fn clone_args_put_the_account_first() {
        let args = clone_args("https://github.com/o/r.git", "/a/b", Some(("github.com", "ana", Path::new("/h/helper")))).unwrap();
        assert_eq!(args[0], "-c");
        assert!(args.contains(&"credential.https://github.com.username=ana".to_string()));
        assert_eq!(&args[args.len() - 3..], ["--", "https://github.com/o/r.git", "/a/b"]);
        let list = vec![
            Account { host: "github.com".into(), username: "a".into(), status: "expired".into(), ..Account::default() },
            Account { host: "github.com".into(), username: "b".into(), ..Account::default() },
            Account { host: "github.com".into(), username: "c".into(), default: true, ..Account::default() },
        ];
        let names: Vec<String> = other_accounts(&list, "github.com", Some("b")).into_iter().map(|a| a.username).collect();
        assert_eq!(names, ["c"]);
    }
}
