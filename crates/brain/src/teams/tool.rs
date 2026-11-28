use super::crew::{self, names};
use super::model::By;
use super::prompt::run_brief;
use super::runs::{self, RunStart, briefs};
use super::sources::{find_space, mcp_names, pick_repo, skill_dir, skills_in};
use crate::app::{App, lock};
use crate::spaces::Space;
use crate::tools::{Args, ToolResult, need, schema, spec, strings, text};
use crate::turns::ToolSpec;
use serde_json::json;
use std::sync::Arc;

pub const NAME: &str = "nixa_team_run";
pub const ACTIONS: [&str; 8] = ["teams", "skills", "mcps", "repos", "start", "list", "stop", "resume"];

const ABOUT: &str = "Teams are custom agents that work one after another, for example the Feature team: Architect, Developer, QA Tester, Code Reviewer. Every team has an id (for example feature-team) the user may say instead of the name. teams lists the teams: id, name, flow, steps, default skills and MCP servers, plus the spaces. repos lists the git repos and branches of a space folder. start runs one team: give team (name or id), space (the space name the user said), repo (a repo name from repos when the space has more than one), branch (from repos, default the current branch) and task (the user's words). project is an absolute folder instead of a space. skills and mcps add names on top of the team's defaults. The same team can run in many spaces at once, but only once per space. Each step opens a new tab in that space. Starting is the user's yes for the whole run: a step that finds problems sends the work back one step (at most 3 rounds), and Nixa writes one chat line per step and speaks at the end. Nothing is committed. list shows the last runs with their ids. stop stops a running run (the tabs stay open). resume goes on with a stopped or cut off run.";

pub fn specs() -> Vec<ToolSpec> {
    vec![spec(
        NAME,
        ABOUT,
        schema(
            json!({
                "action": {"type": "string", "enum": ACTIONS},
                "team": {"type": "string", "description": "For start: the team name or id, for example Feature team or feature-team."},
                "task": {"type": "string", "description": "For start: the task, in the user's words."},
                "space": {"type": "string", "description": "For start and repos: the space name or id."},
                "repo": {"type": "string", "description": "For start: the repo name or folder from repos, when the space folder holds more than one repo."},
                "branch": {"type": "string", "description": "For start: the branch to work on, from repos. Default: the repo's current branch."},
                "project": {"type": "string", "description": "For start, repos and mcps: absolute project folder, when no space fits."},
                "skills": {"type": "array", "items": {"type": "string"}, "description": "For start: skill names for this run. Only names from the skills action."},
                "mcps": {"type": "array", "items": {"type": "string"}, "description": "For start: MCP server names for this run. Only names from the mcps action."},
                "run": {"type": "string", "description": "For stop and resume: the run id from list. Never invent ids."}
            }),
            &["action"],
        ),
    )]
}

pub fn folder_of(app: &App, args: &Args) -> Result<(String, Option<Space>), String> {
    let spaces = app.spaces();
    if let Some(wanted) = text(args, "space") {
        let space = find_space(&spaces, &wanted).cloned().ok_or_else(|| {
            let all: Vec<String> = spaces.iter().map(|s| s.name.clone()).collect();
            format!("No space \"{wanted}\". Spaces: {}.", if all.is_empty() { "none".to_string() } else { all.join(", ") })
        })?;
        return Ok((space.folder.clone(), Some(space)));
    }
    let Some(project) = text(args, "project") else {
        return Err("No space: name one, or give an absolute project folder.".into());
    };
    let space = crate::spaces::for_folder(&spaces, &project).cloned();
    Ok((project, space))
}

pub fn teams_view(app: &App) -> Result<serde_json::Value, String> {
    let file = runs::teams_file(app)?;
    let agents = runs::agents(app);
    let runs = lock(&app.teams.runs).clone();
    let teams: Vec<serde_json::Value> = file
        .teams
        .iter()
        .map(|t| {
            let steps: Vec<String> = t
                .steps
                .iter()
                .map(|s| {
                    let name = crew::step_name(&agents, &s.agent);
                    if s.note.is_empty() { name } else { format!("{name}: {}", s.note) }
                })
                .collect();
            let shown: Vec<serde_json::Value> = super::model::shown_runs_of(&runs, &t.id, 5).into_iter().map(run_brief).collect();
            json!({"id": t.id, "name": t.name, "flow": if t.flow.is_empty() { "task" } else { &t.flow }, "purpose": t.purpose, "steps": steps, "skills": t.skills, "mcps": t.mcps, "runs": shown})
        })
        .collect();
    let spaces: Vec<serde_json::Value> = app.spaces().iter().map(|s| json!({"id": s.id, "name": s.name, "folder": s.folder})).collect();
    Ok(json!({"teams": teams, "spaces": spaces}))
}

pub async fn call(app: &Arc<App>, args: &Args) -> ToolResult {
    let action = need(args, "action")?;
    match action.as_str() {
        "teams" => Ok(teams_view(app)?.to_string()),
        "skills" => Ok(json!({"skills": skills_in(&skill_dir())}).to_string()),
        "mcps" => {
            let folder = match folder_of(app, args) {
                Ok((folder, _)) => folder,
                Err(_) => String::new(),
            };
            Ok(json!({"mcps": mcp_names(&folder)}).to_string())
        }
        "repos" => {
            let (folder, _) = folder_of(app, args)?;
            let repos = crate::schedules::runner::repos(&folder).await;
            Ok(json!({"folder": folder, "repos": repos}).to_string())
        }
        "list" => Ok(json!({"runs": briefs(&lock(&app.teams.runs), 10)}).to_string()),
        "start" => {
            let (project, space) = folder_of(app, args)?;
            let repos = crate::schedules::runner::repos(&project).await;
            let wanted = text(args, "repo").unwrap_or_default();
            let repo = pick_repo(&repos, &wanted, &project);
            if !wanted.is_empty() && repo.is_none() {
                let all: Vec<String> = repos.iter().map(|r| r.name.clone()).collect();
                return Err(format!("No repo \"{wanted}\" in {project}. Repos: {}.", if all.is_empty() { "none".to_string() } else { all.join(", ") }));
            }
            let branch = text(args, "branch").or_else(|| repo.map(|r| r.current.clone()).filter(|c| !c.is_empty())).unwrap_or_default();
            if let Some(repo) = repo
                && !branch.is_empty()
                && !repo.branches.is_empty()
                && !repo.branches.contains(&branch)
            {
                return Err(format!("No branch \"{branch}\" in {}. Branches: {}.", repo.name, repo.branches.join(", ")));
            }
            let team = need(args, "team")?;
            if let Some(why) = runs::locked(app, space.as_ref(), repo.map(|r| r.folder.as_str()).unwrap_or(&project)) {
                return Err(why);
            }
            if let Some(held) = crate::modes::hold_work(app, "team-run", &format!("start the {team} team run"), None) {
                return Ok(held);
            }
            let pr_team = runs::teams_file(app).ok().and_then(|f| crate::teams::crew::find_team(&f, &team).map(|t| t.flow == "pr")).unwrap_or(false);
            if pr_team {
                let start = crate::git_tools::pr_flow::FlowStart {
                    team,
                    project: repo.map(|r| r.folder.clone()).unwrap_or(project),
                    branch,
                    space,
                    skills: names(&strings(args, "skills")),
                    mcps: names(&strings(args, "mcps")),
                    task: text(args, "task").unwrap_or_default(),
                    by: By::Voice,
                };
                let flow = crate::git_tools::pr_flow::start(app, start).await?;
                return Ok(serde_json::to_value(&flow).map_err(|e| e.to_string())?.to_string());
            }
            let input = RunStart {
                team,
                task: text(args, "task").unwrap_or_default(),
                project: repo.map(|r| r.folder.clone()).unwrap_or(project),
                by: By::Voice,
                space,
                repo: repo.map(|r| r.folder.clone()).unwrap_or_default(),
                branch,
                skills: names(&strings(args, "skills")),
                mcps: names(&strings(args, "mcps")),
                ..RunStart::default()
            };
            let run = runs::start(app, input)?;
            Ok(run_brief(&run).to_string())
        }
        "stop" => Ok(run_brief(&runs::stop(app, &need(args, "run")?)?).to_string()),
        "resume" => {
            let id = need(args, "run")?;
            if let Some(held) = crate::modes::hold_work(app, "team-step", &format!("resume the team run {id}"), None) {
                return Ok(held);
            }
            Ok(run_brief(&runs::resume(app, &id)?).to_string())
        }
        _ => Err(format!("Use one of: {}.", ACTIONS.join(", "))),
    }
}
