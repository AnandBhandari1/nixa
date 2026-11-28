pub mod checks;
pub mod commit;
pub mod gate;
pub mod gh_gate;
pub mod github;
pub mod identity;
pub mod policy;
pub mod pr_flow;
pub mod push;
pub mod repo;
pub mod run;

#[cfg(test)]
mod checks_tests;
#[cfg(test)]
mod gate_tests;
#[cfg(test)]
mod tests;

use crate::app::App;
use crate::project_tools::find::{find_project_folder, projects_dir, spaces_of};
use crate::tools::{Args, ToolResult, need, number, schema, spec, text};
use crate::turns::ToolSpec;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;

pub const STATE: &str = "nixa_git_state";
pub const COMMIT: &str = "nixa_git_commit";
pub const PUSH: &str = "nixa_git_push";
pub const PR_FLOW: &str = "nixa_pr_flow";
pub const CHECKS: &str = "nixa_git_checks";
pub const NAMES: [&str; 5] = [STATE, COMMIT, PUSH, PR_FLOW, CHECKS];

pub struct State {
    pub flows: pr_flow::Flows,
}

impl State {
    pub fn open(dir: &Path) -> State {
        State { flows: pr_flow::Flows::open(dir) }
    }
}

pub fn specs() -> Vec<ToolSpec> {
    let project = json!({"type": "string", "description": "Space name, repo folder name, or absolute folder"});
    let repo = json!({"type": "string", "description": "Repo folder name, when the project folder holds several repos"});
    vec![
        spec(STATE, "Read git for one project or folder, repo by repo: branch, upstream, commits ahead and behind, changed files, last commits, other branches with unpushed commits, and the git account and profile of the repo. A folder that holds several repos shows each one. Read only. Ahead and behind are from the last fetch.", schema(json!({"project": project, "repo": repo}), &["project"])),
        spec(COMMIT, "Commit in one repo with a message: all its changed files, or only the files named. It runs without waiting for approval. It never commits env or secret files, AI or editor config, or markdown files other than README.md, and lists them as left out. It never pushes. Returns the proof: branch, commit, files, what was left out and what is still changed.", schema(json!({"project": project, "repo": repo, "message": {"type": "string", "description": "The commit message: what changed, one line"}, "files": {"type": "array", "items": {"type": "string"}, "description": "Optional: only these changed files, relative to the repo"}}), &["project", "message"])),
        spec(PUSH, "Push one named branch of one repo to its remote. Never main or master. The first call returns held; the push runs only after the user says yes push in chat and you call again with the same input. Returns the proof: branch, the commits pushed, and anything not pushed.", schema(json!({"project": project, "repo": repo, "branch": {"type": "string", "description": "The branch to push, for example dev. Never main or master."}}), &["project", "branch"])),
        spec(PR_FLOW, "The PR team flow: Nixa works through the open GitHub PRs of one repo, one PR after another, with a team whose flow is pr (start it with nixa_team_run start on that team). The flow itself never commits: a PR with picked parts leaves them uncommitted and ends the flow, so the next PRs wait for the next run. Commit those parts yourself with nixa_git_commit and go on. Nixa never posts comments, closes PRs or does anything else on GitHub: the user posts the comment drafts. list shows the last flows, what each PR got (picked, nothing, failed) and which comment drafts wait. comment shows the comment draft of one PR. stop stops a running flow. facts reads one PR from GitHub (title, author, state, branches, description), read only. checks runs the checks of the project's repo, the same as nixa_git_checks.", schema(json!({"action": {"type": "string", "enum": ["list", "comment", "stop", "facts", "checks"]}, "pr": {"type": "integer", "description": "For comment: the PR number."}, "flow": {"type": "string", "description": "For stop: the flow id from list."}, "url": {"type": "string", "description": "For facts: the PR link."}, "project": project}), &["action"])),
        spec(CHECKS, "Run the checks of one repo now and get a structured result for each check: ok, time, a short summary and the last lines of a failing one. The commands come from the space settings, else the Check and Test parts of docs/commands.md, else the Checks part of docs/nixa.md, else the repo's own check, typecheck, lint and test scripts or cargo check, clippy and test. found lists every command found; only picks some of them by number or exact text. At most 4 run at once. Automatic runs selected valid commands without approval. seconds is the whole time limit (default 120, at most 600). Nixa itself does not commit, push or change git here; the commands run as written in the repo folder.", schema(json!({"project": project, "repo": repo, "only": {"type": "array", "items": {"type": "string"}, "description": "Optional: numbers or exact commands from found"}, "seconds": {"type": "number"}}), &["project"])),
    ]
}

pub fn find(app: &App, args: &Args) -> Result<String, String> {
    find_project_folder(&need(args, "project")?, &spaces_of(&app.db_file), &projects_dir())
}

pub async fn pick_repos(app: &App, args: &Args, one: bool) -> Result<(String, Vec<String>), String> {
    let folder = find(app, args)?;
    let roots = repo::repos_in(&folder, &spaces_of(&app.db_file)).await;
    let roots = repo::pick(roots, &folder, text(args, "repo").as_deref(), one)?;
    Ok((folder, roots))
}

pub async fn git_state(app: &App, args: &Args) -> Result<Value, String> {
    let (folder, roots) = pick_repos(app, args, false).await?;
    let mut repos = Vec::new();
    for root in roots {
        repos.push(repo::state_of(&root).await?);
    }
    Ok(json!({"project": folder, "repos": repos, "observed_at": crate::util::now_ms()}))
}

pub fn flow_tool(app: &App, args: &Args) -> Result<Value, String> {
    match need(args, "action")?.as_str() {
        "list" => {
            let waiting: Vec<String> = pr_flow::prs(app).into_iter().filter(|p| p.comment_state == pr_flow::CommentState::Waiting).map(|p| format!("#{} {} ({})", p.number, p.title, repo::base_name(&p.repo))).collect();
            let flows: Vec<Value> = pr_flow::list(app).iter().rev().take(5).map(pr_flow::flow_brief).collect();
            Ok(json!({"flows": flows, "waiting_comments": waiting}))
        }
        "comment" => {
            let wanted = number(args, "pr").or_else(|| text(args, "pr").and_then(|t| t.trim_start_matches('#').parse().ok())).ok_or("Give the PR number.")?;
            let mut found: Vec<pr_flow::PrRecord> = pr_flow::prs(app).into_iter().filter(|p| p.number == wanted).collect();
            found.sort_by_key(|p| std::cmp::Reverse(p.at));
            let found = found.into_iter().next().ok_or_else(|| format!("No PR #{wanted} in the PR memory."))?;
            let draft = if found.comment.is_empty() { String::new() } else { std::fs::read_to_string(&found.comment).unwrap_or_default() };
            Ok(json!({"pr": format!("#{} {}", found.number, found.title), "outcome": found.outcome.word(), "comment_state": found.comment_state, "comment": if draft.trim().is_empty() { "(no draft)".to_string() } else { draft }}))
        }
        "post" | "close" | "merge" => Err("Nixa does not post PR comments, close or merge PRs on GitHub. The user does that.".into()),
        _ => Err("Use one of: list, comment, stop, facts.".into()),
    }
}

pub fn checks_held(app: &Arc<App>, args: &Args) -> Option<Value> {
    let project = text(args, "project").unwrap_or_default();
    let held = crate::modes::hold_work(app, "check", &format!("run the checks of {project}"), None)?;
    Some(json!({"ran": false, "held": true, "note": held}))
}

pub async fn run_checks_tool(app: &Arc<App>, args: &Args) -> Result<Value, String> {
    if let Some(held) = checks_held(app, args) {
        return Ok(held);
    }
    let (_, roots) = pick_repos(app, args, true).await?;
    checks::run_checks(app, &roots[0], args).await
}

pub async fn call(app: &Arc<App>, name: &str, args: &Args) -> Option<ToolResult> {
    if !NAMES.contains(&name) {
        return None;
    }
    let done: Result<Value, String> = match name {
        STATE => git_state(app, args).await,
        COMMIT => match pick_repos(app, args, true).await {
            Ok((_, roots)) => commit::commit(app, &roots[0], args).await,
            Err(e) => Err(e),
        },
        CHECKS => run_checks_tool(app, args).await,
        PUSH => match (pick_repos(app, args, true).await, need(args, "branch")) {
            (Ok((_, roots)), Ok(branch)) => push::push(app, &roots[0], &branch).await,
            (Err(e), _) | (_, Err(e)) => Err(e),
        },
        _ => match text(args, "action").as_deref() {
            Some("stop") => match need(args, "flow") {
                Ok(id) => pr_flow::stop(app, &id).map(|f| pr_flow::flow_brief(&f)),
                Err(e) => Err(e),
            },
            Some("checks") => run_checks_tool(app, args).await,
            Some("facts") => match need(args, "url") {
                Ok(url) => {
                    let folder = text(args, "project").and_then(|p| find_project_folder(&p, &spaces_of(&app.db_file), &projects_dir()).ok());
                    github::facts_for(app, &url, folder.as_deref()).await.and_then(|f| serde_json::to_value(f).map_err(|e| e.to_string()))
                }
                Err(e) => Err(e),
            },
            _ => flow_tool(app, args),
        },
    };
    Some(done.map(|v| v.to_string()))
}
