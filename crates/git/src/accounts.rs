pub mod helper;
pub mod profiles;
pub mod repo;
pub mod run;
pub mod settings;
pub mod time;

#[cfg(test)]
pub mod testkit;

use crate::keychain;
use regex::Regex;
use run::Opts;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const SERVICE: &str = "Nixa Git";
pub const DEFAULT_HOST: &str = "github.com";
pub const STATUSES: &[&str] = &["ok", "unverified", "expired", "revoked", "error"];
pub const TOKEN_SCOPES: &[&str] = &["repo", "read:org", "workflow"];
pub const GH_CANDIDATES: &[&str] = &["/opt/homebrew/bin/gh", "/usr/local/bin/gh", "/usr/bin/gh"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Env {
    pub dir: PathBuf,
    pub home: PathBuf,
    pub service: String,
    pub security: PathBuf,
    pub curl: PathBuf,
    pub git: PathBuf,
    pub gh: Option<PathBuf>,
    pub git_env: Vec<(String, String)>,
}

pub fn find_gh() -> Option<PathBuf> {
    let from_path = std::env::var("PATH").ok().and_then(|path| {
        path.split(':')
            .filter(|d| d.starts_with('/'))
            .map(|d| Path::new(d).join("gh"))
            .find(|p| executable(p))
    });
    from_path.or_else(|| {
        GH_CANDIDATES
            .iter()
            .map(PathBuf::from)
            .find(|p| executable(p))
    })
}

pub fn executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

pub fn env_at(data_dir: &Path) -> Env {
    Env {
        dir: data_dir.join("git"),
        home: std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/")),
        service: SERVICE.into(),
        security: PathBuf::from("/usr/bin/security"),
        curl: PathBuf::from("/usr/bin/curl"),
        git: PathBuf::from("git"),
        gh: find_gh(),
        git_env: Vec::new(),
    }
}

pub fn accounts_file(env: &Env) -> PathBuf {
    env.dir.join("accounts.json")
}

pub fn profiles_file(env: &Env) -> PathBuf {
    env.dir.join("profiles.json")
}

pub fn bin_dir(env: &Env) -> PathBuf {
    env.dir.join("bin")
}

pub fn ensure_dir(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).ok();
    Ok(())
}

pub fn write_private(path: &Path, text: &str, mode: u32) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        ensure_dir(dir)?;
    }
    if fs::read_to_string(path).ok().as_deref() == Some(text) {
        return Ok(());
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(mode)
        .open(&tmp)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    file.write_all(text.as_bytes())
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    drop(file);
    fs::set_permissions(&tmp, fs::Permissions::from_mode(mode)).ok();
    fs::rename(&tmp, path).map_err(|e| format!("cannot save {}: {e}", path.display()))
}

pub fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

pub fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    write_private(path, &format!("{text}\n"), 0o600)
}

pub fn list_of(value: Option<Value>, key: &str) -> Vec<Value> {
    match value {
        Some(Value::Array(list)) => list,
        Some(Value::Object(mut map)) => match map.remove(key) {
            Some(Value::Array(list)) => list,
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Account {
    pub host: String,
    pub username: String,
    pub name: String,
    pub avatar_url: String,
    pub default: bool,
    pub source: String,
    pub status: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<String>,
    pub checked_at: Option<String>,
    pub message: Option<String>,
}

pub fn normalize_host(value: &str) -> String {
    let mut h = value.trim().to_lowercase();
    if let Some(at) = h.find("://") {
        let scheme = &h[..at];
        if !scheme.is_empty()
            && scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
        {
            h = h[at + 3..].to_string();
        }
    }
    let first_slash = h.find('/').unwrap_or(h.len());
    if let Some(at) = h[..first_slash].find('@') {
        h = h[at + 1..].to_string();
    }
    h.split('/').next().unwrap_or("").to_string()
}

pub fn bare_host(host: &str) -> String {
    normalize_host(host).split(':').next().unwrap_or("").to_string()
}

pub fn same_host(a: &str, b: &str) -> bool {
    let (x, y) = (normalize_host(a), normalize_host(b));
    x == y || bare_host(&x) == y || x == bare_host(&y)
}

pub fn same_user(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

pub fn same_account(a: &Account, b: &Account) -> bool {
    same_host(&a.host, &b.host) && same_user(&a.username, &b.username)
}

pub fn account_id(a: &Account) -> String {
    format!("{}:{}", normalize_host(&a.host), a.username)
}

pub fn host_label(host: &str) -> String {
    if bare_host(host) == DEFAULT_HOST {
        "GitHub".into()
    } else {
        normalize_host(host)
    }
}

pub fn api_base(host: &str) -> String {
    let h = normalize_host(host);
    let h = if h.is_empty() { DEFAULT_HOST.to_string() } else { h };
    if h == "github.com" || h == "www.github.com" {
        "https://api.github.com".into()
    } else {
        format!("https://{h}/api/v3")
    }
}

pub fn token_page_url(host: &str, name: &str) -> String {
    let bare = bare_host(host);
    let bare = if bare.is_empty() { DEFAULT_HOST.to_string() } else { bare };
    let name: String = name
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    format!(
        "https://{bare}/settings/tokens/new?scopes={}&description={name}",
        TOKEN_SCOPES.join(",")
    )
}

pub fn normalize_account(a: &Account) -> Account {
    let host = normalize_host(&a.host);
    Account {
        host: if host.is_empty() { DEFAULT_HOST.into() } else { host },
        username: a.username.trim().to_string(),
        name: a.name.clone(),
        avatar_url: a.avatar_url.clone(),
        default: a.default,
        source: if a.source == "gh" { "gh".into() } else { "paste".into() },
        status: if STATUSES.contains(&a.status.as_str()) {
            a.status.clone()
        } else {
            "unverified".into()
        },
        scopes: a.scopes.iter().filter(|s| !s.is_empty()).cloned().collect(),
        expires_at: a.expires_at.clone().filter(|s| !s.is_empty()),
        checked_at: a.checked_at.clone().filter(|s| !s.is_empty()),
        message: a.message.clone().filter(|s| !s.is_empty()),
    }
}

pub fn ensure_defaults(accounts: &[Account]) -> Vec<Account> {
    let mut seen = Vec::new();
    let mut list: Vec<Account> = Vec::new();
    for raw in accounts {
        let a = normalize_account(raw);
        let key = format!("{}\n{}", a.host, a.username.to_lowercase());
        if a.username.is_empty() || seen.contains(&key) {
            continue;
        }
        seen.push(key);
        list.push(a);
    }
    let mut hosts: Vec<String> = Vec::new();
    for a in list.iter_mut() {
        if a.default && !hosts.contains(&a.host) {
            hosts.push(a.host.clone());
        } else {
            a.default = false;
        }
    }
    for a in list.iter_mut() {
        if !hosts.contains(&a.host) {
            hosts.push(a.host.clone());
            a.default = true;
        }
    }
    list
}

pub fn upsert(accounts: &[Account], account: Account) -> Vec<Account> {
    match accounts.iter().position(|a| same_account(a, &account)) {
        None => {
            let first = !accounts.iter().any(|a| same_host(&a.host, &account.host));
            let mut out = accounts.to_vec();
            out.push(Account {
                default: first,
                ..account
            });
            out
        }
        Some(index) => accounts
            .iter()
            .enumerate()
            .map(|(i, a)| {
                if i == index {
                    Account {
                        default: a.default,
                        ..account.clone()
                    }
                } else {
                    a.clone()
                }
            })
            .collect(),
    }
}

pub fn account_for<'a>(accounts: &'a [Account], host: &str, username: Option<&str>) -> Option<&'a Account> {
    let mut list = accounts.iter().filter(|a| same_host(&a.host, host));
    match username {
        Some(user) if !user.is_empty() => list.find(|a| same_user(&a.username, user)),
        _ => {
            let all: Vec<&Account> = list.collect();
            all.iter().find(|a| a.default).copied().or(all.first().copied())
        }
    }
}

pub fn load_accounts(env: &Env) -> Vec<Account> {
    let list: Vec<Account> = list_of(read_json(&accounts_file(env)), "accounts")
        .into_iter()
        .filter(|v| v.is_object())
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect();
    ensure_defaults(&list)
}

pub fn save_accounts(env: &Env, accounts: &[Account]) -> Result<Vec<Account>, String> {
    let list = ensure_defaults(accounts);
    write_json(&accounts_file(env), &json!({"version": 1, "accounts": list}))?;
    helper::write_defaults(env, &list)?;
    helper::ensure_helper(env)?;
    Ok(list)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GhLogin {
    pub host: String,
    pub username: String,
    pub active: bool,
    pub ok: bool,
}

pub fn gh_logins_from_json(text: &str) -> Option<Vec<GhLogin>> {
    let data: Value = serde_json::from_str(text).ok()?;
    let hosts = data.get("hosts")?.as_object()?;
    let mut out = Vec::new();
    for (host, entries) in hosts {
        for e in entries.as_array().into_iter().flatten() {
            let Some(login) = e.get("login").and_then(Value::as_str).filter(|s| !s.is_empty()) else {
                continue;
            };
            let h = e.get("host").and_then(Value::as_str).unwrap_or(host);
            out.push(GhLogin {
                host: normalize_host(if h.is_empty() { host } else { h }),
                username: login.to_string(),
                active: e.get("active").and_then(Value::as_bool).unwrap_or(false),
                ok: e.get("state").and_then(Value::as_str).is_none_or(|s| s == "success"),
            });
        }
    }
    Some(out)
}

pub fn gh_logins_from_text(text: &str) -> Vec<GhLogin> {
    let login = Regex::new(r"(?i)(Logged in to|Failed to log in to) (\S+) (?:account|as) ([A-Za-z0-9._-]+)").expect("login pattern");
    let active = Regex::new(r"(?i)Active account:\s*(true|false)").expect("active pattern");
    let mut out: Vec<GhLogin> = Vec::new();
    for line in text.lines() {
        if let Some(m) = login.captures(line) {
            out.push(GhLogin {
                host: normalize_host(&m[2]),
                username: m[3].to_string(),
                active: false,
                ok: m[1].to_lowercase().starts_with("logged"),
            });
            continue;
        }
        if let (Some(m), Some(last)) = (active.captures(line), out.last_mut()) {
            last.active = m[1].eq_ignore_ascii_case("true");
        }
    }
    out
}

fn unique_logins(logins: Vec<GhLogin>) -> Vec<GhLogin> {
    let mut out: Vec<GhLogin> = Vec::new();
    for l in logins {
        if !out.iter().any(|x| same_host(&x.host, &l.host) && same_user(&x.username, &l.username)) {
            out.push(l);
        }
    }
    out
}

fn short_opts() -> Opts {
    Opts {
        timeout: Some(Duration::from_secs(30)),
        ..Opts::default()
    }
}

pub fn list_gh_logins(env: &Env) -> Vec<GhLogin> {
    let Some(gh) = env.gh.as_ref() else {
        return Vec::new();
    };
    let json = run::run(gh, &run::strings(&["auth", "status", "--json", "hosts"]), &short_opts());
    if json.ok()
        && let Some(list) = gh_logins_from_json(&json.stdout)
    {
        return unique_logins(list);
    }
    let text = run::run(gh, &run::strings(&["auth", "status"]), &short_opts());
    unique_logins(gh_logins_from_text(&format!("{}\n{}", text.stdout, text.stderr)))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Http {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: String,
}

pub fn parse_http(text: &str) -> Http {
    let status_line = Regex::new(r"^HTTP/\S+\s+(\d{3})").expect("status pattern");
    let mut rest = text.to_string();
    let mut out = Http::default();
    while let Some(m) = status_line.captures(&rest) {
        let status = m[1].parse().unwrap_or(0);
        let (block, after) = match (rest.find("\r\n\r\n"), rest.find("\n\n")) {
            (Some(a), Some(b)) if b < a => (rest[..b].to_string(), rest[b + 2..].to_string()),
            (Some(a), _) => (rest[..a].to_string(), rest[a + 4..].to_string()),
            (None, Some(b)) => (rest[..b].to_string(), rest[b + 2..].to_string()),
            (None, None) => (rest.clone(), String::new()),
        };
        out.status = status;
        out.headers.clear();
        for line in block.lines().skip(1) {
            if let Some(i) = line.find(':').filter(|i| *i > 0) {
                out.headers.insert(
                    line[..i].trim().to_lowercase(),
                    line[i + 1..].trim().to_string(),
                );
            }
        }
        rest = after;
    }
    out.body = rest;
    out
}

pub fn parse_scopes(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn missing_scopes(account: &Account) -> Vec<&'static str> {
    if account.scopes.is_empty() {
        return Vec::new();
    }
    TOKEN_SCOPES
        .iter()
        .copied()
        .filter(|need| {
            let covers: &[&str] = if *need == "read:org" {
                &["read:org", "write:org", "admin:org"]
            } else {
                std::slice::from_ref(need)
            };
            !covers.iter().any(|c| account.scopes.iter().any(|s| s == c))
        })
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reply {
    pub http: Http,
    pub missing: bool,
    pub error: Option<String>,
    pub source: Option<&'static str>,
}

fn safe_api_url(url: &str) -> bool {
    Regex::new(r"^https://[A-Za-z0-9.-]+(?::\d+)?/[^\s\x00]*$")
        .expect("url pattern")
        .is_match(url)
}

fn api_user(user: &str) -> bool {
    Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")
        .expect("user pattern")
        .is_match(user)
}

pub fn curl_get(env: &Env, url: &str, token: &str) -> Result<Http, String> {
    let args = run::strings(&[
        "-sS",
        "-i",
        "--max-time",
        "20",
        "-H",
        "@-",
        "-H",
        "Accept: application/vnd.github+json",
        "-H",
        "User-Agent: Nixa",
        "-H",
        "X-GitHub-Api-Version: 2022-11-28",
        "--",
        url,
    ]);
    let out = run::run(
        &env.curl,
        &args,
        &Opts {
            stdin: Some(format!("Authorization: Bearer {token}\n")),
            timeout: Some(Duration::from_secs(30)),
            ..Opts::default()
        },
    );
    if !out.ok() {
        let why = run::last_line(&out.stderr);
        let why = Regex::new(r"^curl: \(\d+\)\s*").expect("curl pattern").replace(&why, "").into_owned();
        return Err(if why.is_empty() {
            format!("no answer, code {}", out.code.map(|c| c.to_string()).unwrap_or("none".into()))
        } else {
            why
        });
    }
    Ok(parse_http(&out.stdout))
}

pub fn api_get(env: &Env, host: &str, username: &str, url: &str) -> Reply {
    let host = {
        let h = normalize_host(host);
        if h.is_empty() { DEFAULT_HOST.to_string() } else { h }
    };
    let username = username.trim();
    if !api_user(username) {
        return Reply {
            missing: true,
            ..Reply::default()
        };
    }
    if !safe_api_url(url) {
        return Reply {
            error: Some("That address is not a safe https address.".into()),
            ..Reply::default()
        };
    }
    let saved = keychain::read_token(env, &host, username).ok().flatten();
    let (token, source) = match saved {
        Some(t) => (t, "keychain"),
        None => {
            return Reply {
                missing: true,
                ..Reply::default()
            };
        }
    };
    match curl_get(env, url, &token) {
        Ok(http) => Reply {
            http,
            source: Some(source),
            ..Reply::default()
        },
        Err(e) => Reply {
            error: Some(e),
            source: Some(source),
            ..Reply::default()
        },
    }
}

fn body_str(body: &Value, key: &str) -> Option<String> {
    body.get(key).and_then(Value::as_str).map(str::to_string)
}

pub fn probe_account(env: &Env, account: &Account) -> Account {
    let a = normalize_account(account);
    let label = host_label(&a.host);
    let reply = api_get(env, &a.host, &a.username, &format!("{}/user", api_base(&a.host)));
    let checked_at = Some(time::now_iso());
    if reply.missing {
        return Account {
            status: "error".into(),
            checked_at,
            message: Some("No token found for this account. Reconnect it with a token.".into()),
            ..a
        };
    }
    if let Some(e) = &reply.error {
        return Account {
            message: Some(format!("Could not check right now. Cannot reach {}. ({e})", a.host)),
            ..a
        };
    }
    let body: Value = serde_json::from_str(&reply.http.body).unwrap_or(Value::Null);
    match reply.http.status {
        200 => {
            let login = body_str(&body, "login").unwrap_or(a.username.clone());
            if !same_user(&login, &a.username) {
                let message = format!(
                    "This token belongs to @{login}, not @{}. Reconnect with the right token.",
                    a.username
                );
                return Account {
                    status: "error".into(),
                    checked_at,
                    message: Some(message),
                    ..a
                };
            }
            let headers = &reply.http.headers;
            Account {
                name: body_str(&body, "name").unwrap_or(a.name.clone()),
                avatar_url: body_str(&body, "avatar_url").unwrap_or(a.avatar_url.clone()),
                status: "ok".into(),
                scopes: parse_scopes(headers.get("x-oauth-scopes").map(String::as_str).unwrap_or("")),
                expires_at: headers
                    .get("github-authentication-token-expiration")
                    .and_then(|e| time::parse_expiry(e)),
                checked_at,
                message: None,
                ..a
            }
        }
        401 => {
            let expired = time::is_past(a.expires_at.as_deref(), time::now());
            let message = if expired {
                "The token expired. Reconnect with a new token.".to_string()
            } else {
                format!("{label} refused the token. It may be expired or deleted. Reconnect with a new token.")
            };
            Account {
                status: if expired { "expired" } else { "revoked" }.into(),
                checked_at,
                message: Some(message),
                ..a
            }
        }
        403 => {
            let said = body_str(&body, "message").map(|m| format!(" {label} says: {m}")).unwrap_or_default();
            Account {
                status: "error".into(),
                checked_at,
                message: Some(format!("{label} blocked the check (403).{said}")),
                ..a
            }
        }
        code => {
            let code = if code == 0 { "none".to_string() } else { code.to_string() };
            Account {
                message: Some(format!("Could not check right now. {label} answered with code {code}.")),
                ..a
            }
        }
    }
}

fn merge_check(saved: &Account, r: &Account) -> Account {
    Account {
        name: r.name.clone(),
        avatar_url: r.avatar_url.clone(),
        status: r.status.clone(),
        scopes: r.scopes.clone(),
        expires_at: r.expires_at.clone(),
        checked_at: r.checked_at.clone(),
        message: r.message.clone(),
        ..saved.clone()
    }
}

pub fn check_accounts(env: &Env, accounts: &[Account]) -> Result<Vec<Account>, String> {
    let results: Vec<Account> = std::thread::scope(|s| {
        let jobs: Vec<_> = accounts
            .iter()
            .map(|a| s.spawn(move || probe_account(env, a)))
            .collect();
        jobs.into_iter()
            .zip(accounts)
            .map(|(j, a)| j.join().unwrap_or_else(|_| normalize_account(a)))
            .collect()
    });
    let current = load_accounts(env);
    if !results
        .iter()
        .any(|r| account_for(&current, &r.host, Some(&r.username)).is_some())
    {
        return Ok(results);
    }
    let merged: Vec<Account> = current
        .iter()
        .map(|a| match results.iter().find(|r| same_account(r, a)) {
            Some(r) => merge_check(a, r),
            None => a.clone(),
        })
        .collect();
    let saved = save_accounts(env, &merged)?;
    Ok(results
        .into_iter()
        .map(|r| account_for(&saved, &r.host, Some(&r.username)).cloned().unwrap_or(r))
        .collect())
}

pub fn check_account(env: &Env, account: &Account) -> Result<Account, String> {
    check_accounts(env, std::slice::from_ref(account))?
        .into_iter()
        .next()
        .ok_or_else(|| "The check gave no answer.".to_string())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddError {
    pub message: String,
    pub network: bool,
    pub needs_username: bool,
}

fn add_error(message: impl Into<String>) -> AddError {
    AddError {
        message: message.into(),
        network: false,
        needs_username: false,
    }
}

impl From<String> for AddError {
    fn from(message: String) -> Self {
        add_error(message)
    }
}

pub struct User {
    pub login: String,
    pub name: String,
    pub avatar_url: String,
}

fn fetch_user(env: &Env, host: &str, token: &str) -> Result<User, AddError> {
    let label = host_label(host);
    let http = curl_get(env, &format!("{}/user", api_base(host)), token).map_err(|_| AddError {
        message: format!("Cannot reach {host}. Check the network and try again."),
        network: true,
        needs_username: false,
    })?;
    let body: Value = serde_json::from_str(&http.body).unwrap_or(Value::Null);
    match http.status {
        200 => {}
        401 => {
            return Err(add_error(format!(
                "{label} did not accept this token. Check that you copied all of it and that it has not expired."
            )));
        }
        403 => {
            let said = body_str(&body, "message").map(|m| format!(" {label} says: {m}")).unwrap_or_default();
            return Err(add_error(format!("{label} refused this token (403).{said}")));
        }
        code => {
            return Err(add_error(format!(
                "{label} answered with code {code}. Try again in a moment."
            )));
        }
    }
    let login = body_str(&body, "login").filter(|l| !l.is_empty());
    let Some(login) = login else {
        return Err(add_error(format!("{label} did not say who owns this token.")));
    };
    Ok(User {
        login,
        name: body_str(&body, "name").unwrap_or_default(),
        avatar_url: body_str(&body, "avatar_url").unwrap_or_default(),
    })
}

fn clean_token(token: &str) -> Result<String, AddError> {
    let clean = token.trim();
    if clean.is_empty() {
        return Err(add_error("Paste a token first."));
    }
    if !keychain::valid_token(clean) {
        return Err(add_error("This does not look like a token. Copy it again and paste it."));
    }
    Ok(clean.to_string())
}

pub fn valid_username(value: &str) -> bool {
    Regex::new(r"^[A-Za-z0-9][A-Za-z0-9-]{0,38}$")
        .expect("username pattern")
        .is_match(value.trim())
}

fn save_offline(env: &Env, host: &str, token: &str, username: Option<&str>, error: AddError) -> Result<Account, AddError> {
    let name = username.unwrap_or("").trim().trim_start_matches('@').to_string();
    if name.is_empty() {
        return Err(AddError {
            message: format!(
                "{} Or type the user name of this token to save it now and check it later.",
                error.message
            ),
            network: true,
            needs_username: true,
        });
    }
    if !valid_username(&name) {
        return Err(AddError {
            message: "This does not look like a user name. Type it without spaces.".into(),
            network: true,
            needs_username: true,
        });
    }
    keychain::save_token(env, host, &name, token)?;
    let account = Account {
        host: host.into(),
        username: name.clone(),
        source: "paste".into(),
        status: "unverified".into(),
        message: Some(format!(
            "Saved without a check because {host} could not be reached. Press Check now when you are online."
        )),
        ..Account::default()
    };
    let saved = save_accounts(env, &upsert(&load_accounts(env), account))?;
    account_for(&saved, host, Some(&name))
        .cloned()
        .ok_or_else(|| add_error("The account was not saved."))
}

pub fn add_account(env: &Env, host: &str, token: &str, username: Option<&str>) -> Result<Account, AddError> {
    let h = {
        let h = normalize_host(host);
        if h.is_empty() { DEFAULT_HOST.to_string() } else { h }
    };
    let clean = clean_token(token)?;
    let user = match fetch_user(env, &h, &clean) {
        Ok(user) => user,
        Err(e) if e.network => return save_offline(env, &h, &clean, username, e),
        Err(e) => return Err(e),
    };
    keychain::save_token(env, &h, &user.login, &clean)?;
    let account = Account {
        host: h.clone(),
        username: user.login.clone(),
        name: user.name,
        avatar_url: user.avatar_url,
        source: "paste".into(),
        status: "ok".into(),
        checked_at: Some(time::now_iso()),
        ..Account::default()
    };
    let saved = save_accounts(env, &upsert(&load_accounts(env), account))?;
    let found = account_for(&saved, &h, Some(&user.login))
        .cloned()
        .ok_or_else(|| add_error("The account was not saved."))?;
    Ok(check_account(env, &found)?)
}

pub fn reconnect(env: &Env, account: &Account, token: &str) -> Result<Account, AddError> {
    let a = normalize_account(account);
    let clean = clean_token(token)?;
    let user = fetch_user(env, &a.host, &clean)?;
    if !same_user(&user.login, &a.username) {
        return Err(add_error(format!(
            "This token is for @{}, not @{}. Use a token from @{}, or add it as a new account.",
            user.login, a.username, a.username
        )));
    }
    keychain::save_token(env, &a.host, &a.username, &clean)?;
    let next = Account {
        name: if user.name.is_empty() { a.name.clone() } else { user.name },
        avatar_url: if user.avatar_url.is_empty() { a.avatar_url.clone() } else { user.avatar_url },
        source: "paste".into(),
        status: "ok".into(),
        checked_at: Some(time::now_iso()),
        message: None,
        ..a.clone()
    };
    let saved = save_accounts(env, &upsert(&load_accounts(env), next))?;
    let found = account_for(&saved, &a.host, Some(&a.username))
        .cloned()
        .ok_or_else(|| add_error("The account was not saved."))?;
    Ok(check_account(env, &found)?)
}

pub fn remove_account(env: &Env, account: &Account, delete_token: bool) -> Result<Vec<Account>, String> {
    let a = normalize_account(account);
    if delete_token {
        keychain::delete_token(env, &a.host, &a.username)?;
    }
    let left: Vec<Account> = load_accounts(env)
        .into_iter()
        .filter(|x| !same_account(x, &a))
        .collect();
    save_accounts(env, &left)
}

pub fn profiles_using<'a>(profiles: &'a [profiles::Profile], account: &Account) -> Vec<&'a profiles::Profile> {
    let a = normalize_account(account);
    profiles
        .iter()
        .filter(|p| {
            p.accounts
                .iter()
                .any(|(h, u)| same_host(h, &a.host) && same_user(u, &a.username))
        })
        .collect()
}

pub fn remove_text(
    account: &Account,
    accounts: &[Account],
    profiles: &[profiles::Profile],
    delete_token: bool,
    app: &str,
) -> Vec<String> {
    let a = normalize_account(account);
    let users = profiles_using(profiles, &a);
    let mut lines = vec![format!("Remove @{} from {app}?", a.username)];
    lines.push(if delete_token {
        format!(
            "Repos set to @{0} will not sign in with it any more. If gh still has a login for @{0}, git uses that one.",
            a.username
        )
    } else {
        format!(
            "The token stays in the Keychain, so repos set to @{} still sign in with it.",
            a.username
        )
    });
    if !users.is_empty() {
        let one = users.len() == 1;
        let names: Vec<String> = users
            .iter()
            .map(|p| {
                [&p.label, &p.name, &p.id]
                    .into_iter()
                    .find(|s| !s.is_empty())
                    .cloned()
                    .unwrap_or_default()
            })
            .collect();
        lines.push(format!(
            "{} {} {} @{}. Pick another account for {} in the Profiles tab.",
            if one { "The profile" } else { "The profiles" },
            names.join(", "),
            if one { "uses" } else { "use" },
            a.username,
            if one { "it" } else { "them" }
        ));
    }
    if a.default {
        let next = accounts
            .iter()
            .map(normalize_account)
            .find(|x| same_host(&x.host, &a.host) && !same_user(&x.username, &a.username));
        lines.push(match next {
            Some(n) => format!("@{} becomes the default for {}.", n.username, a.host),
            None => format!("No account will be the default for {}.", a.host),
        });
    }
    lines
}

pub fn set_default(env: &Env, host: &str, username: &str) -> Result<Vec<Account>, String> {
    let list = load_accounts(env);
    if account_for(&list, host, Some(username)).is_none() {
        return Err(format!("@{username} is not in your accounts."));
    }
    let next: Vec<Account> = list
        .into_iter()
        .map(|a| {
            if same_host(&a.host, host) {
                let default = same_user(&a.username, username);
                Account { default, ..a }
            } else {
                a
            }
        })
        .collect();
    save_accounts(env, &next)
}

pub fn token_for(env: &Env, host: &str, username: &str) -> Option<String> {
    let h = normalize_host(host);
    keychain::read_token(env, &h, username).ok().flatten()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Warn,
    Bad,
    Hint,
}

pub fn describe_status(account: &Account, now: i64) -> (Tone, String) {
    let a = normalize_account(account);
    match a.status.as_str() {
        "ok" => {
            if time::is_past(a.expires_at.as_deref(), now) {
                return (Tone::Bad, "Token expired".into());
            }
            match a.expires_at.as_deref().and_then(|e| time::days_left(e, now)) {
                Some(days) if days <= 14 => (
                    Tone::Warn,
                    format!(
                        "Works, but the token ends in {days} {}",
                        if days == 1 { "day" } else { "days" }
                    ),
                ),
                _ => (Tone::Ok, "Works".into()),
            }
        }
        "unverified" => (Tone::Hint, "Not checked yet".into()),
        "expired" => (Tone::Bad, "Token expired".into()),
        "revoked" => (Tone::Bad, format!("{} refused the token", host_label(&a.host))),
        _ => (Tone::Bad, "Has a problem".into()),
    }
}

pub fn expiry_text(account: &Account, now: i64) -> String {
    let a = normalize_account(account);
    match a.expires_at.as_deref() {
        None if a.status == "ok" => "No end date".into(),
        None => String::new(),
        Some(e) => format!(
            "{} {}",
            if time::is_past(Some(e), now) { "Ended" } else { "Ends" },
            time::format_date(e)
        ),
    }
}

pub fn checked_text(account: &Account, now: i64) -> String {
    let Some(at) = account.checked_at.as_deref() else {
        return "Never checked".into();
    };
    let Some(secs) = time::parse(at) else {
        return String::new();
    };
    let mins = (now - secs).div_euclid(60);
    if mins < 1 {
        "Checked just now".into()
    } else if mins < 60 {
        format!("Checked {mins} min ago")
    } else if mins < 1440 {
        format!("Checked {} h ago", mins / 60)
    } else {
        format!("Checked {}", time::format_date(at))
    }
}

pub fn for_remote(env: &Env) -> crate::remote::Accounts {
    let list = load_accounts(env)
        .iter()
        .map(|a| crate::remote::Account {
            host: a.host.clone(),
            username: a.username.clone(),
            default: a.default,
            status: a.status.clone(),
            has_token: crate::keychain::has_token(env, &a.host, &a.username).unwrap_or(false),
        })
        .collect();
    let helper = helper::helper_path(env);
    crate::remote::Accounts {
        list,
        helper: helper.is_file().then(|| helper.display().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acct(host: &str, user: &str, default: bool) -> Account {
        Account {
            host: host.into(),
            username: user.into(),
            default,
            ..Account::default()
        }
    }

    #[test]
    fn hosts_are_normalized_and_compared() {
        assert_eq!(normalize_host("HTTPS://user@GitHub.com/owner/repo"), "github.com");
        assert_eq!(normalize_host("git.example.com:8443/x"), "git.example.com:8443");
        assert!(same_host("github.com:443", "github.com"));
        assert!(!same_host("gitlab.com", "github.com"));
        assert_eq!(api_base("github.com"), "https://api.github.com");
        assert_eq!(api_base("ghe.corp"), "https://ghe.corp/api/v3");
        assert_eq!(host_label("github.com"), "GitHub");
        assert_eq!(
            token_page_url("github.com", "Nixa"),
            "https://github.com/settings/tokens/new?scopes=repo,read:org,workflow&description=Nixa"
        );
    }

    #[test]
    fn one_default_per_host() {
        let list = ensure_defaults(&[
            acct("github.com", "a", false),
            acct("github.com", "b", true),
            acct("github.com", "c", true),
            acct("ghe.corp", "d", false),
            acct("github.com", "A", false),
            acct("github.com", "", false),
        ]);
        let names: Vec<(&str, bool)> = list.iter().map(|a| (a.username.as_str(), a.default)).collect();
        assert_eq!(names, [("a", false), ("b", true), ("c", false), ("d", true)]);
        let added = upsert(&list, acct("ghe.corp", "e", false));
        assert!(!added.last().unwrap().default);
        let fresh = upsert(&[], acct("x.org", "z", false));
        assert!(fresh[0].default);
        assert_eq!(account_for(&list, "github.com", None).unwrap().username, "b");
        assert_eq!(account_for(&list, "github.com", Some("C")).unwrap().username, "c");
    }

    #[test]
    fn gh_logins_are_read() {
        let json = r#"{"hosts":{"github.com":[{"login":"ana","active":true,"state":"success"},{"login":"bob","active":false,"state":"error"}]}}"#;
        let list = gh_logins_from_json(json).unwrap();
        assert_eq!(list.len(), 2);
        assert!(list[0].active && list[0].ok);
        assert!(!list[1].ok);
        let text = "github.com\n  ✓ Logged in to github.com account ana (keyring)\n  - Active account: true\n  X Failed to log in to github.com account bob (keyring)";
        let list = gh_logins_from_text(text);
        assert_eq!(list.len(), 2);
        assert!(list[0].active);
        assert!(!list[1].ok);
    }

    #[test]
    fn http_replies_are_split() {
        let reply = parse_http("HTTP/1.1 100 Continue\r\n\r\nHTTP/2 200\r\nX-OAuth-Scopes: repo, workflow\r\n\r\n{\"a\":1}");
        assert_eq!(reply.status, 200);
        assert_eq!(reply.headers.get("x-oauth-scopes").unwrap(), "repo, workflow");
        assert_eq!(reply.body, "{\"a\":1}");
        let a = Account {
            scopes: parse_scopes("repo, workflow"),
            ..Account::default()
        };
        assert_eq!(missing_scopes(&a), ["read:org"]);
        let b = Account {
            scopes: parse_scopes("repo, admin:org, workflow"),
            ..Account::default()
        };
        assert!(missing_scopes(&b).is_empty());
    }

    #[test]
    fn add_check_default_and_remove() {
        let kit = testkit::kit();
        let env = &kit.env;
        kit.curl_reply("tok_ana", &testkit::user_reply("ana", "repo, read:org, workflow", "2099-01-01 00:00:00 UTC"));
        let added = add_account(env, "github.com", " tok_ana ", None).unwrap();
        assert_eq!(added.username, "ana");
        assert_eq!(added.status, "ok");
        assert_eq!(added.name, "Name ana");
        assert!(added.default);
        assert_eq!(added.expires_at.as_deref(), Some("2099-01-01T00:00:00Z"));
        assert!(!kit.curl_args().contains("tok_ana"));
        assert_eq!(token_for(env, "github.com", "ana").as_deref(), Some("tok_ana"));
        kit.curl_reply("tok_bob", &testkit::user_reply("bob", "repo", ""));
        add_account(env, "github.com", "tok_bob", None).unwrap();
        let list = set_default(env, "github.com", "bob").unwrap();
        assert!(list.iter().find(|a| a.username == "bob").unwrap().default);
        let text = remove_text(&list[1], &list, &[], true, "Nixa");
        assert_eq!(text[0], "Remove @bob from Nixa?");
        assert!(text.last().unwrap().contains("@ana becomes the default"));
        let left = remove_account(env, &list[1], true).unwrap();
        assert_eq!(left.len(), 1);
        assert!(left[0].default);
        assert_eq!(keychain::read_token(env, "github.com", "bob").unwrap(), None);
        let helper = bin_dir(env).join(helper::HELPER_NAME);
        assert!(executable(&helper));
        assert_eq!(fs::read_to_string(bin_dir(env).join("defaults")).unwrap(), "github.com=ana\n");
        let mode = fs::metadata(accounts_file(env)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn wrong_token_owner_and_bad_token_are_refused() {
        let kit = testkit::kit();
        let env = &kit.env;
        kit.curl_reply("tok_ana", &testkit::user_reply("ana", "repo", ""));
        let a = add_account(env, "github.com", "tok_ana", None).unwrap();
        kit.curl_reply("tok_eve", &testkit::user_reply("eve", "repo", ""));
        let err = reconnect(env, &a, "tok_eve").unwrap_err();
        assert!(err.message.contains("This token is for @eve, not @ana"));
        kit.curl_reply("tok_dead", "HTTP/2 401\r\n\r\n{}");
        let err = add_account(env, "github.com", "tok_dead", None).unwrap_err();
        assert!(err.message.contains("did not accept this token"));
        assert!(add_account(env, "github.com", "", None).unwrap_err().message.contains("Paste a token"));
    }

    #[test]
    fn offline_add_asks_for_the_name_then_saves() {
        let kit = testkit::kit();
        let env = &kit.env;
        let err = add_account(env, "github.com", "tok_offline", None).unwrap_err();
        assert!(err.needs_username);
        let saved = add_account(env, "github.com", "tok_offline", Some("@ana")).unwrap();
        assert_eq!(saved.username, "ana");
        assert_eq!(saved.status, "unverified");
        assert!(saved.message.unwrap().contains("could not be reached"));
    }

    #[test]
    fn probe_reports_each_answer() {
        let kit = testkit::kit();
        let env = &kit.env;
        let a = Account {
            host: "github.com".into(),
            username: "ana".into(),
            ..Account::default()
        };
        assert_eq!(probe_account(env, &a).status, "error");
        keychain::save_token(env, "github.com", "ana", "tok_x").unwrap();
        kit.curl_reply("tok_x", "HTTP/2 401\r\n\r\n{}");
        assert_eq!(probe_account(env, &a).status, "revoked");
        kit.curl_reply("tok_x", "HTTP/2 403\r\n\r\n{\"message\":\"rate limit\"}");
        let r = probe_account(env, &a);
        assert!(r.message.unwrap().contains("GitHub says: rate limit"));
        kit.curl_reply("tok_x", &testkit::user_reply("bob", "", ""));
        assert!(probe_account(env, &a).message.unwrap().contains("belongs to @bob"));
        keychain::delete_token(env, "github.com", "ana").unwrap();
        kit.gh_token("ana", "ghtok_x");
        kit.curl_reply("ghtok_x", &testkit::user_reply("ana", "", ""));
        let r = probe_account(env, &a);
        assert_eq!(r.status, "error");
        assert!(token_for(env, "github.com", "ana").is_none());
    }

    #[test]
    fn status_words() {
        let now = time::parse("2026-11-22T00:00:00Z").unwrap();
        let mut a = Account {
            host: "github.com".into(),
            username: "ana".into(),
            status: "ok".into(),
            ..Account::default()
        };
        assert_eq!(describe_status(&a, now), (Tone::Ok, "Works".into()));
        assert_eq!(expiry_text(&a, now), "No end date");
        a.expires_at = Some("2026-11-25T00:00:00Z".into());
        assert_eq!(describe_status(&a, now).0, Tone::Warn);
        assert_eq!(expiry_text(&a, now), "Ends 25 Nov 2026");
        a.expires_at = Some("2026-11-01T00:00:00Z".into());
        assert_eq!(describe_status(&a, now).1, "Token expired");
        a.status = "revoked".into();
        assert_eq!(describe_status(&a, now).1, "GitHub refused the token");
        assert_eq!(checked_text(&a, now), "Never checked");
        a.checked_at = Some("2026-11-21T23:30:00Z".into());
        assert_eq!(checked_text(&a, now), "Checked 30 min ago");
    }

    #[test]
    fn remote_accounts_come_from_the_saved_list() {
        let kit = testkit::kit();
        let env = &kit.env;
        assert!(for_remote(env).list.is_empty());
        save_accounts(env, &[acct("github.com", "ana", true)]).unwrap();
        keychain::save_token(env, "github.com", "ana", "tok_one").unwrap();
        let remote = for_remote(env);
        assert_eq!(remote.list.len(), 1);
        assert_eq!(remote.list[0].username, "ana");
        assert!(remote.list[0].default && remote.list[0].has_token);
        assert!(remote.helper.is_some());
    }
}
