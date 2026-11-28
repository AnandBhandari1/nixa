use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::LazyLock;
use std::time::Duration;

pub const KEYCHAIN_SERVICE: &str = "Nixa Git";
pub const API_ENV: &str = "NIXA_GITHUB_API";
pub const TIMEOUT: Duration = Duration::from_secs(30);
pub const BODY_CHARS: usize = 4000;
pub const OPEN_MAX: usize = 50;

static PR_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^https?://([^/]+)/([^/]+)/([^/]+)/pull/(\d+)(?:[/?#]|$)").expect("pr url regex"));
static HTTPS_REMOTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^https?://(?:[^@/]+@)?([^/:]+)(?::\d+)?/([^/]+)/([^/]+?)(?:\.git)?/?$").expect("https remote regex"));
static SSH_REMOTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:ssh://)?(?:[^@]+@)?([^:/]+)[:/]([^/]+)/([^/]+?)(?:\.git)?/?$").expect("ssh remote regex"));

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrRef {
    pub host: String,
    pub owner: String,
    pub repo: String,
    pub number: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoRef {
    pub host: String,
    pub owner: String,
    pub repo: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Head {
    pub sha: String,
    #[serde(rename = "ref")]
    pub branch: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub repo: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrFacts {
    pub number: u64,
    pub title: String,
    pub url: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub author: String,
    pub state: String,
    pub draft: bool,
    pub head: Head,
    pub base: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub changed_files: Option<u64>,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub body: String,
}

pub fn parse_pr_url(url: &str) -> Option<PrRef> {
    let found = PR_URL.captures(url.trim())?;
    Some(PrRef { host: found[1].to_lowercase(), owner: found[2].to_string(), repo: found[3].trim_end_matches(".git").to_string(), number: found[4].parse().ok()? })
}

pub fn parse_remote(url: &str) -> Option<RepoRef> {
    let value = url.trim();
    let found = HTTPS_REMOTE.captures(value).or_else(|| SSH_REMOTE.captures(value))?;
    Some(RepoRef { host: found[1].to_lowercase(), owner: found[2].to_string(), repo: found[3].to_string() })
}

pub fn pr_repo(pr: &PrRef) -> RepoRef {
    RepoRef { host: pr.host.clone(), owner: pr.owner.clone(), repo: pr.repo.clone() }
}

pub fn same_repo(a: Option<&RepoRef>, b: Option<&RepoRef>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.host == b.host && a.owner.eq_ignore_ascii_case(&b.owner) && a.repo.eq_ignore_ascii_case(&b.repo),
        _ => false,
    }
}

pub fn api_base(host: &str, given: Option<String>) -> String {
    let default = if host == "github.com" || host == "www.github.com" { "https://api.github.com".to_string() } else { format!("https://{host}/api/v3") };
    let allowed = |g: &str| {
        let Some(rest) = g.strip_prefix("https://").or_else(|| g.strip_prefix("http://")) else { return false };
        let given_host = rest.split(['/', '?', '#']).next().unwrap_or("").to_ascii_lowercase();
        if g.starts_with("http://") {
            return given_host == "127.0.0.1" || given_host.strip_prefix("127.0.0.1:").is_some_and(|port| !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()));
        }
        let default_host = default.trim_start_matches("https://").split('/').next().unwrap_or("").to_ascii_lowercase();
        given_host == default_host
    };
    match given.map(|g| g.trim().trim_end_matches('/').to_string()).filter(|g| allowed(g)) {
        Some(base) => base,
        None => default,
    }
}

pub fn keychain_account(host: &str, user: &str) -> String {
    format!("acct:{host}:{user}")
}

pub async fn keychain_token(host: &str, user: &str) -> Option<String> {
    let out = tokio::process::Command::new("/usr/bin/security").args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-a", &keychain_account(host, user), "-w"]).kill_on_drop(true).output();
    let out = tokio::time::timeout(Duration::from_secs(10), out).await.ok()?.ok()?;
    let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !token.is_empty()).then_some(token)
}

fn text(value: Option<&Value>) -> String {
    value.and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string()
}

pub fn read_facts(value: &Value) -> Result<PrFacts, String> {
    if !value.is_object() {
        return Err("GitHub sent no pull request.".into());
    }
    let head = value.get("head").cloned().unwrap_or_default();
    let state = text(value.get("state"));
    Ok(PrFacts {
        number: value.get("number").and_then(Value::as_u64).unwrap_or(0),
        title: text(value.get("title")),
        url: text(value.get("html_url")),
        author: text(value.get("user").and_then(|u| u.get("login"))),
        state: if state.is_empty() { "open".into() } else { state },
        draft: value.get("draft").and_then(Value::as_bool) == Some(true),
        head: Head { sha: text(head.get("sha")), branch: text(head.get("ref")), repo: text(head.get("repo").and_then(|r| r.get("full_name"))) },
        base: text(value.get("base").and_then(|b| b.get("ref"))),
        changed_files: value.get("changed_files").and_then(Value::as_u64),
        body: crate::util::clip(&text(value.get("body")), BODY_CHARS),
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenPr {
    pub number: u64,
    pub title: String,
    pub url: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub author: String,
    pub created_at: String,
    pub draft: bool,
}

pub fn read_open(value: &Value) -> Vec<OpenPr> {
    value
        .as_array()
        .map(|list| {
            list.iter()
                .filter(|v| v.get("number").and_then(Value::as_u64).is_some())
                .map(|v| OpenPr {
                    number: v.get("number").and_then(Value::as_u64).unwrap_or(0),
                    title: text(v.get("title")),
                    url: text(v.get("html_url")),
                    author: text(v.get("user").and_then(|u| u.get("login"))),
                    created_at: text(v.get("created_at")),
                    draft: v.get("draft").and_then(Value::as_bool) == Some(true),
                })
                .collect()
        })
        .unwrap_or_default()
}

async fn get_json(http: &reqwest::Client, url: &str, host: &str, user: &str, what: &str) -> Result<Value, String> {
    let token = keychain_token(host, user).await.ok_or_else(|| format!("No GitHub token for {user} on {host}. Connect the account in Settings, Git accounts."))?;
    let reply = http
        .get(url)
        .bearer_auth(token)
        .header("accept", "application/vnd.github+json")
        .header("user-agent", "Nixa")
        .header("x-github-api-version", "2022-11-28")
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("GitHub did not answer for {what}: {e}"))?;
    let status = reply.status();
    let raw = reply.text().await.unwrap_or_default();
    let parsed: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    if !status.is_success() {
        let why = parsed.get("message").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| crate::util::clip(&raw, 200));
        return Err(format!("GitHub answered {} for {what}{}", status.as_u16(), if why.trim().is_empty() { String::new() } else { format!(": {why}") }));
    }
    Ok(parsed)
}

pub fn open_url(base: &str, repo: &RepoRef) -> String {
    format!("{base}/repos/{}/{}/pulls?state=open&per_page={OPEN_MAX}&sort=created&direction=asc", repo.owner, repo.repo)
}

pub async fn open_prs(http: &reqwest::Client, repo: &RepoRef, user: &str, base: &str) -> Result<Vec<OpenPr>, String> {
    let what = format!("{}/{}", repo.owner, repo.repo);
    Ok(read_open(&get_json(http, &open_url(base, repo), &repo.host, user, &what).await?))
}

pub fn facts_url(base: &str, pr: &PrRef) -> String {
    format!("{base}/repos/{}/{}/pulls/{}", pr.owner, pr.repo, pr.number)
}

pub async fn pr_facts(http: &reqwest::Client, pr: &PrRef, user: &str, base: &str) -> Result<PrFacts, String> {
    let what = format!("{}/{}#{}", pr.owner, pr.repo, pr.number);
    read_facts(&get_json(http, &facts_url(base, pr), &pr.host, user, &what).await?)
}

pub async fn facts_for(app: &crate::app::App, url: &str, folder: Option<&str>) -> Result<PrFacts, String> {
    let pr = parse_pr_url(url).ok_or("Give a pull request link like https://github.com/owner/repo/pull/12.")?;
    let user = match folder {
        Some(folder) => super::identity::prepare(&super::identity::prep_for(&app.dir, folder, false)).await.account,
        None => None,
    };
    let user = user.or_else(|| {
        let profiles = super::identity::read_profiles(&super::identity::profiles_path(&app.dir));
        profiles.iter().find_map(|p| p.accounts.get(&pr.host).cloned())
    });
    let user = user.ok_or_else(|| format!("No git account for {} in the git profiles. Add one in Settings, Git accounts.", pr.host))?;
    pr_facts(&app.http, &pr, &user, &api_base(&pr.host, std::env::var(API_ENV).ok())).await
}
