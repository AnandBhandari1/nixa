use crate::errors::{classify, describe};
use crate::ops::{EMPTY_TREE, current_branch, find_hooks, lines, pick_remote, remote_name_problem, remote_names, upstream_of};
use crate::outcome::{Outcome, described, failure, plural, refusal, stopped, success};
use crate::run::{self, Job, Out, git, read_ok};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::LazyLock;

pub const MAX_OTHER_ACCOUNTS: usize = 5;

pub const REBASE_PREVIEW_LIMIT: usize = 300;

pub const AUTH_KINDS: [&str; 2] = ["auth", "notFound"];

static HOST_OK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9.-]+(:\d+)?$").expect("host"));

static USER_OK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9._-]+$").expect("user"));

static AUTOSTASH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)Applying autostash resulted in conflicts").expect("autostash"));

pub fn pull_args(mode: &str) -> Option<&'static [&'static str]> {
    match mode {
        "rebase" => Some(&["--rebase", "--autostash"]),
        "merge" => Some(&["--no-rebase"]),
        "ff-only" => Some(&["--ff-only"]),
        _ => None,
    }
}

pub fn autostash_clash(text: &str) -> bool {
    AUTOSTASH.is_match(text)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushRef {
    pub flag: char,
    pub from: String,
    pub to: String,
    pub summary: String,
    pub reason: String,
}

pub fn parse_push_porcelain(text: &str) -> Vec<PushRef> {
    static LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([ +\-*!=])\t([^\t]*)\t(.*)$").expect("porcelain"));
    static REASON: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\(([^)]*)\)\s*$").expect("reason"));
    text.split('\n')
        .filter_map(|line| {
            let m = LINE.captures(line)?;
            let spec = &m[2];
            let (from, to) = spec.split_once(':').unwrap_or((spec, spec));
            Some(PushRef {
                flag: m[1].chars().next().unwrap_or(' '),
                from: from.into(),
                to: to.into(),
                summary: m[3].into(),
                reason: REASON.captures(&m[3]).map(|r| r[1].to_string()).unwrap_or_default(),
            })
        })
        .collect()
}

pub fn short_ref(reference: &str) -> String {
    for prefix in ["refs/heads/", "refs/tags/", "refs/remotes/"] {
        if let Some(rest) = reference.strip_prefix(prefix) {
            return rest.to_string();
        }
    }
    reference.to_string()
}

fn is_tag(r: &PushRef) -> bool {
    r.to.starts_with("refs/tags/")
}

pub fn tag_names(refs: &[PushRef]) -> Vec<String> {
    refs.iter().filter(|r| is_tag(r) && r.flag != '=' && r.flag != '!').map(|r| short_ref(&r.to)).collect()
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub host: String,
    pub username: String,
    pub default: bool,
    pub status: String,
    pub has_token: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Accounts {
    pub list: Vec<Account>,
    pub helper: Option<String>,
}

fn same_name(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

pub fn auth_failed(out: &Out) -> bool {
    !out.cancelled
        && !out.timed_out
        && !out.failed_to_start
        && out.code.is_some_and(|c| c != 0)
        && !parse_push_porcelain(&out.stdout).iter().any(|r| r.flag == '!')
        && AUTH_KINDS.contains(&classify(&out.said()))
}

pub fn account_args(host: &str, username: &str, helper: &str) -> Result<Vec<String>, String> {
    if !HOST_OK.is_match(host) || !USER_OK.is_match(username) || helper.is_empty() || helper.contains(['\n', '\0']) {
        return Err("cannot build the account settings for git".into());
    }
    let key = format!("credential.https://{host}");
    Ok(vec![
        "-c".into(),
        format!("{key}.helper="),
        "-c".into(),
        format!("{key}.helper={helper}"),
        "-c".into(),
        format!("{key}.username={username}"),
    ])
}

pub fn other_accounts(accounts: &[Account], host: &str, tried: Option<&str>) -> Vec<Account> {
    let mut list: Vec<Account> = accounts
        .iter()
        .filter(|a| {
            !a.username.is_empty()
                && USER_OK.is_match(&a.username)
                && same_name(&a.host, host)
                && !tried.is_some_and(|t| same_name(&a.username, t))
                && a.status != "revoked"
                && a.status != "expired"
        })
        .cloned()
        .collect();
    list.sort_by_key(|a| !a.default);
    list
}

pub fn https_host(url: &str) -> Option<(String, bool)> {
    static FULL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^([a-z][a-z0-9+.-]*)://(?:([^@/]*)@)?(\[[^\]]+\]|[^/:@]+)(?::(\d*))?(/.*)?$").expect("url"));
    let m = FULL.captures(url.trim())?;
    if !m[1].eq_ignore_ascii_case("https") {
        return None;
    }
    let mut host = m[3].to_lowercase();
    if let Some(port) = m.get(4).filter(|p| !p.as_str().is_empty()) {
        host = format!("{host}:{}", port.as_str());
    }
    let creds = m.get(2).is_some_and(|i| !i.as_str().is_empty());
    Some((host, creds))
}

pub fn https_target(urls: &[Option<String>]) -> Option<String> {
    let parts: Vec<(String, bool)> = urls.iter().flatten().filter_map(|u| https_host(u)).collect();
    if parts.is_empty() || parts.iter().any(|(_, creds)| *creds) {
        return None;
    }
    let first = &parts[0].0;
    if parts.iter().all(|(h, _)| h == first) && HOST_OK.is_match(first) { Some(first.clone()) } else { None }
}

fn remote_url_of(root: &Path, name: &str, push: bool) -> Option<String> {
    if name.is_empty() || remote_name_problem(name).is_some() {
        return None;
    }
    let mut args = vec!["remote", "get-url"];
    if push {
        args.push("--push");
    }
    args.push(name);
    let out = git(root, &args);
    let text = out.stdout.trim();
    if out.ok() && !text.is_empty() { Some(text.into()) } else { None }
}

pub fn remote_target(root: &Path, names: &[String], push: bool) -> Option<String> {
    let urls: Vec<Option<String>> = names.iter().map(|n| remote_url_of(root, n, push)).collect();
    https_target(&urls)
}

fn url_config(root: &Path, key: &str, host: &str) -> Option<String> {
    let url = format!("https://{host}/");
    let out = git(root, &["config", "--get-urlmatch", key, &url]);
    let text = out.stdout.trim();
    if out.ok() && !text.is_empty() { Some(text.into()) } else { None }
}

fn first_account(root: &Path, host: &str, accounts: &Accounts) -> Option<String> {
    if let Some(user) = url_config(root, "credential.username", host) {
        return Some(user);
    }
    let helper = url_config(root, "credential.helper", host)?;
    if accounts.helper.as_deref() != Some(helper.as_str()) {
        return None;
    }
    accounts.list.iter().find(|a| a.default && same_name(&a.host, host)).map(|a| a.username.clone())
}

#[derive(Debug, Clone, Default)]
pub struct Retry {
    pub out: Out,
    pub account: Option<Value>,
    pub tried: Vec<String>,
}

pub fn try_other_accounts(root: &Path, args: &[&str], accounts: &Accounts, job: &Job, first: Out, host: Option<String>) -> Retry {
    let none = |out: Out, tried: Vec<String>| Retry { out, account: None, tried };
    if !auth_failed(&first) || accounts.list.is_empty() {
        return none(first, Vec::new());
    }
    let Some(host) = host.filter(|h| HOST_OK.is_match(h)) else { return none(first, Vec::new()) };
    let bound = first_account(root, &host, accounts);
    let list: Vec<Account> = other_accounts(&accounts.list, &host, bound.as_deref()).into_iter().take(MAX_OTHER_ACCOUNTS).collect();
    let mut tried = Vec::new();
    for account in list {
        if job.cancelled() {
            return none(Out { cancelled: true, ..Out::default() }, tried);
        }
        if !account.has_token {
            continue;
        }
        let Some(helper) = accounts.helper.as_deref() else { return none(first, tried) };
        let Ok(extra) = account_args(&host, &account.username, helper) else { continue };
        let mut all: Vec<&str> = extra.iter().map(String::as_str).collect();
        all.extend_from_slice(args);
        let out = run::job(root, &all, job);
        tried.push(account.username.clone());
        if out.cancelled || out.timed_out || out.failed_to_start {
            return none(out, tried);
        }
        if !auth_failed(&out) {
            let note = match &bound {
                Some(b) => format!("Signed in as @{}, because @{b} could not. This repo is set to @{b}.", account.username),
                None => format!("Signed in as @{}.", account.username),
            };
            let value = json!({ "host": host, "username": account.username, "fallback": true, "bound": bound, "note": note });
            return Retry { out, account: Some(value), tried };
        }
    }
    none(first, tried)
}

pub fn with_tries(result: Outcome, retry: &Retry) -> Outcome {
    if let Some(account) = &retry.account {
        return result.with("account", account.clone());
    }
    if retry.tried.is_empty() || result.ok {
        return result;
    }
    let names: Vec<String> = retry.tried.iter().map(|n| format!("@{n}")).collect();
    let also = format!(
        "Also tried {}. {} could not sign in either.",
        names.join(", "),
        if retry.tried.len() == 1 { "It" } else { "They" }
    );
    let hint = [result.hint.as_str(), also.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ");
    result.with("triedAccounts", json!(retry.tried)).hint(&hint)
}

fn upstream_remote(root: &Path, branch: &str) -> Option<String> {
    let spec = format!("refs/heads/{branch}");
    let out = git(root, &["for-each-ref", "--format=%(upstream:remotename)", &spec]);
    let text = out.stdout.trim();
    if out.ok() && !text.is_empty() { Some(text.into()) } else { None }
}

fn rejection(refs: &[PushRef], out: &Out) -> Outcome {
    if let Some(side) = refs.iter().find(|r| r.summary.to_lowercase().contains("remote rejected")) {
        let why = if side.reason.is_empty() { &side.summary } else { &side.reason };
        return refusal("remoteRejected", "The remote refused this push", "The branch may be protected. Push to a new branch and open a pull request.")
            .detail(&format!("{}: {why}", short_ref(&side.to)))
            .stderr(&out.stderr)
            .with("reason", side.reason.clone().into());
    }
    let r = &refs[0];
    let lower = r.reason.to_lowercase();
    let stale = lower.contains("stale info") || lower.contains("updated since checkout");
    let why = if r.reason.is_empty() { &r.summary } else { &r.reason };
    let (title, hint) = if stale {
        ("The remote changed since your last fetch", "Pull first, then push again.")
    } else {
        ("The remote has newer commits", "Pull first, or force push with lease.")
    };
    refusal("rejected", title, hint)
        .detail(&format!("{}: {why}", short_ref(&r.to)))
        .stderr(&out.stderr)
        .with("reason", r.reason.clone().into())
        .with("stale", stale.into())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Dest {
    remote: String,
    reference: String,
}

fn push_destination(root: &Path, branch: &str) -> Option<Dest> {
    let spec = format!("refs/heads/{branch}");
    let out = git(root, &["for-each-ref", "--format=%(push:remotename)%00%(push)", &spec]);
    let text = if out.ok() { out.stdout.trim().to_string() } else { String::new() };
    let (remote, track) = text.split_once('\0').unwrap_or((text.as_str(), ""));
    let prefix = format!("refs/remotes/{remote}/");
    if remote.is_empty() || remote_name_problem(remote).is_some() || !track.starts_with(&prefix) || track.len() == prefix.len() {
        return None;
    }
    Some(Dest { remote: remote.into(), reference: format!("refs/heads/{}", &track[prefix.len()..]) })
}

#[derive(Debug, Clone, Default)]
pub struct PushOpts {
    pub remote: Option<String>,
    pub branch: Option<String>,
    pub force_lease: bool,
    pub tags: bool,
    pub accounts: Accounts,
    pub job: Job,
}

fn base(out: Outcome) -> Outcome {
    out.with("published", false.into())
}

fn stderr_or_said(out: &Out) -> String {
    if out.stderr.trim().is_empty() { out.said() } else { out.stderr.clone() }
}

pub fn push(root: &Path, opts: &PushOpts) -> Outcome {
    let Some(branch) = current_branch(root) else {
        return base(refusal("detached", "Not on a branch", "Check out a branch, then push."));
    };
    if let Some(want) = &opts.branch
        && want != &branch
    {
        return base(refusal("branchChanged", &format!("You are on {branch} now, not {want}"), &format!("Nothing was pushed. Switch back to {want}, then push again.")))
            .with("branch", branch.into());
    }
    if opts.force_lease && opts.branch.is_none() {
        return base(refusal("branchChanged", "Pick the branch to force push", "Nothing was pushed. Push again from the Changes panel."))
            .with("branch", branch.into());
    }
    if let Some(problem) = opts.remote.as_deref().and_then(remote_name_problem) {
        return base(refusal("invalid", "That remote name is not valid", problem));
    }
    let upstream = upstream_of(root);
    let remote = opts.remote.clone().or_else(|| if upstream.is_some() { None } else { pick_remote(root, Some(&branch), "push") });
    if upstream.is_none() && remote.is_none() {
        return base(refusal("noRemote", "This repo has no remote", "Add a remote in the repo settings first."));
    }
    let publish = upstream.is_none();
    let forced = if opts.force_lease && upstream.is_some() && opts.remote.is_none() { push_destination(root, &branch) } else { None };
    let lease_flag = forced.as_ref().map(|d| format!("--force-with-lease={}", d.reference));
    let forced_spec = forced.as_ref().map(|d| format!("refs/heads/{branch}:{}", d.reference));
    let head_spec = format!("refs/heads/{branch}");
    let mut args: Vec<&str> = vec!["push", "--porcelain"];
    if opts.tags {
        args.push("--follow-tags");
    }
    if let (Some(dest), Some(lease), Some(spec)) = (&forced, &lease_flag, &forced_spec) {
        args.extend([lease.as_str(), "--force-if-includes", "--end-of-options", dest.remote.as_str(), spec.as_str()]);
    } else {
        if opts.force_lease {
            args.extend(["--force-with-lease", "--force-if-includes"]);
        }
        if let Some(r) = &remote {
            if publish {
                args.push("-u");
            }
            args.extend(["--end-of-options", r.as_str(), if opts.force_lease { head_spec.as_str() } else { "HEAD" }]);
        }
    }
    let first = run::job(root, &args, &opts.job);
    let host = if auth_failed(&first) && !opts.accounts.list.is_empty() {
        let name = forced
            .as_ref()
            .map(|d| d.remote.clone())
            .or_else(|| remote.clone())
            .or_else(|| push_destination(root, &branch).map(|d| d.remote))
            .or_else(|| upstream.as_deref().and_then(|u| u.split('/').next()).map(String::from));
        remote_target(root, &name.into_iter().collect::<Vec<_>>(), true)
    } else {
        None
    };
    let retry = try_other_accounts(root, &args, &opts.accounts, &opts.job, first, host);
    let out = &retry.out;
    if let Some(stop) = stopped(out) {
        return base(stop);
    }
    let refs = parse_push_porcelain(&out.stdout);
    let bad: Vec<PushRef> = refs.iter().filter(|r| r.flag == '!').cloned().collect();
    let where_to = forced
        .as_ref()
        .map(|d| d.remote.clone())
        .or_else(|| remote.clone())
        .or_else(|| upstream.as_deref().and_then(|u| u.split('/').next()).map(String::from))
        .unwrap_or_default();
    let extra = |o: Outcome| {
        base(o).with("branch", branch.clone().into()).with("remote", where_to.clone().into()).with("forced", opts.force_lease.into())
    };
    if !bad.is_empty() {
        return with_tries(extra(rejection(&bad, out)), &retry);
    }
    if !out.ok() {
        let result = extra(failure(&stderr_or_said(out)));
        if result.kind == "other" && !find_hooks(root, &["pre-push"]).is_empty() {
            let words = describe("pre-push hook");
            return with_tries(
                Outcome { kind: "hook".into(), title: words.title, hint: "Fix what the pre-push hook says, then push again.".into(), ..result },
                &retry,
            );
        }
        return with_tries(result, &retry);
    }
    let up_to_date = !refs.is_empty() && refs.iter().all(|r| r.flag == '=');
    let elsewhere = match (&opts.remote, &upstream) {
        (Some(r), Some(u)) => !u.starts_with(&format!("{r}/")),
        _ => false,
    };
    let title = if up_to_date {
        "Already up to date".to_string()
    } else if publish {
        format!("Published {branch} to {where_to}")
    } else if opts.force_lease {
        format!("Force pushed {branch}")
    } else if elsewhere {
        format!("Pushed {branch} to {where_to}")
    } else {
        format!("Pushed {branch}")
    };
    let mut done = extra(success(&title)).with("published", publish.into()).with("upToDate", up_to_date.into()).stderr(&out.stderr);
    if opts.tags {
        done = done.with("pushedTags", json!(tag_names(&refs)));
    }
    with_tries(done, &retry)
}

#[derive(Debug, Clone, Default)]
pub struct TagsOpts {
    pub remote: Option<String>,
    pub accounts: Accounts,
    pub job: Job,
}

pub fn push_tags(root: &Path, opts: &TagsOpts) -> Outcome {
    let tag_base = |o: Outcome| base(o).with("pushedTags", json!([])).with("rejectedTags", json!([]));
    if let Some(problem) = opts.remote.as_deref().and_then(remote_name_problem) {
        return tag_base(refusal("invalid", "That remote name is not valid", problem));
    }
    let remote = opts.remote.clone().or_else(|| pick_remote(root, current_branch(root).as_deref(), "push"));
    let Some(remote) = remote else {
        return tag_base(refusal("noRemote", "This repo has no remote", "Add a remote in the repo settings first."));
    };
    let local = git(root, &["for-each-ref", "--count=1", "--format=%(refname)", "refs/tags"]);
    if local.ok() && local.stdout.trim().is_empty() {
        return tag_base(refusal("nothing", "This repo has no tags", "Make a tag first, then push it.")).with("remote", remote.into());
    }
    let args = ["push", "--porcelain", "--tags", "--end-of-options", remote.as_str()];
    let first = run::job(root, &args, &opts.job);
    let host = if auth_failed(&first) { remote_target(root, std::slice::from_ref(&remote), true) } else { None };
    let retry = try_other_accounts(root, &args, &opts.accounts, &opts.job, first, host);
    let out = &retry.out;
    if let Some(stop) = stopped(out) {
        return tag_base(stop).with("remote", remote.into());
    }
    let refs: Vec<PushRef> = parse_push_porcelain(&out.stdout).into_iter().filter(is_tag).collect();
    let pushed = tag_names(&refs);
    let refused: Vec<&PushRef> = refs.iter().filter(|r| r.flag == '!').collect();
    let rejected: Vec<String> = refused.iter().map(|r| short_ref(&r.to)).collect();
    let extra = |o: Outcome| {
        base(o).with("remote", remote.clone().into()).with("pushedTags", json!(pushed)).with("rejectedTags", json!(rejected))
    };
    if !refused.is_empty() {
        let title = if refused.len() == 1 {
            format!("{remote} refused the tag {}", short_ref(&refused[0].to))
        } else {
            format!("{remote} refused {} tags", refused.len())
        };
        let others = if pushed.is_empty() { String::new() } else { format!(" {} pushed.", plural(pushed.len(), "other tag was", "other tags were")) };
        let hint = format!("{remote} already has a tag with the same name on another commit. Those tags were not changed.{others}");
        let detail: Vec<String> = refused
            .iter()
            .take(6)
            .map(|r| format!("{}: {}", short_ref(&r.to), if r.reason.is_empty() { &r.summary } else { &r.reason }))
            .collect();
        return with_tries(extra(refusal("rejected", &title, &hint)).detail(&detail.join("\n")).stderr(&out.stderr), &retry);
    }
    if !out.ok() {
        return with_tries(extra(failure(&stderr_or_said(out))), &retry);
    }
    let title = if pushed.is_empty() {
        format!("All tags are already on {remote}")
    } else {
        format!("Pushed {} to {remote}", plural(pushed.len(), "tag", "tags"))
    };
    with_tries(extra(success(&title)).with("upToDate", pushed.is_empty().into()).stderr(&out.stderr), &retry)
}

#[derive(Debug, Clone, Default)]
pub struct PullOpts {
    pub mode: Option<String>,
    pub accounts: Accounts,
    pub job: Job,
}

pub fn pull(root: &Path, opts: &PullOpts) -> Outcome {
    let mode = opts.mode.as_deref().filter(|m| pull_args(m).is_some()).unwrap_or("rebase").to_string();
    let pbase = |o: Outcome| base(o).with("mode", mode.clone().into());
    let Some(branch) = current_branch(root) else {
        return pbase(refusal("detached", "Not on a branch", "Check out a branch, then pull."));
    };
    let upstream = upstream_of(root);
    let mut target: Vec<String> = Vec::new();
    let mut picked = None;
    if upstream.is_none() {
        let Some(remote) = pick_remote(root, Some(&branch), "fetch") else {
            return pbase(refusal("noRemote", "This repo has no remote", "Add a remote in the repo settings first."));
        };
        let spec = format!("refs/remotes/{remote}/{branch}");
        if !git(root, &["rev-parse", "-q", "--verify", &spec]).ok() {
            return pbase(described("has no upstream branch"));
        }
        target = vec!["--end-of-options".into(), remote.clone(), branch.clone()];
        picked = Some(remote);
    }
    let mut args: Vec<&str> = vec!["pull"];
    args.extend_from_slice(pull_args(&mode).unwrap_or(&[]));
    args.extend(target.iter().map(String::as_str));
    let first = run::job(root, &args, &opts.job);
    let host = if auth_failed(&first) {
        let name = picked.clone().or_else(|| upstream_remote(root, &branch));
        remote_target(root, &name.into_iter().collect::<Vec<_>>(), false)
    } else {
        None
    };
    let retry = try_other_accounts(root, &args, &opts.accounts, &opts.job, first, host);
    let out = &retry.out;
    if let Some(stop) = stopped(out) {
        return pbase(stop);
    }
    let text = out.said();
    if !out.ok() {
        static DIVERGED: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"(?i)Not possible to fast-forward|Diverging branches can't be fast-forwarded").expect("diverged"));
        if mode == "ff-only" && DIVERGED.is_match(&text) {
            return with_tries(
                pbase(refusal("diverged", "Your branch and the remote both have new commits", "Pull with rebase or merge instead."))
                    .stderr(&out.stderr)
                    .detail(&describe(&text).detail),
                &retry,
            );
        }
        return with_tries(pbase(failure(&text)), &retry);
    }
    if autostash_clash(&text) {
        return with_tries(
            pbase(success("Pulled, but your changes clashed"))
                .with("warning", true.into())
                .with("stashConflict", true.into())
                .stderr(&out.stderr)
                .hint("Your own changes did not fit with the new commits. They are kept in the stash, and the clashing files are marked. Fix those files, or open the stash window."),
            &retry,
        );
    }
    static UP_TO_DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)Already up to date|is up to date").expect("up to date"));
    let up_to_date = UP_TO_DATE.is_match(&text);
    with_tries(
        pbase(success(if up_to_date { "Already up to date" } else { "Pulled" })).with("upToDate", up_to_date.into()).stderr(&out.stderr),
        &retry,
    )
}

#[derive(Debug, Clone, Default)]
pub struct FetchOpts {
    pub remote: Option<String>,
    pub all: bool,
    pub prune: Option<bool>,
    pub tags: bool,
    pub accounts: Accounts,
    pub job: Job,
}

pub fn fetch(root: &Path, opts: &FetchOpts) -> Outcome {
    let mut flags: Vec<&str> = Vec::new();
    if opts.prune != Some(false) {
        flags.push("--prune");
    }
    if opts.tags {
        flags.push("--tags");
    }
    let with_tags = if opts.tags { ", with tags" } else { "" };
    let mut args: Vec<String> = vec!["fetch".into()];
    let label;
    let names: Option<Vec<String>>;
    if opts.all {
        args.push("--all".into());
        args.extend(flags.iter().map(|f| f.to_string()));
        label = format!("Fetched all remotes{with_tags}");
        names = None;
    } else {
        if let Some(problem) = opts.remote.as_deref().and_then(remote_name_problem) {
            return base(refusal("invalid", "That remote name is not valid", problem));
        }
        let remote = opts.remote.clone().or_else(|| pick_remote(root, current_branch(root).as_deref(), "fetch"));
        let Some(remote) = remote else {
            return base(refusal("noRemote", "This repo has no remote", "Add a remote in the repo settings first."));
        };
        args.extend(flags.iter().map(|f| f.to_string()));
        args.push("--end-of-options".into());
        args.push(remote.clone());
        label = format!("Fetched {remote}{with_tags}");
        names = Some(vec![remote]);
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let first = run::job(root, &refs, &opts.job);
    let host = if auth_failed(&first) { remote_target(root, &names.unwrap_or_else(|| remote_names(root)), false) } else { None };
    let retry = try_other_accounts(root, &refs, &opts.accounts, &opts.job, first, host);
    let out = &retry.out;
    if let Some(stop) = stopped(out) {
        return base(stop);
    }
    if !out.ok() {
        return with_tries(base(failure(&out.said())), &retry);
    }
    with_tries(base(success(&label)).stderr(&out.stderr), &retry)
}

fn z_list(text: &str) -> Vec<String> {
    text.split('\0').map(|p| p.trim_matches('\n').to_string()).filter(|p| !p.is_empty()).collect()
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeTree {
    pub clean: bool,
    pub tree: String,
    pub conflicts: Vec<String>,
}

pub fn parse_merge_tree(text: &str) -> (String, Vec<String>) {
    let mut all = z_list(text);
    if all.is_empty() {
        return (String::new(), Vec::new());
    }
    let tree = all.remove(0).trim().to_string();
    all.sort();
    all.dedup();
    (tree, all)
}

pub fn parse_status_paths(text: &str) -> Vec<String> {
    let parts: Vec<&str> = text.split('\0').collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < parts.len() {
        let entry = parts[i];
        if entry.len() >= 4 {
            let x = entry.as_bytes()[0];
            let y = entry.as_bytes()[1];
            out.push(entry[3..].to_string());
            if b"RC".contains(&x) || b"RC".contains(&y) {
                if let Some(next) = parts.get(i + 1).filter(|p| !p.is_empty()) {
                    out.push(next.to_string());
                }
                i += 1;
            }
        }
        i += 1;
    }
    out
}

pub fn touched_paths(local: &[String], incoming: &[String]) -> Vec<String> {
    let mut hit: Vec<String> = Vec::new();
    for path in local {
        if path.ends_with('/') {
            hit.extend(incoming.iter().filter(|o| o.starts_with(path.as_str())).cloned());
        } else if incoming.contains(path) {
            hit.push(path.clone());
        }
    }
    hit.sort();
    hit.dedup();
    hit
}

pub fn merge_tree(root: &Path, args: &[&str]) -> Result<MergeTree, String> {
    let mut all = vec!["merge-tree", "--write-tree", "--name-only", "--no-messages", "-z"];
    all.extend_from_slice(args);
    let out = git(root, &all);
    if out.code != Some(0) && out.code != Some(1) {
        let text = out.stderr.trim();
        return Err(if text.is_empty() { "git merge-tree failed".into() } else { text.into() });
    }
    let (tree, conflicts) = parse_merge_tree(&out.stdout);
    Ok(MergeTree { clean: out.ok(), tree, conflicts })
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopStep {
    pub n: usize,
    pub total: usize,
    pub sha: String,
    pub short: String,
    pub subject: String,
}

#[derive(Debug, Clone, Default)]
struct Replay {
    conflicts: Vec<String>,
    step: Option<StopStep>,
}

fn replay_rebase(root: &Path) -> Result<Option<Replay>, String> {
    let text = read_ok(root, &["rev-list", "--reverse", "--no-merges", "--right-only", "--cherry-pick", "--parents", "@{u}...HEAD"])?;
    let commits: Vec<Vec<String>> = lines(&text).iter().map(|l| l.split(' ').map(String::from).collect()).collect();
    if commits.len() > REBASE_PREVIEW_LIMIT {
        return Ok(None);
    }
    let mut onto = read_ok(root, &["rev-parse", "@{u}^{tree}"])?.trim().to_string();
    for (i, commit) in commits.iter().enumerate() {
        let sha = &commit[0];
        let parent = commit.get(1).map(String::as_str).unwrap_or(EMPTY_TREE);
        let base_flag = format!("--merge-base={parent}");
        let step = merge_tree(root, &[&base_flag, &onto, sha])?;
        if !step.clean {
            let info = git(root, &["log", "-1", "--format=%h%x00%s", sha]);
            let trimmed = info.stdout.trim().to_string();
            let (short, subject) = trimmed.split_once('\0').unwrap_or((trimmed.as_str(), ""));
            let short = if short.is_empty() { sha[..7.min(sha.len())].to_string() } else { short.to_string() };
            return Ok(Some(Replay {
                conflicts: step.conflicts,
                step: Some(StopStep { n: i + 1, total: commits.len(), sha: sha.clone(), short, subject: subject.into() }),
            }));
        }
        onto = step.tree;
    }
    Ok(Some(Replay::default()))
}

fn local_touches(root: &Path) -> Result<Vec<String>, String> {
    let status = read_ok(root, &["-c", "core.quotepath=off", "status", "--porcelain=v1", "-z", "--untracked-files=normal"])?;
    let incoming = read_ok(root, &["-c", "core.quotepath=off", "diff", "--name-only", "-z", "--no-renames", "HEAD...@{u}"])?;
    Ok(touched_paths(&parse_status_paths(&status), &z_list(&incoming)))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreviewFacts {
    pub mode: String,
    pub ahead: usize,
    pub behind: usize,
    pub conflicts: Vec<String>,
    pub step: Option<StopStep>,
    pub local_changes: Vec<String>,
    pub diverged: bool,
    pub branch: String,
    pub upstream: String,
}

pub fn preview_words(f: &PreviewFacts) -> (String, String) {
    if f.behind == 0 {
        let hint = if f.ahead > 0 {
            format!("You have {} to push.", plural(f.ahead, "commit", "commits"))
        } else {
            format!("{} matches {}.", f.branch, f.upstream)
        };
        return ("Nothing new to pull".into(), hint);
    }
    let mut notes: Vec<String> = Vec::new();
    let news = plural(f.behind, "new commit", "new commits");
    let (title, hint);
    if f.mode == "ff-only" && f.diverged {
        title = "Fast-forward only will not work".to_string();
        hint = format!("Your branch and {} both have new commits. Pull with rebase or merge instead.", f.upstream);
        if !f.conflicts.is_empty() {
            notes.push(format!("A merge would have conflicts in {}.", plural(f.conflicts.len(), "file", "files")));
        }
    } else if !f.conflicts.is_empty() {
        title = format!("The pull will stop on conflicts in {}", plural(f.conflicts.len(), "file", "files"));
        hint = match &f.step {
            Some(s) => format!("It stops at your commit {} \"{}\". You can fix the files in the Changes panel.", s.short, s.subject),
            None => "You can fix the files in the Changes panel after the pull.".into(),
        };
    } else {
        title = format!("{news}. No conflicts.");
        hint = if f.ahead == 0 {
            "Your branch will move forward.".into()
        } else if f.mode == "merge" {
            "Git will make a merge commit.".into()
        } else {
            format!("Your {} on top of the new ones.", plural(f.ahead, "commit goes", "commits go"))
        };
    }
    if !f.local_changes.is_empty() {
        let n = plural(f.local_changes.len(), "file", "files");
        notes.push(if f.mode == "rebase" {
            format!("Your uncommitted changes touch {n} that the pull also changes. They are put away and brought back, and may clash.")
        } else {
            format!("Your uncommitted changes touch {n} that the pull also changes. Git may stop and ask you to commit or stash them first.")
        });
    }
    let all: Vec<String> = std::iter::once(hint).chain(notes).filter(|s| !s.is_empty()).collect();
    (title, all.join(" "))
}

pub fn preview_pull(root: &Path, mode: Option<&str>) -> Outcome {
    let pbase = |o: Outcome| {
        base(o)
            .with("ahead", 0.into())
            .with("behind", 0.into())
            .with("clean", true.into())
            .with("conflicts", json!([]))
            .with("localChanges", json!([]))
            .with("step", Value::Null)
            .with("diverged", false.into())
    };
    let Some(branch) = current_branch(root) else {
        return pbase(refusal("detached", "Not on a branch", "Check out a branch, then check again."));
    };
    let Some(upstream) = upstream_of(root) else {
        return pbase(described("has no upstream branch")).with("branch", branch.into());
    };
    let mode = mode.filter(|m| pull_args(m).is_some()).unwrap_or("rebase").to_string();
    let run = || -> Result<Outcome, String> {
        let counts: Vec<usize> = read_ok(root, &["rev-list", "--left-right", "--count", "HEAD...@{u}"])?
            .split_whitespace()
            .map(|v| v.parse().unwrap_or(0))
            .collect();
        let ahead = counts.first().copied().unwrap_or(0);
        let behind = counts.get(1).copied().unwrap_or(0);
        let mut facts = PreviewFacts {
            mode: mode.clone(),
            ahead,
            behind,
            diverged: ahead > 0 && behind > 0,
            branch: branch.clone(),
            upstream: upstream.clone(),
            ..PreviewFacts::default()
        };
        let mut checked_as = mode.clone();
        if behind > 0 {
            facts.local_changes = local_touches(root)?;
            if ahead > 0 {
                let replay = if mode == "rebase" { replay_rebase(root)? } else { None };
                match replay {
                    Some(found) => {
                        facts.conflicts = found.conflicts;
                        facts.step = found.step;
                    }
                    None => {
                        facts.conflicts = merge_tree(root, &["HEAD", "@{u}"])?.conflicts;
                        checked_as = "merge".into();
                    }
                }
            }
        }
        let clean = facts.conflicts.is_empty() && !(mode == "ff-only" && facts.diverged);
        let (title, hint) = preview_words(&facts);
        let mut out = success(&title)
            .hint(&hint)
            .with("published", false.into())
            .with("branch", branch.clone().into())
            .with("upstream", upstream.clone().into())
            .with("mode", mode.clone().into())
            .with("ahead", ahead.into())
            .with("behind", behind.into())
            .with("diverged", facts.diverged.into())
            .with("clean", (if behind == 0 { true } else { clean }).into())
            .with("conflicts", json!(facts.conflicts))
            .with("localChanges", json!(facts.local_changes))
            .with("step", json!(facts.step));
        if behind > 0 {
            out = out.with("checkedAs", checked_as.into());
        }
        Ok(out)
    };
    run().unwrap_or_else(|e| pbase(failure(&e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testrepo;

    #[test]
    fn porcelain_and_tags() {
        let text = "To /x\n=\trefs/heads/main:refs/heads/main\t[up to date]\n!\trefs/tags/v1:refs/tags/v1\t[rejected] (already exists)\n*\trefs/tags/v2:refs/tags/v2\t[new tag]\nDone\n";
        let refs = parse_push_porcelain(text);
        assert_eq!(refs.len(), 3);
        assert_eq!(refs[1].flag, '!');
        assert_eq!(refs[1].reason, "already exists");
        assert_eq!(tag_names(&refs), vec!["v2".to_string()]);
        assert_eq!(short_ref("refs/heads/a/b"), "a/b");
    }

    #[test]
    fn account_helpers() {
        let args = account_args("github.com", "me", "/x/helper").unwrap();
        assert_eq!(args[1], "credential.https://github.com.helper=");
        assert_eq!(args[5], "credential.https://github.com.username=me");
        assert!(account_args("bad host", "me", "/h").is_err());
        let list = vec![
            Account { host: "github.com".into(), username: "a".into(), ..Account::default() },
            Account { host: "github.com".into(), username: "b".into(), default: true, ..Account::default() },
            Account { host: "github.com".into(), username: "c".into(), status: "revoked".into(), ..Account::default() },
            Account { host: "gitlab.com".into(), username: "d".into(), ..Account::default() },
        ];
        let others: Vec<String> = other_accounts(&list, "github.com", Some("A")).into_iter().map(|a| a.username).collect();
        assert_eq!(others, vec!["b".to_string()]);
        assert_eq!(https_target(&[Some("https://github.com/me/x.git".into())]).as_deref(), Some("github.com"));
        assert_eq!(https_target(&[Some("https://tok@github.com/me/x.git".into())]), None);
        assert_eq!(https_target(&[Some("git@github.com:me/x.git".into())]), None);
        let out = Out { code: Some(128), stderr: "fatal: Authentication failed for 'https://github.com/x'".into(), ..Out::default() };
        assert!(auth_failed(&out));
        let tried = with_tries(failure("x"), &Retry { tried: vec!["a".into(), "b".into()], ..Retry::default() });
        assert!(tried.hint.ends_with("Also tried @a, @b. They could not sign in either."));
    }

    #[test]
    fn push_publish_and_reject() {
        let remote = testrepo::bare();
        let repo = testrepo::new();
        let root = repo.root();
        assert_eq!(push(root, &PushOpts::default()).kind, "noRemote");
        let url = remote.root().to_string_lossy().into_owned();
        repo.git(&["remote", "add", "origin", &url]);
        let out = push(root, &PushOpts::default());
        assert!(out.ok, "{out:?}");
        assert_eq!(out.title, "Published main to origin");
        assert!(out.flag("published"));
        assert_eq!(push(root, &PushOpts::default()).title, "Already up to date");
        let other = testrepo::clone_of(&remote);
        other.commit_file("b.txt", "b\n", "theirs");
        other.git(&["push", "-q", "origin", "main"]);
        repo.commit_file("c.txt", "c\n", "mine");
        let rejected = push(root, &PushOpts::default());
        assert_eq!(rejected.kind, "rejected", "{rejected:?}");
        assert_eq!(push(root, &PushOpts { force_lease: true, ..PushOpts::default() }).kind, "branchChanged");
        assert_eq!(push(root, &PushOpts { branch: Some("dev".into()), ..PushOpts::default() }).kind, "branchChanged");
        let fetched = fetch(root, &FetchOpts::default());
        assert_eq!(fetched.title, "Fetched origin");
        let preview = preview_pull(root, Some("rebase"));
        assert!(preview.ok, "{preview:?}");
        assert_eq!(preview.title, "1 new commit. No conflicts.");
        assert_eq!(preview.get("ahead"), Some(&json!(1)));
        let unsafe_force = push(root, &PushOpts { force_lease: true, branch: Some("main".into()), ..PushOpts::default() });
        assert_eq!(unsafe_force.kind, "rejected", "{unsafe_force:?}");
        assert!(unsafe_force.flag("stale"));
        let (mine, _bare) = testrepo::with_remote();
        mine.write("a.txt", "amended\n");
        mine.git(&["commit", "-q", "-a", "--amend", "-m", "first again"]);
        let forced = push(mine.root(), &PushOpts { force_lease: true, branch: Some("main".into()), ..PushOpts::default() });
        assert!(forced.ok, "{forced:?}");
        assert_eq!(forced.title, "Force pushed main");
    }

    #[test]
    fn pull_modes_and_preview_conflicts() {
        let (repo, remote) = testrepo::with_remote();
        let root = repo.root();
        assert_eq!(preview_pull(root, None).title, "Nothing new to pull");
        let other = testrepo::clone_of(&remote);
        other.commit_file("a.txt", "theirs\n", "theirs");
        other.git(&["push", "-q", "origin", "main"]);
        repo.commit_file("a.txt", "mine\n", "mine");
        repo.git(&["fetch", "-q"]);
        let preview = preview_pull(root, Some("rebase"));
        assert_eq!(preview.title, "The pull will stop on conflicts in 1 file", "{preview:?}");
        assert!(preview.hint.contains("\"mine\""));
        let ff = preview_pull(root, Some("ff-only"));
        assert_eq!(ff.title, "Fast-forward only will not work");
        let out = pull(root, &PullOpts { mode: Some("ff-only".into()), ..PullOpts::default() });
        assert_eq!(out.kind, "diverged", "{out:?}");
        let out = pull(root, &PullOpts { mode: Some("merge".into()), ..PullOpts::default() });
        assert_eq!(out.kind, "conflict", "{out:?}");
    }

    #[test]
    fn pull_clean_and_tags() {
        let (repo, remote) = testrepo::with_remote();
        let root = repo.root();
        let other = testrepo::clone_of(&remote);
        other.commit_file("z.txt", "z\n", "theirs");
        other.git(&["push", "-q", "origin", "main"]);
        let out = pull(root, &PullOpts::default());
        assert!(out.ok, "{out:?}");
        assert_eq!(out.title, "Pulled");
        assert_eq!(pull(root, &PullOpts::default()).title, "Already up to date");
        assert_eq!(push_tags(root, &TagsOpts::default()).kind, "nothing");
        repo.git(&["tag", "v1"]);
        let tags = push_tags(root, &TagsOpts::default());
        assert!(tags.ok, "{tags:?}");
        assert_eq!(tags.title, "Pushed 1 tag to origin");
        assert_eq!(push_tags(root, &TagsOpts::default()).title, "All tags are already on origin");
        assert_eq!(touched_paths(&["dir/".into(), "a".into()], &["dir/x".into(), "a".into(), "b".into()]), vec!["a".to_string(), "dir/x".to_string()]);
        assert_eq!(parse_status_paths("R  new\0old\0 M a\0"), vec!["new".to_string(), "old".to_string(), "a".to_string()]);
    }
}
