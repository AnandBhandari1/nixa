use crate::accounts::helper::{self, looks_like_token, mask_url, percent_decode};
use crate::accounts::profiles::{KEY_PROFILE, Profile, load_profiles};
use crate::accounts::run::{self, Opts, Output};
use crate::accounts::{Account, Env, load_accounts, token_for};
use regex::Regex;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub const PR_FIELDS: &str = "number,title,url,state,isDraft,headRefName,baseRefName,author,isCrossRepository,headRepositoryOwner,createdAt,updatedAt";

const TRUNK: &[&str] = &["main", "master", "develop", "dev", "trunk", "release", "staging", "production"];
const URL_USERS_TO_SKIP: &[&str] = &["git", "x-access-token", "oauth2", "token"];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    pub url: String,
    pub host: String,
    pub owner: String,
    pub repo: String,
    pub user: Option<String>,
    pub token_in_url: bool,
}

impl Remote {
    pub fn usable(&self) -> bool {
        !self.host.is_empty() && !self.owner.is_empty()
    }
}

fn shape(host: &str, path: &str, user: Option<String>, token_in_url: bool) -> Option<Remote> {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if host.is_empty() || parts.len() < 2 {
        return None;
    }
    Some(Remote {
        host: host.to_lowercase(),
        owner: parts[..parts.len() - 1].join("/"),
        repo: parts[parts.len() - 1].to_string(),
        user: user.filter(|u| !u.is_empty() && !looks_like_token(u)),
        token_in_url,
        ..Remote::default()
    })
}

pub fn parse_remote(url: &str) -> Option<Remote> {
    let t = url.trim().trim_end_matches('/');
    let git_end = Regex::new(r"(?i)\.git$").expect("git end");
    let t = git_end.replace(t, "");
    let t = t.trim_end_matches('/');
    let web = Regex::new(r"(?i)^https?://(?:([^@/]*)@)?([^/@]+)/(.+)$").expect("web url");
    if let Some(m) = web.captures(t) {
        let info = m.get(1).map(|x| x.as_str());
        let user = info.map(|i| percent_decode(i.split(':').next().unwrap_or("")));
        let token = info.is_some_and(|i| i.contains(':')) || user.as_deref().is_some_and(looks_like_token);
        return shape(&m[2], &m[3], user, token);
    }
    let ssh = Regex::new(r"(?i)^(?:git\+)?ssh://(?:[^@/]+@)?([^/:@]+)(?::\d+)?/(.+)$").expect("ssh url");
    if let Some(m) = ssh.captures(t) {
        return shape(&m[1], &m[2], None, false);
    }
    let scp = Regex::new(r"^[^@/:]+@([^:/]+):(.+)$").expect("scp url");
    if let Some(m) = scp.captures(t) {
        return shape(&m[1], &m[2], None, false);
    }
    None
}

pub fn repo_slug(r: &Remote) -> String {
    format!("{}/{}/{}", r.host, r.owner, r.repo)
}

pub fn web_url_of(r: &Remote) -> String {
    format!("https://{}/{}/{}", r.host, r.owner, r.repo)
}

pub fn same_repo(a: &Remote, b: &Remote) -> bool {
    a.host.eq_ignore_ascii_case(&b.host) && a.owner.eq_ignore_ascii_case(&b.owner) && a.repo.eq_ignore_ascii_case(&b.repo)
}

pub fn pr_head(source: Option<&Remote>, target: Option<&Remote>, branch: &str) -> String {
    match (source, target) {
        (Some(s), Some(t)) if !same_repo(s, t) => format!("{}:{branch}", s.owner),
        _ => branch.to_string(),
    }
}

pub fn title_from_branch(branch: &str) -> String {
    if branch.is_empty() || TRUNK.contains(&branch) {
        return String::new();
    }
    let leaf = branch.rsplit('/').next().unwrap_or(branch);
    let dashes = Regex::new(r"[-_]+").expect("dashes");
    let words = dashes.replace_all(leaf, " ").trim().to_string();
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => branch.to_string(),
    }
}

pub fn pick_base(options: &[String], preferred: Option<&str>) -> String {
    if let Some(p) = preferred.filter(|p| !p.is_empty())
        && (options.is_empty() || options.iter().any(|o| o == p))
    {
        return p.to_string();
    }
    for name in ["main", "master"] {
        if options.iter().any(|o| o == name) {
            return name.to_string();
        }
    }
    options
        .first()
        .cloned()
        .or(preferred.map(str::to_string))
        .unwrap_or_default()
}

pub fn pick_remote<'a>(remotes: &'a [Remote], preferred: Option<&str>) -> Option<&'a Remote> {
    let usable: Vec<&Remote> = remotes.iter().filter(|r| r.usable()).collect();
    usable
        .iter()
        .find(|r| Some(r.name.as_str()) == preferred)
        .or(usable.iter().find(|r| r.name == "origin"))
        .or(usable.first())
        .copied()
}

pub fn forge_targets(remotes: &[Remote], source: &Remote) -> Vec<Remote> {
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for r in std::iter::once(source).chain(remotes.iter()) {
        if !r.usable() || !r.host.eq_ignore_ascii_case(&source.host) {
            continue;
        }
        let key = format!("{}/{}", r.owner.to_lowercase(), r.repo.to_lowercase());
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        out.push(r.clone());
    }
    out
}

pub fn default_target(targets: &[Remote], source_name: &str) -> Option<String> {
    if let Some(up) = targets.iter().find(|t| t.name == "upstream")
        && up.name != source_name
    {
        return Some(up.name.clone());
    }
    targets
        .iter()
        .find(|t| t.name == source_name)
        .or(targets.first())
        .map(|t| t.name.clone())
        .or_else(|| (!source_name.is_empty()).then(|| source_name.to_string()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pick {
    pub username: String,
    pub source: &'static str,
}

pub fn pick_account(
    host: &str,
    profile: Option<&Profile>,
    bound: Option<&str>,
    url_user: Option<&str>,
    accounts: &[Account],
) -> Option<Pick> {
    let from_profile = profile.and_then(|p| {
        p.accounts
            .iter()
            .find(|(h, _)| h.eq_ignore_ascii_case(host))
            .map(|(_, u)| u.clone())
    });
    if let Some(u) = from_profile.filter(|u| !u.is_empty()) {
        return Some(Pick { username: u, source: "profile" });
    }
    if let Some(b) = bound.filter(|b| !b.is_empty()) {
        return Some(Pick { username: b.into(), source: "binding" });
    }
    if let Some(u) = url_user.filter(|u| !u.is_empty() && !URL_USERS_TO_SKIP.contains(&u.to_lowercase().as_str())) {
        return Some(Pick { username: u.into(), source: "url" });
    }
    let on_host: Vec<&Account> = accounts
        .iter()
        .filter(|a| !a.username.is_empty() && a.host.eq_ignore_ascii_case(host))
        .collect();
    if let Some(a) = on_host.iter().find(|a| a.default) {
        return Some(Pick { username: a.username.clone(), source: "default" });
    }
    on_host.first().map(|a| Pick {
        username: a.username.clone(),
        source: "account",
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrAccount {
    pub host: String,
    pub username: String,
    pub source: &'static str,
    pub profile_id: Option<String>,
    pub profile_label: Option<String>,
    pub remote: Remote,
}

pub fn account_reason(a: &PrAccount) -> String {
    match (a.source, &a.profile_label) {
        ("profile", Some(label)) => format!("from profile {label}"),
        ("profile", None) => "from the repo profile".into(),
        ("binding", _) => "set for this repo".into(),
        ("url", _) => "from the remote URL".into(),
        ("default", _) => "default account".into(),
        ("account", _) => "first account for this host".into(),
        _ => String::new(),
    }
}

fn git_text(env: &Env, root: &Path, args: &[&str]) -> String {
    helper::git_text(env, root, args)
}

pub fn repo_remotes(env: &Env, root: &Path) -> Vec<Remote> {
    let out = git_text(env, root, &["remote", "-v"]);
    let line = Regex::new(r"^(\S+)\s+(.+?)\s+\(fetch\)$").expect("remote line");
    let mut found: Vec<Remote> = Vec::new();
    for l in out.lines() {
        let Some(m) = line.captures(l) else {
            continue;
        };
        if found.iter().any(|r| r.name == m[1]) {
            continue;
        }
        let mut r = parse_remote(&m[2]).unwrap_or_default();
        r.name = m[1].to_string();
        r.url = mask_url(&m[2]);
        found.push(r);
    }
    found
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BranchState {
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub push_remote: Option<String>,
}

pub fn branch_state(env: &Env, root: &Path) -> BranchState {
    let branch = git_text(env, root, &["symbolic-ref", "--short", "-q", "HEAD"]);
    if branch.is_empty() {
        return BranchState::default();
    }
    let upstream = git_text(env, root, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]);
    let counts = if upstream.is_empty() {
        String::new()
    } else {
        git_text(env, root, &["rev-list", "--left-right", "--count", "@{u}...HEAD"])
    };
    let nums: Vec<u32> = counts.split_whitespace().filter_map(|n| n.parse().ok()).collect();
    let push_remote = [
        git_text(env, root, &["config", "--get", &format!("branch.{branch}.pushRemote")]),
        git_text(env, root, &["config", "--get", "remote.pushDefault"]),
        git_text(env, root, &["config", "--get", &format!("branch.{branch}.remote")]),
    ]
    .into_iter()
    .find(|n| !n.is_empty() && n != ".");
    BranchState {
        branch: Some(branch),
        upstream: Some(upstream).filter(|u| !u.is_empty()),
        behind: nums.first().copied().unwrap_or(0),
        ahead: nums.get(1).copied().unwrap_or(0),
        push_remote,
    }
}

pub fn base_branches(env: &Env, root: &Path, remote: &str, exclude: &[String]) -> Vec<String> {
    if remote.is_empty() {
        return Vec::new();
    }
    let prefix = format!("refs/remotes/{remote}/");
    let out = git_text(env, root, &["for-each-ref", "--format=%(refname)", &prefix]);
    let mut names: Vec<String> = out
        .lines()
        .filter_map(|r| r.strip_prefix(&prefix))
        .filter(|n| !n.is_empty() && *n != "HEAD" && !exclude.iter().any(|e| e == n))
        .map(str::to_string)
        .collect();
    names.sort();
    names.dedup();
    names
}

pub fn pr_account_for(env: &Env, root: &Path, remotes: &[Remote], preferred: Option<&str>) -> Option<PrAccount> {
    let pushed = branch_state(env, root).push_remote;
    let remote = pick_remote(remotes, preferred.or(pushed.as_deref()))?.clone();
    let host = remote.host.clone();
    let profile_id = Some(git_text(env, root, &["config", "--local", "--get", KEY_PROFILE])).filter(|p| !p.is_empty());
    let bound = git_text(env, root, &["config", "--local", "--get", &format!("credential.https://{host}.username")]);
    let profiles = load_profiles(env);
    let profile = profile_id.as_ref().and_then(|id| profiles.iter().find(|p| &p.id == id));
    let accounts = load_accounts(env);
    let pick = pick_account(&host, profile, Some(&bound), remote.user.as_deref(), &accounts)?;
    Some(PrAccount {
        host,
        username: pick.username,
        source: pick.source,
        profile_label: profile.map(|p| p.label.clone()),
        profile_id,
        remote,
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ctx {
    pub username: String,
    pub host: String,
    pub slug: String,
    pub base: String,
    pub cross_repo: bool,
}

pub fn ctx_of(a: Option<&PrAccount>) -> Ctx {
    Ctx {
        username: a.map(|a| a.username.clone()).unwrap_or_default(),
        host: a.map(|a| a.host.clone()).unwrap_or_default(),
        slug: a
            .filter(|a| !a.remote.owner.is_empty())
            .map(|a| format!("{}/{}", a.remote.owner, a.remote.repo))
            .unwrap_or_default(),
        ..Ctx::default()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForgeError {
    pub kind: String,
    pub title: String,
    pub hint: String,
    pub detail: String,
    pub url: Option<String>,
}

impl ForgeError {
    pub fn text(&self) -> String {
        crate::accounts::repo::sentence(&[&self.title, &self.hint, &self.detail])
    }
}

pub const NO_TOKEN: &str = "nixa: no token for";
pub const NO_GH: &str = "nixa: gh is missing";

const RULES: &[(&str, &str)] = &[
    ("noToken", r"nixa: no token for"),
    ("noGh", r"(?i)gh: command not found|gh: not found|nixa: gh is missing"),
    ("exists", r"(?i)a pull request for branch .* already exists"),
    ("sso", r"(?i)SAML enforcement|SAML SSO|organization SAML"),
    ("scope", r"(?i)Resource not accessible by (personal access token|integration)"),
    ("auth", r"(?i)HTTP 401|Bad credentials|gh auth login|authentication required|requires authentication"),
    ("forbidden", r"HTTP 403"),
    ("noHead", r"(?i)Head ref must be a branch|Head sha can't be blank|head.*invalid|could not find any commits"),
    ("badBase", r"(?i)Base ref must be a branch|Base sha can't be blank"),
    ("noCommits", r"(?i)No commits between"),
    ("notFound", r"Could not resolve to a Repository|HTTP 404|Not Found"),
    ("network", r"(?i)error connecting to|dial tcp|no such host|i/o timeout|connection (refused|reset)|TLS handshake|could not resolve host"),
];

fn or(text: &str, fallback: &str) -> String {
    if text.is_empty() { fallback.into() } else { text.into() }
}

fn words(kind: &str, c: &Ctx) -> Option<(&'static str, String, String)> {
    let user = or(&c.username, "this account");
    let host = or(&c.host, "GitHub");
    let slug = or(&c.slug, "this repository");
    let base = or(&c.base, "the base branch");
    Some(match kind {
        "noToken" => ("auth", format!("No token for {user} on {host}"), "Add or import this account in Settings, Git accounts.".into()),
        "noGh" => ("noGh", "GitHub CLI (gh) is not installed".into(), "Install it with: brew install gh. Then try again.".into()),
        "exists" => ("exists", "A pull request for this branch is already open".into(), String::new()),
        "sso" => ("auth", "The organization needs SSO for this token".into(), format!("Open the token on GitHub and allow it for the organization of {slug}.")),
        "scope" => (
            "auth",
            "This token cannot open pull requests here".into(),
            if c.cross_repo {
                format!("A fine-grained token can only act on repos of its own account, so it cannot open a pull request into {slug}. Use a classic token with the repo scope.")
            } else {
                "A fine-grained token needs Pull requests: read and write. A classic token needs the repo scope.".into()
            },
        ),
        "auth" => ("auth", "GitHub sign-in failed".into(), format!("The token for {user} may be expired or removed. Check it in Settings, Git accounts.")),
        "forbidden" => ("auth", "GitHub refused this".into(), format!("{user} may not have access to {slug}. Check the account in Settings, Git accounts.")),
        "noHead" => ("noUpstream", "GitHub cannot see this branch".into(), "Push the branch first, then try again.".into()),
        "badBase" => ("badBase", format!("The branch {base} is not on GitHub"), "Pick another base branch.".into()),
        "noCommits" => ("nothing", "Nothing to put in a pull request".into(), format!("This branch has no commits that are not already in {base}.")),
        "notFound" => ("notFound", "GitHub cannot find this repository".into(), format!("Check that {user} can see {slug}.")),
        "network" => ("network", "Cannot reach GitHub".into(), "Check the network and try again.".into()),
        "timeout" => ("timeout", "GitHub took too long".into(), "It was stopped. Try again.".into()),
        "cancelled" => ("cancelled", "Stopped".into(), String::new()),
        _ => return None,
    })
}

fn clean_detail(text: &str) -> String {
    let skip = Regex::new(r"(?i)^nixa:|^Try authenticating with|^\s*gh auth login|^hint:").expect("skip lines");
    let kept: Vec<String> = run::strip_ansi(text)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !skip.is_match(l))
        .collect();
    let from = kept.len().saturating_sub(4);
    kept[from..].join("\n")
}

fn pr_url(text: &str) -> Option<String> {
    Regex::new(r"https?://\S+/pull/\d+")
        .expect("pr url")
        .find(text)
        .map(|m| m.as_str().to_string())
}

fn made(kind: &str, c: &Ctx, detail: String, url: Option<String>) -> Option<ForgeError> {
    words(kind, c).map(|(k, title, hint)| ForgeError {
        kind: k.into(),
        title,
        hint,
        detail,
        url,
    })
}

pub fn explain_gh(out: &Output, c: &Ctx) -> ForgeError {
    if out.cancelled {
        return made("cancelled", c, String::new(), None).unwrap_or_default();
    }
    if out.timed_out {
        return made("timeout", c, String::new(), None).unwrap_or_default();
    }
    let text = format!("{}\n{}", out.stderr, out.stdout);
    let detail = clean_detail(if out.stderr.trim().is_empty() { &out.stdout } else { &out.stderr });
    let url = pr_url(&text);
    for (kind, pattern) in RULES {
        if Regex::new(pattern).expect("gh rule").is_match(&text)
            && let Some(e) = made(kind, c, detail.clone(), url.clone())
        {
            return e;
        }
    }
    let git = run::describe(&text);
    if git.kind != "other" {
        return ForgeError {
            kind: git.kind,
            title: git.title,
            hint: git.hint,
            detail: git.detail,
            url,
        };
    }
    ForgeError {
        kind: "other".into(),
        title: "GitHub CLI failed".into(),
        hint: String::new(),
        detail,
        url,
    }
}

fn invalid(title: &str, hint: &str) -> ForgeError {
    ForgeError {
        kind: "invalid".into(),
        title: title.into(),
        hint: hint.into(),
        ..ForgeError::default()
    }
}

fn fake(code: i32, stderr: &str) -> Output {
    Output {
        code: Some(code),
        stderr: stderr.into(),
        ..Output::default()
    }
}

#[derive(Clone, Debug, Default)]
pub struct GhOpts {
    pub timeout: Option<Duration>,
    pub cancel: Option<Arc<AtomicBool>>,
}

pub fn gh_env(token: &str, host: &str) -> Vec<(String, String)> {
    [
        ("GH_TOKEN", token),
        ("GH_ENTERPRISE_TOKEN", token),
        ("GH_HOST", host),
        ("GH_PROMPT_DISABLED", "1"),
        ("GH_NO_UPDATE_NOTIFIER", "1"),
        ("GH_SPINNER_DISABLED", "1"),
        ("GIT_TERMINAL_PROMPT", "0"),
        ("NO_COLOR", "1"),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

pub fn gh_run(env: &Env, root: &Path, account: &PrAccount, args: &[String], opts: &GhOpts) -> Output {
    if account.host.is_empty() || account.username.is_empty() {
        return fake(4, NO_TOKEN);
    }
    let Some(gh) = env.gh.clone() else {
        return fake(127, NO_GH);
    };
    let Some(token) = token_for(env, &account.host, &account.username) else {
        return fake(4, &format!("{NO_TOKEN} {} on {}", account.username, account.host));
    };
    let mut all = env.git_env.clone();
    all.extend(gh_env(&token, &crate::accounts::bare_host(&account.host)));
    let cwd = if root.as_os_str().is_empty() { env.home.clone() } else { root.to_path_buf() };
    run::run(
        &gh,
        args,
        &Opts {
            cwd: Some(cwd),
            env: all,
            stdin: None,
            timeout: Some(opts.timeout.unwrap_or(Duration::from_secs(120))),
            cancel: opts.cancel.clone(),
        },
    )
}

fn repo_flag(a: &PrAccount) -> Vec<String> {
    if a.remote.owner.is_empty() { Vec::new() } else { vec!["-R".into(), repo_slug(&a.remote)] }
}

fn gh_ok(env: &Env, root: &Path, a: &PrAccount, args: &[String], opts: &GhOpts) -> Result<Output, ForgeError> {
    let out = gh_run(env, root, a, args, opts);
    if out.ok() { Ok(out) } else { Err(explain_gh(&out, &ctx_of(Some(a)))) }
}

pub fn default_branch(env: &Env, root: &Path, a: &PrAccount) -> Result<Option<String>, ForgeError> {
    let mut args = run::strings(&["repo", "view"]);
    if !a.remote.owner.is_empty() {
        args.push(repo_slug(&a.remote));
    }
    args.extend(run::strings(&["--json", "defaultBranchRef", "-q", ".defaultBranchRef.name"]));
    let out = gh_ok(env, root, a, &args, &GhOpts::default())?;
    Ok(Some(out.stdout.trim().to_string()).filter(|b| !b.is_empty()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub number: u64,
    pub url: String,
    pub state: String,
    pub title: String,
}

pub fn find_pr(env: &Env, root: &Path, a: &PrAccount, branch: &str) -> Result<Option<Found>, ForgeError> {
    if branch.is_empty() {
        return Ok(None);
    }
    let mut args = run::strings(&["pr", "view", branch]);
    args.extend(repo_flag(a));
    args.extend(run::strings(&["--json", "number,url,state,title"]));
    let out = gh_run(env, root, a, &args, &GhOpts::default());
    if !out.ok() {
        if Regex::new(r"(?i)no pull requests found").expect("none").is_match(&out.stderr) {
            return Ok(None);
        }
        return Err(explain_gh(&out, &ctx_of(Some(a))));
    }
    let v: Value = serde_json::from_str(&out.stdout).unwrap_or(Value::Null);
    let number = v["number"].as_u64().unwrap_or(0);
    if number == 0 {
        return Ok(None);
    }
    Ok(Some(Found {
        number,
        url: v["url"].as_str().unwrap_or("").to_string(),
        state: v["state"].as_str().unwrap_or("OPEN").to_string(),
        title: v["title"].as_str().unwrap_or("").to_string(),
    }))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NewPr {
    pub title: String,
    pub body: String,
    pub base: String,
    pub head: String,
    pub draft: bool,
}

pub fn pr_create_args(slug: Option<&str>, pr: &NewPr, body_file: &Path) -> Vec<String> {
    let mut a = run::strings(&["pr", "create"]);
    if let Some(s) = slug {
        a.push("-R".into());
        a.push(s.into());
    }
    a.push(format!("--title={}", pr.title.trim()));
    a.push(format!("--body-file={}", body_file.display()));
    a.push(format!("--base={}", pr.base));
    a.push(format!("--head={}", pr.head));
    if pr.draft {
        a.push("--draft".into());
    }
    a
}

pub fn temp_body(text: &str) -> Result<(tempfile::TempDir, PathBuf), String> {
    let dir = tempfile::Builder::new()
        .prefix("nixa-git-pr.")
        .tempdir_in("/private/tmp")
        .map_err(|e| format!("cannot make a temp file: {e}"))?;
    let path = dir.path().join("body.md");
    crate::accounts::write_private(&path, text, 0o600)?;
    Ok((dir, path))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opened {
    pub url: String,
    pub number: Option<u64>,
}

pub fn create_pr(env: &Env, root: &Path, a: &PrAccount, pr: &NewPr, opts: &GhOpts) -> Result<Opened, ForgeError> {
    if pr.title.trim().is_empty() {
        return Err(invalid("Write a title first", ""));
    }
    if pr.base.is_empty() {
        return Err(invalid("Pick a base branch first", ""));
    }
    if pr.head.is_empty() {
        return Err(invalid("There is no branch to open a pull request from", ""));
    }
    if pr.base == pr.head {
        return Err(invalid("The base branch is the same as this branch", "Pick another base branch."));
    }
    let (dir, file) = temp_body(&pr.body).map_err(|e| invalid(&e, ""))?;
    let slug = (!a.remote.owner.is_empty()).then(|| repo_slug(&a.remote));
    let args = pr_create_args(slug.as_deref(), pr, &file);
    let job = GhOpts {
        timeout: Some(opts.timeout.unwrap_or(Duration::from_secs(300))),
        cancel: opts.cancel.clone(),
    };
    let out = gh_run(env, root, a, &args, &job);
    drop(dir);
    if !out.ok() {
        let mut c = ctx_of(Some(a));
        c.base = pr.base.clone();
        c.cross_repo = pr.head.contains(':');
        return Err(explain_gh(&out, &c));
    }
    let url = pr_url(&out.stdout).unwrap_or_else(|| run::last_line(&out.stdout));
    let number = Regex::new(r"/pull/(\d+)")
        .expect("pr number")
        .captures(&url)
        .and_then(|m| m[1].parse().ok());
    Ok(Opened { url, number })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pr {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: &'static str,
    pub is_draft: bool,
    pub author: String,
    pub source_branch: String,
    pub target_branch: String,
    pub is_from_fork: bool,
}

pub fn map_pr(v: &Value) -> Pr {
    let number = v["number"].as_u64().unwrap_or(0);
    Pr {
        number,
        title: v["title"].as_str().map(str::to_string).unwrap_or(format!("#{number}")),
        url: v["url"].as_str().unwrap_or("").into(),
        state: match v["state"].as_str() {
            Some("MERGED") => "merged",
            Some("CLOSED") => "closed",
            _ => "open",
        },
        is_draft: v["isDraft"].as_bool() == Some(true),
        author: v["author"]["login"].as_str().unwrap_or("unknown").into(),
        source_branch: v["headRefName"].as_str().unwrap_or("").into(),
        target_branch: v["baseRefName"].as_str().unwrap_or("").into(),
        is_from_fork: v["isCrossRepository"].as_bool() == Some(true),
    }
}

pub fn list_prs(env: &Env, root: &Path, a: &PrAccount, state: &str, limit: u32) -> Result<Vec<Pr>, ForgeError> {
    let state = if ["open", "closed", "merged", "all"].contains(&state) { state } else { "open" };
    let mut args = run::strings(&["pr", "list"]);
    args.extend(repo_flag(a));
    args.extend(run::strings(&["--state", state, "--limit", &limit.clamp(1, 200).to_string(), "--json", PR_FIELDS]));
    let out = gh_ok(env, root, a, &args, &GhOpts::default())?;
    match serde_json::from_str::<Value>(&out.stdout) {
        Ok(Value::Array(list)) => Ok(list.iter().map(map_pr).collect()),
        _ => Err(explain_gh(&fake(1, "GitHub returned an unexpected answer"), &ctx_of(Some(a)))),
    }
}

pub fn push_branch(env: &Env, root: &Path, a: &PrAccount, branch: &str, cancel: Option<Arc<AtomicBool>>) -> Result<(), ForgeError> {
    if branch.is_empty() || a.remote.name.is_empty() {
        return Err(invalid("There is no branch to push", ""));
    }
    let destination = format!("refs/heads/{branch}:refs/heads/{branch}");
    run_branch_remote(env, root, a, &["push", "-u", "--", &a.remote.name, &destination], cancel)
}

pub fn pull_branch(
    env: &Env,
    root: &Path,
    account: &PrAccount,
    branch: &str,
    mode: &str,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<(), ForgeError> {
    if account.remote.name.is_empty() || branch_state(env, root).branch.as_deref() != Some(branch) {
        return Err(invalid("The branch changed", "Close this and open it again."));
    }
    let mut args = vec!["pull"];
    args.extend(crate::remote::pull_args(mode).unwrap_or(&["--ff-only"]));
    let source = format!("refs/heads/{branch}");
    args.extend(["--", &account.remote.name, &source]);
    run_branch_remote(env, root, account, &args, cancel)
}

fn run_branch_remote(
    env: &Env,
    root: &Path,
    account: &PrAccount,
    command: &[&str],
    cancel: Option<Arc<AtomicBool>>,
) -> Result<(), ForgeError> {
    let helper_path = helper::ensure_helper(env).map_err(|e| invalid(&e, ""))?;
    let mut args = helper::account_args(&account.host, &account.username, &helper_path).map_err(|e| invalid(&e, ""))?;
    args.extend(run::strings(command));
    let mut all = env.git_env.clone();
    all.extend(run::GIT_ENV.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    all.push(("GIT_SSH_COMMAND".into(), run::SSH_BATCH.into()));
    let out = run::run(
        &env.git,
        &args,
        &Opts {
            cwd: Some(root.to_path_buf()),
            env: all,
            stdin: None,
            timeout: Some(Duration::from_secs(300)),
            cancel,
        },
    );
    if out.ok() {
        if command.first() == Some(&"pull") && crate::remote::autostash_clash(&out.said()) {
            return Err(invalid("Pull applied with conflicts", "Resolve conflicts in the Changes panel before pushing."));
        }
        return Ok(());
    }
    if out.cancelled {
        return Err(invalid("Stopped", "Check the branch before trying again."));
    }
    let said = run::describe(&out.said());
    Err(ForgeError {
        kind: said.kind,
        title: said.title,
        hint: said.hint,
        detail: said.detail,
        url: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::testkit;
    use crate::keychain;

    fn account(remote: Remote) -> PrAccount {
        PrAccount {
            host: "github.com".into(),
            username: "ana".into(),
            source: "default",
            profile_id: None,
            profile_label: None,
            remote,
        }
    }

    #[test]
    fn remotes_parse_and_pick() {
        let r = parse_remote("https://ana@github.com/me/app.git/").unwrap();
        assert_eq!((r.host.as_str(), r.owner.as_str(), r.repo.as_str()), ("github.com", "me", "app"));
        assert_eq!(r.user.as_deref(), Some("ana"));
        assert!(parse_remote("https://x:y@github.com/me/app").unwrap().token_in_url);
        assert_eq!(parse_remote("git@github.com:org/sub/app.git").unwrap().owner, "org/sub");
        assert_eq!(parse_remote("ssh://git@host:22/o/r").unwrap().host, "host");
        assert!(parse_remote("/local/path").is_none());
        let mut up = parse_remote("https://github.com/org/app").unwrap();
        up.name = "upstream".into();
        let mut origin = parse_remote("https://github.com/me/app").unwrap();
        origin.name = "origin".into();
        let list = vec![origin.clone(), up.clone()];
        assert_eq!(pick_remote(&list, None).unwrap().name, "origin");
        let targets = forge_targets(&list, &origin);
        assert_eq!(targets.len(), 2);
        assert_eq!(default_target(&targets, "origin").as_deref(), Some("upstream"));
        assert_eq!(pr_head(Some(&origin), Some(&up), "feat"), "me:feat");
        assert_eq!(pr_head(Some(&origin), Some(&origin), "feat"), "feat");
        assert_eq!(title_from_branch("feature/add-login_page"), "Add login page");
        assert_eq!(title_from_branch("main"), "");
        let opts = vec!["dev".to_string(), "master".to_string()];
        assert_eq!(pick_base(&opts, None), "master");
        assert_eq!(pick_base(&opts, Some("dev")), "dev");
        assert_eq!(pick_base(&[], Some("x")), "x");
    }

    #[test]
    fn account_choice_order() {
        let accounts = vec![
            Account { host: "github.com".into(), username: "one".into(), ..Account::default() },
            Account { host: "github.com".into(), username: "two".into(), default: true, ..Account::default() },
        ];
        assert_eq!(pick_account("github.com", None, None, None, &accounts).unwrap().username, "two");
        assert_eq!(pick_account("github.com", None, None, Some("git"), &accounts).unwrap().source, "default");
        assert_eq!(pick_account("github.com", None, None, Some("me"), &accounts).unwrap().source, "url");
        assert_eq!(pick_account("github.com", None, Some("b"), Some("me"), &accounts).unwrap().source, "binding");
        let p = Profile {
            accounts: [("GitHub.com".to_string(), "pro".to_string())].into(),
            ..Profile::default()
        };
        assert_eq!(pick_account("github.com", Some(&p), Some("b"), None, &accounts).unwrap().username, "pro");
        assert!(pick_account("gitlab.com", None, None, None, &accounts).is_none());
    }

    #[test]
    fn gh_errors_are_explained() {
        let c = Ctx { username: "ana".into(), base: "main".into(), ..Ctx::default() };
        let e = explain_gh(&fake(1, "pull request create failed: GraphQL: No commits between main and feat"), &c);
        assert_eq!(e.kind, "nothing");
        assert!(e.hint.contains("main"));
        let e = explain_gh(&fake(1, "a pull request for branch \"x\" into branch \"main\" already exists:\nhttps://github.com/o/r/pull/7"), &c);
        assert_eq!(e.kind, "exists");
        assert_eq!(e.url.as_deref(), Some("https://github.com/o/r/pull/7"));
        assert_eq!(explain_gh(&fake(4, NO_TOKEN), &c).title, "No token for ana on GitHub");
        assert_eq!(explain_gh(&fake(1, "boom"), &c).title, "GitHub CLI failed");
    }

    #[test]
    fn create_pr_uses_the_keychain_token_and_a_body_file() {
        let kit = testkit::kit();
        let env = &kit.env;
        let repo = kit.repo("app");
        keychain::save_token(env, "github.com", "ana", "tok_unit_token").unwrap();
        kit.gh_file("reply-pr-create", "https://github.com/me/app/pull/12\n");
        let a = account(Remote { name: "origin".into(), ..parse_remote("https://github.com/me/app").unwrap() });
        let pr = NewPr { title: " Add login ".into(), body: "Body text".into(), base: "main".into(), head: "feat".into(), draft: true };
        let opened = create_pr(env, &repo, &a, &pr, &GhOpts::default()).unwrap();
        assert_eq!(opened.number, Some(12));
        let calls = kit.gh_calls();
        assert!(calls.contains("pr create -R github.com/me/app --title=Add login"));
        assert!(calls.contains("--draft"));
        assert!(calls.contains("GH_TOKEN=tok_unit_token"));
        assert!(!calls.split('|').next().unwrap().contains("tok_unit_token"));
        assert_eq!(std::fs::read_to_string(kit.root().join("gh/body.seen")).unwrap(), "Body text");
        let same = NewPr { head: "main".into(), ..pr.clone() };
        assert_eq!(create_pr(env, &repo, &a, &same, &GhOpts::default()).unwrap_err().kind, "invalid");
        kit.gh_file("err-pr-create", "HTTP 401: Bad credentials\n");
        kit.gh_file("code-pr-create", "1");
        assert_eq!(create_pr(env, &repo, &a, &pr, &GhOpts::default()).unwrap_err().title, "GitHub sign-in failed");
        let nobody = PrAccount { username: "zed".into(), ..a.clone() };
        assert_eq!(create_pr(env, &repo, &nobody, &pr, &GhOpts::default()).unwrap_err().kind, "auth");
    }

    #[test]
    fn find_and_list_prs() {
        let kit = testkit::kit();
        let env = &kit.env;
        let repo = kit.repo("app");
        keychain::save_token(env, "github.com", "ana", "tok_unit_token").unwrap();
        let a = account(parse_remote("https://github.com/me/app").unwrap());
        kit.gh_file("err-pr-view", "no pull requests found for branch \"feat\"\n");
        kit.gh_file("code-pr-view", "1");
        assert_eq!(find_pr(env, &repo, &a, "feat").unwrap(), None);
        kit.gh_file("reply-pr-list", r#"[{"number":3,"title":"T","state":"MERGED","author":{"login":"bo"},"headRefName":"f","baseRefName":"main"}]"#);
        let list = list_prs(env, &repo, &a, "all", 10).unwrap();
        assert_eq!(list[0].state, "merged");
        assert_eq!(list[0].author, "bo");
    }

    #[test]
    fn branch_state_and_bases_and_push() {
        let kit = testkit::kit();
        let env = &kit.env;
        let bare = kit.root().join("remote.git");
        std::fs::create_dir_all(&bare).unwrap();
        kit.git(&bare, &["init", "-q", "--bare", "-b", "main"]);
        let repo = kit.repo("app");
        kit.git(&repo, &["config", "user.name", "T"]);
        kit.git(&repo, &["config", "user.email", "t@x.org"]);
        kit.git(&repo, &["commit", "-q", "--allow-empty", "-m", "one"]);
        kit.git(&repo, &["remote", "add", "origin", &bare.display().to_string()]);
        kit.git(&repo, &["push", "-q", "-u", "origin", "main"]);
        kit.git(&repo, &["checkout", "-q", "-b", "feat"]);
        kit.git(&repo, &["commit", "-q", "--allow-empty", "-m", "two"]);
        let s = branch_state(env, &repo);
        assert_eq!(s.branch.as_deref(), Some("feat"));
        assert_eq!(s.upstream, None);
        assert_eq!(base_branches(env, &repo, "origin", &["feat".into()]), ["main"]);
        let a = account(Remote { name: "origin".into(), host: "github.com".into(), ..Remote::default() });
        push_branch(env, &repo, &a, "feat", None).unwrap();
        let s = branch_state(env, &repo);
        assert_eq!(s.upstream.as_deref(), Some("origin/feat"));
        assert_eq!((s.ahead, s.behind), (0, 0));
        assert_eq!(s.push_remote.as_deref(), Some("origin"));
        assert!(repo_remotes(env, &repo).iter().all(|r| !r.usable()));
        let peer = kit.root().join("peer");
        kit.git(&kit.root(), &["clone", "-q", "-b", "feat", &bare.display().to_string(), &peer.display().to_string()]);
        kit.git(&peer, &["-c", "user.name=Git Test", "-c", "user.email=git@example.test", "commit", "-q", "--allow-empty", "-m", "remote change"]);
        kit.git(&peer, &["push", "-q", "origin", "feat"]);
        assert!(pull_branch(env, &repo, &a, "main", "ff-only", None).is_err());
        pull_branch(env, &repo, &a, "feat", "ff-only", None).unwrap();
        assert_eq!(git_text(env, &repo, &["log", "-1", "--format=%s"]), "remote change");
    }
}
