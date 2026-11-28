use super::helper::{self, HELPER_NAME, git_ok, git_str, looks_like_token, percent_decode};
use super::{Account, DEFAULT_HOST, Env, list_of, profiles_file, read_json, write_json};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const COLORS: &[&str] = &["ok", "info", "purple", "cyan", "warn"];

pub const KEY_PROFILE: &str = "nixa.profile";
pub const KEY_KEEP: &str = "nixa.keepIdentity";
pub const KEY_UNMANAGED: &str = "nixa.unmanaged";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub id: String,
    pub label: String,
    pub name: String,
    pub email: String,
    pub accounts: BTreeMap<String, String>,
    pub folders: Vec<String>,
    pub owners: Vec<String>,
    pub color: String,
}

fn unique(list: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in list {
        if !out.contains(&item) {
            out.push(item);
        }
    }
    out
}

fn clean_list(list: &[String]) -> Vec<String> {
    unique(
        list.iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
    )
}

pub fn normalize_profile(p: &Profile) -> Profile {
    let mut accounts = BTreeMap::new();
    for (host, user) in &p.accounts {
        let (h, u) = (host.trim().to_lowercase(), user.trim().to_string());
        if !h.is_empty() && !u.is_empty() {
            accounts.insert(h, u);
        }
    }
    Profile {
        id: p.id.trim().to_string(),
        label: p.label.trim().to_string(),
        name: p.name.trim().to_string(),
        email: p.email.trim().to_string(),
        accounts,
        folders: clean_list(&p.folders),
        owners: clean_list(&p.owners),
        color: if COLORS.contains(&p.color.as_str()) {
            p.color.clone()
        } else {
            "info".into()
        },
    }
}

fn fold_accents(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            other => other,
        })
        .collect()
}

pub fn profile_id(label: &str, taken: &[String]) -> String {
    let low = fold_accents(&label.trim().to_lowercase());
    let mut base = String::new();
    let mut dash = false;
    for c in low.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            base.push(c);
            dash = false;
        } else if !dash {
            base.push('-');
            dash = true;
        }
    }
    let mut base: String = base.trim_matches('-').chars().take(32).collect();
    base = base.trim_end_matches('-').to_string();
    if base.is_empty() {
        base = "profile".into();
    }
    let mut id = base.clone();
    let mut n = 2;
    while taken.contains(&id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

fn with_ids(list: Vec<Profile>) -> Vec<Profile> {
    let mut out: Vec<Profile> = Vec::new();
    for item in list {
        let mut p = normalize_profile(&item);
        if p.id.is_empty() || out.iter().any(|x| x.id == p.id) {
            let taken: Vec<String> = out.iter().map(|x| x.id.clone()).collect();
            let from = if p.label.is_empty() { p.name.clone() } else { p.label.clone() };
            p.id = profile_id(&from, &taken);
        }
        out.push(p);
    }
    out
}

pub fn load_profiles(env: &Env) -> Vec<Profile> {
    with_ids(
        list_of(read_json(&profiles_file(env)), "profiles")
            .into_iter()
            .filter(Value::is_object)
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect(),
    )
}

pub fn save_profiles(env: &Env, profiles: &[Profile]) -> Result<Vec<Profile>, String> {
    let list = with_ids(profiles.to_vec());
    write_json(&profiles_file(env), &json!({"version": 1, "profiles": list}))?;
    Ok(list)
}

pub fn validate_profile(label: &str, name: &str, email: &str) -> Option<&'static str> {
    if label.trim().is_empty() {
        return Some("Give the profile a label, like Work or Personal.");
    }
    if name.trim().is_empty() {
        return Some("Type the name to use in commits.");
    }
    let email = email.trim();
    if email.is_empty() {
        return Some("Type the email to use in commits.");
    }
    let shape = Regex::new(r"^[^\s@]+@[^\s@]+$").expect("email pattern");
    if !shape.is_match(email) {
        return Some("That email does not look right.");
    }
    None
}

pub fn next_color(profiles: &[Profile]) -> String {
    COLORS
        .iter()
        .find(|c| !profiles.iter().any(|p| p.color == **c))
        .unwrap_or(&COLORS[profiles.len() % COLORS.len()])
        .to_string()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub host: String,
    pub username: String,
    pub default: bool,
}

pub fn account_options(accounts: &[Account], current: &BTreeMap<String, String>) -> Vec<Choice> {
    let mut seen: Vec<Choice> = Vec::new();
    let mut add = |host: &str, user: &str, default: bool| {
        let (h, u) = (host.trim().to_lowercase(), user.trim().to_string());
        if h.is_empty() || u.is_empty() {
            return;
        }
        if !seen
            .iter()
            .any(|c| c.host == h && c.username.to_lowercase() == u.to_lowercase())
        {
            seen.push(Choice { host: h, username: u, default });
        }
    };
    for a in accounts {
        add(&a.host, &a.username, a.default);
    }
    for (h, u) in current {
        add(h, u, false);
    }
    seen.sort_by(|a, b| {
        (a.host != DEFAULT_HOST)
            .cmp(&(b.host != DEFAULT_HOST))
            .then(a.host.cmp(&b.host))
            .then(a.username.cmp(&b.username))
    });
    seen
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteUrl {
    pub host: Option<String>,
    pub owner: Option<String>,
    pub repo: Option<String>,
    pub user: Option<String>,
    pub protocol: Option<String>,
    pub token_in_url: bool,
}

pub fn parse_remote_url(url: &str) -> RemoteUrl {
    let raw = url.trim();
    let full = Regex::new(r"(?i)^([a-z][a-z0-9+.-]*)://(?:([^@/]*)@)?(\[[^\]]+\]|[^/:@]+)(?::(\d*))?(/.*)?$").expect("url pattern");
    let scp = Regex::new(r"^(?:([^@/:]+)@)?([^/:@]+):(.*)$").expect("scp pattern");
    let (host, user, path, protocol, token_in_url);
    if let Some(m) = full.captures(raw) {
        let proto = m[1].to_lowercase();
        if proto == "file" {
            return RemoteUrl::default();
        }
        let info = m.get(2).map(|x| x.as_str()).unwrap_or("");
        let mut h = m[3].to_lowercase();
        let web = proto == "https" || proto == "http";
        if web && let Some(port) = m.get(4).filter(|p| !p.as_str().is_empty()) {
            h = format!("{h}:{}", port.as_str());
        }
        let name = info.split(':').next().unwrap_or("");
        let mut u = (web && !name.is_empty()).then(|| percent_decode(name));
        let tokenish = u.as_deref().is_some_and(looks_like_token);
        token_in_url = info.contains(':') || tokenish;
        if tokenish {
            u = None;
        }
        host = h;
        user = u;
        path = m.get(5).map(|x| x.as_str().to_string()).unwrap_or_default();
        protocol = proto;
    } else if let Some(m) = scp.captures(raw).filter(|m| !m[3].starts_with("//") && !(m[2].len() == 1 && m[2].chars().all(|c| c.is_ascii_alphabetic()))) {
        protocol = "ssh".to_string();
        host = m[2].to_lowercase();
        path = m[3].to_string();
        user = None;
        token_in_url = false;
    } else {
        return RemoteUrl::default();
    }
    let trimmed = path.trim_matches('/');
    let trimmed = Regex::new(r"(?i)\.git$").expect("git suffix").replace(trimmed, "").into_owned();
    let parts: Vec<&str> = trimmed.split('/').filter(|s| !s.is_empty()).collect();
    RemoteUrl {
        host: Some(host),
        owner: (parts.len() > 1).then(|| parts[..parts.len() - 1].join("/")),
        repo: parts.last().map(|s| s.to_string()),
        user: user.filter(|u| !u.is_empty()),
        protocol: Some(protocol),
        token_in_url,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub scope: String,
    pub key: String,
    pub value: Option<String>,
}

pub fn parse_config_list(output: &str) -> Vec<Entry> {
    let parts: Vec<&str> = output.split('\0').collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < parts.len() {
        let body = parts[i + 1];
        let (key, value) = match body.find('\n') {
            Some(nl) => (&body[..nl], Some(body[nl + 1..].to_string())),
            None => (body, None),
        };
        out.push(Entry {
            scope: parts[i].to_string(),
            key: key.to_string(),
            value,
        });
        i += 2;
    }
    out
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    pub url: String,
    pub push_url: String,
    pub host: Option<String>,
    pub owner: Option<String>,
    pub repo: Option<String>,
    pub user: Option<String>,
    pub push_user: Option<String>,
    pub protocol: Option<String>,
    pub token_in_url: bool,
}

pub fn parse_remote_list(output: &str) -> Vec<Remote> {
    let line_re = Regex::new(r"^(\S+)\t(.+?) \((fetch|push)\)$").expect("remote line");
    let mut by_name: Vec<(String, Option<String>, Option<String>)> = Vec::new();
    for line in output.lines() {
        let Some(m) = line_re.captures(line) else {
            continue;
        };
        let name = m[1].to_string();
        let index = match by_name.iter().position(|(n, _, _)| *n == name) {
            Some(i) => i,
            None => {
                by_name.push((name, None, None));
                by_name.len() - 1
            }
        };
        if &m[3] == "fetch" {
            by_name[index].1 = Some(m[2].to_string());
        } else {
            by_name[index].2 = Some(m[2].to_string());
        }
    }
    let mut out: Vec<Remote> = by_name
        .into_iter()
        .map(|(name, fetch, push)| {
            let url = fetch.clone().or(push.clone()).unwrap_or_default();
            let push_url = push.unwrap_or(url.clone());
            let f = parse_remote_url(&url);
            let p = parse_remote_url(&push_url);
            Remote {
                name,
                url,
                push_url,
                host: f.host.clone().or(p.host.clone()),
                owner: f.owner.clone().or(p.owner.clone()),
                repo: f.repo.clone().or(p.repo.clone()),
                user: f.user.clone().or(p.user.clone()),
                push_user: p.user.clone(),
                protocol: f.protocol.clone().or(p.protocol.clone()),
                token_in_url: f.token_in_url || p.token_in_url,
            }
        })
        .collect();
    out.sort_by_key(|r| r.name != "origin");
    out
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Binding {
    pub username: Option<String>,
    pub helper_installed: bool,
    pub ours: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub profile_id: Option<String>,
    pub name: Option<String>,
    pub email: Option<String>,
    pub name_scope: String,
    pub email_scope: String,
    pub local_name: Option<String>,
    pub local_email: Option<String>,
    pub fallback_name: Option<String>,
    pub fallback_email: Option<String>,
    pub keep_identity: bool,
    pub unmanaged: bool,
    pub remotes: Vec<Remote>,
    pub bindings: BTreeMap<String, Binding>,
    pub is_repo: bool,
}

impl Default for Identity {
    fn default() -> Self {
        Identity {
            profile_id: None,
            name: None,
            email: None,
            name_scope: "none".into(),
            email_scope: "none".into(),
            local_name: None,
            local_email: None,
            fallback_name: None,
            fallback_email: None,
            keep_identity: false,
            unmanaged: false,
            remotes: Vec::new(),
            bindings: BTreeMap::new(),
            is_repo: true,
        }
    }
}

fn scope_of(scope: &str) -> &'static str {
    match scope {
        "local" | "worktree" | "command" => "local",
        "global" => "global",
        _ => "system",
    }
}

fn credential_host(url: &str) -> Option<String> {
    let re = Regex::new(r"(?i)^https://([^/@]+)/?$").expect("credential host");
    re.captures(url).map(|m| m[1].to_lowercase())
}

pub fn build_identity(entries: &[Entry], remotes: Vec<Remote>, helper_path: Option<&Path>) -> Identity {
    let mut out = Identity {
        remotes,
        ..Identity::default()
    };
    let cred = Regex::new(r"(?i)^credential\.(.+)\.(helper|username)$").expect("credential key");
    let mut creds: BTreeMap<String, (Vec<String>, Option<String>)> = BTreeMap::new();
    for e in entries {
        let place = scope_of(&e.scope);
        let key = e.key.to_lowercase();
        let local = place == "local";
        match key.as_str() {
            "user.name" => {
                out.name = e.value.clone();
                out.name_scope = place.into();
                if local {
                    out.local_name = e.value.clone();
                } else {
                    out.fallback_name = e.value.clone();
                }
            }
            "user.email" => {
                out.email = e.value.clone();
                out.email_scope = place.into();
                if local {
                    out.local_email = e.value.clone();
                } else {
                    out.fallback_email = e.value.clone();
                }
            }
            "nixa.profile" if local => {
                out.profile_id = e.value.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
            }
            "nixa.keepidentity" if local => {
                out.keep_identity = e.value.as_deref().is_some_and(|v| v.trim().eq_ignore_ascii_case("true"));
            }
            "nixa.unmanaged" if local => {
                out.unmanaged = e.value.as_deref().is_some_and(|v| v.trim().eq_ignore_ascii_case("true"));
            }
            _ if local => {
                if let Some(m) = cred.captures(&e.key)
                    && let Some(host) = credential_host(&m[1])
                {
                    let slot = creds.entry(host).or_default();
                    if m[2].eq_ignore_ascii_case("helper") {
                        slot.0.push(e.value.clone().unwrap_or_default());
                    } else {
                        slot.1 = e.value.clone();
                    }
                }
            }
            _ => {}
        }
    }
    let mut hosts: Vec<String> = creds.keys().cloned().collect();
    for r in &out.remotes {
        if let Some(h) = &r.host
            && !hosts.contains(h)
        {
            hosts.push(h.clone());
        }
    }
    let helper = helper_path.map(|p| p.display().to_string());
    for host in hosts {
        let (helpers, username) = creds.get(&host).cloned().unwrap_or_default();
        let quoted = helper_path.map(helper::helper_value);
        let at = helpers.iter().rposition(|x| helper.as_ref() == Some(x) || quoted.as_ref() == Some(x));
        let reset = helpers.iter().rposition(|x| x.is_empty());
        let ours = helpers.iter().any(|h| {
            helper.as_deref() == Some(h.as_str()) || helper_path.is_some_and(|p| helper::helper_value(p) == *h) || h.ends_with(&format!("/{HELPER_NAME}")) || h.ends_with(&format!("/{HELPER_NAME}'"))
        });
        out.bindings.insert(
            host,
            Binding {
                username: username.map(|u| u.trim().to_string()).filter(|u| !u.is_empty()),
                helper_installed: matches!((at, reset), (Some(a), Some(r)) if r < a),
                ours,
            },
        );
    }
    out
}

pub fn origin_scope(origin: &str) -> &'static str {
    if origin.starts_with("command line:") {
        return "command";
    }
    let path = origin.strip_prefix("file:").unwrap_or(origin);
    let local = Regex::new(r"(^|/)\.git/(config|config\.worktree)$").expect("local config");
    let worktree = Regex::new(r"/worktrees/[^/]+/config\.worktree$").expect("worktree config");
    if local.is_match(path) || worktree.is_match(path) {
        return "local";
    }
    if path.starts_with("/etc/")
        || path.contains("/etc/gitconfig")
        || path.starts_with("/Library/")
        || path.starts_with("/Applications/")
    {
        return "system";
    }
    "global"
}

fn read_config_entries(env: &Env, root: &Path) -> Result<Vec<Entry>, String> {
    let scoped = git_str(env, root, &["config", "--list", "--show-scope", "-z"]);
    if scoped.ok() || !scoped.stdout.is_empty() {
        return Ok(parse_config_list(&scoped.stdout));
    }
    let origin = git_str(env, root, &["config", "--list", "--show-origin", "-z"]);
    if !origin.ok() && origin.stdout.is_empty() {
        let why = if origin.stderr.trim().is_empty() { &scoped.stderr } else { &origin.stderr };
        let why = why.trim();
        return Err(if why.is_empty() { "cannot read git config".into() } else { why.to_string() });
    }
    Ok(parse_config_list(&origin.stdout)
        .into_iter()
        .map(|e| Entry {
            scope: origin_scope(&e.scope).into(),
            ..e
        })
        .collect())
}

pub fn read_repo_identity(env: &Env, root: &Path) -> Result<Identity, String> {
    let entries = read_config_entries(env, root)?;
    let remotes = git_str(env, root, &["remote", "-v"]);
    let helper = helper::helper_path(env);
    let list = if remotes.ok() { parse_remote_list(&remotes.stdout) } else { Vec::new() };
    let mut id = build_identity(&entries, list, Some(&helper));
    id.is_repo = remotes.ok();
    Ok(id)
}

pub fn repo_hosts(identity: &Identity) -> Vec<String> {
    let hosts = unique(identity.remotes.iter().filter_map(|r| r.host.clone()).collect());
    if hosts.is_empty() { vec![DEFAULT_HOST.to_string()] } else { hosts }
}

fn expand_folder(folder: &str, home: &Path) -> Option<PathBuf> {
    let mut path = folder.trim().to_string();
    let base = home.display().to_string().trim_end_matches('/').to_string();
    if path == "~" {
        path = base.clone();
    } else if let Some(rest) = path.strip_prefix("~/") {
        if base.is_empty() {
            return None;
        }
        path = format!("{base}/{rest}");
    }
    let path = path
        .trim_end_matches("/**")
        .trim_end_matches("/*")
        .trim_end_matches('/')
        .to_string();
    path.starts_with('/').then(|| PathBuf::from(path))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub profile: Profile,
    pub rule: &'static str,
    pub reason: String,
}

fn lower(text: &str) -> String {
    text.trim().to_lowercase()
}

pub fn suggest_profile(profiles: &[Profile], identity: &Identity, root: &Path, home: &Path) -> Option<Suggestion> {
    let list: Vec<&Profile> = profiles.iter().filter(|p| !p.id.is_empty()).collect();
    if list.is_empty() {
        return None;
    }
    let pick = |p: &Profile, rule, reason: String| Some(Suggestion {
        profile: p.clone(),
        rule,
        reason,
    });
    let mut remotes = identity.remotes.clone();
    remotes.sort_by_key(|r| r.name != "origin");
    if let Some(id) = &identity.profile_id
        && let Some(bound) = list.iter().find(|p| &p.id == id)
    {
        return pick(bound, "bound", format!("This repo is already set to {}.", bound.label));
    }
    let local_email = identity
        .local_email
        .clone()
        .or_else(|| (identity.email_scope == "local").then(|| identity.email.clone()).flatten());
    if let Some(email) = local_email
        && let Some(m) = list.iter().find(|p| !p.email.is_empty() && lower(&p.email) == lower(&email))
    {
        return pick(m, "email", format!("The repo email {} belongs to {}.", email.trim(), m.label));
    }
    for r in &remotes {
        let (Some(host), Some(user)) = (&r.host, &r.user) else {
            continue;
        };
        if let Some(m) = list
            .iter()
            .find(|p| p.accounts.get(host).is_some_and(|u| lower(u) == lower(user)))
        {
            return pick(m, "user", format!("The {} URL uses the account {user}.", r.name));
        }
    }
    for r in &remotes {
        let Some(owner) = &r.owner else {
            continue;
        };
        let names = unique(vec![lower(owner), lower(owner).split('/').next().unwrap_or("").to_string()]);
        let by_owner = list
            .iter()
            .find(|p| p.owners.iter().any(|o| names.contains(&lower(o))));
        let by_account = r.host.as_ref().and_then(|h| {
            list.iter()
                .find(|p| p.accounts.get(h).is_some_and(|u| names.contains(&lower(u))))
        });
        if let Some(m) = by_owner.or(by_account) {
            return pick(m, "owner", format!("The repo owner {owner} belongs to {}.", m.label));
        }
    }
    let here = root.display().to_string().trim_end_matches('/').to_string();
    let mut best: Option<(&Profile, String, String)> = None;
    for p in &list {
        for folder in &p.folders {
            let Some(path) = expand_folder(folder, home) else {
                continue;
            };
            let path = path.display().to_string();
            let inside = here == path || here.starts_with(&format!("{path}/"));
            if inside && best.as_ref().is_none_or(|b| path.len() > b.1.len()) {
                best = Some((p, path, folder.clone()));
            }
        }
    }
    if let Some((p, _, folder)) = best {
        return pick(p, "folder", format!("The repo is inside {folder}."));
    }
    if list.len() == 1 {
        return pick(list[0], "only", format!("{} is your only profile.", list[0].label));
    }
    None
}

pub fn has_own_identity(identity: &Identity) -> bool {
    identity.local_name.is_some() && identity.local_email.is_some()
}

pub fn identity_drift(profile: &Profile, identity: &Identity) -> Vec<String> {
    let mut out = Vec::new();
    let keep = identity.keep_identity && has_own_identity(identity);
    if !keep && identity.local_name.as_deref() != Some(profile.name.as_str()) {
        out.push("name".to_string());
    }
    if !keep && identity.local_email.as_deref() != Some(profile.email.as_str()) {
        out.push("email".to_string());
    }
    for host in repo_hosts(identity) {
        let account = profile.accounts.get(&host);
        let binding = identity.bindings.get(&host);
        match account {
            Some(user) => {
                if binding.and_then(|b| b.username.as_ref()) != Some(user)
                    || !binding.is_some_and(|b| b.helper_installed)
                {
                    out.push(format!("{host} account"));
                }
            }
            None => {
                if binding.is_some_and(|b| b.ours) {
                    out.push(format!("{host} account"));
                }
            }
        }
    }
    out
}

pub fn global_warnings(identity: &Identity) -> Vec<String> {
    if identity.fallback_name.is_none() && identity.fallback_email.is_none() {
        return Vec::new();
    }
    let who: Vec<String> = [
        identity.fallback_name.clone(),
        identity.fallback_email.clone().map(|e| format!("<{e}>")),
    ]
    .into_iter()
    .flatten()
    .collect();
    vec![format!(
        "Your global git config has a name or email: {}. Terminals and agents would commit with it. Remove it with git config --global --unset user.name and git config --global --unset user.email.",
        who.join(" ")
    )]
}

pub fn repo_warnings(identity: &Identity, profile: Option<&Profile>) -> Vec<String> {
    let mut out = Vec::new();
    if profile.is_none() && !has_own_identity(identity) {
        out.push("This repo has no name and email of its own. Pick a profile. Your global git name and email are never used.".to_string());
    }
    for r in &identity.remotes {
        if r.token_in_url {
            out.push(format!(
                "The {} URL has a password or token in it. Remove it from the URL and use an account instead.",
                r.name
            ));
        }
        let users = unique(
            [r.user.clone(), r.push_user.clone()]
                .into_iter()
                .flatten()
                .filter(|u| !looks_like_token(u))
                .collect(),
        );
        for user in users {
            let account = profile.and_then(|p| r.host.as_ref().and_then(|h| p.accounts.get(h)));
            if let (Some(account), Some(p)) = (account, profile)
                && lower(account) != lower(&user)
            {
                out.push(format!(
                    "The {} URL names the user {user}. Git will sign in as {user}, not as {account} from {}. Remove \"{user}@\" from the URL to fix it.",
                    r.name, p.label
                ));
            }
        }
    }
    out
}

fn unset_local(env: &Env, root: &Path, key: &str) -> Result<(), String> {
    let out = git_str(env, root, &["config", "--local", "--unset-all", key]);
    match out.code {
        Some(0) | Some(5) => Ok(()),
        _ => Err({
            let why = out.stderr.trim();
            if why.is_empty() { format!("cannot clear {key}") } else { why.to_string() }
        }),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stamped {
    pub bound: Vec<(String, String)>,
    pub unbound: Vec<String>,
}

pub fn stamp_repo(env: &Env, root: &Path, profile: &Profile, keep_identity: bool) -> Result<Stamped, String> {
    let p = normalize_profile(profile);
    if p.id.is_empty() {
        return Err("This profile has no id. Save it first.".into());
    }
    if let Some(problem) = validate_profile(&p.label, &p.name, &p.email) {
        return Err(problem.into());
    }
    let writes: Vec<(&str, &str)> = if keep_identity {
        vec![(KEY_PROFILE, p.id.as_str()), (KEY_KEEP, "true")]
    } else {
        vec![("user.name", p.name.as_str()), ("user.email", p.email.as_str()), (KEY_PROFILE, p.id.as_str())]
    };
    for (key, value) in writes {
        git_ok(env, root, &["config", "--local", key, value])?;
    }
    if !keep_identity {
        unset_local(env, root, KEY_KEEP)?;
    }
    unset_local(env, root, KEY_UNMANAGED)?;
    let identity = read_repo_identity(env, root)?;
    let mut done = Stamped::default();
    for host in repo_hosts(&identity) {
        match p.accounts.get(&host) {
            Some(user) => {
                helper::bind_repo(env, root, &host, user)?;
                done.bound.push((host, user.clone()));
            }
            None => {
                if identity.bindings.get(&host).is_some_and(|b| b.ours) {
                    helper::unbind_repo(env, root, &host)?;
                    done.unbound.push(host);
                }
            }
        }
    }
    Ok(done)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ensured {
    pub status: &'static str,
    pub profile: Option<Profile>,
    pub reason: String,
    pub warnings: Vec<String>,
    pub fixed: Vec<String>,
    pub identity: Identity,
    pub kept_identity: bool,
    pub no_profiles: bool,
}

fn ensured(status: &'static str, profile: Option<Profile>, reason: String, warnings: Vec<String>, identity: Identity) -> Ensured {
    Ensured {
        status,
        profile,
        reason,
        warnings,
        fixed: Vec::new(),
        identity,
        kept_identity: false,
        no_profiles: false,
    }
}

pub fn ensure_identity(env: &Env, root: &Path, profiles: &[Profile]) -> Result<Ensured, String> {
    let identity = read_repo_identity(env, root)?;
    let own = has_own_identity(&identity);
    if identity.unmanaged {
        if own {
            return Ok(ensured("ok", None, "The app does not manage this repo.".into(), Vec::new(), identity));
        }
        let warnings = repo_warnings(&identity, None);
        return Ok(ensured("ask", None, "This repo has no name and email of its own.".into(), warnings, identity));
    }
    let bound = identity
        .profile_id
        .as_ref()
        .and_then(|id| profiles.iter().find(|p| &p.id == id))
        .cloned();
    if let Some(bound) = bound {
        let fixed = identity_drift(&bound, &identity);
        let warnings = repo_warnings(&identity, Some(&bound));
        if fixed.is_empty() {
            let reason = format!("This repo uses {}.", bound.label);
            return Ok(ensured("ok", Some(bound), reason, warnings, identity));
        }
        stamp_repo(env, root, &bound, identity.keep_identity && own)?;
        let reason = format!("Set the {} again to match {}.", fixed.join(", "), bound.label);
        let mut out = ensured("stamped", Some(bound), reason, warnings, identity);
        out.fixed = fixed;
        return Ok(out);
    }
    if profiles.is_empty() {
        let warnings = repo_warnings(&identity, None);
        let mut out = ensured("ask", None, "There are no profiles yet. Make one for this repo.".into(), warnings, identity);
        out.no_profiles = true;
        return Ok(out);
    }
    let Some(suggestion) = suggest_profile(profiles, &identity, root, &env.home) else {
        let warnings = repo_warnings(&identity, None);
        return Ok(ensured("ask", None, "No profile matches this repo.".into(), warnings, identity));
    };
    let profile = suggestion.profile;
    let clash = identity
        .local_email
        .as_ref()
        .is_some_and(|e| lower(e) != lower(&profile.email));
    let own_email = clash
        && identity.local_email.as_deref().map(lower) != identity.fallback_email.as_deref().map(lower);
    if clash && own_email && own && profiles.len() == 1 && identity.profile_id.is_none() {
        stamp_repo(env, root, &profile, true)?;
        let reason = format!(
            "{} is your only profile. This repo keeps its own email {}.",
            profile.label,
            identity.local_email.clone().unwrap_or_default()
        );
        let warnings = repo_warnings(&identity, Some(&profile));
        let mut out = ensured("stamped", Some(profile), reason, warnings, identity);
        out.fixed = vec!["profile".into()];
        out.kept_identity = true;
        return Ok(out);
    }
    if clash {
        let mut warnings = vec![format!(
            "This repo has its own email {}, but {} uses {}. Pick the right profile.",
            identity.local_email.clone().unwrap_or_default(),
            profile.label,
            profile.email
        )];
        warnings.extend(repo_warnings(&identity, Some(&profile)));
        return Ok(ensured("ask", Some(profile), suggestion.reason, warnings, identity));
    }
    stamp_repo(env, root, &profile, false)?;
    let warnings = repo_warnings(&identity, Some(&profile));
    let mut out = ensured("stamped", Some(profile), suggestion.reason, warnings, identity);
    out.fixed = vec!["profile".into()];
    Ok(out)
}

pub fn clear_repo(env: &Env, root: &Path) -> Result<Vec<String>, String> {
    let identity = read_repo_identity(env, root)?;
    unset_local(env, root, KEY_PROFILE)?;
    unset_local(env, root, KEY_KEEP)?;
    git_ok(env, root, &["config", "--local", KEY_UNMANAGED, "true"])?;
    let hosts: Vec<String> = identity
        .bindings
        .iter()
        .filter(|(_, b)| b.ours || b.helper_installed)
        .map(|(h, _)| h.clone())
        .collect();
    for host in &hosts {
        helper::unbind_repo(env, root, host)?;
    }
    Ok(hosts)
}

pub fn describe_accounts(profile: &Profile) -> String {
    profile
        .accounts
        .iter()
        .map(|(h, u)| if h == DEFAULT_HOST { u.clone() } else { format!("{u} on {h}") })
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn profile_summary(profile: &Profile) -> String {
    let who = format!("{} <{}>", profile.name, profile.email);
    let account = describe_accounts(profile);
    if account.is_empty() { who } else { format!("{who} · {account}") }
}

pub fn split_lines(value: &str) -> Vec<String> {
    value
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn split_list(value: &str) -> Vec<String> {
    value
        .split([',', '\n'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::testkit;

    fn work() -> Profile {
        Profile {
            id: "work".into(),
            label: "Work".into(),
            name: "Ana Work".into(),
            email: "ana@corp.com".into(),
            accounts: BTreeMap::from([("github.com".to_string(), "ana-corp".to_string())]),
            folders: vec!["~/Projects/corp".into()],
            owners: vec!["corp-org".into()],
            color: "ok".into(),
        }
    }

    fn home() -> Profile {
        Profile {
            id: "home".into(),
            label: "Home".into(),
            name: "Ana".into(),
            email: "ana@home.org".into(),
            accounts: BTreeMap::from([("github.com".to_string(), "ana".to_string())]),
            color: "info".into(),
            ..Profile::default()
        }
    }

    #[test]
    fn ids_and_checks() {
        assert_eq!(profile_id("Work Stuff!", &[]), "work-stuff");
        assert_eq!(profile_id("Work", &["work".into()]), "work-2");
        assert_eq!(profile_id("Café", &[]), "cafe");
        assert_eq!(profile_id("!!!", &[]), "profile");
        assert_eq!(validate_profile("", "a", "b@c"), Some("Give the profile a label, like Work or Personal."));
        assert_eq!(validate_profile("W", "a", "bad"), Some("That email does not look right."));
        assert_eq!(validate_profile("W", "a", "b@c"), None);
        assert_eq!(next_color(&[work()]), "info");
    }

    #[test]
    fn remote_urls_are_read() {
        let r = parse_remote_url("https://ana@github.com/owner/repo.git");
        assert_eq!(r.host.as_deref(), Some("github.com"));
        assert_eq!(r.owner.as_deref(), Some("owner"));
        assert_eq!(r.repo.as_deref(), Some("repo"));
        assert_eq!(r.user.as_deref(), Some("ana"));
        assert!(!r.token_in_url);
        let t = parse_remote_url("https://ghp_abc@github.com/o/r");
        assert!(t.token_in_url);
        assert_eq!(t.user, None);
        let s = parse_remote_url("git@github.com:group/sub/repo.git");
        assert_eq!(s.protocol.as_deref(), Some("ssh"));
        assert_eq!(s.owner.as_deref(), Some("group/sub"));
        assert_eq!(parse_remote_url("file:///x/y"), RemoteUrl::default());
        assert_eq!(parse_remote_url("C:/x"), RemoteUrl::default());
        let list = parse_remote_list("up\thttps://github.com/a/b (fetch)\nup\thttps://github.com/a/b (push)\norigin\tgit@github.com:me/b.git (fetch)\norigin\tgit@github.com:me/b.git (push)\n");
        assert_eq!(list[0].name, "origin");
        assert_eq!(list[1].owner.as_deref(), Some("a"));
    }

    #[test]
    fn suggestions_follow_the_rules() {
        let homedir = Path::new("/home/demo");
        let profiles = [work(), home()];
        let mut id = Identity {
            remotes: parse_remote_list("origin\thttps://github.com/corp-org/app (fetch)\n"),
            ..Identity::default()
        };
        let s = suggest_profile(&profiles, &id, Path::new("/x"), homedir).unwrap();
        assert_eq!((s.profile.id.as_str(), s.rule), ("work", "owner"));
        id.local_email = Some("ANA@home.org".into());
        let s = suggest_profile(&profiles, &id, Path::new("/x"), homedir).unwrap();
        assert_eq!(s.rule, "email");
        assert_eq!(s.profile.id, "home");
        let id = Identity::default();
        let s = suggest_profile(&profiles, &id, Path::new("/home/demo/Projects/corp/app"), homedir).unwrap();
        assert_eq!(s.rule, "folder");
        assert!(suggest_profile(&profiles, &id, Path::new("/tmp/x"), homedir).is_none());
        let s = suggest_profile(&[home()], &id, Path::new("/tmp/x"), homedir).unwrap();
        assert_eq!(s.rule, "only");
    }

    #[test]
    fn stamp_sets_identity_and_sign_in() {
        let kit = testkit::kit();
        let env = &kit.env;
        let repo = kit.repo("app");
        kit.git(&repo, &["remote", "add", "origin", "https://ana-corp@github.com/corp-org/app.git"]);
        let done = stamp_repo(env, &repo, &work(), false).unwrap();
        assert_eq!(done.bound, [("github.com".to_string(), "ana-corp".to_string())]);
        let id = read_repo_identity(env, &repo).unwrap();
        assert_eq!(id.profile_id.as_deref(), Some("work"));
        assert_eq!(id.local_email.as_deref(), Some("ana@corp.com"));
        assert!(id.bindings["github.com"].helper_installed);
        assert!(identity_drift(&work(), &id).is_empty());
        let warn = repo_warnings(&id, Some(&home()));
        assert!(warn.iter().any(|w| w.contains("names the user ana-corp")));
        let hosts = clear_repo(env, &repo).unwrap();
        assert_eq!(hosts, ["github.com"]);
        let id = read_repo_identity(env, &repo).unwrap();
        assert!(id.unmanaged);
        assert_eq!(id.profile_id, None);
        assert_eq!(id.local_email.as_deref(), Some("ana@corp.com"));
    }

    #[test]
    fn ensure_identity_stamps_asks_and_repairs() {
        let kit = testkit::kit();
        let env = &kit.env;
        let repo = kit.repo("app");
        kit.git(&repo, &["remote", "add", "origin", "https://github.com/corp-org/app.git"]);
        let none = ensure_identity(env, &repo, &[]).unwrap();
        assert_eq!(none.status, "ask");
        assert!(none.no_profiles);
        let first = ensure_identity(env, &repo, &[work(), home()]).unwrap();
        assert_eq!(first.status, "stamped");
        assert_eq!(first.profile.unwrap().id, "work");
        let again = ensure_identity(env, &repo, &[work(), home()]).unwrap();
        assert_eq!(again.status, "ok");
        kit.git(&repo, &["config", "--local", "user.email", "other@x.org"]);
        let fixed = ensure_identity(env, &repo, &[work(), home()]).unwrap();
        assert_eq!(fixed.status, "stamped");
        assert_eq!(fixed.fixed, ["email"]);
        let other = kit.repo("other");
        kit.git(&other, &["config", "--local", "user.name", "X"]);
        kit.git(&other, &["config", "--local", "user.email", "x@y.org"]);
        let keep = ensure_identity(env, &other, &[home()]).unwrap();
        assert!(keep.kept_identity);
        let id = read_repo_identity(env, &other).unwrap();
        assert!(id.keep_identity);
        assert_eq!(id.local_email.as_deref(), Some("x@y.org"));
    }

    #[test]
    fn profiles_save_and_load_with_ids() {
        let kit = testkit::kit();
        let env = &kit.env;
        let mut a = work();
        a.id.clear();
        let mut b = work();
        b.id.clear();
        let saved = save_profiles(env, &[a, b]).unwrap();
        assert_eq!(saved[0].id, "work");
        assert_eq!(saved[1].id, "work-2");
        assert_eq!(load_profiles(env), saved);
        let opts = account_options(&[], &saved[0].accounts);
        assert_eq!(opts[0].username, "ana-corp");
        assert_eq!(profile_summary(&saved[0]), "Ana Work <ana@corp.com> · ana-corp");
    }
}
