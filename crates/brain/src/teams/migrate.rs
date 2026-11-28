use super::crew::{self, roles_of};
use super::model::{Role, Team, TeamStep, TeamsFile};
use anyhow::{Result, anyhow, bail};
use nixa_store::agents::{AddAgent, CustomAgent, StartProfile};
use regex::Regex;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

pub const NEW_MARK: &str = "new:";
pub const BY: &str = "team migration";
pub const SKIPPED: [(&str, &str); 1] = [("drone", "drones were removed")];

pub struct ModelMove {
    pub job: &'static str,
    pub agent: &'static str,
    pub model: &'static str,
    pub effort: &'static str,
    pub why: &'static str,
}

pub const MODEL_MOVES: [ModelMove; 5] = [
    ModelMove { job: "developer", agent: "claude", model: "claude-opus-5-5", effort: "high", why: "#417: builders run on Claude Opus" },
    ModelMove { job: "junior-developer", agent: "claude", model: "claude-opus-5-5", effort: "", why: "#417: builders run on Claude Opus" },
    ModelMove { job: "designer", agent: "claude", model: "claude-opus-5-5", effort: "high", why: "#417: builders run on Claude Opus" },
    ModelMove { job: "devops", agent: "claude", model: "claude-opus-5-5", effort: "high", why: "deep brain: DevOps writes scripts and config, so it is a builder (#417 rule A)" },
    ModelMove { job: "qa-tester", agent: "cursor", model: "composer-2.5", effort: "", why: "deep brain: tests and browser checks run on cursor, never Claude (#417 rule B)" },
];

static OLD_HAND: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"hand jobs to team members with [a-z]+-team or [a-z]+_start_member").expect("valid pattern"));
static OLD_START: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b[a-z]+_start_member\b").expect("valid pattern"));

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct OldMember {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub role: String,
    pub agent: String,
    pub model: String,
    pub effort: String,
    pub args: Vec<String>,
    pub rules: String,
    pub status: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct OldStep {
    pub member: String,
    pub note: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct OldCrew {
    pub id: String,
    pub name: String,
    pub purpose: String,
    pub steps: Vec<OldStep>,
    pub skills: Vec<String>,
    pub mcps: Vec<String>,
    pub flow: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct OldTeam {
    pub roles: Vec<Role>,
    pub members: Vec<OldMember>,
    pub teams: Vec<OldCrew>,
}

pub fn parse_old(text: &str) -> Result<OldTeam> {
    let mut old: OldTeam = serde_json::from_str(text).map_err(|e| anyhow!("The old team file is not valid JSON: {e}."))?;
    if let Some(at) = old.roles.iter().position(|r| r.id.trim().is_empty()) {
        bail!("Role {} in the old team file has no id. Fix it by hand first.", at + 1);
    }
    if let Some(at) = old.members.iter().position(|m| m.name.trim().is_empty()) {
        bail!("Member {} in the old team file has no name. Fix it by hand first.", at + 1);
    }
    if let Some(at) = old.teams.iter().position(|t| t.name.trim().is_empty()) {
        bail!("Team {} in the old team file has no name. Fix it by hand first.", at + 1);
    }
    for member in &mut old.members {
        member.name = member.name.trim().to_string();
        if member.id.trim().is_empty() {
            member.id = crew::slug(&member.name);
        }
        for word in [&mut member.model, &mut member.effort] {
            if word.trim() == "default" {
                word.clear();
            }
        }
    }
    for team in &mut old.teams {
        if team.id.trim().is_empty() {
            team.id = crew::slug(&team.name);
        }
        team.steps.retain(|s| !s.member.trim().is_empty());
    }
    for role in &mut old.roles {
        role.words = role.words.iter().map(|w| w.to_lowercase()).collect();
        if role.title.trim().is_empty() {
            role.title = role.id.clone();
        }
    }
    Ok(old)
}

pub fn fix_rules(rules: &str) -> String {
    let text = OLD_HAND.replace_all(rules, "hand jobs to custom agents with nixa task agents use (or the nixa-delegate skill)");
    OLD_START.replace_all(&text, "nixa task agents use").to_string()
}

fn job_ok(job: &str) -> bool {
    job.len() <= 40 && job.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) && job.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn profile_line(start: &StartProfile) -> String {
    let model = if start.model.is_empty() { "default model" } else { &start.model };
    let effort = if start.effort.is_empty() { "default effort" } else { &start.effort };
    format!("{} · {model} · {effort}", start.agent)
}

pub struct NewAgent {
    pub member: String,
    pub input: AddAgent,
}

pub struct Moved {
    pub member: String,
    pub from: String,
    pub to: String,
    pub why: String,
}

pub struct Plan {
    pub added: Vec<NewAgent>,
    pub kept: Vec<(String, String)>,
    pub skipped: Vec<(String, String)>,
    pub moved: Vec<Moved>,
    pub teams: Vec<Team>,
    pub roles: Vec<Role>,
}

impl Plan {
    pub fn empty(&self) -> bool {
        self.added.is_empty() && self.teams.is_empty() && self.roles.is_empty()
    }
}

pub fn plan(old: &OldTeam, agents: &[CustomAgent], file: &TeamsFile) -> Result<Plan> {
    let mut out = Plan { added: Vec::new(), kept: Vec::new(), skipped: Vec::new(), moved: Vec::new(), teams: Vec::new(), roles: Vec::new() };
    let mut id_of: Vec<(String, String)> = Vec::new();
    let roles = if old.roles.is_empty() { crew::default_roles() } else { old.roles.clone() };
    for member in &old.members {
        if member.kind == "person" {
            out.skipped.push((member.id.clone(), "a person card, not an agent".into()));
            continue;
        }
        if let Some((_, why)) = SKIPPED.iter().find(|(id, _)| *id == member.id) {
            out.skipped.push((member.id.clone(), why.to_string()));
            continue;
        }
        if let Some(done) = agents.iter().find(|a| a.migrated_from == member.id) {
            id_of.push((member.id.clone(), done.id.clone()));
            out.kept.push((member.id.clone(), done.id.clone()));
            continue;
        }
        let lower = member.name.to_lowercase();
        if let Some(clash) = agents.iter().find(|a| a.title.to_lowercase() == lower) {
            bail!("{}: a custom agent with this title exists already ({}). Rename one first.", member.name, clash.id);
        }
        if out.added.iter().any(|a| a.input.title.to_lowercase() == lower) {
            bail!("{}: two members have this name. Rename one first.", member.name);
        }
        let job = member.role.trim().to_lowercase();
        if !job.is_empty() && !job_ok(&job) {
            bail!("{}: the role \"{}\" is not a job id like developer or code-reviewer.", member.name, member.role);
        }
        let mv = MODEL_MOVES.iter().find(|m| m.job == job);
        let wanted = match mv {
            Some(m) => StartProfile { agent: m.agent.into(), model: m.model.into(), effort: m.effort.into(), args: Vec::new() },
            None => StartProfile { agent: member.agent.clone(), model: member.model.clone(), effort: member.effort.clone(), args: member.args.clone() },
        };
        let Some(start) = nixa_store::agents::read_profile(&wanted) else {
            bail!("{}: the agent \"{}\" cannot be a start profile.", member.name, wanted.agent);
        };
        let role = roles.iter().find(|r| r.id == job);
        let does = role.map(|r| r.does.trim().to_string()).unwrap_or_default();
        let rules = fix_rules(member.rules.trim());
        let instructions = [if does.is_empty() { String::new() } else { format!("You own: {does}") }, if rules.is_empty() { String::new() } else { format!("Job rules: {rules}") }]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        let purpose = if !does.is_empty() { does.clone() } else { role.map(|r| r.title.clone()).unwrap_or_else(|| member.name.clone()) };
        if mv.is_some() {
            let from = StartProfile { agent: member.agent.clone(), model: member.model.clone(), effort: member.effort.clone(), args: Vec::new() };
            out.moved.push(Moved { member: member.id.clone(), from: profile_line(&from), to: profile_line(&start), why: mv.map(|m| m.why.to_string()).unwrap_or_default() });
        }
        out.added.push(NewAgent {
            member: member.id.clone(),
            input: AddAgent {
                title: member.name.clone(),
                purpose: crate::util::clip(&purpose, 199),
                instructions: instructions.chars().take(2000).collect(),
                job,
                start: Some(start),
                off: member.status == "bench",
                migrated_from: member.id.clone(),
                ..AddAgent::default()
            },
        });
        id_of.push((member.id.clone(), format!("{NEW_MARK}{}", member.id)));
    }
    for old_team in &old.teams {
        if file.teams.iter().any(|t| t.migrated_from == old_team.id || t.id == old_team.id) {
            continue;
        }
        let mut steps = Vec::new();
        for step in &old_team.steps {
            let Some((_, id)) = id_of.iter().find(|(m, _)| *m == step.member) else {
                bail!("Team {}: step member {} has no custom agent.", old_team.name, step.member);
            };
            steps.push(TeamStep { agent: id.clone(), note: step.note.trim().to_string() });
        }
        out.teams.push(Team {
            id: old_team.id.clone(),
            name: old_team.name.trim().to_string(),
            purpose: old_team.purpose.trim().to_string(),
            steps,
            skills: crew::names(&old_team.skills),
            mcps: crew::names(&old_team.mcps),
            flow: if old_team.flow == "pr" { "pr".into() } else { String::new() },
            migrated_from: old_team.id.clone(),
        });
    }
    if file.roles.is_empty() {
        out.roles = roles;
    }
    Ok(out)
}

pub fn plan_text(plan: &Plan) -> String {
    let mut lines = vec![format!("New custom agents: {}.", plan.added.len())];
    for a in &plan.added {
        let start = a.input.start.as_ref().map(profile_line).unwrap_or_default();
        lines.push(format!(
            "- \"{}\" from member {}, job {}, starts as {start}{}, no space (starts in the run's space), instructions {} chars.",
            a.input.title,
            a.member,
            if a.input.job.is_empty() { "none" } else { &a.input.job },
            if a.input.off { ", off" } else { "" },
            a.input.instructions.chars().count()
        ));
    }
    if !plan.kept.is_empty() {
        let kept: Vec<String> = plan.kept.iter().map(|(m, id)| format!("{m} -> {id}")).collect();
        lines.push(format!("Already migrated, unchanged: {}.", kept.join(", ")));
    }
    let moved: Vec<String> = plan.moved.iter().map(|m| format!("{} {} -> {} ({})", m.member, m.from, m.to, m.why)).collect();
    lines.push(format!("Model changes: {}.", if moved.is_empty() { "none".into() } else { moved.join("; ") }));
    let skipped: Vec<String> = plan.skipped.iter().map(|(m, why)| format!("{m} ({why})")).collect();
    lines.push(format!("Skipped: {}.", if skipped.is_empty() { "none".into() } else { skipped.join(", ") }));
    let teams: Vec<String> = plan
        .teams
        .iter()
        .map(|t| {
            let steps: Vec<&str> = t.steps.iter().map(|s| s.agent.as_str()).collect();
            format!("{} [{}]{}", t.id, steps.join(" -> "), if t.flow.is_empty() { String::new() } else { format!(" flow {}", t.flow) })
        })
        .collect();
    lines.push(format!("Teams moved to teams.json: {}.", if teams.is_empty() { "none".into() } else { teams.join("; ") }));
    lines.push(format!("Roles copied to teams.json: {}.", if plan.roles.is_empty() { "none (already there)".to_string() } else { plan.roles.len().to_string() }));
    lines.push("The old team file is not changed. Saved custom agents are not changed.".into());
    lines.join("\n")
}

pub fn allowed_roots(data_dir: &Path) -> Vec<PathBuf> {
    let import = nixa_protocol::paths::dev_root().map(|r| r.join(".config").join("import"));
    let roots = [Some(data_dir.to_path_buf()), import, Some(std::env::temp_dir())];
    roots.iter().flatten().filter_map(|r| std::fs::canonicalize(r).ok()).collect()
}

pub fn source_ok(from: &Path, roots: &[PathBuf]) -> Result<PathBuf> {
    let real = std::fs::canonicalize(from).map_err(|_| anyhow!("The old team file {} does not exist.", from.display()))?;
    if !real.is_file() {
        bail!("{} is not a file.", from.display());
    }
    if !roots.iter().any(|r| real.starts_with(r)) {
        let shown: Vec<String> = roots.iter().map(|r| r.display().to_string()).collect();
        bail!("Migrate reads only a copy of the old team file inside {}. Copy it there first.", shown.join(" or "));
    }
    Ok(real)
}

pub fn apply(conn: &nixa_store::Connection, teams_path: &Path, plan: &Plan, backup_dir: &Path) -> Result<Option<PathBuf>> {
    let mut file = crew::load(teams_path)?;
    let backup = if teams_path.exists() {
        nixa_protocol::paths::ensure_private_dir(backup_dir)?;
        let to = backup_dir.join(teams_path.file_name().unwrap_or_default());
        std::fs::copy(teams_path, &to)?;
        Some(to)
    } else {
        None
    };
    let tx = conn.unchecked_transaction()?;
    let mut made: Vec<(String, String)> = Vec::new();
    for a in &plan.added {
        let saved = nixa_store::agents::add_agent(&tx, &a.input, BY)?;
        made.push((format!("{NEW_MARK}{}", a.member), saved.id));
    }
    for team in &plan.teams {
        let mut team = team.clone();
        for step in &mut team.steps {
            if let Some((_, id)) = made.iter().find(|(mark, _)| *mark == step.agent) {
                step.agent = id.clone();
            }
            if step.agent.starts_with(NEW_MARK) {
                bail!("Team {}: step {} has no custom agent.", team.name, step.agent);
            }
        }
        file.teams.push(team);
    }
    if !plan.roles.is_empty() && file.roles.is_empty() {
        file.roles = plan.roles.clone();
    }
    crew::save(teams_path, &file)?;
    if let Err(e) = tx.commit() {
        if let Some(saved) = &backup {
            let _ = std::fs::copy(saved, teams_path);
        }
        return Err(e.into());
    }
    Ok(backup)
}

pub fn run_migration(data_dir: &Path, db_file: &Path, teams_path: &Path, from: &Path, write: bool, at: u64) -> Result<String> {
    let real = source_ok(from, &allowed_roots(data_dir))?;
    let old = parse_old(&std::fs::read_to_string(&real)?)?;
    let conn = nixa_store::open(db_file)?;
    let agents = nixa_store::agents::list_agents(&conn)?;
    let file = crew::load(teams_path)?;
    let plan = plan(&old, &agents, &file)?;
    let text = plan_text(&plan);
    if !write {
        return Ok(format!("DRY RUN, nothing written.\n{text}"));
    }
    if plan.empty() {
        return Ok(format!("Nothing to write: everything is migrated already.\n{text}"));
    }
    let backup_dir = data_dir.join("backups").join(format!("{}-{at}-team-migrate", crate::util::day_of(at)));
    let backup = apply(&conn, teams_path, &plan, &backup_dir)?;
    let saved = match backup {
        Some(path) => format!("Backup of teams.json: {}", path.display()),
        None => "There was no teams.json before, so no backup.".to_string(),
    };
    let after = crew::load(teams_path)?;
    Ok(format!("WRITTEN. {saved}\n{text}\nTeams now: {}. Roles: {}.", after.teams.len(), roles_of(&after).len()))
}
