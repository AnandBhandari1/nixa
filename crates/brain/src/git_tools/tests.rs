use super::gh_gate::{agent_chain, gh_write};
use super::github::{RepoRef, api_base, parse_pr_url, parse_remote, read_facts, read_open, same_repo};
use super::identity::{Prep, Profile, RepoConfig, parse_config, helper_value, parse_profiles, pick_profile, prepare, repair_plan};
use super::policy::{clean_message, hide_credentials, is_protected_branch, never_commit, tail};
use super::pr_flow::{CommentState, DoneItem, FlowState, Outcome, PrFlow, WorkFile, brief_text, comment_ready, cut_running, end_text, flow_brief, parse_work, pr_key, wanted_number};
use super::repo::{named_roots, pair_of, parse_status, pick, relative_to};
use crate::app::{App, lock};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

fn s(list: &[&str]) -> Vec<String> {
    list.iter().map(|x| x.to_string()).collect()
}

fn sh(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").args(args).current_dir(dir).env("GIT_CONFIG_NOSYSTEM", "1").output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn new_repo(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    sh(dir, &["init", "-q", "-b", "main"]);
    for (k, v) in [("user.name", "Test User"), ("user.email", "test@example.invalid"), ("commit.gpgsign", "false"), ("core.hooksPath", "/dev/null")] {
        sh(dir, &["config", "--local", k, v]);
    }
    std::fs::write(dir.join("README.md"), "start\n").unwrap();
    sh(dir, &["add", "README.md"]);
    sh(dir, &["commit", "-q", "-m", "start"]);
}

fn real(dir: &Path) -> PathBuf {
    std::fs::canonicalize(dir).unwrap()
}

fn app_in(dir: &Path) -> Arc<App> {
    App::open(&dir.join("data")).unwrap()
}

fn said(app: &App, line: &str) {
    lock(&app.brain).last_user = Some((line.to_string(), crate::util::now_ms() + 1));
}

#[test]
fn branch_and_file_rules() {
    assert!(is_protected_branch("main"));
    assert!(is_protected_branch("refs/heads/Master"));
    assert!(!is_protected_branch("dev_main"));
    assert_eq!(hide_credentials("https://user:tok@github.com/a/b.git"), "https://github.com/a/b.git");
    assert_eq!(never_commit(".env.local"), Some("an env file"));
    assert_eq!(never_commit("keys/server.pem"), Some("a secret file"));
    assert_eq!(never_commit(".claude/settings.json"), Some("AI or editor config"));
    assert_eq!(never_commit("CLAUDE.md"), Some("AI config"));
    assert!(never_commit("docs/plan.md").is_some());
    assert_eq!(never_commit("README.md"), None);
    assert_eq!(never_commit("src/main.rs"), None);
    assert_eq!(clean_message(Some("  fix\r\nmore ")).unwrap(), "fix\nmore");
    assert!(clean_message(Some(" ")).is_err());
    assert!(clean_message(Some("x\nCo-authored-by: a <b>")).is_err());
    assert!(clean_message(Some(&"x".repeat(2001))).is_err());
    assert_eq!(tail("a\n\nb\nhttps://u:p@h/x", 100), "a | b | https://h/x");
}

#[test]
fn status_and_paths() {
    let list = parse_status(" M a.rs\0R  new.rs\0old.rs\0?? c d.txt\0");
    assert_eq!(list.len(), 3);
    assert_eq!(list[1].path, "new.rs");
    assert_eq!(list[1].from.as_deref(), Some("old.rs"));
    assert_eq!(list[2].code, "??");
    assert_eq!(list[2].path, "c d.txt");
    assert_eq!(relative_to("/r/app", "src/a.rs").unwrap(), "src/a.rs");
    assert_eq!(relative_to("/r/app", "/r/app/x/../y.rs").unwrap(), "y.rs");
    assert!(relative_to("/r/app", "../other").is_err());
    assert!(relative_to("/r/app", "/r/appx/a").is_err());
    assert_eq!(pair_of("3\t4\n"), (3, 4));
    assert_eq!(pair_of("x"), (0, 0));
    let roots = s(&["/p/one", "/p/two"]);
    assert!(pick(roots.clone(), "/p", None, true).unwrap_err().contains("Name one"));
    assert_eq!(pick(roots.clone(), "/p", Some("TWO"), true).unwrap(), vec!["/p/two"]);
    assert!(pick(roots.clone(), "/p", Some("x"), true).unwrap_err().contains("one, two"));
    assert!(pick(Vec::new(), "/p", None, false).is_err());
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("a/.git")).unwrap();
    std::fs::create_dir_all(dir.path().join("b")).unwrap();
    let base = dir.path().display().to_string();
    assert_eq!(named_roots(&base, &s(&["a", "b"])), vec![dir.path().join("a").display().to_string()]);
}

#[test]
fn profiles_and_repair_plan() {
    let profiles = parse_profiles(r#"{"profiles": [{"id": "work", "name": "W", "email": "w@example.invalid", "accounts": {"GitHub.com": "wuser"}, "folders": ["~/Work"]}, {"label": "no id"}, {"id": "home"}]}"#);
    assert_eq!(profiles.len(), 2);
    assert_eq!(profiles[0].accounts.get("github.com").map(String::as_str), Some("wuser"));
    assert_eq!(profiles[1].label, "home");
    assert_eq!(parse_profiles("[{\"id\": \"x\"}]").len(), 1);
    let config = parse_config("/h/Work/app", "user.name\nOld\0nixa.profile\nhome\0credential.https://github.com.helper\n\0credential.https://github.com.helper\nosxkeychain\0", "github.com");
    assert_eq!(config.name.as_deref(), Some("Old"));
    assert_eq!(config.helpers, s(&["", "osxkeychain"]));
    assert_eq!(pick_profile(&profiles, &config, Some(Path::new("/h"))).unwrap().id, "home");
    let free = RepoConfig { root: "/h/Work/app".into(), ..RepoConfig::default() };
    assert_eq!(pick_profile(&profiles, &free, Some(Path::new("/h"))).unwrap().id, "work");
    assert!(pick_profile(&profiles, &RepoConfig { root: "/elsewhere".into(), ..RepoConfig::default() }, Some(Path::new("/h"))).is_none());
    let plan = repair_plan(&free, &profiles[0], "/bin/helper", "github.com", true);
    let flat: Vec<String> = plan.iter().map(|p| p.join(" ")).collect();
    assert_eq!(flat, s(&["config --local user.name W", "config --local user.email w@example.invalid", "config --local nixa.profile work", "config --local --unset-all credential.https://github.com.helper", "config --local --add credential.https://github.com.helper ", "config --local --add credential.https://github.com.helper !'/bin/helper'", "config --local credential.https://github.com.username wuser"]));
    let bound = RepoConfig { root: "/h/Work/app".into(), name: Some("W".into()), email: Some("w@example.invalid".into()), profile: Some("work".into()), helpers: s(&["", "!'/bin/helper'"]), username: Some("wuser".into()) };
    assert!(repair_plan(&bound, &profiles[0], "/bin/helper", "github.com", true).is_empty());
    assert_eq!(repair_plan(&bound, &profiles[0], "/bin/helper", "github.com", false).len(), 3);
}

#[tokio::test]
async fn identity_repair_on_a_temp_repo() {
    let dir = tempfile::tempdir().unwrap();
    let root = real(dir.path()).join("app");
    new_repo(&root);
    let helper = dir.path().join("helper");
    std::fs::write(&helper, "#!/bin/sh\n").unwrap();
    let profile = Profile { id: "p1".into(), label: "Work".into(), name: "Work Name".into(), email: "work@example.invalid".into(), accounts: [("github.com".to_string(), "wuser".to_string())].into(), folders: vec![root.display().to_string()], owners: Vec::new() };
    let mut input = Prep { folder: root.display().to_string(), profiles: vec![profile], helper: helper.display().to_string(), host: "github.com".into(), repair: false, home: None };
    let checked = prepare(&input).await;
    assert!(checked.repaired.is_empty());
    assert!(checked.notes[0].contains("not set right"), "{:?}", checked.notes);
    assert_eq!(checked.account.as_deref(), Some("wuser"));
    input.repair = true;
    let fixed = prepare(&input).await;
    assert_eq!(fixed.repaired.len(), 1, "{:?}", fixed.notes);
    assert_eq!(sh(&root, &["config", "--local", "user.email"]), "work@example.invalid");
    assert_eq!(sh(&root, &["config", "--local", "nixa.profile"]), "p1");
    assert_eq!(sh(&root, &["config", "--local", "--get-all", "credential.https://github.com.helper"]), helper_value(&helper.display().to_string()));
    assert_eq!(sh(&root, &["config", "--local", "credential.https://github.com.username"]), "wuser");
    let again = prepare(&input).await;
    assert!(again.repaired.is_empty() && again.notes.is_empty(), "{:?}", again.notes);
    input.profiles.clear();
    assert!(prepare(&input).await.repos.is_empty());
}

#[test]
fn gh_gate_rules() {
    assert_eq!(gh_write(&s(&["pr", "view", "3"])), None);
    assert_eq!(gh_write(&s(&["pr", "merge", "3"])), Some("pr merge".into()));
    assert_eq!(gh_write(&s(&["-R", "a/b", "issue", "comment", "1"])), Some("issue comment".into()));
    assert_eq!(gh_write(&s(&["--repo", "a/b", "pr", "list"])), None);
    assert_eq!(gh_write(&s(&["pr"])), None);
    assert_eq!(gh_write(&s(&["search", "repos", "x"])), None);
    assert_eq!(gh_write(&s(&["auth", "token"])), Some("auth token".into()));
    assert_eq!(gh_write(&s(&["api", "repos/a/b"])), None);
    assert_eq!(gh_write(&s(&["api", "-X", "POST", "repos/a/b/issues"])), Some("api write".into()));
    assert_eq!(gh_write(&s(&["api", "--method=GET", "x"])), None);
    assert_eq!(gh_write(&s(&["api", "-XPATCH", "x"])), Some("api write".into()));
    assert_eq!(gh_write(&s(&["api", "x", "-f", "a=b"])), Some("api write".into()));
    assert_eq!(gh_write(&s(&["api", "graphql", "-F", "query=mutation { x }"])), Some("api write".into()));
    assert_eq!(gh_write(&s(&["repo", "delete", "a/b"])), Some("repo delete".into()));
    assert_eq!(gh_write(&s(&["alias"])), Some("alias".into()));
    assert_eq!(agent_chain(&s(&["/bin/zsh -l", "node /opt/bin/codex.js exec", "login"])), Some("codex".into()));
    assert_eq!(agent_chain(&s(&["/bin/zsh", "vim notes"])), None);
}

#[test]
fn github_links_and_facts() {
    let pr = parse_pr_url("https://GitHub.com/me/app/pull/42/files").unwrap();
    assert_eq!((pr.host.as_str(), pr.owner.as_str(), pr.repo.as_str(), pr.number), ("github.com", "me", "app", 42));
    assert!(parse_pr_url("https://github.com/me/app/issues/4").is_none());
    let https = parse_remote("https://tok@github.com/me/app.git").unwrap();
    let ssh = parse_remote("git@github.com:me/App.git").unwrap();
    let ssh2 = parse_remote("ssh://git@github.com/me/app").unwrap();
    assert_eq!(https, RepoRef { host: "github.com".into(), owner: "me".into(), repo: "app".into() });
    assert!(same_repo(Some(&https), Some(&ssh)));
    assert!(same_repo(Some(&https), Some(&ssh2)));
    assert!(!same_repo(Some(&https), None));
    assert_eq!(api_base("github.com", None), "https://api.github.com");
    assert_eq!(api_base("git.example.com", None), "https://git.example.com/api/v3");
    assert_eq!(api_base("github.com", Some("http://evil.example/".into())), "https://api.github.com");
    assert_eq!(api_base("github.com", Some("http://127.0.0.1:9/".into())), "http://127.0.0.1:9");
    assert_eq!(api_base("github.com", Some("https://evil.example".into())), "https://api.github.com");
    assert_eq!(api_base("github.com", Some("http://127.0.0.1:80@evil.example".into())), "https://api.github.com");
    assert_eq!(api_base("git.example.com", Some("https://git.example.com/api/v3/".into())), "https://git.example.com/api/v3");
    let facts = read_facts(&json!({"number": 5, "title": "T", "html_url": "u", "state": "open", "draft": true, "user": {"login": "x"}, "head": {"sha": "abc", "ref": "feat", "repo": {"full_name": "x/app"}}, "base": {"ref": "main"}, "changed_files": 3, "body": "b"})).unwrap();
    assert_eq!((facts.author.as_str(), facts.head.branch.as_str(), facts.base.as_str(), facts.changed_files, facts.draft), ("x", "feat", "main", Some(3), true));
    assert!(read_facts(&json!([])).is_err());
    let open = read_open(&json!([{"number": 2, "title": "A", "html_url": "u2", "created_at": "2026-01-01"}, {"title": "no number"}]));
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].number, 2);
}

fn flow_of(state: FlowState) -> PrFlow {
    PrFlow { id: "pr-1".into(), team_name: "PR team".into(), repo: "/r/app".into(), slug: "github.com/me/app".into(), branch: "dev".into(), state, ..PrFlow::default() }
}

#[test]
fn pr_work_file_and_texts() {
    let mut file = WorkFile::default();
    file.flows.push(flow_of(FlowState::Running));
    file.flows.push(flow_of(FlowState::Done));
    let text = serde_json::to_string(&file).unwrap();
    let mut back = parse_work(Some(&text));
    assert!(cut_running(&mut back));
    assert_eq!(back.flows[0].state, FlowState::Stopped);
    assert!(!cut_running(&mut back));
    assert_eq!(parse_work(Some("{bad")).flows.len(), 0);
    let many = WorkFile { flows: (0..40).map(|_| flow_of(FlowState::Done)).collect(), ..WorkFile::default() };
    assert_eq!(parse_work(Some(&serde_json::to_string(&many).unwrap())).flows.len(), 30);
    assert_eq!(pr_key("github.com/me/app", 3), "github.com/me/app#3");
    assert_eq!(wanted_number("review PR #12 please"), Some(12));
    assert_eq!(wanted_number("only 7"), Some(7));
    assert_eq!(wanted_number("all of them"), None);
    let mut flow = flow_of(FlowState::Done);
    flow.done = vec![DoneItem { number: 1, outcome: Outcome::Picked, comment: CommentState::Waiting, ..DoneItem::default() }, DoneItem { number: 2, outcome: Outcome::Nothing, ..DoneItem::default() }];
    let (chat, spoken) = end_text(&flow, &[1]);
    assert!(chat.contains("#1 picked") && chat.contains("#2 nothing to take") && chat.contains("Nixa posts 1 comment draft: #1"), "{chat}");
    assert!(spoken.contains("Nixa posts 1 comment draft"), "{spoken}");
    let brief = flow_brief(&flow);
    assert_eq!(brief["done"][0], "#1: picked, comment ready");
    assert_eq!(brief["state"], "done");
    let pr = crate::teams::model::RunPr { number: 9, title: "Fix".into(), url: "https://github.com/me/app/pull/9".into(), ..Default::default() };
    let input = super::pr_flow::BriefInput { flow: &flow, pr: &pr, purpose: "Take good parts", task: "", facts: None, diff: Path::new("/t/pr-9.diff"), stat: " a.rs | 2 +-", comment: Path::new("/t/comment.md"), skills: &[], mcps: &[], skill_dir: Path::new("/t/skills") };
    let text = brief_text(&input);
    assert!(text.contains("Nixa commits the picked parts with nixa_git_commit"));
    assert!(text.starts_with("# PR #9: Fix"));
    assert!(text.contains("refs/nixa/pr/9") && text.contains("/t/comment.md") && text.contains("Never commit"));
    let dir = tempfile::tempdir().unwrap();
    let note = dir.path().join("comment.md");
    assert!(!comment_ready(&note));
    std::fs::write(&note, "  \n").unwrap();
    assert!(!comment_ready(&note));
    std::fs::write(&note, "good").unwrap();
    assert!(comment_ready(&note));
    let _: Value = json!(FlowState::Running);
}

#[tokio::test]
async fn dirty_and_recent_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = real(dir.path()).join("app");
    new_repo(&root);
    let path = root.display().to_string();
    assert!(super::pr_flow::dirty_files(&path).await.unwrap().is_empty());
    assert!(!super::pr_flow::recent_change(&path, super::pr_flow::RECENT_MS, crate::util::now_ms()).await);
    std::fs::write(root.join("new.txt"), "x").unwrap();
    assert_eq!(super::pr_flow::dirty_files(&path).await.unwrap(), vec!["new.txt"]);
    assert!(super::pr_flow::recent_change(&path, super::pr_flow::RECENT_MS, crate::util::now_ms()).await);
    super::pr_flow::reset_tree(&path).await.unwrap();
    assert!(super::pr_flow::dirty_files(&path).await.unwrap().is_empty());
}

fn args(value: Value) -> crate::tools::Args {
    value.as_object().unwrap().clone()
}

#[tokio::test]
async fn state_and_commit_run_and_push_waits_for_a_yes() {
    let dir = tempfile::tempdir().unwrap();
    let base = real(dir.path());
    let app = app_in(&base);
    let root = base.join("app");
    new_repo(&root);
    let remote = base.join("remote.git");
    sh(&base, &["init", "-q", "--bare", "remote.git"]);
    sh(&root, &["remote", "add", "origin", &remote.display().to_string()]);
    sh(&root, &["checkout", "-q", "-b", "dev"]);
    std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
    std::fs::write(root.join(".env"), "KEY=1\n").unwrap();
    std::fs::write(root.join("notes.md"), "mine\n").unwrap();
    let project = root.display().to_string();
    let state: Value = serde_json::from_str(&super::call(&app, super::STATE, &args(json!({"project": project}))).await.unwrap().unwrap()).unwrap();
    let repo = &state["repos"][0];
    assert_eq!(repo["branch"], "dev");
    assert_eq!(repo["changed"], 3);
    assert_eq!(repo["untracked"], 3);
    let input = args(json!({"project": project, "message": "add a"}));
    let done: Value = serde_json::from_str(&super::call(&app, super::COMMIT, &input).await.unwrap().unwrap()).unwrap();
    assert_eq!(done["committed"], true, "{done}");
    assert!(done.get("held").is_none(), "{done}");
    assert_eq!(done["files"], json!(["a.rs"]));
    assert_eq!(done["left_out"].as_array().unwrap().len(), 2);
    assert_eq!(sh(&root, &["log", "-1", "--format=%s"]), "add a");
    let again: Value = serde_json::from_str(&super::call(&app, super::COMMIT, &input).await.unwrap().unwrap()).unwrap();
    assert_eq!(again["committed"], false);
    let push_input = args(json!({"project": project, "branch": "dev"}));
    let held: Value = serde_json::from_str(&super::call(&app, super::PUSH, &push_input).await.unwrap().unwrap()).unwrap();
    assert_eq!(held["held"], true, "{held}");
    assert_eq!(held["pushed"], json!([]));
    assert!(!Command::new("git").args(["--git-dir", &remote.display().to_string(), "rev-parse", "--verify", "-q", "refs/heads/dev"]).output().unwrap().status.success());
    let main = super::call(&app, super::PUSH, &args(json!({"project": project, "branch": "main"}))).await.unwrap().unwrap_err();
    assert!(main.contains("never pushes main") && main.contains("Nothing was pushed"), "{main}");
    assert!(!Command::new("git").args(["--git-dir", &remote.display().to_string(), "rev-parse", "--verify", "-q", "refs/heads/main"]).output().unwrap().status.success());
    said(&app, "yes push");
    let pushed: Value = serde_json::from_str(&super::call(&app, super::PUSH, &push_input).await.unwrap().unwrap()).unwrap();
    assert_eq!(pushed["pushed"].as_array().unwrap().len(), 2, "{pushed}");
    assert_eq!(pushed["upstream_set"], "origin/dev");
    assert!(pushed["not_pushed"][0].as_str().unwrap().contains("not committed"));
    assert!(Command::new("git").args(["--git-dir", &remote.display().to_string(), "rev-parse", "--verify", "-q", "refs/heads/dev"]).output().unwrap().status.success());
    let nothing: Value = serde_json::from_str(&super::call(&app, super::PUSH, &push_input).await.unwrap().unwrap()).unwrap();
    assert!(nothing["note"].as_str().unwrap().contains("Nothing to push"));
    let missing = super::call(&app, super::PUSH, &args(json!({"project": project, "branch": "nope"}))).await.unwrap().unwrap_err();
    assert!(missing.contains("no local branch"));
    assert!(super::call(&app, "other_tool", &args(json!({}))).await.is_none());
    let listed: Value = serde_json::from_str(&super::call(&app, super::PR_FLOW, &args(json!({"action": "list"}))).await.unwrap().unwrap()).unwrap();
    assert_eq!(listed["flows"], json!([]));
    assert!(super::call(&app, super::PR_FLOW, &args(json!({"action": "post"}))).await.unwrap().is_err());
}

#[tokio::test]
async fn behind_remote_is_not_pushed() {
    let dir = tempfile::tempdir().unwrap();
    let base = real(dir.path());
    let app = app_in(&base);
    let root = base.join("app");
    new_repo(&root);
    sh(&base, &["init", "-q", "--bare", "remote.git"]);
    sh(&root, &["remote", "add", "origin", &base.join("remote.git").display().to_string()]);
    sh(&root, &["checkout", "-q", "-b", "dev"]);
    sh(&root, &["push", "-q", "-u", "origin", "dev"]);
    let other = base.join("other");
    sh(&base, &["clone", "-q", "-b", "dev", &base.join("remote.git").display().to_string(), "other"]);
    for (k, v) in [("user.name", "T"), ("user.email", "t@example.invalid"), ("commit.gpgsign", "false"), ("core.hooksPath", "/dev/null")] {
        sh(&other, &["config", "--local", k, v]);
    }
    std::fs::write(other.join("b.txt"), "b").unwrap();
    sh(&other, &["add", "b.txt"]);
    sh(&other, &["commit", "-q", "-m", "b"]);
    sh(&other, &["push", "-q"]);
    sh(&root, &["fetch", "-q"]);
    std::fs::write(root.join("c.txt"), "c").unwrap();
    sh(&root, &["add", "c.txt"]);
    sh(&root, &["commit", "-q", "-m", "c"]);
    said(&app, "push it");
    let out: Value = serde_json::from_str(&super::call(&app, super::PUSH, &args(json!({"project": root.display().to_string(), "branch": "dev"}))).await.unwrap().unwrap()).unwrap();
    assert!(out["not_pushed"][0].as_str().unwrap().contains("behind"), "{out}");
    assert_eq!(out["pushed"], json!([]));
}

#[tokio::test]
async fn locked_space_still_commits() {
    let dir = tempfile::tempdir().unwrap();
    let base = real(dir.path());
    let app = app_in(&base);
    let root = base.join("app");
    new_repo(&root);
    std::fs::write(root.join("a.rs"), "x").unwrap();
    let conn = nixa_store::open(&app.db_file).unwrap();
    nixa_store::spaces::set_space(&conn, "s1", &nixa_store::spaces::SpacePatch { name: Some("App".into()), folder: Some(root.display().to_string()), locked: Some(true), ..Default::default() }).unwrap();
    let out: Value = serde_json::from_str(&super::call(&app, super::COMMIT, &args(json!({"project": "App", "message": "x"}))).await.unwrap().unwrap()).unwrap();
    assert_eq!(out["committed"], true, "{out}");
}
