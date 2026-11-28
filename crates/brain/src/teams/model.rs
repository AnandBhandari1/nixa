use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const TEAMS_FILE: &str = "teams.json";
pub const RUNS_FILE: &str = "team-runs.json";
pub const RUNS_KEEP: usize = 50;
pub const MAX_ROUNDS: u32 = 3;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Role {
    pub id: String,
    pub title: String,
    pub words: Vec<String>,
    pub does: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TeamStep {
    pub agent: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub purpose: String,
    pub steps: Vec<TeamStep>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub mcps: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub flow: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub migrated_from: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TeamsFile {
    pub version: u32,
    pub roles: Vec<Role>,
    pub teams: Vec<Team>,
}

impl Default for TeamsFile {
    fn default() -> TeamsFile {
        TeamsFile { version: 1, roles: Vec::new(), teams: Vec::new() }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    #[default]
    Waiting,
    Working,
    Done,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Running,
    Done,
    #[default]
    Failed,
    Stopped,
    Cut,
}

impl RunState {
    pub fn word(self) -> &'static str {
        match self {
            RunState::Running => "running",
            RunState::Done => "done",
            RunState::Failed => "failed",
            RunState::Stopped => "stopped",
            RunState::Cut => "cut",
        }
    }

    pub fn open(self) -> bool {
        matches!(self, RunState::Running | RunState::Cut | RunState::Stopped)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum By {
    #[default]
    Button,
    Voice,
    Schedule,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pass,
    Fail,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunStep {
    pub agent: String,
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    pub state: StepState,
    pub round: u32,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub terminal: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Verdict>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub report: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunPr {
    pub number: u64,
    pub title: String,
    pub url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub author: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub head: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Run {
    pub id: String,
    pub team: String,
    pub team_name: String,
    pub task: String,
    pub project: String,
    pub by: By,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub space: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub space_name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub repo: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub branch: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub mcps: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub brief: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub flow: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr: Option<RunPr>,
    pub state: RunState,
    pub step: usize,
    pub rounds: u32,
    pub started_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    pub steps: Vec<RunStep>,
}

pub fn parse_runs(text: Option<&str>) -> Vec<Run> {
    let Some(text) = text.filter(|t| !t.trim().is_empty()) else {
        return Vec::new();
    };
    let Ok(raw) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let list = raw.get("runs").and_then(Value::as_array).cloned().unwrap_or_default();
    let runs: Vec<Run> = list
        .into_iter()
        .filter_map(|v| serde_json::from_value::<Run>(v).ok())
        .filter(|r| !r.id.trim().is_empty() && !r.team.trim().is_empty())
        .map(|mut r| {
            if r.team_name.trim().is_empty() {
                r.team_name = r.team.clone();
            }
            for step in &mut r.steps {
                if step.name.trim().is_empty() {
                    step.name = step.agent.clone();
                }
            }
            r.steps.retain(|s| !s.agent.trim().is_empty());
            r
        })
        .collect();
    let skip = runs.len().saturating_sub(RUNS_KEEP);
    runs.into_iter().skip(skip).collect()
}

pub fn runs_text(runs: &[Run]) -> String {
    let skip = runs.len().saturating_sub(RUNS_KEEP);
    let kept: Vec<&Run> = runs.iter().skip(skip).collect();
    let body = serde_json::json!({"version": 1, "runs": kept});
    format!("{}\n", serde_json::to_string_pretty(&body).unwrap_or_else(|_| "{}".into()))
}

pub fn cut_running(runs: &mut [Run]) -> bool {
    let mut cut = false;
    for run in runs.iter_mut().filter(|r| r.state == RunState::Running) {
        run.state = RunState::Cut;
        run.note = format!("Nixa stopped during step {}. Press Resume to go on.", run.step + 1);
        cut = true;
    }
    cut
}

pub fn last_run_of<'a>(runs: &'a [Run], team: &str) -> Option<&'a Run> {
    runs.iter().rev().find(|r| r.team == team)
}

pub fn open_runs_of<'a>(runs: &'a [Run], team: &str) -> Vec<&'a Run> {
    runs.iter().rev().filter(|r| r.team == team && r.state.open()).collect()
}

pub fn shown_runs_of<'a>(runs: &'a [Run], team: &str, most: usize) -> Vec<&'a Run> {
    let open: Vec<&Run> = open_runs_of(runs, team).into_iter().take(most).collect();
    if !open.is_empty() {
        return open;
    }
    last_run_of(runs, team).into_iter().collect()
}

pub fn running_in<'a>(runs: &'a [Run], team: &str, project: &str) -> Option<&'a Run> {
    runs.iter().find(|r| r.team == team && r.state == RunState::Running && r.project == project)
}

pub fn space_label(run: &Run) -> String {
    if !run.space_name.is_empty() {
        return run.space_name.clone();
    }
    run.project.rsplit('/').find(|p| !p.is_empty()).unwrap_or(&run.project).to_string()
}
