use super::identity::{DEFAULT_HOST, helper_path, helper_value, prep_for, prepare, read_repo_config};
use super::policy::{clip, hide_credentials, lines, plural, tail};
use super::repo::{base_name, count_pair, other_branches, status_of, upstream_of};
use super::run::{READ_MS, git, git_args, read};
use crate::app::App;
use serde_json::{Value, json};

pub const PUSH_COMMITS: usize = 100;
pub const PUSH_MS: u64 = 180_000;

pub fn pick_remote(remotes: &[String], configured: &str) -> Option<String> {
    if !configured.is_empty() && configured != "." && remotes.iter().any(|r| r == configured) {
        return Some(configured.to_string());
    }
    if remotes.iter().any(|r| r == "origin") {
        return Some("origin".into());
    }
    (remotes.len() == 1).then(|| remotes[0].clone())
}

pub fn push_ask(branch: &str, repo: &str, remote: &str, commits: &[String]) -> String {
    let shown = if commits.len() == 1 { format!("1 commit ({})", clip(&commits[0], 70)) } else { format!("{} ({} and more)", plural(commits.len(), "commit"), clip(commits.first().map(String::as_str).unwrap_or_default(), 50)) };
    format!("Push branch {branch} of {repo} to {remote}: {shown}. Never main or master, never force")
}

static HELD: crate::yes_once::HeldAsk = std::sync::Mutex::new(None);

pub fn push_flag(line: &str) -> char {
    line.chars().next().unwrap_or('?')
}

pub async fn account_problem(app: &App, root: &str, url: &str) -> Option<String> {
    let lower = url.to_lowercase();
    if !lower.starts_with("http://") && !lower.starts_with("https://") {
        return None;
    }
    let config = read_repo_config(root, DEFAULT_HOST).await;
    if config.helpers.contains(&helper_value(&helper_path(&app.dir).display().to_string())) {
        return None;
    }
    let found = prepare(&prep_for(&app.dir, root, false)).await;
    if found.profile.is_some() && found.account.is_some() {
        return None;
    }
    Some(format!("{} has no git account from the app. Pick a profile for it in the Git panel first.", base_name(root)))
}

async fn tip_of(root: &str, branch: &str) -> String {
    read(root, &["rev-parse", "--verify", "-q", &format!("refs/heads/{branch}^{{commit}}")]).await.map(|s| s.trim().to_string()).unwrap_or_default()
}

pub async fn push(app: &App, root: &str, branch: &str) -> Result<Value, String> {
    let branch = branch.trim();
    let branch = branch.strip_prefix("refs/heads/").unwrap_or(branch).to_string();
    let repo = base_name(root);
    if super::policy::is_protected_branch(&branch) {
        return Err(format!("Nixa never pushes {branch}. Push it yourself if you want it. Nothing was pushed."));
    }
    let checked = git(root, &["check-ref-format", "--branch", &branch], READ_MS).await;
    if !checked.ok() || checked.stdout.trim() != branch {
        return Err(format!("\"{branch}\" is not a branch name. Nothing was pushed."));
    }
    let tip = tip_of(root, &branch).await;
    if tip.is_empty() {
        return Err(format!("{repo} has no local branch {branch}. Nothing was pushed."));
    }
    let remotes = lines(&read(root, &["remote"]).await.unwrap_or_default());
    let configured = read(root, &["config", "--get", &format!("branch.{branch}.remote")]).await.map(|s| s.trim().to_string()).unwrap_or_default();
    let Some(remote) = pick_remote(&remotes, &configured) else {
        let why = if remotes.is_empty() { "no remote".to_string() } else { format!("{} and none is origin", plural(remotes.len(), "remote")) };
        return Err(format!("{repo} has {why}. Nothing was pushed."));
    };
    if read(root, &["config", "--get", "--bool", &format!("remote.{remote}.mirror")]).await.is_some_and(|s| s.trim() == "true") {
        return Err(format!("The remote {remote} of {repo} is a mirror. Nixa does not push to a mirror. Nothing was pushed."));
    }
    let url = hide_credentials(read(root, &["remote", "get-url", "--push", &remote]).await.unwrap_or_default().trim());
    let tracking_ref = format!("refs/remotes/{remote}/{branch}");
    let tracking = read(root, &["rev-parse", "--verify", "-q", &tracking_ref]).await.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let upstream = upstream_of(root, &branch).await;
    let behind = if tracking.is_some() { count_pair(root, &format!("{tracking_ref}...refs/heads/{branch}")).await.0 } else { 0 };
    let mut log_args = vec!["log".to_string(), format!("--max-count={PUSH_COMMITS}"), "--format=%h %s".into()];
    if tracking.is_some() {
        log_args.push(format!("{tracking_ref}..refs/heads/{branch}"));
    } else {
        log_args.extend([format!("refs/heads/{branch}"), "--not".into(), format!("--remotes={remote}")]);
    }
    let commits = lines(&git_args(root, &log_args, READ_MS).await.text().unwrap_or_default());
    let changed = status_of(root).await.map(|e| e.len()).unwrap_or(0);
    let mut not_pushed: Vec<String> = Vec::new();
    if changed > 0 {
        let (is, it) = if changed == 1 { ("is", "it was") } else { ("are", "they were") };
        not_pushed.push(format!("{} in {repo} {is} not committed, so {it} not pushed.", plural(changed, "changed file")));
    }
    let mut out = json!({"repo": repo, "path": root, "branch": branch, "remote": remote, "url": url});
    if behind > 0 {
        let mut list = vec![format!("{branch} is {} behind {remote}/{branch}. Nixa never forces and never pulls. Pull first, then push again.", plural(behind as usize, "commit"))];
        list.extend(not_pushed);
        out["pushed"] = json!([]);
        out["not_pushed"] = json!(list);
        return Ok(out);
    }
    if commits.is_empty() {
        out["pushed"] = json!([]);
        out["tip"] = json!(tip.chars().take(7).collect::<String>());
        out["note"] = json!(format!("Nothing to push: {branch} has no commit that {remote} lacks."));
        out["not_pushed"] = json!(not_pushed);
        return Ok(out);
    }
    let what = push_ask(&branch, &repo, &remote, &commits);
    if !crate::yes_once::approved_once(app, &HELD, &format!("{root}\n{branch}\n{remote}\n{tip}"), &["push"], &what) {
        out["pushed"] = json!([]);
        out["held"] = json!(true);
        out["note"] = json!(crate::yes_once::held_note(&what, "push"));
        out["not_pushed"] = json!(not_pushed);
        return Ok(out);
    }
    let fixed = prepare(&prep_for(&app.dir, root, true)).await;
    let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
    let mut push_args = vec!["push".to_string(), "--porcelain".into(), "--no-follow-tags".into(), "--recurse-submodules=no".into()];
    if upstream.is_empty() {
        push_args.push("--set-upstream".into());
    }
    push_args.extend([remote.clone(), refspec.clone()]);
    let sent = git_args(root, &push_args, PUSH_MS).await;
    let result = lines(&sent.stdout).into_iter().find(|l| l.contains(&refspec)).unwrap_or_default();
    let flag = push_flag(&result);
    let remote_tip = read(root, &["rev-parse", "--verify", "-q", &tracking_ref]).await.map(|s| s.trim().to_string()).unwrap_or_default();
    let done = sent.ok() && [' ', '*', '='].contains(&flag);
    crate::util::log("git", &format!("push {branch} of {root} to {remote}: {}", if done { "ok" } else { "failed" }));
    out["tip"] = json!(tip.chars().take(7).collect::<String>());
    if !fixed.notes.is_empty() {
        out["identity"] = json!(fixed.notes);
    }
    if !done {
        let why = if flag == '!' {
            let reason = result.split('\t').skip(2).collect::<Vec<_>>().join(" ");
            format!("{remote} refused it: {}. Nixa never forces. Pull first if the remote has new commits.", clip(&reason, 200))
        } else {
            let raw = if sent.stderr.trim().is_empty() { &sent.stdout } else { &sent.stderr };
            let why = tail(raw, 400);
            format!("git push failed: {}.", if why.is_empty() { "no output".into() } else { why })
        };
        let mut list = vec![why];
        list.extend(not_pushed);
        out["pushed"] = json!([]);
        out["not_pushed"] = json!(list);
        return Ok(out);
    }
    app.info(&format!("Nixa pushed {} of {branch} in {repo} to {remote}.", plural(commits.len(), "commit")));
    for other in other_branches(root, &branch).await {
        not_pushed.push(format!("Other branch not pushed: {other}."));
    }
    out["pushed"] = json!(commits);
    if !remote_tip.is_empty() {
        out["remote_tip"] = json!(remote_tip.chars().take(7).collect::<String>());
    }
    if upstream.is_empty() {
        out["upstream_set"] = json!(format!("{remote}/{branch}"));
    }
    out["not_pushed"] = json!(not_pushed);
    Ok(out)
}
