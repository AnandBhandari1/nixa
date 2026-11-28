use super::changes::{changed_files, git_changes, parse_status};
use super::crew::{self, find_team, read_team, step_agent};
use super::flow::{Check, IDLE_GRACE_MS, Live, Next, STUCK_MS, Seen, after_step, check_step, resume_mode};
use super::migrate::{self, NEW_MARK};
use super::model::{self, MAX_ROUNDS, Run, RunState, RunStep, StepState, Team, TeamStep, TeamsFile, Verdict};
use super::prompt::{Mode, StepAsk, report_path, run_summary, step_prompt, step_result};
use super::sources::{McpFiles, mcp_names_in};
use super::watch;
use nixa_store::agents::{CustomAgent, StartProfile};
use serde_json::json;
use std::path::Path;

fn agent(id: &str, title: &str, job: &str, off: bool) -> CustomAgent {
    CustomAgent {
        id: id.into(),
        title: title.into(),
        purpose: "p".into(),
        instructions: String::new(),
        role: String::new(),
        job: job.into(),
        workflow: String::new(),
        start: Some(StartProfile { agent: "codex".into(), ..StartProfile::default() }),
        off,
        migrated_from: String::new(),
        provider: "codex".into(),
        session: String::new(),
        space: String::new(),
        space_name: String::new(),
        folder: String::new(),
        terminal: String::new(),
        created_at: 0,
        last_used: 0,
        by: String::new(),
        skills: Vec::new(),
    }
}

fn run_of(names: &[&str]) -> Run {
    Run {
        id: "run-1".into(),
        team: "feature-team".into(),
        team_name: "Feature team".into(),
        task: "Add the login page".into(),
        project: "/tmp/proj".into(),
        state: RunState::Running,
        steps: names.iter().map(|n| RunStep { agent: n.to_lowercase(), name: n.to_string(), ..RunStep::default() }).collect(),
        ..Run::default()
    }
}

fn teams_file() -> TeamsFile {
    TeamsFile {
        version: 1,
        roles: Vec::new(),
        teams: vec![
            Team { id: "feature-team".into(), name: "Feature team".into(), steps: vec![TeamStep { agent: "a1".into(), note: String::new() }], ..Team::default() },
            Team { id: "pr-team".into(), name: "PR team".into(), flow: "pr".into(), ..Team::default() },
        ],
    }
}

#[test]
fn runs_file_is_read_kindly_and_cut() {
    let mut list: Vec<serde_json::Value> = (0..60).map(|n| json!({"id": format!("r{n}"), "team": "t", "state": "done"})).collect();
    list.push(json!({"id": "", "team": "t"}));
    list.push(json!({"id": "bad", "team": "t", "state": "weird"}));
    list.push(json!({"id": "live", "team": "t", "state": "running", "step": 1, "steps": [{"agent": "a1"}, {"agent": "a2", "name": "Dev"}]}));
    let text = json!({"version": 1, "runs": list}).to_string();
    let mut runs = model::parse_runs(Some(&text));
    assert_eq!(runs.len(), model::RUNS_KEEP);
    assert_eq!(runs.last().unwrap().id, "live");
    assert_eq!(runs.last().unwrap().steps[0].name, "a1");
    assert!(!runs.iter().any(|r| r.id == "bad"));
    assert!(model::cut_running(&mut runs));
    let live = runs.last().unwrap();
    assert_eq!(live.state, RunState::Cut);
    assert!(live.note.contains("step 2"));
    assert!(model::parse_runs(Some("not json")).is_empty());
    assert!(model::parse_runs(None).is_empty());
    let back = model::parse_runs(Some(&model::runs_text(&runs)));
    assert_eq!(back, runs);
}

#[test]
fn open_and_shown_runs() {
    let mut a = run_of(&["Dev"]);
    a.state = RunState::Done;
    let mut b = run_of(&["Dev"]);
    b.id = "run-2".into();
    b.state = RunState::Stopped;
    let runs = vec![a.clone(), b.clone()];
    assert_eq!(model::shown_runs_of(&runs, "feature-team", 5).len(), 1);
    assert_eq!(model::shown_runs_of(&runs, "feature-team", 5)[0].id, "run-2");
    assert_eq!(model::shown_runs_of(&[a.clone()], "feature-team", 5)[0].id, "run-1");
    assert!(model::running_in(&runs, "feature-team", "/tmp/proj").is_none());
    assert_eq!(model::space_label(&a), "proj");
}

#[test]
fn finds_teams_by_id_name_and_short_name() {
    let file = teams_file();
    assert_eq!(find_team(&file, "feature-team").unwrap().id, "feature-team");
    assert_eq!(find_team(&file, "Feature Team").unwrap().id, "feature-team");
    assert_eq!(find_team(&file, "team:feature").unwrap().id, "feature-team");
    assert_eq!(find_team(&file, "pr").unwrap().id, "pr-team");
    assert!(find_team(&file, "").is_none());
    assert!(find_team(&file, "ops").is_none());
    assert!(find_team(&file, "é").is_none());
}

#[test]
fn step_agent_takes_a_stand_in_when_off() {
    let file = TeamsFile::default();
    let agents = vec![agent("a1", "Dev One", "developer", true), agent("a2", "Dev Two", "developer", false), agent("a3", "Tester", "qa-tester", true)];
    let pick = step_agent(&file, &agents, "a2");
    assert_eq!(pick.agent.unwrap().id, "a2");
    assert!(pick.note.is_empty());
    let pick = step_agent(&file, &agents, "a1");
    assert_eq!(pick.agent.unwrap().id, "a2");
    assert!(pick.note.contains("Dev Two does this step"));
    let pick = step_agent(&file, &agents, "a3");
    assert!(pick.agent.is_none());
    assert!(pick.note.contains("QA Tester"));
    assert!(step_agent(&file, &agents, "a9").note.contains("No custom agent"));
    let mut bare = agent("a4", "Dev Four", "developer", true);
    bare.start = None;
    let with_bare = vec![bare.clone(), agent("a2", "Dev Two", "developer", false)];
    let pick = step_agent(&file, &with_bare, "a4");
    assert_eq!(pick.agent.unwrap().id, "a2");
    assert!(pick.note.contains("Dev Four is turned off, so Dev Two does this step."));
    bare.off = false;
    let pick = step_agent(&file, &[bare.clone()], "a4");
    assert!(pick.agent.is_none());
    assert!(pick.note.contains("has no start profile"));
}

#[test]
fn work_folder_uses_the_picked_repo() {
    use super::runs::work_folder;
    let dir = tempfile::tempdir().unwrap();
    let space = std::fs::canonicalize(dir.path()).unwrap();
    let sub = space.join("subrepo");
    std::fs::create_dir_all(sub.join(".git")).unwrap();
    std::fs::create_dir_all(space.join("plain")).unwrap();
    let s = space.display().to_string();
    let sub_s = sub.display().to_string();
    assert_eq!(work_folder("", "", &s).unwrap(), s);
    assert_eq!(work_folder("", &sub_s, &s).unwrap(), sub_s);
    assert_eq!(work_folder("", "subrepo", &s).unwrap(), sub_s);
    assert_eq!(work_folder(&s, "subrepo", &s).unwrap(), s);
    assert!(work_folder("", "plain", &s).unwrap_err().contains("not a git repo"));
    assert!(work_folder("", "../x", &s).is_err());
    assert!(work_folder("", "missing", &s).unwrap_err().contains("not a folder"));
    let other = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(other.path().join(".git")).unwrap();
    assert!(work_folder("", &other.path().display().to_string(), &s).unwrap_err().contains("not inside the space folder"));
    assert_eq!(work_folder("", "", "").unwrap_err(), "");
}

#[test]
fn reads_the_result_line() {
    assert_eq!(step_result("Did it.\nRESULT: PASS"), Some(Verdict::Pass));
    assert_eq!(step_result("Did it.\n- **RESULT: fail** two bugs\nmore"), Some(Verdict::Fail));
    assert_eq!(step_result("RESULT: FAIL\nlater\nRESULT: PASS"), Some(Verdict::Pass));
    assert_eq!(step_result("No result here."), None);
    assert_eq!(step_result("the RESULT: PASS is inside"), None);
}

#[test]
fn step_prompts_follow_the_mode() {
    let mut run = run_of(&["Architect", "Developer", "Reviewer"]);
    run.skills = vec!["git-checks".into()];
    run.space_name = "Proj".into();
    run.steps[0].summary = "Planned it.".into();
    run.steps[0].report = "/tmp/proj/docs/team-runs/run-1/01-a1-r0.md".into();
    run.steps[0].files = vec!["plan.md".into()];
    let report = Path::new("/tmp/r.md");
    let skills = Path::new("/home/x/skills");
    let text = step_prompt(&StepAsk { run: &run, index: 1, purpose: "Build features", note: "write the code", mode: Mode::First, fresh: true, report, skill_dir: skills });
    assert!(text.starts_with("Team run: the Feature team, step 2 of 3. You are the Developer."));
    assert!(text.contains("Task: Add the login page"));
    assert!(text.contains("Team purpose: Build features"));
    assert!(text.contains("Your part: write the code"));
    assert!(text.contains("Space: Proj (/tmp/proj)"));
    assert!(text.contains("/home/x/skills/git-checks/SKILL.md"));
    assert!(text.contains("- Step 1, Architect: Planned it."));
    assert!(text.contains("Changed files: plan.md"));
    assert!(text.contains("write your final reply to /tmp/r.md"));
    assert!(text.contains("Do not commit, push or deploy yourself"));
    assert!(text.contains(&crate::status_rule::rule()), "a team member tells Nixa its status");
    run.rounds = 1;
    run.steps[2].summary = "Bug in login.".into();
    let fix = step_prompt(&StepAsk { run: &run, index: 1, purpose: "", note: "", mode: Mode::Fix, fresh: false, report, skill_dir: skills });
    assert!(fix.contains("The Reviewer (step 3) found problems. Fix them. This is round 1 of 3."));
    assert!(fix.contains("Bug in login."));
    assert!(!fix.contains("Task:"));
    let again = step_prompt(&StepAsk { run: &run, index: 2, purpose: "", note: "", mode: Mode::Recheck, fresh: false, report, skill_dir: skills });
    assert!(again.contains("The Developer fixed the problems you found."));
    let go_on = step_prompt(&StepAsk { run: &run, index: 2, purpose: "", note: "", mode: Mode::Resume, fresh: false, report, skill_dir: skills });
    assert!(go_on.contains("Nixa has no report from you"));
    run.brief = "/tmp/brief.md".into();
    let brief = step_prompt(&StepAsk { run: &run, index: 1, purpose: "", note: "", mode: Mode::First, fresh: true, report, skill_dir: skills });
    assert!(brief.contains("Read the brief first: /tmp/brief.md"));
    assert!(brief.contains("Earlier reports: step 1 Architect"));
    assert!(!brief.contains("The user commits") && brief.contains("do not wait for the user"));
    assert_eq!(super::prompt::rule_line(false), super::prompt::PLAIN_RULE);
}

#[test]
fn report_paths_do_not_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().display().to_string();
    let first = report_path(&project, "run-1", 0, "a1", 0);
    assert!(first.ends_with("docs/team-runs/run-1/01-a1-r0.md"));
    std::fs::create_dir_all(first.parent().unwrap()).unwrap();
    std::fs::write(&first, "x").unwrap();
    assert!(report_path(&project, "run-1", 0, "a1", 0).ends_with("01-a1-r0-2.md"));
}

#[test]
fn step_results_move_the_run() {
    let mut run = run_of(&["Dev", "Reviewer"]);
    let (line, next) = after_step(&mut run, 0, "Built it.\nRESULT: PASS", vec!["a.rs".into()], 5);
    assert_eq!(next, Next::Begin(1, Mode::First));
    assert!(line.contains("Dev is done, 1 changed file"));
    assert_eq!(run.steps[0].state, StepState::Done);
    assert_eq!(run.steps[0].files, vec!["a.rs".to_string()]);
    let (_, next) = after_step(&mut run, 1, "Two bugs.\nRESULT: FAIL", Vec::new(), 6);
    assert_eq!(next, Next::Begin(0, Mode::Fix));
    assert_eq!(run.rounds, 1);
    assert_eq!(run.steps[1].state, StepState::Failed);
    let (_, next) = after_step(&mut run, 0, "Fixed.\nRESULT: PASS", vec!["b.rs".into()], 7);
    assert_eq!(next, Next::Begin(1, Mode::Recheck));
    assert_eq!(run.steps[0].files, vec!["a.rs".to_string(), "b.rs".to_string()]);
    run.rounds = MAX_ROUNDS;
    let (_, next) = after_step(&mut run, 1, "Still bad.\nRESULT: FAIL", Vec::new(), 8);
    assert!(matches!(next, Next::End(RunState::Failed, ref note) if note.contains("after 3 rounds")));
    let (_, next) = after_step(&mut run, 1, "Good now.\nRESULT: PASS", Vec::new(), 9);
    assert_eq!(next, Next::End(RunState::Done, String::new()));
    let mut one = run_of(&["Dev", "Reviewer"]);
    let (_, next) = after_step(&mut one, 0, "Could not.\nRESULT: FAIL", Vec::new(), 1);
    assert!(matches!(next, Next::End(RunState::Failed, ref note) if note == "Step 1 (Dev) failed."));
    let mut plain = run_of(&["Dev"]);
    let (_, next) = after_step(&mut plain, 0, "Done, no result line.", Vec::new(), 1);
    assert_eq!(next, Next::End(RunState::Done, String::new()));
    assert_eq!(plain.steps[0].result, None);
}

#[test]
fn checks_use_idle_and_stuck_limits() {
    let run = run_of(&["Dev"]);
    let mut live = Live::default();
    assert_eq!(check_step(&mut live, &run, 0, Seen::Idle, false, 0), Check::Wait);
    match check_step(&mut live, &run, 0, Seen::Idle, false, STUCK_MS) {
        Check::Tell(text) => assert!(text.contains("has not started after 1 minute")),
        other => panic!("{other:?}"),
    }
    assert_eq!(check_step(&mut live, &run, 0, Seen::Idle, false, STUCK_MS * 3), Check::Wait);
    assert_eq!(check_step(&mut live, &run, 0, Seen::Working, false, STUCK_MS * 4), Check::Wait);
    assert!(live.seen_working && !live.warned);
    let t = STUCK_MS * 5;
    assert_eq!(check_step(&mut live, &run, 0, Seen::Idle, false, t), Check::Wait);
    assert_eq!(check_step(&mut live, &run, 0, Seen::Idle, false, t + IDLE_GRACE_MS - 1), Check::Wait);
    assert!(matches!(check_step(&mut live, &run, 0, Seen::Idle, false, t + IDLE_GRACE_MS), Check::End(RunState::Stopped, ref n) if n.contains("without writing its report")));
    let mut asking = Live::default();
    assert_eq!(check_step(&mut asking, &run, 0, Seen::Waiting, false, 10), Check::Wait);
    assert!(matches!(check_step(&mut asking, &run, 0, Seen::Waiting, false, 10 + STUCK_MS), Check::Tell(ref t) if t.contains("waits for an answer")));
    let mut done = Live::default();
    assert_eq!(check_step(&mut done, &run, 0, Seen::Idle, true, 1), Check::Finish);
    assert_eq!(check_step(&mut done, &run, 0, Seen::Gone, true, 1), Check::Finish);
    assert_eq!(check_step(&mut done, &run, 0, Seen::Working, true, 1), Check::Wait);
    assert!(matches!(check_step(&mut done, &run, 0, Seen::Gone, false, 1), Check::End(RunState::Stopped, ref n) if n.contains("was closed")));
}

#[test]
fn a_stuck_step_tells_the_brain() {
    let mut run = run_of(&["Dev"]);
    run.steps[0].terminal = "m7".into();
    let mut asking = Live::default();
    assert_eq!(check_step(&mut asking, &run, 0, Seen::Waiting, false, 10), Check::Wait);
    match check_step(&mut asking, &run, 0, Seen::Waiting, false, 10 + STUCK_MS) {
        Check::Tell(text) => assert!(text.contains("m7") && text.contains("nixa_answer_prompt") && !text.contains("Answer it there"), "{text}"),
        other => panic!("{other:?}"),
    }
    let mut idle = Live::default();
    assert_eq!(check_step(&mut idle, &run, 0, Seen::Idle, false, 0), Check::Wait);
    assert!(matches!(check_step(&mut idle, &run, 0, Seen::Idle, false, STUCK_MS), Check::Tell(ref t) if t.contains("folder trust") && t.contains("nixa_term_read")));
}

#[test]
fn resume_picks_the_right_mode() {
    let mut run = run_of(&["Dev", "Reviewer"]);
    assert_eq!(resume_mode(&run, 0), Mode::First);
    run.steps[0].terminal = "m4".into();
    assert_eq!(resume_mode(&run, 0), Mode::Resume);
    run.steps[1].state = StepState::Failed;
    assert_eq!(resume_mode(&run, 0), Mode::Fix);
}

#[test]
fn summary_lists_steps_and_files() {
    let mut run = run_of(&["Dev", "Reviewer"]);
    run.state = RunState::Done;
    run.rounds = 1;
    run.space_name = "Proj".into();
    run.steps[0].summary = "# Report\nAdded the page.\nRESULT: PASS".into();
    run.steps[0].files = vec!["b.rs".into(), "a.rs".into()];
    run.steps[1].summary = "Looked good.\nRESULT: PASS".into();
    run.steps[1].files = vec!["a.rs".into()];
    let s = run_summary(&run);
    assert!(s.chat.starts_with("The Feature team is done in Proj: Add the login page."));
    assert!(s.chat.contains("- Dev: Added the page."));
    assert!(s.chat.contains("All checks passed after 1 round."));
    assert!(s.chat.contains("2 changed files: a.rs, b.rs."));
    assert!(s.chat.ends_with("Nothing is committed."));
    assert!(s.spoken.contains("2 files changed."));
    run.state = RunState::Stopped;
    run.note = "Stopped by the user. The tabs stay open.".into();
    assert!(run_summary(&run).chat.starts_with("The Feature team in Proj is stopped."));
}

#[test]
fn status_lines_and_changed_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("new.txt"), "x").unwrap();
    let raw = b" M src/a.rs\0R  new.txt\0old.txt\0?? notes.md\0";
    let found = parse_status(dir.path(), raw);
    assert_eq!(found.len(), 3);
    assert!(found["new.txt"].starts_with("R :"));
    assert!(found["src/a.rs"].ends_with(":gone"));
    assert!(!found.contains_key("old.txt"));
    let mut after = found.clone();
    after.insert("src/a.rs".into(), " M:123".into());
    after.remove("notes.md");
    after.insert("z.rs".into(), "??:1".into());
    assert_eq!(changed_files(&found, &after), vec!["notes.md".to_string(), "src/a.rs".to_string(), "z.rs".to_string()]);
}

#[tokio::test]
async fn git_change_lists_on_a_temp_repo() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().display().to_string();
    let ok = std::process::Command::new("git").args(["init", "-q", &folder]).status().map(|s| s.success()).unwrap_or(false);
    if !ok {
        return;
    }
    let before = git_changes(&folder).await;
    assert!(before.is_empty());
    std::fs::write(dir.path().join("one.txt"), "1").unwrap();
    std::fs::create_dir_all(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub/two.txt"), "2").unwrap();
    let after = git_changes(&folder).await;
    assert_eq!(changed_files(&before, &after), vec!["one.txt".to_string(), "sub/two.txt".to_string()]);
    assert!(git_changes(&dir.path().join("missing").display().to_string()).await.is_empty());
}

#[test]
fn member_watch_warns_once_on_a_quiet_screen() {
    assert_eq!(watch::quiet_key("⠋ Working 12s\nline 3"), "Working s line");
    let mut memory = watch::Memory::default();
    let look = |screen: &'static str, working: bool| watch::Look { terminal: "m3", number: Some(3), agent: "a1", name: "Dev", tab: "Dev · task", working, screen };
    assert!(watch::due(&memory, "m3", 0, watch::READ_MS));
    assert!(watch::check(&mut memory, &look("same", true), 0, watch::QUIET_MS).is_none());
    assert!(!watch::due(&memory, "m3", 1000, watch::READ_MS));
    assert!(watch::check(&mut memory, &look("same ✻ 4", true), watch::QUIET_MS - 1, watch::QUIET_MS).is_none());
    let note = watch::check(&mut memory, &look("same ✶ 5", true), watch::QUIET_MS, watch::QUIET_MS).unwrap();
    assert!(note.contains("The Dev in tab \"Dev · task\", terminal 3 (m3) is working, but its screen has not changed for 15 minutes."));
    assert!(note.contains(watch::QUIET_AUTO) && !note.contains("Tell the user"), "{note}");
    assert!(watch::check(&mut memory, &look("same", true), watch::QUIET_MS * 2, watch::QUIET_MS).is_none());
    assert!(watch::check(&mut memory, &look("other", true), watch::QUIET_MS * 3, watch::QUIET_MS).is_none());
    assert!(watch::check(&mut memory, &look("other", false), watch::QUIET_MS * 5, watch::QUIET_MS).is_none());
    watch::keep(&mut memory, &std::collections::HashSet::new());
    assert!(memory.seen.is_empty());
}

#[test]
fn mcp_names_from_each_file() {
    let files = McpFiles {
        claude: Some(json!({"mcpServers": {"notes": {}}, "projects": {"/w/app": {"mcpServers": {"linear": {}}}, "/w/other": {"mcpServers": {"nope": {}}}}}).to_string()),
        project: Some(json!({"mcpServers": {"graft": {}}}).to_string()),
        codex: Some("[mcp_servers.maestro]\ncommand = \"x\"\n[mcp_servers.maestro.env]\n".into()),
        cursor: Some("not json".into()),
    };
    assert_eq!(mcp_names_in(&files, "/w/app/sub"), vec!["graft", "linear", "maestro", "notes"]);
}

#[test]
fn team_save_checks_its_fields() {
    let agents = vec![agent("a1", "Dev", "developer", false)];
    let team = read_team(&json!({"name": "Feature Team", "steps": [{"agent": "a1", "note": "code"}], "skills": "git-checks, git-checks", "flow": "task"}), &agents).unwrap();
    assert_eq!(team.id, "feature-team");
    assert_eq!(team.skills, vec!["git-checks".to_string()]);
    assert!(team.flow.is_empty());
    assert!(read_team(&json!({"name": "X", "steps": []}), &agents).is_err());
    assert!(read_team(&json!({"name": "X", "steps": [{"agent": "a9"}]}), &agents).unwrap_err().to_string().contains("a9"));
    assert!(read_team(&json!({"name": "X", "id": "Bad Id", "steps": [{"agent": "a1"}]}), &agents).is_err());
    assert!(read_team(&json!({"name": "X", "steps": [{"agent": "a1"}], "mcps": ["bad name"]}), &agents).is_err());
    let mut file = TeamsFile::default();
    crew::put_team(&mut file, team.clone());
    crew::put_team(&mut file, Team { purpose: "new".into(), ..team.clone() });
    assert_eq!(file.teams.len(), 1);
    assert_eq!(file.teams[0].purpose, "new");
    assert!(crew::remove_team(&mut file, "feature-team").is_ok());
    assert!(crew::remove_team(&mut file, "feature-team").is_err());
}

#[test]
fn teams_file_round_trip_and_errors() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("teams.json");
    let empty = crew::load(&path).unwrap();
    assert!(empty.teams.is_empty());
    assert_eq!(crew::roles_of(&empty).len(), 14);
    crew::save(&path, &teams_file()).unwrap();
    assert_eq!(crew::load(&path).unwrap().teams.len(), 2);
    std::fs::write(&path, "{bad").unwrap();
    assert!(crew::load(&path).unwrap_err().to_string().contains("Nothing was changed"));
}

fn old_team_text() -> String {
    json!({
        "version": 3,
        "roles": [{"id": "developer", "title": "Developer", "words": ["Dev"], "does": "Normal fixes and features."}, {"id": "code-reviewer", "title": "Code Reviewer", "does": "Finds bugs."}],
        "members": [
            {"id": "sam", "name": "Sam", "kind": "person", "role": "cto"},
            {"id": "dev", "name": "Developer", "kind": "agent", "role": "developer", "agent": "codex", "model": "gpt-5", "status": "active", "rules": "Always hand jobs to team members with old-team or old_start_member."},
            {"id": "rev", "name": "Code Reviewer", "kind": "agent", "role": "code-reviewer", "agent": "agy", "model": "default", "status": "bench"},
            {"id": "drone", "name": "Drone", "kind": "agent", "role": "developer", "agent": "codex"}
        ],
        "teams": [{"id": "feature", "name": "Feature team", "purpose": "Build", "steps": [{"member": "dev", "note": "code"}, {"member": "rev"}], "skills": ["git-checks"]}]
    })
    .to_string()
}

#[test]
fn migrate_plan_maps_members_and_teams() {
    let old = migrate::parse_old(&old_team_text()).unwrap();
    let plan = migrate::plan(&old, &[], &TeamsFile::default()).unwrap();
    assert_eq!(plan.added.len(), 2);
    let dev = &plan.added[0].input;
    assert_eq!(dev.title, "Developer");
    assert_eq!(dev.job, "developer");
    assert_eq!(dev.start.as_ref().unwrap().agent, "claude");
    assert_eq!(dev.start.as_ref().unwrap().model, "claude-opus-5-5");
    assert_eq!(dev.migrated_from, "dev");
    assert!(dev.instructions.starts_with("You own: Normal fixes and features."));
    assert!(dev.instructions.contains("hand jobs to custom agents with nixa task agents use"));
    let rev = &plan.added[1].input;
    assert!(rev.off);
    assert_eq!(rev.start.as_ref().unwrap().agent, "agy");
    assert!(rev.start.as_ref().unwrap().model.is_empty());
    assert_eq!(plan.moved.len(), 1);
    assert_eq!(plan.skipped.len(), 2);
    assert_eq!(plan.teams.len(), 1);
    assert_eq!(plan.teams[0].steps[0].agent, format!("{NEW_MARK}dev"));
    assert_eq!(plan.roles.len(), 2);
    let text = migrate::plan_text(&plan);
    assert!(text.contains("New custom agents: 2."));
    assert!(text.contains("Skipped: sam (a person card, not an agent), drone (drones were removed)."));
    let mut clash = agent("a1", "developer", "", false);
    clash.migrated_from = String::new();
    assert!(migrate::plan(&old, &[clash], &TeamsFile::default()).err().map(|e| e.to_string()).unwrap_or_default().contains("Rename one first"));
}

#[test]
fn migrate_writes_once_and_then_keeps() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("nixa.db");
    let teams = dir.path().join("teams.json");
    let from = dir.path().join("old-team.json");
    std::fs::write(&from, old_team_text()).unwrap();
    crew::save(&teams, &TeamsFile::default()).unwrap();
    let dry = migrate::run_migration(dir.path(), &db, &teams, &from, false, 1).unwrap();
    assert!(dry.starts_with("DRY RUN, nothing written."));
    assert!(crew::load(&teams).unwrap().teams.is_empty());
    let written = migrate::run_migration(dir.path(), &db, &teams, &from, true, 2).unwrap();
    assert!(written.starts_with("WRITTEN. Backup of teams.json:"), "{written}");
    let file = crew::load(&teams).unwrap();
    assert_eq!(file.teams.len(), 1);
    let conn = nixa_store::open(&db).unwrap();
    let saved = nixa_store::agents::list_agents(&conn).unwrap();
    assert_eq!(saved.len(), 2);
    let steps: Vec<&str> = file.teams[0].steps.iter().map(|s| s.agent.as_str()).collect();
    assert!(steps.iter().all(|s| saved.iter().any(|a| a.id == *s)));
    assert_eq!(file.roles.len(), 2);
    let again = migrate::run_migration(dir.path(), &db, &teams, &from, true, 3).unwrap();
    assert!(again.starts_with("Nothing to write"), "{again}");
    assert!(again.contains("Already migrated, unchanged: dev -> "));
}

#[test]
fn migrate_reads_only_allowed_folders() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("team.json");
    std::fs::write(&file, "{}").unwrap();
    let roots = vec![std::fs::canonicalize(dir.path()).unwrap()];
    assert!(migrate::source_ok(&file, &roots).is_ok());
    let other = tempfile::tempdir().unwrap();
    let outside = other.path().join("team.json");
    std::fs::write(&outside, "{}").unwrap();
    assert!(migrate::source_ok(&outside, &roots).unwrap_err().to_string().contains("only a copy"));
    assert!(migrate::source_ok(&dir.path().join("none.json"), &roots).is_err());
    assert!(migrate::parse_old("{\"members\": [{\"id\": \"x\"}]}").unwrap_err().to_string().contains("no name"));
}

#[tokio::test]
async fn a_locked_space_stops_resume() {
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let folder = std::fs::canonicalize(work.path()).unwrap().display().to_string();
    let app = crate::app::App::open(data.path()).unwrap();
    let run = Run {
        id: "run-1".into(),
        team: "t".into(),
        team_name: "Team".into(),
        task: "x".into(),
        project: folder.clone(),
        space: "s1".into(),
        state: RunState::Stopped,
        steps: vec![RunStep { agent: "a".into(), name: "A".into(), ..RunStep::default() }],
        ..Run::default()
    };
    crate::app::lock(&app.teams.runs).push(run.clone());
    assert!(super::runs::run_locked(&app, &run).is_none());
    let conn = nixa_store::open(&app.db_file).unwrap();
    let patch = nixa_store::spaces::SpacePatch { name: Some("Work".into()), folder: Some(folder), locked: Some(true), ..Default::default() };
    nixa_store::spaces::set_space(&conn, "s1", &patch).unwrap();
    let why = super::runs::resume_check(&app, "run-1").unwrap_err();
    assert!(why.contains("is locked"), "{why}");
    assert_eq!(crate::app::lock(&app.teams.runs)[0].state, RunState::Stopped);
    let other = Run { space: String::new(), ..run };
    assert!(super::runs::run_locked(&app, &other).is_some());
}
