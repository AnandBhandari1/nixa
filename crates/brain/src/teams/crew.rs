use super::model::{Role, Team, TeamStep, TeamsFile};
use anyhow::{Result, anyhow, bail};
use nixa_store::agents::CustomAgent;
use std::path::Path;

const ROLES: [(&str, &str, &[&str], &str); 14] = [
    ("cto", "CTO", &["cto", "chief"], "Final say on plans, deletes, deploys and money. Nixa or the deep brain decides for it."),
    ("tech-lead", "Tech Lead", &["tech lead", "lead", "team lead", "master"], "Plans the work, splits it, hands it to team members, checks the result."),
    ("product-manager", "Product Manager", &["product manager", "pm", "product owner", "prd", "user story", "user stories", "requirements", "acceptance criteria", "roadmap"], "Writes the PRD, user stories and acceptance criteria, keeps the roadmap, checks that the work matches what users need."),
    ("architect", "Architect", &["architect", "architecture", "planner", "plan", "spec"], "Big plans, specs, test plans, architecture and spec reviews."),
    ("senior-developer", "Senior Developer", &["senior developer", "senior", "senior dev", "hard bug", "debug", "crash", "performance", "rust", "migration", "upgrade"], "Big features, hard bugs, crashes, performance, Rust, migrations."),
    ("developer", "Developer", &["developer", "dev", "coder", "programmer", "fix", "feature", "refactor", "sql", "mobile", "firestore"], "Normal fixes and features, refactors, SQL, mobile, screenshot to code."),
    ("junior-developer", "Junior Developer", &["junior developer", "junior", "quick edit", "small fix", "small edit", "boilerplate"], "Small exact edits and boilerplate."),
    ("qa-tester", "QA Tester", &["qa tester", "tester", "qa", "test", "testing", "e2e", "ci", "browser check", "criteria"], "Writes and runs tests, fixes failing tests and CI, browser checks, criteria checks."),
    ("code-reviewer", "Code Reviewer", &["code reviewer", "reviewer", "review", "code review"], "Finds bugs in code written by another company's model. Read only."),
    ("security-reviewer", "Security Reviewer", &["security reviewer", "security", "audit", "pentest"], "Security reviews of login, money and personal data. Read only."),
    ("designer", "Designer", &["designer", "ui", "ux", "design", "look", "mockup", "prototype"], "Layout, colours, polish, mockups and prototypes."),
    ("writer", "Writer", &["writer", "docs", "documentation", "copy", "text", "translation", "translate"], "Docs, text users read, translation."),
    ("researcher", "Researcher", &["researcher", "research", "read", "summary", "summarize", "analyst", "analysis", "lookup"], "Reads repos, summarizes, data analysis, lookups."),
    ("devops", "DevOps", &["devops", "deploy", "script", "config", "shell", "pipeline"], "Shell scripts, CI and config."),
];

pub fn default_roles() -> Vec<Role> {
    ROLES
        .iter()
        .map(|(id, title, words, does)| Role {
            id: id.to_string(),
            title: title.to_string(),
            words: words.iter().map(|w| w.to_string()).collect(),
            does: does.to_string(),
        })
        .collect()
}

pub fn roles_of(file: &TeamsFile) -> Vec<Role> {
    if file.roles.is_empty() { default_roles() } else { file.roles.clone() }
}

pub fn role_title(file: &TeamsFile, job: &str) -> String {
    roles_of(file).into_iter().find(|r| r.id == job).map(|r| r.title).unwrap_or_else(|| job.to_string())
}

pub fn name_key(text: &str) -> String {
    text.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "team".into() } else { out }
}

pub fn names(value: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in value.iter().flat_map(|v| v.split(',')) {
        let name = part.trim();
        if !name.is_empty() && !out.iter().any(|n| n == name) {
            out.push(name.to_string());
        }
    }
    out
}

pub fn good_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'))
}

pub fn load(path: &Path) -> Result<TeamsFile> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(TeamsFile::default()),
        Err(e) => bail!("Cannot read {}: {e}. Nothing was changed.", path.display()),
    };
    if text.trim().is_empty() {
        return Ok(TeamsFile::default());
    }
    let mut file: TeamsFile = serde_json::from_str(&text).map_err(|e| anyhow!("Cannot read {}: {e}. Nothing was changed.", path.display()))?;
    file.version = 1;
    file.teams.retain(|t| !t.name.trim().is_empty());
    for team in &mut file.teams {
        if team.id.trim().is_empty() {
            team.id = slug(&team.name);
        }
        team.steps.retain(|s| !s.agent.trim().is_empty());
    }
    file.roles.retain(|r| !r.id.trim().is_empty());
    Ok(file)
}

pub fn save(path: &Path, file: &TeamsFile) -> Result<()> {
    let text = format!("{}\n", serde_json::to_string_pretty(file)?);
    crate::util::atomic_write(path, &text)?;
    Ok(())
}

pub fn find_team<'a>(file: &'a TeamsFile, wanted: &str) -> Option<&'a Team> {
    let value = wanted.trim();
    let value = match value.get(..5) {
        Some(head) if head.eq_ignore_ascii_case("team:") => value[5..].trim(),
        _ => value,
    };
    let key = name_key(value);
    if key.is_empty() {
        return None;
    }
    file.teams
        .iter()
        .find(|t| t.id == value)
        .or_else(|| file.teams.iter().find(|t| name_key(&t.name) == key))
        .or_else(|| file.teams.iter().find(|t| name_key(&t.name) == format!("{key}team") || format!("{}team", name_key(&t.name)) == key))
}

pub fn title_of(agent: &CustomAgent) -> String {
    if agent.title.trim().is_empty() { agent.id.clone() } else { agent.title.clone() }
}

pub fn can_start(agent: &CustomAgent) -> bool {
    !agent.off && agent.start.as_ref().and_then(nixa_store::agents::read_profile).is_some()
}

pub struct Pick {
    pub agent: Option<CustomAgent>,
    pub note: String,
}

pub fn step_agent(file: &TeamsFile, agents: &[CustomAgent], id: &str) -> Pick {
    let Some(own) = agents.iter().find(|a| a.id == id) else {
        return Pick { agent: None, note: format!("No custom agent \"{id}\" any more.") };
    };
    let name = title_of(own);
    if can_start(own) {
        return Pick { agent: Some(own.clone()), note: String::new() };
    }
    let why = if own.off { "is turned off" } else { "has no start profile" };
    if !own.job.is_empty()
        && let Some(other) = agents.iter().find(|a| a.id != own.id && a.job == own.job && can_start(a))
    {
        return Pick { agent: Some(other.clone()), note: format!("{name} {why}, so {} does this step.", title_of(other)) };
    }
    let what = if own.job.is_empty() { "agent for this job".to_string() } else { role_title(file, &own.job) };
    Pick { agent: None, note: format!("{name} {why} and no other active custom agent is a {what}.") }
}

pub fn step_name(agents: &[CustomAgent], id: &str) -> String {
    agents.iter().find(|a| a.id == id).map(title_of).unwrap_or_else(|| id.to_string())
}

pub fn read_team(value: &serde_json::Value, agents: &[CustomAgent]) -> Result<Team> {
    let text = |key: &str| value.get(key).and_then(serde_json::Value::as_str).map(str::trim).unwrap_or_default().to_string();
    let list = |key: &str| -> Vec<String> {
        match value.get(key) {
            Some(serde_json::Value::Array(l)) => names(&l.iter().filter_map(|v| v.as_str().map(str::to_string)).collect::<Vec<_>>()),
            Some(serde_json::Value::String(s)) => names(std::slice::from_ref(s)),
            _ => Vec::new(),
        }
    };
    let name = text("name");
    if name.is_empty() {
        bail!("A team needs a name.");
    }
    if name.chars().count() > 60 {
        bail!("A team name is at most 60 characters.");
    }
    let id = { let given = text("id"); if given.is_empty() { slug(&name) } else { given } };
    if !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') || id.len() > 60 {
        bail!("A team id is small letters, numbers and dashes, for example feature-team.");
    }
    let steps: Vec<TeamStep> = value
        .get("steps")
        .and_then(serde_json::Value::as_array)
        .map(|l| {
            l.iter()
                .map(|s| TeamStep {
                    agent: s.get("agent").and_then(serde_json::Value::as_str).unwrap_or_default().trim().to_string(),
                    note: crate::util::clip(s.get("note").and_then(serde_json::Value::as_str).unwrap_or_default().trim(), 500),
                })
                .collect()
        })
        .unwrap_or_default();
    if steps.is_empty() {
        bail!("A team needs at least one step.");
    }
    for (at, step) in steps.iter().enumerate() {
        if !agents.iter().any(|a| a.id == step.agent) {
            bail!("Step {}: no custom agent \"{}\".", at + 1, step.agent);
        }
    }
    let skills = list("skills");
    let mcps = list("mcps");
    if let Some(bad) = skills.iter().chain(mcps.iter()).find(|n| !good_name(n)) {
        bail!("\"{bad}\" is not a skill or MCP server name.");
    }
    let flow = text("flow");
    if !flow.is_empty() && flow != "task" && flow != "pr" {
        bail!("The flow must be task or pr.");
    }
    Ok(Team {
        id,
        name,
        purpose: crate::util::clip(&text("purpose"), 300),
        steps,
        skills,
        mcps,
        flow: if flow == "pr" { flow } else { String::new() },
        migrated_from: String::new(),
    })
}

pub fn put_team(file: &mut TeamsFile, team: Team) {
    if let Some(found) = file.teams.iter_mut().find(|t| t.id == team.id) {
        let migrated = std::mem::take(&mut found.migrated_from);
        *found = Team { migrated_from: migrated, ..team };
    } else {
        file.teams.push(team);
    }
}

pub fn remove_team(file: &mut TeamsFile, id: &str) -> Result<Team> {
    let at = file.teams.iter().position(|t| t.id == id).ok_or_else(|| anyhow!("No team \"{id}\"."))?;
    Ok(file.teams.remove(at))
}
